use log::info;
use metadata::describe::{describe_file, FileDescription};
use metadata::scanner::{FileMetadata, MetadataScanner};
use sea_orm::entity::prelude::*;
use sea_orm::{ActiveValue, ColumnTrait, EntityTrait, QueryFilter};
use std::path::PathBuf;

use crate::entities::media_files;
use crate::entities::media_metadata;

pub async fn process_file(
    db: &DatabaseConnection,
    metadata: FileMetadata,
    description: &mut FileDescription,
) {
    info!(
        "Starting to process file: {}, in dir: {}",
        description.file_name.clone(),
        description.directory.clone(),
    );

    // Check if the file already exists in the database
    let existing_file = media_files::Entity::find()
        .filter(media_files::Column::Directory.eq(description.directory.clone()))
        .filter(media_files::Column::FileName.eq(description.file_name.clone()))
        .one(db)
        .await
        .unwrap();

    if let Some(existing_file) = existing_file {
        info!(
            "File exists in the database: {}",
            description.file_name.clone()
        );

        // File exists in the database
        if existing_file.last_modified == description.last_modified {
            // If the file's last modified date hasn't changed, skip it
            info!(
                "File's last modified date hasn't changed, skipping: {}",
                description.file_name.clone()
            );
            return;
        } else {
            // If the file's last modified date has changed, check the hash
            info!(
                "File's last modified date has changed, checking hash: {}",
                description.file_name.clone()
            );
            let new_hash = description.get_crc().unwrap();
            if existing_file.file_hash == new_hash {
                // If the hash is the same, update the last modified date
                info!(
                    "File hash is the same, updating last modified date: {}",
                    description.file_name.clone()
                );
                update_last_modified(db, &existing_file, description).await;
            } else {
                // If the hash is different, update the metadata
                info!(
                    "File hash is different, updating metadata: {}",
                    description.file_name.clone()
                );
                update_file_metadata(db, &existing_file, description, metadata).await;
            }
        }
    } else {
        // If the file is new, insert a new record
        info!(
            "File is new, inserting new record: {}",
            description.file_name.clone()
        );
        insert_new_file(db, metadata, description).await;
    }

    info!(
        "Finished processing file: {}",
        description.file_name.clone()
    );
}

pub async fn update_last_modified(
    db: &DatabaseConnection,
    existing_file: &media_files::Model,
    description: &FileDescription,
) {
    let mut active_model: media_files::ActiveModel = existing_file.clone().into();
    active_model.last_modified = ActiveValue::Set(description.last_modified.clone());
    active_model.update(db).await.unwrap();
}

pub async fn update_file_metadata(
    db: &DatabaseConnection,
    existing_file: &media_files::Model,
    description: &mut FileDescription,
    metadata: FileMetadata,
) {
    let mut active_model: media_files::ActiveModel = existing_file.clone().into();
    active_model.last_modified = ActiveValue::Set(description.last_modified.clone());
    active_model.file_hash = ActiveValue::Set(description.get_crc().unwrap());
    active_model.update(db).await.unwrap();

    // Update metadata
    // First, delete existing metadata for the file
    media_metadata::Entity::delete_many()
        .filter(media_metadata::Column::FileId.eq(existing_file.id))
        .exec(db)
        .await
        .unwrap();

    // Then, insert new metadata
    let new_metadata: Vec<media_metadata::ActiveModel> = metadata
        .metadata
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
        .await
        .unwrap();
}

pub async fn insert_new_file(
    db: &DatabaseConnection,
    metadata: FileMetadata,
    description: &mut FileDescription,
) {
    let new_file = media_files::ActiveModel {
        file_name: ActiveValue::Set(description.file_name.to_string()),
        directory: ActiveValue::Set(description.directory.clone()),
        extension: ActiveValue::Set(description.extension.clone()),
        file_hash: ActiveValue::Set(description.get_crc().unwrap().clone()),
        last_modified: ActiveValue::Set(description.last_modified.clone()),
        ..Default::default()
    };
    let inserted_file = media_files::Entity::insert(new_file)
        .exec(db)
        .await
        .unwrap();

    // Insert metadata
    let new_metadata: Vec<media_metadata::ActiveModel> = metadata
        .metadata
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
        .await
        .unwrap();
}

async fn clean_up_database(db: &DatabaseConnection, root_path: &PathBuf) {
    let db_files = media_files::Entity::find().all(db).await.unwrap();
    for db_file in db_files {
        let full_path = root_path.join(PathBuf::from(&db_file.file_name));
        if !full_path.exists() {
            // Delete the file record
            media_files::Entity::delete_by_id(db_file.id)
                .exec(db)
                .await
                .unwrap();

            // Delete associated metadata
            media_metadata::Entity::delete_many()
                .filter(media_metadata::Column::FileId.eq(db_file.id))
                .exec(db)
                .await
                .unwrap();
        }
    }
}

pub async fn scan_audio_library(db: &DatabaseConnection, root_path: &PathBuf, cleanup: bool) {
    let root_path_str = root_path.to_str().expect("Invalid UTF-8 sequence in path");
    let mut scanner = MetadataScanner::new(&root_path_str);

    // Example usage: Read 5 audio files at a time until no more files are available.
    while !scanner.has_ended() {
        let files = scanner.read_metadata(5);
        for file in files {
            let mut description = describe_file(&file.path, root_path).unwrap();

            process_file(db, file, &mut description).await;
        }
    }

    if cleanup {
        clean_up_database(&db, &root_path).await;
    }
}
