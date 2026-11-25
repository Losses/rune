use database::actions::collection::CollectionQueryType;
use requests::NamedLibraries;
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, SignalPiece, NamedLibraries,
)]
pub enum SearchField {
    Track,
    Artist,
    Directory,
    Album,
    Mix,
    Playlist,
    Genre,
}

impl From<CollectionQueryType> for SearchField {
    fn from(value: CollectionQueryType) -> Self {
        match value {
            CollectionQueryType::Track => Self::Track,
            CollectionQueryType::Artist => Self::Artist,
            CollectionQueryType::Directory => Self::Directory,
            CollectionQueryType::Album => Self::Album,
            CollectionQueryType::Mix => Self::Mix,
            CollectionQueryType::Playlist => Self::Playlist,
            CollectionQueryType::Genre => Self::Genre,
        }
    }
}

impl From<SearchField> for CollectionQueryType {
    fn from(value: SearchField) -> Self {
        match value {
            SearchField::Track => Self::Track,
            SearchField::Artist => Self::Artist,
            SearchField::Directory => Self::Directory,
            SearchField::Album => Self::Album,
            SearchField::Mix => Self::Mix,
            SearchField::Playlist => Self::Playlist,
            SearchField::Genre => Self::Genre,
        }
    }
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct SearchForRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
    pub query_str: String,
    pub fields: Vec<SearchField>,
    pub n: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SearchForResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
    pub artists: Vec<i32>,
    pub albums: Vec<i32>,
    pub playlists: Vec<i32>,
    pub tracks: Vec<i32>,
    pub genres: Vec<i32>,
    pub directories: Vec<i32>,
    pub mixes: Vec<i32>,
}
