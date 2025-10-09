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
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchParsedMediaFileRequest {
    pub id: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchParsedMediaFileResponse {
    pub file: MediaFile,
    pub artists: Vec<Artist>,
    pub album: Album,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchMediaFilesResponse {
    pub media_files: Vec<MediaFile>,
    pub cover_art_map: HashMap<i32, String>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchMediaFileByIdsRequest {
    pub ids: Vec<i32>,
    pub bake_cover_arts: bool,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchMediaFileByIdsResponse {
    pub media_files: Vec<MediaFile>,
    pub cover_art_map: HashMap<i32, String>,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct MediaFileSummary {
    pub id: i32,
    pub name: String,
    pub cover_art_id: i32,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SearchMediaFileSummaryRequest {
    pub n: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SearchMediaFileSummaryResponse {
    pub result: Vec<MediaFileSummary>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetMediaFilesCountRequest {}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetMediaFilesCountResponse {
    pub count: i32,
}
