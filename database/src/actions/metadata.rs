use log::{debug, error, info};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

use sea_orm::entity::prelude::*;
use sea_orm::{ActiveValue, ColumnTrait, EntityTrait, QueryFilter};
use sea_orm::{DatabaseConnection, DatabaseTransaction, TransactionTrait};

use metadata::describe::{describe_file, FileDescription};
use metadata::scanner::{FileMetadata, MetadataScanner};

use crate::entities::media_metadata;
use crate::entities::{media_analysis, media_files};

pub async fn process_files(
    db: &DatabaseConnection,
    metadatas: &[FileMetadata],
    descriptions: &mut [Option<FileDescription>],
) -> Result<(), Box<dyn std::error::Error>> {
    // Ensure the lengths of metadatas and descriptions match
    if metadatas.len() != descriptions.len() {
        return Err(
            "The number of metadata entries does not match the number of descriptions".into(),
        );
    }

    info!("Starting to process multiple files");

    // Start a transaction
    let txn = db.begin().await?;

    for (metadata, description) in metadatas.iter().zip(descriptions.iter_mut()) {
        match description {
            None => continue,
            Some(description) => {
                info!(
                    "Processing file: {}, in dir: {}",
                    description.file_name.clone(),
                    description.directory.clone(),
                );

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
                            update_file_metadata(&txn, &existing_file, description, metadata)
                                .await?;
                        }
                    }
                } else {
                    // If the file is new, insert a new record
                    debug!(
                        "File is new, inserting new record: {}",
                        description.file_name.clone()
                    );
                    insert_new_file(&txn, metadata, description).await?;
                }
            }
        };
    }

    // Commit the transaction
    txn.commit().await?;

    info!("Finished processing multiple files");

    Ok(())
}

pub trait DatabaseExecutor: Send + Sync {}

impl DatabaseExecutor for DatabaseConnection {}
impl DatabaseExecutor for DatabaseTransaction {}

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

pub async fn insert_new_file<E>(
    db: &E,
    metadata: &FileMetadata,
    description: &mut FileDescription,
) -> Result<(), Box<dyn std::error::Error>>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let new_file = media_files::ActiveModel {
        file_name: ActiveValue::Set(description.file_name.to_string()),
        directory: ActiveValue::Set(description.directory.clone()),
        extension: ActiveValue::Set(description.extension.clone()),
        file_hash: ActiveValue::Set(description.get_crc()?.clone()),
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
    let mut scanner = MetadataScanner::new(&root_path_str);

    info!("Starting audio library scan.");

    // Example usage: Read 5 audio files at a time until no more files are available.
    while !scanner.has_ended() {
        info!("Reading metadata for the next 8 files.");
        let files = scanner.read_metadata(8);
        let mut descriptions: Vec<Option<FileDescription>> = files
            .clone()
            .into_iter()
            .map(|file| describe_file(&file.path, root_path))
            .map(|result| result.ok())
            .collect();
        match process_files(db, &files, &mut descriptions).await {
            Ok(_) => {
                debug!("Finished one batch");
            }
            Err(e) => {
                error!("Error describing files: {:?}", e);
            }
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
    pub path: String,
    pub artist: String,
    pub album: String,
    pub title: String,
    pub duration: f64,
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

    // Fetch all metadata entries for the given file IDs
    let metadata_entries: Vec<media_metadata::Model> = media_metadata::Entity::find()
        .filter(
            media_metadata::Column::FileId
                .is_in(file_ids.clone())
                .and(media_metadata::Column::MetaKey.is_in(["artist", "album", "track_title"])),
        )
        .all(db)
        .await?;

    // Fetch all analysis entries for the given file IDs
    let analysis_entries: Vec<media_analysis::Model> = media_analysis::Entity::find()
        .filter(media_analysis::Column::FileId.is_in(file_ids.clone()))
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

    // Create a map for analysis entries
    let mut analysis_map: HashMap<i32, f64> = HashMap::new();
    for entry in analysis_entries {
        analysis_map.insert(entry.file_id, entry.duration);
    }

    // Create a map for file entries
    let mut file_map: HashMap<i32, String> = HashMap::new();
    for entry in file_entries {
        let path = format!("{}/{}", entry.directory, entry.file_name);
        file_map.insert(entry.id, path);
    }

    // Prepare the final result
    let mut results: Vec<MetadataSummary> = Vec::new();
    for file_id in file_ids {
        let path = file_map.get(&file_id).cloned().unwrap_or_default();
        let _metadata: HashMap<String, String> = HashMap::new();
        let metadata = metadata_map.get(&file_id).unwrap_or(&_metadata);
        let duration = *analysis_map.get(&file_id).unwrap_or(&0.0);

        let summary = MetadataSummary {
            id: file_id,
            path,
            artist: metadata.get("artist").cloned().unwrap_or_default(),
            album: metadata.get("album").cloned().unwrap_or_default(),
            title: metadata.get("track_title").cloned().unwrap_or_default(),
            duration,
        };

        results.push(summary);
    }

    Ok(results)
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
