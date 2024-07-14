use metadata::describe::{describe_file, FileDescription};
use metadata::scanner::{MetadataScanner, FileMetadata};
use sea_orm::entity::prelude::*;
use sea_orm::{ActiveValue, ColumnTrait, EntityTrait, QueryFilter};
use std::path::PathBuf;

use crate::entities::media_files;
use crate::entities::media_metadata;

fn to_unix_path_string(path_buf: PathBuf) -> Option<String> {
    let path = path_buf.as_path();
    path.to_str().map(|path_str| path_str.replace("\\", "/"))
}

pub async fn process_file(
    db: &DatabaseConnection,
    metadata: FileMetadata,
    description: &FileDescription,
    unix_path: &str,
) {
    // Check if the file already exists in the database
    let existing_file = media_files::Entity::find()
        .filter(media_files::Column::FileHash.eq(description.file_hash.clone()))
        .one(db)
        .await
        .unwrap();

    if let Some(existing_file) = existing_file {
        // File exists in the database
        if existing_file.last_modified == description.last_modified {
            // If the file's last modified date hasn't changed, skip it
            return;
        } else {
            // If the file's last modified date has changed, check the hash
            if existing_file.file_hash == description.file_hash {
                // If the hash is the same, update the last modified date
                update_last_modified(db, &existing_file, description).await;
            } else {
                // If the hash is different, update the metadata
                update_file_metadata(db, &existing_file, description, metadata).await;
            }
        }
    } else {
        // If the file is new, insert a new record
        insert_new_file(db, metadata, description, unix_path).await;
    }
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
    description: &FileDescription,
    metadata: FileMetadata,
) {
    let mut active_model: media_files::ActiveModel = existing_file.clone().into();
    active_model.last_modified = ActiveValue::Set(description.last_modified.clone());
    active_model.file_hash = ActiveValue::Set(description.file_hash.clone());
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
    description: &FileDescription,
    unix_path: &str,
) {
    let new_file = media_files::ActiveModel {
        file_name: ActiveValue::Set(unix_path.to_string()),
        directory: ActiveValue::Set(description.directory.clone()),
        extension: ActiveValue::Set(description.extension.clone()),
        file_hash: ActiveValue::Set(description.file_hash.clone()),
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

async fn clean_up_database(db: &DatabaseConnection) {
    let db_files = media_files::Entity::find().all(db).await.unwrap();
    for db_file in db_files {
        let file_path = PathBuf::from(&db_file.file_name);
        if !file_path.exists() {
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
            let description = describe_file(&file.path, root_path).unwrap();
            let unix_path = to_unix_path_string(file.path.clone()).unwrap();

            process_file(db, file, &description, &unix_path).await;
        }
    }

    if cleanup {
        clean_up_database(&db).await;
    }
}