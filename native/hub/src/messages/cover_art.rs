use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::mix::MixQuery;
use super::playback::PlayingItemRequest;

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct GetCoverArtIdsByMixQueriesRequestUnit {
    pub id: i32,
    pub queries: Vec<MixQuery>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetCoverArtIdsByMixQueriesRequest {
    pub requests: Vec<GetCoverArtIdsByMixQueriesRequestUnit>,
    pub n: i32,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct GetCoverArtIdsByMixQueriesResponseUnit {
    pub id: i32,
    pub cover_art_ids: Vec<i32>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetCoverArtIdsByMixQueriesResponse {
    pub result: Vec<GetCoverArtIdsByMixQueriesResponseUnit>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetPrimaryColorByTrackIdRequest {
    pub item: Option<PlayingItemRequest>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetPrimaryColorByTrackIdResponse {
    pub item: PlayingItemRequest,
    pub primary_color: Option<i32>,
}
