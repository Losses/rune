use requests::NamedLibraries;
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct Playlist {
    pub id: i32,
    pub name: String,
    pub group: String,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct PlaylistsGroup {
    pub group_title: String,
    pub playlists: Vec<Playlist>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct FetchAllPlaylistsRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchAllPlaylistsResponse {
    pub playlists: Vec<Playlist>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct CreatePlaylistRequest {
    pub name: String,
    pub group: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CreatePlaylistResponse {
    pub playlist: Playlist,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct UpdatePlaylistRequest {
    pub playlist_id: i32,
    pub name: String,
    pub group: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct UpdatePlaylistResponse {
    pub playlist: Playlist,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct RemovePlaylistRequest {
    pub playlist_id: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemovePlaylistResponse {
    pub playlist_id: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct AddItemToPlaylistRequest {
    pub playlist_id: i32,
    pub media_file_id: i32,
    pub position: Option<i32>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct AddItemToPlaylistResponse {
    pub occurrence_id: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct ReorderPlaylistItemPositionRequest {
    pub playlist_id: i32,
    pub occurrence_id: String,
    pub new_position: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ReorderPlaylistItemPositionResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct GetPlaylistByIdRequest {
    pub playlist_id: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetPlaylistByIdResponse {
    pub playlist: Playlist,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct PlaylistSummary {
    pub id: i32,
    pub name: String,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct CreateM3u8PlaylistRequest {
    pub name: String,
    pub group: String,
    pub path: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CreateM3u8PlaylistResponse {
    pub playlist: Option<Playlist>,
    pub imported_count: Option<i32>,
    pub not_found_paths: Vec<String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct RemoveItemFromPlaylistRequest {
    pub playlist_id: i32,
    pub occurrence_id: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoveItemFromPlaylistResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece, NamedLibraries)]
pub struct PlaylistOccurrenceCursor {
    pub position: i32,
    pub occurrence_id: String,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct PlaylistOccurrence {
    pub occurrence_id: String,
    pub position: i32,
    pub track_uuid: String,
    pub file: Option<super::MediaFile>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct FetchPlaylistOccurrencesRequest {
    pub request_id: String,
    pub playlist_id: i32,
    pub after: Option<PlaylistOccurrenceCursor>,
    pub page_size: u32,
    pub bake_cover_arts: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct FetchPlaylistOccurrencesResponse {
    pub request_id: String,
    pub entries: Vec<PlaylistOccurrence>,
    pub next: Option<PlaylistOccurrenceCursor>,
    pub cover_art_map: std::collections::HashMap<i32, String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}
