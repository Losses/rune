use rinf::{DartSignal, RustSignal};
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

/// Directories with `.milk` / `.milk2` files used by next, previous and random. An empty list
/// stops using preset folders: a running visualizer changes to the default preset.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerPresetDirsRequest {
    pub paths: Vec<String>,
}

/// The app's appearance, which picks the light or the dark version of the default preset.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerAppearanceRequest {
    pub light: bool,
}

/// A transition of 0 seconds or less is a hard cut.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct LoadVisualizerPresetRequest {
    pub path: String,
    pub transition_seconds: f64,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct NextVisualizerPresetRequest {
    pub transition_seconds: f64,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct PreviousVisualizerPresetRequest {
    pub transition_seconds: f64,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RandomVisualizerPresetRequest {
    pub transition_seconds: f64,
}

/// Pauses the automatic switching of presets while `paused`; the timer starts over when it ends.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerAutoSwitchPausedRequest {
    pub paused: bool,
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
    pub preset_count: u32,
    pub adapter: String,
    /// How frames reach the screen: "iosurface" (no copies, macOS), "readback", or empty before
    /// the render thread runs.
    pub transport: String,
    pub last_error: Option<String>,
}
