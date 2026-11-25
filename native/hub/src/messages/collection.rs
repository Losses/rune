use std::collections::HashMap;

use requests::NamedLibraries;
use rinf::{DartSignal, RustSignal, SignalPiece};
use std::fmt;

use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use super::appended::{appended_field, field};
use super::mix::MixQuery;

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

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct FetchCollectionGroupsRequest {
    pub collection_type: CollectionType,
    pub bake_cover_arts: bool,
    pub group_titles: Vec<String>,
    #[serde(default)]
    pub call_id: Option<u64>,
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

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct FetchCollectionByIdsRequest {
    pub collection_type: CollectionType,
    pub bake_cover_arts: bool,
    pub ids: Vec<i32>,
    #[serde(default)]
    pub call_id: Option<u64>,
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

    use super::{CollectionGroupSummary, CollectionGroupSummaryResponse, CollectionType};

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
}
