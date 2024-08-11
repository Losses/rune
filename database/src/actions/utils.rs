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
macro_rules! generate_get_groups_fn {
    ($fn_name:ident, $entity:ident, $media_file_entity:ident, $xxid_col:ident) => {
        pub async fn $fn_name(
            db: &DatabaseConnection,
            groups: Vec<String>,
        ) -> Result<Vec<(String, Vec<($entity::Model, HashSet<i32>)>)>, sea_orm::DbErr> {
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
            use std::collections::{HashMap, HashSet};

            // Step 0: Get the magic coverart ID
            let magic_cover_art = media_cover_art::Entity::find()
                .filter(media_cover_art::Column::FileHash.eq(String::new()))
                .one(db)
                .await;

            let magic_cover_art_id = magic_cover_art.ok().flatten().map_or(-1, |s| s.id);

            // Step 1: Fetch entities belonging to the specified groups
            let entities: Vec<$entity::Model> = $entity::Entity::find()
                .filter($entity::Column::Group.is_in(groups.clone()))
                .all(db)
                .await?;

            // Step 2: Collect entity IDs
            let entity_ids: Vec<i32> = entities.iter().map(|x| x.id).collect();

            // Step 3: Fetch related media files for these entities
            let media_files = $entity::Entity::find()
                .filter($entity::Column::Id.is_in(entity_ids.clone()))
                .find_with_related($media_file_entity::Entity)
                .all(db)
                .await?
                .into_iter()
                .flat_map(|(entity, media_file_vec)| {
                    media_file_vec
                        .into_iter()
                        .map(move |media_file| (entity.id, media_file))
                })
                .collect::<Vec<_>>();

            // Step 4: Map entity IDs to their media file IDs
            let mut entity_to_media_file_ids: HashMap<i32, Vec<i32>> = HashMap::new();
            for (entity_id, media_file) in media_files {
                entity_to_media_file_ids
                    .entry(entity_id)
                    .or_default()
                    .push(media_file.media_file_id);
            }

            // Step 5: Map entity IDs to their cover IDs
            let mut entity_to_cover_ids: HashMap<i32, HashSet<i32>> = HashMap::new();
            for (entity_id, media_file_ids) in entity_to_media_file_ids {
                let media_files = media_files::Entity::find()
                    .filter(media_files::Column::Id.is_in(media_file_ids))
                    .filter(media_files::Column::CoverArtId.ne(magic_cover_art_id))
                    .all(db)
                    .await?;

                let cover_ids = media_files
                    .into_iter()
                    .filter_map(|media_file| media_file.cover_art_id)
                    .collect::<HashSet<i32>>();

                entity_to_cover_ids.insert(entity_id, cover_ids);
            }

            // Step 6: Group entities by their group and associate cover IDs
            let mut grouped_entities: HashMap<String, Vec<($entity::Model, HashSet<i32>)>> =
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

            // Step 7: Prepare the final result
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
