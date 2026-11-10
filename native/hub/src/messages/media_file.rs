use database::actions::file::{
    AudioAvailability as FileAudioAvailability, AudioOrigin as FileAudioOrigin,
};
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::album::Album;
use super::artist::Artist;

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchMediaFilesRequest {
    pub cursor: i32,
    pub page_size: i32,
    pub bake_cover_arts: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Local state only: RemoteOnly records a blob reference, not a reachable peer.
#[derive(Clone, Copy, Serialize, Deserialize, SignalPiece, Debug, PartialEq, Eq)]
pub enum AudioAvailability {
    Local,
    RemoteOnly,
    Unavailable,
    ImportPending,
}

impl AudioAvailability {
    /// A pending persistent download does not remove an existing remote content reference.
    pub fn from_local(value: FileAudioAvailability, has_remote_content: bool) -> Self {
        use database::actions::file::AudioAvailability as State;
        match value {
            State::Local => Self::Local,
            State::RemoteOnly => Self::RemoteOnly,
            State::Unavailable => Self::Unavailable,
            State::ImportPending if has_remote_content => Self::RemoteOnly,
            State::ImportPending => Self::ImportPending,
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize, SignalPiece, Debug, PartialEq, Eq)]
pub enum AudioOrigin {
    Host,
    SyncedCopy,
    Remote,
}

impl From<FileAudioOrigin> for AudioOrigin {
    fn from(value: FileAudioOrigin) -> Self {
        match value {
            FileAudioOrigin::Host => Self::Host,
            FileAudioOrigin::SyncedCopy => Self::SyncedCopy,
            FileAudioOrigin::Remote => Self::Remote,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, SignalPiece, Debug)]
pub struct MediaFile {
    pub id: i32,
    pub path: String,
    pub artist: String,
    pub album: String,
    pub title: String,
    pub duration: f64,
    pub cover_art_id: i32,
    pub track_number: i32,
    pub audio_availability: AudioAvailability,
    pub audio_origin: AudioOrigin,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchParsedMediaFileRequest {
    pub id: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchParsedMediaFileResponse {
    pub file: MediaFile,
    pub artists: Vec<Artist>,
    pub album: Album,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchMediaFilesResponse {
    pub media_files: Vec<MediaFile>,
    pub cover_art_map: HashMap<i32, String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchMediaFileByIdsRequest {
    pub ids: Vec<i32>,
    pub bake_cover_arts: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchMediaFileByIdsResponse {
    pub media_files: Vec<MediaFile>,
    pub cover_art_map: HashMap<i32, String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct MediaFileSummary {
    pub id: i32,
    pub name: String,
    pub cover_art_id: i32,
    pub audio_availability: AudioAvailability,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SearchMediaFileSummaryRequest {
    pub n: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SearchMediaFileSummaryResponse {
    pub result: Vec<MediaFileSummary>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetMediaFilesCountRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetMediaFilesCountResponse {
    pub count: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}
