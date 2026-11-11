use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::playback::PlayingItemRef;

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct LyricContentLine {
    pub start_time: i32,
    pub end_time: i32,
    pub sections: Vec<LyricContentLineSection>,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct LyricContentLineSection {
    pub start_time: i32,
    pub end_time: i32,
    pub content: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetLyricByTrackIdRequest {
    pub item: Option<PlayingItemRef>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize, RustSignal)]
pub struct GetLyricByTrackIdResponse {
    pub item: PlayingItemRef,
    pub lines: Vec<LyricContentLine>,
    #[serde(default)]
    pub call_id: Option<u64>,
}
