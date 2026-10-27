//! The lens effects of the built-in post-processing (`flare`, `leak`, `blocks`; see "Shared
//! effects" in `docs/music.md`). Each is an effect that is started at a moment with a strength
//! and runs its course (a burst of streaks, a glow that seeps in and goes, a frame or two of flat
//! colour); none is a constant layer. [`Lens::tick`] makes the small uniform `post.wgsl` reads,
//! and a scene that lists none gets a uniform of zeros, which costs the post pass nothing.
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
/// How long a block frame shows the picture's shapes in the theme colour, and then how long its
/// negative stutters after it. About two frames and one at 60 fps, one and one at 30 fps; each is
/// always drawn at least once.
pub const BLOCKS_SECONDS: f32 = 0.03;
pub const BLOCKS_INVERTED_SECONDS: f32 = 0.015;
/// A flare's glare comes in over this many beats and goes away over the next ones.
pub const FLARE_IN_BEATS: f32 = 0.25;
pub const FLARE_OUT_BEATS: f32 = 6.0;
/// A flare's bright streak sweeps out from the brightest highlight to the frame's sides in this
/// many beats, as it starts.
pub const FLARE_SWEEP_BEATS: f32 = 1.0;
/// A light leak comes in over this many beats, and goes away over the next ones.
pub const LEAK_IN_BEATS: f32 = 1.0;
pub const LEAK_OUT_BEATS: f32 = 16.0;
/// A light leak burns hot at the edge it comes from: up over this many beats as it comes in, and
/// down over the next ones.
pub const LEAK_BURN_IN_BEATS: f32 = 0.5;
pub const LEAK_BURN_OUT_BEATS: f32 = 2.0;
/// How far a leak's source wanders along its edge over the whole leak, as a share of the edge.
const LEAK_DRIFT: f32 = 0.18;
/// The radius of a light leak's glow, as a share of the frame's long side.
const LEAK_SIZE: f32 = 0.5;
/// The seconds of a beat while the beat is not followed (120 bpm), so that durations in beats
/// are fixed seconds then.
pub const FALLBACK_BEAT_SECONDS: f32 = 0.5;
/// The flare is drawn from this quality (`frame.quality`) up: it is lowered first.
pub const FLARE_MIN_QUALITY: f32 = 0.5;

/// The size of [`LensFrame`] in the uniform: twelve f32.
pub const LENS_SIZE: u64 = 48;

/// What the post pass is told for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LensFrame {
    /// The strength of the flare's glare, 0 (off) and up.
    pub flare: f32,
    /// How far the flare's bright streak has swept out, 0 to 1 (1 once it reached the sides).
    pub flare_sweep: f32,
    /// 1 when the post pass takes the frame's key (its brightest, its mean and where the
    /// brightest is, which the flare and the block frame start from) as it is instead of
    /// following the last one smoothly: when the frame before drew none (a flare's first frame
    /// is at strength 0 and draws none, so a flare started again looks afresh too).
    pub fresh: f32,
    /// How much of the light leak is in, 0 to 1.
    pub leak: f32,
    /// Where the leak's source is (in uv, on or just beyond an edge), and its radius as a share
    /// of the frame's long side.
    pub leak_at: [f32; 2],
    pub leak_size: f32,
    /// A number that grows over the leak, which the shader turns into slow changes of shape.
    pub leak_phase: f32,
    /// How hot the leak burns at its edge, 0 to 1: only as it comes in.
    pub leak_burn: f32,
    /// How much of a colour-block frame is on, 0 or its strength.
    pub blocks: f32,
    /// 1 while the block frame stutters to its negative.
    pub blocks_inverted: f32,
}

impl LensFrame {
    /// Whether the post pass has anything to do beyond its normal work.
    #[cfg(test)]
    pub fn is_idle(&self) -> bool {
        self.flare <= 0.0 && self.leak <= 0.0 && self.blocks <= 0.0
    }

    /// The uniform `struct Lens` of `post.wgsl`.
    pub fn bytes(&self) -> [u8; LENS_SIZE as usize] {
        let mut bytes = [0; LENS_SIZE as usize];
        let values = [
            self.flare,
            self.flare_sweep,
            self.fresh,
            self.blocks,
            self.leak,
            self.leak_at[0],
            self.leak_at[1],
            self.leak_size,
            self.leak_phase,
            self.leak_burn,
            self.blocks_inverted,
            0.0,
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
    pub blocks_listed: bool,
    /// The effect is under way.
    pub flare_running: bool,
    pub leak_running: bool,
    pub blocks_running: bool,
    /// The glitch and the mosaic over the picture: listed (they are under way while the
    /// transition layer draws them, which it tells).
    pub glitch_listed: bool,
    pub mosaic_listed: bool,
}

/// An effect under way: how strong it was started, and its age in beats.
#[derive(Clone, Copy, Debug)]
struct Run {
    strength: f32,
    age: f32,
}

/// A colour-block frame under way: its strength, its age in seconds, and whether its plain and
/// its negative frame were drawn yet.
#[derive(Clone, Copy, Debug)]
struct Blocks {
    strength: f32,
    age: f32,
    plain_drawn: bool,
    inverted_drawn: bool,
}

/// A light leak under way, which comes from a corner.
#[derive(Clone, Copy, Debug)]
struct Leak {
    run: Run,
    /// The corner it comes from, 0 to 3, and the way it drifts along its edge (1 or -1).
    corner: u32,
    along: f32,
}

/// The lens effects of one scene.
pub struct Lens {
    /// The effects that start at a moment: those the scene listed and those a lab forced.
    flare_listed: bool,
    leak_listed: bool,
    blocks_listed: bool,
    glitch_listed: bool,
    mosaic_listed: bool,
    flare: Option<Run>,
    leak: Option<Leak>,
    blocks: Option<Blocks>,
    /// How many leaks started, which picks the corner of the next.
    leaks: u32,
    /// Whether the frame before drew the key (it had a flare or a block frame on).
    keyed: bool,
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
            blocks_listed: has("blocks"),
            glitch_listed: has("glitch"),
            mosaic_listed: has("mosaic"),
            flare: None,
            leak: None,
            blocks: None,
            leaks: 0,
            keyed: false,
            own: LabLens {
                flare: has("flare"),
                leak: has("leak"),
                blocks: has("blocks"),
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
        self.blocks_listed = self.own.blocks || forced.blocks;
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

    /// Start a flare of `strength` (0 to 1) now. One at a time: one under way starts again.
    pub fn start_flare(&mut self, strength: f32) {
        self.flare = Some(Run {
            strength: strength.clamp(0.0, 1.0),
            age: 0.0,
        });
    }

    /// Start a light leak of `strength` (0 to 1) now, from the next corner in turn.
    pub fn start_leak(&mut self, strength: f32) {
        let corner = self.leaks.wrapping_mul(0x9e37_79b9).rotate_left(7) >> 30;
        self.leaks = self.leaks.wrapping_add(1);
        self.leak = Some(Leak {
            run: Run {
                strength: strength.clamp(0.0, 1.0),
                age: 0.0,
            },
            corner,
            along: if corner.is_multiple_of(2) { 1.0 } else { -1.0 },
        });
    }

    /// Fire a colour-block frame of `strength` (0 to 1: how much of the graphic frame is over
    /// the picture) now: its plain frame, then its negative.
    pub fn start_blocks(&mut self, strength: f32) {
        self.blocks = Some(Blocks {
            strength: strength.clamp(0.0, 1.0),
            age: 0.0,
            plain_drawn: false,
            inverted_drawn: false,
        });
    }

    /// The names of the effects that start at a moment: those the scene listed and those a lab
    /// forced.
    fn listed(&self) -> Vec<String> {
        [
            (self.flare_listed, "flare"),
            (self.leak_listed, "leak"),
            (self.blocks_listed, "blocks"),
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
            Some(moments::Lens::Blocks) => self.start_blocks(strength),
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
        if forced.blocks {
            self.start_blocks(strength);
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
        forced.flare || forced.leak || forced.blocks || (strong && (forced.glitch || forced.mosaic))
    }

    /// What a lab forced on, to give a rebuilt scene.
    pub fn forced(&self) -> LabLens {
        self.forced
    }

    pub fn view(&self) -> LensView {
        LensView {
            flare_listed: self.flare_listed,
            leak_listed: self.leak_listed,
            blocks_listed: self.blocks_listed,
            flare_running: self.flare.is_some(),
            leak_running: self.leak.is_some(),
            blocks_running: self.blocks.is_some(),
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

        // The flare: its streak sweeps out over a beat, its glare comes in fast and goes over a
        // bar or two. It gives way to a low quality.
        if let Some(run) = &mut self.flare {
            let total = FLARE_IN_BEATS + FLARE_OUT_BEATS;
            if run.age >= total {
                self.flare = None;
            } else {
                if quality >= FLARE_MIN_QUALITY {
                    frame.flare = run.strength * envelope(run.age, FLARE_IN_BEATS, FLARE_OUT_BEATS);
                    frame.flare_sweep = (run.age / FLARE_SWEEP_BEATS).min(1.0);
                }
                run.age += beats;
            }
        }

        // The leak: in over a beat with a burn at its edge, away over some bars, its source
        // drifting.
        if let Some(leak) = &mut self.leak {
            let total = LEAK_IN_BEATS + LEAK_OUT_BEATS;
            if leak.run.age >= total {
                self.leak = None;
            } else {
                let age = leak.run.age;
                frame.leak = leak.run.strength * envelope(age, LEAK_IN_BEATS, LEAK_OUT_BEATS);
                frame.leak_burn =
                    leak.run.strength * envelope(age, LEAK_BURN_IN_BEATS, LEAK_BURN_OUT_BEATS);
                let share = age / total;
                // The source sits a little beyond the corner and wanders along its edge.
                let (x, y) = match leak.corner {
                    0 => (-0.05, 0.1 + LEAK_DRIFT * share * leak.along),
                    1 => (0.1 + LEAK_DRIFT * share, -0.05),
                    2 => (1.05, 0.9 - LEAK_DRIFT * share),
                    _ => (0.9 - LEAK_DRIFT * share, 1.05),
                };
                frame.leak_at = [x, y];
                frame.leak_size = LEAK_SIZE;
                frame.leak_phase = share * std::f32::consts::TAU;
                leak.run.age += beats;
            }
        }

        // The block frame: its plain frame, then its negative; each is drawn at least once.
        if let Some(blocks) = &mut self.blocks {
            let inverted = blocks.plain_drawn && blocks.age >= BLOCKS_SECONDS;
            let over = BLOCKS_SECONDS + BLOCKS_INVERTED_SECONDS;
            if inverted && blocks.inverted_drawn && blocks.age >= over {
                self.blocks = None;
            } else {
                frame.blocks = blocks.strength;
                if inverted {
                    frame.blocks_inverted = 1.0;
                    blocks.inverted_drawn = true;
                } else {
                    blocks.plain_drawn = true;
                }
                blocks.age += dt;
            }
        }
        let keyed = frame.flare > 0.0 || frame.blocks > 0.0;
        if keyed && !self.keyed {
            frame.fresh = 1.0;
        }
        self.keyed = keyed;
        frame
    }
}

/// 0 to 1 and back: a smoothstep up over `rise` beats at `age`, then down over `fall`.
fn envelope(age: f32, rise: f32, fall: f32) -> f32 {
    smooth(age / rise) * smooth((rise + fall - age) / fall)
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
            most.blocks = most.blocks.max(f.blocks);
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
        assert!(!view.blocks_listed);
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
        let mut all = lens(&["flare", "leak", "blocks"]);
        all.start_at(strong);
        assert_eq!(all.tick(1.0 / 60.0, &music(0.0, 0), 1.0).blocks, 1.0);
        let mut light = lens(&["flare", "blocks"]);
        light.start_at(medium);
        let view = light.view();
        assert!(view.flare_running && !view.blocks_running);
        assert!(run(&mut light, 1.0).flare <= MEDIUM_STRENGTH);
        // A cue alone starts nothing any more: the scheduler does.
        let mut cued = lens(&["flare"]);
        assert!(cued.tick(1.0 / 60.0, &music(1.0, 1), 1.0).is_idle());
        // What a lab forced plays at every moment, all of it.
        let mut forced = lens(&["leak"]);
        forced.set_forced(LabLens {
            flare: true,
            blocks: true,
            ..LabLens::default()
        });
        assert!(forced.start_forced(medium));
        let view = forced.view();
        assert!(view.flare_running && view.blocks_running && !view.leak_running);
        assert!(!lens(&["leak"]).start_forced(strong));
        let mut only_blocks = lens(&["blocks"]);
        only_blocks.start_at(medium);
        assert!(only_blocks.tick(1.0 / 60.0, &music(0.0, 0), 1.0).is_idle());
    }

    /// An effect runs its course in beats and is over: the flare in a bar or two, the leak over
    /// some bars; a stronger start is a stronger effect.
    #[test]
    fn an_effect_runs_its_course_and_ends() {
        let mut lens = lens(&[]);
        lens.start_flare(1.0);
        let peak = run(&mut lens, 0.5);
        assert!(peak.flare > 0.9, "{}", peak.flare);
        // 6.25 beats at 120 bpm are 3.125 s.
        run(&mut lens, 2.5);
        assert!(lens.view().flare_running);
        run(&mut lens, 1.0);
        assert!(!lens.view().flare_running);
        lens.start_flare(0.4);
        let weak = run(&mut lens, 4.0);
        assert!(weak.flare > 0.35 && weak.flare <= 0.4, "{}", weak.flare);

        lens.start_leak(1.0);
        let peak = run(&mut lens, 2.0);
        assert!(peak.leak > 0.95, "{}", peak.leak);
        // 17 beats are 8.5 s.
        run(&mut lens, 6.7);
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
        for _ in 0..(4.4 * 60.0) as u32 {
            lens.tick(1.0 / 60.0, &fast, 1.0);
        }
        assert!(!lens.view().leak_running, "17 beats at 240 bpm are 4.25 s");
        // Without a followed beat a beat is half a second.
        lens.start_flare(1.0);
        let free = MusicValues {
            locked: 0.0,
            tempo: 240.0,
            ..music(0.0, 0)
        };
        for _ in 0..(3.4 * 60.0) as u32 {
            lens.tick(1.0 / 60.0, &free, 1.0);
        }
        assert!(!lens.view().flare_running);
    }

    /// A block frame is a plain frame or two and then one of its negative, each drawn at least
    /// once even at a low frame rate.
    #[test]
    fn a_block_frame_is_short_but_drawn() {
        let mut lens = lens(&[]);
        lens.start_blocks(1.0);
        let mut plain = 0;
        let mut inverted = 0;
        loop {
            let frame = lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0);
            if frame.blocks <= 0.0 {
                break;
            }
            if frame.blocks_inverted > 0.0 {
                inverted += 1;
            } else {
                assert_eq!(inverted, 0, "the negative comes last");
                plain += 1;
            }
        }
        assert_eq!((plain, inverted), (2, 1));
        lens.start_blocks(1.0);
        let slow = |lens: &mut Lens| {
            let f = lens.tick(0.1, &music(0.0, 0), 1.0);
            (f.blocks, f.blocks_inverted)
        };
        assert_eq!(slow(&mut lens), (1.0, 0.0));
        assert_eq!(slow(&mut lens), (1.0, 1.0));
        assert_eq!(slow(&mut lens).0, 0.0);
    }

    /// A flare's streak sweeps out over a beat and says so. The key is taken afresh in the
    /// first frame that draws one after a frame that drew none (a flare started again too), and
    /// followed after that. A leak burns at its edge only as it comes in.
    #[test]
    fn the_flare_sweeps_and_the_leak_burns_as_they_start() {
        let mut lens = lens(&[]);
        let tick = |lens: &mut Lens| lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0);
        lens.start_flare(1.0);
        let first = tick(&mut lens);
        assert_eq!(
            (first.flare, first.fresh, first.flare_sweep),
            (0.0, 0.0, 0.0)
        );
        let second = tick(&mut lens);
        assert!(second.flare > 0.0 && second.fresh == 1.0);
        assert!(second.flare_sweep > 0.0 && second.flare_sweep < 0.1);
        assert_eq!(tick(&mut lens).fresh, 0.0);
        // A beat at 120 bpm is 30 frames.
        run(&mut lens, 0.5);
        let swept = tick(&mut lens);
        assert_eq!(swept.flare_sweep, 1.0);
        assert!(swept.flare > 0.8);
        lens.start_flare(1.0);
        tick(&mut lens);
        assert_eq!(tick(&mut lens).fresh, 1.0);
        // A block frame over a flare follows the key; alone, it takes it afresh.
        lens.start_blocks(1.0);
        assert_eq!(tick(&mut lens).fresh, 0.0);
        let mut blocks = self::lens(&[]);
        blocks.start_blocks(1.0);
        let firsts: Vec<f32> = (0..3).map(|_| tick(&mut blocks).fresh).collect();
        assert_eq!(firsts, [1.0, 0.0, 0.0]);

        lens.start_leak(1.0);
        let mut burn = Vec::new();
        for _ in 0..(8.5 * 60.0) as u32 {
            let f = lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0);
            burn.push((f.leak, f.leak_burn));
        }
        let hottest = burn.iter().map(|b| b.1).fold(0.0, f32::max);
        assert!(hottest > 0.9, "{hottest}");
        // Three beats in (90 frames) the burn is gone and the glow is still in.
        assert_eq!(burn[90].1, 0.0);
        assert!(burn[90].0 > 0.9, "{:?}", burn[90]);
        assert!(burn.iter().all(|b| b.1 <= 1.0 && b.0 <= 1.0));
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
        lens.start_blocks(1.0);
        let frame = lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0);
        assert_eq!(frame.blocks, 1.0);
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

    /// Leaks come from different corners in turn, inside the picture's reach.
    #[test]
    fn leaks_vary_their_corner() {
        let mut lens = lens(&[]);
        let mut corners = std::collections::HashSet::new();
        for _ in 0..8 {
            lens.start_leak(1.0);
            let frame = lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0);
            let [x, y] = frame.leak_at;
            assert!((-0.1..=1.1).contains(&x) && (-0.1..=1.1).contains(&y));
            corners.insert(((x > 0.5) as u8, (y > 0.5) as u8));
        }
        assert!(corners.len() >= 2, "{corners:?}");
    }
}
