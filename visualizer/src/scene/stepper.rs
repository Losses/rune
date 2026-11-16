//! The decisions of a song with a chart, stepped by song time.
//!
//! With a chart the scene's [`Decider`] is not stepped by the frames the screen draws but by the
//! song: every frame takes the steps of [`STEP`] whose song time has come (none at a high frame
//! rate, a few after a slow frame), each heard from the chart alone with the song's seed. The
//! cuts, looks and moments then come at the same song times, and are the same, whatever the frame
//! rate, and whenever the scene was opened.
//!
//! Where the song time jumps (the scene opens on a song under way, a seek) the decider is put
//! where it would be by playing the song from its start, silently: from the latest snapshot
//! before that time, which are kept every ten seconds of song from such replays and from playing
//! on untouched, else from the start. What it skipped is not played: the screen jumps to the
//! shot on, without a transition or an effect.
//!
//! A chart that comes while the scene plays (the analysis of the song finished) does not jump:
//! the song's decider is stepped next to the one on screen until it cuts, and the screen takes it
//! over on that cut. A scene without shots, or with a shot a lab holds, never cuts: there the
//! song's decider takes over at once.

use std::sync::Arc;

use super::CutInfo;
use super::decider::{Decider, Decision, STEP, Settings};
use super::inputs::Levels;
use super::lens::LensFrame;
use super::manifest::Manifest;
use crate::chart::Chart;
use crate::inspect::LabEvent;
use crate::rhythm::Rhythm;

/// The seed of the song in the file at `path`: a hash of the path (FNV-1a), the same each time
/// the song plays and on every run.
pub fn song_seed(path: &str) -> u32 {
    path.bytes().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

/// A song's whole chart and the seed its decisions are made with (see [`song_seed`]).
#[derive(Clone)]
pub struct SongChart {
    pub chart: Arc<Chart>,
    pub seed: u32,
}

impl SongChart {
    fn is(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.chart, &other.chart) && self.seed == other.seed
    }
}

/// A snapshot of the decider is kept every this many steps: ten seconds of song.
const SNAPSHOT_EVERY: u32 = 600;
/// A song time up to this many steps behind the steps taken (a third of a second) is waited for
/// rather than taken for a seek: the song clock closes a gap to the player of up to its seek
/// threshold (0.3 s) by pulling back a little each frame, which adds up at a high frame rate.
const BEHIND: u32 = 20;
/// A song time up to this many steps ahead (a second) is stepped through, as after a slow frame;
/// further ahead is a seek.
const AHEAD: u32 = 60;

/// What a frame decided, stepped by song time.
pub struct Stepped {
    /// What the steps of the frame decided, the last of each kind: a cut, a look change, a
    /// moment, a glitch or mosaic. The lens is that of the last step taken, also in a frame that
    /// takes none.
    pub decision: Decision,
    /// The event log of the steps, each at its own song time.
    pub events: Vec<LabEvent>,
    /// The decider was put where the song is without playing what it skipped: the screen jumps to
    /// the shot on.
    pub jumped: bool,
    /// The decider is another one than in the last frame (a jump, or the song's own decider taking
    /// over on its cut): what it counted is not to be compared with what the last one did.
    pub replaced: bool,
    /// Song seconds since the time of the last step taken, by which what runs with time (a shot's
    /// age) is ahead of what the decider says.
    pub since_step: f64,
}

/// Steps a scene's decider by the song (see the module's doc).
#[derive(Default)]
pub struct Stepper {
    /// The song the decider on screen is stepped through, while it is.
    on: Option<SongChart>,
    /// How many steps it took: it decided the frames at song times 0, [`STEP`], … up to
    /// `steps - 1` steps.
    steps: u32,
    /// The decider before step n of the song, for the n kept, in order.
    snapshots: Vec<(u32, Decider)>,
    /// What the snapshots are of: the song, what a lab or the user set on the decider, and the
    /// version of the runtime's decisions.
    of: Option<(SongChart, Settings, u64)>,
    /// Whether the decider on screen is as the song alone made it, which a lab's command breaks
    /// (see [`Stepper::taint`]); only then are its snapshots kept.
    pure: bool,
    /// The song's own decider while another is on screen, with the steps it took.
    shadow: Option<(SongChart, Decider, u32)>,
    /// What the post pass was told by the last step.
    lens: LensFrame,
    /// Whether the event log of the steps is made (see [`Stepper::record`]).
    record: bool,
    /// Tests only: every step taken on screen, with what it decided.
    #[cfg(test)]
    decided: Vec<(u32, Decision)>,
}

/// How many steps are due at song time `time`: those at song times up to it.
fn steps_until(time: f64) -> u32 {
    (time.max(0.0) / STEP).floor() as u32 + 1
}

/// The live audio a step hears: nothing, since the chart decides (a test says the live audio would
/// change nothing).
fn unheard() -> (Rhythm, Levels) {
    (Rhythm::default(), Levels::default())
}

/// Takes what `step` decided into `frame`, where the last of each kind stands.
fn merge(frame: &mut Decision, step: Decision) {
    frame.cut = step.cut.or(frame.cut);
    frame.shot = step.shot;
    frame.look = step.look.or(frame.look);
    frame.moment = step.moment.or(frame.moment);
    frame.lens = step.lens;
    frame.overlay = step.overlay.or(frame.overlay);
}

impl Stepper {
    /// The frame at song time `time` of `song`, for `decider`, the decider of a scene of
    /// `manifest`. `forced` is a cut a lab made, taken by the first step of the frame; `first`
    /// tells the scene's first frame; `runtime` is the version of the runtime's decisions the
    /// decider reads. None while a chart that came during the song waits for its decider's cut:
    /// the caller decides that frame itself.
    #[allow(clippy::too_many_arguments)]
    pub fn frame(
        &mut self,
        decider: &mut Decider,
        manifest: &Manifest,
        song: &SongChart,
        time: f64,
        forced: &mut Option<CutInfo>,
        quality: f32,
        first: bool,
        runtime: u64,
    ) -> Option<Stepped> {
        self.check_snapshots(decider, song, runtime);
        let target = steps_until(time);
        if !self.on.as_ref().is_some_and(|on| on.is(song)) {
            if first {
                // Nothing on screen to keep: the scene opens where the song is.
                self.shadow = None;
                *forced = None;
                self.restore(decider, manifest, song, target, quality);
                return Some(self.jump(decider, time));
            }
            return self.follow(decider, manifest, song, target, time, forced, quality);
        }
        if target + BEHIND < self.steps || target > self.steps + AHEAD {
            // A cut a lab made is of the decider replaced.
            *forced = None;
            self.restore(decider, manifest, song, target, quality);
            return Some(self.jump(decider, time));
        }
        let (decision, events) = self.run(decider, song, target, forced, quality);
        Some(Stepped {
            decision,
            events,
            jumped: false,
            replaced: false,
            since_step: self.since_step(time),
        })
    }

    /// The song has no chart (or a lab holds the music without one): the caller decides frame by
    /// frame. The snapshots stay, for the chart that may come back.
    pub fn off(&mut self) {
        self.on = None;
        self.shadow = None;
    }

    /// Whether the steps make the event log, which only a lab shows: off, [`Stepped::events`] is
    /// empty and nothing is built for it.
    pub fn record(&mut self, on: bool) {
        self.record = on;
    }

    /// What `decision` of a step at song time `time` is for the event log, when it is kept.
    fn events(
        &self,
        decider: &Decider,
        time: f64,
        decision: &Decision,
        first: bool,
    ) -> Vec<LabEvent> {
        if self.record {
            decider.events(time, decision, first)
        } else {
            Vec::new()
        }
    }

    /// A lab changed the decider on screen by hand (a cut, an effect, a hold, an override): it is
    /// no longer what the song alone makes, and no snapshot is taken from it.
    pub fn taint(&mut self) {
        self.pure = false;
    }

    /// Drops the snapshots when they are of another song, or of the decider set otherwise.
    fn check_snapshots(&mut self, decider: &Decider, song: &SongChart, runtime: u64) {
        let settings = decider.settings();
        let same = self
            .of
            .as_ref()
            .is_some_and(|(of, of_settings, of_runtime)| {
                of.is(song) && *of_settings == settings && *of_runtime == runtime
            });
        if !same {
            self.snapshots.clear();
            self.of = Some((song.clone(), settings, runtime));
            self.pure = false;
            self.shadow = None;
        }
    }

    /// Takes the steps of `song` due up to `target` (not counting) with the decider on screen.
    fn run(
        &mut self,
        decider: &mut Decider,
        song: &SongChart,
        target: u32,
        forced: &mut Option<CutInfo>,
        quality: f32,
    ) -> (Decision, Vec<LabEvent>) {
        let mut frame = self.standing(decider);
        let mut events = Vec::new();
        while self.steps < target {
            if self.pure {
                self.keep(self.steps, decider);
            }
            let step = self.steps;
            let decision = decider.step_at(
                &song.chart,
                step,
                song.seed,
                unheard(),
                forced.take(),
                quality,
            );
            #[cfg(test)]
            self.decided.push((step, decision.clone()));
            events.extend(self.events(decider, f64::from(step) * STEP, &decision, false));
            merge(&mut frame, decision);
            self.steps += 1;
        }
        self.lens = frame.lens;
        (frame, events)
    }

    /// The song's own decider, stepped next to the one on screen until it cuts; then it is put on
    /// screen and the frame reports its cut. Where the decider on screen cannot cut (no shots, or a
    /// held shot) it is put on screen at once.
    #[allow(clippy::too_many_arguments)]
    fn follow(
        &mut self,
        decider: &mut Decider,
        manifest: &Manifest,
        song: &SongChart,
        target: u32,
        time: f64,
        forced: &mut Option<CutInfo>,
        quality: f32,
    ) -> Option<Stepped> {
        let (mut shadow, mut steps) = match self.shadow.take() {
            Some((of, shadow, steps))
                if of.is(song) && target + BEHIND >= steps && target <= steps + AHEAD =>
            {
                (shadow, steps)
            }
            // Started now, or the song time jumped: from where the song is.
            _ => (
                self.replay(decider, manifest, song, target, quality),
                target,
            ),
        };
        let cuts = decider
            .director()
            .is_some_and(|_| decider.settings().shot.is_none());
        if !cuts {
            // No cut to wait for: the song's decider takes over now, with the look it is on.
            while steps < target {
                self.lens = shadow
                    .step_at(&song.chart, steps, song.seed, unheard(), None, quality)
                    .lens;
                steps += 1;
            }
            *decider = shadow;
            self.on = Some(song.clone());
            self.steps = steps;
            self.pure = true;
            let decision = Decision {
                look: decider.look_on(),
                ..self.standing(decider)
            };
            return Some(Stepped {
                events: self.events(decider, time, &decision, false),
                decision,
                jumped: false,
                replaced: true,
                since_step: self.since_step(time),
            });
        }
        while steps < target {
            let decision = shadow.step_at(&song.chart, steps, song.seed, unheard(), None, quality);
            let step = steps;
            steps += 1;
            if decision.cut.is_none() {
                continue;
            }
            // The song's decider cut: it takes over with this cut, and the screen hears of the
            // look it is on.
            *decider = shadow;
            self.on = Some(song.clone());
            self.steps = steps;
            self.pure = true;
            #[cfg(test)]
            self.decided.push((steps - 1, decision.clone()));
            let at = f64::from(step) * STEP;
            let mut events = self.events(decider, at, &decision, false);
            let mut frame = decision;
            if frame.look.is_none()
                && let Some(look) = decider.look_on()
            {
                // The look the song's decider is on is told with the cut.
                frame.look = Some(look);
                let told = Decision {
                    cut: None,
                    look: Some(look),
                    moment: None,
                    overlay: None,
                    ..frame.clone()
                };
                events.extend(self.events(decider, at, &told, false));
            }
            let (rest, more) = self.run(decider, song, target, forced, quality);
            merge(&mut frame, rest);
            events.extend(more);
            self.lens = frame.lens;
            return Some(Stepped {
                decision: frame,
                events,
                jumped: false,
                replaced: true,
                since_step: self.since_step(time),
            });
        }
        self.shadow = Some((song.clone(), shadow, steps));
        None
    }

    /// Puts `decider` where `song` is before step `target`, silently (see [`Stepper::replay`]).
    fn restore(
        &mut self,
        decider: &mut Decider,
        manifest: &Manifest,
        song: &SongChart,
        target: u32,
        quality: f32,
    ) {
        *decider = self.replay(decider, manifest, song, target, quality);
        self.on = Some(song.clone());
        self.steps = target;
        self.pure = true;
    }

    /// A decider set like `decider` that played `song` up to step `target` (not counting), from the
    /// latest snapshot before it or else from the start, keeping snapshots on the way. Nothing it
    /// decides on the way is reported.
    fn replay(
        &mut self,
        decider: &Decider,
        manifest: &Manifest,
        song: &SongChart,
        target: u32,
        quality: f32,
    ) -> Decider {
        let (mut steps, mut replayed) =
            match self.snapshots.iter().rev().find(|(s, _)| *s <= target) {
                Some((steps, snapshot)) => (*steps, snapshot.clone()),
                None => (0, decider.fresh(manifest)),
            };
        while steps < target {
            self.keep(steps, &replayed);
            self.lens = replayed
                .step_at(&song.chart, steps, song.seed, unheard(), None, quality)
                .lens;
            steps += 1;
        }
        replayed
    }

    /// Keeps a snapshot of `decider` before step `steps`, when one is due there and not kept yet.
    fn keep(&mut self, steps: u32, decider: &Decider) {
        if !steps.is_multiple_of(SNAPSHOT_EVERY) {
            return;
        }
        if let Err(at) = self.snapshots.binary_search_by_key(&steps, |(s, _)| *s) {
            self.snapshots.insert(at, (steps, decider.clone()));
        }
    }

    /// The frame of a jump: the shot and the look the decider is on, as the scene's first frame
    /// tells them, and nothing else.
    fn jump(&self, decider: &Decider, time: f64) -> Stepped {
        let decision = Decision {
            look: decider.look_on(),
            ..self.standing(decider)
        };
        Stepped {
            events: self.events(decider, time, &decision, true),
            decision,
            jumped: true,
            replaced: true,
            since_step: self.since_step(time),
        }
    }

    /// A frame in which nothing is decided.
    fn standing(&self, decider: &Decider) -> Decision {
        Decision {
            cut: None,
            shot: decider.shot_index(),
            look: None,
            moment: None,
            lens: self.lens,
            overlay: None,
        }
    }

    fn since_step(&self, time: f64) -> f64 {
        let last = f64::from(self.steps.saturating_sub(1)) * STEP;
        (time - last).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    impl Stepper {
        fn snapshot_steps(&self) -> Vec<u32> {
            self.snapshots.iter().map(|(s, _)| *s).collect()
        }
    }

    use super::*;
    use crate::scene::MusicValues;
    use crate::scene::Pace;
    use crate::scene::fixtures;

    /// A quiet section, a high one the director bursts into, and a quieter one: 100 s.
    const SECTIONS: [(f64, f64, f64); 3] =
        [(0.0, 30.0, 0.2), (30.0, 70.0, 0.9), (70.0, 100.0, 0.3)];
    const SONG_STEPS: u32 = 6000;

    fn song() -> SongChart {
        SongChart {
            chart: Arc::new(fixtures::chart(&SECTIONS)),
            seed: song_seed("/music/a.flac"),
        }
    }

    /// What a fresh decider of the scene decides at each step of `song`, stepped from its start
    /// with the live audio `live` gives at each step.
    fn reference_with(song: &SongChart, live: impl Fn(u32) -> (Rhythm, Levels)) -> Vec<Decision> {
        let mut decider = fixtures::decider();
        (0..SONG_STEPS)
            .map(|step| decider.step_at(&song.chart, step, song.seed, live(step), None, 1.0))
            .collect()
    }

    fn reference(song: &SongChart) -> Vec<Decision> {
        reference_with(song, |_| unheard())
    }

    /// Every step the screen took decided what the fresh decider did at that step; the steps taken.
    fn check(stepper: &Stepper, reference: &[Decision]) -> Vec<u32> {
        for (step, decision) in &stepper.decided {
            assert_eq!(decision, &reference[*step as usize], "step {step}");
        }
        stepper.decided.iter().map(|(step, _)| *step).collect()
    }

    /// Plays `song` on screen through these song times, one frame each, the first being the
    /// scene's first; the song times of the frames that jumped.
    fn play(
        stepper: &mut Stepper,
        decider: &mut Decider,
        song: &SongChart,
        times: &[f64],
    ) -> Vec<f64> {
        let manifest = fixtures::manifest();
        let mut jumps = Vec::new();
        for (i, &t) in times.iter().enumerate() {
            let stepped = stepper
                .frame(decider, &manifest, song, t, &mut None, 1.0, i == 0, 0)
                .expect("stepped");
            if stepped.jumped {
                jumps.push(t);
            }
        }
        jumps
    }

    #[test]
    fn events_are_made_only_while_they_are_asked_for() {
        let song = song();
        let manifest = fixtures::manifest();
        let events_of = |record: bool| {
            let mut stepper = Stepper::default();
            stepper.record(record);
            let mut decider = fixtures::decider();
            let mut count = 0;
            // The first frame tells the shot the scene opens on; the frames that follow cross the
            // cuts of the first section.
            for (i, time) in [0.0, 5.0, 10.0, 20.0, 29.0].into_iter().enumerate() {
                let stepped = stepper
                    .frame(
                        &mut decider,
                        &manifest,
                        &song,
                        time,
                        &mut None,
                        1.0,
                        i == 0,
                        0,
                    )
                    .expect("stepped");
                count += stepped.events.len();
            }
            count
        };
        assert!(events_of(true) > 0);
        assert_eq!(events_of(false), 0);
    }

    /// Song times from `from` to `to` at a frame rate that wanders between 30 and 200 fps.
    fn jittery(from: f64, to: f64) -> Vec<f64> {
        let mut times = Vec::new();
        let mut t = from;
        let mut i = 0u32;
        while t < to {
            times.push(t);
            let wander = (f64::from(i) * 12.9898).sin().abs();
            t += 1.0 / 200.0 + wander * (1.0 / 30.0 - 1.0 / 200.0);
            i += 1;
        }
        times
    }

    /// The steps from the one after song time `from` to the one at `to`.
    fn steps_between(from: f64, to: f64) -> Vec<u32> {
        (steps_until(from)..steps_until(to)).collect()
    }

    #[test]
    fn the_seed_follows_the_song() {
        assert_eq!(song_seed(""), 0x811c_9dc5);
        assert_eq!(song_seed("a"), 0xe40c_292c);
        assert_ne!(song_seed("/music/a.flac"), song_seed("/music/b.flac"));
    }

    #[test]
    fn the_live_audio_decides_nothing_where_the_song_has_a_chart() {
        let song = song();
        // Kicks, energy and bands that change every step.
        let noisy = |step: u32| {
            let t = f64::from(step) * STEP;
            let h = |k: f64| ((t * 1000.0 + k).sin() * 43758.5453).fract().abs();
            (
                Rhythm {
                    bpm: 97.0,
                    beat: t * 1.6,
                    confidence: 0.9,
                    kick: h(1.0),
                    snare: h(2.0),
                    hat: h(3.0),
                    energy: 2.0 * h(4.0),
                },
                Levels {
                    bass: (3.0 * h(5.0)) as f32,
                    mids: (3.0 * h(6.0)) as f32,
                    treble: (3.0 * h(7.0)) as f32,
                    feel: 6.0,
                    song_feel: 6.0,
                },
            )
        };
        let quiet = reference(&song);
        assert!(quiet.iter().filter(|d| d.cut.is_some()).count() > 5);
        assert_eq!(quiet, reference_with(&song, noisy));
    }

    #[test]
    fn the_screen_decides_what_the_song_does_at_any_frame_rate() {
        let song = song();
        let mut decider = fixtures::decider();
        let mut stepper = Stepper::default();
        let times = jittery(0.0, 100.0);
        assert_eq!(play(&mut stepper, &mut decider, &song, &times), [0.0]);
        let steps = check(&stepper, &reference(&song));
        // Every step after the first, which the opening took silently.
        assert_eq!(steps, steps_between(0.0, *times.last().unwrap()));
    }

    #[test]
    fn a_scene_opened_on_a_song_under_way_goes_on_as_if_it_had_played_it_all() {
        let song = song();
        let mut decider = fixtures::decider();
        let mut stepper = Stepper::default();
        let times = jittery(47.3, 100.0);
        assert_eq!(play(&mut stepper, &mut decider, &song, &times), [47.3]);
        let steps = check(&stepper, &reference(&song));
        assert_eq!(steps, steps_between(47.3, *times.last().unwrap()));
        // Snapshots are kept every ten seconds of song: from the replay up to the opening, then
        // from playing on.
        let every_ten: Vec<u32> = (0..10).map(|i| i * 600).collect();
        assert_eq!(stepper.snapshot_steps(), every_ten);
    }

    #[test]
    fn a_seek_jumps_and_the_song_goes_on_from_there_as_it_would() {
        let song = song();
        let mut decider = fixtures::decider();
        let mut stepper = Stepper::default();
        let segments = [
            jittery(0.0, 60.0),
            jittery(21.0, 40.0),
            jittery(80.0, 100.0),
        ];
        let jumps = play(&mut stepper, &mut decider, &song, &segments.concat());
        assert_eq!(jumps, [0.0, 21.0, 80.0]);
        let steps = check(&stepper, &reference(&song));
        let expected: Vec<u32> = segments
            .iter()
            .flat_map(|times| steps_between(times[0], *times.last().unwrap()))
            .collect();
        assert_eq!(steps, expected);
    }

    #[test]
    fn a_song_clock_that_pulls_back_a_little_is_waited_for() {
        let song = song();
        let manifest = fixtures::manifest();
        let mut decider = fixtures::decider();
        let mut stepper = Stepper::default();
        let mut frame = |t: f64, first: bool| {
            stepper
                .frame(&mut decider, &manifest, &song, t, &mut None, 1.0, first, 0)
                .unwrap()
        };
        frame(10.0, true);
        frame(10.5, false);
        let back = frame(10.45, false);
        assert!(!back.jumped && back.decision.cut.is_none());
        assert!(!frame(10.52, false).jumped);
        // Up to a third of a second back is waited for too.
        assert!(!frame(10.22, false).jumped);
        assert!(frame(10.0, false).jumped);
        assert!(frame(12.0, false).jumped);
    }

    #[test]
    fn a_chart_that_comes_during_the_song_takes_over_on_its_own_cut() {
        let song = song();
        let reference = reference(&song);
        let manifest = fixtures::manifest();
        let mut decider = fixtures::decider();
        let mut stepper = Stepper::default();
        // The scene has been on for a while, deciding by itself.
        let mut times = jittery(5.0, 100.0).into_iter();
        let first = times.next().unwrap();
        assert!(
            stepper
                .frame(
                    &mut decider,
                    &manifest,
                    &song,
                    first,
                    &mut None,
                    1.0,
                    false,
                    0
                )
                .is_none()
        );
        let next_cut = (steps_until(first)..SONG_STEPS)
            .find(|&step| reference[step as usize].cut.is_some())
            .expect("a cut");
        let mut took_over = false;
        for t in times.by_ref() {
            if let Some(stepped) =
                stepper.frame(&mut decider, &manifest, &song, t, &mut None, 1.0, false, 0)
            {
                assert!(stepped.replaced && !stepped.jumped);
                assert!(stepped.decision.cut.is_some());
                took_over = true;
                break;
            }
        }
        assert!(took_over);
        // It took over on that cut, and from then on it is the song's decider.
        let later: Vec<f64> = times.collect();
        for &t in &later {
            let stepped = stepper
                .frame(&mut decider, &manifest, &song, t, &mut None, 1.0, false, 0)
                .expect("stepped");
            assert!(!stepped.jumped && !stepped.replaced);
        }
        let steps = check(&stepper, &reference);
        assert_eq!(steps[0], next_cut);
        assert_eq!(
            steps,
            (next_cut..steps_until(*later.last().unwrap())).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_chart_that_comes_while_a_lab_holds_a_shot_takes_over_at_once() {
        let song = song();
        let manifest = fixtures::manifest();
        let mut decider = fixtures::decider();
        decider.hold_shot(1, &MusicValues::default());
        let mut stepper = Stepper::default();
        let mut frame = |decider: &mut Decider, t: f64| {
            stepper.frame(decider, &manifest, &song, t, &mut None, 1.0, false, 0)
        };
        let stepped = frame(&mut decider, 12.0).expect("taken over");
        assert!(stepped.replaced && !stepped.jumped && stepped.decision.cut.is_none());
        assert_eq!(decider.shot_index(), 1);
        // The song's decider is on screen from then on.
        let next = frame(&mut decider, 12.02).expect("stepped");
        assert!(!next.replaced);
    }

    #[test]
    fn a_change_of_what_the_decider_is_set_to_drops_the_snapshots() {
        let song = song();
        let manifest = fixtures::manifest();
        let mut decider = fixtures::decider();
        let mut stepper = Stepper::default();
        let frame = |stepper: &mut Stepper, decider: &mut Decider, t: f64, runtime: u64| {
            stepper.frame(
                decider,
                &manifest,
                &song,
                t,
                &mut None,
                1.0,
                t == 25.0,
                runtime,
            );
        };
        frame(&mut stepper, &mut decider, 25.0, 0);
        assert_eq!(stepper.snapshot_steps(), [0, 600, 1200]);
        decider
            .director_mut()
            .unwrap()
            .set_pace_override(Some(Pace::Calm));
        frame(&mut stepper, &mut decider, 25.02, 0);
        assert!(stepper.snapshot_steps().is_empty());
        // So does a new version of the runtime's decisions.
        frame(&mut stepper, &mut decider, 5.0, 0);
        assert_eq!(stepper.snapshot_steps(), [0]);
        frame(&mut stepper, &mut decider, 5.02, 1);
        assert!(stepper.snapshot_steps().is_empty());
    }

    #[test]
    fn a_decider_a_lab_changed_by_hand_keeps_no_snapshot() {
        let song = song();
        let manifest = fixtures::manifest();
        let mut decider = fixtures::decider();
        let mut stepper = Stepper::default();
        stepper.frame(&mut decider, &manifest, &song, 1.0, &mut None, 1.0, true, 0);
        stepper.taint();
        for t in jittery(1.0, 25.0) {
            stepper.frame(&mut decider, &manifest, &song, t, &mut None, 1.0, false, 0);
        }
        assert_eq!(stepper.snapshot_steps(), [0]);
    }
}
