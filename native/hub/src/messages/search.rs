use database::actions::collection::CollectionQueryType;
use requests::NamedLibraries;
use rinf::{DartSignal, RustSignal, SignalPiece};
use std::fmt;

use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use super::appended::{appended_field, field};

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

#[derive(Serialize, DartSignal, NamedLibraries)]
pub struct SearchForRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
    pub query_str: String,
    pub fields: Vec<SearchField>,
    pub n: i32,
    /// When set, the library searches its service again rather than answering from what a
    /// recent search for the same text found. A library that keeps no searches ignores it.
    /// A controller of protocol 2.0 sends none, which is read as unset.
    #[serde(default)]
    pub refresh: bool,
}

impl<'de> Deserialize<'de> for SearchForRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields;

        impl<'de> Visitor<'de> for Fields {
            type Value = SearchForRequest;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct SearchForRequest")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                Ok(SearchForRequest {
                    call_id: field(&mut seq, 0, &self)?,
                    query_str: field(&mut seq, 1, &self)?,
                    fields: field(&mut seq, 2, &self)?,
                    n: field(&mut seq, 3, &self)?,
                    refresh: appended_field(&mut seq),
                })
            }
        }

        deserializer.deserialize_struct(
            "SearchForRequest",
            &["call_id", "query_str", "fields", "n", "refresh"],
            Fields,
        )
    }
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

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use serde::{Deserialize, Serialize};

    use super::{SearchField, SearchForRequest};

    /// A search request as a peer of protocol 2.0 sends and reads it.
    #[derive(Serialize, Deserialize)]
    struct SearchForRequestOf2_0 {
        call_id: Option<u64>,
        query_str: String,
        fields: Vec<SearchField>,
        n: i32,
    }

    #[test]
    fn a_search_from_a_controller_of_protocol_2_0_is_read_as_one_that_does_not_refresh()
    -> Result<()> {
        let old = rinf::serialize(&SearchForRequestOf2_0 {
            call_id: Some(7),
            query_str: "night".into(),
            fields: vec![SearchField::Track],
            n: 20,
        })?;
        let read: SearchForRequest = rinf::deserialize(&old)?;
        assert_eq!(
            (read.call_id, read.query_str.as_str(), read.n, read.refresh),
            (Some(7), "night", 20, false)
        );
        assert!(matches!(read.fields[..], [SearchField::Track]));
        Ok(())
    }

    #[test]
    fn a_hub_of_protocol_2_0_reads_a_search_that_refreshes_as_far_as_its_fields_go() -> Result<()> {
        let new = rinf::serialize(&SearchForRequest {
            call_id: None,
            query_str: "day".into(),
            fields: vec![SearchField::Album],
            n: 5,
            refresh: true,
        })?;
        assert!(rinf::deserialize::<SearchForRequest>(&new)?.refresh);
        let old: SearchForRequestOf2_0 = rinf::deserialize(&new)?;
        assert_eq!((old.query_str.as_str(), old.n), ("day", 5));
        Ok(())
    }
}
