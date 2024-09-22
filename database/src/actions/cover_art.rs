use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use dunce::canonicalize;
use log::info;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, FromQueryResult,
    Order, PaginatorTrait, QueryFilter, QueryTrait,
};

use migration::{Func, SimpleExpr};

use metadata::cover_art::{extract_cover_art_binary, CoverArt};
use tokio_util::sync::CancellationToken;

use crate::{
    entities::{media_cover_art, media_files},
    parallel_media_files_processing,
};

use super::utils::DatabaseExecutor;

pub async fn get_magic_cover_art(
    main_db: &DatabaseConnection,
) -> std::result::Result<std::option::Option<media_cover_art::Model>, sea_orm::DbErr> {
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
) -> Result<media_cover_art::Model> {
    if let Some(magic_cover_art) = get_magic_cover_art(main_db).await? {
        Ok(magic_cover_art)
    } else {
        // If the magic value does not exist, create one and update the file's cover_art_id
        let new_magic_cover_art = media_cover_art::ActiveModel {
            id: ActiveValue::NotSet,
            file_hash: ActiveValue::Set(String::new()),
            binary: ActiveValue::Set(Vec::new()),
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

pub async fn ensure_magic_cover_art_id(main_db: &DatabaseConnection) -> Result<i32> {
    let magic_cover_art = ensure_magic_cover_art(main_db).await?;
    Ok(magic_cover_art.id)
}

pub async fn get_cover_art_by_file_id(
    main_db: &DatabaseConnection,
    file_id: i32,
) -> Result<Option<(i32, Vec<u8>)>> {
    // Query file information
    let file: Option<media_files::Model> = media_files::Entity::find_by_id(file_id)
        .one(main_db)
        .await?;

    if let Some(file) = file {
        if let Some(cover_art_id) = file.cover_art_id {
            // If cover_art_id already exists, directly retrieve the cover art from the database
            let cover_art = media_cover_art::Entity::find_by_id(cover_art_id)
                .one(main_db)
                .await?
                .unwrap();
            Ok(Some((cover_art.id, cover_art.binary)))
        } else {
            Ok(None)
        }
    } else {
        Ok(None)
    }
}

pub fn extract_cover_art_by_file_id(
    file: &media_files::Model,
    lib_path: &Path,
) -> Option<CoverArt> {
    let file_path = canonicalize(
        Path::new(lib_path)
            .join(file.directory.clone())
            .join(file.file_name.clone()),
    )
    .unwrap();

    // If cover_art_id is empty, it means the file has not been checked before
    extract_cover_art_binary(&file_path)
}

pub async fn insert_extract_result(
    main_db: &DatabaseConnection,
    file: &media_files::Model,
    magic_cover_art_id: i32,
    result: Option<CoverArt>,
) -> Result<Option<(i32, Vec<u8>)>> {
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

            Ok(Some((existing_cover_art.id, existing_cover_art.binary)))
        } else {
            // If there is no file with the same CRC, store the cover art in the database and update the file's cover_art_id
            let new_cover_art = media_cover_art::ActiveModel {
                id: ActiveValue::NotSet,
                file_hash: ActiveValue::Set(cover_art.crc.clone()),
                binary: ActiveValue::Set(cover_art.data.clone()),
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

            Ok(Some((new_cover_art_id, cover_art.data)))
        }
    } else {
        // update the file's cover_art_id
        let mut file_active_model: media_files::ActiveModel = file.into();
        file_active_model.cover_art_id = ActiveValue::Set(Some(magic_cover_art_id));
        media_files::Entity::update(file_active_model)
            .exec(main_db)
            .await?;

        Ok(Some((magic_cover_art_id, Vec::<u8>::new())))
    }
}

pub async fn scan_cover_arts<F>(
    main_db: &DatabaseConnection,
    lib_path: &Path,
    batch_size: usize,
    progress_callback: F,
    cancel_token: Option<CancellationToken>,
) -> Result<usize>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    info!(
        "Starting cover art processing with batch size: {}",
        batch_size
    );

    let cursor_query = media_files::Entity::find();

    let lib_path = Arc::new(lib_path.to_path_buf());
    let magic_cover_art_id = ensure_magic_cover_art_id(main_db).await?;

    parallel_media_files_processing!(
        main_db,
        batch_size,
        progress_callback,
        cancel_token,
        cursor_query,
        lib_path,
        move |file, lib_path| { extract_cover_art_by_file_id(file, lib_path) },
        |db, file: media_files::Model, result| async move {
            match insert_extract_result(db, &file, magic_cover_art_id, result).await {
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

    if let Some(file) = file {
        if let Some(cover_art_id) = file.cover_art_id {
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
    }

    Ok(())
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

pub async fn get_random_cover_art_ids(
    main_db: &DatabaseConnection,
    n: usize,
) -> Result<Vec<media_cover_art::Model>> {
    let mut query: sea_orm::sea_query::SelectStatement = media_cover_art::Entity::find()
        .filter(media_cover_art::Column::FileHash.ne(String::new()))
        .as_query()
        .to_owned();

    let select = query
        .order_by_expr(SimpleExpr::FunctionCall(Func::random()), Order::Asc)
        .limit(n as u64);
    let statement = main_db.get_database_backend().build(select);

    let files = media_cover_art::Model::find_by_statement(statement)
        .all(main_db)
        .await?;

    Ok(files)
}
