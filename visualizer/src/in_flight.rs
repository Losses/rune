//! Frames on their way through the GPU. The render thread submits a frame and goes on to the next
//! one instead of waiting for it, so that encoding a frame overlaps drawing the one before and
//! the GPU never sits idle between frames. A GPU with gaps between its work is clocked down: on an
//! iPhone, waiting for every frame kept the GPU at its lowest performance state nearly all the
//! time, which made every frame slower again.
//!
//! A frame is handed out only once the GPU has finished it, and frames are handed out in the order
//! they were submitted. The queue holds each frame's target (pixel buffer, shared texture, shared
//! image), so that target is not drawn into again while the GPU still writes it.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Frames that the GPU may have yet to finish at once. A frame beyond these waits for the oldest
/// before it is drawn.
pub const MAX_IN_FLIGHT: usize = 2;

struct Entry<T> {
    frame: T,
    done: Arc<AtomicBool>,
}

impl<T> Entry<T> {
    fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }
}

/// Frames that were submitted and not handed out yet, oldest first.
pub struct InFlight<T> {
    entries: VecDeque<Entry<T>>,
}

impl<T> InFlight<T> {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
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

    /// Add `frame`, which was just submitted. The caller sets the returned flag once the GPU has
    /// finished that submission.
    #[cfg_attr(target_os = "android", allow(dead_code))]
    pub fn push(&mut self, frame: T) -> Arc<AtomicBool> {
        let done = Arc::new(AtomicBool::new(false));
        self.entries.push_back(Entry {
            frame,
            done: Arc::clone(&done),
        });
        done
    }

    /// The newest of the frames that the GPU has finished, taken out together with the older ones,
    /// which are dropped unseen. Frames are only taken from the front: a finished frame behind one
    /// that is not stays until that one is finished too, so a frame is never handed out before
    /// an older one, and an older one never after a newer one. (A queue finishes its submissions
    /// in order anyway.)
    pub fn take_finished(&mut self) -> Option<T> {
        let mut newest = None;
        while self.entries.front().is_some_and(Entry::is_done) {
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

    fn finish(flag: &AtomicBool) {
        flag.store(true, Ordering::Release);
    }

    /// Frames come out in the order they went in, never before the GPU finished them, and an older
    /// frame that finished together with a newer one is dropped.
    #[test]
    fn frames_come_out_in_order_once_finished() {
        let mut queue = InFlight::new();
        let first = queue.push(1);
        let second = queue.push(2);
        let third = queue.push(3);
        assert_eq!(queue.take_finished(), None);

        // The second is done before the first: it waits.
        finish(&second);
        assert_eq!(queue.take_finished(), None);
        assert_eq!(queue.unfinished(), 2);

        // Once the first is done too, the second comes out and the first is dropped.
        finish(&first);
        assert_eq!(queue.take_finished(), Some(2));
        assert_eq!(queue.take_finished(), None);

        finish(&third);
        assert_eq!(queue.take_finished(), Some(3));
        assert!(queue.is_empty());
        assert_eq!(queue.unfinished(), 0);
    }

    /// The queue is full with [`MAX_IN_FLIGHT`] unfinished frames; finished ones that were not
    /// taken out yet do not count.
    #[test]
    fn only_unfinished_frames_fill_the_queue() {
        let mut queue = InFlight::new();
        let flags: Vec<_> = (0..MAX_IN_FLIGHT).map(|i| queue.push(i)).collect();
        assert!(queue.is_full());

        finish(&flags[0]);
        assert!(!queue.is_full());
        assert_eq!(queue.unfinished(), MAX_IN_FLIGHT - 1);

        queue.push(MAX_IN_FLIGHT);
        assert!(queue.is_full());
        assert_eq!(queue.take_all(), (0..=MAX_IN_FLIGHT).collect::<Vec<_>>());
        assert!(queue.is_empty());
    }
}
