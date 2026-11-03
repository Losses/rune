use crate::error::ExpectedError;
use std::ffi::OsStr;
mod access;
mod content;
mod integrity;

pub use access::{
    AudioAvailability, AudioOrigin, audio_availability, audio_origin, is_materialized,
    materialized_path, remote_required_ids, resolve_local_id,
};
pub use content::{
    MANAGED_AUDIO_DIRECTORY, MAX_BLOB_BYTES, attach_download, blob_path, blob_path_for_track,
    cached_blob_path, catalog_track, hash_file, is_managed_directory, local_audio_path,
    remote_content_ids, validate_digest, validate_extension,
};
pub use integrity::{ManagedAudioVerifier, VerificationStep};

use sea_orm::ExprTrait;
use std::collections::HashMap;
use std::path::Path;

use crate::actions::{collection::CollectionQueryType, search::remove_term};
use crate::entities::{
    media_file_albums, media_file_artists, media_file_fingerprint, media_file_genres,
    media_metadata,
};
use crate::sync::SeaOrmSyncStore;
use crate::sync::model::{Collection, ObjectKey};
use anyhow::Result;
use metadata::describe::FileDescription;
use rust_decimal::prelude::ToPrimitive;
use sea_orm::entity::prelude::*;
use sea_orm::{
    ColumnTrait, DatabaseTransaction, EntityTrait, FromQueryResult, Order, QueryFilter, QueryOrder,
    QuerySelect, QueryTrait,
};

use migration::{Func, SimpleExpr};

use crate::entities::media_files;

pub async fn get_files_by_ids(
    db: &DatabaseConnection,
    ids: &[i32],
) -> Result<Vec<media_files::Model>, DbErr> {
    media_files::Entity::find()
        .filter(media_files::Column::Id.is_in(ids.to_vec()))
        .all(db)
        .await
}
pub async fn get_file_by_id(
    db: &DatabaseConnection,
    id: i32,
) -> Result<Option<media_files::Model>, DbErr> {
    media_files::Entity::find_by_id(id).one(db).await
}
pub async fn list_files(db: &DatabaseConnection, n: u64) -> Result<Vec<media_files::Model>, DbErr> {
    media_files::Entity::find().limit(n).all(db).await
}

pub async fn get_ordered_files_by_ids(
    main_db: &DatabaseConnection,
    file_ids: &[i32],
) -> Result<Vec<media_files::Model>> {
    let files = get_files_by_ids(main_db, file_ids).await?;

    // Create a HashMap to store file_id -> file mapping
    let file_map: HashMap<i32, media_files::Model> =
        files.into_iter().map(|file| (file.id, file)).collect();

    // Reorder files based on the order of recommendation_ids
    let ordered_files: Vec<media_files::Model> = file_ids
        .iter()
        .filter_map(|id| file_map.get(id).cloned())
        .collect();

    Ok(ordered_files)
}

pub async fn get_random_files(
    db: &DatabaseConnection,
    n: usize,
) -> Result<Vec<media_files::Model>, sea_orm::DbErr> {
    let mut query: sea_orm::sea_query::SelectStatement =
        media_files::Entity::find().as_query().to_owned();
    let select = query
        .order_by_expr(SimpleExpr::FunctionCall(Func::random()), Order::Asc)
        .limit(n as u64);
    let statement = db.get_database_backend().build(select);

    let files = media_files::Model::find_by_statement(statement)
        .all(db)
        .await?;

    Ok(files)
}

pub async fn get_file_by_path(
    db: &DatabaseConnection,
    relative_path: &Path,
) -> Result<Option<media_files::Model>, sea_orm::DbErr> {
    let directory = relative_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_str()
        .unwrap_or("")
        .to_string();
    let file_name = relative_path
        .file_name()
        .unwrap_or_else(|| OsStr::new(""))
        .to_str()
        .unwrap_or("")
        .to_string();

    let file = media_files::Entity::find()
        .filter(media_files::Column::Directory.eq(directory))
        .filter(media_files::Column::FileName.eq(file_name))
        .one(db)
        .await?;

    Ok(file)
}

pub async fn get_file_id_from_path(
    db: &DatabaseConnection,
    root_path: &Path,
    file_path: &Path,
) -> Result<i32, String> {
    // Check if the file exists as an absolute path
    let absolute_path = if file_path.is_absolute() {
        file_path.to_path_buf()
    } else {
        root_path.join(file_path)
    };

    if !absolute_path.exists() {
        return Err(format!("File does not exist: {absolute_path:?}"));
    }

    let relative_path = match absolute_path.strip_prefix(root_path) {
        Ok(path) => path,
        Err(_) => {
            return Err(format!(
                "File is not within the specified library path: {absolute_path:?}"
            ));
        }
    };

    let file_info = match get_file_by_path(db, relative_path).await {
        Ok(Some(file_info)) => file_info,
        Ok(_none) => {
            return Err(format!("File is not in the database: {relative_path:?}"));
        }
        Err(e) => {
            return Err(format!("Failed to query the database: {e}"));
        }
    };

    Ok(file_info.id)
}

pub async fn get_media_files(
    db: &DatabaseConnection,
    cursor: usize,
    page_size: usize,
) -> Result<Vec<media_files::Model>, sea_orm::DbErr> {
    media_files::Entity::find()
        .cursor_by(media_files::Column::Id)
        .after(cursor as i32)
        .first(page_size as u64)
        .all(db)
        .await
}

pub async fn get_reverse_listed_media_files(
    main_db: &DatabaseConnection,
    cursor: usize,
    page_size: usize,
) -> Result<Vec<media_files::Model>, sea_orm::DbErr> {
    media_files::Entity::find()
        .order_by(media_files::Column::Id, Order::Desc)
        .offset(cursor as u64)
        .limit(page_size as u64)
        .all(main_db)
        .await
}

pub async fn get_file_ids_by_descriptions(
    db: &DatabaseConnection,
    descriptions: &[Option<FileDescription>],
) -> Result<Vec<i32>, DbErr> {
    if descriptions.is_empty() {
        return Ok(vec![]);
    }

    let mut conditions = sea_orm::Condition::any();

    for description in descriptions {
        match description {
            Some(x) => {
                conditions = conditions.add(
                    media_files::Column::Directory
                        .eq(x.directory.clone())
                        .and(media_files::Column::FileName.eq(x.file_name.clone())),
                );
            }
            _none => {}
        }
    }

    let file_entries = media_files::Entity::find()
        .filter(conditions)
        .all(db)
        .await?;

    let file_ids = file_entries.into_iter().map(|entry| entry.id).collect();

    Ok(file_ids)
}

pub async fn get_duration_by_file_id(db: &DatabaseConnection, file_id: i32) -> Result<f64> {
    let analysis_entry: Option<media_files::Model> = media_files::Entity::find()
        .filter(media_files::Column::Id.eq(file_id))
        .one(db)
        .await?;

    if let Some(entry) = analysis_entry {
        Ok(entry.duration.to_f64().unwrap())
    } else {
        Err(ExpectedError::NotFound {
            what: "analysis record",
        }
        .into())
    }
}

pub async fn get_media_files_count(db: &DatabaseConnection) -> Result<u64, sea_orm::DbErr> {
    media_files::Entity::find().count(db).await
}

/// Remove a locally hosted track and its scanner-owned catalog records.
pub async fn remove_track(
    tx: &DatabaseTransaction,
    store: &SeaOrmSyncStore,
    file_id: i32,
) -> Result<()> {
    let file = media_files::Entity::find_by_id(file_id)
        .one(tx)
        .await?
        .ok_or(ExpectedError::NotFound { what: "track" })?;
    let track = file.hlc_uuid.as_str();
    remove_track_links::<media_file_albums::Entity>(
        tx,
        store,
        Collection::TrackAlbums,
        media_file_albums::Column::TrackUuid,
        media_file_albums::Column::HlcUuid,
        track,
    )
    .await?;
    remove_track_links::<media_file_artists::Entity>(
        tx,
        store,
        Collection::FileArtists,
        media_file_artists::Column::TrackUuid,
        media_file_artists::Column::HlcUuid,
        track,
    )
    .await?;
    remove_track_links::<media_file_genres::Entity>(
        tx,
        store,
        Collection::FileGenres,
        media_file_genres::Column::TrackUuid,
        media_file_genres::Column::HlcUuid,
        track,
    )
    .await?;
    remove_track_links::<media_file_fingerprint::Entity>(
        tx,
        store,
        Collection::Fingerprints,
        media_file_fingerprint::Column::TrackUuid,
        media_file_fingerprint::Column::HlcUuid,
        track,
    )
    .await?;
    media_metadata::Entity::delete_many()
        .filter(media_metadata::Column::FileId.eq(file_id))
        .exec(tx)
        .await?;
    media_files::Entity::delete_by_id(file_id).exec(tx).await?;
    remove_term(tx, CollectionQueryType::Track, file_id).await?;
    author_removal(tx, store, Collection::Tracks, track).await
}

/// Delete the rows of `E` that belong to `track` and author their removal.
async fn remove_track_links<E: EntityTrait>(
    tx: &DatabaseTransaction,
    store: &SeaOrmSyncStore,
    collection: Collection,
    track_column: E::Column,
    uuid_column: E::Column,
    track: &str,
) -> Result<()> {
    let ids: Vec<String> = E::find()
        .select_only()
        .column(uuid_column)
        .filter(track_column.eq(track))
        .into_tuple()
        .all(tx)
        .await?;
    E::delete_many()
        .filter(track_column.eq(track))
        .exec(tx)
        .await?;
    for id in ids {
        author_removal(tx, store, collection, &id).await?;
    }
    Ok(())
}

async fn author_removal(
    tx: &DatabaseTransaction,
    store: &SeaOrmSyncStore,
    collection: Collection,
    id: &str,
) -> Result<()> {
    let key = ObjectKey {
        collection,
        id: id.parse()?,
    };
    store.author_on(tx, key, None).await?;
    Ok(())
}
