//! What a developer tool can see of the render thread and ask of it: [`Inspect`], a copy of what
//! the thread saw in its last frame (see [`crate::Visualizer::inspect`]), and [`LabCommand`], the
//! things it can be told to do now (see [`crate::Visualizer::lab`]). The app uses neither: the
//! render thread only fills an [`Inspect`] while inspection is on.

use crate::scene::{DirectorView, Effects, MusicValues};
use crate::switching::FOLLOWED;
use crate::transition::{CutInStyle, Transition};

/// What the render thread saw in its last frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inspect {
    /// The name of the scene on screen.
    pub scene: String,
    /// Frames per second, measured over the last half second.
    pub fps: f32,
    /// The GPU time of a recent frame in milliseconds, where frames are timed.
    pub gpu_ms: Option<f32>,
    pub music: InspectMusic,
    /// The sections of the song's chart, once it is known (else empty).
    pub sections: Vec<InspectSection>,
    /// The camera director of the scene on screen, if the scene declares shots.
    pub director: Option<InspectDirector>,
    /// The transitions and lens effects the scene on screen declares.
    pub effects: InspectEffects,
    pub switching: InspectSwitching,
}

/// The music's values of the last frame, as the scenes and the switching hear them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectMusic {
    /// The song time in seconds.
    pub time: f32,
    pub tempo: f32,
    /// Beats per bar.
    pub meter: f32,
    /// Whether the beat is followed (so that the tempo can be used for durations).
    pub locked: bool,
    /// The beat in the bar, from 1.
    pub beat_in_bar: u32,
    /// The bars since the first one, from 0.
    pub bar: u32,
    /// The slow energy of the music, 0 to 1.
    pub drive: f32,
    /// The number of the section, from 0, and its label ("A", "B", ...).
    pub section: u32,
    pub section_label: String,
    pub section_energy: f32,
    pub next_energy: f32,
    /// The strength of the last cue (an envelope, 0 to 1) and the seconds to the next section
    /// (-1 without a chart).
    pub cue: f32,
    pub cue_in: f32,
    pub has_chart: bool,
}

/// One section of the song's chart.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectSection {
    /// Seconds.
    pub start: f64,
    pub end: f64,
    pub label: String,
    pub energy: f32,
}

/// The camera director of the scene on screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectDirector {
    pub shot_index: u32,
    pub shot_name: String,
    /// "wide", "medium", "close", "detail" or "top".
    pub size: String,
    pub cuts: u32,
    /// 0 normally, else the position in a burst of cuts.
    pub fill: u32,
    /// The dutch angle in radians, signed.
    pub roll: f32,
    /// Seconds since the shot began, and its planned length (-1 when unknown).
    pub age: f32,
    pub length: f32,
    /// How lively the section is, which sets how often the director cuts: "quiet", "normal" or
    /// "high".
    pub tier: String,
    /// Whether the shot is held (see [`LabCommand::Shot`]).
    pub held: bool,
    /// The scene's shots in order (the numbers of [`LabCommand::Shot`]).
    pub shots: Vec<InspectShot>,
    /// How often the director cuts: "sections", "calm", "normal" or "busy". The pace in effect,
    /// the scene's own, and whether a lab overrides it (see [`LabCommand::CyclePace`]).
    pub pace: String,
    pub own_pace: String,
    pub pace_overridden: bool,
}

/// One of the scene's shots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectShot {
    /// The shot's `name`, empty if it has none.
    pub name: String,
    /// "wide", "medium", "close", "detail" or "top".
    pub size: String,
}

/// The scene's `[effects]` lists, as it declares them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectEffects {
    pub switch: Vec<String>,
    pub cut: Vec<String>,
    pub lens: Vec<String>,
}

impl InspectEffects {
    pub(crate) fn of(effects: &Effects) -> Self {
        Self {
            switch: effects.switch.clone(),
            cut: effects.cut.clone(),
            lens: effects.lens.clone(),
        }
    }
}

/// The automatic switching of scenes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectSwitching {
    /// Seconds on this scene on the visualizer's clock, once it is counted.
    pub scene_seconds: Option<f64>,
    /// Whether the next scene is being built, or is built and held for the switch.
    pub preparing: bool,
    pub held: bool,
    /// The transition under way.
    pub transition: Option<InspectTransition>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InspectTransition {
    pub kind: LabTransition,
    /// 0 to 1 over the whole transition.
    pub progress: f32,
}

/// How a scene switch the lab asks for looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabTransition {
    Cut,
    Fade,
    Dissolve,
    /// The band with the cover, in this style.
    CutIn(CutInStyle),
}

impl From<Transition> for LabTransition {
    fn from(transition: Transition) -> Self {
        match transition {
            Transition::Cut => Self::Cut,
            Transition::Fade => Self::Fade,
            Transition::Dissolve => Self::Dissolve,
            Transition::CutIn(style) => Self::CutIn(style),
        }
    }
}

impl From<LabTransition> for Transition {
    fn from(transition: LabTransition) -> Self {
        match transition {
            LabTransition::Cut => Self::Cut,
            LabTransition::Fade => Self::Fade,
            LabTransition::Dissolve => Self::Dissolve,
            LabTransition::CutIn(style) => Self::CutIn(style),
        }
    }
}

/// What the render thread is told to do now, for a scene lab (see
/// [`crate::Visualizer::lab`]). More will come (lens effects, a scene's pace).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LabCommand {
    /// The camera director cuts now, by its rules for the size, as if a cut were due.
    Cut,
    /// `Some(i)`: cut to shot `i` (from 0) and hold it, so that the director cuts no more, until
    /// `None`. The hold ends with the scene: another scene has its own director.
    Shot(Option<u32>),
    /// Switch to the next scene now, as the automatic switch would, built in the background.
    NextScene(LabTransition),
    /// Step the pace of the scene on screen: its own, then sections, calm, normal and busy, then
    /// its own again. It changes how often the director cuts, and lasts as long as the scene.
    CyclePace,
}

/// The name of a shot size, from the number `struct Shot` has.
fn size_name(size: u32) -> &'static str {
    ["wide", "medium", "close", "detail", "top"]
        .get(size as usize)
        .copied()
        .unwrap_or("wide")
}

/// The letter of a section label, from the number the music's values have (0 for "A").
pub(crate) fn label_name(label: f64) -> String {
    let letter = b'A' + (label.max(0.0) as u32).min(25) as u8;
    char::from(letter).to_string()
}

impl InspectMusic {
    pub(crate) fn of(music: &MusicValues) -> Self {
        let meter = music.meter.max(1.0);
        Self {
            time: music.song_time,
            tempo: music.tempo,
            meter: music.meter,
            locked: music.locked > FOLLOWED,
            beat_in_bar: (music.bar.max(0.0).fract() * meter) as u32 + 1,
            bar: music.bar.max(0.0) as u32,
            drive: music.drive,
            section: music.section.max(0.0) as u32,
            section_label: label_name(f64::from(music.section_label)),
            section_energy: music.section_energy,
            next_energy: music.next_energy,
            cue: music.cue,
            cue_in: music.cue_in,
            has_chart: music.has_chart != 0,
        }
    }
}

impl InspectDirector {
    pub(crate) fn of(view: DirectorView) -> Self {
        let state = view.state;
        Self {
            shot_index: state.index,
            shot_name: view.shot_name,
            size: size_name(state.size).to_string(),
            cuts: state.cuts,
            fill: state.fill,
            roll: state.roll,
            age: state.age,
            length: state.length,
            tier: view.tier.to_string(),
            held: view.held,
            shots: view
                .shots
                .into_iter()
                .map(|(name, size)| InspectShot {
                    name,
                    size: size_name(size as u32).to_string(),
                })
                .collect(),
            pace: view.pace.name().to_string(),
            own_pace: view.own_pace.name().to_string(),
            pace_overridden: view.pace_overridden,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_music_reads_as_bar_and_beat() {
        let music = MusicValues {
            bar: 12.5,
            meter: 4.0,
            locked: 1.0,
            section_label: 1.0,
            ..MusicValues::default()
        };
        let read = InspectMusic::of(&music);
        assert_eq!((read.bar, read.beat_in_bar), (12, 3));
        assert!(read.locked);
        assert_eq!(read.section_label, "B");
        // Before the first bar and with no meter nothing breaks.
        let read = InspectMusic::of(&MusicValues::default());
        assert_eq!((read.bar, read.beat_in_bar), (0, 1));
    }
}
