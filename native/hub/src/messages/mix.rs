use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::media_file::MediaFile;

#[derive(Debug, Serialize, Deserialize, Clone, SignalPiece)]
pub struct MixQuery {
    pub operator: String,
    pub parameter: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct MixQueryRequest {
    pub queries: Vec<MixQuery>,
    pub cursor: i32,
    pub page_size: i32,
    pub bake_cover_arts: bool,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct MixQueryResponse {
    pub files: Vec<MediaFile>,
    pub cover_art_map: HashMap<i32, String>,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct Mix {
    pub id: i32,
    pub name: String,
    pub group: String,
    pub locked: bool,
    pub mode: i32,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchAllMixesRequest {}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchAllMixesResponse {
    pub mixes: Vec<Mix>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct CreateMixRequest {
    pub name: String,
    pub group: String,
    pub scriptlet_mode: bool,
    pub mode: i32,
    pub queries: Vec<MixQuery>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CreateMixResponse {
    pub mix: Mix,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct UpdateMixRequest {
    pub mix_id: i32,
    pub name: String,
    pub group: String,
    pub scriptlet_mode: bool,
    pub mode: i32,
    pub queries: Vec<MixQuery>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct UpdateMixResponse {
    pub mix: Mix,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemoveMixRequest {
    pub mix_id: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoveMixResponse {
    pub mix_id: i32,
    pub success: bool,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct AddItemToMixRequest {
    pub mix_id: i32,
    pub operator: String,
    pub parameter: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct AddItemToMixResponse {
    pub success: bool,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetMixByIdRequest {
    pub mix_id: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetMixByIdResponse {
    pub mix: Mix,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchMixQueriesRequest {
    pub mix_id: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchMixQueriesResponse {
    pub result: Vec<MixQuery>,
}
