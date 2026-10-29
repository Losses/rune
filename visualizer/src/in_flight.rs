//! Frames on their way through the GPU. The render thread submits a frame and goes on to the next
//! one instead of waiting for it, so that encoding a frame overlaps drawing the one before and
//! the GPU never sits idle between frames. A GPU with gaps between its work is clocked down: on an
//! iPhone, waiting for every frame kept the GPU at its lowest performance state nearly all the
//! time, which made every frame slower again.
//!
//! A frame is handed out only once the GPU has finished it, and frames are handed out in the order
//! they were submitted. The queue holds each frame's target (pixel buffer, shared texture, shared
//! image), so that target is not drawn into again while the GPU still writes it.
//!
//! A frame that the GPU finished sooner than most is held back until most would have been
//! finished ([`InFlight::hold`]). A scene whose GPU time changes from frame to frame (Armillary's
//! from about 4 to 16 ms on an iPhone, with what it lights) would otherwise hand out frames
//! unevenly: one early after one late came within one display refresh, and the reader took one of
//! them a refresh late, or the next refresh had nothing new (see `crate::published`).

use std::collections::VecDeque;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// Frames that the GPU may have yet to finish at once. A frame beyond these waits for the oldest
/// before it is drawn.
pub const MAX_IN_FLIGHT: usize = 2;

/// How many of the latest frames' times from submission to being finished [`InFlight::hold`]
/// looks at, and how many it needs before it holds frames back at all.
const LATENCIES: usize = 60;
const MIN_LATENCIES: usize = 15;
/// The share of those frames finished within the time that frames are held back to.
const HOLD_SHARE: f64 = 0.75;

struct Entry<T> {
    frame: T,
    submitted: Instant,
    /// When the GPU was seen to have finished it.
    done: Arc<OnceLock<Instant>>,
}

impl<T> Entry<T> {
    fn is_done(&self) -> bool {
        self.done.get().is_some()
    }
}

/// Frames that were submitted and not handed out yet, oldest first.
pub struct InFlight<T> {
    entries: VecDeque<Entry<T>>,
    /// The time from submission to being finished of the latest frames handed out.
    latencies: VecDeque<Duration>,
}

impl<T> InFlight<T> {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            latencies: VecDeque::new(),
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many frames the GPU has not finished yet.
    pub fn unfinished(&self) -> usize {
        self.entries.iter().filter(|entry| !entry.is_done()).count()
    }

    /// Whether as many frames as may be are still being drawn, so the next one has to wait.
    pub fn is_full(&self) -> bool {
        self.unfinished() >= MAX_IN_FLIGHT
    }

    /// Add `frame`, which was submitted at `submitted`. The caller sets the returned time once the
    /// GPU has finished that submission.
    #[cfg_attr(target_os = "android", allow(dead_code))]
    pub fn push(&mut self, frame: T, submitted: Instant) -> Arc<OnceLock<Instant>> {
        let done = Arc::new(OnceLock::new());
        self.entries.push_back(Entry {
            frame,
            submitted,
            done: Arc::clone(&done),
        });
        done
    }

    /// How long after its submission a finished frame is held back: as long as it took the GPU to
    /// finish [`HOLD_SHARE`] of the latest frames, or nothing before there are enough of them.
    pub fn hold(&self) -> Duration {
        if self.latencies.len() < MIN_LATENCIES {
            return Duration::ZERO;
        }
        let mut sorted: Vec<Duration> = self.latencies.iter().copied().collect();
        sorted.sort();
        let rank = (HOLD_SHARE * sorted.len() as f64).ceil() as usize;
        sorted[rank.clamp(1, sorted.len()) - 1]
    }

    /// When the oldest frame, finished but held back, is handed out, if it is one.
    pub fn release_at(&self) -> Option<Instant> {
        let front = self.entries.front().filter(|entry| entry.is_done())?;
        Some(front.submitted + self.hold())
    }

    /// The newest of the frames that the GPU has finished and that are not held back any more at
    /// `now`, taken out together with the older ones, which are dropped unseen. Frames are only
    /// taken from the front: a finished frame behind one that is not stays until that one is
    /// finished too, so a frame is never handed out before an older one, and an older one never
    /// after a newer one. (A queue finishes its submissions in order anyway.)
    pub fn take_finished(&mut self, now: Instant) -> Option<T> {
        let hold = self.hold();
        let mut newest = None;
        while let Some(entry) = self.entries.front() {
            let Some(&done) = entry.done.get() else { break };
            if entry.submitted + hold > now {
                break;
            }
            self.latencies
                .push_back(done.saturating_duration_since(entry.submitted));
            if self.latencies.len() > LATENCIES {
                self.latencies.pop_front();
            }
            newest = self.entries.pop_front().map(|entry| entry.frame);
        }
        newest
    }

    /// Every frame, finished or not, taken out oldest first. The caller makes sure the GPU is done
    /// with them, or keeps them from being dropped.
    pub fn take_all(&mut self) -> Vec<T> {
        self.entries.drain(..).map(|entry| entry.frame).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn finish(done: &OnceLock<Instant>) {
        let _ = done.set(Instant::now());
    }

    /// Frames come out in the order they went in, never before the GPU finished them, and an older
    /// frame that finished together with a newer one is dropped.
    #[test]
    fn frames_come_out_in_order_once_finished() {
        let mut queue = InFlight::new();
        let first = queue.push(1, Instant::now());
        let second = queue.push(2, Instant::now());
        let third = queue.push(3, Instant::now());
        assert_eq!(queue.take_finished(Instant::now()), None);

        // The second is done before the first: it waits.
        finish(&second);
        assert_eq!(queue.take_finished(Instant::now()), None);
        assert_eq!(queue.unfinished(), 2);

        // Once the first is done too, the second comes out and the first is dropped.
        finish(&first);
        assert_eq!(queue.take_finished(Instant::now()), Some(2));
        assert_eq!(queue.take_finished(Instant::now()), None);

        finish(&third);
        assert_eq!(queue.take_finished(Instant::now()), Some(3));
        assert!(queue.is_empty());
        assert_eq!(queue.unfinished(), 0);
    }

    /// The queue is full with [`MAX_IN_FLIGHT`] unfinished frames; finished ones that were not
    /// taken out yet do not count.
    #[test]
    fn only_unfinished_frames_fill_the_queue() {
        let mut queue = InFlight::new();
        let flags: Vec<_> = (0..MAX_IN_FLIGHT)
            .map(|i| queue.push(i, Instant::now()))
            .collect();
        assert!(queue.is_full());

        finish(&flags[0]);
        assert!(!queue.is_full());
        assert_eq!(queue.unfinished(), MAX_IN_FLIGHT - 1);

        queue.push(MAX_IN_FLIGHT, Instant::now());
        assert!(queue.is_full());
        assert_eq!(queue.take_all(), (0..=MAX_IN_FLIGHT).collect::<Vec<_>>());
        assert!(queue.is_empty());
    }

    /// Once enough frames were finished, a frame the GPU finished sooner than most is held back
    /// until most would have been finished, measured from its submission.
    #[test]
    fn early_frames_are_held_back() {
        let start = Instant::now();
        let mut queue = InFlight::new();
        assert_eq!(queue.hold(), Duration::ZERO);
        // Frames finished 6 to 15 ms after their submission.
        for i in 0..MIN_LATENCIES as u32 + 5 {
            let submitted = start + i * 20 * MS;
            let done = queue.push(i, submitted);
            let _ = done.set(submitted + (6 + i % 10) * MS);
            assert_eq!(queue.take_finished(submitted + 20 * MS), Some(i));
        }
        let hold = queue.hold();
        assert!((12 * MS..=14 * MS).contains(&hold), "{hold:?}");
        // One finished 5 ms after its submission waits for the hold.
        let submitted = start + 1000 * MS;
        let done = queue.push(99, submitted);
        assert_eq!(queue.release_at(), None);
        let _ = done.set(submitted + 5 * MS);
        assert_eq!(queue.release_at(), Some(submitted + hold));
        assert_eq!(queue.take_finished(submitted + 6 * MS), None);
        assert_eq!(queue.take_finished(submitted + hold), Some(99));
    }
}
