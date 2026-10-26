//! Adaptive quality: keeps the GPU time of a frame inside the frame budget by lowering the scene's
//! internal resolution and then its quality (`frame.quality`), and raises them again when there is
//! clearly room.
//!
//! The two form one ladder. Going down, the resolution goes first, in one jump to where the frame
//! should take [`TARGET`] of the budget (a frame costs about the square of the resolution factor),
//! down to [`MIN_FACTOR`]; only then does the quality drop, a step at a time. Going up, the
//! quality comes back first, then the resolution. Decisions are made on the average of a window
//! of frames (leaving out its worst one), never on single frames: lowering takes an average over
//! [`HIGH`] of the budget, raising needs [`HOLD`] seconds of room and an estimate that stays under
//! [`TARGET`]. A raise that had to be taken back soon after doubles the time raising waits, so a
//! load at the edge does not flicker.

/// The lowest resolution factor, against the scene's own scale.
pub const MIN_FACTOR: f32 = 0.5;
/// Resolution factors come in steps of this, from [`MIN_FACTOR`] to 1.
const FACTOR_STEP: f32 = 0.05;
/// Steps from [`MIN_FACTOR`] to 1.
const FACTOR_STEPS: u32 = 10;
/// Quality comes in steps of this, from 1 down to 0.
const QUALITY_STEP: f32 = 0.25;
const QUALITY_STEPS: u32 = 4;
/// Over this share of the budget, on average, frames are made cheaper.
const HIGH: f64 = 0.85;
/// What a change aims for, as a share of the budget; a higher resolution is only taken when the
/// estimate for it stays under this.
const TARGET: f64 = 0.7;
/// Under this share of the budget there is room for a higher quality (whose cost is unknown).
const LOW: f64 = 0.55;
/// How long a window of frames is, in seconds.
const WINDOW: f64 = 0.5;
/// Frames a window needs for a decision.
const MIN_FRAMES: u32 = 8;
/// Frames are not counted for this long after a change, a new scene or a pause (new targets, and
/// frames still in flight at the old size).
const SETTLE: f64 = 0.5;
/// A gap this long between frames is a pause.
const GAP: f64 = 0.5;
/// Seconds of room before raising; doubled, up to [`MAX_HOLD`], when a raise is taken back.
const HOLD: f64 = 2.0;
const MAX_HOLD: f64 = 32.0;
/// A lowering this soon after a raise takes it back.
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

/// Decides the level from the GPU time of each frame.
pub struct Controller {
    /// Seconds per frame.
    budget: f64,
    steps: Steps,
    /// Frames before this time are not counted.
    settle_until: f64,
    /// The window: the time of its first frame, the sum of the frame times, the worst and their
    /// number.
    window: Option<(f64, f64, f64, u32)>,
    last_frame: Option<f64>,
    /// Since when the windows have shown room.
    room_since: Option<f64>,
    hold: f64,
    /// When the last raise was, while it could still be taken back.
    raised_at: Option<f64>,
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
            last_frame: None,
            room_since: None,
            hold: HOLD,
            raised_at: None,
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
        self.room_since = None;
    }

    /// A frame that took `seconds` on the GPU, measured at `now`. Returns the new level when it
    /// changes.
    pub fn frame(&mut self, now: f64, seconds: f64) -> Option<Level> {
        if self.last_frame.is_some_and(|last| now - last > GAP) {
            self.settle(now);
        }
        self.last_frame = Some(now);
        if self.raised_at.is_some_and(|at| now - at > REVERT) {
            // The raise held: raising may wait less again.
            self.raised_at = None;
            self.hold = (self.hold / 2.0).max(HOLD);
        }
        if now < self.settle_until || !seconds.is_finite() || seconds < 0.0 {
            return None;
        }
        let (start, sum, worst, count) = self.window.get_or_insert((now, 0.0, 0.0, 0));
        *sum += seconds;
        *worst = worst.max(seconds);
        *count += 1;
        if now - *start < WINDOW || *count < MIN_FRAMES {
            return None;
        }
        // Without the worst frame, so that a single hitch does not count.
        let average = (*sum - *worst) / f64::from(*count - 1);
        self.window = None;

        let next = if average > HIGH * self.budget {
            self.room_since = None;
            let next = self.lower(average)?;
            if self.raised_at.take().is_some() {
                self.hold = (self.hold * 2.0).min(MAX_HOLD);
            }
            next
        } else {
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
            next
        };
        self.steps = next;
        self.settle(now);
        Some(next.level())
    }

    /// Where to go from frames of `average` seconds that are over the budget, if anywhere.
    fn lower(&self, average: f64) -> Option<Steps> {
        let Steps { factor, quality } = self.steps;
        if factor > 0 {
            // A frame costs about the square of the factor.
            let wanted = factor_at(factor) as f64 * (TARGET * self.budget / average).sqrt();
            let step = ((wanted as f32 - MIN_FACTOR) / FACTOR_STEP + 1e-4).floor();
            let step = (step.max(0.0) as u32).min(factor - 1);
            return Some(Steps {
                factor: step,
                quality,
            });
        }
        (quality > 0).then(|| Steps {
            factor,
            quality: quality - 1,
        })
    }

    /// Where to go from frames of `average` seconds that are within the budget, if there is room.
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

/// The GPU time of the frames over a stretch of time, for the log.
pub struct Report {
    interval: f64,
    /// The time of the first frame, the sum of the frame times, the worst and their number.
    window: Option<(f64, f64, f64, u32)>,
}

/// What a [`Report`] saw over its interval, in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub average: f64,
    pub worst: f64,
    pub frames: u32,
}

impl Report {
    /// A summary at most every `interval` seconds.
    pub fn new(interval: f64) -> Self {
        Self {
            interval,
            window: None,
        }
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
        let summary = Summary {
            average: *sum / f64::from(*count),
            worst: *worst,
            frames: *count,
        };
        self.window = None;
        Some(summary)
    }

    /// Forget the frames so far (another scene).
    pub fn clear(&mut self) {
        self.window = None;
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

    /// Run `seconds` of frames at 60 fps from `start`, each costing `load(time, level)` (times a
    /// little noise); returns the times of the changes.
    fn run(
        controller: &mut Controller,
        start: f64,
        seconds: f64,
        load: impl Fn(f64, Level) -> f64,
    ) -> Vec<(f64, Level)> {
        let mut changes = Vec::new();
        let mut noise = 12345u32;
        let frames = (seconds * 60.0) as u32;
        for i in 0..frames {
            let now = start + f64::from(i) / 60.0;
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let jitter = 0.9 + 0.2 * f64::from(noise >> 8) / f64::from(1u32 << 24);
            let level = controller.level();
            assert!((MIN_FACTOR..=1.0).contains(&level.factor), "{level:?}");
            assert!((0.0..=1.0).contains(&level.quality), "{level:?}");
            if let Some(level) = controller.frame(now, load(now, level) * jitter) {
                changes.push((now, level));
            }
        }
        changes
    }

    #[test]
    fn a_light_scene_stays_at_full() {
        let mut c = Controller::new(BUDGET, 0.0);
        let changes = run(&mut c, 0.0, 60.0, |_, l| cost(0.008, l));
        assert!(changes.is_empty(), "{changes:?}");
        assert_eq!(c.level(), Level::FULL);
    }

    #[test]
    fn a_heavy_scene_settles_and_stays() {
        // 25 ms at full: about two thirds of the resolution fits.
        let mut c = Controller::new(BUDGET, 0.0);
        let changes = run(&mut c, 0.0, 60.0, |_, l| cost(0.025, l));
        assert!(!changes.is_empty());
        assert!(
            changes.iter().all(|(t, _)| *t < 5.0),
            "it keeps changing: {changes:?}"
        );
        let level = c.level();
        assert_eq!(level.quality, 1.0);
        assert!(level.factor < 0.8 && level.factor > MIN_FACTOR, "{level:?}");
        assert!(cost(0.025, level) < HIGH * BUDGET);
    }

    #[test]
    fn quality_drops_only_at_the_lowest_resolution_and_stops_at_its_floor() {
        // 80 ms at full: even half the resolution is 20 ms.
        let mut c = Controller::new(BUDGET, 0.0);
        let changes = run(&mut c, 0.0, 60.0, |_, l| cost(0.08, l));
        for (_, level) in &changes {
            assert!(
                level.quality == 1.0 || level.factor == MIN_FACTOR,
                "{level:?}"
            );
        }
        assert!(changes.iter().all(|(t, _)| *t < 10.0), "{changes:?}");
        let level = c.level();
        assert_eq!(level.factor, MIN_FACTOR);
        assert!(level.quality < 1.0);
        assert!(cost(0.08, level) < HIGH * BUDGET * 1.1, "{level:?}");

        // Hopelessly heavy: everything goes to its floor and stays there.
        let mut c = Controller::new(BUDGET, 0.0);
        run(&mut c, 0.0, 30.0, |_, l| cost(1.0, l));
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
        run(&mut c, 0.0, 20.0, |_, l| cost(0.08, l));
        assert!(c.level().quality < 1.0);
        // The load drops to a light one: quality comes back first, then the resolution.
        let changes = run(&mut c, 20.0, 40.0, |_, l| cost(0.008, l));
        assert_eq!(c.level(), Level::FULL);
        let first_resolution = changes.iter().position(|(_, l)| l.factor > MIN_FACTOR);
        let last_quality = changes.iter().rposition(|(_, l)| l.quality < 1.0);
        assert!(first_resolution > last_quality, "{changes:?}");
        assert!(changes.last().unwrap().0 < 45.0, "too slow: {changes:?}");
    }

    #[test]
    fn a_load_at_the_edge_does_not_flicker() {
        // At the lowest resolution a quality of 0.5 seems to leave room, but 0.75 is over the
        // budget: every raise is taken back, and raising waits longer each time.
        let mut c = Controller::new(BUDGET, 0.0);
        let changes = run(&mut c, 0.0, 120.0, |_, l| {
            if l.factor > MIN_FACTOR {
                return 0.1 * f64::from(l.factor).powi(2);
            }
            match (l.quality * 4.0) as u32 {
                4 => 0.020,
                3 => 0.0145,
                2 => 0.009,
                1 => 0.006,
                _ => 0.004,
            }
        });
        let raises: Vec<f64> = changes
            .windows(2)
            .filter(|w| w[1].1.quality > w[0].1.quality)
            .map(|w| w[1].0)
            .collect();
        assert!(raises.len() >= 3, "{changes:?}");
        for w in raises.windows(3) {
            assert!(
                w[2] - w[1] > w[1] - w[0],
                "raising does not wait longer: {raises:?}"
            );
        }
        let late = changes.iter().filter(|(t, _)| *t > 20.0).count();
        assert!(late <= 8, "{late} changes after 20 s: {changes:?}");
        assert!(c.level().quality <= 0.75);
    }

    #[test]
    fn pauses_and_single_spikes_change_nothing() {
        let mut c = Controller::new(BUDGET, 0.0);
        // One frame of 200 ms now and then, and a pause of 10 s.
        let load = |t: f64, l| {
            if ((t * 60.0).round() as u32).is_multiple_of(97) {
                0.2
            } else {
                cost(0.008, l)
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
            let ms = if i == 100 { 0.030 } else { 0.010 };
            if let Some(s) = report.frame(f64::from(i) / 60.0, ms) {
                summaries.push(s);
            }
        }
        assert_eq!(summaries.len(), 3);
        assert!((summaries[0].average - 0.010).abs() < 0.0002);
        assert_eq!(summaries[0].worst, 0.030);
        assert_eq!(summaries[1].worst, 0.010);
    }
}
