//! What a developer tool can see of the render thread and ask of it: [`Inspect`], a copy of what
//! the thread saw in its last frame (see [`crate::Visualizer::inspect`]), and [`LabCommand`], the
//! things it can be told to do now (see [`crate::Visualizer::lab`]). The app uses neither: the
//! render thread only fills an [`Inspect`] while inspection is on.

use crate::bind::{Level, Report, Used};
use crate::scene::Pace;
use crate::scene::{DirectorView, Effects, LensView, LooksView, MusicValues};
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
    /// What every source of the registry ([`crate::bind::SOURCES`]) said in this frame, in the
    /// registry's order: the readings the bindings of the scene on screen hear.
    pub signals: Vec<InspectSignal>,
    /// The sections of the song's chart, once it is known (else empty).
    pub sections: Vec<InspectSection>,
    /// The camera director of the scene on screen, if the scene declares shots.
    pub director: Option<InspectDirector>,
    /// The looks of the scene on screen, if the scene declares any.
    pub looks: Option<InspectLooks>,
    /// The transitions and lens effects the scene on screen declares.
    pub effects: InspectEffects,
    pub switching: InspectSwitching,
    /// The lens effects of the scene on screen.
    pub lens: InspectLens,
    /// What a lab overrides of the choices the visualizer makes by itself.
    pub overrides: LabOverrides,
    /// The bindings of the scene on screen: what drives each of its inputs now.
    pub bindings: Vec<InspectBinding>,
    /// The inputs the scene on screen declares (the targets of its bindings).
    pub inputs: Vec<InspectInput>,
    /// The runtime's own bindings (`runtime.toml`) as of the last frame: for each, the target
    /// (a decision), the source, the level and which means gave the value (a decision that is off
    /// at the level shows as `Used::Rest`), the source's value and the value the decision read.
    pub runtime_bindings: Vec<InspectBinding>,
    /// The chance in percent a lab set for every moment to get an effect, or none for the user's
    /// setting (see [`LabCommand::EffectChance`]).
    pub chance: Option<u8>,
}

/// One `[[bind]]` of the scene on screen as it was in the last frame.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectBinding {
    /// The input it drives.
    pub target: String,
    pub source: String,
    pub how: String,
    /// The fallback level the music was at.
    pub level: Level,
    /// Which of its means gave the output: its source, a fallback, or none.
    pub used: Used,
    /// The value of the source, and the output.
    pub source_value: f32,
    pub output: f32,
    /// The bind as it was written, whether or not it is in use: the `when` as an inline TOML
    /// table (`{ strength = 0.5 }`, `{}` for none), the `fallback` as a TOML value (`"hits"`,
    /// `"rest"`, `{ value = 2.0 }`; empty for none) and the `ease` (empty for none).
    pub when: String,
    pub fallback: String,
    pub ease: String,
}

/// One `[[input]]` of the scene on screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectInput {
    pub id: String,
    /// The label for people (English if it has one), else the id.
    pub label: String,
    pub range: [f32; 2],
    /// The value with nothing driving it.
    pub rest: f32,
    /// The input as TOML (the keys of its `[[input]]` table), so that it can be written back as
    /// it was declared.
    pub declared: String,
}

impl From<Report> for InspectBinding {
    fn from(report: Report) -> Self {
        Self {
            target: report.target,
            source: report.source.to_string(),
            how: report.how.to_string(),
            level: report.tick.level,
            used: report.tick.used,
            source_value: report.tick.source,
            output: report.tick.output,
            when: report.when,
            fallback: report.fallback,
            ease: report.ease,
        }
    }
}

/// Which lens effects the scene on screen has (see [`LabCommand::Flare`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InspectLens {
    /// The scene lists the effect (the lab can start one all the same), and it is under way.
    pub flare_listed: bool,
    pub flare_running: bool,
    pub leak_listed: bool,
    pub leak_running: bool,
    /// The glitch and the mosaic over the picture (see [`LabCommand::LensGlitch`]).
    pub glitch_listed: bool,
    pub glitch_running: bool,
    pub mosaic_listed: bool,
    pub mosaic_running: bool,
}

impl From<LensView> for InspectLens {
    fn from(view: LensView) -> Self {
        Self {
            flare_listed: view.flare_listed,
            flare_running: view.flare_running,
            leak_listed: view.leak_listed,
            leak_running: view.leak_running,
            glitch_listed: view.glitch_listed,
            // The transition layer draws them and tells (see `Pipeline::lens_view`).
            glitch_running: false,
            mosaic_listed: view.mosaic_listed,
            mosaic_running: false,
        }
    }
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
    /// The envelope of the last impact, 0 to 1.
    pub impact: f32,
    pub has_chart: bool,
}

/// What one source of the binding engine said in the last frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectSignal {
    pub name: String,
    /// Whether it counts events (then `count` and `value` is the strength of the event).
    pub event: bool,
    /// The value the bindings read; for an event, its strength.
    pub value: f32,
    /// How many events came in the frame; 0 for a source that is not an event.
    pub count: u32,
    /// Whether the source means something at the fallback level the music was at.
    pub meaningful: bool,
}

impl InspectSignal {
    /// Every source's reading from the frame's music and whether the scene's director cut in it
    /// (what the bindings read through [`crate::bind::Source::read_in`]), with whether it means
    /// something at the level of that music.
    pub(crate) fn all(music: &MusicValues, cut: bool) -> Vec<Self> {
        let level = Level::of(music);
        crate::bind::SOURCES
            .iter()
            .map(|source| {
                let reading = source.read_in(music, cut);
                Self {
                    name: source.name.to_string(),
                    event: source.event,
                    value: reading.value,
                    count: reading.count,
                    meaningful: source.meaningful_at(level),
                }
            })
            .collect()
    }
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
    /// Whether the shot was entered by a hard cut (the scene jumps) and not a glide.
    pub hard: bool,
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
    /// The chance (0 to 1) that a section start that lifts gets its burst, whether the coming
    /// section will burst, and what a lab decided (see [`LabCommand::BurstsWith`]).
    pub burst_chance: f32,
    pub burst_next: bool,
    pub bursts: Option<bool>,
}

/// The looks of the scene on screen (see [`LabCommand::Look`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectLooks {
    /// The scene's looks in order (the numbers of [`LabCommand::Look`]): the `name` of each, empty
    /// if it has none.
    pub names: Vec<String>,
    /// The look on, and the one before it.
    pub on: u32,
    pub prev: u32,
    /// How far the fade from the one before is, 0 to 1 (eased).
    pub mix: f32,
    /// How many changes so far: a lab marks the timeline where it grows.
    pub changes: u32,
    /// Whether the look is held, so that the runtime does not change it.
    pub held: bool,
}

/// One of the scene's shots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectShot {
    /// The shot's `name`, empty if it has none.
    pub name: String,
    /// "wide", "medium", "close", "detail" or "top".
    pub size: String,
    /// Its place in the burst (1 to 4) for a `[[burst]]` shot; 0 for a `[[shot]]`.
    pub burst: u32,
    /// Whether the song still owes the shot a cut (it says `at_least_once` and has not been on
    /// in this song, with its last chance still ahead).
    pub owed: bool,
}

/// The scene's `effects` list, kept apart by kind as the runtime uses it.
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
    Flash,
    Glitch,
    Mosaic,
}

impl From<Transition> for LabTransition {
    fn from(transition: Transition) -> Self {
        match transition {
            Transition::Cut => Self::Cut,
            Transition::Fade => Self::Fade,
            Transition::Dissolve => Self::Dissolve,
            Transition::CutIn(style) => Self::CutIn(style),
            Transition::Flash => Self::Flash,
            Transition::Glitch => Self::Glitch,
            Transition::Mosaic => Self::Mosaic,
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
            LabTransition::Flash => Self::Flash,
            LabTransition::Glitch => Self::Glitch,
            LabTransition::Mosaic => Self::Mosaic,
        }
    }
}

/// Lens effects a lab turns on at every moment, as if the scene listed them (see
/// [`LabCommand::LensWith`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LabLens {
    pub flare: bool,
    pub leak: bool,
    /// The glitch and the mosaic over the picture: at every strong moment (they are strong).
    pub glitch: bool,
    pub mosaic: bool,
}

/// How often the camera director cuts, as a lab picks it (see [`LabCommand::PaceWith`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabPace {
    Sections,
    Calm,
    Normal,
    Busy,
}

impl LabPace {
    pub const ALL: [LabPace; 4] = [Self::Sections, Self::Calm, Self::Normal, Self::Busy];

    /// The word `scene.toml` uses for it.
    pub fn name(self) -> &'static str {
        Pace::from(self).name()
    }
}

impl From<LabPace> for Pace {
    fn from(pace: LabPace) -> Self {
        match pace {
            LabPace::Sections => Self::Sections,
            LabPace::Calm => Self::Calm,
            LabPace::Normal => Self::Normal,
            LabPace::Busy => Self::Busy,
        }
    }
}

/// What a lab replaces of the choices the visualizer makes by itself. Each replaces whatever was
/// chosen, at the last point before the switch or the cut is drawn; none leaves the choice be.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LabOverrides {
    /// The fallback level the music is held at, whatever the song has (see
    /// [`LabCommand::ForceLevel`]).
    pub level: Option<Level>,
    /// What every scene switch uses, the automatic ones and those asked for (see
    /// [`LabCommand::SwitchWith`]).
    pub switch: Option<LabTransition>,
    /// What dresses every camera cut of the director, a cut being plain (see
    /// [`LabCommand::CutsWith`]).
    pub cuts: Option<LabTransition>,
    /// Whether every section start that lifts gets a burst (`Some(true)`), none does
    /// (`Some(false)`), or the runtime's chance decides (none; see [`LabCommand::BurstsWith`]).
    pub bursts: Option<bool>,
    /// The lens effects on at every moment.
    pub lens: LabLens,
}

/// What the render thread is told to do now, for a scene lab (see
/// [`crate::Visualizer::lab`]).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LabCommand {
    /// The camera director cuts now, by its rules for the size, as if a cut were due.
    Cut,
    /// The chance in percent (0 to 100) that every moment of the music gets an effect, in place
    /// of the user's setting; none for the setting's again.
    EffectChance(Option<u8>),
    /// `Some(i)`: cut to shot `i` (from 0) and hold it, so that the director cuts no more, until
    /// `None`. The hold ends with the scene: another scene has its own director.
    Shot(Option<u32>),
    /// `Some(i)`: change to look `i` (from 0) now with the usual fade and hold it, so that the
    /// runtime changes the look no more, until `None`, which hands the look back to the runtime.
    /// A hold stays when the scene's files are saved.
    Look(Option<u32>),
    /// Switch to the next scene now, as the automatic switch would, built in the background.
    NextScene(LabTransition),
    /// Step the pace of the scene on screen: its own, then sections, calm, normal and busy, then
    /// its own again. It changes how often the director cuts, and lasts as long as the scene.
    CyclePace,
    /// The camera director cuts now, dressed with the transition (see
    /// [`crate::pipeline::Pipeline::cut_dressed`]), whether the scene lists it or not.
    CutDressed(LabTransition),
    /// Start a lens flare now, as a section start would (even if the scene lists none).
    Flare,
    /// Start a light leak now, as a section start would (even if the scene lists none).
    Leak,
    /// Play a glitch over the picture now, as a strong moment with no cut does in a scene that
    /// lists `glitch` (even if the scene lists none).
    LensGlitch,
    /// Play a mosaic over the picture now, the same way.
    LensMosaic,
    /// Every scene switch, the automatic ones and those of [`LabCommand::NextScene`], takes this
    /// transition (with the durations its kind takes), whatever was chosen; none leaves the choice.
    SwitchWith(Option<LabTransition>),
    /// Every camera cut of the director is dressed with this transition, whatever the scene and
    /// the moment say (a cut is plain), through the path of [`LabCommand::CutDressed`]; none
    /// leaves the choice. A cut-in is not drawn over a camera cut: it counts as none.
    CutsWith(Option<LabTransition>),
    /// Every section start that lifts the energy gets its burst of cuts (`Some(true)`), or none
    /// does (`Some(false)`), in place of `director.burst`'s chance and the burst a song is
    /// promised; none leaves it to the runtime. For every scene, as [`LabCommand::CutsWith`] is.
    /// The burst of [`LabCommand::Burst`] is always taken.
    BurstsWith(Option<bool>),
    /// These lens effects start at every moment, as if every scene listed them, for this scene
    /// and every one that comes, until set again with none on.
    LensWith(LabLens),
    /// The scene on screen uses this pace in place of its own (see [`LabCommand::CyclePace`]),
    /// or with none its own again.
    PaceWith(Option<LabPace>),
    /// Hold the music at this fallback level, as if the song had no more than that: C passes no
    /// chart, B keeps the chart and loses its beat. It can only take the music down from what the
    /// song has (A is the song as it is). None leaves the music be.
    ForceLevel(Option<Level>),
    /// Start a burst of cuts now: the director cuts on each of the next 4 beats (or every
    /// quarter second without a followed beat) to medium, close, detail and top, each with its
    /// punch, as the burst before a lift does.
    Burst,
    /// Replace the runtime's decisions (`runtime.toml`: when effects play, when the camera cuts,
    /// when scenes switch, what the effects' course follows) with the bindings of this text, the
    /// whole file, for every scene from the next frame on. The text is read in full first: if it
    /// is wrong the old bindings stay and the error, which names the bind, is in
    /// [`crate::Status::last_error`].
    RuntimeBindings(String),
    /// Replace the bindings of the scene on screen with those of this text: TOML holding the
    /// scene's `[[input]]` and `[[bind]]` arrays and nothing else (no array means none). The text
    /// is read and checked as the manifest is, in full first: if it is wrong the old bindings
    /// stay and the error, which names the bind, is in [`crate::Status::last_error`]. The new
    /// bindings start over from the next frame; only if the inputs changed is the scene rebuilt
    /// (as a reload of the file would, keeping its state where it can). Nothing is written to
    /// the scene's folder, and what is on screen is what a reload of the saved text finds, so
    /// saving the same text does not reload the scene.
    SceneBindings(String),
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
            impact: music.impact,
            has_chart: music.has_chart != 0,
        }
    }
}

impl InspectLooks {
    pub(crate) fn of(view: LooksView) -> Self {
        Self {
            names: view.names,
            on: view.state.to,
            prev: view.state.prev,
            mix: view.state.mix,
            changes: view.state.changes,
            held: view.held,
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
            hard: state.hard == 1,
            roll: state.roll,
            age: state.age,
            length: state.length,
            tier: view.tier.to_string(),
            held: view.held,
            shots: view
                .shots
                .into_iter()
                .zip(view.owed)
                .map(|((name, size, burst), owed)| InspectShot {
                    name,
                    size: size_name(size as u32).to_string(),
                    burst,
                    owed,
                })
                .collect(),
            pace: view.pace.name().to_string(),
            own_pace: view.own_pace.name().to_string(),
            pace_overridden: view.pace_overridden,
            burst_chance: view.burst_chance,
            burst_next: view.burst_next,
            bursts: view.bursts,
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

    #[test]
    fn every_source_is_read_as_the_bindings_read_it() {
        let music = MusicValues {
            tempo: 120.0,
            hit: 0.75,
            hits: 2,
            has_chart: 0,
            ..MusicValues::default()
        };
        let signals = InspectSignal::all(&music, true);
        // All of them, in the registry's order.
        let names: Vec<&str> = signals.iter().map(|s| s.name.as_str()).collect();
        let registry: Vec<&str> = crate::bind::SOURCES.iter().map(|s| s.name).collect();
        assert_eq!(names, registry);
        let find = |name: &str| signals.iter().find(|s| s.name == name).unwrap();
        assert_eq!(find("tempo").value, 120.0);
        assert!(!find("tempo").event);
        let hits = find("hits");
        assert!(hits.event);
        assert_eq!((hits.value, hits.count), (0.75, 2));
        // The cut source hears the frame's cut flag.
        assert_eq!(find("cut").count, 1);
        let quiet = InspectSignal::all(&music, false);
        assert_eq!(quiet.iter().find(|s| s.name == "cut").unwrap().count, 0);
        // Without a chart (level C) `bars` means nothing, and `tempo` does.
        assert!(!find("bars").meaningful);
        assert!(find("tempo").meaningful);
        for (signal, source) in signals.iter().zip(crate::bind::SOURCES) {
            let reading = source.read_in(&music, true);
            assert_eq!((signal.value, signal.count), (reading.value, reading.count));
        }
    }
}
