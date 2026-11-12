use crate::actions::{
    file::{
        MANAGED_AUDIO_DIRECTORY, get_file_ids_by_descriptions, is_managed_directory, remove_track,
    },
    index::index_media_files,
    logging::{LogLevel, insert_log},
};
use crate::entities::{
    albums, artists, media_file_albums, media_file_artists, media_files, media_metadata,
    sync_download_jobs, sync_download_jobs::Column,
};
use crate::error::ExpectedError;
use sea_orm::ExprTrait;
use sea_orm::sqlx::Error;
use std::collections::HashSet;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
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

use super::utils::DatabaseExecutor;

#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub path: PathBuf,
    pub metadata: Vec<(String, String)>,
}

pub fn read_metadata(fsio: &FsIo, fs_node: &FsNode) -> Result<FileMetadata> {
    match get_metadata(fsio, fs_node, None)
        .with_context(|| format!("Unable to read metadata: {:#?}", fs_node.path))
    {
        Ok(metadata) => Ok(FileMetadata {
            path: fs_node.path.clone(),
            metadata,
        }),
        Err(e) => Err(e),
    }
}

/// Batch query to check which files have been modified since last database entry
/// Returns a map of (directory, file_name) -> change token for files that exist in database
pub async fn get_existing_files_modified(
    main_db: &DatabaseConnection,
    file_keys: Vec<(String, String)>, // (directory, file_name)
) -> Result<std::collections::HashMap<(String, String), Option<String>>> {
    if file_keys.is_empty() {
        return Ok(HashMap::new());
    }

    // Split into chunks to avoid SQL query size limits
    let chunk_size = 500;
    let mut result = HashMap::new();

    for chunk in file_keys.chunks(chunk_size) {
        let mut conditions = sea_orm::Condition::any();

        for (directory, file_name) in chunk {
            conditions = conditions.add(
                sea_orm::Condition::all()
                    .add(media_files::Column::Directory.eq(directory.clone()))
                    .add(media_files::Column::FileName.eq(file_name.clone())),
            );
        }

        let existing_files = media_files::Entity::find()
            .filter(media_files::Column::LocalAvailable.eq(true))
            .filter(media_files::Column::ContentHash.is_not_null())
            .filter(media_files::Column::ContentSize.is_not_null())
            .filter(conditions)
            .all(main_db)
            .await
            .with_context(|| "Failed to query existing files")?;

        for file in existing_files {
            result.insert((file.directory, file.file_name), file.change_token);
        }
    }

    Ok(result)
}

/// Filter out files whose change token has not changed.
/// Returns (files_to_process, skipped_count)
pub fn filter_modified_files(
    descriptions: Vec<Option<FileDescription>>,
    existing_modified_map: &std::collections::HashMap<(String, String), Option<String>>,
) -> (Vec<Option<FileDescription>>, usize) {
    let mut filtered = Vec::new();
    let mut skipped_count = 0;

    for description in descriptions {
        match description {
            Some(desc) => {
                let key = (desc.directory.clone(), desc.file_name.clone());

                if let Some(db_last_modified) = existing_modified_map.get(&key) {
                    // File exists in database, check if it's been modified
                    if change_token_is_current(
                        db_last_modified.as_deref(),
                        desc.change_token.as_deref(),
                    ) {
                        // File hasn't been modified, skip it
                        skipped_count += 1;
                        continue;
                    }
                }

                // Either file doesn't exist in database or has been modified
                filtered.push(Some(desc));
            }
            None => filtered.push(None),
        }
    }

    (filtered, skipped_count)
}

fn change_token_is_current(stored: Option<&str>, scanned: Option<&str>) -> bool {
    stored
        .zip(scanned)
        .is_some_and(|(stored, scanned)| stored == scanned)
}

// SQLite constraint failures and explicit row validation reject one record. Connection,
// query, and I/O failures are fatal even if they occur inside a savepoint.
pub(crate) fn is_fatal_scan_error(error: &anyhow::Error) -> bool {
    use std::ops::Deref;
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<sea_orm::DbErr>())
        .any(|db| match db {
            sea_orm::DbErr::Custom(_) => false,
            sea_orm::DbErr::Exec(sea_orm::RuntimeErr::SqlxError(error))
            | sea_orm::DbErr::Query(sea_orm::RuntimeErr::SqlxError(error)) => match error.deref() {
                Error::Database(error) => !error
                    .code()
                    .and_then(|code| code.parse::<i32>().ok())
                    .is_some_and(|code| code & 0xff == 19),
                _ => true,
            },
            _ => true,
        })
}

pub async fn sync_file_descriptions(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    node_id: &str,
    descriptions: &mut [Option<FileDescription>],
    force: bool,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<()> {
    let mut work = descriptions.to_vec();
    let failed =
        sync_file_descriptions_inner(fsio, main_db, node_id, &mut work, force, store).await?;
    for (original, result) in descriptions.iter_mut().zip(work) {
        if result.is_some() {
            *original = result;
        }
    }
    if failed > 0 {
        anyhow::bail!("Failed to save metadata for {failed} files");
    }
    Ok(())
}

async fn sync_file_descriptions_inner(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    node_id: &str,
    descriptions: &mut [Option<FileDescription>],
    force: bool,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<usize> {
    // Bound both prepared metadata memory and write-lock duration. File I/O is
    // finished before opening each batch transaction; savepoints isolate bad files.
    const WRITE_BATCH_SIZE: usize = 16;
    let mut failed = 0;
    let mut rejected = HashSet::new();
    for batch in descriptions.chunks_mut(WRITE_BATCH_SIZE) {
        let mut ready = Vec::new();
        for description in batch.iter_mut().flatten() {
            // Managed copies are verified by file actions and attached after download.
            // Unreferenced old versions must never become newly authored files.
            if Path::new(&description.directory).starts_with(MANAGED_AUDIO_DIRECTORY) {
                rejected.insert((description.directory.clone(), description.file_name.clone()));
                continue;
            }
            // The download queue reserves destinations until attachment completes.
            if sync_download_jobs::Entity::find()
                .filter(Column::Directory.eq(&description.directory))
                .filter(Column::FileName.eq(&description.file_name))
                .one(main_db)
                .await?
                .is_some()
            {
                rejected.insert((description.directory.clone(), description.file_name.clone()));
                continue;
            }
            let existing = media_files::Entity::find()
                .filter(media_files::Column::Directory.eq(&description.directory))
                .filter(media_files::Column::FileName.eq(&description.file_name))
                .one(main_db)
                .await?;
            if !force
                && existing.as_ref().is_some_and(|file| {
                    file.local_available
                        && file.content_hash.is_some()
                        && file.content_size.is_some()
                        && change_token_is_current(
                            file.change_token.as_deref(),
                            description.change_token.as_deref(),
                        )
                })
            {
                rejected.insert((description.directory.clone(), description.file_name.clone()));
                continue;
            }

            let fs = fsio.clone();
            let mut prepared = description.clone();
            let old_hash = existing.as_ref().and_then(|file| file.content_hash.clone());
            let prepared = tokio::task::spawn_blocking(move || -> Result<_> {
                prepared.get_crc(&fs)?;
                let mut stream = fs.open(&prepared.raw_node.path, "r")?;
                let mut hasher = blake3::Hasher::new();
                let size = std::io::copy(&mut stream, &mut hasher)?;
                let size = i64::try_from(size).context("File size exceeds supported range")?;
                let digest = hasher.finalize().to_hex().to_string();
                let metadata = if !force && old_hash.as_ref() == Some(&digest) {
                    None
                } else {
                    prepared.get_codec_information(&fs)?;
                    Some(read_metadata(&fs, &prepared.raw_node)?)
                };
                Ok((prepared, metadata, digest, size))
            })
            .await
            .context("Metadata worker failed")?;
            let (prepared, metadata, digest, size) = match prepared {
                Ok(value) => value,
                Err(e) => {
                    error!("Failed to prepare {}: {e:#}", description.file_name);
                    failed += 1;
                    rejected.insert((description.directory.clone(), description.file_name.clone()));
                    continue;
                }
            };
            if existing.as_ref().is_some_and(|file| {
                !file.local_available
                    && file
                        .content_hash
                        .as_ref()
                        .is_some_and(|expected| expected != &digest)
            }) {
                error!(
                    "Local content does not match the catalog reference: {}",
                    description.file_name
                );
                failed += 1;
                rejected.insert((description.directory.clone(), description.file_name.clone()));
                continue;
            }
            *description = prepared;

            ready.push((description, existing, metadata, digest, size));
        }
        if ready.is_empty() {
            continue;
        }
        let batch_txn = main_db.begin().await?;
        store.lock(&batch_txn).await?;
        for (description, existing, metadata, digest, size) in ready {
            if sync_download_jobs::Entity::find()
                .filter(Column::Directory.eq(&description.directory))
                .filter(Column::FileName.eq(&description.file_name))
                .one(&batch_txn)
                .await?
                .is_some()
            {
                rejected.insert((description.directory.clone(), description.file_name.clone()));
                continue;
            }
            let current = media_files::Entity::find()
                .filter(media_files::Column::Directory.eq(&description.directory))
                .filter(media_files::Column::FileName.eq(&description.file_name))
                .one(&batch_txn)
                .await?;
            if current != existing {
                error!(
                    "File changed during metadata scan: {}",
                    description.file_name
                );
                failed += 1;
                rejected.insert((description.directory.clone(), description.file_name.clone()));
                continue;
            }
            let txn = batch_txn.begin().await?;
            let result: Result<()> = async {
                match (&existing, &metadata) {
                    (Some(file), Some(metadata)) => {
                        update_file_codec_information(fsio, &txn, file, description).await?;
                        update_file_metadata(fsio, &txn, node_id, file, description, metadata)
                            .await?;
                        unlink_cover_art(&txn, file).await?;
                    }
                    (Some(file), None) => update_last_modified(&txn, file, description).await?,
                    (None, Some(metadata)) => {
                        insert_new_file(fsio, &txn, metadata, description).await?
                    }
                    (None, None) => unreachable!("New files always have metadata"),
                }
                media_files::Entity::update_many()
                    .col_expr(media_files::Column::LocalAvailable, Expr::value(true))
                    .col_expr(media_files::Column::ContentHash, Expr::value(Some(digest)))
                    .col_expr(media_files::Column::ContentSize, Expr::value(Some(size)))
                    .filter(media_files::Column::Directory.eq(&description.directory))
                    .filter(media_files::Column::FileName.eq(&description.file_name))
                    .exec(&txn)
                    .await?;

                author_track(&txn, store, &description.directory, &description.file_name).await?;

                Ok(())
            }
            .await;
            match result {
                Ok(()) => txn.commit().await?,
                Err(e) => {
                    txn.rollback().await?;
                    error!("Failed to save {}: {e:#}", description.file_name);
                    if is_fatal_scan_error(&e) {
                        return Err(e);
                    }
                    failed += 1;
                    rejected.insert((description.directory.clone(), description.file_name.clone()));
                }
            }
        }
        batch_txn
            .commit()
            .await
            .context("Failed to commit metadata batch")?;
    }
    for description in descriptions.iter_mut() {
        if description
            .as_ref()
            .is_some_and(|d| rejected.contains(&(d.directory.clone(), d.file_name.clone())))
        {
            *description = None;
        }
    }
    Ok(failed)
}

async fn author_track(
    txn: &sea_orm::DatabaseTransaction,
    store: &crate::sync::SeaOrmSyncStore,
    directory: &str,
    file_name: &str,
) -> Result<()> {
    let file = media_files::Entity::find()
        .filter(media_files::Column::Directory.eq(directory))
        .filter(media_files::Column::FileName.eq(file_name))
        .one(txn)
        .await?
        .context("scanned file missing")?;
    author_track_row(txn, store, file).await
}

pub async fn author_track_row(
    txn: &sea_orm::DatabaseTransaction,
    store: &crate::sync::SeaOrmSyncStore,
    file: media_files::Model,
) -> Result<()> {
    use crate::sync::model::{Collection, ObjectKey, author_source_on};
    author_source_on(
        txn,
        store,
        ObjectKey {
            collection: Collection::Tracks,
            id: file.source_uuid.parse()?,
        },
    )
    .await
}

pub async fn author_indexed_track_row(
    txn: &sea_orm::DatabaseTransaction,
    store: &crate::sync::SeaOrmSyncStore,
    file: media_files::Model,
) -> Result<()> {
    author_track_row(txn, store, file).await
}

pub async fn process_files(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    node_id: &str,
    descriptions: &mut [Option<FileDescription>],
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<()> {
    sync_file_descriptions(fsio, main_db, node_id, descriptions, false, store).await
}

async fn unlink_cover_art<E>(db: &E, existing_file: &media_files::Model) -> Result<()>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let mut active_model: media_files::ActiveModel = existing_file.clone().into();
    active_model.cover_art_uuid = ActiveValue::Set(None);
    active_model.cover_art_checked_at = ActiveValue::Set(None);
    active_model.update(db).await?;
    Ok(())
}

async fn update_last_modified<E>(
    db: &E,
    existing_file: &media_files::Model,
    description: &FileDescription,
) -> Result<()>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let mut active_model: media_files::ActiveModel = existing_file.clone().into();
    active_model.last_modified = ActiveValue::Set(description.last_modified.clone());
    active_model.change_token = ActiveValue::Set(description.change_token.clone());
    active_model.local_available = ActiveValue::Set(true);
    active_model.update(db).await?;
    Ok(())
}

async fn update_file_metadata(
    fsio: &FsIo,
    db: &sea_orm::DatabaseTransaction,
    _node_id: &str,
    existing_file: &media_files::Model,
    description: &mut FileDescription,
    metadata: &FileMetadata,
) -> Result<()> {
    let mut active_model: media_files::ActiveModel = existing_file.clone().into();

    // Update last modified and file hash
    active_model.last_modified = ActiveValue::Set(description.last_modified.clone());
    active_model.change_token = ActiveValue::Set(description.change_token.clone());
    active_model.local_available = ActiveValue::Set(true);

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

    replace_metadata_on(db, existing_file.id, &metadata.metadata).await?;
    Ok(())
}

async fn update_file_codec_information<E>(
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

async fn insert_new_file(
    fsio: &FsIo,
    main_db: &sea_orm::DatabaseTransaction,
    metadata: &FileMetadata,
    description: &mut FileDescription,
) -> Result<()> {
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
        change_token: ActiveValue::Set(description.change_token.clone()),
        source_uuid: ActiveValue::Set(Uuid::new_v4().to_string()),
        local_available: ActiveValue::Set(true),
        ..Default::default()
    };
    let inserted_file = media_files::Entity::insert(new_file).exec(main_db).await?;

    let file_id = inserted_file.last_insert_id;

    // Insert metadata
    replace_metadata_on(main_db, file_id, &metadata.metadata).await?;

    Ok(())
}

async fn replace_metadata_on(
    tx: &sea_orm::DatabaseTransaction,
    file_id: i32,
    tags: &[(String, String)],
) -> Result<()> {
    use crate::sync::model::{MetadataMap, MetadataTag};
    use sync_orm::owned_list::{ListBudget, OwnedListPolicy};
    type Tags = OwnedListPolicy<media_metadata::Entity, MetadataMap>;
    let owner = [file_id.into()];
    let previous = Tags::read_on(tx, &owner, Tags::read_ceiling()).await?;
    let mut used = HashSet::new();
    let incoming = tags
        .iter()
        .enumerate()
        .map(|(position, (key, value))| {
            let retained = previous
                .iter()
                .find(|tag| !used.contains(&tag.id) && tag.key == *key && tag.value == *value)
                .or_else(|| {
                    previous
                        .iter()
                        .find(|tag| !used.contains(&tag.id) && tag.key == *key)
                });
            let id = retained.map_or_else(Uuid::new_v4, |tag| tag.id);
            used.insert(id);
            Ok(MetadataTag {
                id,
                position: i32::try_from(position)?,
                key: key.clone(),
                value: value.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Tags::prepare_on(
        tx,
        &owner,
        &incoming,
        ListBudget {
            max_reads: 129,
            max_mutations: 384,
        },
    )
    .await?
    .apply_on(tx)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum MissingHostFilePolicy {
    #[default]
    RemoveEverywhere,
    KeepAsRemote,
}

async fn clean_up_database(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    root_path: &Path,
    policy: MissingHostFilePolicy,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<()> {
    let files = media_files::Entity::find()
        .filter(media_files::Column::LocalAvailable.eq(true))
        .all(main_db)
        .await?;
    let host_total = files
        .iter()
        .filter(|file| !is_managed_directory(&file.directory))
        .count();
    let fsio_for_missing = fsio.clone();
    let root_for_missing = root_path.to_path_buf();
    let missing: Vec<_> = tokio::task::spawn_blocking(move || {
        files
            .into_iter()
            .filter(|file| {
                matches!(
                    fsio_for_missing
                        .exists(&root_for_missing.join(&file.directory).join(&file.file_name)),
                    Ok(false)
                )
            })
            .collect()
    })
    .await?;
    let host_missing = missing
        .iter()
        .filter(|file| !is_managed_directory(&file.directory))
        .count();
    let mass_missing = host_missing > 5 && host_missing * 2 > host_total;
    if mass_missing {
        log::warn!("Skipping missing host file removals because the library looks unmounted");
    }
    let root_listable = fsio.read_dir(root_path).await.is_ok();
    for file in missing {
        let result: Result<()> = async {
            if !is_managed_directory(&file.directory)
                && policy == MissingHostFilePolicy::RemoveEverywhere
                && root_listable
                && !mass_missing
            {
                let tx = main_db.begin().await?;
                store.lock(&tx).await?;
                let current = media_files::Entity::find_by_id(file.id).one(&tx).await?;
                if current.as_ref().is_some_and(|row| {
                    row.local_available
                        && row.directory == file.directory
                        && row.file_name == file.file_name
                }) {
                    remove_track(&tx, store, file.id).await?;
                }
                tx.commit().await?;
            } else {
                media_files::Entity::update_many()
                    .col_expr(media_files::Column::LocalAvailable, Expr::value(false))
                    .filter(media_files::Column::Id.eq(file.id))
                    .filter(media_files::Column::Directory.eq(file.directory))
                    .filter(media_files::Column::FileName.eq(file.file_name))
                    .filter(media_files::Column::LocalAvailable.eq(true))
                    .exec(main_db)
                    .await?;
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            error!("Failed to clean up missing file {}: {error:#}", file.id);
        }
    }
    Ok(())
}

pub fn empty_progress_callback(_processed: usize) {}

/// Index one verified audio file without walking unrelated library directories.
pub async fn scan_audio_file(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    node_id: &str,
    lib_path: &Path,
    path: &Path,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<i32> {
    let fsio_for_describe = fsio.clone();
    let path_for_describe = path.to_path_buf();
    let root_for_describe = lib_path.to_path_buf();
    let description = tokio::task::spawn_blocking(move || {
        let node = fsio_for_describe.canonicalize(&path_for_describe)?;
        let description = describe_file(&fsio_for_describe, &node, &Some(root_for_describe))?;
        Ok::<_, anyhow::Error>(description)
    })
    .await??;
    let mut descriptions = vec![Some(description)];
    sync_file_descriptions(fsio, main_db, node_id, &mut descriptions, false, store).await?;
    let ids = get_file_ids_by_descriptions(main_db, &descriptions).await?;
    let id = *ids.first().context("Audio file was not indexed")?;
    index_media_files(main_db, node_id, ids, None, store).await?;
    Ok(id)
}

pub struct ScanOptions {
    pub cleanup: bool,
    pub force: bool,
    pub missing_host_file: MissingHostFilePolicy,
    pub cancel_token: Option<CancellationToken>,
}

pub async fn scan_audio_library<F>(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    node_id: &str,
    lib_path: &Path,
    options: ScanOptions,
    progress_callback: F,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<usize>
where
    F: Fn(usize) + Send + Sync,
{
    let ScanOptions {
        cleanup,
        force,
        missing_host_file: policy,
        cancel_token,
    } = options;
    let fsio_for_scan = fsio.clone();
    let root_for_scan = lib_path.to_path_buf();
    let mut scanner = match tokio::task::spawn_blocking(move || {
        AudioScanner::new(&fsio_for_scan, &root_for_scan)
    })
    .await?
    {
        Ok(scanner) => scanner,
        Err(error) => {
            if cleanup
                && let Err(cleanup_error) =
                    clean_up_database(fsio, main_db, lib_path, policy, store).await
            {
                log::error!("Unable to clean up unavailable library: {cleanup_error:#}");
            }
            return Err(error.into());
        }
    };

    info!("Starting audio library scan with last-modified pre-filtering");

    let mut processed_files = 0;
    let mut total_files_scanned = 0;
    let mut skipped_files = 0;

    // Read audio files in larger batches for better performance
    const BATCH_SIZE: usize = 48;

    while !scanner.has_ended() {
        // Check if the cancellation token has been triggered
        if let Some(ref token) = cancel_token
            && token.is_cancelled()
        {
            info!("Scan cancelled.");
            return Ok(processed_files);
        }

        debug!("Reading metadata for the next {} files", BATCH_SIZE);
        let files = scanner.read_files(BATCH_SIZE);
        total_files_scanned += files.len();

        if files.is_empty() {
            continue;
        }

        let mut descriptions: Vec<Option<FileDescription>> = Vec::with_capacity(files.len());
        for file in files.clone() {
            let fsio_for_describe = fsio.clone();
            let root_for_describe = lib_path.to_path_buf();
            let description = tokio::task::spawn_blocking(move || {
                describe_file(&fsio_for_describe, &file, &Some(root_for_describe))
            })
            .await?;
            match description {
                Ok(value) => descriptions.push(Some(value)),
                Err(error) => {
                    error!("Failed to describe audio file: {error:#}");
                    descriptions.push(None);
                }
            }
        }

        // Pre-filtering: Skip files that haven't been modified
        if !force {
            // Collect file keys for batch database query
            let file_keys: Vec<(String, String)> = descriptions
                .iter()
                .filter_map(|desc| {
                    desc.as_ref()
                        .map(|d| (d.directory.clone(), d.file_name.clone()))
                })
                .collect();

            // Batch query existing files' last_modified timestamps
            match get_existing_files_modified(main_db, file_keys).await {
                Ok(existing_modified_map) => {
                    // Filter out unchanged files
                    let (filtered_descriptions, skipped_count) =
                        filter_modified_files(descriptions, &existing_modified_map);

                    skipped_files += skipped_count;
                    descriptions = filtered_descriptions;

                    if skipped_count > 0 {
                        debug!("Skipped {} unchanged files in this batch", skipped_count);
                    }
                }
                Err(e) => {
                    error!("Failed to query existing files for pre-filtering: {e:#?}");
                    // Continue with all files if pre-filtering fails
                }
            }
        }

        // Process only the files that need updating
        if !descriptions.iter().any(|d| d.is_some()) {
            debug!("All files in batch were unchanged, skipping");
            progress_callback(total_files_scanned);
            continue;
        }

        match sync_file_descriptions_inner(fsio, main_db, node_id, &mut descriptions, force, store)
            .await
            .with_context(|| "Unable to describe files")
        {
            Ok(failed) => {
                if failed > 0 {
                    info!("Failed to scan {failed} files in this batch");
                }
                debug!(
                    "Finished one batch with {} files to process",
                    descriptions.iter().filter(|d| d.is_some()).count()
                );
            }
            Err(e) => return Err(e),
        };

        if cancel_token
            .as_ref()
            .is_some_and(|token| token.is_cancelled())
        {
            return Ok(processed_files);
        }
        let file_ids = get_file_ids_by_descriptions(main_db, &descriptions).await?;

        match index_media_files(main_db, node_id, file_ids, cancel_token.as_ref(), store)
            .await
            .with_context(|| "Unable to index files")
        {
            Ok(_) => {}
            Err(e) => return Err(e),
        };

        // Update the number of processed files
        processed_files += descriptions.iter().filter(|d| d.is_some()).count();

        // Call the progress callback with total files scanned
        progress_callback(total_files_scanned);
    }

    if skipped_files > 0 {
        info!("Pre-filtering skipped {} unchanged files", skipped_files);
    }

    if cleanup
        && !cancel_token
            .as_ref()
            .is_some_and(|token| token.is_cancelled())
    {
        info!("Starting cleanup process.");
        match clean_up_database(fsio, main_db, lib_path, policy, store)
            .await
            .with_context(|| "Unable to cleanup database")
        {
            Ok(_) => info!("Cleanup completed successfully."),
            Err(e) => error!("{e:#?}"),
        }
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
    pub track_number: Option<i32>,
    pub duration: f64,
    pub cover_art_id: Option<i32>,
    pub file_hash: String,
    pub file_uuid: String,
}

pub async fn get_metadata_summary_by_files<C: sea_orm::ConnectionTrait>(
    db: &C,
    files: Vec<media_files::Model>,
) -> Result<Vec<MetadataSummary>> {
    // Extract file IDs from the provided file entries
    let file_ids: Vec<i32> = files.iter().map(|file| file.id).collect();
    use crate::entities::media_cover_art;
    use sea_orm::QuerySelect;
    let artwork_uuids: Vec<_> = files
        .iter()
        .filter_map(|file| file.cover_art_uuid.clone())
        .collect();
    let artwork_ids: HashMap<String, i32> = media_cover_art::Entity::find()
        .select_only()
        .columns([
            media_cover_art::Column::SourceUuid,
            media_cover_art::Column::Id,
        ])
        .filter(media_cover_art::Column::SourceUuid.is_in(artwork_uuids))
        .into_tuple::<(String, i32)>()
        .all(db)
        .await?
        .into_iter()
        .collect();

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

        let cover_art_id = file
            .cover_art_uuid
            .as_ref()
            .and_then(|id| artwork_ids.get(id))
            .copied();

        let parsed_disk_number = metadata.get("disc_number").and_then(|s| extract_number(s));

        let parsed_track_number = metadata.get("track_number").and_then(|s| extract_number(s));

        let track_number =
            parsed_track_number.map(|track| parsed_disk_number.unwrap_or(0) * 1000 + track);

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
            cover_art_id,
            file_hash: file.file_hash.clone(),
            file_uuid: file.source_uuid.clone(),
        };

        results.push(summary);
    }

    Ok(results)
}

pub async fn get_metadata_summary_by_file_ids<C: sea_orm::ConnectionTrait>(
    db: &C,
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

pub async fn get_metadata_summary_by_file_id<C: sea_orm::ConnectionTrait>(
    db: &C,
    file_id: i32,
) -> anyhow::Result<MetadataSummary> {
    let results = get_metadata_summary_by_file_ids(db, vec![file_id])
        .await
        .context("Database query failed")?;

    results
        .into_iter()
        .next()
        .ok_or(ExpectedError::NotFound {
            what: "metadata summary",
        })
        .with_context(|| format!("Metadata summary not found for file ID: {file_id}"))
}

pub async fn get_parsed_file_by_id(
    db: &DatabaseConnection,
    file_id: i32,
) -> Result<(MetadataSummary, Vec<artists::Model>, Option<albums::Model>)> {
    let file = get_metadata_summary_by_file_id(db, file_id).await?;

    let artist_ids = media_file_artists::Entity::find()
        .filter(media_file_artists::Column::TrackUuid.eq(&file.file_uuid))
        .all(db)
        .await?;

    let artists = artists::Entity::find()
        .filter(artists::Column::SourceUuid.is_in(artist_ids.into_iter().map(|x| x.artist_uuid)))
        .all(db)
        .await?;

    let album_id = media_file_albums::Entity::find()
        .filter(media_file_albums::Column::TrackUuid.eq(&file.file_uuid))
        .one(db)
        .await?;

    let album = match album_id {
        Some(album_id) => {
            albums::Entity::find()
                .filter(albums::Column::SourceUuid.eq(album_id.album_uuid))
                .one(db)
                .await?
        }
        _none => None,
    };

    Ok((file, artists, album))
}

#[cfg(test)]
mod change_token_tests {
    use super::change_token_is_current;

    #[test]
    fn missing_change_tokens_never_mark_a_file_unchanged() {
        assert!(!change_token_is_current(None, None));
        assert!(!change_token_is_current(Some("etag"), None));
        assert!(!change_token_is_current(None, Some("etag")));
        assert!(change_token_is_current(Some("etag"), Some("etag")));
    }
}
