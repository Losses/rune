use std::{collections::HashSet, fmt, str::FromStr, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;

use crate::connection::MainDbConnection;

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

#[derive(Eq, Hash, PartialEq, Clone, Debug)]
pub enum CollectionQueryType {
    Track,
    Artist,
    Directory,
    Album,
    Mix,
    Playlist,
}

impl From<CollectionQueryType> for i32 {
    fn from(val: CollectionQueryType) -> Self {
        match val {
            CollectionQueryType::Album => 0,
            CollectionQueryType::Artist => 1,
            CollectionQueryType::Playlist => 2,
            CollectionQueryType::Mix => 3,
            CollectionQueryType::Track => 4,
            CollectionQueryType::Directory => 5,
        }
    }
}

impl FromStr for CollectionQueryType {
    type Err = ParseCollectionTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "track" => Ok(CollectionQueryType::Track),
            "artist" => Ok(CollectionQueryType::Artist),
            "directory" => Ok(CollectionQueryType::Directory),
            "album" => Ok(CollectionQueryType::Album),
            "playlist" => Ok(CollectionQueryType::Playlist),
            "mix" => Ok(CollectionQueryType::Mix),
            _ => Err(ParseCollectionTypeError::InvalidType),
        }
    }
}

impl fmt::Display for CollectionQueryType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            CollectionQueryType::Track => "track",
            CollectionQueryType::Artist => "artist",
            CollectionQueryType::Directory => "directory",
            CollectionQueryType::Album => "album",
            CollectionQueryType::Playlist => "playlist",
            CollectionQueryType::Mix => "mix",
        };
        write!(f, "{}", s)
    }
}

#[async_trait]
pub trait CollectionQuery: Send + Sync + 'static {
    fn collection_type() -> CollectionQueryType;
    fn query_operator() -> &'static str;
    async fn count_by_first_letter(main_db: &Arc<MainDbConnection>) -> Result<Vec<(String, i32)>>;
    async fn get_groups(
        main_db: &Arc<MainDbConnection>,
        group_titles: Vec<String>,
    ) -> Result<Vec<(String, Vec<(Self, HashSet<i32>)>)>>
    where
        Self: std::marker::Sized;
    async fn get_by_ids(main_db: &Arc<MainDbConnection>, ids: &[i32]) -> Result<Vec<Self>>
    where
        Self: std::marker::Sized;
    async fn list(main_db: &Arc<MainDbConnection>, limit: u64) -> Result<Vec<Self>>
    where
        Self: std::marker::Sized;

    fn id(&self) -> i32;
    fn name(&self) -> &str;
}

#[macro_export]
macro_rules! collection_query {
    ($model:ty, $entity:ty, $collection_type:expr, $query_operator:expr, $get_groups:path, $get_by_ids:path, $list:path) => {
        #[async_trait]
        impl CollectionQuery for $model {
            fn collection_type() -> CollectionQueryType {
                $collection_type
            }
            fn query_operator() -> &'static str {
                $query_operator
            }
            async fn count_by_first_letter(
                main_db: &Arc<MainDbConnection>,
            ) -> Result<Vec<(String, i32)>> {
                create_count_by_first_letter::<$entity>()(main_db)
                    .await
                    .with_context(|| "Failed to count collection by first letter")
            }
            async fn get_groups(
                main_db: &Arc<MainDbConnection>,
                group_titles: Vec<String>,
            ) -> Result<Vec<(String, Vec<(Self, HashSet<i32>)>)>> {
                $get_groups(main_db, group_titles)
                    .await
                    .with_context(|| "Failed to get collection groups")
            }
            async fn get_by_ids(main_db: &Arc<MainDbConnection>, ids: &[i32]) -> Result<Vec<Self>> {
                $get_by_ids(main_db, ids)
                    .await
                    .with_context(|| "Failed to get collection item by ids")
            }
            async fn list(main_db: &Arc<MainDbConnection>, limit: u64) -> Result<Vec<Self>> {
                $list(main_db, limit)
                    .await
                    .with_context(|| "Failed to get collection list")
            }

            fn id(&self) -> i32 {
                self.id
            }

            fn name(&self) -> &str {
                &self.name
            }
        }
    };
}
