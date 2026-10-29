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

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::source::SeekError;
use rodio::{Sample, Source};
use rtrb::{Consumer, Producer, RingBuffer};

/// Capacity of the ring in samples (about 1.4 s of 48 kHz stereo).
pub const PCM_TAP_CAPACITY: usize = 1 << 17;

/// Samples staged on the audio thread before they are moved to the ring.
const BATCH: usize = 1024;

pub struct PcmTap {
    enabled: AtomicBool,
    channels: Arc<AtomicU32>,
    sample_rate: Arc<AtomicU32>,
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

    /// Take the consumer end. There is only one; give it back with [`Self::restore_consumer`].
    pub fn take_consumer(&self) -> Option<Consumer<f32>> {
        self.consumer.lock().ok()?.take()
    }

    pub fn restore_consumer(&self, consumer: Consumer<f32>) {
        if let Ok(mut slot) = self.consumer.lock() {
            *slot = Some(consumer);
        }
    }

    /// Channel count of the current source (0 before the first track).
    pub fn channels(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.channels)
    }

    /// Sample rate of the current source (0 before the first track).
    pub fn sample_rate(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.sample_rate)
    }

    /// Wrap `source` so that its samples are copied into the ring while the tap is enabled.
    pub fn wrap<S: Source>(self: &Arc<Self>, source: S) -> PcmTapSource<S>
    where
        S::Item: Sample,
    {
        self.channels
            .store(u32::from(source.channels()), Ordering::Relaxed);
        self.sample_rate
            .store(source.sample_rate(), Ordering::Relaxed);
        PcmTapSource {
            inner: source,
            tap: Arc::clone(self),
            staged: [0.0; BATCH],
            len: 0,
            position: 0,
            active: false,
        }
    }

    /// Move staged samples to the ring, dropping all of them if the producer is busy or the ring
    /// cannot take them all (a partial batch would split a frame and shift the channels).
    fn push(&self, samples: &[f32]) {
        if let Ok(mut producer) = self.producer.try_lock()
            && let Ok(mut chunk) = producer.write_chunk_uninit(samples.len())
        {
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

pub struct PcmTapSource<S: Source>
where
    S::Item: Sample,
{
    inner: S,
    tap: Arc<PcmTap>,
    staged: [f32; BATCH],
    len: usize,
    /// Index of the next sample within its frame (0 = first channel).
    position: u16,
    /// Staging has started at a frame boundary since the tap was enabled.
    active: bool,
}

impl<S: Source> PcmTapSource<S>
where
    S::Item: Sample,
{
    fn flush(&mut self) {
        self.tap.push(&self.staged[..self.len]);
        self.len = 0;
    }
}

impl<S: Source> Iterator for PcmTapSource<S>
where
    S::Item: Sample,
{
    type Item = S::Item;

    fn next(&mut self) -> Option<Self::Item> {
        let Some(sample) = self.inner.next() else {
            self.flush();
            return None;
        };
        let channels = self.inner.channels().max(1);
        if !self.tap.is_enabled() {
            self.active = false;
            self.len = 0;
        } else if self.active || self.position == 0 {
            self.active = true;
            self.staged[self.len] = sample.to_f32();
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

impl<S: Source> Source for PcmTapSource<S>
where
    S::Item: Sample,
{
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }

    fn channels(&self) -> u16 {
        self.inner.channels()
    }

    fn sample_rate(&self) -> u32 {
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

    fn source(samples: Vec<i16>) -> SamplesBuffer<i16> {
        SamplesBuffer::new(2, 48_000, samples)
    }

    #[test]
    fn samples_arrive_in_order_as_f32() {
        let tap = PcmTap::new();
        tap.set_enabled(true);
        let mut consumer = tap.take_consumer().unwrap();
        let input: Vec<i16> = (0..3000).map(|i| (i % 2000 - 1000) as i16 * 16).collect();
        let played: Vec<i16> = tap.wrap(source(input.clone())).collect();
        assert_eq!(played, input);

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
        let _: Vec<i16> = tap.wrap(source(vec![1; 4000])).collect();
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
        let _: Vec<i16> = source.collect(); // starts on a right sample
        let got: Vec<f32> = std::iter::from_fn(|| consumer.pop().ok()).collect();
        assert!(!got.is_empty() && got.len().is_multiple_of(2));
        assert!(
            got.chunks_exact(2)
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
        let _: Vec<i16> = tap.wrap(SamplesBuffer::new(3, 48_000, input)).collect();
        let got: Vec<f32> = std::iter::from_fn(|| consumer.pop().ok()).collect();
        assert!(got.len().is_multiple_of(3));
        assert!(
            got.chunks_exact(3)
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
        let _: Vec<i16> = tap.wrap(source(input)).collect();
        // One 1024-sample batch fits; the next would only partly fit and is dropped whole.
        assert_eq!(consumer.slots(), 1024);
        let got: Vec<f32> = std::iter::from_fn(|| consumer.pop().ok()).collect();
        assert!(
            got.chunks_exact(2)
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
}
