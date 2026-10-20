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

/// Directory with `.milk` / `.milk2` files used by next, previous and random.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerPresetDirRequest {
    pub path: String,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualizerSource {
    /// Nothing selected yet.
    None,
    /// The preset pack embedded in builds that opt in.
    Builtin,
    /// The folder of `SetVisualizerPresetDirRequest`.
    Folder,
}

/// Where next, previous and random take presets and textures from. `None` is ignored.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetVisualizerSourceRequest {
    pub source: VisualizerSource,
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
    pub source: VisualizerSource,
    pub builtin_available: bool,
    pub adapter: String,
    pub last_error: Option<String>,
}
