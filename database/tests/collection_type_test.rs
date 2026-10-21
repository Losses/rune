use std::collections::HashSet;

use anyhow::Result;
use sea_orm::Database;

use ::database::{
    actions::{
        collection::{CollectionQuery, CollectionQueryType, UnifiedCollection},
        mixes::{create_mix, replace_mix_queries},
    },
    connection::initialize_db,
    entities::mixes,
};

const NODE_ID: &str = "collection-type-test";

fn all_types() -> Vec<CollectionQueryType> {
    vec![
        CollectionQueryType::Track,
        CollectionQueryType::Artist,
        CollectionQueryType::Directory,
        CollectionQueryType::Album,
        CollectionQueryType::Mix,
        CollectionQueryType::Playlist,
        CollectionQueryType::Genre,
    ]
}

#[test]
fn every_type_parses_back_from_its_name() {
    for collection_type in all_types() {
        let name = collection_type.to_string();
        assert_eq!(
            name.parse::<CollectionQueryType>().ok(),
            Some(collection_type),
            "{name:?} does not parse back"
        );
    }
}

#[test]
fn every_type_has_its_own_number() {
    let numbers: HashSet<i32> = all_types().into_iter().map(i32::from).collect();
    assert_eq!(numbers.len(), all_types().len());
}

#[test]
fn every_type_reads_back_from_its_search_index_number() {
    for collection_type in all_types() {
        let number = i64::from(collection_type.clone());
        assert_eq!(
            CollectionQueryType::try_from(number).ok(),
            Some(collection_type)
        );
    }
}

// The steps the hub takes for a Mix in FetchCollectionGroupSummaryRequest,
// FetchCollectionGroupsRequest and FetchCollectionByIdsRequest
#[tokio::test]
async fn a_mix_with_genre_and_directory_queries_reads_back() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    initialize_db(&db, NODE_ID).await?;

    let mix = create_mix(
        &db,
        NODE_ID,
        "Genres and folders".to_owned(),
        "Mine".to_owned(),
        false,
        99,
        false,
    )
    .await?;
    let queries = vec![
        ("lib::genre".to_owned(), "1".to_owned()),
        ("lib::directory.deep".to_owned(), "Music/Rock".to_owned()),
    ];
    replace_mix_queries(&db, NODE_ID, mix.id, &mix.hlc_uuid, queries.clone(), None).await?;

    let summary = mixes::Model::count_by_first_letter(&db).await?;
    assert!(summary.contains(&("Mine".to_owned(), 1)), "{summary:?}");

    let groups = mixes::Model::get_groups(&db, vec!["Mine".to_owned()]).await?;
    let ids: Vec<i32> = groups
        .iter()
        .flat_map(|(_, items)| items.iter().map(|(model, _)| model.id))
        .collect();
    assert_eq!(ids, vec![mix.id]);

    let model = mixes::Model::get_by_ids(&db, &[mix.id])
        .await?
        .pop()
        .expect("the mix is found by its id");
    let collection = UnifiedCollection::from_model(&db, model, false).await?;
    assert_eq!(collection.collection_type, CollectionQueryType::Mix);
    let mut read_back = collection.queries;
    read_back.sort();
    let mut expected = queries;
    expected.sort();
    assert_eq!(read_back, expected);

    Ok(())
}
