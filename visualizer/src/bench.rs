//! Benchmark mode (`RUNE_VISUALIZER_BENCH=<seconds per scene>`): the render thread goes through
//! every built-in scene in order and measures it, so that a run on a phone needs no hands.
//!
//! Each scene gets two phases of equal length. In the first the adaptive quality is off and the
//! scene stays at full level, which measures what it costs. In the second the adaptive quality is
//! on, starting from full, which shows where it settles. The first [`WARM_UP`] seconds of a phase
//! (shaders compile, the controller settles) are left out of its numbers. A phase ends with one
//! log line, and a pass over all the scenes with `Visualizer bench: cycle done`; then it starts
//! again.

use std::time::{Duration, Instant};

use log::{info, warn};

use crate::builtin;
use crate::pipeline::Pipeline;

/// Seconds at the start of a phase that do not count.
const WARM_UP: f64 = 1.0;

/// The seconds per scene asked for in the environment, if any.
pub fn seconds_from_env() -> Option<f64> {
    parse_seconds(std::env::var("RUNE_VISUALIZER_BENCH").ok().as_deref())
}

/// `value` as a number of seconds, if it is a positive one.
fn parse_seconds(value: Option<&str>) -> Option<f64> {
    value?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
}

/// The scenes of `names` that `wanted` (comma-separated names, any case) lists, or all of them
/// without a list.
fn only(names: impl Iterator<Item = String>, wanted: Option<&str>) -> Vec<String> {
    let wanted: Vec<String> = wanted
        .unwrap_or_default()
        .split(',')
        .map(|name| name.trim().to_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    names
        .filter(|name| wanted.is_empty() || wanted.contains(&name.to_lowercase()))
        .collect()
}

/// The GPU times of the frames of a phase, in seconds.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Summary {
    pub average: f64,
    pub median: f64,
    /// The time that 95 % of the frames stayed under.
    pub p95: f64,
    pub max: f64,
}

/// The average, median, 95th percentile (nearest rank) and maximum of `times`; all zero for none.
pub fn summarize(times: &[f64]) -> Summary {
    if times.is_empty() {
        return Summary::default();
    }
    let mut sorted = times.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = |share: f64| {
        sorted[((share * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len()) - 1]
    };
    Summary {
        average: sorted.iter().sum::<f64>() / sorted.len() as f64,
        median: rank(0.5),
        p95: rank(0.95),
        max: sorted[sorted.len() - 1],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Full,
    Adaptive,
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Phase::Full => "full",
            Phase::Adaptive => "adaptive",
        }
    }
}

/// Where the benchmark is: the scene and phase being measured and what it collected so far.
pub struct Bench {
    /// Seconds of one phase.
    phase_seconds: f64,
    scenes: Vec<String>,
    scene: usize,
    phase: Phase,
    /// Whether the scene of this phase loaded.
    loaded: bool,
    /// When the phase began; `None` before the benchmark has started.
    began: Option<Instant>,
    /// The GPU times (seconds) and the frames rendered since the warm-up ended.
    times: Vec<f64>,
    frames: u32,
    /// How long the frames Flutter took had waited from being published (seconds).
    ages: Vec<f64>,
    /// The frames taken by Flutter and skipped without a free target so far, when the warm-up
    /// ended and now.
    taken: Option<(u64, u64)>,
    skipped: Option<(u64, u64)>,
    /// Whether the log has said that the benchmark overrides the scene asked for.
    said_override: bool,
}

impl Bench {
    /// A benchmark that gives every built-in scene `seconds` (half for each phase).
    pub fn new(seconds: f64) -> Self {
        Self {
            phase_seconds: seconds / 2.0,
            scenes: only(
                builtin::scenes().into_iter().map(|s| s.name),
                std::env::var("RUNE_VISUALIZER_BENCH_SCENES")
                    .ok()
                    .as_deref(),
            ),
            scene: 0,
            phase: Phase::Full,
            loaded: false,
            began: None,
            times: Vec::new(),
            frames: 0,
            ages: Vec::new(),
            taken: None,
            skipped: None,
            said_override: false,
        }
    }

    /// Called before each frame: starts the benchmark, moves on to the next phase or scene when
    /// one is over, and puts the benchmark's scene back if something else loaded one. Whether the
    /// scene on screen changed.
    pub fn step(&mut self, pipeline: &mut Pipeline, now: Instant) -> bool {
        if self.scenes.is_empty() {
            return false;
        }
        let Some(began) = self.began else {
            info!(
                "Visualizer bench: start scenes={} seconds_per_scene={} timestamps={}",
                self.scenes.len(),
                self.phase_seconds * 2.0,
                if pipeline.has_timestamps() {
                    "yes"
                } else {
                    "no"
                }
            );
            pipeline.record_frame_times(true);
            return self.begin(pipeline, now, true);
        };
        if now.duration_since(began).as_secs_f64() >= self.phase_seconds {
            self.finish(pipeline, now);
            match self.phase {
                Phase::Full => {
                    self.phase = Phase::Adaptive;
                    return self.begin(pipeline, now, false);
                }
                Phase::Adaptive => {
                    self.scene += 1;
                    if self.scene == self.scenes.len() {
                        self.scene = 0;
                        info!("Visualizer bench: cycle done");
                    }
                    self.phase = Phase::Full;
                    return self.begin(pipeline, now, true);
                }
            }
        }
        if self.loaded && pipeline.preset_name() != self.scenes[self.scene] {
            if !self.said_override {
                self.said_override = true;
                info!("Visualizer bench: bench mode overrides the scene that was asked for");
            }
            return self.begin(pipeline, now, true);
        }
        false
    }

    /// Called after each frame with how many frames Flutter took so far and how long the one it
    /// took last had waited, if it took one since: keeps what the GPU timer reported, once the
    /// warm-up is over.
    pub fn frame(
        &mut self,
        pipeline: &mut Pipeline,
        now: Instant,
        taken: u64,
        age: Option<Duration>,
    ) {
        let times = pipeline.take_frame_times();
        let Some(began) = self.began else { return };
        if now.duration_since(began).as_secs_f64() >= WARM_UP {
            self.times.extend(times);
            self.ages.extend(age.map(|age| age.as_secs_f64()));
            self.frames += 1;
            let skipped = pipeline.skipped();
            self.taken.get_or_insert((taken, taken)).1 = taken;
            self.skipped.get_or_insert((skipped, skipped)).1 = skipped;
        }
    }

    /// Start the phase (`load`: with its scene loaded again, which the first phase of a scene and
    /// a scene that was replaced need). Whether the scene on screen changed.
    fn begin(&mut self, pipeline: &mut Pipeline, now: Instant, load: bool) -> bool {
        if load {
            let name = &self.scenes[self.scene];
            self.loaded = match pipeline.load_builtin_scene(name) {
                Ok(()) => true,
                Err(e) => {
                    warn!("Visualizer bench: scene {name} did not load: {e:#}");
                    false
                }
            };
        }
        pipeline.set_adaptive(self.phase == Phase::Adaptive);
        self.times.clear();
        self.frames = 0;
        self.ages.clear();
        self.taken = None;
        self.skipped = None;
        // The clock starts when loading is done.
        self.began = Some(if load { Instant::now() } else { now });
        load
    }

    /// Log the line of the phase that ended.
    fn finish(&mut self, pipeline: &mut Pipeline, now: Instant) {
        let began = self.began.unwrap_or(now);
        let measured = now.duration_since(began).as_secs_f64() - WARM_UP;
        let fps = if measured > 0.0 {
            f64::from(self.frames) / measured
        } else {
            0.0
        };
        let summary = summarize(&self.times);
        let ages = summarize(&self.ages);
        let count = |counts: Option<(u64, u64)>| counts.map_or(0, |(first, last)| last - first);
        let shown = if measured > 0.0 {
            count(self.taken) as f64 / measured
        } else {
            0.0
        };
        let (level, (width, height)) = pipeline.level();
        info!(
            "Visualizer bench: scene={} phase={} frames={} fps={:.1} shown_fps={shown:.1} wait_p50={:.1} wait_p95={:.1} skipped={} gpu_avg={:.1} gpu_p50={:.1} gpu_p95={:.1} gpu_max={:.1} factor={:.2} quality={:.2} size={width}x{height}",
            self.scenes[self.scene],
            self.phase.name(),
            self.frames,
            fps,
            ages.median * 1e3,
            ages.p95 * 1e3,
            count(self.skipped),
            summary.average * 1e3,
            summary.median * 1e3,
            summary.p95 * 1e3,
            summary.max * 1e3,
            level.factor,
            level.quality,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_must_be_a_positive_number() {
        assert_eq!(parse_seconds(Some("30")), Some(30.0));
        assert_eq!(parse_seconds(Some(" 7.5 ")), Some(7.5));
        for bad in [
            None,
            Some(""),
            Some("0"),
            Some("-3"),
            Some("abc"),
            Some("NaN"),
            Some("inf"),
        ] {
            assert_eq!(parse_seconds(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn scenes_can_be_picked() {
        let names = || {
            ["Growth", "CRT Wall", "Paperclip"]
                .map(String::from)
                .into_iter()
        };
        assert_eq!(only(names(), None).len(), 3);
        assert_eq!(only(names(), Some("")).len(), 3);
        assert_eq!(
            only(names(), Some("crt wall, growth")),
            ["Growth", "CRT Wall"]
        );
    }

    #[test]
    fn summary_of_frame_times() {
        assert_eq!(summarize(&[]), Summary::default());
        assert_eq!(
            summarize(&[0.003]),
            Summary {
                average: 0.003,
                median: 0.003,
                p95: 0.003,
                max: 0.003
            }
        );
        // 1 to 100 ms, shuffled: the nearest rank of 95 % of 100 is the 95th.
        let times: Vec<f64> = (0..100)
            .map(|i| f64::from((i * 37) % 100 + 1) / 1e3)
            .collect();
        let summary = summarize(&times);
        assert!((summary.average - 0.0505).abs() < 1e-9);
        assert!((summary.median - 0.050).abs() < 1e-9);
        assert!((summary.p95 - 0.095).abs() < 1e-9);
        assert!((summary.max - 0.100).abs() < 1e-9);
    }
}
