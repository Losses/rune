use core::fmt;
use std::collections::HashMap;
use std::str::FromStr;

use anyhow::Result;
use deunicode::deunicode;
use log::warn;
use sea_orm::{
    ColumnTrait, DatabaseConnection, DbBackend, EntityTrait, FromQueryResult, QueryFilter,
    Statement,
};

use crate::entities::search_index;

use super::utils::DatabaseExecutor;

#[derive(Eq, Hash, PartialEq, Clone, Debug)]
pub enum CollectionType {
    Track,
    Artist,
    Directory,
    Album,
    Playlist,
}

#[derive(Debug, Clone)]
pub enum ParseCollectionTypeError {
    InvalidType,
}

impl fmt::Display for ParseCollectionTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid collection type")
    }
}

impl std::error::Error for ParseCollectionTypeError {}

impl FromStr for CollectionType {
    type Err = ParseCollectionTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "track" => Ok(CollectionType::Track),
            "artist" => Ok(CollectionType::Artist),
            "directory" => Ok(CollectionType::Directory),
            "album" => Ok(CollectionType::Album),
            "playlist" => Ok(CollectionType::Playlist),
            _ => Err(ParseCollectionTypeError::InvalidType),
        }
    }
}

impl fmt::Display for CollectionType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            CollectionType::Track => "track",
            CollectionType::Artist => "artist",
            CollectionType::Directory => "directory",
            CollectionType::Album => "album",
            CollectionType::Playlist => "playlist",
        };
        write!(f, "{}", s)
    }
}

pub fn convert_to_collection_types(input: Vec<String>) -> Vec<CollectionType> {
    input
        .into_iter()
        .filter_map(|s| s.parse::<CollectionType>().ok())
        .collect()
}

impl From<CollectionType> for i64 {
    fn from(collection_type: CollectionType) -> Self {
        match collection_type {
            CollectionType::Track => 0,
            CollectionType::Artist => 1,
            CollectionType::Album => 2,
            CollectionType::Directory => 3,
            CollectionType::Playlist => 4,
        }
    }
}

impl TryFrom<i64> for CollectionType {
    type Error = &'static str;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(CollectionType::Track),
            1 => Ok(CollectionType::Artist),
            2 => Ok(CollectionType::Album),
            3 => Ok(CollectionType::Directory),
            4 => Ok(CollectionType::Playlist),
            _ => Err("Invalid value for CollectionType"),
        }
    }
}

pub async fn remove_term<E>(main_db: &E, entry_type: CollectionType, id: i32) -> Result<()>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    search_index::Entity::delete_many()
        .filter(search_index::Column::Key.eq(id.to_string()))
        .filter(search_index::Column::EntryType.eq(entry_type.to_string()))
        .exec(main_db)
        .await?;

    Ok(())
}

pub async fn add_term<E>(main_db: &E, entry_type: CollectionType, id: i32, name: &str) -> Result<()>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    remove_term(main_db, entry_type.clone(), id).await?;

    search_index::Entity::find().from_raw_sql(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        r#"INSERT INTO search_index (id, key, entry_type, doc) VALUES ('', ?, ?, ?), ('', ?, ?, ?);"#,
        [
            id.to_string().into(),
            entry_type.to_string().into(),
            name.to_string().into(),
            id.to_string().into(),
            entry_type.to_string().into(),
            deunicode(name).into(),
        ],
    )).all(main_db).await?;

    Ok(())
}

#[derive(Debug, FromQueryResult)]
pub struct SearchResult {
    pub key: String,
    pub entry_type: String,
    pub doc: String,
}

pub async fn search_for(
    main_db: &DatabaseConnection,
    query_str: &str,
    search_fields: Option<Vec<CollectionType>>,
    n: usize,
) -> Result<HashMap<CollectionType, Vec<i64>>> {
    let mut results: HashMap<CollectionType, Vec<i64>> = HashMap::new();

    if query_str.is_empty() {
        return Ok(results);
    }

    for collection_type in [
        CollectionType::Track,
        CollectionType::Artist,
        CollectionType::Album,
        CollectionType::Directory,
        CollectionType::Playlist,
    ] {
        if let Some(ref search_fields) = search_fields {
            if !search_fields.contains(&collection_type) {
                continue;
            }
        }

        let top_docs = SearchResult::find_by_statement(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            r#"SELECT * FROM search_index WHERE doc MATCH ? AND entry_type = ? ORDER BY rank LIMIT ?;"#,
            [ query_str.to_string().into(), collection_type.to_string().into(), (n * 2).to_string().into() ],
        )).all(main_db).await?;

        for item in top_docs {
            let id = item.key.parse::<i64>();
            if let Ok(id) = id {
                results.entry(collection_type.clone()).or_default().push(id);
            } else {
                warn!("Invalid document ID found!");
            }
        }
    }

    Ok(results)
}
