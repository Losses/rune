use log::{debug, error, info};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sea_orm::entity::prelude::*;
use sea_orm::{ActiveValue, ColumnTrait, EntityTrait, QueryFilter};
use sea_orm::{DatabaseConnection, TransactionTrait};
use tokio_util::sync::CancellationToken;

use metadata::describe::{describe_file, FileDescription};
use metadata::reader::get_metadata;
use metadata::scanner::AudioScanner;

use crate::actions::cover_art::remove_cover_art_by_file_id;
use crate::actions::file::get_file_ids_by_descriptions;
use crate::actions::index::index_media_files;
use crate::actions::search::{add_term, remove_term, CollectionType};
use crate::connection::SearchDbConnection;
use crate::entities::{albums, artists, media_file_albums, media_files};
use crate::entities::{media_file_artists, media_metadata};

use super::cover_art::get_magic_cover_art_id;
use super::utils::DatabaseExecutor;

#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub path: PathBuf,
    pub metadata: Vec<(String, String)>,
}

pub fn read_metadata(description: &FileDescription) -> Option<FileMetadata> {
    let full_path = match description.full_path.to_str() {
        Some(x) => x,
        _none => return None,
    };

    match get_metadata(full_path, None) {
        Ok(metadata) => Some(FileMetadata {
            path: description.rel_path.clone(),
            metadata,
        }),
        Err(err) => {
            error!(
                "Error reading metadata for {}: {}",
                description.rel_path.display(),
                err
            );
            // Continue to the next file instead of returning an empty list
            None
        }
    }
}

pub async fn sync_file_descriptions(
    main_db: &DatabaseConnection,
    search_db: &mut SearchDbConnection,
    descriptions: &mut [Option<FileDescription>],
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
        }
    };

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

                        let new_hash = description.get_crc().with_context(|| {
                            format!("Failed to get CRC: {}", description.file_name)
                        })?;

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

                            update_file_codec_information(&txn, &existing_file, description)
                                .await
                                .with_context(|| {
                                    format!(
                                        "Failed to update file codec information: {}",
                                        description.file_name.clone(),
                                    )
                                })?;

                            let file_metadata = read_metadata(description);

                            match file_metadata {
                                Some(x) => {
                                    update_file_metadata(&txn, &existing_file, description, &x)
                                        .await
                                        .with_context(|| {
                                            format!(
                                                "Failed to update file metadata: {}",
                                                description.file_name.clone(),
                                            )
                                        })?;

                                    update_search_term(existing_file.id, &x);
                                }
                                _none => {
                                    error!(
                                        "Unable to get metadata of the file: {:?}",
                                        description.rel_path,
                                    );
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

                    let file_metadata = read_metadata(description);

                    if let Some(ref x) = file_metadata {
                        match insert_new_file(&txn, search_db, x, description).await {
                            Ok(_) => {
                                if let Some(existing_file) = existing_file {
                                    update_search_term(existing_file.id, x);
                                }
                            }
                            Err(_) => error!(
                                "Failed to insert new file: {}",
                                description.file_name.clone(),
                            ),
                        }
                    } else {
                        error!(
                            "Unable to get metadata of the file: {:?}",
                            description.rel_path,
                        );
                    }
                }
            }
        };
    }

    // Commit the transaction
    txn.commit().await?;

    if let Some((id, name)) = search_term {
        add_term(search_db, CollectionType::Track, id, &name);

        search_db.w.commit()?;
    }

    debug!("Finished syncing file data");

    Ok(())
}

pub async fn process_files(
    main_db: &DatabaseConnection,
    search_db: &mut SearchDbConnection,
    descriptions: &mut [Option<FileDescription>],
) -> Result<()> {
    debug!("Starting to process multiple files");

    // Start a transaction
    let txn = main_db.begin().await?;

    let mut modified = false;

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
                        let new_hash = description.get_crc()?;
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

                            update_file_codec_information(&txn, &existing_file, description)
                                .await?;

                            remove_cover_art_by_file_id(&txn, existing_file.id).await?;

                            let file_metadata = read_metadata(description);

                            match file_metadata {
                                Some(x) => {
                                    update_file_metadata(&txn, &existing_file, description, &x)
                                        .await?;
                                }
                                _none => {
                                    error!(
                                        "Unable to get metadata of the file: {:?}",
                                        description.rel_path,
                                    );
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

                    let file_metadata = read_metadata(description);

                    match file_metadata {
                        Some(x) => {
                            match insert_new_file(&txn, search_db, &x, description).await {
                                Ok(_) => modified = true,
                                Err(_) => error!(
                                    "Failed to insert new file: {}",
                                    description.file_name.clone(),
                                ),
                            };
                        }
                        _none => {
                            error!(
                                "Unable to get metadata of the file: {:?}",
                                description.rel_path,
                            );
                        }
                    }
                }
            }
        };
    }

    // Commit the transaction
    txn.commit().await?;
    if modified {
        search_db.w.commit()?;
    }

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
    db: &E,
    existing_file: &media_files::Model,
    description: &mut FileDescription,
    metadata: &FileMetadata,
) -> Result<()>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let mut active_model: media_files::ActiveModel = existing_file.clone().into();
    active_model.last_modified = ActiveValue::Set(description.last_modified.clone());
    active_model.file_hash = ActiveValue::Set(description.get_crc()?);
    active_model.update(db).await?;

    // Update metadata
    // First, delete existing metadata for the file
    media_metadata::Entity::delete_many()
        .filter(media_metadata::Column::FileId.eq(existing_file.id))
        .exec(db)
        .await?;

    // Then, insert new metadata
    let new_metadata: Vec<media_metadata::ActiveModel> = metadata
        .metadata
        .clone()
        .into_iter()
        .map(|(key, value)| media_metadata::ActiveModel {
            file_id: ActiveValue::Set(existing_file.id),
            meta_key: ActiveValue::Set(key),
            meta_value: ActiveValue::Set(value),
            ..Default::default()
        })
        .collect();
    media_metadata::Entity::insert_many(new_metadata)
        .exec(db)
        .await?;
    Ok(())
}

pub async fn update_file_codec_information<E>(
    db: &E,
    existing_file: &media_files::Model,
    description: &mut FileDescription,
) -> Result<()>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let (sample_rate, duration_in_seconds) = description.get_codec_information()?;

    let mut active_model: media_files::ActiveModel = existing_file.clone().into();
    active_model.sample_rate = ActiveValue::Set(sample_rate.try_into()?);
    active_model.duration = ActiveValue::Set(duration_in_seconds);
    active_model.update(db).await?;

    Ok(())
}

pub async fn insert_new_file<E>(
    main_db: &E,
    search_db: &mut SearchDbConnection,
    metadata: &FileMetadata,
    description: &mut FileDescription,
) -> Result<()>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let (sample_rate, duration_in_seconds) = description.get_codec_information()?;

    description
        .get_crc()
        .with_context(|| format!("Failed to get CRC: {}", description.file_name))?;
    let new_hash = if let Ok(hash) = description.get_crc() {
        hash.clone()
    } else {
        bail!("");
    };

    let new_file = media_files::ActiveModel {
        file_name: ActiveValue::Set(description.file_name.to_string()),
        directory: ActiveValue::Set(description.directory.clone()),
        extension: ActiveValue::Set(description.extension.clone()),
        file_hash: ActiveValue::Set(new_hash),
        sample_rate: ActiveValue::Set(sample_rate.try_into()?),
        duration: ActiveValue::Set(duration_in_seconds),
        last_modified: ActiveValue::Set(description.last_modified.clone()),
        ..Default::default()
    };
    let inserted_file = media_files::Entity::insert(new_file).exec(main_db).await?;

    if let Some((_, value)) = metadata
        .metadata
        .iter()
        .find(|(key, _)| key == "track_title")
    {
        add_term(
            search_db,
            CollectionType::Track,
            inserted_file.last_insert_id,
            value,
        );
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

async fn clean_up_database(
    main_db: &DatabaseConnection,
    search_db: &mut SearchDbConnection,
    root_path: &Path,
) -> Result<()> {
    let db_files = media_files::Entity::find().all(main_db).await?;

    let mut modified = false;

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

            modified = true;
            remove_term(search_db, CollectionType::Track, db_file.id)
        }
    }

    if modified {
        search_db.w.commit()?;
    }

    Ok(())
}

pub fn empty_progress_callback(_processed: usize) {}

pub async fn scan_audio_library<F>(
    main_db: &DatabaseConnection,
    search_db: &mut SearchDbConnection,
    lib_path: &Path,
    cleanup: bool,
    progress_callback: F,
    cancel_token: Option<CancellationToken>,
) -> Result<usize, sea_orm::DbErr>
where
    F: Fn(usize) + Send + Sync,
{
    let root_path_str = lib_path.to_str().expect("Invalid UTF-8 sequence in path");
    let mut scanner = AudioScanner::new(&root_path_str);

    info!("Starting audio library scan");

    // Get the total number of files to scan (assuming AudioScanner has this method)
    let mut processed_files = 0;

    // Read audio files at a time until no more files are available.
    while !scanner.has_ended() {
        // Check if the cancellation token has been triggered
        if let Some(ref token) = cancel_token {
            if token.is_cancelled() {
                info!("Scan cancelled.");
                return Ok(processed_files);
            }
        }

        debug!("Reading metadata for the next 12 files");
        let files = scanner.read_files(12);
        let mut descriptions: Vec<Option<FileDescription>> = files
            .clone()
            .into_iter()
            .map(|file| describe_file(file.path(), lib_path))
            .map(|result| result.ok())
            .collect();

        match sync_file_descriptions(main_db, search_db, &mut descriptions).await {
            Ok(_) => {
                debug!("Finished one batch");
            }
            Err(e) => {
                error!("Error describing files: {:?}", e);
            }
        };

        let file_ids = get_file_ids_by_descriptions(main_db, &descriptions).await?;

        match index_media_files(main_db, search_db, file_ids).await {
            Ok(_) => {}
            Err(e) => error!("Error indexing files: {:?}", e),
        };

        // Update the number of processed files
        processed_files += files.len();

        // Call the progress callback if it is provided
        progress_callback(processed_files);
    }

    if cleanup {
        info!("Starting cleanup process.");
        match clean_up_database(main_db, search_db, lib_path).await {
            Ok(_) => info!("Cleanup completed successfully."),
            Err(e) => error!("Error during cleanup: {:?}", e),
        }
    }

    info!("Audio library scan completed.");

    Ok(processed_files)
}

#[derive(Debug, Clone, Default)]
pub struct MetadataSummary {
    pub id: i32,
    pub directory: String,
    pub file_name: String,
    pub artist: String,
    pub album: String,
    pub title: String,
    pub track_number: Option<i32>,
    pub duration: f64,
    pub cover_art_id: Option<i32>,
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
        .filter(
            media_metadata::Column::FileId
                .is_in(file_ids.clone())
                .and(media_metadata::Column::MetaKey.is_in(["artist", "album", "track_title"])),
        )
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

        let summary = MetadataSummary {
            id: file_id,
            directory: file.directory.clone(),
            file_name: file.file_name.clone(),
            artist: metadata.get("artist").cloned().unwrap_or_default(),
            album: metadata.get("album").cloned().unwrap_or_default(),
            title: metadata.get("track_title").cloned().unwrap_or_default(),
            track_number: metadata
                .get("track_number")
                .map(|s| s.parse::<i32>().ok())
                .unwrap_or(None),
            duration,
            cover_art_id: if cover_art_id == magic_cover_art_id {
                None
            } else {
                cover_art_id
            },
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
    let file_entries: Vec<media_files::Model> = media_files::Entity::find()
        .filter(media_files::Column::Id.is_in(file_ids.clone()))
        .all(db)
        .await?;

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
