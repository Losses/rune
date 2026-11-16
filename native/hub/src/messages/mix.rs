use requests::NamedLibraries;
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{
    media_file::MediaFile,
    playback::{PlaybackMode, PlayingItemRef},
};

#[derive(Debug, Serialize, Deserialize, Clone, SignalPiece, NamedLibraries)]
pub struct MixQuery {
    pub operator: String,
    pub parameter: String,
}

/// Tracks a mix result leaves out. They are named only to be skipped: nothing is read from
/// their libraries. An empty list excludes nothing.
#[derive(Debug, Default, Serialize, Deserialize, Clone, SignalPiece)]
pub struct ExcludedItems {
    pub items: Vec<PlayingItemRef>,
}

/// Excluding the caller's queue reads nothing from those libraries, so it names none.
impl crate::rpc::NamedLibraries for ExcludedItems {
    const NAMES_LIBRARY: bool = false;

    fn visit_libraries<'a>(&'a self, _visit: &mut dyn FnMut(&'a str)) {}
}

/// The tracks of the caller's own player queue. A mix query reads the queue only to select
/// them with `lib::queue`; nothing is read from their libraries. An empty list is an empty queue.
#[derive(Debug, Default, Serialize, Deserialize, Clone, SignalPiece)]
pub struct CallerQueue {
    pub items: Vec<PlayingItemRef>,
}

/// Listing the caller's queue reads nothing from those libraries, so it names none.
impl crate::rpc::NamedLibraries for CallerQueue {
    const NAMES_LIBRARY: bool = false;

    fn visit_libraries<'a>(&'a self, _visit: &mut dyn FnMut(&'a str)) {}
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct ResolveMixQueryRequest {
    pub queries: Vec<MixQuery>,
    pub hint_position: i32,
    pub initial_playback_item: Option<PlayingItemRef>,
    pub instantly_play: bool,
    pub fallback_playing_items: Vec<PlayingItemRef>,
    /// The caller's own player queue, to leave out of the result.
    pub excluded: ExcludedItems,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ResolveMixQueryResponse {
    pub playing_items: Vec<PlayingItemRef>,
    pub start_index: Option<u32>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct MixQueryRequest {
    pub queries: Vec<MixQuery>,
    pub cursor: i32,
    pub page_size: i32,
    pub bake_cover_arts: bool,
    /// The caller's own player queue, which `lib::queue` selects.
    pub queue: CallerQueue,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct MixQueryResponse {
    pub files: Vec<MediaFile>,
    pub cover_art_map: HashMap<i32, String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct Mix {
    pub id: i32,
    pub name: String,
    pub group: String,
    pub locked: bool,
    pub mode: Option<PlaybackMode>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct FetchAllMixesRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchAllMixesResponse {
    pub mixes: Vec<Mix>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct CreateMixRequest {
    pub name: String,
    pub group: String,
    pub mode: Option<PlaybackMode>,
    pub queries: Vec<MixQuery>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CreateMixResponse {
    pub mix: Mix,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct UpdateMixRequest {
    pub mix_id: i32,
    pub name: String,
    pub group: String,
    pub mode: Option<PlaybackMode>,
    pub queries: Vec<MixQuery>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct UpdateMixResponse {
    pub mix: Mix,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct RemoveMixRequest {
    pub mix_id: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoveMixResponse {
    pub mix_id: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct AddItemToMixRequest {
    pub mix_id: i32,
    pub operator: String,
    pub parameter: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct AddItemToMixResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct GetMixByIdRequest {
    pub mix_id: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetMixByIdResponse {
    pub mix: Mix,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct FetchMixQueriesRequest {
    pub mix_id: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Clone, Deserialize, Serialize, SignalPiece)]
pub struct StoredMixQuery {
    pub clause_id: String,
    pub operator: String,
    pub parameter: String,
    pub target_uuid: Option<String>,
    pub available: bool,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchMixQueriesResponse {
    pub result: Vec<StoredMixQuery>,
    #[serde(default)]
    pub call_id: Option<u64>,
}
