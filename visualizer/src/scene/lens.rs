//! The lens effects of the built-in post-processing (`flare`, `leak`, `blocks`; see "Shared
//! effects" in `docs/music.md`). Each is an effect that is started at a moment with a strength
//! and runs its course (a burst of streaks, a glow that seeps in and goes, a frame or two of flat
//! colour); none is a constant layer. [`Lens::tick`] makes the small uniform `post.wgsl` reads,
//! and a scene that lists none gets a uniform of zeros, which costs the post pass nothing.
//!
//! What starts them is [`Lens::auto_start`], kept small on purpose: the moments are meant to
//! come from one scheduler shared by the effects. A scene lab starts them by hand.

use super::inputs::MusicValues;
use crate::switching::{FOLLOWED, SECTION_CUE};

/// The cue strength from which a colour-block frame is started: the strongest moments only.
pub const BLOCKS_CUE: f32 = 0.8;
/// How long a block frame lasts. About one frame at 30 fps and two at 60 fps; it is always
/// drawn at least once.
pub const BLOCKS_SECONDS: f32 = 0.03;
/// A flare comes in over this many beats and goes away over the next ones.
pub const FLARE_IN_BEATS: f32 = 0.5;
pub const FLARE_OUT_BEATS: f32 = 6.0;
/// A light leak comes in over this many beats, and goes away over the next ones.
pub const LEAK_IN_BEATS: f32 = 4.0;
pub const LEAK_OUT_BEATS: f32 = 16.0;
/// How far a leak's source wanders along its edge over the whole leak, as a share of the edge.
const LEAK_DRIFT: f32 = 0.18;
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
    /// The strength of the flare, 0 (off) and up.
    pub flare: f32,
    /// How much of the light leak is in, 0 to 1.
    pub leak: f32,
    /// How much of a colour-block frame is on, 0 or 1.
    pub blocks: f32,
    /// Where the leak's source is (in uv, on or just beyond an edge), and its radius in uv.
    pub leak_at: [f32; 2],
    pub leak_size: f32,
    /// A number that grows over the leak, which the shader turns into slow changes of shape.
    pub leak_phase: f32,
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
            self.leak,
            self.blocks,
            self.leak_phase,
            self.leak_at[0],
            self.leak_at[1],
            self.leak_size,
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
}

/// An effect under way: how strong it was started, and its age in beats.
#[derive(Clone, Copy, Debug)]
struct Run {
    strength: f32,
    age: f32,
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
    flare_listed: bool,
    leak_listed: bool,
    blocks_listed: bool,
    flare: Option<Run>,
    leak: Option<Leak>,
    /// The strength of the block frame on, and the seconds of it left.
    blocks: Option<(f32, f32)>,
    /// How many leaks started, which picks the corner of the next.
    leaks: u32,
}

impl Lens {
    /// For a scene whose `effects.lens` is `listed` (names the runtime knows; others are ignored).
    pub fn new(listed: &[String]) -> Self {
        let has = |name: &str| listed.iter().any(|l| l == name);
        Self {
            flare_listed: has("flare"),
            leak_listed: has("leak"),
            blocks_listed: has("blocks"),
            flare: None,
            leak: None,
            blocks: None,
            leaks: 0,
        }
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

    /// Fire a colour-block frame of `strength` (0 to 1: how much of the flat colour) now.
    pub fn start_blocks(&mut self, strength: f32) {
        self.blocks = Some((strength.clamp(0.0, 1.0), BLOCKS_SECONDS));
    }

    /// Start the effects the scene listed on the cue of this frame, if there is one: a flare and a
    /// leak on a section start (a cue of 0.5 or more), a block frame on a strong cue (0.8 or
    /// more), each as strong as the cue. This is all the automatic triggering there is for now.
    pub fn auto_start(&mut self, music: &MusicValues) {
        if music.cues == 0 {
            return;
        }
        if music.cue >= SECTION_CUE {
            if self.flare_listed {
                self.start_flare(music.cue);
            }
            if self.leak_listed {
                self.start_leak(music.cue);
            }
        }
        if music.cue >= BLOCKS_CUE && self.blocks_listed {
            self.start_blocks(music.cue);
        }
    }

    pub fn view(&self) -> LensView {
        LensView {
            flare_listed: self.flare_listed,
            leak_listed: self.leak_listed,
            blocks_listed: self.blocks_listed,
            flare_running: self.flare.is_some(),
            leak_running: self.leak.is_some(),
            blocks_running: self.blocks.is_some(),
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
        self.auto_start(music);

        let mut frame = LensFrame::default();

        // The flare: in fast, away over a bar or two. It gives way to a low quality.
        if let Some(run) = &mut self.flare {
            let total = FLARE_IN_BEATS + FLARE_OUT_BEATS;
            if run.age >= total {
                self.flare = None;
            } else {
                if quality >= FLARE_MIN_QUALITY {
                    frame.flare = run.strength * envelope(run.age, FLARE_IN_BEATS, FLARE_OUT_BEATS);
                }
                run.age += beats;
            }
        }

        // The leak: in over a few beats, away over some bars, its source drifting.
        if let Some(leak) = &mut self.leak {
            let total = LEAK_IN_BEATS + LEAK_OUT_BEATS;
            if leak.run.age >= total {
                self.leak = None;
            } else {
                frame.leak =
                    leak.run.strength * envelope(leak.run.age, LEAK_IN_BEATS, LEAK_OUT_BEATS);
                let share = leak.run.age / total;
                // The source sits a little beyond the corner and wanders along its edge.
                let (x, y) = match leak.corner {
                    0 => (-0.05, 0.1 + LEAK_DRIFT * share * leak.along),
                    1 => (0.1 + LEAK_DRIFT * share, -0.05),
                    2 => (1.05, 0.9 - LEAK_DRIFT * share),
                    _ => (0.9 - LEAK_DRIFT * share, 1.05),
                };
                frame.leak_at = [x, y];
                frame.leak_size = 0.55;
                frame.leak_phase = share * std::f32::consts::TAU;
                leak.run.age += beats;
            }
        }

        // The block frame is drawn at least once, then for the rest of its time.
        if let Some((strength, left)) = self.blocks {
            frame.blocks = strength;
            let left = left - dt;
            self.blocks = (left > 0.0).then_some((strength, left));
        }
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

    /// A weak cue starts nothing; a section start starts a flare and a leak, a strong cue also a
    /// block frame, each only if the scene listed it.
    #[test]
    fn cues_start_the_listed_effects() {
        let mut lens = lens(&["flare", "leak", "blocks"]);
        assert!(lens.tick(1.0 / 60.0, &music(0.49, 1), 1.0).is_idle());
        assert!(lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0).is_idle());
        let frame = lens.tick(1.0 / 60.0, &music(0.5, 1), 1.0);
        assert!(lens.view().flare_running && lens.view().leak_running);
        assert_eq!(frame.blocks, 0.0);
        let frame = lens.tick(1.0 / 60.0, &music(0.8, 1), 1.0);
        assert_eq!(frame.blocks, 0.8);
        // The cue's count is what starts them, not its envelope.
        let mut other = self::lens(&["flare"]);
        other.tick(1.0 / 60.0, &music(1.0, 0), 1.0);
        assert!(!other.view().flare_running);
        let mut only_leak = self::lens(&["leak"]);
        only_leak.tick(1.0 / 60.0, &music(1.0, 1), 1.0);
        let view = only_leak.view();
        assert!(view.leak_running && !view.flare_running && !view.blocks_running);
    }

    /// An effect runs its course in beats and is over: the flare in a bar or two, the leak over
    /// some bars; a stronger start is a stronger effect.
    #[test]
    fn an_effect_runs_its_course_and_ends() {
        let mut lens = lens(&[]);
        lens.start_flare(1.0);
        let peak = run(&mut lens, 0.5);
        assert!(peak.flare > 0.9, "{}", peak.flare);
        // 6.5 beats at 120 bpm are 3.25 s.
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
        // 20 beats are 10 s.
        run(&mut lens, 8.2);
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
        for _ in 0..(5.2 * 60.0) as u32 {
            lens.tick(1.0 / 60.0, &fast, 1.0);
        }
        assert!(!lens.view().leak_running, "20 beats at 240 bpm are 5 s");
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

    /// A block frame is one or two frames long, and at least one even at a low frame rate.
    #[test]
    fn a_block_frame_is_short_but_drawn() {
        let mut lens = lens(&[]);
        lens.start_blocks(1.0);
        let mut frames = 0;
        while lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0).blocks > 0.0 {
            frames += 1;
        }
        assert!((1..=2).contains(&frames), "{frames} frames");
        lens.start_blocks(1.0);
        assert_eq!(lens.tick(0.1, &music(0.0, 0), 1.0).blocks, 1.0);
        assert_eq!(lens.tick(0.1, &music(0.0, 0), 1.0).blocks, 0.0);
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
