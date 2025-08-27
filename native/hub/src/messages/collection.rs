use std::collections::HashMap;

use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::mix::MixQuery;

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionType {
    Album,
    Artist,
    Playlist,
    Mix,
    Track,
    Genre,
    Directory,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchCollectionGroupSummaryRequest {
    pub collection_type: CollectionType,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct CollectionGroupSummary {
    pub group_title: String,
    pub count: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CollectionGroupSummaryResponse {
    pub collection_type: CollectionType,
    pub groups: Vec<CollectionGroupSummary>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchCollectionGroupsRequest {
    pub collection_type: CollectionType,
    pub bake_cover_arts: bool,
    pub group_titles: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct Collection {
    pub id: i32,
    pub name: String,
    pub queries: Vec<MixQuery>,
    pub collection_type: CollectionType,
    pub cover_art_map: HashMap<i32, String>,
    pub readonly: bool,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct CollectionGroup {
    pub group_title: String,
    pub collections: Vec<Collection>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchCollectionGroupsResponse {
    pub groups: Vec<CollectionGroup>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchCollectionByIdsRequest {
    pub collection_type: CollectionType,
    pub bake_cover_arts: bool,
    pub ids: Vec<i32>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchCollectionByIdsResponse {
    pub collection_type: CollectionType,
    pub result: Vec<Collection>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SearchCollectionSummaryRequest {
    pub collection_type: Option<CollectionType>,
    pub bake_cover_arts: Option<bool>,
    pub n: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SearchCollectionSummaryResponse {
    pub collection_type: CollectionType,
    pub result: Vec<Collection>,
}
