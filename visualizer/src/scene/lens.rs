//! The lens effects of the built-in post-processing (`flare`, `leak`; see "Shared effects" in
//! `docs/music.md`). Each is an effect that is started at a moment with a strength and runs its
//! course (streaks or a glow that rise slowly from nothing and thin out again over two bars);
//! none is a constant layer. [`Lens::tick`] makes the small uniform `post.wgsl` reads, and a
//! scene that lists none gets a uniform of zeros, which costs the post pass nothing.
//!
//! What starts them is the shared scheduler of the moments (`crate::moments`): at a moment that
//! plays an effect and has no cut, [`Lens::start_at`] starts one of those the scene listed. A
//! scene lab starts them by hand.
//!
//! A glitch or a mosaic the scene lists plays here too, over the live picture, but the
//! transition layer draws it (with the functions of the transitions themselves, see
//! `crate::transition::Overlay`): this only says that one starts ([`Lens::take_overlay`]).

use super::inputs::MusicValues;
use crate::inspect::LabLens;
use crate::moments::{self, Played, Strength};
use crate::switching::FOLLOWED;
use crate::transition::Overlay;

/// How strong an effect a medium moment starts (a strong moment's is at full strength).
pub const MEDIUM_STRENGTH: f32 = 0.6;
/// A flare and a light leak each run a long, gentle ramp of this many beats (two bars; 4 s while
/// the beat is not followed): from nothing they rise with an eased curve to their peak a third of
/// the way in ([`RAMP_PEAK`]), then thin out slowly and are gone at the end.
pub const RAMP_BEATS: f32 = 8.0;
pub const RAMP_PEAK: f32 = 1.0 / 3.0;
/// The flare's intensity at the peak of its ramp: subtle, as it lasts.
pub const FLARE_PEAK: f32 = 0.8;
/// How far the flare's streaks reach at the start and the peak of its ramp: the weight of each
/// wider level of the streaks' pyramid against the one before (see `post.wgsl`).
pub const FLARE_STRETCH_FROM: f32 = 0.5;
pub const FLARE_STRETCH: f32 = 0.85;
/// How far a leak's source wanders along its edge over the whole leak, as a share of the edge.
const LEAK_DRIFT: f32 = 0.18;
/// The radius of a light leak's glow, as a share of the frame's long side.
const LEAK_SIZE: f32 = 0.5;
/// How far beyond its corner a light leak's source sits at the peak of its ramp, and how much
/// further out at its start and end (in uv), so that it reaches in and draws back.
const LEAK_OUTSIDE: f32 = 0.05;
const LEAK_REACH: f32 = 0.25;
/// The seconds of a beat while the beat is not followed (120 bpm), so that durations in beats
/// are fixed seconds then.
pub const FALLBACK_BEAT_SECONDS: f32 = 0.5;
/// The flare is drawn from this quality (`frame.quality`) up: it is lowered first.
pub const FLARE_MIN_QUALITY: f32 = 0.5;

/// The size of [`LensFrame`] in the uniform: eight f32.
pub const LENS_SIZE: u64 = 32;

/// What the post pass is told for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LensFrame {
    /// The intensity of the flare, 0 (off) and up.
    pub flare: f32,
    /// How far its streaks reach (see [`FLARE_STRETCH`]).
    pub flare_stretch: f32,
    /// 1 when the frame before drew the flare, so that its highlights are followed from it
    /// (set by the post pass, which knows).
    pub flare_history: f32,
    /// How much of the light leak is in, 0 to 1.
    pub leak: f32,
    /// Where the leak's source is (in uv, on or just beyond an edge), and its radius as a share
    /// of the frame's long side.
    pub leak_at: [f32; 2],
    pub leak_size: f32,
    /// A number that grows over the leak, which the shader turns into slow changes of shape.
    pub leak_phase: f32,
}

impl LensFrame {
    /// Whether the post pass has anything to do beyond its normal work.
    #[cfg(test)]
    pub fn is_idle(&self) -> bool {
        self.flare <= 0.0 && self.leak <= 0.0
    }

    /// The uniform `struct Lens` of `post.wgsl`.
    pub fn bytes(&self) -> [u8; LENS_SIZE as usize] {
        let mut bytes = [0; LENS_SIZE as usize];
        let values = [
            self.flare,
            self.flare_stretch,
            self.flare_history,
            self.leak,
            self.leak_at[0],
            self.leak_at[1],
            self.leak_size,
            self.leak_phase,
        ];
        for (chunk, v) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(values) {
            *chunk = v.to_le_bytes();
        }
        bytes
    }
}

/// What a scene lab shows of the lens effects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LensView {
    /// The scene lists the effect (a lab can start one by hand all the same).
    pub flare_listed: bool,
    pub leak_listed: bool,
    /// The effect is under way.
    pub flare_running: bool,
    pub leak_running: bool,
    /// The glitch and the mosaic over the picture: listed (they are under way while the
    /// transition layer draws them, which it tells).
    pub glitch_listed: bool,
    pub mosaic_listed: bool,
}

/// An effect under way on the ramp: how strong it was started, and its age in beats.
#[derive(Clone, Copy, Debug)]
struct Run {
    strength: f32,
    age: f32,
}

impl Run {
    fn new(strength: f32) -> Self {
        Self {
            strength: strength.clamp(0.0, 1.0),
            age: 0.0,
        }
    }

    /// How much of the effect is on, 0 to its strength.
    fn level(&self) -> f32 {
        self.strength * ramp(self.age / RAMP_BEATS)
    }

    /// The run started again at `strength` while this one is under way: it carries on rising
    /// (at the stronger of the two strengths) from the level it has now, back on the rising side
    /// of the ramp, so it swells again without a jump and two never add up.
    fn restarted(self, strength: f32) -> Self {
        let strength = strength.clamp(0.0, 1.0).max(self.strength);
        let share = (self.level() / strength.max(1e-6)).min(1.0);
        Self {
            strength,
            age: unsmooth(share) * RAMP_PEAK * RAMP_BEATS,
        }
    }
}

/// A light leak under way, which comes from a corner.
#[derive(Clone, Copy, Debug)]
struct Leak {
    run: Run,
    /// Beats since it started, which a gentle restart does not set back: what its drift follows.
    drift: f32,
    /// The corner it comes from, 0 to 3, and the way it drifts along its edge (1 or -1).
    corner: u32,
    along: f32,
}

/// The lens effects of one scene.
pub struct Lens {
    /// The effects that start at a moment: those the scene listed and those a lab forced.
    flare_listed: bool,
    leak_listed: bool,
    glitch_listed: bool,
    mosaic_listed: bool,
    flare: Option<Run>,
    leak: Option<Leak>,
    /// How many leaks started, which picks the corner of the next.
    leaks: u32,
    /// What the scene itself listed, which a lab's forcing adds to and takes away from again.
    own: LabLens,
    /// What a lab forced on (see [`Lens::set_forced`]).
    forced: LabLens,
    /// A glitch or a mosaic over the picture that starts with the next frame, for the transition
    /// layer to take (see [`Lens::take_overlay`]).
    overlay: Option<Overlay>,
}

impl Lens {
    /// For a scene whose `effects.lens` is `listed` (names the runtime knows; others are ignored).
    pub fn new(listed: &[String]) -> Self {
        let has = |name: &str| listed.iter().any(|l| l == name);
        Self {
            flare_listed: has("flare"),
            leak_listed: has("leak"),
            glitch_listed: has("glitch"),
            mosaic_listed: has("mosaic"),
            flare: None,
            leak: None,
            leaks: 0,
            own: LabLens {
                flare: has("flare"),
                leak: has("leak"),
                glitch: has("glitch"),
                mosaic: has("mosaic"),
            },
            forced: LabLens::default(),
            overlay: None,
        }
    }

    /// Start the effects of `forced` at every moment too, as if the scene listed them; with none
    /// on, only the scene's own list again.
    pub fn set_forced(&mut self, forced: LabLens) {
        self.forced = forced;
        self.flare_listed = self.own.flare || forced.flare;
        self.leak_listed = self.own.leak || forced.leak;
        self.glitch_listed = self.own.glitch || forced.glitch;
        self.mosaic_listed = self.own.mosaic || forced.mosaic;
    }

    /// Start a glitch (or a mosaic) over the picture now: the transition layer takes it with the
    /// next frame (see [`Lens::take_overlay`]). One at a time: one started later wins.
    pub fn start_overlay(&mut self, kind: Overlay) {
        self.overlay = Some(kind);
    }

    /// The glitch or mosaic over the picture that starts with this frame, if one does, once.
    pub fn take_overlay(&mut self) -> Option<Overlay> {
        self.overlay.take()
    }

    /// Start a flare of `strength` (0 to 1) now. One at a time: one under way swells again from
    /// where it is (see [`Run::restarted`]).
    pub fn start_flare(&mut self, strength: f32) {
        self.flare = Some(match self.flare {
            Some(run) => run.restarted(strength),
            None => Run::new(strength),
        });
    }

    /// Start a light leak of `strength` (0 to 1) now, from the next corner in turn. One under
    /// way swells again from where it is, from its corner (see [`Run::restarted`]).
    pub fn start_leak(&mut self, strength: f32) {
        if let Some(leak) = &mut self.leak {
            leak.run = leak.run.restarted(strength);
            return;
        }
        let corner = self.leaks.wrapping_mul(0x9e37_79b9).rotate_left(7) >> 30;
        self.leaks = self.leaks.wrapping_add(1);
        self.leak = Some(Leak {
            run: Run::new(strength),
            drift: 0.0,
            corner,
            along: if corner.is_multiple_of(2) { 1.0 } else { -1.0 },
        });
    }

    /// Keep a flare at its full swell from now on, as long as this is called before each frame
    /// (a probe's look at a steady flare).
    #[cfg(test)]
    pub fn hold_flare(&mut self) {
        self.flare = Some(Run {
            strength: 1.0,
            age: RAMP_PEAK * RAMP_BEATS,
        });
    }

    /// The names of the effects that start at a moment: those the scene listed and those a lab
    /// forced.
    fn listed(&self) -> Vec<String> {
        [
            (self.flare_listed, "flare"),
            (self.leak_listed, "leak"),
            (self.glitch_listed, "glitch"),
            (self.mosaic_listed, "mosaic"),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, name)| name.to_string())
        .collect()
    }

    /// Start the effect a moment that `played` picks among those listed (see
    /// [`moments::lens_effect`]), at full strength for a strong moment; nothing if the scene
    /// lists none that suits the moment.
    pub fn start_at(&mut self, played: Played) {
        let strength = match played.strength {
            Strength::Strong => 1.0,
            Strength::Medium => MEDIUM_STRENGTH,
        };
        match moments::lens_effect(&self.listed(), played) {
            Some(moments::Lens::Flare) => self.start_flare(strength),
            Some(moments::Lens::Leak) => self.start_leak(strength),
            Some(moments::Lens::Glitch) => self.start_overlay(Overlay::Glitch),
            Some(moments::Lens::Mosaic) => self.start_overlay(Overlay::Mosaic),
            None => {}
        }
    }

    /// Start every effect a lab forced on, at a moment that `played` (what "on at every moment"
    /// means in a lab: the forced ones play, whatever the scene lists or the cut does). Whether
    /// any started.
    pub fn start_forced(&mut self, played: Played) -> bool {
        let strength = match played.strength {
            Strength::Strong => 1.0,
            Strength::Medium => MEDIUM_STRENGTH,
        };
        let forced = self.forced;
        if forced.flare {
            self.start_flare(strength);
        }
        if forced.leak {
            self.start_leak(strength);
        }
        // The glitch and the mosaic are strong: only at a strong moment; one at a time, the
        // mosaic if both are forced.
        let strong = played.strength == Strength::Strong;
        if strong && forced.glitch {
            self.start_overlay(Overlay::Glitch);
        }
        if strong && forced.mosaic {
            self.start_overlay(Overlay::Mosaic);
        }
        forced.flare || forced.leak || (strong && (forced.glitch || forced.mosaic))
    }

    /// What a lab forced on, to give a rebuilt scene.
    pub fn forced(&self) -> LabLens {
        self.forced
    }

    pub fn view(&self) -> LensView {
        LensView {
            flare_listed: self.flare_listed,
            leak_listed: self.leak_listed,
            flare_running: self.flare.is_some(),
            leak_running: self.leak.is_some(),
            glitch_listed: self.glitch_listed,
            mosaic_listed: self.mosaic_listed,
        }
    }

    /// Advance by one frame of `dt` seconds with the music of the frame and the quality the scene
    /// draws with (see `frame.quality`), and say what the post pass does in it.
    pub fn tick(&mut self, dt: f32, music: &MusicValues, quality: f32) -> LensFrame {
        let beat = if music.locked > FOLLOWED && music.tempo > 0.0 {
            60.0 / music.tempo
        } else {
            FALLBACK_BEAT_SECONDS
        };
        let beats = dt / beat;

        let mut frame = LensFrame::default();

        // The flare: its intensity and the reach of its streaks follow the ramp. It gives way to
        // a low quality.
        if let Some(run) = &mut self.flare {
            if run.age >= RAMP_BEATS {
                self.flare = None;
            } else {
                if quality >= FLARE_MIN_QUALITY {
                    let shape = ramp(run.age / RAMP_BEATS);
                    frame.flare = FLARE_PEAK * run.level();
                    frame.flare_stretch =
                        FLARE_STRETCH_FROM + (FLARE_STRETCH - FLARE_STRETCH_FROM) * shape;
                }
                run.age += beats;
            }
        }

        // The leak: its strength, its size (in the shader) and how far its source reaches in
        // from beyond its corner follow the ramp, while the source drifts along its edge.
        if let Some(leak) = &mut self.leak {
            if leak.run.age >= RAMP_BEATS {
                self.leak = None;
            } else {
                frame.leak = leak.run.level();
                let shape = ramp(leak.run.age / RAMP_BEATS);
                let outside = LEAK_OUTSIDE + LEAK_REACH * (1.0 - shape);
                let share = (leak.drift / RAMP_BEATS).min(1.5);
                let wander = LEAK_DRIFT * share;
                // The source sits beyond the corner and wanders along its edge.
                let (x, y) = match leak.corner {
                    0 => (-outside, 0.1 + wander * leak.along),
                    1 => (0.1 + wander, -outside),
                    2 => (1.0 + outside, 0.9 - wander),
                    _ => (0.9 - wander, 1.0 + outside),
                };
                frame.leak_at = [x, y];
                frame.leak_size = LEAK_SIZE;
                frame.leak_phase = share * std::f32::consts::TAU;
                leak.run.age += beats;
                leak.drift += beats;
            }
        }

        frame
    }
}

/// The ramp at `x` of its length: 0 at the start, a smoothstep up to 1 at [`RAMP_PEAK`], and a
/// smoothstep down to 0 at the end; 0 outside.
fn ramp(x: f32) -> f32 {
    if !(0.0..1.0).contains(&x) {
        return 0.0;
    }
    if x < RAMP_PEAK {
        smooth(x / RAMP_PEAK)
    } else {
        smooth((1.0 - x) / (1.0 - RAMP_PEAK))
    }
}

/// Where a smoothstep from 0 to 1 reaches `y`.
fn unsmooth(y: f32) -> f32 {
    let y = y.clamp(0.0, 1.0);
    0.5 - ((1.0 - 2.0 * y).asin() / 3.0).sin()
}

/// Smoothstep from 0 to 1.
fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn music(cue: f32, cues: u32) -> MusicValues {
        MusicValues {
            tempo: 120.0,
            locked: 1.0,
            cue,
            cues,
            ..MusicValues::default()
        }
    }

    fn lens(names: &[&str]) -> Lens {
        let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        Lens::new(&names)
    }

    /// Run `seconds` of frames at 60 fps with no cue; the largest of each value is kept.
    fn run(lens: &mut Lens, seconds: f32) -> LensFrame {
        let mut most = LensFrame::default();
        for _ in 0..(seconds * 60.0) as u32 {
            let f = lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0);
            most.flare = most.flare.max(f.flare);
            most.leak = most.leak.max(f.leak);
        }
        most
    }

    /// A scene that lists nothing gets nothing, however strong the cue.
    #[test]
    fn nothing_is_on_by_default() {
        let mut lens = lens(&[]);
        let frame = lens.tick(1.0 / 60.0, &music(1.0, 1), 1.0);
        assert!(frame.is_idle());
        assert_eq!(lens.view(), LensView::default());
        assert!(run(&mut lens, 5.0).is_idle());
        assert_eq!(frame.bytes(), [0; LENS_SIZE as usize]);
    }

    /// A lab's forcing makes a scene act as if it listed the effect, and taking it away leaves what
    /// the scene itself lists.
    #[test]
    fn forcing_an_effect_adds_to_the_scene_list_and_none_restores_it() {
        let mut lens = lens(&["leak"]);
        let flare = LabLens {
            flare: true,
            ..LabLens::default()
        };
        lens.set_forced(flare);
        assert_eq!(lens.listed(), ["flare", "leak"]);
        let view = lens.view();
        assert!(view.flare_listed && view.leak_listed);
        // Without the forcing the flare is not listed, the scene's leak still is.
        lens.set_forced(LabLens::default());
        let view = lens.view();
        assert!(!view.flare_listed && view.leak_listed);
        let mut quiet = self::lens(&[]);
        quiet.set_forced(LabLens::default());
        assert_eq!(quiet.view(), LensView::default());
    }

    /// A moment starts one effect of those listed, as the scheduler picks it: a strong one at a
    /// strong moment, a light one at a medium moment, a weaker one there; none if none suits.
    #[test]
    fn a_moment_starts_one_listed_effect() {
        let strong = Played {
            strength: Strength::Strong,
            label: 1.0,
        };
        let medium = Played {
            strength: Strength::Medium,
            label: 1.0,
        };
        let mut all = lens(&["flare", "leak"]);
        all.start_at(strong);
        let view = all.view();
        assert!(view.flare_running != view.leak_running);
        let mut light = lens(&["flare"]);
        light.start_at(medium);
        assert!(light.view().flare_running);
        assert!(run(&mut light, 4.0).flare <= MEDIUM_STRENGTH * FLARE_PEAK + 1e-6);
        // A cue alone starts nothing any more: the scheduler does.
        let mut cued = lens(&["flare"]);
        assert!(cued.tick(1.0 / 60.0, &music(1.0, 1), 1.0).is_idle());
        // What a lab forced plays at every moment, all of it.
        let mut forced = lens(&["leak"]);
        forced.set_forced(LabLens {
            flare: true,
            ..LabLens::default()
        });
        assert!(forced.start_forced(medium));
        let view = forced.view();
        assert!(view.flare_running && !view.leak_running);
        assert!(!lens(&["leak"]).start_forced(strong));
        // A name the runtime no longer knows starts nothing.
        let mut only_blocks = lens(&["blocks"]);
        only_blocks.start_at(strong);
        assert!(only_blocks.tick(1.0 / 60.0, &music(0.0, 0), 1.0).is_idle());
    }

    /// The flare and the leak run their two bars and are over: the flare at 0.8 at its peak, the
    /// leak at 1; a weaker start is a weaker effect.
    #[test]
    fn an_effect_runs_its_course_and_ends() {
        let mut lens = lens(&[]);
        lens.start_flare(1.0);
        let peak = run(&mut lens, 2.0);
        assert!((peak.flare - FLARE_PEAK).abs() < 0.01, "{}", peak.flare);
        // 8 beats at 120 bpm are 4 s.
        run(&mut lens, 1.9);
        assert!(lens.view().flare_running);
        run(&mut lens, 0.2);
        assert!(!lens.view().flare_running);
        lens.start_flare(0.4);
        let weak = run(&mut lens, 4.2);
        assert!(
            (weak.flare - 0.4 * FLARE_PEAK).abs() < 0.01,
            "{}",
            weak.flare
        );

        lens.start_leak(1.0);
        let peak = run(&mut lens, 4.2);
        assert!(peak.leak > 0.99, "{}", peak.leak);
        assert!(!lens.view().leak_running);
    }

    /// Durations are in beats: twice the tempo, half the seconds.
    #[test]
    fn effects_follow_the_tempo() {
        let mut lens = lens(&[]);
        lens.start_leak(1.0);
        let fast = MusicValues {
            tempo: 240.0,
            ..music(0.0, 0)
        };
        for _ in 0..(2.1 * 60.0) as u32 {
            lens.tick(1.0 / 60.0, &fast, 1.0);
        }
        assert!(!lens.view().leak_running, "8 beats at 240 bpm are 2 s");
        // Without a followed beat a beat is half a second: 4 s.
        lens.start_flare(1.0);
        let free = MusicValues {
            locked: 0.0,
            tempo: 240.0,
            ..music(0.0, 0)
        };
        for _ in 0..(3.9 * 60.0) as u32 {
            lens.tick(1.0 / 60.0, &free, 1.0);
        }
        assert!(lens.view().flare_running);
        for _ in 0..(0.2 * 60.0) as u32 {
            lens.tick(1.0 / 60.0, &free, 1.0);
        }
        assert!(!lens.view().flare_running);
    }

    /// The flare and the leak rise from nothing, slowly at first, to their peak a third of the
    /// way through the two bars, and thin out slowly to nothing: in small steps, the flare's
    /// streaks reaching furthest and the leak's source furthest in at the peak.
    #[test]
    fn the_flare_and_the_leak_ramp_gently() {
        let mut lens = lens(&[]);
        lens.start_flare(1.0);
        lens.start_leak(1.0);
        // 8 beats at 120 bpm are 240 frames; the peak is at frame 80.
        let course: Vec<LensFrame> = (0..240)
            .map(|_| lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0))
            .collect();
        let after = lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0);
        assert!(after.flare < 0.01 && after.leak < 0.01);
        lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0);
        assert!(!lens.view().flare_running && !lens.view().leak_running);
        let flare: Vec<f32> = course.iter().map(|f| f.flare).collect();
        let leak: Vec<f32> = course.iter().map(|f| f.leak).collect();
        for values in [&flare, &leak] {
            assert_eq!(values[0], 0.0);
            assert!(values[1..80].windows(2).all(|w| w[1] > w[0]));
            assert!(values[81..].windows(2).all(|w| w[1] < w[0]));
            assert!(values.windows(2).all(|w| (w[1] - w[0]).abs() < 0.03));
            // Slowly at first: a tenth of the way to the peak, under a tenth of it.
            assert!(values[8] < 0.1 * values[80], "{}", values[8]);
            assert!(*values.last().unwrap() < 0.01);
        }
        assert!((flare[80] - FLARE_PEAK).abs() < 0.01 && (leak[80] - 1.0).abs() < 0.01);
        assert!((course[0].flare_stretch - FLARE_STRETCH_FROM).abs() < 1e-6);
        assert!((course[80].flare_stretch - FLARE_STRETCH).abs() < 1e-3);
        // The leak's source is furthest in at the peak.
        let outside = |f: &LensFrame| {
            let [x, y] = f.leak_at;
            (-x).max(-y).max(x - 1.0).max(y - 1.0)
        };
        assert!(outside(&course[80]) < outside(&course[10]));
        assert!(outside(&course[80]) < outside(&course[200]));
        assert!(
            course
                .iter()
                .all(|f| f.leak <= 1.0 && f.flare <= FLARE_PEAK)
        );
    }

    /// A start while one runs never stacks the two and never jumps: it swells again from where
    /// it is, back to the peak, and then runs its course; a stronger start rises to its strength.
    #[test]
    fn a_new_start_swells_again_without_a_jump() {
        let mut lens = lens(&[]);
        let mut values = Vec::new();
        lens.start_flare(0.5);
        lens.start_leak(1.0);
        for i in 0..600 {
            // Started again past the peak, before it, and once more stronger.
            if i == 150 || i == 170 {
                lens.start_flare(0.5);
                lens.start_leak(1.0);
            }
            if i == 300 {
                lens.start_flare(1.0);
            }
            let f = lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0);
            values.push((f.flare, f.leak, f.leak_at));
        }
        for w in values.windows(2) {
            assert!((w[1].0 - w[0].0).abs() < 0.03, "{w:?}");
            assert!((w[1].1 - w[0].1).abs() < 0.03, "{w:?}");
            let [x0, y0] = w[0].2;
            let [x1, y1] = w[1].2;
            if w[0].1 > 0.0 && w[1].1 > 0.0 {
                assert!((x1 - x0).abs() < 0.02 && (y1 - y0).abs() < 0.02, "{w:?}");
            }
        }
        assert!(values.iter().all(|v| v.0 <= FLARE_PEAK && v.1 <= 1.0));
        // Past the peak at frame 150 the leak was falling; started again, it rises once more.
        assert!(values[149].1 < 1.0 && values[160].1 > values[150].1);
        // The stronger start rises above the weaker's peak.
        let weak_peak = values[..300].iter().map(|v| v.0).fold(0.0, f32::max);
        let strong_peak = values[300..].iter().map(|v| v.0).fold(0.0, f32::max);
        assert!(weak_peak <= 0.5 * FLARE_PEAK + 1e-4 && strong_peak > 0.95 * FLARE_PEAK);
    }

    /// The flare gives way to a low quality, and the rest of the course goes by unseen.
    #[test]
    fn the_flare_gives_way_to_a_low_quality() {
        let mut lens = lens(&[]);
        lens.start_flare(1.0);
        let mut most = 0.0f32;
        for _ in 0..30 {
            most = most.max(lens.tick(1.0 / 60.0, &music(0.0, 0), 0.25).flare);
        }
        assert_eq!(most, 0.0);
        assert!(lens.view().flare_running);
    }

    /// The lab can start any effect in a scene that lists none.
    #[test]
    fn the_lab_starts_effects_the_scene_did_not_list() {
        let mut lens = lens(&[]);
        lens.start_flare(1.0);
        lens.start_leak(1.0);
        lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0);
        let view = lens.view();
        assert!(view.flare_running && view.leak_running && !view.leak_listed);
    }

    /// A glitch or a mosaic the scene lists starts over the picture at a strong moment, for the
    /// transition layer to take once; a lab can force them, and start them by hand.
    #[test]
    fn a_listed_glitch_or_mosaic_starts_over_the_picture_once() {
        let strong = Played {
            strength: Strength::Strong,
            label: 1.0,
        };
        let medium = Played {
            strength: Strength::Medium,
            label: 1.0,
        };
        let mut glitch = lens(&["glitch"]);
        assert!(glitch.view().glitch_listed && !glitch.view().mosaic_listed);
        glitch.start_at(medium);
        assert_eq!(glitch.take_overlay(), None);
        glitch.start_at(strong);
        assert_eq!(glitch.take_overlay(), Some(Overlay::Glitch));
        assert_eq!(glitch.take_overlay(), None);
        // It draws nothing in the post pass.
        glitch.start_at(strong);
        assert!(glitch.tick(1.0 / 60.0, &music(0.0, 0), 1.0).is_idle());
        // Forced by a lab: at a strong moment only.
        let mut forced = lens(&[]);
        forced.set_forced(LabLens {
            mosaic: true,
            ..LabLens::default()
        });
        assert!(forced.view().mosaic_listed);
        assert!(!forced.start_forced(medium));
        assert_eq!(forced.take_overlay(), None);
        assert!(forced.start_forced(strong));
        assert_eq!(forced.take_overlay(), Some(Overlay::Mosaic));
        forced.set_forced(LabLens::default());
        assert!(!forced.view().mosaic_listed);
        // By hand.
        forced.start_overlay(Overlay::Glitch);
        assert_eq!(forced.take_overlay(), Some(Overlay::Glitch));
    }

    /// Leaks one after another come from different corners in turn, inside the picture's reach.
    #[test]
    fn leaks_vary_their_corner() {
        let mut lens = lens(&[]);
        let mut corners = std::collections::HashSet::new();
        for _ in 0..8 {
            lens.start_leak(1.0);
            let frame = lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0);
            let [x, y] = frame.leak_at;
            assert!((-0.35..=1.35).contains(&x) && (-0.35..=1.35).contains(&y));
            corners.insert(((x > 0.5) as u8, (y > 0.5) as u8));
            // A new leak while one runs continues it, from its corner: let it end.
            run(&mut lens, 4.1);
            assert!(!lens.view().leak_running);
        }
        assert!(corners.len() >= 2, "{corners:?}");
    }
}
