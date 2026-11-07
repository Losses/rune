use playback::player::LibraryItem;
use playback::player::UnavailableReason;
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::mix::MixQuery;

#[derive(Clone, Deserialize, Serialize, RustSignal)]
pub struct PlaybackStatus {
    pub session_id: String,
    pub state: String,
    pub progress_seconds: f32,
    pub progress_percentage: f32,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub title: Option<String>,
    pub duration: f64,
    pub index: Option<i32>,
    pub item: Option<String>,
    pub item_request: Option<PlayingItemRequest>,
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
    #[serde(default)]
    pub expected_revision: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct MovePlaylistItemRequest {
    pub old_index: u32,
    pub new_index: u32,
    #[serde(default)]
    pub expected_revision: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct PlaylistItem {
    pub unavailable: Option<PlaylistUnavailableReason>,
    pub item: PlayingItemRequest,
    pub artist: String,
    pub album: String,
    pub title: String,
    pub duration: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SignalPiece)]
pub enum PlaylistUnavailableReason {
    AccessRevoked,
    LibraryUnavailable,
    SourceUnreachable,
    Unreadable,
}

impl From<UnavailableReason> for PlaylistUnavailableReason {
    fn from(reason: UnavailableReason) -> Self {
        match reason {
            UnavailableReason::AccessRevoked => Self::AccessRevoked,
            UnavailableReason::LibraryUnavailable => Self::LibraryUnavailable,
            UnavailableReason::SourceUnreachable => Self::SourceUnreachable,
            UnavailableReason::Unreadable => Self::Unreadable,
        }
    }
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct PlaylistUpdate {
    pub items: Vec<PlaylistItem>,
    pub session_id: String,
    pub revision: u64,
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

#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastLibraryState {
    Ready,
    NotHeld,
    AccessRevoked,
    Unreachable,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct CastPreflightRequest {
    pub library_ids: Vec<String>,
}

#[derive(Deserialize, Serialize, SignalPiece)]
pub struct CastLibraryStatus {
    pub library_id: String,
    pub state: CastLibraryState,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CastPreflightResponse {
    pub libraries: Vec<CastLibraryStatus>,
}

#[derive(Clone, Serialize, Deserialize, DartSignal)]
pub struct CastLoadRequest {
    pub items: Vec<LibraryItemRequest>,
    #[serde(default)]
    pub start_index: Option<u32>,
    pub mode: PlaylistOperateMode,
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub playback_mode: Option<u32>,
    #[serde(default)]
    pub position_seconds: Option<f64>,
    #[serde(default)]
    pub play: Option<bool>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CastLoadResponse {}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SessionSnapshotRequest {}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SessionSnapshotResponse {
    pub items: Vec<LibraryItemRequest>,
    pub index: Option<u32>,
    pub position_seconds: f64,
    pub playback_mode: u32,
    pub playing: bool,
    pub revision: u64,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct HandoffRequest {
    pub target: Option<String>,
}

#[derive(Deserialize, Serialize, RustSignal, Debug)]
pub struct HandoffResponse {}

#[derive(Debug, Clone, Serialize, Deserialize, SignalPiece)]
pub struct InLibraryPlayingItem {
    pub file_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, SignalPiece)]
pub struct IndependentFilePlayingItem {
    pub raw_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, SignalPiece)]
pub struct LibraryItemRequest {
    pub library_id: String,
    pub file_uuid: String,
}

impl From<LibraryItem> for LibraryItemRequest {
    fn from(item: LibraryItem) -> Self {
        Self {
            library_id: item.library_id.to_string(),
            file_uuid: item.file_uuid.to_string(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, SignalPiece)]
pub struct PlayingItemRequest {
    pub in_library: Option<InLibraryPlayingItem>,
    pub independent_file: Option<IndependentFilePlayingItem>,
    pub library_item: Option<LibraryItemRequest>,
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
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub cast_to: Option<CastTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, SignalPiece)]
pub struct CastTarget {
    pub fingerprint: String,
    pub attach: bool,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct OperatePlaybackWithMixQueryResponse {
    pub playing_items: Vec<PlayingItemRequest>,
}
