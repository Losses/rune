use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::mix::MixQuery;
use super::playback::PlayingItemRef;

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct GetCoverArtIdsByMixQueriesRequestUnit {
    pub id: i32,
    pub queries: Vec<MixQuery>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetCoverArtIdsByMixQueriesRequest {
    pub requests: Vec<GetCoverArtIdsByMixQueriesRequestUnit>,
    pub n: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct GetCoverArtIdsByMixQueriesResponseUnit {
    pub id: i32,
    pub cover_art_ids: Vec<i32>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetCoverArtIdsByMixQueriesResponse {
    pub result: Vec<GetCoverArtIdsByMixQueriesResponseUnit>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetPrimaryColorByTrackIdRequest {
    pub item: Option<PlayingItemRef>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetPrimaryColorByTrackIdResponse {
    pub item: PlayingItemRef,
    pub primary_color: Option<i32>,
    #[serde(default)]
    pub call_id: Option<u64>,
}
