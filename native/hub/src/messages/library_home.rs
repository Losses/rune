use std::collections::HashMap;

use requests::NamedLibraries;
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::collection::CollectionType;
use super::mix::MixQuery;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, SignalPiece)]
pub enum LibraryHomeDomain {
    Artists,
    Albums,
    Genres,
    Playlists,
    Mixes,
    Tracks,
    Liked,
    Most,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, SignalPiece, NamedLibraries,
)]
pub enum LibraryHomeListMode {
    Name,
    Forward,
    Reverse,
    Random,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, SignalPiece, NamedLibraries,
)]
pub enum LibraryHomeQuery {
    Artists(LibraryHomeListMode),
    Albums(LibraryHomeListMode),
    Genres(LibraryHomeListMode),
    Playlists(LibraryHomeListMode),
    Mixes(LibraryHomeListMode),
    Tracks(LibraryHomeListMode),
    Liked,
    Most,
}

impl LibraryHomeQuery {
    pub fn domain(self) -> LibraryHomeDomain {
        match self {
            Self::Artists(_) => LibraryHomeDomain::Artists,
            Self::Albums(_) => LibraryHomeDomain::Albums,
            Self::Genres(_) => LibraryHomeDomain::Genres,
            Self::Playlists(_) => LibraryHomeDomain::Playlists,
            Self::Mixes(_) => LibraryHomeDomain::Mixes,
            Self::Tracks(_) => LibraryHomeDomain::Tracks,
            Self::Liked => LibraryHomeDomain::Liked,
            Self::Most => LibraryHomeDomain::Most,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, SignalPiece, NamedLibraries)]
pub struct ComplexQuery {
    pub title: String,
    pub query: LibraryHomeQuery,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct ComplexQueryRequest {
    pub queries: Vec<ComplexQuery>,
    #[serde(default)]
    pub call_id: Option<u64>,
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
    pub domain: LibraryHomeDomain,
    pub title: String,
    pub entries: Vec<ComplexQueryEntry>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ComplexQueryResponse {
    pub result: Vec<ComplexQueryGroup>,
    #[serde(default)]
    pub call_id: Option<u64>,
}
