use playback::player::UnavailableReason;
use playback::player::{LibraryItem, SubsonicItem};
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

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
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct PlayRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct PauseRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct NextRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct PreviousRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetPlaybackModeRequest {
    pub mode: u32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SwitchRequest {
    pub index: u32,
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SeekRequest {
    pub position_seconds: f64,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemoveRequest {
    pub index: u32,
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct MovePlaylistItemRequest {
    pub old_index: u32,
    pub new_index: u32,
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub call_id: Option<u64>,
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
            UnavailableReason::Unreadable | UnavailableReason::TrackGone => Self::Unreadable,
            UnavailableReason::CredentialsRejected => Self::AccessRevoked,
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
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetAdaptiveSwitchingEnabledRequest {
    pub enabled: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Lets other apps keep playing while this device plays (iOS only; other platforms ignore it).
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetMixWithOthersRequest {
    pub enabled: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RealtimeFFT {
    pub value: Vec<f32>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct VolumeRequest {
    pub volume: f32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct VolumeResponse {
    pub volume: f32,
    #[serde(default)]
    pub call_id: Option<u64>,
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
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, SignalPiece)]
pub struct CastLibraryStatus {
    pub library_id: String,
    pub state: CastLibraryState,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CastPreflightResponse {
    pub libraries: Vec<CastLibraryStatus>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SessionSnapshotRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Re-send the renderer's current player bar and queue after attachment.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct RefreshSessionRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SessionSnapshotResponse {
    pub items: Vec<LibraryItemRequest>,
    pub index: Option<u32>,
    pub position_seconds: f64,
    pub playback_mode: u32,
    pub playing: bool,
    pub revision: u64,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct PlaylistRevisionRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemotePlaylistRevisionRequest {
    pub fingerprint: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemotePlaylistRevisionResponse {
    pub revision: u64,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct PlaylistRevisionResponse {
    pub revision: u64,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct HandoffRequest {
    pub target: Option<String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal, Debug)]
pub struct HandoffResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

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

#[derive(Debug, Clone, Serialize, Deserialize, SignalPiece)]
pub struct SubsonicItemRequest {
    pub library_id: String,
    pub track_id: String,
}

impl From<SubsonicItem> for SubsonicItemRequest {
    fn from(item: SubsonicItem) -> Self {
        Self {
            library_id: item.library_id.to_string(),
            track_id: item.track_id,
        }
    }
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
    #[serde(default)]
    pub subsonic_item: Option<SubsonicItemRequest>,
}

impl From<LibraryItemRequest> for PlayingItemRequest {
    fn from(library_item: LibraryItemRequest) -> Self {
        Self {
            in_library: None,
            independent_file: None,
            library_item: Some(library_item),
            subsonic_item: None,
        }
    }
}

impl From<LibraryItem> for PlayingItemRequest {
    fn from(item: LibraryItem) -> Self {
        Self::from(LibraryItemRequest {
            library_id: item.library_id.to_string(),
            file_uuid: item.file_uuid.to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, DartSignal)]
pub struct LoadResolvedItemsRequest {
    pub items: Vec<PlayingItemRequest>,
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
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, DartSignal)]
pub struct CastResolvedItemsRequest {
    pub fingerprint: String,
    pub items: Vec<PlayingItemRequest>,
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
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CastResolvedItemsResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct LoadResolvedItemsResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, SignalPiece)]
pub struct CastTarget {
    pub fingerprint: String,
    pub attach: bool,
}
