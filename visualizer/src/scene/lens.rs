//! The lens effects of the built-in post-processing (`flare`, `leak`; see "Shared effects" in
//! `docs/music.md`). Each is an effect that is started at a moment with a strength and runs its
//! course (streaks or a glow that rise slowly from nothing and thin out again by the end of the
//! phrase, with the music; see `crate::course`); none is a constant layer. [`Lens::tick`] makes the small uniform `post.wgsl` reads, and a
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
use std::sync::Arc;

use crate::bind::{CourseBinds, MomentBinds, RuntimeBindings};
use crate::course::{Course, Phrase};
use crate::inspect::LabLens;
use crate::moments::{self, Played, Strength};
use crate::transition::Overlay;

/// How strong an effect a medium moment starts (a strong moment's is at full strength).
pub const MEDIUM_STRENGTH: f32 = 0.6;
/// The flare's intensity at the peak of its course, at full intensity of the music: subtle, as
/// it lasts.
pub const FLARE_PEAK: f32 = 0.8;
/// How far the flare's streaks reach at the start and the peak of its course: the weight of each
/// wider level of the streaks' pyramid against the one before (see `post.wgsl`).
pub const FLARE_STRETCH_FROM: f32 = 0.5;
pub const FLARE_STRETCH: f32 = 0.85;
/// How far a leak's source travels along the frame's edge over its course, in edges.
const LEAK_TRAVEL: f32 = 0.15;
/// The radius of a light leak's glow, as a share of the frame's long side.
const LEAK_SIZE: f32 = 0.5;
/// How far beyond the edge a light leak's source sits at the peak of its course, and how much
/// further out at its start and end (in uv), so that it reaches in and draws back.
const LEAK_OUTSIDE: f32 = 0.05;
const LEAK_REACH: f32 = 0.25;
/// The flare is drawn from this quality (`frame.quality`) up: it is lowered first.
pub const FLARE_MIN_QUALITY: f32 = 0.5;

/// The size of [`LensFrame`] in the uniform: twelve f32 (nine, padded to a multiple of 16
/// bytes).
pub const LENS_SIZE: u64 = 48;
/// The most the flare's streaks lean from the horizontal, either way (radians, 40 degrees): a
/// streak always level is dull, and one near upright reads as a fault.
pub const FLARE_TILT: f32 = 0.698_131_7;
/// The number the flare's lean is hashed with, beside the section's label.
const FLARE_TILT_KEY: u32 = 211;

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
    /// How far the flare's streaks lean from the horizontal, in radians (see [`FLARE_TILT`]).
    pub flare_angle: f32,
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
            self.flare_angle,
            0.0,
            0.0,
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
    /// The effect is under way.
    pub flare_running: bool,
    pub leak_running: bool,
    /// The glitch and the mosaic over the picture: listed (they are under way while the
    /// transition layer draws them, which it tells).
    pub glitch_listed: bool,
    pub mosaic_listed: bool,
}

/// An effect under way: how strong it was started, and its course.
#[derive(Clone, Copy, Debug)]
struct Run {
    strength: f32,
    course: Course,
}

impl Run {
    /// How much of the effect is on, 0 to its strength.
    fn level(&self) -> f32 {
        self.strength * self.course.amount()
    }

    /// The run started again at `strength` while this one is under way, as `fresh` would start:
    /// it carries on rising (at the stronger of the two strengths) from the level it has now,
    /// back on the rising side, so it swells again without a jump and two never add up.
    fn restarted(self, strength: f32, fresh: Course) -> Self {
        let strength = strength.clamp(0.0, 1.0).max(self.strength);
        let share = (self.strength * self.course.shape() / strength.max(1e-6)).min(1.0);
        Self {
            strength,
            course: self.course.restarted(fresh, share),
        }
    }
}

/// A light leak under way. It comes from a point on the frame's edge that the song's place
/// picks, and travels slowly along the edge from there.
#[derive(Clone, Copy, Debug)]
struct Leak {
    run: Run,
    /// Where it starts, as the distance along the edge clockwise from the top left corner in
    /// edges (0 to 4), and the way it travels (1 or -1).
    origin: f32,
    way: f32,
    /// How far it has travelled, as a share of a course: it goes on from where it is when the
    /// leak starts again.
    travelled: f32,
}

/// The point of the frame's edge `along` edges clockwise from the top left corner (in uv), and
/// `outside` beyond it, away from the centre: continuous round the corners.
fn edge_point(along: f32, outside: f32) -> [f32; 2] {
    let along = along.rem_euclid(4.0);
    let f = along.fract();
    let [x, y] = match along as u32 {
        0 => [f, 0.0],
        1 => [1.0, f],
        2 => [1.0 - f, 1.0],
        _ => [0.0, 1.0 - f],
    };
    let (dx, dy) = (x - 0.5, y - 0.5);
    let length = (dx * dx + dy * dy).sqrt().max(1e-3);
    [x + dx / length * outside, y + dy / length * outside]
}

/// Where a light leak that starts at this place of the song comes from (see [`Leak`]): a hash of
/// the section and of where in it the start is, so that the same song does the same thing.
fn leak_origin(music: &MusicValues, phrase: &Phrase) -> (f32, f32) {
    let (bars, beats) = phrase.place();
    let mut h = (music.section.max(0.0) as u32).wrapping_mul(0x9e37_79b9)
        ^ bars.wrapping_mul(0x85eb_ca6b)
        ^ beats.wrapping_mul(0xc2b2_ae35)
        ^ ((music.song_time.max(0.0) * 2.0) as u32).wrapping_mul(0x27d4_eb2f);
    h = (h ^ (h >> 16)).wrapping_mul(0x7feb_352d);
    h = (h ^ (h >> 15)).wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    let origin = (h >> 8) as f32 / (1u32 << 24) as f32 * 4.0;
    (origin, if h & 1 == 0 { 1.0 } else { -1.0 })
}

/// How far a flare leans its streaks (see [`FLARE_TILT`]): by the section's label with the
/// song's analysis, so the same kind of section flares the same way; without it (an online track,
/// the first seconds before the analysis) every section is one, so by how many flares came before
/// (`count`), each another way, the same order each time the song plays.
fn flare_angle(music: &MusicValues, count: u32) -> f32 {
    let r = if music.has_chart > 0 {
        super::director::label_hash(music.section_label, FLARE_TILT_KEY)
    } else {
        super::director::label_hash(count as f32, FLARE_TILT_KEY + 1)
    };
    (r * 2.0 - 1.0) * FLARE_TILT
}

/// The lens effects of one scene.
pub struct Lens {
    /// The effects that start at a moment: those the scene listed and those a lab forced.
    flare_listed: bool,
    leak_listed: bool,
    glitch_listed: bool,
    mosaic_listed: bool,
    flare: Option<Run>,
    /// How far the flare under way leans (see [`FLARE_TILT`]): taken when it starts, kept while
    /// it swells again; and how many flares have started.
    flare_angle: f32,
    flares: u32,
    leak: Option<Leak>,
    /// Where the music is in its phrase, and the music last heard: what an effect that starts
    /// takes its course and its place from (see [`Lens::listen`]).
    phrase: Phrase,
    music: MusicValues,
    /// What the scene itself listed, which a lab's forcing adds to and takes away from again.
    own: LabLens,
    /// What a lab forced on (see [`Lens::set_forced`]).
    forced: LabLens,
    /// A glitch or a mosaic over the picture that starts with the next frame, for the transition
    /// layer to take (see [`Lens::take_overlay`]).
    overlay: Option<Overlay>,
    /// What a section start is, and how a course reads the music (`runtime.toml`).
    moments: Arc<MomentBinds>,
    binds: CourseBinds,
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
            flare_angle: 0.0,
            flares: 0,
            leak: None,
            phrase: Phrase::default(),
            music: MusicValues::default(),
            own: LabLens {
                flare: has("flare"),
                leak: has("leak"),
                glitch: has("glitch"),
                mosaic: has("mosaic"),
            },
            forced: LabLens::default(),
            overlay: None,
            moments: Arc::default(),
            binds: CourseBinds::default(),
        }
    }

    /// Read the music by these bindings from now on (a scene lab replaced `runtime.toml`).
    pub fn set_binds(&mut self, runtime: &RuntimeBindings) {
        self.moments = runtime.moments.clone();
        self.binds = runtime.course;
    }

    /// How a course of this lens reads the music, for the effects the transition layer runs.
    pub fn course_binds(&self) -> CourseBinds {
        self.binds
    }

    /// Hear a frame's music, before the moments of the frame start anything: where it is in its
    /// phrase, for the course of an effect that starts.
    pub fn listen(&mut self, music: &MusicValues) {
        self.phrase.tick_with(&self.moments, music);
        self.music = *music;
    }

    /// Where the music is in its phrase.
    pub fn phrase(&self) -> Phrase {
        self.phrase
    }

    /// A course that starts now.
    fn course(&self) -> Course {
        Course::with(self.binds, &self.phrase, &self.music)
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

    /// Start a flare of `strength` (0 to 1) now, to the end of the phrase. One at a time: one
    /// under way swells again from where it is (see [`Run::restarted`]).
    ///
    /// A new flare leans its streaks by an angle of its own, up to [`FLARE_TILT`] either way,
    /// picked by the section's label: the same kind of section flares the same way, and a song the
    /// same way each time it plays.
    pub fn start_flare(&mut self, strength: f32) {
        let fresh = self.course();
        if self.flare.is_none() {
            self.flare_angle = flare_angle(&self.music, self.flares);
            self.flares = self.flares.wrapping_add(1);
        }
        self.flare = Some(match self.flare {
            Some(run) => run.restarted(strength, fresh),
            None => Run {
                strength: strength.clamp(0.0, 1.0),
                course: fresh,
            },
        });
    }

    /// Start a light leak of `strength` (0 to 1) now, to the end of the phrase, from the place of
    /// the edge the song's place picks (see [`leak_origin`]). One under way swells again from
    /// where it is, from its place (see [`Run::restarted`]).
    pub fn start_leak(&mut self, strength: f32) {
        let fresh = self.course();
        if let Some(leak) = &mut self.leak {
            leak.run = leak.run.restarted(strength, fresh);
            return;
        }
        let (origin, way) = leak_origin(&self.music, &self.phrase);
        self.leak = Some(Leak {
            run: Run {
                strength: strength.clamp(0.0, 1.0),
                course: fresh,
            },
            origin,
            way,
            travelled: 0.0,
        });
    }

    /// Keep a flare at its full swell from now on, as long as this is called before each frame
    /// (a probe's look at a steady flare).
    #[cfg(test)]
    pub fn hold_flare(&mut self) {
        self.flare_angle = flare_angle(&self.music, self.flares);
        self.flare = Some(Run {
            strength: 1.0,
            course: self.course().at_peak(),
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
        let lens = moments::lens_effect(&self.listed(), played);
        self.start_lens(played, lens);
    }

    /// Start the effect a moment that `played` picks over a cut the scene glides to: a light one
    /// only (see [`moments::lens_over_glide`]), at the moment's strength.
    pub fn start_over_glide(&mut self, played: Played) {
        let lens = moments::lens_over_glide(&self.listed(), played);
        self.start_lens(played, lens);
    }

    fn start_lens(&mut self, played: Played, lens: Option<moments::Lens>) {
        let strength = match played.strength {
            Strength::Strong => 1.0,
            Strength::Medium => MEDIUM_STRENGTH,
        };
        match lens {
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
        let mut frame = LensFrame::default();

        // The flare: its intensity and the reach of its streaks follow its course. It gives way
        // to a low quality.
        if let Some(run) = &mut self.flare {
            if run.course.is_over() {
                self.flare = None;
            } else {
                if quality >= FLARE_MIN_QUALITY {
                    frame.flare = FLARE_PEAK * run.level();
                    frame.flare_angle = self.flare_angle;
                    frame.flare_stretch = FLARE_STRETCH_FROM
                        + (FLARE_STRETCH - FLARE_STRETCH_FROM) * run.course.shape();
                }
                run.course.tick(dt, music);
            }
        }

        // The leak: its strength, its size (in the shader) and how far its source reaches in
        // from beyond the edge follow its course, while the source travels along the edge.
        if let Some(leak) = &mut self.leak {
            if leak.run.course.is_over() {
                self.leak = None;
            } else {
                let course = leak.run.course;
                frame.leak = leak.run.level();
                let outside = LEAK_OUTSIDE + LEAK_REACH * (1.0 - course.shape());
                let travelled = leak.travelled.min(1.5);
                let along = leak.origin + leak.way * LEAK_TRAVEL * travelled;
                frame.leak_at = edge_point(along, outside);
                frame.leak_size = LEAK_SIZE;
                frame.leak_phase = travelled * std::f32::consts::TAU;
                leak.run.course.tick(dt, music);
                leak.travelled += (leak.run.course.progress() - course.progress()).max(0.0);
            }
        }

        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Music at 120 bpm in 4/4, the beat followed, at full intensity.
    fn music(cue: f32, cues: u32) -> MusicValues {
        MusicValues {
            tempo: 120.0,
            locked: 1.0,
            meter: 4.0,
            intensity: 1.0,
            cue,
            cues,
            ..MusicValues::default()
        }
    }

    /// A lens that has heard the music at a section's start: its effects last a phrase of 4 bars,
    /// 16 beats, 8 s at 60 fps (480 frames).
    fn lens(names: &[&str]) -> Lens {
        let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        let mut lens = Lens::new(&names);
        lens.listen(&music(0.0, 0));
        lens
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

    /// A flare leans its streaks by an angle of its section's label, within the tilt either way:
    /// the same label the same lean, another label another, kept while it swells again.
    #[test]
    fn a_flare_leans_by_its_section_and_keeps_its_lean() {
        let angle_in = |label: f32| {
            let mut lens = lens(&["flare"]);
            lens.listen(&MusicValues {
                has_chart: 1,
                section_label: label,
                ..music(0.0, 0)
            });
            lens.start_flare(1.0);
            lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0).flare_angle
        };
        let angles: Vec<f32> = (0..12).map(|label| angle_in(label as f32)).collect();
        assert!(angles.iter().all(|a| a.abs() <= FLARE_TILT), "{angles:?}");
        assert_eq!(angle_in(3.0), angles[3]);
        assert!(
            angles.iter().any(|a| (a - angles[0]).abs() > 0.1),
            "{angles:?}"
        );
        assert!(angles.iter().any(|a| *a > 0.0) && angles.iter().any(|a| *a < 0.0));

        let mut lens = lens(&["flare"]);
        lens.listen(&MusicValues {
            has_chart: 1,
            section_label: 2.0,
            ..music(0.0, 0)
        });
        lens.start_flare(1.0);
        let first = lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0).flare_angle;
        lens.listen(&MusicValues {
            has_chart: 1,
            section_label: 5.0,
            ..music(0.0, 0)
        });
        lens.start_flare(1.0);
        assert_eq!(
            lens.tick(1.0 / 60.0, &music(0.0, 0), 1.0).flare_angle,
            first
        );
        // Without the song's analysis, flare after flare leans another way.
        let mut fresh = self::lens(&["flare"]);
        let mut unlabelled = Vec::new();
        for _ in 0..6 {
            fresh.start_flare(1.0);
            unlabelled.push(fresh.tick(1.0 / 60.0, &music(0.0, 0), 1.0).flare_angle);
            fresh.flare = None;
        }
        assert!(
            unlabelled.windows(2).all(|w| w[0] != w[1]),
            "{unlabelled:?}"
        );
        assert!(unlabelled.iter().all(|a| a.abs() <= FLARE_TILT));
        // The angle is told in the uniform, after the leak's phase.
        let frame = LensFrame {
            flare_angle: 0.5,
            ..LensFrame::default()
        };
        assert_eq!(&frame.bytes()[32..36], &0.5f32.to_le_bytes());
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

    /// The flare and the leak run to the end of the phrase and are over: the flare at 0.8 at its
    /// peak, the leak at 1; a weaker start is a weaker effect.
    #[test]
    fn an_effect_runs_its_course_and_ends() {
        let mut lens = lens(&[]);
        lens.start_flare(1.0);
        let peak = run(&mut lens, 3.0);
        assert!((peak.flare - FLARE_PEAK).abs() < 0.01, "{}", peak.flare);
        run(&mut lens, 4.9);
        assert!(lens.view().flare_running);
        run(&mut lens, 0.2);
        assert!(!lens.view().flare_running);
        lens.start_flare(0.4);
        let weak = run(&mut lens, 8.2);
        assert!(
            (weak.flare - 0.4 * FLARE_PEAK).abs() < 0.01,
            "{}",
            weak.flare
        );

        lens.start_leak(1.0);
        let peak = run(&mut lens, 8.2);
        assert!(peak.leak > 0.99, "{}", peak.leak);
        assert!(!lens.view().leak_running);
    }

    /// Durations are in beats: twice the tempo, half the seconds; 4 s without a followed beat.
    #[test]
    fn effects_follow_the_tempo() {
        let mut lens = lens(&[]);
        let fast = MusicValues {
            tempo: 240.0,
            ..music(0.0, 0)
        };
        lens.listen(&fast);
        lens.start_leak(1.0);
        for _ in 0..(4.1 * 60.0) as u32 {
            lens.tick(1.0 / 60.0, &fast, 1.0);
        }
        assert!(!lens.view().leak_running, "16 beats at 240 bpm are 4 s");
        let free = MusicValues {
            locked: 0.0,
            tempo: 240.0,
            ..music(0.0, 0)
        };
        lens.listen(&free);
        lens.start_flare(1.0);
        for _ in 0..(3.9 * 60.0) as u32 {
            lens.tick(1.0 / 60.0, &free, 1.0);
        }
        assert!(lens.view().flare_running);
        for _ in 0..(0.2 * 60.0) as u32 {
            lens.tick(1.0 / 60.0, &free, 1.0);
        }
        assert!(!lens.view().flare_running);
    }

    /// An effect's strength follows the music: at still music, a share of what it is at full,
    /// never nothing.
    #[test]
    fn an_effect_follows_the_music() {
        let still = MusicValues {
            intensity: 0.0,
            ..music(0.0, 0)
        };
        let mut lens = lens(&[]);
        lens.listen(&still);
        lens.start_flare(1.0);
        lens.start_leak(1.0);
        let mut most = LensFrame::default();
        for _ in 0..480 {
            let f = lens.tick(1.0 / 60.0, &still, 1.0);
            most.flare = most.flare.max(f.flare);
            most.leak = most.leak.max(f.leak);
        }
        let least = crate::course::LEAST;
        assert!(
            (most.flare - least * FLARE_PEAK).abs() < 0.01,
            "{}",
            most.flare
        );
        assert!((most.leak - least).abs() < 0.01, "{}", most.leak);
    }

    /// The flare and the leak rise from nothing, slowly at first, to their peak a third of the
    /// way through the phrase, and thin out slowly to nothing: in small steps, the flare's
    /// streaks reaching furthest and the leak's source furthest in at the peak.
    #[test]
    fn the_flare_and_the_leak_ramp_gently() {
        let mut lens = lens(&[]);
        lens.start_flare(1.0);
        lens.start_leak(1.0);
        // 16 beats at 120 bpm are 480 frames; the peak is at frame 160.
        let course: Vec<LensFrame> = (0..480)
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
            assert!(values[1..160].windows(2).all(|w| w[1] > w[0]));
            assert!(values[161..].windows(2).all(|w| w[1] < w[0]));
            assert!(values.windows(2).all(|w| (w[1] - w[0]).abs() < 0.02));
            // Slowly at first: a tenth of the way to the peak, under a tenth of it.
            assert!(values[16] < 0.1 * values[160], "{}", values[16]);
            assert!(*values.last().unwrap() < 0.01);
        }
        assert!((flare[160] - FLARE_PEAK).abs() < 0.01 && (leak[160] - 1.0).abs() < 0.01);
        assert!((course[0].flare_stretch - FLARE_STRETCH_FROM).abs() < 1e-6);
        assert!((course[160].flare_stretch - FLARE_STRETCH).abs() < 1e-3);
        // The leak's source is furthest in at the peak.
        let outside = |f: &LensFrame| {
            let [x, y] = f.leak_at;
            (-x).max(-y).max(x - 1.0).max(y - 1.0)
        };
        assert!(outside(&course[160]) < outside(&course[20]));
        assert!(outside(&course[160]) < outside(&course[400]));
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
        for i in 0..1000 {
            // Started again past the peak, before it, and once more stronger.
            if i == 300 || i == 320 {
                lens.start_flare(0.5);
                lens.start_leak(1.0);
            }
            if i == 450 {
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
        // Past the peak at frame 300 the leak was falling; started again, it rises once more.
        assert!(values[299].1 < values[250].1 && values[310].1 > values[300].1);
        // The stronger start rises above the weaker's peak.
        let weak_peak = values[..450].iter().map(|v| v.0).fold(0.0, f32::max);
        let strong_peak = values[450..].iter().map(|v| v.0).fold(0.0, f32::max);
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

    /// Where a leak comes from is picked by the song's place (the section, and the bars and beats
    /// into it): the same place, the same point; different places, points all round the frame's
    /// edge. Within one leak the source only travels a little along the edge.
    #[test]
    fn a_leak_comes_from_where_the_songs_place_picks() {
        let start_at = |section: f32, bars: u32| {
            let mut lens = lens(&[]);
            let m = MusicValues {
                section,
                ..music(0.0, 0)
            };
            for _ in 0..bars {
                lens.listen(&MusicValues { bars: 1, ..m });
            }
            lens.start_leak(1.0);
            let first = lens.tick(1.0 / 60.0, &m, 1.0).leak_at;
            let mut furthest = 0.0f32;
            for _ in 0..480 {
                let [x, y] = lens.tick(1.0 / 60.0, &m, 1.0).leak_at;
                if lens.view().leak_running {
                    furthest = furthest.max((x - first[0]).hypot(y - first[1]));
                }
            }
            (first, furthest)
        };
        assert_eq!(start_at(2.0, 5).0, start_at(2.0, 5).0);
        let mut sides = std::collections::HashSet::new();
        let mut points = Vec::new();
        for section in 0..4 {
            for bars in [0, 3, 6] {
                let (first, furthest) = start_at(section as f32, bars);
                let [x, y] = first;
                assert!((-0.35..=1.35).contains(&x) && (-0.35..=1.35).contains(&y));
                // On or beyond the edge, not inside the frame.
                assert!(
                    !((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)),
                    "{first:?}"
                );
                assert!(furthest < 0.5, "{furthest}");
                sides.insert((x < 0.0, x > 1.0, y < 0.0, y > 1.0));
                points.push(first);
            }
        }
        assert!(sides.len() >= 3, "{sides:?}");
        let distinct = points
            .iter()
            .enumerate()
            .filter(|(i, p)| {
                points[..*i]
                    .iter()
                    .all(|q| (p[0] - q[0]).hypot(p[1] - q[1]) > 0.05)
            })
            .count();
        assert!(distinct >= 9, "{points:?}");
    }
}
