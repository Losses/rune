use std::collections::HashMap;
use std::fmt;

use requests::NamedLibraries;
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use super::appended::{appended_field, field};
use super::collection::CollectionType;
use super::mix::{CallerQueue, MixQuery};

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

#[derive(Serialize, DartSignal, NamedLibraries)]
pub struct ComplexQueryRequest {
    pub queries: Vec<ComplexQuery>,
    #[serde(default)]
    pub call_id: Option<u64>,
    /// The caller's own player queue, which `lib::queue` selects. A controller of protocol 2.0
    /// sends none, which is read as an empty queue.
    #[serde(default)]
    pub queue: CallerQueue,
}

impl<'de> Deserialize<'de> for ComplexQueryRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields;

        impl<'de> Visitor<'de> for Fields {
            type Value = ComplexQueryRequest;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct ComplexQueryRequest")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                Ok(ComplexQueryRequest {
                    queries: field(&mut seq, 0, &self)?,
                    call_id: field(&mut seq, 1, &self)?,
                    queue: appended_field(&mut seq),
                })
            }
        }

        deserializer.deserialize_struct(
            "ComplexQueryRequest",
            &["queries", "call_id", "queue"],
            Fields,
        )
    }
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

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use serde::{Deserialize, Serialize};

    use super::{ComplexQuery, ComplexQueryRequest, LibraryHomeListMode, LibraryHomeQuery};
    use crate::messages::{CallerQueue, PlayingItemRef};

    /// A library home request as a peer of protocol 2.0 sends and reads it.
    #[derive(Serialize, Deserialize)]
    struct ComplexQueryRequestOf2_0 {
        queries: Vec<ComplexQuery>,
        call_id: Option<u64>,
    }

    fn mixes() -> Vec<ComplexQuery> {
        vec![ComplexQuery {
            title: "Mixes".into(),
            query: LibraryHomeQuery::Mixes(LibraryHomeListMode::Forward),
        }]
    }

    fn queue() -> CallerQueue {
        CallerQueue {
            items: vec![PlayingItemRef::LibraryTrack {
                library_id: "library".into(),
                track: "track".into(),
            }],
        }
    }

    #[test]
    fn a_home_request_from_a_controller_of_protocol_2_0_is_read_with_an_empty_queue() -> Result<()>
    {
        let old = rinf::serialize(&ComplexQueryRequestOf2_0 {
            queries: mixes(),
            call_id: Some(7),
        })?;
        let read: ComplexQueryRequest = rinf::deserialize(&old)?;
        assert_eq!(read.queries.len(), 1);
        assert_eq!(read.queries[0].title, "Mixes");
        assert_eq!(read.call_id, Some(7));
        assert!(read.queue.items.is_empty());
        Ok(())
    }

    #[test]
    fn a_hub_of_protocol_2_0_reads_a_home_request_with_a_queue_as_far_as_its_fields_go()
    -> Result<()> {
        let new = rinf::serialize(&ComplexQueryRequest {
            queries: mixes(),
            call_id: Some(7),
            queue: queue(),
        })?;
        let old: ComplexQueryRequestOf2_0 = rinf::deserialize(&new)?;
        assert_eq!(old.queries.len(), 1);
        assert_eq!(old.queries[0].title, "Mixes");
        assert_eq!(old.call_id, Some(7));
        let read: ComplexQueryRequest = rinf::deserialize(&new)?;
        assert_eq!(read.queue.items, queue().items);
        Ok(())
    }
}
