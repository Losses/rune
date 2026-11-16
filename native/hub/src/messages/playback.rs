use crate::request_error::ExpectedRequestError;
use playback::{
    PlaybackMode as PlayerPlaybackMode,
    player::{
        HandoffBlockId as PlaybackHandoffBlockId, PlaybackState as PlayerPlaybackState,
        UnavailableReason,
    },
};
use requests::NamedLibraries;
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Deserialize, Serialize, RustSignal)]
pub struct PlaybackStatus {
    pub session_id: String,
    pub state: PlaybackState,
    pub progress_seconds: f32,
    pub progress_percentage: f32,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub title: Option<String>,
    pub duration: f64,
    pub index: Option<i32>,
    pub item: Option<String>,
    pub item_request: Option<PlayingItemRef>,
    pub playback_mode: PlaybackMode,
    pub ready: bool,
    pub cover_art_path: Option<String>,
    pub lib_path: String,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct LoadRequest {
    pub index: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct PlayRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct PauseRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct NextRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct PreviousRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct SetPlaybackModeRequest {
    pub mode: PlaybackMode,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct SwitchRequest {
    pub index: u32,
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct SeekRequest {
    pub position_seconds: f64,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct RemoveRequest {
    pub index: u32,
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
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
    pub item: PlayingItemRef,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, SignalPiece)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
}

impl From<PlayerPlaybackState> for PlaybackState {
    fn from(state: PlayerPlaybackState) -> Self {
        match state {
            PlayerPlaybackState::Playing => Self::Playing,
            PlayerPlaybackState::Paused => Self::Paused,
            PlayerPlaybackState::Stopped => Self::Stopped,
        }
    }
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

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct SetRealtimeFFTEnabledRequest {
    pub enabled: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
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

/// A completed track analysis from the renderer, keyed by the portable item identity.
#[derive(Deserialize, Serialize, RustSignal)]
pub struct VisualizerFeedChart {
    pub item: PlayingItemRef,
    pub chart: Vec<u8>,
}

/// The playing item's artwork and the bounded cover wall selection from the renderer.
#[derive(Deserialize, Serialize, RustSignal)]
pub struct VisualizerFeedCovers {
    pub item: PlayingItemRef,
    pub playing: Option<Vec<u8>>,
    pub random: Vec<Vec<u8>>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
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

/// How the queue moves on from a track.
#[derive(
    Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq, NamedLibraries,
)]
pub enum PlaybackMode {
    Sequential,
    RepeatOne,
    RepeatAll,
    Shuffle,
}

impl From<PlayerPlaybackMode> for PlaybackMode {
    fn from(mode: PlayerPlaybackMode) -> Self {
        match mode {
            PlayerPlaybackMode::Sequential => Self::Sequential,
            PlayerPlaybackMode::RepeatOne => Self::RepeatOne,
            PlayerPlaybackMode::RepeatAll => Self::RepeatAll,
            PlayerPlaybackMode::Shuffle => Self::Shuffle,
        }
    }
}

impl From<PlaybackMode> for PlayerPlaybackMode {
    fn from(mode: PlaybackMode) -> Self {
        match mode {
            PlaybackMode::Sequential => Self::Sequential,
            PlaybackMode::RepeatOne => Self::RepeatOne,
            PlaybackMode::RepeatAll => Self::RepeatAll,
            PlaybackMode::Shuffle => Self::Shuffle,
        }
    }
}

/// Whether a load starts playing.
#[derive(
    Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, Default, PartialEq, Eq, NamedLibraries,
)]
pub enum LoadPlay {
    /// Start playing the loaded track.
    Play,
    /// Leave the loaded track paused.
    Pause,
    /// Play, except that a load which names a position without a playback mode stays paused.
    #[default]
    Auto,
}

#[derive(
    Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq, NamedLibraries,
)]
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

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
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

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct SessionSnapshotRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Re-send the renderer's current player bar and queue after attachment.
#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct RefreshSessionRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SessionSnapshotResponse {
    pub items: Vec<PlayingItemRef>,
    pub index: Option<u32>,
    pub position_seconds: f64,
    pub playback_mode: PlaybackMode,
    pub playing: bool,
    pub revision: u64,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct PlaylistRevisionRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
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

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash, SignalPiece)]
#[serde(rename_all = "snake_case")]
pub enum PlayingItemRef {
    LibraryTrack { library_id: String, track: String },
    IndependentFile { path: String },
}

/// A library track names the library it belongs to; an independent file names none.
impl crate::rpc::NamedLibraries for PlayingItemRef {
    const NAMES_LIBRARY: bool = true;

    fn visit_libraries<'a>(&'a self, visit: &mut dyn FnMut(&'a str)) {
        match self {
            Self::LibraryTrack { library_id, .. } => visit(library_id),
            Self::IndependentFile { .. } => {}
        }
    }
}

impl From<playback::player::LibraryItem> for PlayingItemRef {
    fn from(item: playback::player::LibraryItem) -> Self {
        Self::LibraryTrack {
            library_id: item.library_id.to_string(),
            track: item.file_uuid.to_string(),
        }
    }
}

impl From<playback::player::SubsonicItem> for PlayingItemRef {
    fn from(item: playback::player::SubsonicItem) -> Self {
        Self::LibraryTrack {
            library_id: item.library_id.to_string(),
            track: item.track_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, SignalPiece, DartSignal, NamedLibraries)]
pub struct LoadResolvedItemsRequest {
    pub items: Vec<PlayingItemRef>,
    pub start_index: Option<u32>,
    pub mode: PlaylistOperateMode,
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub playback_mode: Option<PlaybackMode>,
    #[serde(default)]
    pub position_seconds: Option<f64>,
    #[serde(default)]
    pub play: LoadPlay,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct CastResolvedItemsRequest {
    pub fingerprint: String,
    pub load: LoadResolvedItemsRequest,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CastResolvedItemsResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
    pub result: ResolvedItemsLoadResult,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct LoadResolvedItemsResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
    pub result: ResolvedItemsLoadResult,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, SignalPiece)]
pub struct ResolvedItemsLoadResult {
    pub left_out_items: Vec<PlayingItemRef>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct HandoffLoadResponse {
    pub handoff_item: PlayingItemRef,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// The id of a handed-off queue block on the wire: a UUID in text form, like the
/// other ids that cross the Dart boundary.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, SignalPiece, NamedLibraries,
)]
pub struct HandoffBlockId {
    pub uuid: String,
}

impl From<PlaybackHandoffBlockId> for HandoffBlockId {
    fn from(block: PlaybackHandoffBlockId) -> Self {
        Self {
            uuid: block.0.to_string(),
        }
    }
}

impl TryFrom<&HandoffBlockId> for PlaybackHandoffBlockId {
    type Error = ExpectedRequestError;

    fn try_from(block: &HandoffBlockId) -> Result<Self, Self::Error> {
        Uuid::parse_str(&block.uuid).map(Self).map_err(|error| {
            ExpectedRequestError::invalid_input(format!("Invalid handoff block id: {error}"))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct HandoffLoadRequest {
    pub block: HandoffBlockId,
    pub load: LoadResolvedItemsRequest,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct TakeBackHandoffBlockRequest {
    pub block: HandoffBlockId,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, SignalPiece)]
pub struct CastTarget {
    pub fingerprint: String,
    pub attach: bool,
}
