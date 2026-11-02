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

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchAllPlaylistsRequest {}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchAllPlaylistsResponse {
    pub playlists: Vec<Playlist>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct CreatePlaylistRequest {
    pub name: String,
    pub group: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CreatePlaylistResponse {
    pub playlist: Playlist,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct UpdatePlaylistRequest {
    pub playlist_id: i32,
    pub name: String,
    pub group: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct UpdatePlaylistResponse {
    pub playlist: Playlist,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemovePlaylistRequest {
    pub playlist_id: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemovePlaylistResponse {
    pub playlist_id: i32,
    pub success: bool,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct AddItemToPlaylistRequest {
    pub playlist_id: i32,
    pub media_file_id: i32,
    pub position: Option<i32>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct AddItemToPlaylistResponse {
    pub success: bool,
    pub occurrence_id: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ReorderPlaylistItemPositionRequest {
    pub playlist_id: i32,
    pub occurrence_id: String,
    pub new_position: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ReorderPlaylistItemPositionResponse {
    pub success: bool,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetPlaylistByIdRequest {
    pub playlist_id: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetPlaylistByIdResponse {
    pub playlist: Playlist,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct PlaylistSummary {
    pub id: i32,
    pub name: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct CreateM3u8PlaylistRequest {
    pub name: String,
    pub group: String,
    pub path: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CreateM3u8PlaylistResponse {
    pub playlist: Option<Playlist>,
    pub imported_count: Option<i32>,
    pub not_found_paths: Vec<String>,
    pub success: bool,
    pub error: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemoveItemFromPlaylistRequest {
    pub playlist_id: i32,
    pub occurrence_id: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoveItemFromPlaylistResponse {
    pub success: bool,
    pub error: String,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
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

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchPlaylistOccurrencesRequest {
    pub request_id: String,
    pub playlist_id: i32,
    pub after: Option<PlaylistOccurrenceCursor>,
    pub page_size: u32,
    pub bake_cover_arts: bool,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct FetchPlaylistOccurrencesResponse {
    pub request_id: String,
    pub entries: Vec<PlaylistOccurrence>,
    pub next: Option<PlaylistOccurrenceCursor>,
    pub cover_art_map: std::collections::HashMap<i32, String>,
}
