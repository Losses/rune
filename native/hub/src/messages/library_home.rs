use std::collections::HashMap;

use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::collection::CollectionType;
use super::mix::MixQuery;

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct ComplexQuery {
    pub id: String,
    pub title: String,
    pub domain: String,
    pub parameter: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ComplexQueryRequest {
    pub queries: Vec<ComplexQuery>,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct ComplexQueryEntry {
    pub id: i32,
    pub name: String,
    pub queries: Vec<MixQuery>,
    pub collection_type: CollectionType,
    pub cover_art_map: HashMap<i32, String>,
    pub readonly: bool,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct ComplexQueryGroup {
    pub id: String,
    pub title: String,
    pub entries: Vec<ComplexQueryEntry>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ComplexQueryResponse {
    pub result: Vec<ComplexQueryGroup>,
}
