use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use log::{debug, error, info};
use once_cell::sync::Lazy;
use regex::Regex;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use sea_orm::{
    ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait,
    entity::prelude::*,
};
use tokio_util::sync::CancellationToken;

use ::fsio::{FsIo, FsNode};
use ::metadata::{
    describe::{FileDescription, describe_file},
    reader::get_metadata,
    scanner::AudioScanner,
};

use crate::actions::{
    collection::CollectionQueryType,
    cover_art::remove_cover_art_by_file_id,
    file::get_file_ids_by_descriptions,
    index::{index_media_files, perform_library_maintenance},
    logging::{LogLevel, insert_log},
    search::{add_term, remove_term},
};
use crate::entities::{
    albums, artists, media_file_albums, media_file_artists, media_files, media_metadata,
};

use super::cover_art::get_magic_cover_art_id;
use super::utils::DatabaseExecutor;

#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub path: PathBuf,
    pub metadata: Vec<(String, String)>,
}

pub fn read_metadata(fs_node: &FsNode) -> Result<FileMetadata> {
    match get_metadata(fs_node, None)
        .with_context(|| format!("Unable to read metadata: {:#?}", fs_node.path))
    {
        Ok(metadata) => Ok(FileMetadata {
            path: fs_node.path.clone(),
            metadata,
        }),
        Err(e) => Err(e),
    }
}

pub async fn sync_file_descriptions(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    node_id: &str,
    descriptions: &mut [Option<FileDescription>],
    force: bool,
) -> Result<()> {
    debug!("Starting to process multiple files");

    // Start a transaction
    let txn = main_db.begin().await?;
    let mut search_term: Option<(i32, String)> = None;

    let mut update_search_term = |file_id: i32, metadata: &FileMetadata| {
        if let Some((_, value)) = metadata
            .metadata
            .iter()
            .find(|(key, _)| key == "track_title")
        {
            search_term = Some((file_id, value.clone()));
        } else if let Some(file_name) = metadata.path.file_name() {
            search_term = Some((file_id, file_name.to_string_lossy().into_owned()))
        }
    };

    for description in descriptions.iter_mut() {
        match description {
            None => continue,
            Some(description) => {
                debug!("Processing file: {}", description.file_name.clone());

                let existing_file = match media_files::Entity::find()
                    .filter(media_files::Column::Directory.eq(description.directory.clone()))
                    .filter(media_files::Column::FileName.eq(description.file_name.clone()))
                    .one(&txn)
                    .await
                    .with_context(|| "Unable to query file")
                {
                    Ok(file) => file,
                    Err(e) => {
                        insert_log(
                            &txn,
                            LogLevel::Error,
                            "actions::metadata::sync_file_descriptions".to_string(),
                            format!("{e:#?}"),
                        )
                        .await?;
                        continue;
                    }
                };

                if let Some(existing_file) = existing_file {
                    debug!(
                        "File exists in the database: {}",
                        description.file_name.clone()
                    );

                    // File exists in the database
                    if existing_file.last_modified == description.last_modified && !force {
                        // If the file's last modified date hasn't changed, skip it
                        debug!(
                            "File's last modified date hasn't changed ({}), skipping: {}",
                            existing_file.last_modified,
                            description.file_name.clone()
                        );
                        continue;
                    } else {
                        // If the file's last modified date has changed, check the hash
                        debug!(
                            "File's last modified date has changed ({} -> {}), checking hash: {}",
                            existing_file.last_modified,
                            description.last_modified,
                            description.file_name.clone()
                        );

                        let new_hash = match description.get_crc(fsio).with_context(|| {
                            format!("Failed to get CRC: {}", description.file_name)
                        }) {
                            Ok(hash) => hash,
                            Err(e) => {
                                error!("{e:?}");
                                insert_log(
                                    &txn,
                                    LogLevel::Error,
                                    "actions::metadata::sync_file_descriptions".to_string(),
                                    format!("{e:#?}"),
                                )
                                .await?;
                                continue;
                            }
                        };

                        if existing_file.file_hash == new_hash && !force {
                            // If the hash is the same, update the last modified date
                            debug!(
                                "File hash is the same, updating last modified date: {}",
                                description.file_name.clone()
                            );

                            if let Err(e) = update_last_modified(&txn, &existing_file, description)
                                .await
                                .with_context(|| {
                                    format!(
                                        "Failed to update last modified: {}",
                                        description.file_name.clone(),
                                    )
                                })
                            {
                                error!("{e:?}");
                                insert_log(
                                    &txn,
                                    LogLevel::Error,
                                    "actions::metadata::sync_file_descriptions".to_string(),
                                    format!("{e:#?}"),
                                )
                                .await?;
                                continue;
                            }

                            if let Err(e) = unlink_cover_art(&txn, &existing_file)
                                .await
                                .with_context(|| {
                                    format!(
                                        "Failed to unlink cover art modified: {}",
                                        description.file_name.clone(),
                                    )
                                })
                            {
                                error!("{e:?}");
                                insert_log(
                                    &txn,
                                    LogLevel::Error,
                                    "actions::metadata::sync_file_descriptions".to_string(),
                                    format!("{e:#?}"),
                                )
                                .await?;
                                continue;
                            }
                        } else {
                            if force {
                                info!("Force scanning triggered: {}", existing_file.id);
                            }

                            // If the hash is different, update the metadata
                            debug!(
                                "File hash is different, updating metadata: {}",
                                description.file_name.clone()
                            );

                            if let Err(e) = update_file_codec_information(
                                fsio,
                                &txn,
                                &existing_file,
                                description,
                            )
                            .await
                            .with_context(|| {
                                format!(
                                    "Failed to update file codec information: {}",
                                    description.file_name.clone(),
                                )
                            }) {
                                error!("{e:?}");
                                insert_log(
                                    &txn,
                                    LogLevel::Error,
                                    "actions::metadata::sync_file_descriptions".to_string(),
                                    format!("{e:#?}"),
                                )
                                .await?;
                                continue;
                            }

                            let file_metadata =
                                read_metadata(&description.raw_node).with_context(|| {
                                    format!(
                                        "Unable to parse file metadata: {:?}",
                                        description.rel_path
                                    )
                                });

                            match file_metadata {
                                Ok(x) => {
                                    if let Err(e) = update_file_metadata(
                                        fsio,
                                        &txn,
                                        node_id,
                                        &existing_file,
                                        description,
                                        &x,
                                    )
                                    .await
                                    .with_context(|| {
                                        format!(
                                            "Failed to update file metadata: {}",
                                            description.file_name.clone(),
                                        )
                                    }) {
                                        error!("{e:?}");
                                        insert_log(
                                            &txn,
                                            LogLevel::Error,
                                            "actions::metadata::sync_file_descriptions".to_string(),
                                            format!("{e:#?}"),
                                        )
                                        .await?;
                                        continue;
                                    }

                                    update_search_term(existing_file.id, &x);
                                }
                                Err(e) => {
                                    error!("{e:?}");
                                    insert_log(
                                        &txn,
                                        LogLevel::Error,
                                        "actions::metadata::sync_file_descriptions".to_string(),
                                        format!("{e:#?}"),
                                    )
                                    .await?;
                                }
                            }
                        }
                    }
                } else {
                    // If the file is new, insert a new recordF
                    debug!(
                        "File is new, inserting new record: {}",
                        description.file_name.clone()
                    );

                    let file_metadata = read_metadata(&description.raw_node).with_context(|| {
                        format!(
                            "Unable to parse metadata: {}",
                            description.rel_path.clone().display()
                        )
                    });

                    match file_metadata {
                        Ok(x) => {
                            match insert_new_file(fsio, &txn, node_id, &x, description)
                                .await
                                .with_context(|| {
                                    format!(
                                        "Failed to insert new file: {}",
                                        description.file_name.clone()
                                    )
                                }) {
                                Ok(_) => {
                                    if let Some(existing_file) = existing_file {
                                        update_search_term(existing_file.id, &x);
                                    }
                                }
                                Err(e) => {
                                    error!("{e:#?}");
                                    insert_log(
                                        &txn,
                                        LogLevel::Error,
                                        "actions::metadata::sync_file_descriptions".to_string(),
                                        format!("{e:#?}"),
                                    )
                                    .await?;
                                }
                            }
                        }
                        Err(e) => {
                            error!("{e:?}");
                            insert_log(
                                &txn,
                                LogLevel::Error,
                                "actions::metadata::sync_file_descriptions".to_string(),
                                format!("{e:#?}"),
                            )
                            .await?;
                        }
                    }
                }
            }
        };
    }

    // Commit the transaction
    if let Err(e) = txn
        .commit()
        .await
        .with_context(|| "Failed to commit transaction")
    {
        error!("{e:?}");
        insert_log(
            main_db,
            LogLevel::Error,
            "actions::metadata::sync_file_descriptions".to_string(),
            format!("{e:?}"),
        )
        .await?;
    }

    if let Some((id, name)) = search_term
        && let Err(e) = add_term(main_db, CollectionQueryType::Track, id, &name)
            .await
            .with_context(|| "Failed to add term")
    {
        error!("{e:?}");
        insert_log(
            main_db,
            LogLevel::Error,
            "actions::metadata::sync_file_descriptions".to_string(),
            format!("{e:?}"),
        )
        .await?;
    }

    debug!("Finished syncing file data");

    Ok(())
}

pub async fn process_files(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    node_id: &str,
    descriptions: &mut [Option<FileDescription>],
) -> Result<()> {
    debug!("Starting to process multiple files");

    // Start a transaction
    let txn = main_db.begin().await?;

    for description in descriptions.iter_mut() {
        match description {
            None => continue,
            Some(description) => {
                debug!("Processing file: {}", description.file_name.clone());

                // Check if the file already exists in the database
                let existing_file = media_files::Entity::find()
                    .filter(media_files::Column::Directory.eq(description.directory.clone()))
                    .filter(media_files::Column::FileName.eq(description.file_name.clone()))
                    .one(&txn)
                    .await?;

                if let Some(existing_file) = existing_file {
                    debug!(
                        "File exists in the database: {}",
                        description.file_name.clone()
                    );

                    // File exists in the database
                    if existing_file.last_modified == description.last_modified {
                        // If the file's last modified date hasn't changed, skip it
                        debug!(
                            "File's last modified date hasn't changed, skipping: {}",
                            description.file_name.clone()
                        );
                        continue;
                    } else {
                        // If the file's last modified date has changed, check the hash
                        debug!(
                            "File's last modified date has changed, checking hash: {}",
                            description.file_name.clone()
                        );
                        let new_hash = description.get_crc(fsio)?;
                        if existing_file.file_hash == new_hash {
                            // If the hash is the same, update the last modified date
                            debug!(
                                "File hash is the same, updating last modified date: {}",
                                description.file_name.clone()
                            );
                            update_last_modified(&txn, &existing_file, description)
                                .await
                                .with_context(|| {
                                    format!(
                                        "Failed to update last modified: {}",
                                        description.file_name.clone(),
                                    )
                                })?;
                            unlink_cover_art(&txn, &existing_file)
                                .await
                                .with_context(|| {
                                    format!(
                                        "Failed to unlink cover art modified: {}",
                                        description.file_name.clone(),
                                    )
                                })?;
                        } else {
                            // If the hash is different, update the metadata
                            debug!(
                                "File hash is different, updating metadata: {}",
                                description.file_name.clone()
                            );

                            update_file_codec_information(fsio, &txn, &existing_file, description)
                                .await?;

                            remove_cover_art_by_file_id(&txn, existing_file.id).await?;

                            let file_metadata =
                                read_metadata(&description.raw_node).with_context(|| {
                                    format!(
                                        "Unable to parse file metadata: {:?}",
                                        description.rel_path
                                    )
                                });

                            match file_metadata {
                                Ok(x) => {
                                    update_file_metadata(
                                        fsio,
                                        &txn,
                                        node_id,
                                        &existing_file,
                                        description,
                                        &x,
                                    )
                                    .await?;
                                }
                                Err(e) => {
                                    error!("{:?}", description.rel_path);
                                    insert_log(
                                        &txn,
                                        LogLevel::Error,
                                        "actions::metadata::sync_file_descriptions".to_string(),
                                        format!("{e:#?}"),
                                    )
                                    .await?;
                                }
                            }
                        }
                    }
                } else {
                    // If the file is new, insert a new record
                    debug!(
                        "File is new, inserting new record: {}",
                        description.file_name.clone()
                    );

                    let file_metadata = read_metadata(&description.raw_node).with_context(|| {
                        format!("Unable to parse file metadata: {:?}", description.rel_path)
                    });

                    match file_metadata {
                        Ok(x) => {
                            match insert_new_file(fsio, &txn, node_id, &x, description)
                                .await
                                .with_context(|| {
                                    format!(
                                        "Failed to insert new file, metadata: {}",
                                        description.file_name.clone(),
                                    )
                                }) {
                                Ok(_) => {}
                                Err(e) => error!("{e:?}"),
                            };
                        }
                        Err(e) => {
                            error!("{e:?}");
                            insert_log(
                                &txn,
                                LogLevel::Error,
                                "actions::metadata::sync_file_descriptions".to_string(),
                                format!("{e:#?}"),
                            )
                            .await?;
                        }
                    }
                }
            }
        };
    }

    // Commit the transaction
    txn.commit().await?;

    info!("Finished processing multiple files");

    Ok(())
}

pub async fn unlink_cover_art<E>(db: &E, existing_file: &media_files::Model) -> Result<()>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let mut active_model: media_files::ActiveModel = existing_file.clone().into();
    active_model.cover_art_id = ActiveValue::Set(None);
    active_model.update(db).await?;
    Ok(())
}

pub async fn update_last_modified<E>(
    db: &E,
    existing_file: &media_files::Model,
    description: &FileDescription,
) -> Result<()>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let mut active_model: media_files::ActiveModel = existing_file.clone().into();
    active_model.last_modified = ActiveValue::Set(description.last_modified.clone());
    active_model.update(db).await?;
    Ok(())
}

pub async fn update_file_metadata<E>(
    fsio: &FsIo,
    db: &E,
    node_id: &str,
    existing_file: &media_files::Model,
    description: &mut FileDescription,
    metadata: &FileMetadata,
) -> Result<()>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let mut active_model: media_files::ActiveModel = existing_file.clone().into();

    // Update last modified and file hash
    active_model.last_modified = ActiveValue::Set(description.last_modified.clone());

    match description
        .get_crc(fsio)
        .with_context(|| "Failed to get CRC")
    {
        Ok(crc) => active_model.file_hash = ActiveValue::Set(crc),
        Err(e) => {
            error!("{e:?}");
            insert_log(
                db,
                LogLevel::Error,
                "actions::metadata::update_file_metadata".to_string(),
                format!("{e:#?}"),
            )
            .await?;
            return Err(e);
        }
    }

    if let Err(e) = active_model
        .update(db)
        .await
        .with_context(|| "Failed to update active model")
    {
        error!("{e:?}");
        insert_log(
            db,
            LogLevel::Error,
            "actions::metadata::update_file_metadata".to_string(),
            format!("{e:?}"),
        )
        .await?;
        return Err(e);
    }

    // Delete existing metadata
    if let Err(e) = media_metadata::Entity::delete_many()
        .filter(media_metadata::Column::FileId.eq(existing_file.id))
        .exec(db)
        .await
        .with_context(|| "Failed to delete existing metadata")
    {
        error!("{e:#?}");
        insert_log(
            db,
            LogLevel::Error,
            "actions::metadata::update_file_metadata".to_string(),
            format!("Failed to delete existing metadata: {e:?}"),
        )
        .await?;
        return Err(e);
    }

    // Insert new metadata
    let new_metadata: Vec<media_metadata::ActiveModel> = metadata
        .metadata
        .clone()
        .into_iter()
        .map(|(key, value)| media_metadata::ActiveModel {
            file_id: ActiveValue::Set(existing_file.id),
            meta_key: ActiveValue::Set(key.clone()),
            meta_value: ActiveValue::Set(value),
            hlc_uuid: ActiveValue::Set(
                Uuid::new_v5(
                    &Uuid::NAMESPACE_OID,
                    format!("RUNE_METADATA::{}::{}", existing_file.file_hash, key).as_bytes(),
                )
                .to_string(),
            ),
            created_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
            updated_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
            created_at_hlc_ver: ActiveValue::Set(0),
            updated_at_hlc_ver: ActiveValue::Set(0),
            created_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
            updated_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
            ..Default::default()
        })
        .collect();

    if !new_metadata.is_empty()
        && let Err(e) = media_metadata::Entity::insert_many(new_metadata.clone())
            .exec(db)
            .await
            .with_context(|| "Failed to insert new metadata while executing updating")
    {
        error!("{e:?}");
        insert_log(
            db,
            LogLevel::Error,
            "actions::metadata::update_file_metadata".to_string(),
            format!("{e:#?}"),
        )
        .await?;
        return Err(e);
    }

    Ok(())
}

pub async fn update_file_codec_information<E>(
    fsio: &FsIo,
    db: &E,
    existing_file: &media_files::Model,
    description: &mut FileDescription,
) -> Result<()>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let (sample_rate, duration_in_seconds) = match description
        .get_codec_information(fsio)
        .with_context(|| "Failed to get codec information")
    {
        Ok(info) => info,
        Err(e) => {
            error!("{e:?}");
            insert_log(
                db,
                LogLevel::Error,
                "actions::metadata::update_file_codec_information".to_string(),
                format!("{e:#?}"),
            )
            .await?;
            return Err(e);
        }
    };

    let mut active_model: media_files::ActiveModel = existing_file.clone().into();

    match sample_rate
        .try_into()
        .with_context(|| "Failed to convert sample rate")
    {
        Ok(rate) => active_model.sample_rate = ActiveValue::Set(rate),
        Err(e) => {
            error!("{e:#?}");
            insert_log(
                db,
                LogLevel::Error,
                "actions::metadata::update_file_codec_information".to_string(),
                format!("{e:#?}"),
            )
            .await?;
            return Err(e);
        }
    }

    active_model.duration = ActiveValue::Set(
        Decimal::from_f64(duration_in_seconds).expect("Unable to convert track duration"),
    );

    if let Err(e) = active_model
        .update(db)
        .await
        .with_context(|| "Failed to update codec information")
    {
        error!("{e:?}");
        insert_log(
            db,
            LogLevel::Error,
            "actions::metadata::update_file_codec_information".to_string(),
            format!("{e:#?}"),
        )
        .await?;
        return Err(e);
    }

    Ok(())
}

pub async fn insert_new_file<E>(
    fsio: &FsIo,
    main_db: &E,
    node_id: &str,
    metadata: &FileMetadata,
    description: &mut FileDescription,
) -> Result<()>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let (sample_rate, duration_in_seconds) = description.get_codec_information(fsio)?;

    description
        .get_crc(fsio)
        .with_context(|| format!("Failed to get CRC: {}", description.file_name))?;
    let new_hash = if let Ok(hash) = description.get_crc(fsio) {
        hash.clone()
    } else {
        bail!("");
    };

    let new_file = media_files::ActiveModel {
        file_name: ActiveValue::Set(description.file_name.to_string()),
        directory: ActiveValue::Set(description.directory.clone()),
        extension: ActiveValue::Set(description.extension.clone()),
        file_hash: ActiveValue::Set(new_hash.clone()),
        sample_rate: ActiveValue::Set(sample_rate.try_into()?),
        duration: ActiveValue::Set(
            Decimal::from_f64(duration_in_seconds).expect("Unable to convert track duration"),
        ),
        last_modified: ActiveValue::Set(description.last_modified.clone()),
        hlc_uuid: ActiveValue::Set(
            Uuid::new_v5(&Uuid::NAMESPACE_OID, new_hash.as_bytes()).to_string(),
        ),
        created_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
        updated_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
        created_at_hlc_ver: ActiveValue::Set(0),
        updated_at_hlc_ver: ActiveValue::Set(0),
        created_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
        updated_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
        ..Default::default()
    };
    let inserted_file = media_files::Entity::insert(new_file).exec(main_db).await?;

    if let Some((_, value)) = metadata
        .metadata
        .iter()
        .find(|(key, _)| key == "track_title")
    {
        add_term(
            main_db,
            CollectionQueryType::Track,
            inserted_file.last_insert_id,
            value,
        )
        .await?;
    } else if let Some(file_name) = metadata.path.file_name() {
        add_term(
            main_db,
            CollectionQueryType::Track,
            inserted_file.last_insert_id,
            &file_name.to_string_lossy(),
        )
        .await?;
    }

    let file_id = inserted_file.last_insert_id;

    // Insert metadata
    let new_metadata: Vec<media_metadata::ActiveModel> = metadata
        .metadata
        .clone()
        .into_iter()
        .map(|(key, value)| media_metadata::ActiveModel {
            file_id: ActiveValue::Set(file_id),
            meta_key: ActiveValue::Set(key),
            meta_value: ActiveValue::Set(value),
            ..Default::default()
        })
        .collect();

    if !new_metadata.is_empty() {
        media_metadata::Entity::insert_many(new_metadata)
            .exec(main_db)
            .await
            .with_context(|| format!("Failed to insert new metadata: {}", description.file_name))?;
    }

    Ok(())
}

async fn clean_up_database(main_db: &DatabaseConnection, root_path: &Path) -> Result<()> {
    let db_files = media_files::Entity::find().all(main_db).await?;

    for db_file in db_files {
        let full_path = root_path
            .join(PathBuf::from(&db_file.directory))
            .join(PathBuf::from(&db_file.file_name));
        if !full_path.exists() {
            info!("Cleaning {}", full_path.to_str().unwrap_or_default());
            // Delete the file record
            media_files::Entity::delete_by_id(db_file.id)
                .exec(main_db)
                .await?;

            remove_term(main_db, CollectionQueryType::Track, db_file.id).await?
        }
    }

    Ok(())
}

pub fn empty_progress_callback(_processed: usize) {}

#[allow(clippy::too_many_arguments)]
pub async fn scan_audio_library<F>(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    node_id: &str,
    lib_path: &Path,
    cleanup: bool,
    force: bool,
    progress_callback: F,
    cancel_token: Option<CancellationToken>,
) -> Result<usize>
where
    F: Fn(usize) + Send + Sync,
{
    let root_path_str = lib_path.to_str().expect("Invalid UTF-8 sequence in path");
    let mut scanner = AudioScanner::new(fsio, &root_path_str)?;

    info!("Starting audio library scan");

    // Get the total number of files to scan (assuming AudioScanner has this method)
    let mut processed_files = 0;

    // Read audio files at a time until no more files are available.
    while !scanner.has_ended() {
        // Check if the cancellation token has been triggered
        if let Some(ref token) = cancel_token
            && token.is_cancelled()
        {
            info!("Scan cancelled.");
            return Ok(processed_files);
        }

        debug!("Reading metadata for the next 12 files");
        let files = scanner.read_files(12);
        let mut descriptions: Vec<Option<FileDescription>> = files
            .clone()
            .into_iter()
            .map(|file| describe_file(&file, &Some(lib_path.to_path_buf())))
            .map(|result| result.ok())
            .collect();

        match sync_file_descriptions(fsio, main_db, node_id, &mut descriptions, force)
            .await
            .with_context(|| "Unable to describe files")
        {
            Ok(_) => {
                debug!("Finished one batch");
            }
            Err(e) => {
                error!("{e:#?}");
            }
        };

        let file_ids = get_file_ids_by_descriptions(main_db, &descriptions).await?;

        match index_media_files(main_db, node_id, file_ids, cancel_token.as_ref())
            .await
            .with_context(|| "Unable to index files")
        {
            Ok(_) => {}
            Err(e) => error!("{e:#?}"),
        };

        // Update the number of processed files
        processed_files += files.len();

        // Call the progress callback if it is provided
        progress_callback(processed_files);
    }

    if cleanup {
        info!("Starting cleanup process.");
        match clean_up_database(main_db, lib_path)
            .await
            .with_context(|| "Unable to cleanup database")
        {
            Ok(_) => info!("Cleanup completed successfully."),
            Err(e) => error!("{e:#?}"),
        }
        // Perform library maintenance after indexing is completed.
        match perform_library_maintenance(main_db, cancel_token.as_ref()).await {
            Ok(_) => info!("Library maintainence successfully."),
            Err(e) => error!("{e:#?}"),
        };
    }

    info!("Audio library scan completed.");

    Ok(processed_files)
}

static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\d+").unwrap());

pub fn extract_number(s: &str) -> Option<i32> {
    RE.find(s).and_then(|mat| mat.as_str().parse::<i32>().ok())
}

#[derive(Debug, Clone, Default)]
pub struct MetadataSummary {
    pub id: i32,
    pub directory: String,
    pub file_name: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub title: String,
    pub track_number: i32,
    pub duration: f64,
    pub cover_art_id: Option<i32>,
    pub file_hash: String,
}

pub async fn get_metadata_summary_by_files(
    db: &DatabaseConnection,
    files: Vec<media_files::Model>,
) -> Result<Vec<MetadataSummary>> {
    // Extract file IDs from the provided file entries
    let file_ids: Vec<i32> = files.iter().map(|file| file.id).collect();
    let magic_cover_art_id = get_magic_cover_art_id(db).await;

    // Fetch all metadata entries for the given file IDs
    let metadata_entries: Vec<media_metadata::Model> = media_metadata::Entity::find()
        .filter(media_metadata::Column::FileId.is_in(file_ids.clone()).and(
            media_metadata::Column::MetaKey.is_in([
                "artist",
                "album",
                "genre",
                "track_title",
                "disc_number",
                "track_number",
            ]),
        ))
        .all(db)
        .await?;

    // Create a map for metadata entries
    let mut metadata_map: HashMap<i32, HashMap<String, String>> = HashMap::new();
    for entry in metadata_entries {
        metadata_map
            .entry(entry.file_id)
            .or_default()
            .insert(entry.meta_key, entry.meta_value);
    }

    // Prepare the final result
    let mut results: Vec<MetadataSummary> = Vec::new();
    for file in files {
        let file_id = file.id;
        let _metadata: HashMap<String, String> = HashMap::new();
        let metadata = metadata_map.get(&file_id).unwrap_or(&_metadata);
        let duration = file.duration;

        let cover_art_id = file.cover_art_id;

        let parsed_disk_number = metadata
            .get("disc_number")
            .and_then(|s| extract_number(s))
            .unwrap_or(0);

        let parsed_track_number = metadata
            .get("track_number")
            .and_then(|s| extract_number(s))
            .unwrap_or(0);

        let track_number = parsed_disk_number * 1000 + parsed_track_number;

        let summary = MetadataSummary {
            id: file_id,
            directory: file.directory.clone(),
            file_name: file.file_name.clone(),
            artist: metadata.get("artist").cloned().unwrap_or_default(),
            album: metadata.get("album").cloned().unwrap_or_default(),
            genre: metadata
                .get("genre")
                .cloned()
                .unwrap_or_default()
                .to_uppercase(),
            title: metadata
                .get("track_title")
                .cloned()
                .unwrap_or(file.file_name.clone()),
            track_number,
            duration: duration.to_f64().expect("Unable to convert track duration"),
            cover_art_id: if cover_art_id == magic_cover_art_id {
                None
            } else {
                cover_art_id
            },
            file_hash: file.file_hash.clone(),
        };

        results.push(summary);
    }

    Ok(results)
}

pub async fn get_metadata_summary_by_file_ids(
    db: &DatabaseConnection,
    file_ids: Vec<i32>,
) -> Result<Vec<MetadataSummary>> {
    // Fetch all file entries for the given file IDs
    let mut file_entries: Vec<media_files::Model> = media_files::Entity::find()
        .filter(media_files::Column::Id.is_in(file_ids.clone()))
        .all(db)
        .await?;

    // Sort file_entries based on the order in file_ids
    file_entries.sort_by_key(|entry| {
        file_ids
            .iter()
            .position(|&id| id == entry.id)
            .unwrap_or(usize::MAX)
    });

    // Use the get_metadata_summary_by_files function to get the metadata summaries
    get_metadata_summary_by_files(db, file_entries).await
}

pub async fn get_metadata_summary_by_file_id(
    db: &DatabaseConnection,
    file_id: i32,
) -> anyhow::Result<MetadataSummary> {
    let results = get_metadata_summary_by_file_ids(db, vec![file_id])
        .await
        .context("Database query failed")?;

    results
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Metadata summary not found for file ID: {}", file_id))
}

pub async fn get_parsed_file_by_id(
    db: &DatabaseConnection,
    file_id: i32,
) -> Result<(MetadataSummary, Vec<artists::Model>, Option<albums::Model>)> {
    let file = get_metadata_summary_by_file_id(db, file_id).await?;

    let artist_ids = media_file_artists::Entity::find()
        .filter(media_file_artists::Column::MediaFileId.eq(file_id))
        .all(db)
        .await?;

    let artists = artists::Entity::find()
        .filter(artists::Column::Id.is_in(artist_ids.into_iter().map(|x| x.artist_id)))
        .all(db)
        .await?;

    let album_id = media_file_albums::Entity::find()
        .filter(media_file_albums::Column::MediaFileId.eq(file_id))
        .one(db)
        .await?;

    let album = match album_id {
        Some(album_id) => {
            albums::Entity::find()
                .filter(albums::Column::Id.eq(album_id.album_id))
                .one(db)
                .await?
        }
        _none => None,
    };

    Ok((file, artists, album))
}
