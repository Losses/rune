use anyhow::Result;
use sea_orm::{Database, DatabaseConnection};

use ::database::{
    actions::{
        collection::CollectionQueryType,
        search::{add_term, search_for},
    },
    connection::initialize_db,
};

const NODE_ID: &str = "search-test";

async fn library(tracks: &[(i32, &str)]) -> Result<DatabaseConnection> {
    let db = Database::connect("sqlite::memory:").await?;
    initialize_db(&db, NODE_ID).await?;
    for (id, name) in tracks {
        add_term(&db, CollectionQueryType::Track, *id, name).await?;
    }
    Ok(db)
}

/// The tracks found for `query`, each once, in no particular order.
async fn tracks_for(db: &DatabaseConnection, query: &str) -> Result<Vec<i64>> {
    let mut ids = search_for(db, query, Some(vec![CollectionQueryType::Track]), 30)
        .await?
        .remove(&CollectionQueryType::Track)
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

// The search page adds `*` to what the user typed.
#[tokio::test]
async fn a_word_being_typed_matches_as_a_prefix() -> Result<()> {
    let db = library(&[(1, "The Beatles - Help"), (2, "Beat It")]).await?;
    assert_eq!(tracks_for(&db, "Beat*").await?, vec![1, 2]);
    assert_eq!(tracks_for(&db, "Beatl*").await?, vec![1]);
    // Without the `*` only whole words match.
    assert_eq!(tracks_for(&db, "Beat").await?, vec![2]);
    Ok(())
}

#[tokio::test]
async fn words_match_in_any_order() -> Result<()> {
    let db = library(&[(1, "The Beatles - Help")]).await?;
    assert_eq!(tracks_for(&db, "help beatles*").await?, vec![1]);
    assert_eq!(tracks_for(&db, "help beat*").await?, vec![1]);
    assert_eq!(tracks_for(&db, "help stones*").await?, Vec::<i64>::new());
    Ok(())
}

#[tokio::test]
async fn symbols_are_searched_as_text() -> Result<()> {
    let db = library(&[(1, "THE IDOLM@STER"), (2, "AC/DC Live")]).await?;
    assert_eq!(tracks_for(&db, "IDOLM@STER*").await?, vec![1]);
    assert_eq!(tracks_for(&db, "idolm@st*").await?, vec![1]);
    assert_eq!(tracks_for(&db, "AC/DC*").await?, vec![2]);
    for query in ["\"Beat*", "AND*", "a OR b*", "(x*", "-*", "*", " ", ""] {
        assert_eq!(
            tracks_for(&db, query).await?,
            Vec::<i64>::new(),
            "{query:?}"
        );
    }
    Ok(())
}

// Chinese is searched by its pinyin, one word per character, so the characters of a word keep
// their order.
#[tokio::test]
async fn chinese_matches_by_its_characters_in_order() -> Result<()> {
    let db = library(&[(1, "周杰伦"), (2, "伦杰")]).await?;
    assert_eq!(tracks_for(&db, "杰伦*").await?, vec![1]);
    assert_eq!(tracks_for(&db, "周*").await?, vec![1]);
    Ok(())
}
