//! Scene switches that are not a cut: the old scene fades to black and the new one fades in, or the
//! old picture dissolves into the new scene, or one of the strong ones: a flash of the theme
//! colour, a glitch, a hard mosaic. While a transition runs, scenes draw into an
//! intermediate texture and one fullscreen pass (`transition.wgsl`) writes the output from it;
//! when none runs, nothing extra is drawn or held.
//!
//! The same passes also dress a camera cut inside one scene (see [`Dressed`]): the picture before
//! the cut is the old one and the scene drawing on after it is the new one. A scene that lets
//! cuts be dressed keeps the last frame in a texture (see [`Plan::Keep`]); one that does not pays
//! nothing.
//!
//! This module owns the GPU side and the arithmetic of the phases. The [`crate::pipeline`] decides
//! when scenes are installed, since that needs the scene.

use std::cell::Cell;

use crate::bind::CourseBinds;
use crate::course::{Course, Phrase};
use crate::covers::{Cell as CoverCell, SHARP};
use crate::scene::{MusicValues, Scene};

/// The pass and, appended, the cut-in it draws for one of its kinds.
const SHADER: &str = concat!(include_str!("transition.wgsl"), include_str!("cutin.wgsl"));

/// How a scene switch looks. They are chosen by `crate::switching`; a cut is what
/// [`crate::pipeline::Pipeline::install`] does and only the placeholder of a scene built ahead
/// names it.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    /// The new scene replaces the old one at once.
    Cut,
    /// The old scene fades to black, then the new one fades in from black.
    Fade,
    /// The old scene's last picture dissolves into the new scene as it draws.
    Dissolve,
    /// A torn band slashes across the old scene, which plays on behind it, shows the cover of
    /// the song playing, and rips apart to the new scene (see [`CutInStyle`] for the accents).
    CutIn(CutInStyle),
    /// The old scene's exposure and bloom rise until the screen is flooded with a light tint of
    /// the theme colour; the new scene comes back out of it.
    Flash,
    /// The old picture breaks into rows of pieces that shift, smear, streak and flare up in
    /// flat light, in stages that hold a few frames each and stutter back to an earlier one;
    /// the new scene shows through more pieces stage by stage (see [`glitch_step`]).
    Glitch,
    /// Hard mosaic: the old picture in flat posterized cells that step up (a region, then the
    /// whole frame), the new picture behind the largest, and the cells step down again (see
    /// `mosaic_switch` in `transition.wgsl`).
    Mosaic,
}

impl Transition {
    /// The transition a name of `effects.switch` or `effects.cut` stands for, if the runtime
    /// draws it (a cut-in in its default style; the unknown ones are not here).
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "cut" => Self::Cut,
            "fade" => Self::Fade,
            "dissolve" => Self::Dissolve,
            "flash" => Self::Flash,
            "glitch" => Self::Glitch,
            "mosaic" => Self::Mosaic,
            "cutin" => Self::CutIn(CutInStyle::default()),
            _ => return None,
        })
    }

    /// Whether it goes out and in with the old scene still drawing live (a fade, a flash), as
    /// against holding the old picture and playing over the new scene (a dissolve, a glitch, a
    /// mosaic) or drawing over the old scene live until it tears to the new one (a cut-in).
    pub fn goes_out_and_in(self) -> bool {
        matches!(self, Self::Fade | Self::Flash)
    }

    /// Whether it is one of the strong transitions (`crate::moments::STRONG`), which strong
    /// moments play, as against the light ones.
    pub fn is_strong(self) -> bool {
        matches!(
            self,
            Self::Flash | Self::Glitch | Self::Mosaic | Self::CutIn(_)
        )
    }
}

/// The accents of a cut-in, picked by the kind of section it lands on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CutInStyle {
    /// Speed lines burst from behind the band as it lands, and a glint runs across the cover.
    #[default]
    Streaks,
    /// The band's edges fade from solid to coarse halftone dots.
    Halftone,
    /// Offset translucent copies of the band trail behind it.
    Echo,
}

impl CutInStyle {
    pub const ALL: [CutInStyle; 3] = [Self::Streaks, Self::Halftone, Self::Echo];

    /// The style after this one, round and round (for a scene lab).
    pub fn next(self) -> Self {
        match self {
            Self::Streaks => Self::Halftone,
            Self::Halftone => Self::Echo,
            Self::Echo => Self::Streaks,
        }
    }

    /// A short name to show.
    pub fn name(self) -> &'static str {
        match self {
            Self::Streaks => "speed lines + glint",
            Self::Halftone => "halftone",
            Self::Echo => "echo",
        }
    }

    /// The style of the section with this label: the same kind of section always gets the same
    /// one. Without a chart there are no sections and the default stands.
    pub fn of_section(label: f32, has_chart: bool) -> Self {
        if !has_chart {
            return Self::default();
        }
        Self::ALL[label_hash(label) as usize % Self::ALL.len()]
    }

    /// `style` in `transition.wgsl`.
    fn index(self) -> f32 {
        match self {
            Self::Streaks => 0.0,
            Self::Halftone => 1.0,
            Self::Echo => 2.0,
        }
    }
}

/// A number that is the same for the same section label and looks unrelated for the next one, to
/// pick among things by the kind of section.
pub fn label_hash(label: f32) -> u32 {
    let mut x = label.max(0.0) as u32;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// How long the band takes to open at the start of a cut-in: ten frames at 60 fps, drawn on twos
/// like hand-drawn animation (the slash, the burst, the flash held, the overshoot settling).
pub const CUTIN_BAND_SECONDS: f64 = 10.0 / 60.0;
/// The shortest the band holds before it tears, in seconds.
pub const CUTIN_MIN_HOLD_SECONDS: f64 = 0.1;

/// How long a transition takes, in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Durations {
    /// A fade or a flash: the old scene on its way out. A cut-in: the band opening and holding
    /// over the old scene, up to the moment it starts to tear (the new scene goes in there).
    /// The others do not use it.
    pub out: f64,
    /// A fade: the new scene fading in. A dissolve, a glitch, a mosaic: the whole of it. A
    /// cut-in: the band tearing away.
    pub into: f64,
}

impl Durations {
    /// Whether a switch of `transition` with these durations shows anything.
    pub fn is_instant(&self, transition: Transition) -> bool {
        match transition {
            Transition::Cut => true,
            Transition::Fade | Transition::Flash | Transition::CutIn(_) => {
                self.out <= 0.0 && self.into <= 0.0
            }
            Transition::Dissolve | Transition::Glitch | Transition::Mosaic => self.into <= 0.0,
        }
    }
}

/// Where a fade is, `seconds` after its start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FadePhase {
    /// The old scene on its way to black, `progress` of the way (0 to 1).
    Out(f32),
    /// The new scene on its way from black, `progress` of the way.
    In(f32),
    /// Over.
    Done,
}

/// The phase of a fade of `out` seconds out and `into` seconds in, `seconds` after its start.
pub fn fade_phase(seconds: f64, out: f64, into: f64) -> FadePhase {
    let seconds = seconds.max(0.0);
    if seconds < out {
        FadePhase::Out((seconds / out) as f32)
    } else if seconds < out + into {
        FadePhase::In(((seconds - out) / into) as f32)
    } else {
        FadePhase::Done
    }
}

/// How far a dissolve of `duration` seconds is, `seconds` after the new scene went in (0 to 1,
/// 1 once it is over).
pub fn dissolve_progress(seconds: f64, duration: f64) -> f32 {
    if duration <= 0.0 {
        1.0
    } else {
        (seconds / duration).clamp(0.0, 1.0) as f32
    }
}

/// Where a cut-in is, `seconds` after it started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutInFrame {
    pub style: CutInStyle,
    pub seconds: f32,
    /// How long the band holds after it opened, and how long it takes to tear.
    pub hold: f32,
    pub tear: f32,
}

impl CutInFrame {
    /// Whether the band is tearing: the new scene is in from this frame on, the old one drew
    /// until the frame before. The pass is told so rather than working it out, so that the two
    /// cannot disagree.
    pub fn tearing(&self) -> bool {
        self.seconds >= CUTIN_BAND_SECONDS as f32 + self.hold
    }
}

/// The cut-in of `out` seconds up to the tear and `into` seconds of tearing, `seconds` after it
/// started; none once it is over. An `out` too short for the band to hold a moment is made
/// longer.
pub fn cutin_frame(seconds: f64, out: f64, into: f64, style: CutInStyle) -> Option<CutInFrame> {
    let seconds = seconds.max(0.0);
    let hold = (out - CUTIN_BAND_SECONDS).max(CUTIN_MIN_HOLD_SECONDS);
    (seconds < CUTIN_BAND_SECONDS + hold + into).then_some(CutInFrame {
        style,
        seconds: seconds as f32,
        hold: hold as f32,
        tear: into as f32,
    })
}

/// How long the punch on a cut of a burst lasts, in seconds.
pub const PUNCH_SECONDS: f64 = 0.12;
/// How much the picture is scaled up at the start of a punch of full intensity.
pub const PUNCH_ZOOM: f32 = 0.07;
/// The length of the smear at the start of a punch of full intensity, as a share of the frame's
/// height.
pub const PUNCH_SMEAR: f32 = 0.05;
/// How far apart the colour channels are at the start of a punch of full intensity, as a share
/// of their distance from the centre.
pub const PUNCH_SPLIT: f32 = 0.012;
/// How far towards the flood the picture is at the start of a punch of full intensity: faint,
/// far under a flash's 1.
pub const PUNCH_FLASH: f32 = 0.15;

/// How strong the punch on cut `fill` of a burst (1 to 4) is: 0.4, 0.6, 0.8, 1.0, so that the
/// last cut hits hardest; none (0) for a cut that is not in a burst.
pub fn punch_intensity(fill: u32) -> f32 {
    match fill {
        0 => 0.0,
        fill => (0.2 + 0.2 * fill as f32).min(1.0),
    }
}

/// What the pass applies in a frame of a punch (see [`punch_frame`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PunchFrame {
    /// How much the picture is scaled about its centre (1 for none).
    pub scale: f32,
    /// Which way the smear runs along the diagonal: 1 up to the right, -1 up to the left.
    pub side: f32,
    /// The length of the smear, as a share of the frame's height.
    pub smear: f32,
    /// How far apart the colour channels are, as a share of their distance from the centre.
    pub split: f32,
    /// How far towards the theme colour's flood the picture is (0 to 1).
    pub flash: f32,
}

/// The punch on cut `fill` of a burst, `seconds` after the cut: the picture starts scaled up and
/// settles to 1, eased out; the smear, the colour split and the flash fade with it. The smear
/// changes side with each cut. None for a cut that is not in a burst, and once it is over.
pub fn punch_frame(seconds: f64, fill: u32) -> Option<PunchFrame> {
    let intensity = punch_intensity(fill);
    let progress = (seconds.max(0.0) / PUNCH_SECONDS) as f32;
    if intensity <= 0.0 || progress >= 1.0 {
        return None;
    }
    let left = 1.0 - progress;
    // The pop settles by an ease-out cubic; the rest fall away faster.
    let pop = left * left * left;
    let fade = left * left;
    Some(PunchFrame {
        scale: 1.0 + PUNCH_ZOOM * intensity * pop,
        side: if fill % 2 == 1 { 1.0 } else { -1.0 },
        smear: PUNCH_SMEAR * intensity * fade,
        split: PUNCH_SPLIT * intensity * fade,
        flash: PUNCH_FLASH * intensity * fade,
    })
}

/// The punch on a cut of a burst, under way: it draws over the live frame only, so it holds no
/// picture and a scene that has no other use for the pass pays for it only while it plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Punch {
    /// The time of the frame of the cut, on the render loop's clock.
    pub start: f64,
    /// The cut's position in the burst (1 to 4).
    pub fill: u32,
}

impl Punch {
    /// What the pass does `seconds` after the cut, none when the course is over.
    pub fn plan(&self, seconds: f64) -> Plan {
        punch_frame(seconds, self.fill).map_or(Plan::None, Plan::Punch)
    }
}

/// What the pass does in a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Plan {
    /// No transition: the scene draws into the output.
    None,
    /// A scene that lets its cuts be dressed: it draws into the texture, the pass shows it as it
    /// is, and the frame is kept for the cut that may come next.
    Keep,
    /// A fade: the scene's frame `amount` (0 to 1) of the way to black.
    Fade(f32),
    /// A flash's rise: the scene's frame `amount` (0 to 1) of the way up, first washed in the
    /// theme colour (the first [`FLASH_WASH`] of the rise), then overexposed into the flood of
    /// light.
    FlashRise(f32),
    /// A flash's fall: the scene's frame at `amount` (1 to 0) of the way to the flood of light.
    Flash(f32),
    /// A dressed cut's fade-out: the held picture `amount` of the way to black.
    HeldFade(f32),
    /// A dressed cut's rise of a flash: the held picture `amount` of the way up, washed and then
    /// flooded as [`Plan::FlashRise`].
    HeldFlash(f32),
    /// A dissolve's first frame: the scene before draws the picture to hold, and it is shown.
    Capture,
    /// A dissolve: `progress` (0 to 1) of the way from the held picture to the scene's frame.
    Dissolve(f32),
    /// A cut-in over the scene's frame: the old scene's until the band tears, then the new
    /// one's.
    CutIn(CutInFrame),
    /// A glitch of the held picture into the scene's frame, `progress` (0 to 1) of the way, from
    /// the pattern `seed` makes.
    Glitch(f32, u32),
    /// A mosaic of the held picture into the scene's frame, the same way.
    Mosaic(f32, u32),
    /// The punch on a cut of a burst, over the scene's live frame.
    Punch(PunchFrame),
    /// A glitch or a mosaic as a lens effect over the scene's live frame (see [`OverlayRun`]).
    Overlay(OverlayFrame),
}

impl Plan {
    /// The plan of a transition that goes out and in (see [`Transition::goes_out_and_in`])
    /// `seconds` after its start, or none when it is over, with the phase it is in (for the
    /// caller that installs the new scene from the way in on). With `held` the way out shows the
    /// picture held, as a dressed cut does.
    pub fn out_and_in(
        kind: Transition,
        seconds: f64,
        out: f64,
        into: f64,
        held: bool,
    ) -> (Self, FadePhase) {
        let flash = kind == Transition::Flash;
        let phase = fade_phase(seconds, out, into);
        let plan = match phase {
            FadePhase::Out(p) => match (flash, held) {
                (false, false) => Self::Fade(p),
                (true, false) => Self::FlashRise(p),
                (false, true) => Self::HeldFade(p),
                (true, true) => Self::HeldFlash(p),
            },
            FadePhase::In(p) if flash => Self::Flash(1.0 - p),
            FadePhase::In(p) => Self::Fade(1.0 - p),
            FadePhase::Done => Self::None,
        };
        (plan, phase)
    }

    /// The plan of a transition that holds the old picture (a dissolve, a glitch, a mosaic) at
    /// `progress`.
    pub fn over_held(kind: Transition, progress: f32, seed: u32) -> Self {
        match kind {
            Transition::Glitch => Self::Glitch(progress, seed),
            Transition::Mosaic => Self::Mosaic(progress, seed),
            _ => Self::Dissolve(progress),
        }
    }

    /// Whether a picture is held besides the scene's frame: the pass reads it, or it is kept for
    /// a cut.
    fn needs_held(self) -> bool {
        match self {
            Self::None
            | Self::Fade(_)
            | Self::FlashRise(_)
            | Self::Flash(_)
            | Self::Punch(_)
            | Self::CutIn(_) => false,
            // An overlay reads the live frame alone; in a scene that keeps its frames the held
            // picture is for the cut that may come.
            Self::Overlay(frame) => frame.keep,
            _ => true,
        }
    }

    /// Whether the pass draws over the scene's live frame alone, as it plays on (a frame of a
    /// scene that keeps its frames, a punch, a lens glitch or mosaic): a cut in it can be
    /// dressed, and the frame kept.
    pub fn is_live(self) -> bool {
        matches!(self, Self::Keep | Self::Punch(_) | Self::Overlay(_))
    }

    /// The plan with an overlay in it keeping the scene's frames, as [`Plan::Keep`] does (the
    /// scene lets its cuts be dressed); any other as it is.
    pub fn keeping(self) -> Self {
        match self {
            Self::Overlay(frame) => Self::Overlay(OverlayFrame {
                keep: true,
                ..frame
            }),
            plan => plan,
        }
    }
}

/// The pattern a glitch or a mosaic that starts at `time` (the render loop's clock) makes.
pub fn seed_of(time: f64) -> u32 {
    let bits = time.to_bits();
    (bits ^ (bits >> 32)) as u32
}

/// One stage of a glitch: the pattern of its pieces (a stage that stutters back repeats an
/// earlier one's), the share of the pieces that break, and the share that show the new picture
/// (the scene's frame) rather than the old one (the picture held).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlitchStep {
    pub index: u32,
    pub pattern: u32,
    pub amount: f32,
    pub reveal: f32,
}

/// The stages of a glitch that switches the scene, in equal shares of its time, as (pattern,
/// amount, reveal). After the reference video (30 fps): it breaks, breaks harder, stutters back to
/// the first stage, then the new picture comes through the heaviest stage and the breaks die down
/// over it.
const GLITCH_SWITCH: [(u32, f32, f32); 6] = [
    (0, 0.55, 0.0),
    (1, 0.85, 0.0),
    (0, 0.7, 0.0),
    (2, 0.95, 0.5),
    (3, 0.7, 1.0),
    (4, 0.25, 1.0),
];

/// The stage of a glitch that switches the scene, `progress` (0 to 1) of the way through, from the
/// patterns `seed` makes.
pub fn glitch_step(progress: f32, seed: u32) -> GlitchStep {
    let count = GLITCH_SWITCH.len();
    let index = ((progress.clamp(0.0, 1.0) * count as f32) as usize).min(count - 1);
    let (pattern, amount, reveal) = GLITCH_SWITCH[index];
    GlitchStep {
        index: index as u32,
        pattern: seed ^ pattern.wrapping_add(1).wrapping_mul(0x9e37_79b9),
        amount,
        reveal,
    }
}

/// A strong transition played as a lens effect over the live picture, at a moment with no cut
/// (see `crate::moments::lens_effect`): a long effect to the end of the phrase, whose amount rises
/// and falls with the music, as the flare's and the leak's do (see `crate::course`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    /// Wide tears that jump with the beat: rows shifted sideways by a tenth of the width or more,
    /// their colours split, some inverted or burnt to the theme colour, a stutter now and then.
    Glitch,
    /// Big hard blocks (flat, posterized) over a region or the whole frame, stepping coarse to
    /// finer with the beat (see `transition.wgsl`).
    Mosaic,
}

/// The most of a glitch overlay: its amount at the peak, which tears 40% of the rows.
pub const OVERLAY_GLITCH_MOST: f32 = 1.0;
/// The most of a mosaic overlay: its amount at the peak of its course with the music at full
/// intensity, which sets the size of its blocks and how often a hit comes.
pub const OVERLAY_MOSAIC_MOST: f32 = 1.0;

/// What the pass draws in one frame of an overlay: all of it moves on its eased beat time (see
/// `crate::course::Course::motion`), with a push on every beat, and takes the beat's accent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayFrame {
    pub kind: Overlay,
    /// What makes its pattern: the run's.
    pub seed: u32,
    /// How much: its course's amount (with the music) times the most of the kind.
    pub amount: f32,
    /// Its eased beat time (it dwells on the whole beats).
    pub beats: f32,
    /// Its linear beat time since it started: what its events and patches are timed by.
    pub linear: f32,
    /// The accent of the last beat, 0 to 1 (see `crate::course::Course::accent`).
    pub accent: f32,
    /// Whether the scene keeps its frames under it, for a cut to be dressed (see
    /// [`Plan::needs_held`]).
    pub keep: bool,
}

/// A lens glitch or mosaic under way: like the punch, it draws over the live frame only, so it
/// holds no picture and a scene that has no other use for the pass pays for it only while it
/// plays. One at a time: see [`Transitions::start_overlay`]. It is told the music of every frame
/// ([`OverlayRun::tick`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayRun {
    pub kind: Overlay,
    /// The time of its first frame, on the render loop's clock, which also makes its pattern, and
    /// of the last frame it was told of.
    pub start: f64,
    pub last: f64,
    /// Its length, its amount with the music and its steps.
    pub course: Course,
}

/// The overlay that plays when `run` starts while `running` is under way: one at a time, the
/// one under way if it is of the same kind (it goes on), else `run`.
fn next_overlay(running: Option<OverlayRun>, run: OverlayRun) -> OverlayRun {
    match running {
        Some(running) if running.kind == run.kind => running,
        _ => run,
    }
}

impl OverlayRun {
    /// An overlay of `kind` that starts at `start`, in the music `music` at `phrase`: to the end
    /// of the phrase (see `crate::course::Phrase::beats_left`).
    #[cfg(test)]
    pub fn new(kind: Overlay, start: f64, phrase: &Phrase, music: &MusicValues) -> Self {
        Self::with(CourseBinds::default(), kind, start, phrase, music)
    }

    /// The same, with a course that reads the music by `binds` (`course.*` of `runtime.toml`).
    pub fn with(
        binds: CourseBinds,
        kind: Overlay,
        start: f64,
        phrase: &Phrase,
        music: &MusicValues,
    ) -> Self {
        Self {
            kind,
            start,
            last: start,
            course: Course::with(binds, phrase, music),
        }
    }

    /// Tell it the frame at `time` and its music.
    pub fn tick(&mut self, time: f64, music: &MusicValues) {
        let dt = (time - self.last).max(0.0) as f32;
        self.last = time;
        self.course.tick(dt, music);
    }

    /// The frame it draws now, none when it is over.
    pub fn frame(&self) -> Option<OverlayFrame> {
        if self.course.is_over() {
            return None;
        }
        let most = match self.kind {
            Overlay::Glitch => OVERLAY_GLITCH_MOST,
            Overlay::Mosaic => OVERLAY_MOSAIC_MOST,
        };
        Some(OverlayFrame {
            kind: self.kind,
            seed: seed_of(self.start),
            amount: self.course.amount() * most,
            beats: self.course.motion(),
            linear: self.course.age_beats(),
            accent: self.course.accent(),
            keep: false,
        })
    }

    /// What the pass does now, none when it is over.
    pub fn plan(&self) -> Plan {
        self.frame().map_or(Plan::None, Plan::Overlay)
    }
}

/// A transition that dresses a camera cut inside a scene, under way. The cut has already happened
/// when it starts, so it plays its whole course after it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dressed {
    pub kind: Transition,
    pub durations: Durations,
    /// The time of the frame of the cut, on the render loop's clock.
    pub start: f64,
}

impl Dressed {
    /// What the pass does `seconds` after the cut (the frame of the cut is 0), none when the
    /// course is over.
    pub fn plan(&self, seconds: f64) -> Plan {
        let Durations { out, into } = self.durations;
        match self.kind {
            Transition::Fade | Transition::Flash => {
                Plan::out_and_in(self.kind, seconds, out, into, true).0
            }
            Transition::Dissolve | Transition::Glitch | Transition::Mosaic => {
                match dissolve_progress(seconds, into) {
                    progress if progress >= 1.0 => Plan::None,
                    progress => Plan::over_held(self.kind, progress, seed_of(self.start)),
                }
            }
            // A cut-in does not dress a cut (see `switching::dress`).
            Transition::Cut | Transition::CutIn(_) => Plan::None,
        }
    }

    /// How far the course is, 0 to 1 over the whole of it.
    pub fn progress(&self, seconds: f64) -> f32 {
        let Durations { out, into } = self.durations;
        let whole = if self.kind.goes_out_and_in() {
            out + into
        } else {
            into
        };
        (seconds / whole.max(f64::EPSILON)).clamp(0.0, 1.0) as f32
    }
}

/// How a scene that lets its cuts be dressed has been asked to dress the next one, when the lab
/// asks for it (see [`crate::pipeline::Pipeline::cut_dressed`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forced {
    None,
    /// Waiting for a frame to be kept, so that the cut has a picture before it.
    Wanted(Transition),
    /// The director was told to cut: the cut of the next frame is dressed like this.
    Armed(Transition),
}

/// A transition under way.
pub struct Running {
    pub kind: Transition,
    pub durations: Durations,
    /// When it started: the time of its first frame, and for a dissolve the time the new scene
    /// went in.
    pub start: Option<f64>,
    /// The scene to install, until it is.
    pub pending: Option<Scene>,
}

/// The transition pass and the textures it needs while one runs.
pub struct Transitions {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    format: wgpu::TextureFormat,
    /// The cover of the song playing, as the bytes of its sharp copy (not decoded from sRGB,
    /// since the frames are blended as bytes), for the band of a cut-in. A pixel until there is
    /// a cover, so that no cover costs nothing.
    cover: wgpu::Texture,
    cover_view: wgpu::TextureView,
    has_cover: bool,
    sampler: wgpu::Sampler,
    pub running: Option<Running>,
    targets: Option<Targets>,
    /// Whether the dissolve's picture was drawn into the held texture since it was asked for.
    captured: Cell<bool>,
    /// The transition that dresses a camera cut, while it plays.
    pub dressed: Option<Dressed>,
    /// What the lab asked a dressed cut to be.
    pub forced: Forced,
    /// What dresses every camera cut in place of the scene's choice, `Cut` for none (see
    /// [`crate::LabCommand::CutsWith`]).
    pub lab_cuts: Option<Transition>,
    /// The punch on a cut of a burst, while it plays.
    pub punch: Option<Punch>,
    /// A lens glitch or mosaic, while it plays.
    pub overlay: Option<OverlayRun>,
}

/// The textures of a running transition.
struct Targets {
    size: (u32, u32),
    fresh: (wgpu::Texture, wgpu::TextureView),
    held: Option<(wgpu::Texture, wgpu::TextureView)>,
    group: wgpu::BindGroup,
    /// Whether `held` has the frame before (see [`Transitions::keep`]).
    kept: bool,
}

impl Transitions {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let texture = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("transition"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(1, texture),
                entry(2, texture),
                entry(
                    3,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    4,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("transition"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("transition"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("transition"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("transition"),
            size: UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cover = cover_texture(device, 1);
        let cover_view = cover.create_view(&Default::default());
        // The cover pans past its edge while the band holds: mirrored there.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("transition cover"),
            address_mode_u: wgpu::AddressMode::MirrorRepeat,
            address_mode_v: wgpu::AddressMode::MirrorRepeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            uniform,
            format,
            cover,
            cover_view,
            has_cover: false,
            sampler,
            running: None,
            targets: None,
            captured: Cell::new(false),
            dressed: None,
            forced: Forced::None,
            lab_cuts: None,
            punch: None,
            overlay: None,
        }
    }

    /// The cover of the song playing for the band of a cut-in, from its sharp copy (none: the
    /// band shows a close crop of the old scene in the theme colour).
    pub fn set_cover(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cell: Option<&CoverCell>,
    ) {
        self.has_cover = cell.is_some_and(|cell| cell.has_cover);
        let Some(cell) = cell.filter(|cell| cell.has_cover) else {
            return;
        };
        if self.cover.width() != SHARP {
            self.cover = cover_texture(device, SHARP);
            self.cover_view = self.cover.create_view(&Default::default());
            // The textures of a transition under way (or of one kept for a dressed cut) are
            // bound with the pixel; they are bound again with the cover.
            let group = self
                .targets
                .as_ref()
                .map(|targets| self.bind(device, &targets.fresh.1, targets.held.as_ref()));
            if let (Some(targets), Some(group)) = (&mut self.targets, group) {
                targets.group = group;
            }
        }
        queue.write_texture(
            self.cover.as_image_copy(),
            &cell.sharp,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SHARP * 4),
                rows_per_image: Some(SHARP),
            },
            wgpu::Extent3d {
                width: SHARP,
                height: SHARP,
                depth_or_array_layers: 1,
            },
        );
    }

    /// The bind group of the pass over `fresh` and `held` (none: `fresh` is bound for both).
    fn bind(
        &self,
        device: &wgpu::Device,
        fresh: &wgpu::TextureView,
        held: Option<&(wgpu::Texture, wgpu::TextureView)>,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("transition"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(fresh),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        held.map_or(fresh, |held| &held.1),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.cover_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Whether any texture is held (none should be once a transition is over).
    #[cfg(test)]
    pub fn holds_textures(&self) -> bool {
        self.targets.is_some()
    }

    /// Whether a picture is held besides the scene's frame.
    #[cfg(test)]
    pub fn has_held(&self) -> bool {
        self.targets
            .as_ref()
            .is_some_and(|targets| targets.held.is_some())
    }

    /// Whether the picture of a dissolve was held (see [`Plan::Capture`]).
    pub fn captured(&self) -> bool {
        self.captured.get()
    }

    /// Start `run` over the picture, one at a time: an overlay of the same kind under way goes on
    /// as it is (a long one is not started again from its few first pieces); one of the other kind
    /// gives way to it.
    pub fn start_overlay(&mut self, run: OverlayRun) {
        self.overlay = Some(next_overlay(self.overlay, run));
    }

    /// The transition is over, or never was: nothing is held any more.
    pub fn end(&mut self) -> Option<Scene> {
        self.targets = None;
        self.dressed = None;
        self.punch = None;
        self.overlay = None;
        self.captured.set(false);
        self.running.take().and_then(|running| running.pending)
    }

    /// The textures of `size`, for a frame of `plan`: created when the transition starts (or the
    /// size changed) and dropped when it ends.
    pub fn prepare(&mut self, device: &wgpu::Device, size: (u32, u32), plan: Plan) {
        if plan == Plan::None {
            self.targets = None;
            return;
        }
        let needs_held = plan.needs_held();
        if let Some(targets) = &self.targets
            && targets.size == size
            && (targets.held.is_some() || !needs_held)
        {
            return;
        }
        let make = |label| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            (texture, view)
        };
        let fresh = make("transition fresh");
        let held = needs_held.then(|| make("transition held"));
        let group = self.bind(device, &fresh.1, held.as_ref());
        self.targets = Some(Targets {
            size,
            fresh,
            held,
            group,
            kept: false,
        });
    }

    /// Whether the frame before is held, for a cut to be dressed with it.
    pub fn has_kept(&self) -> bool {
        self.targets.as_ref().is_some_and(|targets| targets.kept)
    }

    /// The course of the dressed cut is over: the picture held is the one before the cut, not
    /// the frame before.
    pub fn forget_kept(&mut self) {
        if let Some(targets) = &mut self.targets {
            targets.kept = false;
        }
    }

    /// After a frame of [`Plan::Keep`] (or of a punch or an overlay, which draw over
    /// the live frame as well, see [`Plan::is_live`]): hold what the scene drew, for a cut in the
    /// next frame.
    pub fn keep(&mut self, encoder: &mut wgpu::CommandEncoder, plan: Plan) {
        let Some(targets) = &mut self.targets else {
            return;
        };
        let (true, Some(held)) = (plan.is_live(), &targets.held) else {
            return;
        };
        encoder.copy_texture_to_texture(
            targets.fresh.0.as_image_copy(),
            held.0.as_image_copy(),
            wgpu::Extent3d {
                width: targets.size.0,
                height: targets.size.1,
                depth_or_array_layers: 1,
            },
        );
        targets.kept = true;
    }

    /// Where the scene draws in a frame of `plan`: the output itself without a transition,
    /// else the texture the pass reads (the held one in a dissolve's first frame).
    pub fn scene_target<'a>(
        &'a self,
        plan: Plan,
        output: &'a wgpu::TextureView,
    ) -> &'a wgpu::TextureView {
        let Some(targets) = &self.targets else {
            return output;
        };
        match (plan, &targets.held) {
            (Plan::None, _) => output,
            (Plan::Capture, Some(held)) => {
                self.captured.set(true);
                &held.1
            }
            _ => &targets.fresh.1,
        }
    }

    /// The pass of `plan` into `output`, which writes the frame's end timestamp (query 1) in
    /// `timestamps`, where there is one. `tint` is the colour a flash floods the screen with, in
    /// the output's bytes (see [`flood_color`]); `accent` (linear) is the theme colour that a
    /// cut-in draws in. Nothing for [`Plan::None`].
    pub fn composite(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        (plan, tint): (Plan, [f32; 3]),
        accent: Option<[f32; 3]>,
        timestamps: Option<&wgpu::QuerySet>,
    ) {
        let Some(targets) = &self.targets else {
            return;
        };
        let black = [0.0, 0.0, 0.0];
        let mut params = Params::default();
        let (kind, progress, seed, color) = match plan {
            Plan::None => return,
            // Nothing of a fade, shown as it is.
            Plan::Keep => (KIND_FADE, 0.0, 0, black),
            Plan::Fade(amount) => (KIND_FADE, amount, 0, black),
            Plan::FlashRise(amount) => {
                params.extra = flash_wash(accent);
                (KIND_FLASH_RISE, amount, 0, tint)
            }
            Plan::Flash(amount) => (KIND_FLASH, amount, 0, tint),
            Plan::HeldFade(amount) => (KIND_FADE_HELD, amount, 0, black),
            Plan::HeldFlash(amount) => {
                params.extra = flash_wash(accent);
                (KIND_FLASH_HELD, amount, 0, tint)
            }
            // The held picture is also what `fresh` reads as in this frame; the pass shows it.
            Plan::Capture => (KIND_DISSOLVE, 0.0, 0, black),
            Plan::Dissolve(progress) => (KIND_DISSOLVE, progress, 0, black),
            Plan::Glitch(progress, seed) => {
                let step = glitch_step(progress, seed);
                params.extra = [step.amount, step.reveal, 0.0, 0.0];
                let theme = accent.unwrap_or(FALLBACK_ACCENT).map(encode_srgb);
                (KIND_GLITCH, progress, step.pattern, theme)
            }
            Plan::Mosaic(progress, seed) => {
                // A switch: its course is worked out in the shader from the progress.
                params.extra = [0.0, 0.0, 0.0, 0.0];
                let theme = accent.unwrap_or(FALLBACK_ACCENT).map(encode_srgb);
                (KIND_MOSAIC, progress, seed, theme)
            }
            Plan::Overlay(frame) => {
                let theme = accent.unwrap_or(FALLBACK_ACCENT).map(encode_srgb);
                match frame.kind {
                    // Wide tears of the live picture that jump with the beat; the
                    // accent of the beat in the progress.
                    Overlay::Glitch => {
                        params.extra = [frame.amount, frame.beats, frame.linear, 1.0];
                        (KIND_GLITCH, frame.accent, frame.seed, theme)
                    }
                    // Big hard blocks that hit a region on the beat.
                    Overlay::Mosaic => {
                        params.extra = [1.0, frame.amount, frame.beats, frame.linear];
                        (KIND_MOSAIC, frame.accent, frame.seed, theme)
                    }
                }
            }
            Plan::CutIn(frame) => {
                params.cutin = [
                    frame.seconds,
                    CUTIN_BAND_SECONDS as f32,
                    frame.hold,
                    frame.tear,
                ];
                params.extra = [
                    frame.style.index(),
                    f32::from(u8::from(self.has_cover)),
                    f32::from(u8::from(frame.tearing())),
                    0.0,
                ];
                let accent = accent.unwrap_or(FALLBACK_ACCENT).map(encode_srgb);
                (KIND_CUTIN, 0.0, 0, accent)
            }
            Plan::Punch(frame) => {
                params.cutin = [frame.side, 0.0, 0.0, 0.0];
                params.extra = [frame.scale, frame.smear, frame.split, frame.flash];
                (KIND_PUNCH, 0.0, 0, tint)
            }
        };
        params.color = [color[0], color[1], color[2], 1.0];
        queue.write_buffer(&self.uniform, 0, &params.bytes(kind, progress, seed));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("transition"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: timestamps.map(|query_set| wgpu::RenderPassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some(1),
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &targets.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The texture of the cover for a cut-in's band, `side` by `side`.
fn cover_texture(device: &wgpu::Device, side: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("transition cover"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// `kind` in `transition.wgsl`.
const KIND_FADE: u32 = 0;
const KIND_DISSOLVE: u32 = 1;
const KIND_FLASH: u32 = 2;
const KIND_GLITCH: u32 = 3;
const KIND_MOSAIC: u32 = 4;
const KIND_FADE_HELD: u32 = 5;
const KIND_FLASH_HELD: u32 = 6;
const KIND_CUTIN: u32 = 7;
const KIND_PUNCH: u32 = 8;
const KIND_FLASH_RISE: u32 = 9;
/// Bytes of `struct Transition`: the kind, progress and seed, a word of padding, and three vec4f.
const UNIFORM_SIZE: u64 = 64;

/// The theme colour (linear) of a cut-in when the app has not told one: a calm blue.
const FALLBACK_ACCENT: [f32; 3] = [0.08, 0.25, 0.8];

/// A linear value as the byte value (0 to 1) it is displayed as: the frames are blended as bytes.
fn encode_srgb(linear: f32) -> f32 {
    let v = linear.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// The brightest a flash gets, as the brightest channel of its flood over 255: night first, the
/// screen never floods to full white.
pub const FLASH_PEAK: f32 = 0.82;
/// How much of white the flood has over the theme colour: a light tint, not the colour itself.
const FLASH_LIGHTEN: f32 = 0.5;

/// The share of a flash's rise that washes the picture in the theme colour, after the reference's
/// colour-block frames, before it is overexposed into the flood; the switch is at the peak.
pub const FLASH_WASH: f32 = 1.0 / 3.0;

/// What a flash's rise washes the picture in, as `extra` of `struct Transition`: the theme colour
/// at its brightest (its brightest channel at 1) in the output's bytes, and [`FLASH_WASH`].
/// Without an accent the calm blue of a cut-in.
fn flash_wash(accent: Option<[f32; 3]>) -> [f32; 4] {
    let accent = accent.unwrap_or(FALLBACK_ACCENT);
    let brightest = accent.iter().copied().fold(1e-3, f32::max);
    let [r, g, b] = accent.map(|c| encode_srgb(c / brightest));
    [r, g, b, FLASH_WASH]
}

/// The colour a flash floods the screen with, in the output's bytes (0 to 1): the accent colour
/// (linear, as the app tells it) lightened, and scaled so that its brightest channel is at most
/// [`FLASH_PEAK`]. Without an accent a pale cool grey.
pub fn flood_color(accent: Option<[f32; 3]>) -> [f32; 3] {
    let encode = encode_srgb;
    let lightened = match accent {
        Some(accent) => accent.map(|c| encode(c + (1.0 - c) * FLASH_LIGHTEN)),
        None => [0.74, 0.8, 0.9],
    };
    let brightest = lightened.iter().copied().fold(f32::MIN_POSITIVE, f32::max);
    lightened.map(|c| c * (FLASH_PEAK / brightest).min(1.0))
}

/// What `struct Transition` holds besides the kind, progress and seed.
#[derive(Default)]
struct Params {
    /// The colour a fade goes to, a flash's flood, the theme colour of a cut-in, a glitch or a
    /// mosaic (display values).
    color: [f32; 4],
    /// A cut-in: seconds since it started, how long the band opens, holds and tears. A punch: the
    /// side of its smear in the first.
    cutin: [f32; 4],
    /// A cut-in: the style, whether the band shows a cover, whether it tears (the frame is the
    /// new scene's). A punch: the scale, the smear, the colour split and the flash. A glitch: its
    /// stage's amount and reveal (see [`GlitchStep`]); over the picture its amount, its time in
    /// beats, 1 for the subtle one and 1 for the one that moves continuously. A mosaic: 0 for a
    /// switch (its course is the progress); over the picture 1, its amount and its time in beats.
    /// A flash's rise: the colour it washes the picture in and the share of the rise that is the
    /// wash (see [`flash_wash`]).
    extra: [f32; 4],
}

impl Params {
    /// The bytes of `struct Transition`.
    fn bytes(&self, kind: u32, progress: f32, seed: u32) -> [u8; UNIFORM_SIZE as usize] {
        let mut bytes = [0; UNIFORM_SIZE as usize];
        bytes[0..4].copy_from_slice(&kind.to_le_bytes());
        bytes[4..8].copy_from_slice(&progress.to_le_bytes());
        bytes[8..12].copy_from_slice(&seed.to_le_bytes());
        for (at, vec) in [(16, self.color), (32, self.cutin), (48, self.extra)] {
            for (i, v) in vec.iter().enumerate() {
                bytes[at + 4 * i..at + 4 * i + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strong_transitions_are_the_ones_strong_moments_play() {
        for name in [
            "cut", "fade", "dissolve", "flash", "glitch", "mosaic", "cutin",
        ] {
            let kind = Transition::from_name(name).unwrap();
            assert_eq!(
                kind.is_strong(),
                crate::moments::STRONG.contains(&name),
                "{name}"
            );
        }
    }

    #[test]
    fn a_fade_goes_out_then_in_then_is_done() {
        assert_eq!(fade_phase(0.0, 0.5, 1.0), FadePhase::Out(0.0));
        assert_eq!(fade_phase(0.25, 0.5, 1.0), FadePhase::Out(0.5));
        assert_eq!(fade_phase(0.5, 0.5, 1.0), FadePhase::In(0.0));
        assert_eq!(fade_phase(1.0, 0.5, 1.0), FadePhase::In(0.5));
        assert_eq!(fade_phase(1.5, 0.5, 1.0), FadePhase::Done);
        // Before the start counts as the start; no fade-out begins with the fade-in.
        assert_eq!(fade_phase(-1.0, 0.5, 1.0), FadePhase::Out(0.0));
        assert_eq!(fade_phase(0.0, 0.0, 1.0), FadePhase::In(0.0));
        assert_eq!(fade_phase(0.4, 0.4, 0.0), FadePhase::Done);
    }

    #[test]
    fn a_dissolve_runs_from_0_to_1_and_stays() {
        assert_eq!(dissolve_progress(0.0, 2.0), 0.0);
        assert_eq!(dissolve_progress(0.5, 2.0), 0.25);
        assert_eq!(dissolve_progress(2.0, 2.0), 1.0);
        assert_eq!(dissolve_progress(9.0, 2.0), 1.0);
        assert_eq!(dissolve_progress(-1.0, 2.0), 0.0);
        assert_eq!(dissolve_progress(0.0, 0.0), 1.0);
    }

    #[test]
    fn a_transition_without_a_duration_is_a_cut() {
        let none = Durations {
            out: 0.0,
            into: 0.0,
        };
        let short = Durations {
            out: 0.0,
            into: 0.2,
        };
        assert!(none.is_instant(Transition::Fade));
        assert!(!short.is_instant(Transition::Fade));
        assert!(short.is_instant(Transition::Cut));
        assert!(none.is_instant(Transition::Dissolve));
        assert!(!short.is_instant(Transition::Dissolve));
    }

    #[test]
    fn a_cutin_runs_until_its_tear_is_over() {
        let style = CutInStyle::Echo;
        // 0.5 s up to the tear, 0.05 s of tearing: the band opens in the first ten frames and
        // holds for the rest of the 0.5 s, then tears.
        let frame = cutin_frame(0.25, 0.5, 0.05, style).unwrap();
        assert_eq!(
            (frame.style, frame.seconds, frame.tear),
            (style, 0.25, 0.05)
        );
        assert!((f64::from(frame.hold) - (0.5 - CUTIN_BAND_SECONDS)).abs() < 1e-6);
        assert!(!frame.tearing());
        assert!(!cutin_frame(0.499, 0.5, 0.05, style).unwrap().tearing());
        assert!(cutin_frame(0.501, 0.5, 0.05, style).unwrap().tearing());
        assert!(cutin_frame(0.549, 0.5, 0.05, style).is_some());
        assert_eq!(cutin_frame(0.55, 0.5, 0.05, style), None);
        // Before the start counts as the start.
        assert_eq!(cutin_frame(-1.0, 0.5, 0.05, style).unwrap().seconds, 0.0);
        // A cut-in that gives no time for the band to hold still holds a moment.
        let short = cutin_frame(0.0, 0.01, 0.05, style).unwrap();
        assert!(f64::from(short.hold) >= CUTIN_MIN_HOLD_SECONDS);
        // And it still tears for the whole of its time.
        let tear_starts = CUTIN_BAND_SECONDS + CUTIN_MIN_HOLD_SECONDS;
        assert!(
            cutin_frame(tear_starts + 0.04, 0.01, 0.05, style)
                .unwrap()
                .tearing()
        );
        assert_eq!(cutin_frame(tear_starts + 0.051, 0.01, 0.05, style), None);
        let none = Durations {
            out: 0.0,
            into: 0.0,
        };
        assert!(none.is_instant(Transition::CutIn(style)));
        let some = Durations {
            out: 0.5,
            into: 0.05,
        };
        assert!(!some.is_instant(Transition::CutIn(style)));
    }

    #[test]
    fn the_same_kind_of_section_gets_the_same_cutin_style() {
        for label in 0..8 {
            let label = label as f32;
            assert_eq!(
                CutInStyle::of_section(label, true),
                CutInStyle::of_section(label, true)
            );
        }
        // Every style is used by some label, and without a chart there is the default.
        for style in CutInStyle::ALL {
            assert!(
                (0..26).any(|label| CutInStyle::of_section(label as f32, true) == style),
                "{style:?} is never picked"
            );
        }
        assert_eq!(CutInStyle::of_section(5.0, false), CutInStyle::default());
        // The lab goes through all of them and back.
        let mut style = CutInStyle::default();
        for _ in 0..CutInStyle::ALL.len() {
            style = style.next();
        }
        assert_eq!(style, CutInStyle::default());
    }

    #[test]
    fn the_uniform_keeps_each_vector_where_the_shader_reads_it() {
        let params = Params {
            color: [1.0, 2.0, 3.0, 4.0],
            cutin: [5.0, 6.0, 7.0, 8.0],
            extra: [9.0, 10.0, 11.0, 12.0],
        };
        let bytes = params.bytes(KIND_CUTIN, 0.5, 7);
        let word = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(
            u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            KIND_CUTIN
        );
        assert_eq!(word(4), 0.5);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 7);
        assert_eq!((word(16), word(28)), (1.0, 4.0));
        assert_eq!((word(32), word(44)), (5.0, 8.0));
        assert_eq!((word(48), word(60)), (9.0, 12.0));
        assert_eq!(bytes.len() as u64, UNIFORM_SIZE);
    }

    #[test]
    fn the_strong_transitions_need_time_like_their_kin() {
        let short = Durations {
            out: 0.0,
            into: 0.2,
        };
        assert!(
            Durations {
                out: 0.0,
                into: 0.0
            }
            .is_instant(Transition::Flash)
        );
        assert!(!short.is_instant(Transition::Flash));
        assert!(short.is_instant(Transition::Cut));
        assert!(!short.is_instant(Transition::Glitch) && !short.is_instant(Transition::Mosaic));
        assert!(
            Durations {
                out: 0.3,
                into: 0.0
            }
            .is_instant(Transition::Mosaic)
        );
    }

    #[test]
    fn names_of_the_transitions_the_runtime_draws_are_known() {
        for (name, kind) in [
            ("fade", Transition::Fade),
            ("flash", Transition::Flash),
            ("glitch", Transition::Glitch),
            ("mosaic", Transition::Mosaic),
        ] {
            assert_eq!(Transition::from_name(name), Some(kind));
        }
        assert_eq!(
            Transition::from_name("cutin"),
            Some(Transition::CutIn(CutInStyle::default()))
        );
        // Not a name.
        assert_eq!(Transition::from_name("sparkle"), None);
    }

    #[test]
    fn a_flash_rises_then_falls_and_installs_at_the_peak() {
        // Rise 1 s, fall 2 s: at the peak the way in begins at full amount.
        let at = |seconds| Plan::out_and_in(Transition::Flash, seconds, 1.0, 2.0, false);
        assert_eq!(at(0.5), (Plan::FlashRise(0.5), FadePhase::Out(0.5)));
        assert_eq!(at(1.0), (Plan::Flash(1.0), FadePhase::In(0.0)));
        assert_eq!(at(2.0), (Plan::Flash(0.5), FadePhase::In(0.5)));
        assert_eq!(at(3.0), (Plan::None, FadePhase::Done));
    }

    /// A flash's rise first washes the picture in the theme colour at its brightest (the first
    /// third of the rise), held or live, and its fall does not.
    #[test]
    fn a_flash_rise_washes_in_the_theme_colour_first() {
        let wash = flash_wash(Some([0.4, 0.05, 0.01]));
        assert!((wash[0] - 1.0).abs() < 1e-6 && wash[1] < wash[0] && wash[2] < wash[1]);
        assert_eq!(wash[3], FLASH_WASH);
        const { assert!(FLASH_WASH > 0.0 && FLASH_WASH < 0.5) };
        assert!(
            flash_wash(None)[..3]
                .iter()
                .all(|c| (0.0..=1.0).contains(c))
        );
        let rise = Plan::out_and_in(Transition::Flash, 0.2, 1.0, 2.0, true).0;
        assert_eq!(rise, Plan::HeldFlash(0.2));
        let fall = Plan::out_and_in(Transition::Flash, 1.5, 1.0, 2.0, true).0;
        assert_eq!(fall, Plan::Flash(0.75));
    }

    #[test]
    fn a_flood_is_a_light_tint_and_never_white() {
        for accent in [
            None,
            Some([0.0, 0.0, 0.0]),
            Some([1.0, 1.0, 1.0]),
            Some([0.8, 0.1, 0.05]),
            Some([0.05, 0.3, 1.0]),
        ] {
            let flood = flood_color(accent);
            let brightest = flood.iter().copied().fold(0.0, f32::max);
            assert!(brightest <= FLASH_PEAK + 1e-6, "{accent:?} {flood:?}");
            assert!(flood.iter().all(|c| *c > 0.0), "{accent:?} {flood:?}");
        }
        // The colour of the theme shows: a red accent floods redder than blue.
        let red = flood_color(Some([0.8, 0.1, 0.05]));
        assert!(red[0] > red[2], "{red:?}");
    }

    #[test]
    fn a_dressed_cut_plays_its_course_after_the_cut() {
        let dressed = |kind, out, into| Dressed {
            kind,
            durations: Durations { out, into },
            start: 10.0,
        };
        // A flash: the held picture rises, then the live scene falls.
        let flash = dressed(Transition::Flash, 0.2, 0.4);
        assert_eq!(flash.plan(0.0), Plan::HeldFlash(0.0));
        assert_eq!(flash.plan(0.1), Plan::HeldFlash(0.5));
        assert_eq!(flash.plan(0.2), Plan::Flash(1.0));
        assert_eq!(flash.plan(0.7), Plan::None);
        assert_eq!(flash.progress(0.3), 0.5);
        // A fade goes out on the held picture and comes in on the scene.
        let fade = dressed(Transition::Fade, 0.1, 0.1);
        assert_eq!(fade.plan(0.05), Plan::HeldFade(0.5));
        assert_eq!(fade.plan(0.15), Plan::Fade(0.5));
        // The others play over the held picture, and the pattern is the cut's.
        let glitch = dressed(Transition::Glitch, 0.0, 0.15);
        assert!(
            matches!(glitch.plan(0.0), Plan::Glitch(p, seed) if p == 0.0 && seed == seed_of(10.0))
        );
        assert_eq!(glitch.plan(0.15), Plan::None);
        let mosaic = dressed(Transition::Mosaic, 0.0, 0.5);
        assert!(matches!(mosaic.plan(0.25), Plan::Mosaic(p, _) if p == 0.5));
        let dissolve = dressed(Transition::Dissolve, 0.0, 0.5);
        assert_eq!(dissolve.plan(0.25), Plan::Dissolve(0.5));
        assert_eq!(dressed(Transition::Cut, 0.0, 0.0).plan(0.0), Plan::None);
    }

    #[test]
    fn only_a_plan_that_shows_the_live_scene_alone_needs_no_held_picture() {
        assert!(!Plan::None.needs_held());
        assert!(!Plan::Fade(0.5).needs_held() && !Plan::Flash(0.5).needs_held());
        assert!(!Plan::FlashRise(0.5).needs_held());
        // Keeping a frame is what the held picture is for.
        assert!(Plan::Keep.needs_held() && Plan::Mosaic(0.1, 0).needs_held());
        assert!(Plan::HeldFlash(0.1).needs_held());
    }

    #[test]
    fn a_punch_pops_and_settles_within_its_time() {
        let first = punch_frame(0.0, 4).unwrap();
        assert!((first.scale - (1.0 + PUNCH_ZOOM)).abs() < 1e-6);
        assert!((first.smear - PUNCH_SMEAR).abs() < 1e-6);
        assert!((first.flash - PUNCH_FLASH).abs() < 1e-6);
        // The pop settles by an ease-out: most of it is gone by halfway.
        let half = punch_frame(PUNCH_SECONDS / 2.0, 4).unwrap();
        assert!(half.scale - 1.0 < (first.scale - 1.0) * 0.2, "{half:?}");
        // It only falls, and is over at its end and for a cut that is not in a burst.
        let mut last = first;
        for i in 1..12 {
            let frame = punch_frame(PUNCH_SECONDS * f64::from(i) / 12.0, 4).unwrap();
            assert!(frame.scale < last.scale && frame.scale > 1.0);
            assert!(frame.smear < last.smear && frame.split < last.split);
            assert!(frame.flash < last.flash);
            last = frame;
        }
        assert_eq!(punch_frame(PUNCH_SECONDS, 4), None);
        assert_eq!(punch_frame(0.0, 0), None);
        // Before the start counts as the start.
        assert_eq!(punch_frame(-1.0, 2), punch_frame(0.0, 2));
        // A faint flash: well under a flash transition's.
        const { assert!(PUNCH_FLASH <= 0.2) };
    }

    #[test]
    fn a_punch_gets_stronger_through_the_burst_and_changes_side() {
        let levels: Vec<f32> = (0..=4).map(punch_intensity).collect();
        assert_eq!(levels, [0.0, 0.4, 0.6, 0.8, 1.0]);
        let frames: Vec<PunchFrame> = (1..=4)
            .map(|fill| punch_frame(0.0, fill).unwrap())
            .collect();
        for pair in frames.windows(2) {
            assert!(pair[1].scale > pair[0].scale);
            assert!(pair[1].smear > pair[0].smear);
            assert!(pair[1].split > pair[0].split);
            assert!(pair[1].flash > pair[0].flash);
            assert_eq!(pair[1].side, -pair[0].side);
        }
    }

    /// The steps of a course of `frames` frames, frame by frame.
    fn course<T>(frames: u32, step: impl Fn(f32) -> T) -> Vec<T> {
        (0..frames)
            .map(|i| step(i as f32 / frames as f32))
            .collect()
    }

    /// How many frames in a row each step holds, in order.
    fn holds(indices: &[u32]) -> Vec<u32> {
        let mut holds: Vec<u32> = Vec::new();
        for (i, index) in indices.iter().enumerate() {
            if i > 0 && indices[i - 1] == *index {
                *holds.last_mut().unwrap() += 1;
            } else {
                holds.push(1);
            }
        }
        holds
    }

    /// A mosaic switch is one continuous course of the progress (worked out in the shader): its
    /// plan only carries the progress, which only grows, and the new scene goes in at half way.
    #[test]
    fn a_mosaic_switch_is_a_continuous_course() {
        let course: Vec<f32> = (0..=30)
            .map(
                |i| match Plan::over_held(Transition::Mosaic, i as f32 / 30.0, 7) {
                    Plan::Mosaic(progress, 7) => progress,
                    plan => panic!("{plan:?}"),
                },
            )
            .collect();
        assert!(course.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn a_glitch_holds_its_stages_stutters_and_hides_the_cut_in_its_heaviest() {
        let steps = course(30, |p| glitch_step(p, 11));
        let indices: Vec<u32> = steps.iter().map(|s| s.index).collect();
        assert_eq!(holds(&indices).len(), GLITCH_SWITCH.len());
        assert!(holds(&indices).iter().all(|&h| h >= 4), "{indices:?}");
        // The old picture first, the new one at the end, never back.
        assert_eq!(steps[0].reveal, 0.0);
        assert_eq!(steps.last().unwrap().reveal, 1.0);
        assert!(steps.windows(2).all(|w| w[0].reveal <= w[1].reveal));
        // The new picture comes through the heaviest stage, and the breaks die down after it.
        let heaviest = steps
            .iter()
            .max_by(|a, b| a.amount.total_cmp(&b.amount))
            .unwrap();
        assert!(
            heaviest.reveal > 0.0 && heaviest.reveal < 1.0,
            "{heaviest:?}"
        );
        assert!(steps.last().unwrap().amount < 0.3);
        // A stage stutters back to an earlier one's pattern; the others are new patterns.
        let patterns: Vec<u32> = (0..GLITCH_SWITCH.len())
            .map(|i| glitch_step((i as f32 + 0.5) / 6.0, 11).pattern)
            .collect();
        assert_eq!(patterns[0], patterns[2]);
        assert_ne!(patterns[0], patterns[1]);
        // Another seed, other patterns.
        assert_ne!(glitch_step(0.0, 12).pattern, patterns[0]);
    }

    /// Music at 120 bpm in 4/4, followed, at `intensity`, with the clock at `clock` beats.
    fn music(intensity: f32, clock: f32) -> MusicValues {
        MusicValues {
            tempo: 120.0,
            locked: 1.0,
            meter: 4.0,
            intensity,
            clock,
            ..MusicValues::default()
        }
    }

    /// The frames of an overlay at 60 fps that starts at a section's start, with the music at
    /// `intensity`: a phrase of 4 bars, 16 beats, 8 s.
    fn overlay_frames(kind: Overlay, intensity: f32) -> Vec<OverlayFrame> {
        let mut run = OverlayRun::new(kind, 9.0, &Phrase::default(), &music(intensity, 0.0));
        let mut frames = Vec::new();
        for i in 0.. {
            let Some(frame) = run.frame() else {
                break;
            };
            frames.push(frame);
            let time = 9.0 + f64::from(i + 1) / 60.0;
            run.tick(time, &music(intensity, (i + 1) as f32 / 30.0));
        }
        frames
    }

    /// An overlay lasts to the end of the phrase; its amount is the course's (rising to its
    /// most a third of the way and falling away) times the most of its kind, with the music.
    #[test]
    fn an_overlay_runs_its_phrase_and_follows_the_music() {
        for (kind, most) in [
            (Overlay::Mosaic, OVERLAY_MOSAIC_MOST),
            (Overlay::Glitch, OVERLAY_GLITCH_MOST),
        ] {
            let frames = overlay_frames(kind, 1.0);
            assert!((480..=481).contains(&frames.len()), "{}", frames.len());
            let amounts: Vec<f32> = frames.iter().map(|f| f.amount).collect();
            let peak = amounts.iter().copied().fold(0.0, f32::max);
            assert!((peak - most).abs() < 1e-3, "{peak}");
            // Few at first, the most a third of the way, few again near the end (blank steps
            // aside).
            let mean = |range: std::ops::Range<usize>| {
                let lit: Vec<f32> = amounts[range]
                    .iter()
                    .copied()
                    .filter(|&a| a > 0.0)
                    .collect();
                lit.iter().sum::<f32>() / lit.len().max(1) as f32
            };
            assert!(mean(0..30) < mean(150..170) * 0.25);
            assert!(mean(440..480) < mean(150..170) * 0.25);
            assert!(frames.iter().all(|f| !f.keep && f.kind == kind));
            // Still music: fewer pieces, never none at the peak.
            let still = overlay_frames(kind, 0.0);
            let still_peak = still.iter().map(|f| f.amount).fold(0.0, f32::max);
            let least = crate::course::LEAST;
            assert!((still_peak - least * most).abs() < 1e-3, "{still_peak}");
        }
    }

    /// An overlay moves continuously: its eased beat time never falls nor moves faster than the
    /// ease's peak allows, a beat in 30 frames at 120 bpm; its amount changes a little from one
    /// frame to the next, and its pattern is the run's from start to end.
    #[test]
    fn an_overlay_moves_continuously() {
        for kind in [Overlay::Glitch, Overlay::Mosaic] {
            let frames = overlay_frames(kind, 1.0);
            assert!(frames.iter().all(|f| f.seed == frames[0].seed));
            let k = crate::course::PUSH_SHARPNESS;
            let peak_speed = k / (1.0 - (-k).exp());
            assert!(
                (frames[240].beats - 8.0).abs() < 0.01,
                "{}",
                frames[240].beats
            );
            for w in frames.windows(2) {
                let step = w[1].beats - w[0].beats;
                assert!((0.0..=peak_speed / 30.0 + 1e-4).contains(&step), "{step}");
                let most = match kind {
                    Overlay::Glitch => OVERLAY_GLITCH_MOST,
                    Overlay::Mosaic => OVERLAY_MOSAIC_MOST,
                };
                assert!((w[1].amount - w[0].amount).abs() < 0.01 * most, "{w:?}");
            }
        }
        // Another start, another pattern.
        let run = |start| {
            OverlayRun::new(Overlay::Glitch, start, &Phrase::default(), &music(1.0, 0.0))
                .frame()
                .unwrap()
                .seed
        };
        assert_ne!(run(9.0), run(10.0));
    }

    #[test]
    fn an_overlay_draws_over_the_live_frame_and_keeps_it_only_when_asked() {
        let mut run = OverlayRun::new(Overlay::Mosaic, 4.0, &Phrase::default(), &music(1.0, 0.0));
        run.tick(5.0, &music(1.0, 2.0));
        let plan = run.plan();
        // It holds no picture, and the frame under it is live: a cut in it can be dressed.
        assert!(matches!(plan, Plan::Overlay(_)));
        assert!(!plan.needs_held() && plan.is_live());
        // In a scene that keeps its frames it holds one, for the cut.
        assert!(plan.keeping().needs_held() && plan.keeping().is_live());
        assert_eq!(Plan::Keep.keeping(), Plan::Keep);
        assert!(Plan::Keep.is_live() && !Plan::Mosaic(0.5, 1).is_live());
        assert!(!Plan::None.is_live());
    }

    #[test]
    fn overlays_play_one_at_a_time() {
        let mut overlay = None;
        let mut start = |kind, start: f64| {
            let run = OverlayRun::new(kind, start, &Phrase::default(), &music(1.0, 0.0));
            let next = next_overlay(overlay, run);
            overlay = Some(next);
            next
        };
        assert_eq!(start(Overlay::Glitch, 1.0).start, 1.0);
        // The same kind again goes on as it is.
        assert_eq!(start(Overlay::Glitch, 2.0).start, 1.0);
        // The other kind takes its place.
        let mosaic = start(Overlay::Mosaic, 3.0);
        assert_eq!((mosaic.kind, mosaic.start), (Overlay::Mosaic, 3.0));
    }

    #[test]
    fn a_punch_draws_over_the_live_frame_alone() {
        let punch = Punch {
            start: 3.0,
            fill: 2,
        };
        assert!(matches!(punch.plan(0.0), Plan::Punch(_)));
        assert_eq!(punch.plan(PUNCH_SECONDS), Plan::None);
        assert!(!punch.plan(0.0).needs_held());
    }
}
