use std::{collections::HashSet, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;

use crate::connection::MainDbConnection;

#[async_trait]
pub trait CollectionQuery: Send + Sync + 'static {
    fn collection_type() -> i32;
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
    ($model:ty, $entity:ty, $collection_type:expr, $type_name:expr, $query_operator:expr, $get_groups:path, $get_by_ids:path, $list:path) => {
        #[async_trait]
        impl CollectionQuery for $model {
            fn collection_type() -> i32 {
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
