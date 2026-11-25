use anyhow::Result;
use sea_orm::Database;

use ::database::{
    actions::{
        collection::CollectionQueryType,
        search::{add_term, search_for},
    },
    connection::initialize_db,
};

#[tokio::test]
async fn genres_are_searched_by_default() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    initialize_db(&db, "search-genres-test").await?;
    add_term(&db, CollectionQueryType::Genre, 7, "Shoegaze").await?;
    add_term(&db, CollectionQueryType::Track, 1, "Shoegaze Song").await?;

    let results = search_for(&db, "Shoegaze", None, 30).await?;
    assert_eq!(
        results.get(&CollectionQueryType::Genre).map(|ids| ids[0]),
        Some(7)
    );
    assert!(results.contains_key(&CollectionQueryType::Track));

    let genres_only =
        search_for(&db, "Shoegaze", Some(vec![CollectionQueryType::Genre]), 30).await?;
    assert_eq!(
        genres_only.keys().collect::<Vec<_>>(),
        vec![&CollectionQueryType::Genre]
    );
    Ok(())
}
