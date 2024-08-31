use async_trait::async_trait;
use deunicode::deunicode;
use sea_orm::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, DatabaseTransaction, EntityTrait, QuerySelect};
use std::future::Future;
use std::pin::Pin;

pub trait DatabaseExecutor: Send + Sync {}

impl DatabaseExecutor for DatabaseConnection {}
impl DatabaseExecutor for DatabaseTransaction {}

pub fn first_char(s: &str) -> char {
    if let Some(first_char) = deunicode(s).chars().next() {
        first_char
    } else {
        '#'
    }
}

pub fn generate_group_name(x: &str) -> String {
    let c = first_char(x);

    if c.is_lowercase() {
        c.to_ascii_uppercase().to_string()
    } else if c.is_ascii_digit() || !c.is_alphabetic() {
        '#'.to_string()
    } else {
        c.to_string()
    }
}

#[async_trait]
pub trait CountByFirstLetter: EntityTrait {
    fn group_column() -> Self::Column;
    fn id_column() -> Self::Column;

    async fn count_by_first_letter(db: &DatabaseConnection) -> Result<Vec<(String, i32)>, DbErr> {
        let results = Self::find()
            .select_only()
            .column(Self::group_column())
            .column_as(Self::id_column().count(), "count")
            .group_by(Self::group_column())
            .into_tuple::<(String, i32)>()
            .all(db)
            .await?;

        Ok(results)
    }
}

pub fn create_count_by_first_letter<E>() -> impl for<'a> Fn(
    &'a DatabaseConnection,
) -> Pin<
    Box<dyn Future<Output = Result<Vec<(String, i32)>, DbErr>> + Send + 'a>,
>
where
    E: EntityTrait + CountByFirstLetter + Send + Sync + 'static,
{
    move |db: &DatabaseConnection| {
        let future = async move { E::count_by_first_letter(db).await };
        Box::pin(future)
    }
}

#[macro_export]
macro_rules! get_entity_to_cover_ids {
    ($db:expr, $entity_ids:expr, $related_entity:ident, $relation_column_name:ident, $magic_cover_art_id:expr) => {
        paste::paste! {{
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
            use std::collections::{HashMap, HashSet};
            use $crate::entities::media_files;

            // Fetch related media files for these entities
            let media_file_relations = <$related_entity::Entity>::find()
                .filter(<$related_entity::Column>::$relation_column_name.is_in($entity_ids.clone()))
                .all($db)
                .await?;

            let mut entity_to_media_file_ids: HashMap<i32, Vec<i32>> = HashMap::new();
            for relation in media_file_relations {
                entity_to_media_file_ids
                    .entry(relation.[<$relation_column_name:snake:lower>])
                    .or_default()
                    .push(relation.media_file_id);
            }

            let mut entity_to_cover_ids: HashMap<i32, HashSet<i32>> = HashMap::new();
            for (entity_id, media_file_ids) in entity_to_media_file_ids {
                let media_files = media_files::Entity::find()
                    .filter(media_files::Column::Id.is_in(media_file_ids))
                    .filter(media_files::Column::CoverArtId.ne($magic_cover_art_id))
                    .all($db)
                    .await?;

                let cover_ids = media_files
                    .into_iter()
                    .filter_map(|media_file| media_file.cover_art_id)
                    .collect::<HashSet<i32>>();

                entity_to_cover_ids.insert(entity_id, cover_ids);
            }

            Ok::<HashMap<i32, HashSet<i32>>, sea_orm::DbErr>(entity_to_cover_ids)
        }}
    };
}

#[macro_export]
macro_rules! get_cover_ids {
    ($fn_name:ident, $item_entity:ident, $related_entity:ident, $relation_column_name:ident) => {
        pub async fn $fn_name(
            db: &DatabaseConnection,
            entities: &[$item_entity::Model],
        ) -> Result<HashMap<i32, HashSet<i32>>, DbErr> {
            use $crate::get_entity_to_cover_ids;

            let entity_ids: Vec<i32> = entities.iter().map(|x| x.id).collect();
            let magic_cover_art_id = -1;

            get_entity_to_cover_ids!(
                db,
                entity_ids,
                $related_entity,
                $relation_column_name,
                magic_cover_art_id
            )
        }
    };
}

#[macro_export]
macro_rules! get_groups {
    ($fn_name:ident, $item_entity:ident, $related_entity:ident, $relation_column_name:ident) => {
        pub async fn $fn_name(
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
    };
}

#[macro_export]
macro_rules! get_all_ids {
    ($fn_name:ident, $related_entity:ident, $relation_column_name:ident) => {
        pub async fn $fn_name(
            db: &DatabaseConnection,
            id: i32,
        ) -> Result<Vec<i32>, sea_orm::DbErr> {
            // Fetch related media files for these entities
            let ids = <$related_entity::Entity>::find()
                .filter(<$related_entity::Column>::$relation_column_name.eq(id))
                .all(db)
                .await?
                .into_iter()
                .map(|x| x.media_file_id)
                .collect();

            Ok(ids)
        }
    };
}

#[macro_export]
macro_rules! get_by_ids {
    ($fn_name:ident, $item_entity:ident) => {
        pub async fn $fn_name(
            db: &DatabaseConnection,
            ids: &[i32],
        ) -> Result<Vec<$item_entity::Model>, sea_orm::DbErr> {
            let items = <$item_entity::Entity>::find()
                .filter(<$item_entity::Column>::Id.is_in(ids.to_vec()))
                .all(db)
                .await?;

            Ok(items)
        }
    };
}

#[macro_export]
macro_rules! get_by_id {
    ($fn_name:ident, $item_entity:ident) => {
        pub async fn $fn_name(
            db: &DatabaseConnection,
            id: i32,
        ) -> Result<Option<$item_entity::Model>, sea_orm::DbErr> {
            let item = $item_entity::Entity::find()
                .filter($item_entity::Column::Id.eq(id))
                .one(db)
                .await?;

            Ok(item)
        }
    };
}
