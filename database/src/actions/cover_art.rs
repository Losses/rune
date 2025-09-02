use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use chrono::Utc;
use log::info;
use once_cell::sync::Lazy;
use sea_orm::{
    ActiveValue, ColumnTrait, Condition, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter,
};
use tokio_util::sync::CancellationToken;

use ::fsio::FsIo;
use ::metadata::cover_art::{CoverArt, extract_cover_art_binary, get_primary_color};
use uuid::Uuid;

use crate::{
    entities::{media_cover_art, media_files},
    parallel_media_files_processing,
};

use super::utils::DatabaseExecutor;

pub async fn get_magic_cover_art(
    main_db: &DatabaseConnection,
) -> Result<Option<media_cover_art::Model>, sea_orm::DbErr> {
    media_cover_art::Entity::find()
        .filter(media_cover_art::Column::FileHash.eq(String::new()))
        .one(main_db)
        .await
}

pub async fn get_magic_cover_art_id(main_db: &DatabaseConnection) -> Option<i32> {
    let magic_cover_art = get_magic_cover_art(main_db);

    magic_cover_art.await.ok().flatten().map(|s| s.id)
}

pub async fn ensure_magic_cover_art(
    main_db: &DatabaseConnection,
    node_id: &str,
) -> Result<media_cover_art::Model> {
    if let Some(magic_cover_art) = get_magic_cover_art(main_db).await? {
        Ok(magic_cover_art)
    } else {
        // If the magic value does not exist, create one and update the file's cover_art_id
        let new_magic_cover_art = media_cover_art::ActiveModel {
            id: ActiveValue::NotSet,
            file_hash: ActiveValue::Set(String::new()),
            binary: ActiveValue::Set(Vec::new()),
            primary_color: ActiveValue::Set(Some(0)),
            hlc_uuid: ActiveValue::Set("00000000-0000-0000-0000-000000000000".to_owned()),
            created_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
            updated_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
            created_at_hlc_ver: ActiveValue::Set(0),
            updated_at_hlc_ver: ActiveValue::Set(0),
            created_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
            updated_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
        };

        let insert_result = media_cover_art::Entity::insert(new_magic_cover_art)
            .exec(main_db)
            .await
            .with_context(|| "Failed to insert the magic cover art")?;

        let inserted_magic_cover_art =
            media_cover_art::Entity::find_by_id(insert_result.last_insert_id)
                .one(main_db)
                .await?
                .with_context(|| "Inserted magic cover art not found")?;

        Ok(inserted_magic_cover_art)
    }
}

pub async fn ensure_magic_cover_art_id(main_db: &DatabaseConnection, node_id: &str) -> Result<i32> {
    let magic_cover_art = ensure_magic_cover_art(main_db, node_id).await?;
    Ok(magic_cover_art.id)
}

pub static COVER_TEMP_DIR: Lazy<PathBuf> =
    Lazy::new(|| env::temp_dir().join("rune").join("cover_arts"));

fn bake_cover_art_by_cover_arts(
    fsio: &FsIo,
    cover_arts: Vec<media_cover_art::Model>,
) -> Result<HashMap<i32, String>> {
    let mut cover_art_id_to_path: HashMap<i32, String> = HashMap::new();

    fsio.create_dir_all(&COVER_TEMP_DIR)?;

    for cover_art in cover_arts.iter() {
        let id: i32 = cover_art.id;
        let hash: String = cover_art.file_hash.clone();

        if hash.is_empty() {
            continue;
        }

        let path: PathBuf = COVER_TEMP_DIR.clone().join(hash);

        if !path.exists() {
            fs::write(path.clone(), cover_art.binary.clone())?;
        }

        cover_art_id_to_path.insert(id, path.to_str().unwrap_or_default().to_string());
    }

    Ok(cover_art_id_to_path)
}

pub async fn bake_cover_art_by_cover_art_ids(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    cover_art_ids: Vec<i32>,
) -> Result<HashMap<i32, String>> {
    let cover_arts: Vec<media_cover_art::Model> = media_cover_art::Entity::find()
        .filter(media_cover_art::Column::Id.is_in(cover_art_ids))
        .all(main_db)
        .await?;

    bake_cover_art_by_cover_arts(fsio, cover_arts)
}

pub async fn bake_cover_art_by_media_files(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    files: Vec<media_files::Model>,
) -> Result<HashMap<i32, String>> {
    let cover_art_ids: Vec<i32> = files
        .clone()
        .into_iter()
        .map(|x| x.cover_art_id.unwrap_or(-1))
        .collect();

    let cover_art_id_to_path =
        bake_cover_art_by_cover_art_ids(fsio, main_db, cover_art_ids).await?;

    let mut file_id_to_path: HashMap<i32, String> = HashMap::new();

    for file in files.iter() {
        let cover_art_path = match file.cover_art_id {
            Some(x) => cover_art_id_to_path.get(&x),
            _none => None,
        };

        let default_path = "".to_string();
        let cover_art_path = cover_art_path.unwrap_or(&default_path);
        file_id_to_path.insert(file.id, cover_art_path.clone());
    }

    Ok(file_id_to_path)
}

pub async fn bake_cover_art_by_file_ids(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    file_ids: Vec<i32>,
) -> Result<HashMap<i32, String>> {
    let magic_cover_art_id = get_magic_cover_art_id(main_db).await;

    // Query file information
    let files: Vec<media_files::Model> = match magic_cover_art_id {
        Some(id) => {
            let mut condition = Condition::all();
            condition = condition.add(media_files::Column::Id.is_in(file_ids));
            condition = condition.add(media_files::Column::CoverArtId.ne(id));

            media_files::Entity::find()
                .filter(condition)
                .all(main_db)
                .await?
        }
        _none => {
            media_files::Entity::find()
                .filter(media_files::Column::Id.is_in(file_ids))
                .all(main_db)
                .await?
        }
    };

    bake_cover_art_by_media_files(fsio, main_db, files).await
}

pub fn extract_cover_art_by_file_id(
    fsio: &FsIo,
    lib_path: &Path,
    file: &media_files::Model,
) -> Option<CoverArt> {
    let file_path = Path::new(lib_path)
        .join(file.directory.clone())
        .join(file.file_name.clone());

    // If cover_art_id is empty, it means the file has not been checked before
    extract_cover_art_binary(fsio, Some(lib_path), &file_path)
}

pub async fn insert_extract_result(
    main_db: &DatabaseConnection,
    file: &media_files::Model,
    magic_cover_art_id: i32,
    result: Option<CoverArt>,
    node_id: &str,
) -> Result<()> {
    let file = file.clone();
    if let Some(cover_art) = result {
        // Check if there is a file with the same CRC in the database
        let existing_cover_art = media_cover_art::Entity::find()
            .filter(media_cover_art::Column::FileHash.eq(cover_art.crc.clone()))
            .one(main_db)
            .await?;

        if let Some(existing_cover_art) = existing_cover_art {
            // If there is a file with the same CRC, update the file's cover_art_id
            let mut file_active_model: media_files::ActiveModel = file.into();
            file_active_model.cover_art_id = ActiveValue::Set(Some(existing_cover_art.id));
            media_files::Entity::update(file_active_model)
                .exec(main_db)
                .await?;

            Ok(())
        } else {
            // If there is no file with the same CRC, store the cover art in the database and update the file's cover_art_id
            let new_cover_art = media_cover_art::ActiveModel {
                id: ActiveValue::NotSet,
                file_hash: ActiveValue::Set(cover_art.crc.clone()),
                binary: ActiveValue::Set(cover_art.data.clone()),
                primary_color: ActiveValue::Set(Some(cover_art.primary_color)),
                hlc_uuid: ActiveValue::Set(
                    Uuid::new_v5(&Uuid::NAMESPACE_OID, cover_art.crc.as_bytes()).to_string(),
                ),
                created_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
                updated_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
                created_at_hlc_ver: ActiveValue::Set(0),
                updated_at_hlc_ver: ActiveValue::Set(0),
                created_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
                updated_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
            };

            let insert_result = media_cover_art::Entity::insert(new_cover_art)
                .exec(main_db)
                .await?;
            let new_cover_art_id = insert_result.last_insert_id;

            let mut file_active_model: media_files::ActiveModel = file.into();
            file_active_model.cover_art_id = ActiveValue::Set(Some(new_cover_art_id));
            media_files::Entity::update(file_active_model)
                .exec(main_db)
                .await?;

            Ok(())
        }
    } else {
        // update the file's cover_art_id
        let mut file_active_model: media_files::ActiveModel = file.into();
        file_active_model.cover_art_id = ActiveValue::Set(Some(magic_cover_art_id));
        media_files::Entity::update(file_active_model)
            .exec(main_db)
            .await?;

        Ok(())
    }
}

pub async fn scan_cover_arts<F>(
    fsio: Arc<FsIo>,
    main_db: &DatabaseConnection,
    lib_path: &Path,
    node_id: &str,
    batch_size: usize,
    progress_callback: F,
    cancel_token: Option<CancellationToken>,
) -> Result<usize>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    info!("Starting cover art processing with batch size: {batch_size}");

    let progress_callback = Arc::new(progress_callback);

    let cursor_query = media_files::Entity::find();

    let magic_cover_art_id = ensure_magic_cover_art_id(main_db, node_id).await?;

    let lib_path = Arc::new(lib_path.to_path_buf());
    let node_id = Arc::new(node_id.to_owned());

    parallel_media_files_processing!(
        main_db,
        batch_size,
        progress_callback,
        cancel_token,
        cursor_query,
        lib_path,
        fsio,
        node_id,
        move |fsio, file, lib_path, _cancel_token| {
            extract_cover_art_by_file_id(fsio, lib_path, file)
        },
        |db, file: media_files::Model, node_id: Arc<String>, result| async move {
            match insert_extract_result(db, &file, magic_cover_art_id, result, &node_id).await {
                Ok(_) => {
                    debug!("Processed cover art for file ID: {}", file.id);
                }
                Err(e) => {
                    error!(
                        "Failed to process cover art for file ID: {}: {}",
                        file.id, e
                    );
                }
            }
        }
    )
}

pub async fn remove_cover_art_by_file_id<E>(main_db: &E, file_id: i32) -> Result<()>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    // Query file information
    let file: Option<media_files::Model> = media_files::Entity::find_by_id(file_id)
        .one(main_db)
        .await?;

    if let Some(file) = file
        && let Some(cover_art_id) = file.cover_art_id
    {
        // Update the file's cover_art_id to None
        let mut file_active_model: media_files::ActiveModel = file.into();
        file_active_model.cover_art_id = ActiveValue::Set(None);
        media_files::Entity::update(file_active_model)
            .exec(main_db)
            .await?;

        // Check if there are other files linked to the same cover_art_id
        let count = media_files::Entity::find()
            .filter(media_files::Column::CoverArtId.eq(cover_art_id))
            .count(main_db)
            .await?;

        if count == 0 {
            // If no other files are linked to the same cover_art_id, delete the corresponding entry in the media_cover_art table
            media_cover_art::Entity::delete_by_id(cover_art_id)
                .exec(main_db)
                .await?;
        }
    }

    Ok(())
}

pub async fn get_cover_art_id_by_track_id(
    main_db: &DatabaseConnection,
    file_id: i32,
) -> Result<Option<i32>> {
    let file: Option<media_files::Model> = media_files::Entity::find_by_id(file_id)
        .one(main_db)
        .await?;

    if let Some(file) = file {
        return Ok(file.cover_art_id);
    }

    Ok(None)
}

pub async fn get_cover_art_by_id(main_db: &DatabaseConnection, id: i32) -> Result<Option<Vec<u8>>> {
    let result = media_cover_art::Entity::find()
        .filter(media_cover_art::Column::Id.eq(id))
        .one(main_db)
        .await?;

    match result {
        Some(result) => Ok(Some(result.binary)),
        _none => Ok(None),
    }
}

pub async fn get_primary_color_by_cover_art_id(
    main_db: &DatabaseConnection,
    cover_art_id: i32,
) -> Result<i32> {
    // Step 1: Retrieve the cover art record from the database
    let cover_art = media_cover_art::Entity::find_by_id(cover_art_id)
        .one(main_db)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Cover art not found"))?;

    // Step 2: Check if the primary color is null
    if let Some(primary_color) = cover_art.primary_color {
        return Ok(primary_color);
    }

    // Step 3: Calculate the primary color
    let primary_color_int = get_primary_color(&cover_art.binary);

    match primary_color_int {
        Some(primary_color_int) => {
            // Step 4: Update the database with the new primary color
            let mut cover_art_active: media_cover_art::ActiveModel = cover_art.into();
            cover_art_active.primary_color = ActiveValue::Set(Some(primary_color_int));
            media_cover_art::Entity::update(cover_art_active)
                .exec(main_db)
                .await?;

            // Step 5: Return the primary color
            Ok(primary_color_int)
        }
        None => Err(anyhow::anyhow!("No primary color found")),
    }
}
