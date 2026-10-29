//! Adaptive quality: lowers a scene's internal resolution and then its quality (`frame.quality`)
//! when frames do not arrive at the target rate, and raises them again when there is room.
//!
//! The GPU time of a frame alone is not a reliable signal: a phone lowers its GPU clock when the
//! load drops, and some scenes are bound by the latency of each ray rather than by the number of
//! pixels, so fewer pixels barely change the time or the frame rate. So the controller follows two
//! things over a window of [`WINDOW`] seconds: the frame rate that is delivered (one tick per
//! pass of the render loop, which the GPU holds back once frames pile up) and the GPU time.
//!
//! - Lowering needs both: the frame rate under [`AT_TARGET`] of the target and the GPU time over
//!   [`HIGH`] of the budget. At the target rate it never lowers, whatever the GPU time says.
//! - It lowers by a single step (a resolution step of 0.1, or one step of quality once the
//!   resolution is at its floor), then judges the step over [`JUDGE`] seconds: if the frame rate
//!   did not reach the target or rise by [`GAIN`], the step is undone, and lowering is off for a
//!   while ([`FLOOR_HOLD`], doubling each time), so a scene whose cost does not follow the
//!   resolution stays where it is. A new scene starts afresh.
//! - Raising comes only at the target rate, after [`HOLD`] seconds of room (quality first, then
//!   resolution), and is judged the same way: a raise that leaves the frame rate under the target
//!   is undone and doubles the time raising waits.

/// The lowest resolution factor, against the scene's own scale.
pub const MIN_FACTOR: f32 = 0.5;
/// Resolution factors come in steps of this, from [`MIN_FACTOR`] to 1.
const FACTOR_STEP: f32 = 0.05;
/// Steps from [`MIN_FACTOR`] to 1.
const FACTOR_STEPS: u32 = 10;
/// A lowering takes the resolution down by this many steps.
const LOWER_STEPS: u32 = 2;
/// Quality comes in steps of this, from 1 down to 0.
const QUALITY_STEP: f32 = 0.25;
const QUALITY_STEPS: u32 = 4;
/// Over this share of the budget, on average, the GPU time is too high.
const HIGH: f64 = 0.85;
/// What a raise of the resolution aims for, as a share of the budget; it is only taken when the
/// estimate for it stays under this.
const TARGET: f64 = 0.7;
/// Under this share of the budget there is room for a higher quality (whose cost is unknown).
const LOW: f64 = 0.55;
/// Frames arrive at the target rate from this share of it.
const AT_TARGET: f64 = 0.95;
/// A lowering helped if the frame rate rose by this share (or reached the target).
const GAIN: f64 = 1.05;
/// How long a window of frames is, in seconds.
const WINDOW: f64 = 0.5;
/// How long a window is that judges a change, in seconds.
const JUDGE: f64 = 1.0;
/// Ticks a window needs for a decision.
const MIN_FRAMES: u32 = 8;
/// Frames are not counted for this long after a change, a new scene or a pause (new targets, and
/// frames still in flight at the old size).
const SETTLE: f64 = 0.5;
/// A gap this long between ticks is a pause.
const GAP: f64 = 0.5;
/// Seconds of room before raising; doubled, up to [`MAX_HOLD`], when a raise is taken back.
const HOLD: f64 = 2.0;
const MAX_HOLD: f64 = 32.0;
/// Seconds without lowering after a lowering was undone; doubled, up to [`MAX_FLOOR_HOLD`], each
/// time.
const FLOOR_HOLD: f64 = 30.0;
const MAX_FLOOR_HOLD: f64 = 240.0;
/// A raise that stayed this long is not taken back any more, and raising may wait less again.
const REVERT: f64 = 4.0;
/// The resolution rises by at most this many steps at a time.
const MAX_RAISE: u32 = 3;

/// The resolution factor (against the scene's own scale) and the quality a scene draws with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    pub factor: f32,
    pub quality: f32,
}

impl Level {
    pub const FULL: Self = Self {
        factor: 1.0,
        quality: 1.0,
    };
}

/// Steps above the floor: of the factor (0 is [`MIN_FACTOR`]) and of the quality (0 is 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Steps {
    factor: u32,
    quality: u32,
}

impl Steps {
    const FULL: Self = Self {
        factor: FACTOR_STEPS,
        quality: QUALITY_STEPS,
    };

    fn level(self) -> Level {
        Level {
            factor: factor_at(self.factor),
            quality: self.quality as f32 * QUALITY_STEP,
        }
    }
}

fn factor_at(step: u32) -> f32 {
    MIN_FACTOR + step as f32 * FACTOR_STEP
}

/// A change that is being judged.
#[derive(Debug, Clone, Copy)]
enum Trial {
    /// Frames were at `fps` before this lowering, from the level `from`.
    Lowered { from: Steps, fps: f64 },
    /// A raise from the level `from`.
    Raised { from: Steps, fps: f64 },
}

/// What a [`Controller`] changed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Change {
    pub level: Level,
    /// Set when the change takes back a step that did not help: what it was, and the frame rate
    /// before it and after.
    pub undone: Option<Undone>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Undone {
    pub raise: bool,
    pub fps_before: f64,
    pub fps_after: f64,
}

/// The ticks and GPU times since the window began.
#[derive(Default)]
struct Window {
    first: f64,
    last: f64,
    ticks: u32,
    gpu_sum: f64,
    gpu_worst: f64,
    gpu_count: u32,
}

/// Decides the level from the frame rate and the GPU time of the frames.
pub struct Controller {
    /// Seconds per frame.
    budget: f64,
    steps: Steps,
    /// Frames before this time are not counted.
    settle_until: f64,
    window: Option<Window>,
    /// GPU times since the last tick, for the window that comes next.
    pending: Window,
    last_tick: Option<f64>,
    trial: Option<Trial>,
    /// Since when the windows have shown room.
    room_since: Option<f64>,
    hold: f64,
    /// When the last raise was, while it could still be taken back.
    raised_at: Option<f64>,
    /// Lowering is off until then (a lowering did not help).
    no_lowering_until: f64,
    floor_hold: f64,
}

impl Controller {
    /// For frames of `budget` seconds (one over the target frame rate), from time `now` (seconds,
    /// any clock that only goes forward).
    pub fn new(budget: f64, now: f64) -> Self {
        Self {
            budget,
            steps: Steps::FULL,
            settle_until: now + SETTLE,
            window: None,
            pending: Window::default(),
            last_tick: None,
            trial: None,
            room_since: None,
            hold: HOLD,
            raised_at: None,
            no_lowering_until: f64::NEG_INFINITY,
            floor_hold: FLOOR_HOLD,
        }
    }

    pub fn level(&self) -> Level {
        self.steps.level()
    }

    pub fn set_budget(&mut self, budget: f64) {
        self.budget = budget;
    }

    /// Start counting frames again after a while, keeping the level (the size changed).
    pub fn settle(&mut self, now: f64) {
        self.settle_until = now + SETTLE;
        self.window = None;
        self.pending = Window::default();
        self.room_since = None;
    }

    /// A frame that took `seconds` on the GPU.
    pub fn sample(&mut self, now: f64, seconds: f64) {
        if now < self.settle_until || !seconds.is_finite() || seconds < 0.0 {
            return;
        }
        let w = &mut self.pending;
        w.gpu_sum += seconds;
        w.gpu_worst = w.gpu_worst.max(seconds);
        w.gpu_count += 1;
    }

    /// One pass of the render loop, at `now`. Returns the change when the level changes.
    pub fn tick(&mut self, now: f64) -> Option<Change> {
        if self.last_tick.is_some_and(|last| now - last > GAP) {
            // A pause: what was being judged is neither kept nor undone.
            self.trial = None;
            self.settle(now);
        }
        self.last_tick = Some(now);
        if self.raised_at.is_some_and(|at| now - at > REVERT) {
            // The raise held: raising may wait less again.
            self.raised_at = None;
            self.hold = (self.hold / 2.0).max(HOLD);
        }
        if now < self.settle_until {
            return None;
        }
        let pending = std::mem::take(&mut self.pending);
        let w = self.window.get_or_insert_with(|| Window {
            first: now,
            ..Window::default()
        });
        w.gpu_sum += pending.gpu_sum;
        w.gpu_worst = w.gpu_worst.max(pending.gpu_worst);
        w.gpu_count += pending.gpu_count;
        w.last = now;
        w.ticks += 1;
        let length = if self.trial.is_some() { JUDGE } else { WINDOW };
        if now - w.first < length || w.ticks < MIN_FRAMES || w.gpu_count == 0 {
            return None;
        }
        let fps = f64::from(w.ticks - 1) / (w.last - w.first);
        // Without the worst frame, so that a single hitch does not count.
        let average = if w.gpu_count > 1 {
            (w.gpu_sum - w.gpu_worst) / f64::from(w.gpu_count - 1)
        } else {
            w.gpu_sum
        };
        self.window = None;
        let at_target = fps >= AT_TARGET / self.budget;

        if let Some(trial) = self.trial.take()
            && let Some(change) = self.judge(now, trial, fps, at_target)
        {
            return Some(change);
        }

        let next = if !at_target && average > HIGH * self.budget && now >= self.no_lowering_until {
            self.room_since = None;
            let next = self.lower()?;
            self.trial = Some(Trial::Lowered {
                from: self.steps,
                fps,
            });
            if self.raised_at.take().is_some() {
                self.hold = (self.hold * 2.0).min(MAX_HOLD);
            }
            next
        } else if at_target {
            let Some(next) = self.raise(average) else {
                self.room_since = None;
                return None;
            };
            let since = *self.room_since.get_or_insert(now);
            if now - since < self.hold {
                return None;
            }
            self.room_since = None;
            self.raised_at = Some(now);
            self.trial = Some(Trial::Raised {
                from: self.steps,
                fps,
            });
            next
        } else {
            self.room_since = None;
            return None;
        };
        self.steps = next;
        self.settle(now);
        Some(Change {
            level: next.level(),
            undone: None,
        })
    }

    /// Judge the step `trial` after the frame rate settled at `fps`: `None` if it stays, else the
    /// change that takes it back.
    fn judge(&mut self, now: f64, trial: Trial, fps: f64, at_target: bool) -> Option<Change> {
        let (from, raise, before) = match trial {
            Trial::Lowered { from, fps: before } => {
                if at_target || fps >= before * GAIN {
                    return None;
                }
                self.no_lowering_until = now + self.floor_hold;
                self.floor_hold = (self.floor_hold * 2.0).min(MAX_FLOOR_HOLD);
                (from, false, before)
            }
            Trial::Raised { from, fps: before } => {
                if at_target {
                    return None;
                }
                self.raised_at = None;
                self.hold = (self.hold * 2.0).min(MAX_HOLD);
                (from, true, before)
            }
        };
        self.steps = from;
        self.settle(now);
        Some(Change {
            level: from.level(),
            undone: Some(Undone {
                raise,
                fps_before: before,
                fps_after: fps,
            }),
        })
    }

    /// One step down from here, if there is one.
    fn lower(&self) -> Option<Steps> {
        let Steps { factor, quality } = self.steps;
        if factor > 0 {
            return Some(Steps {
                factor: factor.saturating_sub(LOWER_STEPS),
                quality,
            });
        }
        (quality > 0).then(|| Steps {
            factor,
            quality: quality - 1,
        })
    }

    /// Where to go from frames of `average` seconds that arrive at the target rate, if there is
    /// room.
    fn raise(&self, average: f64) -> Option<Steps> {
        let Steps { factor, quality } = self.steps;
        if quality < QUALITY_STEPS {
            return (average < LOW * self.budget).then_some(Steps {
                factor,
                quality: quality + 1,
            });
        }
        let now = factor_at(factor) as f64;
        (factor + 1..=(factor + MAX_RAISE).min(FACTOR_STEPS))
            .rev()
            .find(|&step| {
                let estimate = average * (factor_at(step) as f64 / now).powi(2);
                estimate < TARGET * self.budget
            })
            .map(|step| Steps {
                factor: step,
                quality,
            })
    }
}

/// The frame rate and GPU time of the frames over a stretch of time, for the log.
pub struct Report {
    interval: f64,
    /// The time of the first sample, the sum of the GPU times, the worst and their number.
    window: Option<(f64, f64, f64, u32)>,
    /// The times of the first and last tick, and their number.
    ticks: Option<(f64, f64, u32)>,
}

/// What a [`Report`] saw over its interval, in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub average: f64,
    pub worst: f64,
    pub frames: u32,
    /// Passes of the render loop per second.
    pub fps: f64,
}

impl Report {
    /// A summary at most every `interval` seconds.
    pub fn new(interval: f64) -> Self {
        Self {
            interval,
            window: None,
            ticks: None,
        }
    }

    /// One pass of the render loop, at `now`.
    pub fn tick(&mut self, now: f64) {
        let (_, last, count) = self.ticks.get_or_insert((now, now, 0));
        *last = now;
        *count += 1;
    }

    /// A frame that took `seconds`, measured at `now`. Returns the summary once the interval has
    /// gone by since its first frame.
    pub fn frame(&mut self, now: f64, seconds: f64) -> Option<Summary> {
        let (start, sum, worst, count) = self.window.get_or_insert((now, 0.0, 0.0, 0));
        *sum += seconds;
        *worst = worst.max(seconds);
        *count += 1;
        if now - *start < self.interval {
            return None;
        }
        let fps = match self.ticks {
            Some((first, last, ticks)) if last > first => f64::from(ticks - 1) / (last - first),
            _ => 0.0,
        };
        let summary = Summary {
            average: *sum / f64::from(*count),
            worst: *worst,
            frames: *count,
            fps,
        };
        self.window = None;
        self.ticks = None;
        Some(summary)
    }

    /// Forget the frames so far (another scene).
    pub fn clear(&mut self) {
        self.window = None;
        self.ticks = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET: f64 = 1.0 / 60.0;

    /// A GPU whose frame costs `full` seconds at the full level, the square of the factor, and
    /// `0.4 + 0.6 * quality` of that.
    fn cost(full: f64, level: Level) -> f64 {
        full * f64::from(level.factor).powi(2) * (0.4 + 0.6 * f64::from(level.quality))
    }

    /// What a frame costs the GPU and how long it takes to arrive.
    struct Frame {
        gpu: f64,
        period: f64,
    }

    /// Frames that arrive as fast as the GPU lets them, at 60 fps at most.
    fn bound(gpu: f64) -> Frame {
        Frame {
            gpu,
            period: gpu.max(BUDGET),
        }
    }

    /// Run `seconds` of frames from `start`, each costing what `load(time, level)` says (times a
    /// little noise); returns the times of the changes.
    fn run(
        controller: &mut Controller,
        start: f64,
        seconds: f64,
        load: impl Fn(f64, Level) -> Frame,
    ) -> Vec<(f64, Change)> {
        let mut changes = Vec::new();
        let mut noise = 12345u32;
        let mut now = start;
        while now < start + seconds {
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let jitter = 0.9 + 0.2 * f64::from(noise >> 8) / f64::from(1u32 << 24);
            let level = controller.level();
            assert!((MIN_FACTOR..=1.0).contains(&level.factor), "{level:?}");
            assert!((0.0..=1.0).contains(&level.quality), "{level:?}");
            let frame = load(now, level);
            controller.sample(now, frame.gpu * jitter);
            if let Some(change) = controller.tick(now) {
                changes.push((now, change));
            }
            now += if frame.period > BUDGET * 1.001 {
                frame.period * jitter
            } else {
                frame.period
            };
        }
        changes
    }

    fn levels(changes: &[(f64, Change)]) -> Vec<Level> {
        changes.iter().map(|(_, c)| c.level).collect()
    }

    #[test]
    fn a_light_scene_stays_at_full() {
        let mut c = Controller::new(BUDGET, 0.0);
        let changes = run(&mut c, 0.0, 60.0, |_, l| bound(cost(0.008, l)));
        assert!(changes.is_empty(), "{changes:?}");
        assert_eq!(c.level(), Level::FULL);
    }

    #[test]
    fn it_never_lowers_at_the_target_rate_whatever_the_gpu_time_says() {
        // The GPU time is over the budget (a phone's GPU shares with the compositor, or counts
        // time spent waiting), but frames arrive at 60 fps.
        for gpu in [0.015, 0.0175, 0.03] {
            let mut c = Controller::new(BUDGET, 0.0);
            let changes = run(&mut c, 0.0, 60.0, |_, _| Frame {
                gpu,
                period: BUDGET,
            });
            assert!(changes.is_empty(), "{gpu}: {changes:?}");
            assert_eq!(c.level(), Level::FULL);
        }
    }

    #[test]
    fn a_lowering_that_does_not_raise_the_frame_rate_is_undone_and_not_tried_soon() {
        // 20 ms at every level: the frame rate does not follow the resolution.
        let mut c = Controller::new(BUDGET, 0.0);
        let changes = run(&mut c, 0.0, 25.0, |_, _| bound(0.020));
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert!(changes[0].1.level.factor < 1.0);
        assert_eq!(changes[1].1.level, Level::FULL);
        assert!(changes[1].1.undone.is_some());
        assert_eq!(c.level(), Level::FULL);

        // The GPU time falls with the resolution, but the frame rate does not (latency-bound).
        let mut c = Controller::new(BUDGET, 0.0);
        let changes = run(&mut c, 0.0, 25.0, |_, l| Frame {
            gpu: 0.012 + 0.004 * f64::from(l.factor),
            period: 0.019,
        });
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert_eq!(c.level(), Level::FULL);

        // It tries again once the hold is over, and takes it back again, for longer.
        let mut c = Controller::new(BUDGET, 0.0);
        let changes = run(&mut c, 0.0, 200.0, |_, _| bound(0.020));
        let undone: Vec<f64> = changes
            .iter()
            .filter(|(_, c)| c.undone.is_some())
            .map(|(t, _)| *t)
            .collect();
        assert!(undone.len() >= 3 && undone.len() <= 4, "{undone:?}");
        assert!(undone[1] - undone[0] > 29.0, "{undone:?}");
        assert!(
            undone[2] - undone[1] > 1.8 * (undone[1] - undone[0]),
            "{undone:?}"
        );
        assert_eq!(c.level(), Level::FULL);
    }

    #[test]
    fn a_scene_that_follows_the_resolution_settles_where_it_reaches_the_target() {
        // 25 ms at full: about two thirds of the resolution fits.
        let mut c = Controller::new(BUDGET, 0.0);
        let changes = run(&mut c, 0.0, 60.0, |_, l| bound(cost(0.025, l)));
        assert!(!changes.is_empty());
        assert!(
            changes.iter().all(|(t, _)| *t < 12.0),
            "it keeps changing: {changes:?}"
        );
        assert!(changes.iter().all(|(_, c)| c.undone.is_none()));
        let level = c.level();
        assert_eq!(level.quality, 1.0);
        assert!(
            level.factor < 0.85 && level.factor > MIN_FACTOR,
            "{level:?}"
        );
        assert!(cost(0.025, level) < BUDGET / AT_TARGET, "{level:?}");
    }

    #[test]
    fn quality_drops_only_at_the_lowest_resolution_and_stops_at_its_floor() {
        // 0.3 s at full: even the lowest resolution is 75 ms.
        let mut c = Controller::new(BUDGET, 0.0);
        let changes = run(&mut c, 0.0, 120.0, |_, l| bound(cost(0.3, l)));
        for level in levels(&changes) {
            assert!(
                level.quality == 1.0 || level.factor == MIN_FACTOR,
                "{level:?}"
            );
        }
        assert!(changes.iter().all(|(t, _)| *t < 40.0), "{changes:?}");
        assert_eq!(
            c.level(),
            Level {
                factor: MIN_FACTOR,
                quality: 0.0
            }
        );
    }

    #[test]
    fn it_recovers_when_the_load_drops() {
        let mut c = Controller::new(BUDGET, 0.0);
        run(&mut c, 0.0, 60.0, |_, l| bound(cost(0.3, l)));
        assert!(c.level().quality < 1.0);
        // The load drops to a light one: quality comes back first, then the resolution.
        let changes = run(&mut c, 60.0, 60.0, |_, l| bound(cost(0.008, l)));
        assert_eq!(c.level(), Level::FULL);
        let levels = levels(&changes);
        let first_resolution = levels.iter().position(|l| l.factor > MIN_FACTOR);
        let last_quality = levels.iter().rposition(|l| l.quality < 1.0);
        assert!(first_resolution > last_quality, "{changes:?}");
        assert!(changes.last().unwrap().0 < 60.0 + 45.0, "too slow");
    }

    #[test]
    fn a_load_at_the_edge_does_not_flicker() {
        // At the lowest resolution a quality of 0.5 leaves room, but 0.75 is too slow for 60 fps:
        // every raise is undone, and raising waits longer each time.
        let mut c = Controller::new(BUDGET, 0.0);
        let changes = run(&mut c, 0.0, 200.0, |_, l| {
            if l.factor > MIN_FACTOR {
                return bound(0.1 * f64::from(l.factor).powi(2));
            }
            bound(match (l.quality * 4.0) as u32 {
                4 => 0.030,
                3 => 0.018,
                2 => 0.009,
                1 => 0.006,
                _ => 0.004,
            })
        });
        let raises: Vec<f64> = changes
            .iter()
            .filter(|(_, c)| c.undone.is_some_and(|u| u.raise))
            .map(|(t, _)| *t)
            .collect();
        assert!(raises.len() >= 3, "{changes:?}");
        for w in raises.windows(3) {
            assert!(
                w[2] - w[1] > w[1] - w[0],
                "raising does not wait longer: {raises:?}"
            );
        }
        let late = changes.iter().filter(|(t, _)| *t > 40.0).count();
        assert!(late <= 12, "{late} changes after 40 s: {changes:?}");
        assert!(c.level().quality <= 0.75);
    }

    #[test]
    fn pauses_and_single_spikes_change_nothing() {
        let mut c = Controller::new(BUDGET, 0.0);
        // One frame of 200 ms now and then, and a pause of 10 s.
        let load = |t: f64, l| {
            if ((t * 60.0).round() as u32).is_multiple_of(97) {
                Frame {
                    gpu: 0.2,
                    period: 0.2,
                }
            } else {
                bound(cost(0.008, l))
            }
        };
        let mut changes = run(&mut c, 0.0, 5.0, load);
        changes.extend(run(&mut c, 15.0, 5.0, load));
        assert!(changes.is_empty(), "{changes:?}");
    }

    #[test]
    fn the_report_comes_once_per_interval() {
        let mut report = Report::new(10.0);
        let mut summaries = Vec::new();
        for i in 0..(35 * 60) {
            let now = f64::from(i) / 60.0;
            let ms = if i == 100 { 0.030 } else { 0.010 };
            report.tick(now);
            if let Some(s) = report.frame(now, ms) {
                summaries.push(s);
            }
        }
        assert_eq!(summaries.len(), 3);
        assert!((summaries[0].average - 0.010).abs() < 0.0002);
        assert!((summaries[0].fps - 60.0).abs() < 0.5, "{:?}", summaries[0]);
        assert_eq!(summaries[0].worst, 0.030);
        assert_eq!(summaries[1].worst, 0.010);
    }
}
