use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, DartSignal)]
pub struct StartVisualizerRequest {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct StopVisualizerRequest {}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ResizeVisualizerRequest {
    pub width: u32,
    pub height: u32,
}

/// Directories with scenes (folders with a `scene.toml`) listed beside the built-in ones: the
/// app's folder of imported scenes. A running visualizer whose scene is gone changes to the
/// built-in scene.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerPresetDirsRequest {
    pub paths: Vec<String>,
}

/// The app's accent colour as 0xAARRGGBB, for scenes that draw in it.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerAccentRequest {
    pub argb: u32,
}

/// Whether the app's window can be seen. While it cannot, the visualizer stands still.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerVisibleRequest {
    pub visible: bool,
}

/// Shows the scene at `path`: a scene folder, or `builtin:<name>` for a built-in scene (as the
/// scenes of [`ListVisualizerPresetsResponse`] say).
#[derive(Serialize, Deserialize, DartSignal)]
pub struct LoadVisualizerPresetRequest {
    pub path: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct NextVisualizerPresetRequest {}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct PreviousVisualizerPresetRequest {}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RandomVisualizerPresetRequest {}

/// How the scenes are picked (paths in the form of `VisualizerPresetEntry::path`), replacing what
/// was set before. Next and the switch at every song go through the scenes in `order`; scenes not
/// in it, such as those of a folder scanned later, come after it. `disabled` are left out, but can
/// still be loaded by path; turning every scene off counts as turning none off.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerScenesRequest {
    pub order: Vec<String>,
    pub disabled: Vec<String>,
    /// The scene on screen when the app last ran, which a visualizer that starts resumes unless
    /// another scene was shown since the app started.
    pub last: Option<String>,
}

/// Unpack the scene file `file` (a `.runescene`, see `visualizer::package`) into a folder of its
/// own in `dir`, the app's folder of imported scenes, replacing a scene of that name. The scenes
/// of `dir` are listed again.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct ImportVisualizerSceneRequest {
    pub file: String,
    pub dir: String,
}

/// `path` is the folder of the imported scene, or `error` says why the file is not one.
#[derive(Serialize, Deserialize, RustSignal)]
pub struct ImportVisualizerSceneResponse {
    pub file: String,
    pub path: Option<String>,
    pub error: Option<String>,
}

/// Delete the imported scene at `path`, which must be a folder directly in `dir`. The scenes of
/// `dir` are listed again.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemoveVisualizerSceneRequest {
    pub path: String,
    pub dir: String,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct RemoveVisualizerSceneResponse {
    pub path: String,
    pub error: Option<String>,
}

/// Whether the visualizer's diagnostics (the GPU it runs on, how frames reach the screen) go to the
/// Log page; its errors always do. The app sends its setting at start and whenever it changes,
/// and the Log page's entries are written from the first one on.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerLogRequest {
    pub enabled: bool,
}

/// Whether this machine can run the visualizer, asked before starting it.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct VisualizerSupportRequest {}

/// `supported`: there is a GPU (`adapter` names it). Otherwise `problem` says why not: no GPU
/// adapter, or only a software renderer, which would draw the scenes on the CPU; then only the
/// cover wall can be shown.
#[derive(Serialize, Deserialize, RustSignal)]
pub struct VisualizerSupportResponse {
    pub supported: bool,
    pub adapter: String,
    pub problem: Option<String>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct VisualizerStatusRequest {}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct VisualizerStatusResponse {
    pub running: bool,
    pub width: u32,
    pub height: u32,
    pub target_fps: u32,
    pub measured_fps: f32,
    pub preset_name: String,
    /// Where the scene on screen is, in the form of `VisualizerPresetEntry::path`;
    /// `app:cover-wall` while the cover wall is on and nothing is drawn.
    pub preset_path: String,
    pub preset_count: u32,
    pub adapter: String,
    /// How frames reach the screen: "iosurface" (no copies, macOS), "dxgi-shared" (no copies,
    /// Windows), "vulkan-shared" (no copies, Linux), "surface" (no copies, Android: presented
    /// into the texture's window), "readback", or empty before the render thread runs.
    pub transport: String,
    pub last_error: Option<String>,
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
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListVisualizerPresetsRequest {}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListVisualizerCreditsRequest {}

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
}

#[derive(Serialize, Deserialize, SignalPiece)]
pub struct VisualizerPresetEntry {
    pub name: String,
    pub path: String,
    /// The folder the scene is in, relative to its scene folder; empty for a built-in scene.
    pub group: String,
    /// Whether the scene is compiled into the program (its `path` is `builtin:<name>`).
    pub builtin: bool,
}

/// Every scene, the built-in ones, the cover wall (path `app:cover-wall`, which the app draws and
/// the visualizer leaves blank) and those of the scene folders, by group and name.
#[derive(Serialize, Deserialize, RustSignal)]
pub struct ListVisualizerPresetsResponse {
    pub presets: Vec<VisualizerPresetEntry>,
}
