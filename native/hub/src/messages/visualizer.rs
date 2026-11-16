use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};
use visualizer::{ColumnGroup, ColumnKind, ColumnPlan, ShotSize, Strength, Tier};

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VisualizerShotSize {
    Wide,
    Medium,
    Close,
    Detail,
    Top,
}

impl From<ShotSize> for VisualizerShotSize {
    fn from(value: ShotSize) -> Self {
        match value {
            ShotSize::Wide => Self::Wide,
            ShotSize::Medium => Self::Medium,
            ShotSize::Close => Self::Close,
            ShotSize::Detail => Self::Detail,
            ShotSize::Top => Self::Top,
        }
    }
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VisualizerMomentStrength {
    Medium,
    Strong,
}

impl From<Strength> for VisualizerMomentStrength {
    fn from(value: Strength) -> Self {
        match value {
            Strength::Medium => Self::Medium,
            Strength::Strong => Self::Strong,
        }
    }
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VisualizerLabColumnGroup {
    Director,
    Looks,
    Signals,
    Inputs,
}

impl From<ColumnGroup> for VisualizerLabColumnGroup {
    fn from(value: ColumnGroup) -> Self {
        match value {
            ColumnGroup::Director => Self::Director,
            ColumnGroup::Looks => Self::Looks,
            ColumnGroup::Signals => Self::Signals,
            ColumnGroup::Inputs => Self::Inputs,
        }
    }
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VisualizerLabColumnKind {
    Number,
    Event,
    Bool,
    Category,
    Phase,
}

impl From<ColumnKind> for VisualizerLabColumnKind {
    fn from(value: ColumnKind) -> Self {
        match value {
            ColumnKind::Number => Self::Number,
            ColumnKind::Event => Self::Event,
            ColumnKind::Bool => Self::Bool,
            ColumnKind::Category => Self::Category,
            ColumnKind::Phase => Self::Phase,
        }
    }
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VisualizerLabColumnPlan {
    Yes,
    No,
}

impl From<ColumnPlan> for VisualizerLabColumnPlan {
    fn from(value: ColumnPlan) -> Self {
        match value {
            ColumnPlan::Yes => Self::Yes,
            ColumnPlan::No => Self::No,
        }
    }
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VisualizerSectionTier {
    Quiet,
    Normal,
    High,
}

impl From<Tier> for VisualizerSectionTier {
    fn from(value: Tier) -> Self {
        match value {
            Tier::Quiet => Self::Quiet,
            Tier::Normal => Self::Normal,
            Tier::High => Self::High,
        }
    }
}

/// Inputs available from the current source to installed scenes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, SignalPiece)]
#[serde(rename_all = "snake_case")]
pub enum VisualizerInputs {
    #[default]
    None,
    SpectrumOnly,
    Full,
}

/// Inputs required by a scene to produce the picture it was designed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SignalPiece)]
#[serde(rename_all = "snake_case")]
pub enum VisualizerSceneInputs {
    Spectrum,
    SpectrumAndChart,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct StartVisualizerRequest {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct StopVisualizerRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ResizeVisualizerRequest {
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Directories with scenes (folders with a `scene.toml`) listed beside the built-in ones: the
/// app's folder of imported scenes. A running visualizer whose scene is gone changes to the
/// built-in scene.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerPresetDirsRequest {
    pub paths: Vec<String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// The app's accent colour as 0xAARRGGBB, for scenes that draw in it.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerAccentRequest {
    pub argb: u32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Whether the app's window can be seen. While it cannot, the visualizer stands still.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerVisibleRequest {
    pub visible: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Shows the scene at `path`: a scene folder, or `builtin:<name>` for a built-in scene (as the
/// scenes of [`ListVisualizerPresetsResponse`] say).
#[derive(Serialize, Deserialize, DartSignal)]
pub struct LoadVisualizerPresetRequest {
    pub path: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct NextVisualizerPresetRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct PreviousVisualizerPresetRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RandomVisualizerPresetRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// How the scenes are picked (paths in the form of `VisualizerPresetEntry::path`), replacing what
/// was set before. "Next" goes through the scenes in `order`, and the visualizer begins with its
/// first one when there is no scene to resume; scenes not in it come after it. `disabled` are left out, but can
/// still be loaded by path; turning every scene off counts as turning none off.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerScenesRequest {
    pub order: Vec<String>,
    pub disabled: Vec<String>,
    /// The scene on screen when the app last ran, which a visualizer that starts resumes unless
    /// another scene was shown since the app started.
    pub last: Option<String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Unpack the scene file `file` (a `.runescene`, see `visualizer::package`) into a folder of its
/// own in `dir`, the app's folder of imported scenes, replacing a scene of that name. The scenes
/// of `dir` are listed again.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct ImportVisualizerSceneRequest {
    pub file: String,
    pub dir: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// `path` is the folder of the imported scene; a file that is not one is a failed request.
#[derive(Serialize, Deserialize, RustSignal)]
pub struct ImportVisualizerSceneResponse {
    pub file: String,
    pub path: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Delete the imported scene at `path`, which must be a folder directly in `dir`. The scenes of
/// `dir` are listed again.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemoveVisualizerSceneRequest {
    pub path: String,
    pub dir: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct RemoveVisualizerSceneResponse {
    pub path: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Whether the visualizer's diagnostics (the GPU it runs on, how frames reach the screen) go to the
/// Log page; its errors always do. The app sends its setting at start and whenever it changes,
/// and the Log page's entries are written from the first one on.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerLogRequest {
    pub enabled: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Whether this machine can run the visualizer, asked before starting it.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct VisualizerSupportRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// `supported`: there is a GPU (`adapter` names it). Otherwise `problem` says why not: no GPU
/// adapter, or only a software renderer, which would draw the scenes on the CPU; then only the
/// cover wall can be shown.
#[derive(Serialize, Deserialize, RustSignal)]
pub struct VisualizerSupportResponse {
    pub support: VisualizerSupport,
    pub inputs: VisualizerInputs,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum VisualizerSupport {
    Supported { adapter: String },
    Unsupported { problem: String },
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct VisualizerStatusRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct VisualizerStatusResponse {
    pub runtime: VisualizerRuntimeStatus,
    pub width: u32,
    pub height: u32,
    pub target_fps: u32,
    pub measured_fps: f32,
    pub preset_name: String,
    /// Where the scene on screen is, in the form of `VisualizerPresetEntry::path`;
    /// `app:cover-wall` while the cover wall is on and nothing is drawn.
    pub preset_path: String,
    pub preset_count: u32,
    pub inputs: VisualizerInputs,
    pub last_error: Option<String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum VisualizerRuntimeStatus {
    NotRunningYet,
    Stopped {
        adapter: String,
        transport: VisualizerTransport,
    },
    Running {
        adapter: String,
        transport: VisualizerTransport,
    },
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualizerTransport {
    Readback,
    IoSurface,
    DxgiShared,
    VulkanShared,
    Surface,
}

impl From<visualizer::Transport> for VisualizerTransport {
    fn from(value: visualizer::Transport) -> Self {
        match value {
            visualizer::Transport::Readback => Self::Readback,
            visualizer::Transport::IoSurface => Self::IoSurface,
            visualizer::Transport::DxgiShared => Self::DxgiShared,
            visualizer::Transport::VulkanShared => Self::VulkanShared,
            visualizer::Transport::Surface => Self::Surface,
        }
    }
}

/// The texture that shows the visualizer (macOS), sent once it started and whenever another one
/// replaces it. Show it, then confirm with `VisualizerTextureShownRequest`.
#[derive(Serialize, Deserialize, RustSignal)]
pub struct VisualizerTextureResponse {
    pub texture_id: i64,
}

/// The app shows the texture `texture_id`: the textures it replaced can go.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct VisualizerTextureShownRequest {
    pub texture_id: i64,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListVisualizerPresetsRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListVisualizerCreditsRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// What the assets of one built-in scene owe their makers.
#[derive(Serialize, Deserialize, SignalPiece)]
pub struct VisualizerCredit {
    /// The scene's name.
    pub scene: String,
    /// The `source` of each asset of the scene that has one, as the scene writes it in English:
    /// the work, its authors, its licence and what was changed.
    pub assets: Vec<String>,
}

/// The credits of the built-in scenes that have any.
#[derive(Serialize, Deserialize, RustSignal)]
pub struct ListVisualizerCreditsResponse {
    pub credits: Vec<VisualizerCredit>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, SignalPiece)]
pub struct VisualizerPresetEntry {
    pub name: String,
    pub path: String,
    /// The folder the scene is in, relative to its scene folder; empty for a built-in scene.
    pub group: String,
    /// Whether the scene is compiled into the program (its `path` is `builtin:<name>`).
    pub builtin: bool,
    /// Inputs required by the scene: chart-dependent scenes are unavailable with spectrum alone.
    pub inputs: VisualizerSceneInputs,
}

/// Every scene, the built-in ones, the cover wall (path `app:cover-wall`, which the app draws and
/// the visualizer leaves blank) and those of the scene folders, by group and name.
#[derive(Serialize, Deserialize, RustSignal)]
pub struct ListVisualizerPresetsResponse {
    pub presets: Vec<VisualizerPresetEntry>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Sent whenever the list of scenes is ready or changes: when the scene folder has been read,
/// and after a scene is imported or removed. Carries the same list as
/// [`ListVisualizerPresetsResponse`].
#[derive(Serialize, Deserialize, RustSignal)]
pub struct VisualizerScenesChanged {
    pub presets: Vec<VisualizerPresetEntry>,
}

/// Whether the visualizer copies what it sees in each frame for the in-app lab
/// ([`VisualizerLabResponse`]) and plans the song for it. Off by default: the lab sends it on while
/// its setting is on (the progress bars show the plan even with the panel closed) and off when
/// the setting is turned off, and the app pays nothing for it otherwise. Turning it off drops what
/// the lab had collected, and the plan.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerLabRequest {
    pub enabled: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// The fallback level the music is at: A has a chart and a followed beat, B a chart alone, C
/// neither.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualizerLabLevel {
    A,
    B,
    C,
}

/// How a scene switch or a dressed camera cut looks.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualizerLabTransition {
    Fade,
    Dissolve,
    Flash,
    Glitch,
    Mosaic,
}

/// Something the lab tells the visualizer to do now. Where a command carries an option, setting
/// it replaces what the visualizer chooses by itself, and none hands the choice back.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum VisualizerLabCommand {
    /// The camera director cuts now, as if a cut were due.
    Cut,
    /// Cut to the shot at `index` (from 0) and hold it, so that the director cuts no more, until
    /// none.
    Shot { index: Option<u32> },
    /// Change to the look at `index` (from 0) now and hold it, so that the runtime changes the
    /// look no more, until none.
    Look { index: Option<u32> },
    /// The camera director cuts now, dressed with this transition.
    CutDressed { transition: VisualizerLabTransition },
    /// Start a lens flare now.
    Flare,
    /// Start a light leak now.
    Leak,
    /// Play a glitch over the picture now.
    LensGlitch,
    /// Play a mosaic over the picture now.
    LensMosaic,
    /// Hold the music at this fallback level, which can only take it down from what the song
    /// has; none leaves it be.
    ForceLevel { level: Option<VisualizerLabLevel> },
    /// Start a burst of cuts now: the director cuts on each of the next 4 beats.
    Burst,
}

/// Tell the visualizer to do something now, for the lab. The command is dropped (and logged) when
/// the visualizer is not running.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct VisualizerLabCommandRequest {
    pub command: VisualizerLabCommand,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// What the visualizer decided or was told to do, at the song time it happened (seconds).
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub enum VisualizerLabEventKind {
    /// The camera cut to a shot.
    Cut {
        shot: String,
        size: VisualizerShotSize,
        /// 0 for a normal cut, else the position in a burst.
        fill: u32,
        /// The cut lands on a strong transition.
        strong: bool,
        /// The transition that dresses the cut, by name.
        dress: Option<String>,
        /// The scene's first shot, which it begins with, or the shot on where the visualizer was
        /// put at a song time without playing up to it (a seek): a jump rather than a cut.
        first: bool,
    },
    /// The look changed to `look`.
    Look { look: String },
    /// A moment of the music played an effect. `strength` is "medium" or "strong"; `effect` is
    /// what it did by name, none when the scene lists nothing that suits it.
    Moment {
        strength: VisualizerMomentStrength,
        effect: Option<String>,
    },
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub struct VisualizerLabEvent {
    /// Song seconds.
    pub time: f64,
    pub kind: VisualizerLabEventKind,
}

/// One of the scene's shots.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub struct VisualizerLabShot {
    /// The shot's name, empty if it has none.
    pub name: String,
    pub size: VisualizerShotSize,
}

/// The camera director of the scene on screen.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub struct VisualizerLabDirector {
    /// The shot on, as a place in `shots`.
    pub shot_index: u32,
    /// Whether the shot is held.
    pub held: bool,
    /// The scene's shots in order, which the numbers of `Shot` count.
    pub shots: Vec<VisualizerLabShot>,
}

/// The looks of the scene on screen.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub struct VisualizerLabLooks {
    /// The scene's looks in order, which the numbers of `Look` count; a look without a name is
    /// empty.
    pub names: Vec<String>,
    /// The look on, as a place in `names`.
    pub on: u32,
    /// Whether the look is held.
    pub held: bool,
}

/// The transitions and lens effects the scene on screen declares.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub struct VisualizerLabEffects {
    /// What may dress a camera cut, by name.
    pub cut: Vec<String>,
    /// The lens effects, by name.
    pub lens: Vec<String>,
}

/// What the visualizer saw in its last frame, for the live readout of the lab.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub struct VisualizerLabNow {
    /// None when the scene declares no shots.
    pub director: Option<VisualizerLabDirector>,
    /// None when the scene declares no looks.
    pub looks: Option<VisualizerLabLooks>,
    pub effects: VisualizerLabEffects,
    /// The level a lab holds the music at, none when it is the music's own.
    pub forced_level: Option<VisualizerLabLevel>,
}

/// Asked about every 100 ms while the lab's panel is open, and every 500 ms while it is closed (to
/// keep recording, and for the plan's marks on the player's slider). `columns_version` is the
/// version of the catalogue of columns the app has (0 for none): the response carries the
/// catalogue only when it is another.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct VisualizerLabRequest {
    pub columns_version: u64,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// `inspecting` says whether the lab is on (see `SetVisualizerLabRequest`); `now` is none until a
/// frame was drawn. Ask for the plan again when `plan_version` is not the one last seen.
#[derive(Serialize, Deserialize, RustSignal)]
pub struct VisualizerLabResponse {
    pub inspecting: bool,
    /// Song seconds.
    pub song_time: f64,
    pub plan_version: u64,
    /// What happened since the last response, oldest first.
    pub events: Vec<VisualizerLabEvent>,
    pub now: Option<VisualizerLabNow>,
    /// The columns of the lab's series, only when their version is not the request's.
    pub columns: Option<VisualizerLabColumns>,
    /// The buckets of song time finished since the last response, oldest first.
    pub samples: Vec<VisualizerLabSample>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// The columns a lab draws as series, for the scene on screen: the same ones for the live samples
/// and the plan.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub struct VisualizerLabColumns {
    /// Changes when the scene, its inputs, its bindings or its parameters change, and with each
    /// new song.
    pub version: u64,
    /// Buckets of song time in a second.
    pub rate: u32,
    pub columns: Vec<VisualizerLabColumn>,
}

/// One column of the catalogue.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub struct VisualizerLabColumn {
    pub id: String,
    pub name: String,
    pub group: VisualizerLabColumnGroup,
    pub kind: VisualizerLabColumnKind,
    pub lo: f32,
    pub hi: f32,
    /// Empty when it has none.
    pub unit: String,
    /// What the places of a category stand for.
    pub categories: Vec<String>,
    /// Whether the plan has the column: "yes" or "no".
    pub plan: VisualizerLabColumnPlan,
    /// For a scene input, the ids of the columns its bindings read.
    pub binds: Vec<String>,
    /// For a scene input, the value it sits at when nothing drives it.
    pub rest: Option<f32>,
}

/// A bucket of song time folded from the frames the visualizer drew.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub struct VisualizerLabSample {
    pub bucket: u32,
    /// The catalogue the values follow.
    pub columns_version: u64,
    /// One per column, in the catalogue's order; NaN where there is none.
    pub values: Vec<f32>,
}

/// Asked for when the `plan_version` of the lab's response changes.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct VisualizerPlanRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// One section of the planned song.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub struct VisualizerPlanSection {
    /// Seconds.
    pub start: f64,
    pub end: f64,
    pub label: String,
    pub tier: VisualizerSectionTier,
}

/// A planned cut of the camera.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub struct VisualizerPlanCut {
    /// Song seconds.
    pub time: f64,
    pub shot: String,
    pub size: VisualizerShotSize,
    /// 0 for a normal cut, else the position in a burst (1 to 4).
    pub fill: u32,
    pub strong: bool,
    /// The transition that dresses it, by name.
    pub dress: Option<String>,
    /// The scene's first shot, which it begins with.
    pub first: bool,
}

/// A planned change of look; the first one is the look the scene opens in.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub struct VisualizerPlanLook {
    /// Song seconds.
    pub time: f64,
    pub look: String,
}

/// A planned moment of the music that plays an effect.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub struct VisualizerPlanMoment {
    /// Song seconds.
    pub time: f64,
    /// "medium" or "strong".
    pub strength: VisualizerMomentStrength,
    /// What it plays, by name; none if the scene lists nothing that suits the moment.
    pub effect: Option<String>,
}

/// What the scene on screen does in the song on screen, played through ahead of time.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub struct VisualizerPlan {
    /// Seconds.
    pub length: f64,
    pub sections: Vec<VisualizerPlanSection>,
    pub cuts: Vec<VisualizerPlanCut>,
    pub looks: Vec<VisualizerPlanLook>,
    pub moments: Vec<VisualizerPlanMoment>,
    /// The columns of the lab's catalogue over the song.
    pub series: Option<VisualizerPlanSeries>,
}

/// The columns of the lab's catalogue over the planned song, in buckets of song time.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq)]
pub struct VisualizerPlanSeries {
    /// The catalogue the values follow.
    pub columns_version: u64,
    /// Buckets of song time in a second.
    pub rate: u32,
    /// How many buckets the song has.
    pub buckets: u32,
    /// Column-major: `values[column * buckets + bucket]`; NaN where the plan has none.
    pub values: Vec<f32>,
}

/// The plan in `version`, none while the lab is off, the song has no chart or the plan is still
/// being made.
#[derive(Serialize, Deserialize, RustSignal)]
pub struct VisualizerPlanResponse {
    pub version: u64,
    pub plan: Option<VisualizerPlan>,
    #[serde(default)]
    pub call_id: Option<u64>,
}
