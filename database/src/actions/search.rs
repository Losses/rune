use std::collections::HashMap;

use anyhow::Result;
use deunicode::deunicode;
use log::warn;
use sea_orm::{
    ActiveValue::Set,
    ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect,
    sea_query::{Alias, BinOper, Expr, ExprTrait, Order},
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

    search_index::Entity::insert_many([name.to_owned(), deunicode(name)].map(|doc| {
        search_index::ActiveModel {
            id: Set(String::new()),
            key: Set(Some(id.to_string())),
            entry_type: Set(Some(entry_type.to_string())),
            doc: Set(Some(doc)),
        }
    }))
    .exec_without_returning(main_db)
    .await?;

    Ok(())
}

/// Builds the FTS5 query for what the user typed. Each word is quoted, so symbols such as `@`
/// are searched as text instead of read as query syntax, and the words may come in any order.
/// A trailing `*` makes the last word match as a prefix; it goes after the closing quote,
/// because inside the quotes it would be plain text.
fn match_expression(query_str: &str) -> String {
    let words = query_str.trim_end_matches('*');
    let mut expression = words
        .split_whitespace()
        .map(|word| format!("\"{}\"", deunicode(word).trim().replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ");
    if words.len() != query_str.len() && !expression.is_empty() {
        expression.push('*');
    }
    expression
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

    let expression = match_expression(query_str);
    if expression.is_empty() {
        return Ok(results);
    }

    // Folders are not indexed.
    for collection_type in [
        CollectionQueryType::Track,
        CollectionQueryType::Artist,
        CollectionQueryType::Album,
        CollectionQueryType::Playlist,
        CollectionQueryType::Genre,
    ] {
        if let Some(ref search_fields) = search_fields
            && !search_fields.contains(&collection_type)
        {
            continue;
        }

        let top_docs = search_index::Entity::find()
            .select_only()
            .columns([
                search_index::Column::Key,
                search_index::Column::EntryType,
                search_index::Column::Doc,
            ])
            .filter(
                Expr::col(search_index::Column::Doc)
                    .binary(BinOper::Custom("MATCH"), expression.clone()),
            )
            .filter(search_index::Column::EntryType.eq(collection_type.to_string()))
            .order_by(Expr::col(Alias::new("rank")), Order::Asc)
            .limit(n.saturating_mul(2) as u64)
            .into_model::<SearchResult>()
            .all(main_db)
            .await?;

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
