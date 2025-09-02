use std::collections::HashMap;

use anyhow::Result;
use deunicode::deunicode;
use log::warn;
use sea_orm::{
    ColumnTrait, DatabaseConnection, DbBackend, EntityTrait, FromQueryResult, QueryFilter,
    Statement,
};

use crate::entities::search_index;

use super::{collection::CollectionQueryType, utils::DatabaseExecutor};

pub fn convert_to_collection_types(input: Vec<String>) -> Vec<CollectionQueryType> {
    input
        .into_iter()
        .filter_map(|s| s.parse::<CollectionQueryType>().ok())
        .collect()
}

impl From<CollectionQueryType> for i64 {
    fn from(collection_type: CollectionQueryType) -> Self {
        match collection_type {
            CollectionQueryType::Track => 0,
            CollectionQueryType::Artist => 1,
            CollectionQueryType::Album => 2,
            CollectionQueryType::Directory => 3,
            CollectionQueryType::Playlist => 4,
            CollectionQueryType::Mix => 5,
            CollectionQueryType::Genre => 6,
        }
    }
}

impl TryFrom<i64> for CollectionQueryType {
    type Error = &'static str;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(CollectionQueryType::Track),
            1 => Ok(CollectionQueryType::Artist),
            2 => Ok(CollectionQueryType::Album),
            3 => Ok(CollectionQueryType::Directory),
            4 => Ok(CollectionQueryType::Playlist),
            5 => Ok(CollectionQueryType::Mix),
            6 => Ok(CollectionQueryType::Genre),
            _ => Err("Invalid value for CollectionType"),
        }
    }
}

pub async fn remove_term<E>(main_db: &E, entry_type: CollectionQueryType, id: i32) -> Result<()>
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

pub async fn add_term<E>(
    main_db: &E,
    entry_type: CollectionQueryType,
    id: i32,
    name: &str,
) -> Result<()>
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
    search_fields: Option<Vec<CollectionQueryType>>,
    n: usize,
) -> Result<HashMap<CollectionQueryType, Vec<i64>>> {
    let mut results: HashMap<CollectionQueryType, Vec<i64>> = HashMap::new();

    if query_str.is_empty() {
        return Ok(results);
    }

    for collection_type in [
        CollectionQueryType::Track,
        CollectionQueryType::Artist,
        CollectionQueryType::Album,
        CollectionQueryType::Directory,
        CollectionQueryType::Playlist,
    ] {
        if let Some(ref search_fields) = search_fields
            && !search_fields.contains(&collection_type)
        {
            continue;
        }

        let query_str = deunicode(query_str);

        let top_docs = SearchResult::find_by_statement(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            r#"SELECT * FROM search_index WHERE doc MATCH ? AND entry_type = ? ORDER BY rank LIMIT ?;"#,
            [ format!("\"{}\"", query_str.replace("\"", "\"\"")).into(), collection_type.to_string().into(), (n * 2).to_string().into() ],
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
