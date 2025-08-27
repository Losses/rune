use std::{collections::HashSet, fmt, str::FromStr};

use anyhow::Result;
use async_trait::async_trait;
use thiserror::Error;

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
    Genre,
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
            CollectionQueryType::Genre => 5,
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
            CollectionQueryType::Genre => "genre",
            CollectionQueryType::Directory => "directory",
            CollectionQueryType::Album => "album",
            CollectionQueryType::Playlist => "playlist",
            CollectionQueryType::Mix => "mix",
        };
        write!(f, "{s}")
    }
}

pub struct UnifiedCollection {
    pub id: i32,
    pub name: String,
    pub queries: Vec<(String, String)>,
    pub collection_type: CollectionQueryType,
    pub readonly: bool,
}

impl UnifiedCollection {
    pub async fn from_model<T: CollectionQuery>(
        main_db: &MainDbConnection,
        model: T,
        readonly: bool,
    ) -> Result<Self> {
        let collection: UnifiedCollection = UnifiedCollection {
            id: model.id(),
            name: model.name().to_owned(),
            queries: T::query_builder(main_db, model.id()).await?,
            collection_type: T::collection_type(),
            readonly,
        };

        Ok(collection)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CollectionQueryListMode {
    Forward,
    Reverse,
    Random,
    Name,
}

#[derive(Debug, Clone, Error)]
pub enum ParseCollectionQueryListModeError {
    #[error("Invalid type for CollectionQueryListMode")]
    InvalidType,
}

impl FromStr for CollectionQueryListMode {
    type Err = ParseCollectionQueryListModeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "name" => Ok(CollectionQueryListMode::Name),
            "forward" => Ok(CollectionQueryListMode::Forward),
            "oldest" => Ok(CollectionQueryListMode::Forward),
            "reverse" => Ok(CollectionQueryListMode::Reverse),
            "newest" => Ok(CollectionQueryListMode::Reverse),
            "random" => Ok(CollectionQueryListMode::Random),
            _ => Err(ParseCollectionQueryListModeError::InvalidType),
        }
    }
}

#[async_trait]
pub trait CollectionQuery: Send + Sync + 'static {
    fn collection_type() -> CollectionQueryType;
    async fn query_builder(main_db: &MainDbConnection, id: i32) -> Result<Vec<(String, String)>>;
    async fn count_by_first_letter(main_db: &MainDbConnection) -> Result<Vec<(String, i32)>>;
    async fn get_groups(
        main_db: &MainDbConnection,
        group_titles: Vec<String>,
    ) -> Result<Vec<(String, Vec<(Self, HashSet<i32>)>)>>
    where
        Self: std::marker::Sized;
    async fn get_by_ids(main_db: &MainDbConnection, ids: &[i32]) -> Result<Vec<Self>>
    where
        Self: std::marker::Sized;
    async fn list(
        main_db: &MainDbConnection,
        limit: u64,
        mode: CollectionQueryListMode,
    ) -> Result<Vec<Self>>
    where
        Self: std::marker::Sized;
    fn id(&self) -> i32;
    fn name(&self) -> &str;
    fn readonly(&self) -> bool;
}

#[macro_export]
macro_rules! collection_query {
    (
        $item_entity:ident,
        $collection_type:expr,
        $query_operator:expr,
        $related_entity:ident,
        $relation_column_name:ident
    ) => {
        // First generate the get_groups function
        async fn get_groups_internal(
            db: &DatabaseConnection,
            groups: Vec<String>,
        ) -> Result<Vec<(String, Vec<($item_entity::Model, HashSet<i32>)>)>, sea_orm::DbErr> {
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
            use std::collections::{HashMap, HashSet};
            use $crate::actions::cover_art::get_magic_cover_art_id;
            use $crate::get_entity_to_cover_ids;

            // Step 0: Get the magic coverart ID
            let magic_cover_art_id = get_magic_cover_art_id(db).await;

            // Step 1: Fetch entities belonging to the specified groups
            let entities: Vec<$item_entity::Model> = $item_entity::Entity::find()
                .filter($item_entity::Column::Group.is_in(groups.clone()))
                .order_by_asc(<$item_entity::Column>::Name)
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

            async fn query_builder(
                _main_db: &MainDbConnection,
                id: i32,
            ) -> Result<Vec<(String, String)>> {
                Ok([($query_operator, id.to_string())].to_vec())
            }

            async fn count_by_first_letter(
                main_db: &MainDbConnection,
            ) -> Result<Vec<(String, i32)>> {
                use anyhow::Context;
                use sea_orm::QuerySelect;

                let group_column = <$item_entity::Entity>::group_column();

                let results = $item_entity::Entity::find()
                    .select_only()
                    .column::<$item_entity::Column>(group_column)
                    .column_as(<$item_entity::Entity>::id_column().count(), "count")
                    .group_by::<$item_entity::Column>(group_column)
                    .into_tuple::<(String, i32)>()
                    .all(main_db)
                    .await
                    .with_context(|| "Failed to count collection by first letter")?;

                Ok(results)
            }

            async fn get_groups(
                main_db: &MainDbConnection,
                group_titles: Vec<String>,
            ) -> Result<Vec<(String, Vec<(Self, HashSet<i32>)>)>> {
                get_groups_internal(&main_db, group_titles)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to get collection groups: {e}"))
            }

            async fn get_by_ids(main_db: &MainDbConnection, ids: &[i32]) -> Result<Vec<Self>> {
                use anyhow::Context;

                <$item_entity::Entity>::find()
                    .filter(<$item_entity::Column>::Id.is_in(ids.to_vec()))
                    .all(main_db)
                    .await
                    .with_context(|| "Failed to get collection item by ids")
            }

            async fn list(
                main_db: &MainDbConnection,
                limit: u64,
                mode: $crate::actions::collection::CollectionQueryListMode,
            ) -> Result<Vec<Self>> {
                use anyhow::Context;
                use sea_orm::{
                    FromQueryResult, Order, QueryOrder, QuerySelect, QueryTrait, sea_query::Func,
                    sea_query::SimpleExpr,
                };
                use $crate::actions::collection::CollectionQueryListMode;

                match mode {
                    CollectionQueryListMode::Name => {
                        $item_entity::Entity::find()
                            .order_by_asc(<$item_entity::Column>::Name)
                            .limit(limit)
                            .all(main_db)
                            .await
                    }
                    CollectionQueryListMode::Forward => {
                        $item_entity::Entity::find().limit(limit).all(main_db).await
                    }
                    CollectionQueryListMode::Reverse => {
                        $item_entity::Entity::find()
                            .order_by_desc(<$item_entity::Column>::Id)
                            .limit(limit)
                            .all(main_db)
                            .await
                    }
                    CollectionQueryListMode::Random => {
                        let mut query: sea_orm::sea_query::SelectStatement =
                            $item_entity::Entity::find().as_query().to_owned();

                        let select = query
                            .order_by_expr(SimpleExpr::FunctionCall(Func::random()), Order::Asc)
                            .limit(limit);

                        let statement = main_db.get_database_backend().build(select);

                        $item_entity::Model::find_by_statement(statement)
                            .all(main_db)
                            .await
                    }
                }
                .with_context(|| "Failed to get collection list")
            }

            fn id(&self) -> i32 {
                self.id
            }

            fn name(&self) -> &str {
                &self.name
            }

            fn readonly(&self) -> bool {
                false
            }
        }
    };
}
