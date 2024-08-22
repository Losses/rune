use log::{debug, error, info};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

use sea_orm::entity::prelude::*;
use sea_orm::{ActiveValue, ColumnTrait, EntityTrait, QueryFilter};
use sea_orm::{DatabaseConnection, TransactionTrait};

use metadata::describe::{describe_file, FileDescription};
use metadata::reader::get_metadata;
use metadata::scanner::AudioScanner;

use crate::actions::file::get_file_ids_by_descriptions;
use crate::actions::index::index_media_files;
use crate::entities::media_files;
use crate::entities::media_metadata;

use super::utils::DatabaseExecutor;

#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub path: PathBuf,
    pub metadata: Vec<(String, String)>,
}

pub fn read_metadata(description: &FileDescription) -> Option<FileMetadata> {
    match get_metadata(description.full_path.to_str().unwrap(), None) {
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
    db: &DatabaseConnection,
    descriptions: &mut [Option<FileDescription>],
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting to process multiple files");

    // Start a transaction
    let txn = db.begin().await?;

    for description in descriptions.iter_mut() {
        match description {
            None => continue,
            Some(description) => {
                info!("Processing file: {}", description.file_name.clone());

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
                            update_last_modified(&txn, &existing_file, description).await?;
                        } else {
                            // If the hash is different, update the metadata
                            debug!(
                                "File hash is different, updating metadata: {}",
                                description.file_name.clone()
                            );

                            update_file_codec_information(&txn, &existing_file, description)
                                .await?;

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
                            insert_new_file(&txn, &x, description).await?;
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

    info!("Finished syncing file data");

    Ok(())
}

pub async fn process_files(
    db: &DatabaseConnection,
    descriptions: &mut [Option<FileDescription>],
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting to process multiple files");

    // Start a transaction
    let txn = db.begin().await?;

    for description in descriptions.iter_mut() {
        match description {
            None => continue,
            Some(description) => {
                info!("Processing file: {}", description.file_name.clone());

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
                            update_last_modified(&txn, &existing_file, description).await?;
                        } else {
                            // If the hash is different, update the metadata
                            debug!(
                                "File hash is different, updating metadata: {}",
                                description.file_name.clone()
                            );

                            update_file_codec_information(&txn, &existing_file, description)
                                .await?;

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
                            insert_new_file(&txn, &x, description).await?;
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

    info!("Finished processing multiple files");

    Ok(())
}

pub async fn update_last_modified<E>(
    db: &E,
    existing_file: &media_files::Model,
    description: &FileDescription,
) -> Result<(), Box<dyn std::error::Error>>
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
) -> Result<(), Box<dyn std::error::Error>>
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
) -> Result<(), Box<dyn std::error::Error>>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let (sample_rate, duration_in_seconds) = description.get_codec_information().unwrap();

    let mut active_model: media_files::ActiveModel = existing_file.clone().into();
    active_model.sample_rate = ActiveValue::Set(sample_rate.try_into().unwrap());
    active_model.duration = ActiveValue::Set(duration_in_seconds);
    active_model.update(db).await?;

    Ok(())
}

pub async fn insert_new_file<E>(
    db: &E,
    metadata: &FileMetadata,
    description: &mut FileDescription,
) -> Result<(), Box<dyn std::error::Error>>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let (sample_rate, duration_in_seconds) = description.get_codec_information().unwrap();
    let new_file = media_files::ActiveModel {
        file_name: ActiveValue::Set(description.file_name.to_string()),
        directory: ActiveValue::Set(description.directory.clone()),
        extension: ActiveValue::Set(description.extension.clone()),
        file_hash: ActiveValue::Set(description.get_crc()?.clone()),
        sample_rate: ActiveValue::Set(sample_rate.try_into().unwrap()),
        duration: ActiveValue::Set(duration_in_seconds),
        last_modified: ActiveValue::Set(description.last_modified.clone()),
        ..Default::default()
    };
    let inserted_file = media_files::Entity::insert(new_file).exec(db).await?;

    // Insert metadata
    let new_metadata: Vec<media_metadata::ActiveModel> = metadata
        .metadata
        .clone()
        .into_iter()
        .map(|(key, value)| media_metadata::ActiveModel {
            file_id: ActiveValue::Set(inserted_file.last_insert_id),
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

async fn clean_up_database(
    db: &DatabaseConnection,
    root_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_files = media_files::Entity::find().all(db).await?;
    for db_file in db_files {
        let full_path = root_path.join(PathBuf::from(&db_file.file_name));
        if !full_path.exists() {
            info!("Cleaning {}", full_path.to_str().unwrap());
            // Delete the file record
            media_files::Entity::delete_by_id(db_file.id)
                .exec(db)
                .await?;
        }
    }
    Ok(())
}

pub async fn scan_audio_library(db: &DatabaseConnection, root_path: &Path, cleanup: bool) {
    let root_path_str = root_path.to_str().expect("Invalid UTF-8 sequence in path");
    let mut scanner = AudioScanner::new(&root_path_str);

    info!("Starting audio library scan");

    // Example usage: Read 8 audio files at a time until no more files are available.
    while !scanner.has_ended() {
        info!("Reading metadata for the next 8 files");
        let files = scanner.read_files(8);
        let mut descriptions: Vec<Option<FileDescription>> = files
            .clone()
            .into_iter()
            .map(|file| describe_file(&file.path().to_path_buf(), root_path))
            .map(|result| result.ok())
            .collect();

        match sync_file_descriptions(db, &mut descriptions).await {
            Ok(_) => {
                debug!("Finished one batch");
            }
            Err(e) => {
                error!("Error describing files: {:?}", e);
            }
        };

        let file_ids = get_file_ids_by_descriptions(db, &descriptions)
            .await
            .unwrap();

        match index_media_files(db, file_ids).await {
            Ok(_) => {}
            Err(e) => error!("Error indexing files: {:?}", e),
        };
    }

    if cleanup {
        info!("Starting cleanup process.");
        match clean_up_database(db, root_path).await {
            Ok(_) => info!("Cleanup completed successfully."),
            Err(e) => error!("Error during cleanup: {:?}", e),
        }
    }

    info!("Audio library scan completed.");
}

#[derive(Error, Debug)]
pub enum MetadataQueryError {
    #[error("Database error: {0}")]
    DbError(#[from] sea_orm::DbErr),
    #[error("Metadata summary not found for file ID: {0}")]
    NotFound(i32),
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
}

pub async fn get_metadata_summary_by_files(
    db: &DatabaseConnection,
    files: Vec<media_files::Model>,
) -> Result<Vec<MetadataSummary>, sea_orm::DbErr> {
    // Extract file IDs from the provided file entries
    let file_ids: Vec<i32> = files.iter().map(|file| file.id).collect();

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
        };

        results.push(summary);
    }

    Ok(results)
}

pub async fn get_metadata_summary_by_file_ids(
    db: &DatabaseConnection,
    file_ids: Vec<i32>,
) -> Result<Vec<MetadataSummary>, sea_orm::DbErr> {
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
) -> Result<MetadataSummary, MetadataQueryError> {
    let results = get_metadata_summary_by_file_ids(db, vec![file_id]).await?;

    results
        .into_iter()
        .next()
        .ok_or(MetadataQueryError::NotFound(file_id))
}
