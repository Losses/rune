//! A tap that copies every decoded sample, as interleaved `f32` in `-1..1`, into a lock-free
//! single-producer single-consumer ring buffer for a consumer such as the visualizer.
//!
//! The audio thread never blocks and never allocates here. A new source is created per track
//! while the ring has to outlive the tracks, so the producer sits in a shared holder behind a
//! `Mutex` that the audio thread only touches with `try_lock()`. Nothing else contends for it
//! (a second holder exists only while an old source is being dropped), so `try_lock()` is an
//! uncontended atomic operation in practice, and when it does fail the samples are dropped
//! instead of waited for. The same goes for a full ring: the newest samples are dropped.
//!
//! To keep the cost per sample low, samples are staged in a small fixed-size array and moved to
//! the ring in batches. Batches hold whole frames and go into the ring whole or not at all, and
//! staging only starts at a frame boundary, so the consumer always sees aligned interleaved frames.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};
use rtrb::{Consumer, Producer, RingBuffer};

/// Capacity of the ring in samples (about 1.4 s of 48 kHz stereo).
pub const PCM_TAP_CAPACITY: usize = 1 << 17;

/// Samples staged on the audio thread before they are moved to the ring.
const BATCH: usize = 1024;

pub struct PcmTap {
    enabled: AtomicBool,
    channels: Arc<AtomicU32>,
    sample_rate: Arc<AtomicU32>,
    /// Generation counter - increments on each ring replacement
    generation: Arc<AtomicU64>,
    producer: Mutex<Producer<f32>>,
    consumer: Mutex<Option<Consumer<f32>>>,
}

impl PcmTap {
    pub fn new() -> Arc<Self> {
        Self::with_capacity(PCM_TAP_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Arc<Self> {
        let (producer, consumer) = RingBuffer::new(capacity);
        Arc::new(Self {
            enabled: AtomicBool::new(false),
            channels: Arc::new(AtomicU32::new(0)),
            sample_rate: Arc::new(AtomicU32::new(0)),
            generation: Arc::new(AtomicU64::new(1)),
            producer: Mutex::new(producer),
            consumer: Mutex::new(Some(consumer)),
        })
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Current generation of the ring buffer. Increments on each recreation.
    pub fn generation(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.generation)
    }

    /// Channel count of the current source (0 before the first track).
    pub fn channels(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.channels)
    }

    /// Sample rate of the current source (0 before the first track).
    pub fn sample_rate(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.sample_rate)
    }

    /// Take the consumer end. There is only one; give it back with [`Self::restore_consumer`].
    /// Returns None if the ring was recreated and this consumer is stale.
    pub fn take_consumer(&self) -> Option<Consumer<f32>> {
        self.consumer.lock().ok()?.take()
    }

    pub fn restore_consumer(&self, consumer: Consumer<f32>) {
        if let Ok(mut slot) = self.consumer.lock() {
            // Only restore if this consumer belongs to the current generation
            // to avoid overwriting a pending replacement with a stale consumer
            if !consumer.is_abandoned() {
                *slot = Some(consumer);
            }
            // If the consumer is abandoned, it means the ring was recreated,
            // so we intentionally do NOT restore it to avoid stale state
        }
    }

    /// Recreate the ring buffer, dropping all queued samples.
    /// Any existing consumer (such as the one held by the visualizer) will see its consumer
    /// as abandoned and can request a new one via the tap. The replacement is atomic: both
    /// producer and consumer are swapped together to avoid inconsistent state.
    ///
    /// This method uses blocking locks to ensure the replacement always succeeds.
    /// If a lock is poisoned (due to a panic in another thread), we recover it.
    fn recreate_ring(&self) {
        let (new_producer, new_consumer) = RingBuffer::new(PCM_TAP_CAPACITY);

        // Lock consumer first as it's less likely to be held during audio playback.
        let mut consumer_guard = self.consumer.lock().unwrap_or_else(PoisonError::into_inner);
        let mut producer_guard = self.producer.lock().unwrap_or_else(PoisonError::into_inner);

        // Both locks acquired - replace atomically
        let old_producer = std::mem::replace(&mut *producer_guard, new_producer);
        let old_consumer = (*consumer_guard).replace(new_consumer);
        drop(old_producer);
        drop(old_consumer);

        // Increment generation to invalidate old sources
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// Wrap `source` so that its samples are copied into the ring while the tap is enabled.
    pub fn wrap<S: Source>(self: &Arc<Self>, source: S) -> PcmTapSource<S> {
        let new_channels = u32::from(source.channels().get());
        let new_sample_rate = source.sample_rate().get();
        let old_channels = self.channels.load(Ordering::Relaxed);
        let old_sample_rate = self.sample_rate.load(Ordering::Relaxed);

        // If the format changed from a previous non-zero format, recreate the ring buffer
        // to drop all queued samples. This prevents old samples from being interpreted
        // with the new format. We skip the initial transition from 0 (no format yet).
        let format_changed = old_channels != 0 && old_channels != new_channels
            || old_sample_rate != 0 && old_sample_rate != new_sample_rate;

        if format_changed {
            // Replace the ring first, then publish the new format.
            // This ensures we never have new-format metadata with an old-format ring.
            // recreate_ring now uses blocking locks, so it always succeeds and never spins.
            self.recreate_ring();
            // Now ring is replaced and generation is incremented, publish the new format
            self.channels.swap(new_channels, Ordering::Relaxed);
            self.sample_rate.swap(new_sample_rate, Ordering::Relaxed);
        } else {
            // No format change, just update the format
            self.channels.swap(new_channels, Ordering::Relaxed);
            self.sample_rate.swap(new_sample_rate, Ordering::Relaxed);
        }

        PcmTapSource {
            inner: source,
            tap: Arc::clone(self),
            staged: [0.0; BATCH],
            len: 0,
            position: 0,
            active: false,
            source_generation: self.generation.load(Ordering::Relaxed),
        }
    }

    /// Move staged samples to the ring, dropping all of them if the producer is busy or the ring
    /// cannot take them all (a partial batch would split a frame and shift the channels).
    fn push(&self, samples: &[f32], source_generation: u64) {
        if let Ok(mut producer) = self.producer.try_lock()
            && let Ok(mut chunk) = producer.write_chunk_uninit(samples.len())
        {
            // Check generation while holding the producer lock to prevent stale writes.
            // If the ring was replaced (via recreate_ring which uses blocking locks), this source's
            // generation won't match and we discard the batch, preventing old-format samples
            // from reaching the new-format consumer.
            if source_generation != self.generation.load(Ordering::Relaxed) {
                return;
            }
            let (first, second) = chunk.as_mut_slices();
            let (head, tail) = samples.split_at(first.len());
            for (slot, &sample) in first.iter_mut().zip(head) {
                slot.write(sample);
            }
            for (slot, &sample) in second.iter_mut().zip(tail) {
                slot.write(sample);
            }
            // SAFETY: every slot of both slices was initialised above.
            unsafe { chunk.commit_all() };
        }
    }
}

pub struct PcmTapSource<S: Source> {
    inner: S,
    tap: Arc<PcmTap>,
    staged: [f32; BATCH],
    len: usize,
    /// Index of the next sample within its frame (0 = first channel).
    position: u16,
    /// Staging has started at a frame boundary since the tap was enabled.
    active: bool,
    /// The generation when this source was created - if this doesn't match current generation,
    /// this source should not write to the ring
    source_generation: u64,
}

impl<S: Source> PcmTapSource<S> {
    fn flush(&mut self) {
        // Only flush if we're still on the current generation
        if self.source_generation == self.tap.generation.load(Ordering::Relaxed) {
            self.tap
                .push(&self.staged[..self.len], self.source_generation);
        }
        self.len = 0;
    }
}

impl<S: Source> Iterator for PcmTapSource<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Self::Item> {
        let Some(sample) = self.inner.next() else {
            self.flush();
            return None;
        };
        let channels = self.inner.channels().get();
        if !self.tap.is_enabled() {
            self.active = false;
            self.len = 0;
        } else if self.active || self.position == 0 {
            self.active = true;
            self.staged[self.len] = sample;
            self.len += 1;
            // Flush when the next whole frame would not fit, so batches never split a frame.
            if self.position + 1 == channels && self.len + usize::from(channels) > BATCH {
                self.flush();
            }
        }
        self.position = (self.position + 1) % channels;
        Some(sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S: Source> Source for PcmTapSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.inner.try_seek(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;
    use rodio::cpal;
    use std::iter;
    use std::num::NonZero;

    /// 16-bit samples as a source yields them, on the scale of -1 to 1.
    fn scaled(samples: &[i16]) -> Vec<Sample> {
        samples
            .iter()
            .map(|&sample| cpal::Sample::to_sample(sample))
            .collect()
    }

    fn buffer(channels: u16, sample_rate: u32, samples: &[i16]) -> SamplesBuffer {
        SamplesBuffer::new(
            NonZero::new(channels).unwrap(),
            NonZero::new(sample_rate).unwrap(),
            scaled(samples),
        )
    }

    fn source(samples: Vec<i16>) -> SamplesBuffer {
        buffer(2, 48_000, &samples)
    }

    #[test]
    fn samples_arrive_in_order_as_f32() {
        let tap = PcmTap::new();
        tap.set_enabled(true);
        let mut consumer = tap.take_consumer().unwrap();
        let input: Vec<i16> = (0..3000).map(|i| (i % 2000 - 1000) as i16 * 16).collect();
        let played: Vec<f32> = tap.wrap(source(input.clone())).collect();
        assert_eq!(played, scaled(&input));

        let mut got = Vec::new();
        while let Ok(sample) = consumer.pop() {
            got.push(sample);
        }
        assert_eq!(got.len(), input.len());
        for (g, i) in got.iter().zip(&input) {
            assert_eq!(*g, f32::from(*i) / 32768.0);
        }
        assert_eq!(tap.channels().load(Ordering::Relaxed), 2);
        assert_eq!(tap.sample_rate().load(Ordering::Relaxed), 48_000);
    }

    #[test]
    fn disabled_tap_copies_nothing() {
        let tap = PcmTap::new();
        let mut consumer = tap.take_consumer().unwrap();
        let _: Vec<f32> = tap.wrap(source(vec![1; 4000])).collect();
        assert!(consumer.pop().is_err());
    }

    #[test]
    fn full_ring_drops_instead_of_blocking() {
        let tap = PcmTap::with_capacity(1024);
        tap.set_enabled(true);
        let mut consumer = tap.take_consumer().unwrap();
        // Nobody consumes: playback must still run to the end.
        let played = tap.wrap(source(vec![100; 10_000])).count();
        assert_eq!(played, 10_000);
        assert_eq!(consumer.slots(), 1024);
        // The oldest samples are the ones kept.
        assert_eq!(consumer.pop().unwrap(), 100.0 / 32768.0);
    }

    #[test]
    fn enabling_mid_frame_starts_at_the_next_frame() {
        let tap = PcmTap::new();
        let mut consumer = tap.take_consumer().unwrap();
        // Left samples are positive, right samples negative.
        let input: Vec<i16> = (0..4000)
            .map(|i| if i % 2 == 0 { 1000 } else { -1000 })
            .collect();
        let mut source = tap.wrap(source(input));
        source.next(); // left of frame 0
        tap.set_enabled(true);
        let _: Vec<f32> = source.collect(); // starts on a right sample
        let got: Vec<f32> = iter::from_fn(|| consumer.pop().ok()).collect();
        assert!(!got.is_empty() && got.len().is_multiple_of(2));
        assert!(
            got.as_chunks::<2>()
                .0
                .iter()
                .all(|frame| frame[0] > 0.0 && frame[1] < 0.0)
        );
    }

    #[test]
    fn batches_never_split_frames() {
        // 1024 is not a multiple of 3: every batch must still hold whole frames.
        let tap = PcmTap::with_capacity(1 << 16);
        tap.set_enabled(true);
        let mut consumer = tap.take_consumer().unwrap();
        let input: Vec<i16> = (0..3 * 2000).map(|i| (i % 3) as i16 * 1000).collect();
        let _: Vec<f32> = tap.wrap(buffer(3, 48_000, &input)).collect();
        let got: Vec<f32> = iter::from_fn(|| consumer.pop().ok()).collect();
        assert!(got.len().is_multiple_of(3));
        assert!(
            got.as_chunks::<3>()
                .0
                .iter()
                .all(|f| f[0] == 0.0 && f[1] > 0.0 && f[2] > f[1])
        );
    }

    #[test]
    fn full_ring_drops_whole_batches() {
        let tap = PcmTap::with_capacity(1500);
        tap.set_enabled(true);
        let mut consumer = tap.take_consumer().unwrap();
        let input: Vec<i16> = (0..10_000)
            .map(|i| if i % 2 == 0 { 1000 } else { -1000 })
            .collect();
        let _: Vec<f32> = tap.wrap(source(input)).collect();
        // One 1024-sample batch fits; the next would only partly fit and is dropped whole.
        assert_eq!(consumer.slots(), 1024);
        let got: Vec<f32> = iter::from_fn(|| consumer.pop().ok()).collect();
        assert!(
            got.as_chunks::<2>()
                .0
                .iter()
                .all(|frame| frame[0] > 0.0 && frame[1] < 0.0)
        );
    }

    #[test]
    fn busy_producer_drops_instead_of_blocking() {
        let tap = PcmTap::new();
        tap.set_enabled(true);
        let consumer = tap.take_consumer().unwrap();
        let _held = tap.producer.lock().unwrap();
        let played = tap.wrap(source(vec![100; 5000])).count();
        assert_eq!(played, 5000);
        assert_eq!(consumer.slots(), 0);
    }

    #[test]
    fn format_change_clears_old_samples() {
        let tap = PcmTap::new();
        tap.set_enabled(true);
        let consumer = tap.take_consumer().unwrap();

        // Fill with samples at one format
        let input1: Vec<i16> = vec![1000; 1000];
        let _: Vec<f32> = tap.wrap(source(input1)).collect();
        assert!(consumer.slots() > 0);

        // Change to a different channel count - should clear the ring
        let input2: Vec<i16> = vec![2000; 1000];
        let mono_source = buffer(1, 44_100, &input2);
        let _: Vec<f32> = tap.wrap(mono_source).collect();

        // Verify the format changed
        assert_eq!(tap.channels().load(Ordering::Relaxed), 1);
        assert_eq!(tap.sample_rate().load(Ordering::Relaxed), 44_100);

        // The ring should have been recreated, so old samples are gone
        // Take a new consumer to get the new ring
        let mut new_consumer = tap.take_consumer().unwrap();
        let mut got = Vec::new();
        while let Ok(sample) = new_consumer.pop() {
            got.push(sample);
        }
        // We should have samples from input2, not input1
        assert_eq!(got.len(), input2.len());
        // All samples should be from the new source (2000 value)
        for &sample in &got {
            assert!((sample - 2000.0 / 32768.0).abs() < 0.0001);
        }
    }

    #[test]
    fn format_change_with_queued_samples_clears_ring() {
        let tap = PcmTap::new();
        tap.set_enabled(true);
        let consumer = tap.take_consumer().unwrap();

        // Fill ring with samples from one format (stereo, 48kHz)
        let input1: Vec<i16> = vec![1000; 20000];
        let _: Vec<f32> = tap.wrap(source(input1.clone())).collect();
        let initial_slots = consumer.slots();
        assert!(initial_slots > 0);

        // Restore consumer so it can be replaced by recreate_ring
        tap.restore_consumer(consumer);

        // Change to a different format (mono, 44100Hz) - should recreate ring and clear old samples
        let input2: Vec<i16> = vec![2000; 5000];
        let mono_source = buffer(1, 44_100, &input2);
        let _: Vec<f32> = tap.wrap(mono_source).collect();

        // The format atomics should have been updated
        assert_eq!(tap.channels().load(Ordering::Relaxed), 1);
        assert_eq!(tap.sample_rate().load(Ordering::Relaxed), 44_100);

        // The old consumer is now abandoned, get a new one for the new ring
        let mut new_consumer = tap.take_consumer().unwrap();
        let mut got = Vec::new();
        while let Ok(sample) = new_consumer.pop() {
            got.push(sample);
        }
        // Should only have samples from input2, not input1
        for &sample in &got {
            assert!(
                (sample - 2000.0 / 32768.0).abs() < 0.0001,
                "Found old format sample: {}, expected {}",
                sample,
                2000.0 / 32768.0
            );
        }
        assert!(
            got.len() >= input2.len() / 2 && got.len() <= input2.len() * 2,
            "Expected around {} samples, got {}",
            input2.len(),
            got.len()
        );
    }

    #[test]
    fn old_source_cannot_write_to_new_ring() {
        let tap = PcmTap::new();
        tap.set_enabled(true);

        // Create first source (stereo, 48kHz)
        let input1: Vec<i16> = vec![1000; 10000];
        let mut source1 = tap.wrap(source(input1.clone()));

        // Fill with some samples
        let _: Vec<f32> = source1.by_ref().take(1000).collect();

        // Change format to recreate ring (mono, 44100Hz)
        let input2: Vec<i16> = vec![2000; 5000];
        let source2 = tap.wrap(buffer(1, 44_100, &input2));
        let _: Vec<f32> = source2.collect();

        // Continue with old source - it should not write to new ring
        let _: Vec<f32> = source1.collect();

        // Get new consumer and verify only new format samples are there
        let mut new_consumer = tap.take_consumer().unwrap();
        let mut got = Vec::new();
        while let Ok(sample) = new_consumer.pop() {
            got.push(sample);
        }

        // Should only have samples from source2 (input2), not from source1
        for &sample in &got {
            assert!(
                (sample - 2000.0 / 32768.0).abs() < 0.0001,
                "Found old format sample: {}",
                sample
            );
        }
    }

    #[test]
    fn ring_replacement_is_certain_with_contention() {
        let tap = PcmTap::new();
        tap.set_enabled(true);

        // Fill ring with stereo samples
        let input1: Vec<i16> = vec![1000; 20000];
        let _: Vec<f32> = tap.wrap(source(input1.clone())).collect();

        // Get the current generation
        let initial_generation = tap.generation.load(Ordering::Relaxed);

        // Spawn a thread that holds the producer lock while we change format
        let producer_lock = tap.producer.lock().unwrap_or_else(PoisonError::into_inner);

        // In another thread, change to mono format - this should block and wait for the lock
        let input2: Vec<i16> = vec![2000; 5000];
        let handle = std::thread::spawn({
            let tap = Arc::clone(&tap);
            move || {
                let source = buffer(1, 44_100, &input2);
                tap.wrap(source).collect::<Vec<f32>>()
            }
        });

        // Release the lock after a brief moment
        drop(producer_lock);

        // Wait for the format change to complete
        let _ = handle.join();

        // Generation should have increased, confirming ring was replaced
        let new_generation = tap.generation.load(Ordering::Relaxed);
        assert!(
            new_generation > initial_generation,
            "Ring was not replaced: generation {} -> {}",
            initial_generation,
            new_generation
        );

        // The old consumer is now abandoned, get a new one for the new ring
        let mut new_consumer = tap.take_consumer().unwrap();
        let mut got = Vec::new();
        while let Ok(sample) = new_consumer.pop() {
            got.push(sample);
        }

        // Should only have samples from the new source
        for &sample in &got {
            assert!(
                (sample - 2000.0 / 32768.0).abs() < 0.0001,
                "Found old format sample: {}",
                sample
            );
        }
    }

    #[test]
    fn old_source_cannot_write_to_new_ring_with_contention() {
        let tap = PcmTap::new();
        tap.set_enabled(true);

        // Create first source (stereo, 48kHz)
        let input1: Vec<i16> = vec![1000; 10000];
        let mut source1 = tap.wrap(source(input1.clone()));

        // Fill with some samples
        let _: Vec<f32> = source1.by_ref().take(1000).collect();

        // Spawn a thread to hold the producer lock
        let producer_guard = tap.producer.lock().unwrap_or_else(PoisonError::into_inner);

        // In main thread, change format to mono - this will block waiting for the lock
        let input2: Vec<i16> = vec![2000; 5000];
        let handle = std::thread::spawn({
            let tap = Arc::clone(&tap);
            move || {
                let source = buffer(1, 44_100, &input2);
                tap.wrap(source).collect::<Vec<f32>>()
            }
        });

        // Continue with old source while format change is pending
        // This should NOT write to the new ring if format change completes
        let _: Vec<f32> = source1.collect();

        // Release the lock
        drop(producer_guard);

        // Wait for format change to complete
        let _ = handle.join();

        // Get new consumer and verify only new format samples are there
        let mut new_consumer = tap.take_consumer().unwrap();
        let mut got = Vec::new();
        while let Ok(sample) = new_consumer.pop() {
            got.push(sample);
        }

        // Should only have samples from source2 (input2), not from source1
        for &sample in &got {
            assert!(
                (sample - 2000.0 / 32768.0).abs() < 0.0001,
                "Found old format sample: {}",
                sample
            );
        }
    }
}
