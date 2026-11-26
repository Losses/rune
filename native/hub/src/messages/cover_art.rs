use std::fmt;

use requests::NamedLibraries;
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use super::appended::{appended_field, field};
use super::mix::{CallerQueue, MixQuery};
use super::playback::PlayingItemRef;

#[derive(Clone, Serialize, Deserialize, SignalPiece, NamedLibraries)]
pub struct GetCoverArtIdsByMixQueriesRequestUnit {
    pub id: i32,
    pub queries: Vec<MixQuery>,
}

#[derive(Serialize, DartSignal, NamedLibraries)]
pub struct GetCoverArtIdsByMixQueriesRequest {
    pub requests: Vec<GetCoverArtIdsByMixQueriesRequestUnit>,
    pub n: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
    /// The caller's own player queue, which `lib::queue` selects. A controller of protocol 2.0
    /// sends none, which is read as an empty queue.
    #[serde(default)]
    pub queue: CallerQueue,
}

impl<'de> Deserialize<'de> for GetCoverArtIdsByMixQueriesRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields;

        impl<'de> Visitor<'de> for Fields {
            type Value = GetCoverArtIdsByMixQueriesRequest;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct GetCoverArtIdsByMixQueriesRequest")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                Ok(GetCoverArtIdsByMixQueriesRequest {
                    requests: field(&mut seq, 0, &self)?,
                    n: field(&mut seq, 1, &self)?,
                    call_id: field(&mut seq, 2, &self)?,
                    queue: appended_field(&mut seq),
                })
            }
        }

        deserializer.deserialize_struct(
            "GetCoverArtIdsByMixQueriesRequest",
            &["requests", "n", "call_id", "queue"],
            Fields,
        )
    }
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct GetCoverArtIdsByMixQueriesResponseUnit {
    pub id: i32,
    pub cover_art_ids: Vec<i32>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetCoverArtIdsByMixQueriesResponse {
    pub result: Vec<GetCoverArtIdsByMixQueriesResponseUnit>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct GetPrimaryColorByTrackIdRequest {
    pub item: Option<PlayingItemRef>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetPrimaryColorByTrackIdResponse {
    pub item: PlayingItemRef,
    pub primary_color: Option<i32>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use serde::{Deserialize, Serialize};

    use super::{GetCoverArtIdsByMixQueriesRequest, GetCoverArtIdsByMixQueriesRequestUnit};
    use crate::messages::{CallerQueue, MixQuery, PlayingItemRef};

    /// A request for the covers of mix queries as a peer of protocol 2.0 sends and reads it.
    #[derive(Serialize, Deserialize)]
    struct CoverArtIdsRequestOf2_0 {
        requests: Vec<GetCoverArtIdsByMixQueriesRequestUnit>,
        n: i32,
        call_id: Option<u64>,
    }

    fn queued_covers() -> Vec<GetCoverArtIdsByMixQueriesRequestUnit> {
        vec![GetCoverArtIdsByMixQueriesRequestUnit {
            id: 3,
            queries: vec![MixQuery {
                operator: "lib::queue".into(),
                parameter: "true".into(),
            }],
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
    fn a_covers_request_from_a_controller_of_protocol_2_0_is_read_with_an_empty_queue() -> Result<()>
    {
        let old = rinf::serialize(&CoverArtIdsRequestOf2_0 {
            requests: queued_covers(),
            n: 4,
            call_id: Some(7),
        })?;
        let read: GetCoverArtIdsByMixQueriesRequest = rinf::deserialize(&old)?;
        assert_eq!(read.requests.len(), 1);
        assert_eq!(read.requests[0].id, 3);
        assert_eq!((read.n, read.call_id), (4, Some(7)));
        assert!(read.queue.items.is_empty());
        Ok(())
    }

    #[test]
    fn a_hub_of_protocol_2_0_reads_a_covers_request_with_a_queue_as_far_as_its_fields_go()
    -> Result<()> {
        let new = rinf::serialize(&GetCoverArtIdsByMixQueriesRequest {
            requests: queued_covers(),
            n: 4,
            call_id: Some(7),
            queue: queue(),
        })?;
        let old: CoverArtIdsRequestOf2_0 = rinf::deserialize(&new)?;
        assert_eq!(old.requests.len(), 1);
        assert_eq!(old.requests[0].id, 3);
        assert_eq!((old.n, old.call_id), (4, Some(7)));
        let read: GetCoverArtIdsByMixQueriesRequest = rinf::deserialize(&new)?;
        assert_eq!(read.queue.items, queue().items);
        Ok(())
    }
}
