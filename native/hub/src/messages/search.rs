use rinf::{DartSignal, RustSignal};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SearchForRequest {
    pub query_str: String,
    pub fields: Vec<String>,
    pub n: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SearchForResponse {
    pub artists: Vec<i32>,
    pub albums: Vec<i32>,
    pub playlists: Vec<i32>,
    pub tracks: Vec<i32>,
}
