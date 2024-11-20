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
    async fn query_builder(id: i32) -> Vec<(String, String)>;
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
    (
        $item_entity:ident, 
        $entity:ty, 
        $collection_type:expr, 
        $query_operator:expr,
        $related_entity:ident,
        $relation_column_name:ident,
        $list:path
    ) => {
        // First generate the get_groups function
        async fn get_groups_internal(
            db: &DatabaseConnection,
            groups: Vec<String>,
        ) -> Result<Vec<(String, Vec<($item_entity::Model, HashSet<i32>)>)>, sea_orm::DbErr> {
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
            use std::collections::{HashMap, HashSet};
            use $crate::actions::cover_art::get_magic_cover_art_id;
            use $crate::get_entity_to_cover_ids;

            // Step 0: Get the magic coverart ID
            let magic_cover_art_id = get_magic_cover_art_id(db).await;

            // Step 1: Fetch entities belonging to the specified groups
            let entities: Vec<$item_entity::Model> = $item_entity::Entity::find()
                .filter($item_entity::Column::Group.is_in(groups.clone()))
                .all(db)
                .await?;

            // Step 2: Collect entity IDs
            let entity_ids: Vec<i32> = entities.iter().map(|x| x.id).collect();

            // Step 3: Get entity to cover IDs mapping
            let entity_to_cover_ids = get_entity_to_cover_ids!(
                db,
                entity_ids,
                $related_entity,
                $relation_column_name,
                magic_cover_art_id
            )?;

            // Step 4: Group entities by their group and associate cover IDs
            let mut grouped_entities: HashMap<String, Vec<($item_entity::Model, HashSet<i32>)>> =
                HashMap::new();
            for entity in entities {
                let cover_ids = entity_to_cover_ids
                    .get(&entity.id)
                    .cloned()
                    .unwrap_or_default();
                grouped_entities
                    .entry(entity.group.clone())
                    .or_default()
                    .push((entity, cover_ids));
            }

            // Step 5: Prepare the final result
            let result = groups
                .into_iter()
                .map(|group| {
                    let entities_in_group = grouped_entities.remove(&group).unwrap_or_default();
                    (group, entities_in_group)
                })
                .collect();

            Ok(result)
        }

        // Then implement CollectionQuery
        #[async_trait]
        impl CollectionQuery for $item_entity::Model {
            fn collection_type() -> CollectionQueryType {
                $collection_type
            }
            
            async fn query_builder(id: i32) -> Vec<(String, String)> {
                [($query_operator, id.to_string())].to_vec()
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
                get_groups_internal(&main_db, group_titles)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to get collection groups: {}", e))
            }
            
            async fn get_by_ids(main_db: &Arc<MainDbConnection>, ids: &[i32]) -> Result<Vec<Self>> {
                <$item_entity::Entity>::find()
                .filter(<$item_entity::Column>::Id.is_in(ids.to_vec()))
                .all(main_db.as_ref())
                .await
                .with_context(|| "Failed to get collection item by ids")
            }
            
            async fn list(main_db: &Arc<MainDbConnection>, limit: u64) -> Result<Vec<Self>> {
                use sea_orm::QuerySelect;

                $item_entity::Entity::find().limit(limit).all(main_db.as_ref()).await.with_context(|| "Failed to get collection list")
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
