//! Published frames on their way to the screen.
//!
//! The render thread publishes a frame when the GPU has finished it, which happens a few
//! milliseconds earlier or later from frame to frame. Flutter takes a frame once per display
//! refresh. Handing it only the newest frame, two frames that finished within one refresh meant
//! one of them was never shown and the refresh before showed nothing new: on an iPhone up to a
//! third of the frames drawn at 60 fps never reached the screen. So frames are handed out one
//! per take, in the order they were published, the way a swapchain presents them.
//!
//! A frame that waits for the next refresh is shown that much later, and once one waits, the
//! next one does too. So the render loop moves its frames by how long the frames taken waited
//! ([`phase_step`]): it aims to publish each frame half a frame before Flutter takes it, which
//! drains a waiting frame by drawing a little later for a while instead of dropping one.
//!
//! Half a frame, because a frame may come that much later or earlier than its time and still be
//! taken at its own refresh. Frames come a few milliseconds early or late (the GPU is shared with
//! Flutter and the system compositor, and the render thread sees a finished frame only between
//! its other work). Aiming at 6 ms, frames that came late missed their refresh about once a
//! second on an iPhone; each miss showed a frame twice and the following ones a frame late until
//! the drain.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Frames that may wait to be taken. A reader that falls behind further skips the oldest, so the
/// picture is never more than this many frames late.
pub const MAX_WAITING: usize = 2;

/// The share of the difference from the aim that one frame makes up.
const GAIN: f64 = 0.1;
/// The furthest one frame is moved, so that frames keep coming at nearly the target rate.
const MAX_STEP: f64 = 0.5e-3;

/// How far (seconds, later if positive) to move the frames after one that waited `age` from
/// being published to being taken, with `interval` from one frame to the next: the aim is half
/// an interval.
pub fn phase_step(age: Duration, interval: Duration) -> f64 {
    ((age.as_secs_f64() - interval.as_secs_f64() / 2.0) * GAIN).clamp(-MAX_STEP, MAX_STEP)
}

/// What [`Published::take`] gives.
#[derive(Debug, PartialEq)]
pub struct Taken<T> {
    pub frame: Option<T>,
    /// The frame that waits to be taken next, if one does.
    pub next: Option<T>,
    /// The frame taken before, which this take replaced, for the caller to drop outside its lock.
    pub replaced: Option<T>,
    /// How long the frame waited from being published, if it was not taken before.
    pub age: Option<Duration>,
}

/// The frame taken last and the newer ones with the time they were published, oldest first.
pub struct Published<T> {
    taken: Option<T>,
    waiting: VecDeque<(T, Instant)>,
    /// Frames taken so far.
    count: u64,
}

impl<T> Default for Published<T> {
    fn default() -> Self {
        Self {
            taken: None,
            waiting: VecDeque::new(),
            count: 0,
        }
    }
}

impl<T: Clone> Published<T> {
    /// Add a frame published at `at`. Returns the ones dropped unseen because too many were
    /// waiting, for the caller to drop outside its lock.
    pub fn push(&mut self, frame: T, at: Instant) -> Vec<T> {
        self.waiting.push_back((frame, at));
        let extra = self.waiting.len().saturating_sub(MAX_WAITING);
        self.waiting
            .drain(..extra)
            .map(|(frame, _)| frame)
            .collect()
    }

    /// The newest frame, taken or not.
    pub fn newest(&self) -> Option<T> {
        self.waiting
            .back()
            .map(|(frame, _)| frame)
            .or(self.taken.as_ref())
            .cloned()
    }

    /// The oldest frame not taken yet, or the one taken last again if there is none, at `now`.
    pub fn take(&mut self, now: Instant) -> Taken<T> {
        let (replaced, age) = match self.waiting.pop_front() {
            Some((next, at)) => {
                self.count += 1;
                (
                    self.taken.replace(next),
                    Some(now.saturating_duration_since(at)),
                )
            }
            None => (None, None),
        };
        Taken {
            frame: self.taken.clone(),
            next: self.waiting.front().map(|(frame, _)| frame.clone()),
            replaced,
            age,
        }
    }

    /// How many frames were taken so far.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// Forget every frame.
    pub fn clear(&mut self) -> Self {
        std::mem::take(self)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};

    use super::*;
    use crate::in_flight::InFlight;

    const MS: Duration = Duration::from_millis(1);

    /// Two frames published within one take are both handed out, in order, each with how long it
    /// waited, and the reader is told that another one waits.
    #[test]
    fn frames_are_taken_in_order() {
        let start = Instant::now();
        let mut frames = Published::default();
        let nothing = Taken {
            frame: None,
            next: None,
            replaced: None,
            age: None,
        };
        assert_eq!(frames.take(start), nothing);
        assert!(frames.push(1, start).is_empty());
        assert!(frames.push(2, start + 5 * MS).is_empty());
        assert_eq!(frames.newest(), Some(2));
        let first = Taken {
            frame: Some(1),
            next: Some(2),
            replaced: None,
            age: Some(10 * MS),
        };
        assert_eq!(frames.take(start + 10 * MS), first);
        let second = Taken {
            frame: Some(2),
            next: None,
            replaced: Some(1),
            age: Some(22 * MS),
        };
        assert_eq!(frames.take(start + 27 * MS), second);
        // Nothing new: the last one again.
        let again = Taken {
            frame: Some(2),
            next: None,
            replaced: None,
            age: None,
        };
        assert_eq!(frames.take(start + 44 * MS), again);
        assert_eq!(frames.newest(), Some(2));
        assert_eq!(frames.count(), 2);
    }

    /// A reader that falls behind skips the oldest frames instead of falling further behind.
    #[test]
    fn at_most_max_waiting_frames_wait() {
        let start = Instant::now();
        let mut frames = Published::default();
        let mut dropped = Vec::new();
        for frame in 0..MAX_WAITING as u32 + 2 {
            dropped.extend(frames.push(frame, start));
        }
        assert_eq!(dropped, vec![0, 1]);
        assert_eq!(frames.take(start).frame, Some(2));
        assert_eq!(frames.take(start).frame, Some(3));
        assert_eq!(frames.take(start).replaced, None);
    }

    /// Frames taken more than half an interval after they were published move the next ones
    /// later, a little at a time, and frames taken sooner move them earlier.
    #[test]
    fn frames_move_toward_half_an_interval() {
        let interval = 16 * MS;
        assert_eq!(phase_step(8 * MS, interval), 0.0);
        let step = phase_step(10 * MS, interval);
        assert!(step > 0.0 && step < 2e-3, "{step}");
        assert!(phase_step(6 * MS, interval) < 0.0);
        // A frame that waited a whole refresh moves the next one by at most MAX_STEP.
        assert_eq!(phase_step(25 * MS, interval), MAX_STEP);
        assert_eq!(phase_step(Duration::ZERO, interval), -MAX_STEP);
    }

    /// What a simulated minute (at 60 fps) of a render loop and a reader shows.
    #[derive(Debug)]
    struct Shown {
        /// Refreshes after the first second that showed no new frame.
        repeats: usize,
        /// Frames dropped unseen, or never taken.
        lost: usize,
        /// The median and the 95th percentile of how long the frames taken after the first
        /// second waited from being published (ms).
        wait: (f64, f64),
        /// The same from the start of the loop pass that drew them.
        latency: (f64, f64),
        /// Frames the loop drew per second.
        fps: f64,
    }

    /// A render loop that draws a frame each interval, moved by `step` (as [`phase_step`]), the
    /// GPU finishing each frame `jitter` (ms, by frame number) earlier or later than usual, and
    /// publishing it as `crate::in_flight` hands it out (`hold`: holding early ones back); and a
    /// reader that takes a frame each refresh (with the period of the loop's interval, `offset`
    /// ms after the loop began) if one was published at least `notice` ms before.
    fn simulate(
        step: impl Fn(Duration, Duration) -> f64,
        jitter: impl Fn(usize) -> f64,
        hold: bool,
        offset: f64,
        notice: f64,
    ) -> Shown {
        const INTERVAL: f64 = 1000.0 / 60.0;
        const FRAMES: usize = 3600;
        /// How long the GPU takes from the start of a loop pass to finishing a frame (ms).
        const LATENCY: f64 = 12.0;
        let start = Instant::now();
        let at = |ms: f64| start + Duration::from_secs_f64(ms / 1e3);
        let ms = |time: Instant| time.duration_since(start).as_secs_f64() * 1e3;
        let interval = Duration::from_secs_f64(INTERVAL / 1e3);
        let mut frames = Published::default();
        let mut in_flight = InFlight::new();
        let (mut deadline, mut read, mut frame) = (0.0, offset, 0);
        // The last wait the reader saw, which the loop picks up at its next pass.
        let mut age: Option<Duration> = None;
        // Frames the GPU is drawing: when each will be finished, in order.
        let mut drawing: VecDeque<(f64, Arc<OnceLock<Instant>>)> = VecDeque::new();
        // When each frame's loop pass began.
        let mut began = Vec::new();
        let (mut repeats, mut lost, mut waits, mut latencies) = (0, 0, Vec::new(), Vec::new());
        while frame < FRAMES || !drawing.is_empty() || !in_flight.is_empty() {
            // The next event: a loop pass, a frame finished, one handed out or a read.
            let pass = (frame < FRAMES).then_some(deadline);
            let finish = drawing.front().map(|(time, _)| *time);
            let release = in_flight.release_at().filter(|_| hold).map(ms);
            let soonest = [pass, finish, release, Some(read)]
                .into_iter()
                .flatten()
                .fold(f64::MAX, f64::min);
            if finish == Some(soonest) || release == Some(soonest) {
                if finish == Some(soonest) {
                    let (time, done) = drawing.pop_front().unwrap();
                    let _ = done.set(at(time));
                }
                // Without the hold, a finished frame goes out at once.
                let now = if hold { at(soonest) } else { at(1e9) };
                let before = in_flight.len();
                if let Some(id) = in_flight.take_finished(now) {
                    lost += before - in_flight.len() - 1;
                    lost += frames.push(id, at(soonest)).len();
                }
            } else if pass == Some(soonest) {
                // As the render loop: the next pass comes an interval later, moved by the last
                // wait seen. A frame is finished only after the one before it.
                let time = deadline + LATENCY + jitter(frame);
                let after = drawing.back().map_or(f64::MIN, |(time, _)| *time);
                drawing.push_back((time.max(after), in_flight.push(frame, at(deadline))));
                began.push(deadline);
                frame += 1;
                deadline += INTERVAL;
                if let Some(age) = age.take() {
                    deadline += step(age, interval) * 1e3;
                }
            } else {
                // The reader sees only frames published `notice` before the refresh.
                let ready = frames
                    .waiting
                    .front()
                    .is_some_and(|(_, time)| *time <= at(read - notice));
                if ready {
                    let taken = frames.take(at(read));
                    age = taken.age;
                    if read > 1000.0 {
                        waits.push(taken.age.unwrap().as_secs_f64() * 1e3);
                        latencies.push(read - began[taken.frame.unwrap()]);
                    }
                } else if read > 1000.0 && frame < FRAMES {
                    repeats += 1;
                }
                read += INTERVAL;
            }
        }
        lost += FRAMES - frames.count() as usize;
        let percentiles = |mut values: Vec<f64>| {
            values.sort_by(f64::total_cmp);
            let rank = |share: f64| values[(share * values.len() as f64) as usize];
            (rank(0.5), rank(0.95))
        };
        Shown {
            repeats,
            lost,
            wait: percentiles(waits),
            latency: percentiles(latencies),
            fps: FRAMES as f64 / (deadline / 1e3),
        }
    }

    /// A number in 0..1 for `frame` and `salt`, always the same.
    fn random(frame: usize, salt: u64) -> f64 {
        let x = (frame as u64 ^ salt).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        ((x >> 11) as f64) / ((1u64 << 53) as f64)
    }

    /// Frames a few milliseconds early or late, about as often as on an iPhone: most within
    /// 1.5 ms, one in ten up to 6 ms late, one in twenty up to 5 ms early.
    fn jitter(frame: usize) -> f64 {
        let base = (random(frame, 1) - 0.5) * 3.0;
        match random(frame, 2) {
            r if r < 0.1 => base + 6.0 * random(frame, 3),
            r if r < 0.15 => base - 5.0 * random(frame, 3),
            _ => base,
        }
    }

    /// A scene whose GPU time changes from frame to frame, as Armillary's (from about 4 to
    /// 16 ms on an iPhone, with what it lights): any frame up to 5 ms earlier or later than
    /// usual, on top of [`jitter`].
    fn uneven(frame: usize) -> f64 {
        jitter(frame) + (random(frame, 4) - 0.5) * 10.0
    }

    /// Starting at any offset from the reader's refreshes, the loop settles on publishing about
    /// half an interval before each refresh, with every frame shown once and (nearly) no refresh
    /// without a new frame.
    #[test]
    fn the_loop_settles_half_an_interval_before_the_reader() {
        for offset in [0.0, 3.0, 8.0, 12.0, 16.0] {
            for hold in [false, true] {
                let shown = simulate(phase_step, jitter, hold, offset, 1.0);
                let case = format!("offset {offset}, hold {hold}: {shown:?}");
                // The last frame drawn may not be taken before the simulation ends.
                assert!(shown.lost <= 1, "{case}");
                assert!(shown.repeats <= 1, "{case}");
                let (median, p95) = shown.wait;
                assert!((7.0..=10.0).contains(&median), "{case}");
                assert!(p95 <= 14.0, "{case}");
                assert!(shown.fps > 59.9, "{case}");
            }
        }
    }

    /// Aiming closer to the refresh (6 ms), frames that come late miss it: each miss shows the
    /// frame before twice and makes the next ones wait a refresh until the loop has drained the
    /// wait again, which is why the aim is half an interval.
    #[test]
    fn aiming_close_to_the_refresh_misses_it() {
        let close =
            |age: Duration, _| ((age.as_secs_f64() - 6e-3) * GAIN).clamp(-MAX_STEP, MAX_STEP);
        let shown = simulate(close, jitter, false, 0.0, 1.0);
        let half = simulate(phase_step, jitter, false, 0.0, 1.0);
        assert!(
            shown.repeats >= 10 * half.repeats.max(1),
            "{shown:?} {half:?}"
        );
        assert!(shown.wait.1 > half.wait.1, "{shown:?} {half:?}");
    }

    /// With a GPU time that changes from frame to frame, frames handed out as soon as they are
    /// finished come too unevenly for the reader: refreshes without a new frame about once a
    /// second, frames waiting behind older ones, and the loop drawing later to drain those, below
    /// 60 fps. Holding early frames back to when most are finished (`crate::in_flight`) shows
    /// every frame once, at 60 fps, and frames are taken no later after they were drawn.
    #[test]
    fn early_frames_held_back_keep_an_uneven_scene_even() {
        let unheld = simulate(phase_step, uneven, false, 0.0, 1.0);
        let held = simulate(phase_step, uneven, true, 0.0, 1.0);
        let case = format!("{unheld:?} {held:?}");
        assert!(unheld.repeats >= 20 && unheld.fps < 59.5, "{case}");
        assert!(held.repeats <= 1 && held.lost <= 1, "{case}");
        assert!(held.fps > 59.9, "{case}");
        assert!(held.wait.1 < 12.0, "{case}");
        // From drawing to being taken: about the same in the middle, much less at worst.
        assert!(held.latency.0 < unheld.latency.0 + 1.0, "{case}");
        assert!(held.latency.1 + 5.0 < unheld.latency.1, "{case}");
    }
}
