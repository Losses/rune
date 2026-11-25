use requests::NamedLibraries;
use rinf::{DartSignal, RustSignal};
use serde::{Deserialize, Serialize};

use super::playback::PlayingItemRef;

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct SetLikedRequest {
    pub item: Option<PlayingItemRef>,
    pub liked: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SetLikedResponse {
    pub item: PlayingItemRef,
    pub liked: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct GetLikedRequest {
    pub item: Option<PlayingItemRef>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetLikedResponse {
    pub item: PlayingItemRef,
    pub liked: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}
