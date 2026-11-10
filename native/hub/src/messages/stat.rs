use rinf::{DartSignal, RustSignal};
use serde::{Deserialize, Serialize};

use super::playback::PlayingItemRequest;

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetLikedRequest {
    pub item: Option<PlayingItemRequest>,
    pub liked: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SetLikedResponse {
    pub item: PlayingItemRequest,
    pub liked: bool,
    pub success: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetLikedRequest {
    pub item: Option<PlayingItemRequest>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetLikedResponse {
    pub item: PlayingItemRequest,
    pub liked: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}
