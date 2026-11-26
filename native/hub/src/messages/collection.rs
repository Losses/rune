use std::collections::HashMap;

use requests::NamedLibraries;
use rinf::{DartSignal, RustSignal, SignalPiece};
use std::fmt;

use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use super::appended::{appended_field, field};
use super::mix::{CallerQueue, MixQuery};

#[derive(
    Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq, NamedLibraries,
)]
pub enum CollectionType {
    Album,
    Artist,
    Playlist,
    Mix,
    Track,
    Genre,
    Directory,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct FetchCollectionGroupSummaryRequest {
    pub collection_type: CollectionType,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct CollectionGroupSummary {
    pub group_title: String,
    pub count: i32,
}

#[derive(Serialize, RustSignal)]
pub struct CollectionGroupSummaryResponse {
    pub collection_type: CollectionType,
    pub groups: Vec<CollectionGroupSummary>,
    #[serde(default)]
    pub call_id: Option<u64>,
    /// The titles of the groups the library fills itself, so no collection can be put in them.
    /// A library of protocol 2.0 sends none.
    #[serde(default)]
    pub reserved_groups: Vec<String>,
}

impl<'de> Deserialize<'de> for CollectionGroupSummaryResponse {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields;

        impl<'de> Visitor<'de> for Fields {
            type Value = CollectionGroupSummaryResponse;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct CollectionGroupSummaryResponse")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                Ok(CollectionGroupSummaryResponse {
                    collection_type: field(&mut seq, 0, &self)?,
                    groups: field(&mut seq, 1, &self)?,
                    call_id: field(&mut seq, 2, &self)?,
                    reserved_groups: appended_field(&mut seq),
                })
            }
        }

        deserializer.deserialize_struct(
            "CollectionGroupSummaryResponse",
            &["collection_type", "groups", "call_id", "reserved_groups"],
            Fields,
        )
    }
}

#[derive(Serialize, DartSignal, NamedLibraries)]
pub struct FetchCollectionGroupsRequest {
    pub collection_type: CollectionType,
    pub bake_cover_arts: bool,
    pub group_titles: Vec<String>,
    #[serde(default)]
    pub call_id: Option<u64>,
    /// The caller's own player queue, which `lib::queue` selects. A controller of protocol 2.0
    /// sends none, which is read as an empty queue.
    #[serde(default)]
    pub queue: CallerQueue,
}

impl<'de> Deserialize<'de> for FetchCollectionGroupsRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields;

        impl<'de> Visitor<'de> for Fields {
            type Value = FetchCollectionGroupsRequest;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct FetchCollectionGroupsRequest")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                Ok(FetchCollectionGroupsRequest {
                    collection_type: field(&mut seq, 0, &self)?,
                    bake_cover_arts: field(&mut seq, 1, &self)?,
                    group_titles: field(&mut seq, 2, &self)?,
                    call_id: field(&mut seq, 3, &self)?,
                    queue: appended_field(&mut seq),
                })
            }
        }

        deserializer.deserialize_struct(
            "FetchCollectionGroupsRequest",
            &[
                "collection_type",
                "bake_cover_arts",
                "group_titles",
                "call_id",
                "queue",
            ],
            Fields,
        )
    }
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
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, DartSignal, NamedLibraries)]
pub struct FetchCollectionByIdsRequest {
    pub collection_type: CollectionType,
    pub bake_cover_arts: bool,
    pub ids: Vec<i32>,
    #[serde(default)]
    pub call_id: Option<u64>,
    /// The caller's own player queue, which `lib::queue` selects. A controller of protocol 2.0
    /// sends none, which is read as an empty queue.
    #[serde(default)]
    pub queue: CallerQueue,
}

impl<'de> Deserialize<'de> for FetchCollectionByIdsRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields;

        impl<'de> Visitor<'de> for Fields {
            type Value = FetchCollectionByIdsRequest;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct FetchCollectionByIdsRequest")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                Ok(FetchCollectionByIdsRequest {
                    collection_type: field(&mut seq, 0, &self)?,
                    bake_cover_arts: field(&mut seq, 1, &self)?,
                    ids: field(&mut seq, 2, &self)?,
                    call_id: field(&mut seq, 3, &self)?,
                    queue: appended_field(&mut seq),
                })
            }
        }

        deserializer.deserialize_struct(
            "FetchCollectionByIdsRequest",
            &[
                "collection_type",
                "bake_cover_arts",
                "ids",
                "call_id",
                "queue",
            ],
            Fields,
        )
    }
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchCollectionByIdsResponse {
    pub collection_type: CollectionType,
    pub result: Vec<Collection>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct SearchCollectionSummaryRequest {
    pub collection_type: Option<CollectionType>,
    pub bake_cover_arts: Option<bool>,
    pub n: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SearchCollectionSummaryResponse {
    pub collection_type: CollectionType,
    pub result: Vec<Collection>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use serde::{Deserialize, Serialize};

    use super::{
        CollectionGroupSummary, CollectionGroupSummaryResponse, CollectionType,
        FetchCollectionByIdsRequest, FetchCollectionGroupsRequest,
    };
    use crate::messages::{CallerQueue, PlayingItemRef};

    /// The group summaries as a peer of protocol 2.0 sends and reads them.
    #[derive(Serialize, Deserialize)]
    struct SummaryOf2_0 {
        collection_type: CollectionType,
        groups: Vec<(String, i32)>,
        call_id: Option<u64>,
    }

    #[test]
    fn the_groups_of_a_library_of_protocol_2_0_are_read_with_none_reserved() -> Result<()> {
        let old = rinf::serialize(&SummaryOf2_0 {
            collection_type: CollectionType::Mix,
            groups: vec![("Night".into(), 2), ("Day".into(), 3)],
            call_id: Some(1),
        })?;
        let read: CollectionGroupSummaryResponse = rinf::deserialize(&old)?;
        let groups: Vec<_> = read
            .groups
            .iter()
            .map(|group| (group.group_title.as_str(), group.count))
            .collect();
        assert_eq!(groups, [("Night", 2), ("Day", 3)]);
        assert_eq!(read.call_id, Some(1));
        assert!(read.reserved_groups.is_empty());
        Ok(())
    }

    #[test]
    fn a_controller_of_protocol_2_0_reads_every_group_of_a_library_that_reserves_one() -> Result<()>
    {
        let group = |title: &str, count| CollectionGroupSummary {
            group_title: title.into(),
            count,
        };
        let new = rinf::serialize(&CollectionGroupSummaryResponse {
            collection_type: CollectionType::Mix,
            groups: vec![group("Night", 2), group("Day", 3)],
            call_id: None,
            reserved_groups: vec!["Day".into()],
        })?;
        let old: SummaryOf2_0 = rinf::deserialize(&new)?;
        assert_eq!(old.groups, [("Night".to_owned(), 2), ("Day".to_owned(), 3)]);
        let read: CollectionGroupSummaryResponse = rinf::deserialize(&new)?;
        assert_eq!(read.reserved_groups, ["Day"]);
        Ok(())
    }

    /// A request for collection groups as a peer of protocol 2.0 sends and reads it.
    #[derive(Serialize, Deserialize)]
    struct GroupsRequestOf2_0 {
        collection_type: CollectionType,
        bake_cover_arts: bool,
        group_titles: Vec<String>,
        call_id: Option<u64>,
    }

    /// A request for collections by ID as a peer of protocol 2.0 sends and reads it.
    #[derive(Serialize, Deserialize)]
    struct ByIdsRequestOf2_0 {
        collection_type: CollectionType,
        bake_cover_arts: bool,
        ids: Vec<i32>,
        call_id: Option<u64>,
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
    fn a_groups_request_from_a_controller_of_protocol_2_0_is_read_with_an_empty_queue() -> Result<()>
    {
        let old = rinf::serialize(&GroupsRequestOf2_0 {
            collection_type: CollectionType::Mix,
            bake_cover_arts: true,
            group_titles: vec!["Night".into()],
            call_id: Some(7),
        })?;
        let read: FetchCollectionGroupsRequest = rinf::deserialize(&old)?;
        assert_eq!(read.collection_type, CollectionType::Mix);
        assert!(read.bake_cover_arts);
        assert_eq!(read.group_titles, ["Night"]);
        assert_eq!(read.call_id, Some(7));
        assert!(read.queue.items.is_empty());
        Ok(())
    }

    #[test]
    fn a_hub_of_protocol_2_0_reads_a_groups_request_with_a_queue_as_far_as_its_fields_go()
    -> Result<()> {
        let new = rinf::serialize(&FetchCollectionGroupsRequest {
            collection_type: CollectionType::Mix,
            bake_cover_arts: true,
            group_titles: vec!["Night".into()],
            call_id: Some(7),
            queue: queue(),
        })?;
        let old: GroupsRequestOf2_0 = rinf::deserialize(&new)?;
        assert_eq!(old.collection_type, CollectionType::Mix);
        assert!(old.bake_cover_arts);
        assert_eq!(old.group_titles, ["Night"]);
        assert_eq!(old.call_id, Some(7));
        let read: FetchCollectionGroupsRequest = rinf::deserialize(&new)?;
        assert_eq!(read.queue.items, queue().items);
        Ok(())
    }

    #[test]
    fn a_by_ids_request_from_a_controller_of_protocol_2_0_is_read_with_an_empty_queue() -> Result<()>
    {
        let old = rinf::serialize(&ByIdsRequestOf2_0 {
            collection_type: CollectionType::Mix,
            bake_cover_arts: true,
            ids: vec![3, 4],
            call_id: Some(7),
        })?;
        let read: FetchCollectionByIdsRequest = rinf::deserialize(&old)?;
        assert_eq!(read.collection_type, CollectionType::Mix);
        assert!(read.bake_cover_arts);
        assert_eq!(read.ids, [3, 4]);
        assert_eq!(read.call_id, Some(7));
        assert!(read.queue.items.is_empty());
        Ok(())
    }

    #[test]
    fn a_hub_of_protocol_2_0_reads_a_by_ids_request_with_a_queue_as_far_as_its_fields_go()
    -> Result<()> {
        let new = rinf::serialize(&FetchCollectionByIdsRequest {
            collection_type: CollectionType::Mix,
            bake_cover_arts: true,
            ids: vec![3, 4],
            call_id: Some(7),
            queue: queue(),
        })?;
        let old: ByIdsRequestOf2_0 = rinf::deserialize(&new)?;
        assert_eq!(old.collection_type, CollectionType::Mix);
        assert!(old.bake_cover_arts);
        assert_eq!(old.ids, [3, 4]);
        assert_eq!(old.call_id, Some(7));
        let read: FetchCollectionByIdsRequest = rinf::deserialize(&new)?;
        assert_eq!(read.queue.items, queue().items);
        Ok(())
    }
}
