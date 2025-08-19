use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::mix::MixQuery;

#[derive(Clone, Deserialize, Serialize, RustSignal)]
pub struct PlaybackStatus {
    pub state: String,
    pub progress_seconds: f32,
    pub progress_percentage: f32,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub title: Option<String>,
    pub duration: f64,
    pub index: Option<i32>,
    pub item: Option<String>,
    pub playback_mode: u32,
    pub ready: bool,
    pub cover_art_path: Option<String>,
    pub lib_path: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct LoadRequest {
    pub index: i32,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct PlayRequest {}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct PauseRequest {}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct NextRequest {}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct PreviousRequest {}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetPlaybackModeRequest {
    pub mode: u32,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SwitchRequest {
    pub index: u32,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SeekRequest {
    pub position_seconds: f64,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemoveRequest {
    pub index: u32,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct MovePlaylistItemRequest {
    pub old_index: u32,
    pub new_index: u32,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct PlaylistItem {
    pub item: PlayingItemRequest,
    pub artist: String,
    pub album: String,
    pub title: String,
    pub duration: f64,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct PlaylistUpdate {
    pub items: Vec<PlaylistItem>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetRealtimeFFTEnabledRequest {
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetAdaptiveSwitchingEnabledRequest {
    pub enabled: bool,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RealtimeFFT {
    pub value: Vec<f32>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct VolumeRequest {
    pub volume: f32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct VolumeResponse {
    pub volume: f32,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaylistOperateMode {
    AppendToEnd,
    PlayNext,
    Replace,
}

#[derive(Debug, Clone, Serialize, Deserialize, SignalPiece)]
pub struct InLibraryPlayingItem {
    pub file_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, SignalPiece)]
pub struct IndependentFilePlayingItem {
    pub raw_path: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, SignalPiece)]
pub struct PlayingItemRequest {
    pub in_library: Option<InLibraryPlayingItem>,
    pub independent_file: Option<IndependentFilePlayingItem>,
}

#[derive(Debug, Serialize, Deserialize, DartSignal)]
pub struct OperatePlaybackWithMixQueryRequest {
    pub queries: Vec<MixQuery>,
    pub playback_mode: u32,
    pub hint_position: i32,
    pub initial_playback_item: Option<PlayingItemRequest>,
    pub instantly_play: bool,
    pub operate_mode: PlaylistOperateMode,
    pub fallback_playing_items: Vec<PlayingItemRequest>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct OperatePlaybackWithMixQueryResponse {
    pub playing_items: Vec<PlayingItemRequest>,
}
