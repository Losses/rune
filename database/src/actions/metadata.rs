use crate::actions::{
    file::{
        MANAGED_AUDIO_DIRECTORY, get_file_ids_by_descriptions, is_managed_directory, remove_track,
    },
    logging::{LogLevel, insert_log},
};
use crate::connection::RUNE_INDEX_DIRECTORY;
use crate::entities::music_metadata::{
    audio as catalog_audio, credits as catalog_credits, values as catalog_values,
};
use crate::entities::{
    media_analysis, media_files, portable_albums, portable_artists, source_tags,
    sync_download_jobs, sync_download_jobs::Column,
};
use crate::error::ExpectedError;
use sea_orm::sqlx::Error;
use sea_orm::{ExprTrait, sea_query::Query};
use std::collections::HashSet;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use log::{debug, error, info, warn};
use once_cell::sync::Lazy;
use regex::Regex;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    TransactionTrait, entity::prelude::*,
};
use tokio_util::sync::CancellationToken;

use ::fsio::{FsIo, FsNode, OpenMode, StorageLocation};
use ::metadata::{
    describe::{FileDescription, describe_file},
    reader::get_metadata,
    resolve::read_local,
    scanner::AudioScanner,
};

use super::resolved_metadata;
use super::utils::DatabaseExecutor;
use blake3::Hasher;
use sea_orm::Condition;
use sea_orm::DbErr;
use sea_orm::RuntimeErr;
use std::collections;
use std::io;
use tokio::task;

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
) -> Result<collections::HashMap<(String, String), Option<String>>> {
    if file_keys.is_empty() {
        return Ok(HashMap::new());
    }

    // Split into chunks to avoid SQL query size limits
    let chunk_size = 500;
    let mut result = HashMap::new();

    for chunk in file_keys.chunks(chunk_size) {
        let mut conditions = Condition::any();

        for (directory, file_name) in chunk {
            conditions = conditions.add(
                Condition::all()
                    .add(media_files::Column::Directory.eq(directory.clone()))
                    .add(media_files::Column::FileName.eq(file_name.clone())),
            );
        }

        let existing_files = media_files::Entity::find()
            .filter(media_files::Column::LocalAvailable.eq(true))
            .filter(media_files::Column::ContentHash.is_not_null())
            .filter(media_files::Column::ContentSize.is_not_null())
            .filter(Expr::exists(
                Query::select()
                    .column(catalog_audio::Column::FileId)
                    .from(catalog_audio::Entity)
                    .and_where(
                        Expr::col(catalog_audio::Column::FileId.as_column_ref())
                            .eq(Expr::col(media_files::Column::Id.as_column_ref())),
                    )
                    .to_owned(),
            ))
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
    existing_change_tokens: &collections::HashMap<(String, String), Option<String>>,
) -> (Vec<Option<FileDescription>>, usize) {
    let mut filtered = Vec::new();
    let mut skipped_count = 0;

    for description in descriptions {
        match description {
            Some(desc) => {
                let key = (desc.directory.clone(), desc.file_name.clone());

                if let Some(stored_change_token) = existing_change_tokens.get(&key) {
                    // File exists in database, check if it's been modified
                    if change_token_is_current(
                        stored_change_token.as_deref(),
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
            DbErr::Custom(_) => false,
            DbErr::Exec(RuntimeErr::SqlxError(error))
            | DbErr::Query(RuntimeErr::SqlxError(error)) => match error.deref() {
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
    let outcome =
        sync_file_descriptions_inner(fsio, main_db, node_id, &mut work, force, None, store).await?;
    resolved_metadata::reconcile_releases(main_db, store, None).await?;
    for (original, result) in descriptions.iter_mut().zip(work) {
        if result.is_some() {
            *original = result;
        }
    }
    if outcome.failed > 0 {
        anyhow::bail!("Failed to save metadata for {} files", outcome.failed);
    }
    Ok(())
}

#[derive(Default)]
struct ScanOutcome {
    saved: usize,
    failed: usize,
}

async fn sync_file_descriptions_inner(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    node_id: &str,
    descriptions: &mut [Option<FileDescription>],
    force: bool,
    scan: Option<&ScanOptions>,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<ScanOutcome> {
    // Read file contents before taking the write lock. Savepoints isolate bad files.
    const WRITE_BATCH_SIZE: usize = 16;
    let mut outcome = ScanOutcome::default();
    let mut rejected = HashSet::new();
    for batch in descriptions.chunks_mut(WRITE_BATCH_SIZE) {
        let mut ready = Vec::new();
        for description in batch.iter_mut().flatten() {
            // Managed copies are verified by file actions and attached after download.
            // Unreferenced old versions must never become newly authored files.
            if Path::new(&description.directory).starts_with(MANAGED_AUDIO_DIRECTORY) {
                if let Err(error) =
                    super::file::index_cached_download(main_db, store, fsio, description).await
                {
                    warn!(
                        "Cannot index cached download {}: {error:#}",
                        description.file_name
                    );
                }
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
            let mut existing = media_files::Entity::find()
                .filter(media_files::Column::Directory.eq(&description.directory))
                .filter(media_files::Column::FileName.eq(&description.file_name))
                .one(main_db)
                .await?;
            let resolved = match &existing {
                Some(file) => resolved_metadata::contains(main_db, file.id).await?,
                None => false,
            };
            if !force
                && resolved
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
                // Keep the accepted description so a single-file scan can revisit its CUE.
                continue;
            }

            let fs = fsio.clone();
            let mut prepared = description.clone();
            let old_hash = existing.as_ref().and_then(|file| file.content_hash.clone());
            let prepared = task::spawn_blocking(move || -> Result<_> {
                prepared.get_crc(&fs)?;
                let mut stream = fs.open(&prepared.raw_node.path, OpenMode::Read)?;
                let mut hasher = Hasher::new();
                let size = io::copy(&mut stream, &mut hasher)?;
                let size = i64::try_from(size).context("File size exceeds supported range")?;
                let digest = hasher.finalize().to_hex().to_string();
                let metadata = if !force && resolved && old_hash.as_ref() == Some(&digest) {
                    None
                } else {
                    let root = prepared
                        .lib_path
                        .as_deref()
                        .unwrap_or_else(|| Path::new(""));
                    let resolved = read_local(&fs, &prepared.raw_node, root)?;
                    let metadata =
                        metadata::reader::legacy_tags(&resolved.diagnostics.raw_tags, None);
                    prepared.set_codec_information(
                        resolved
                            .audio
                            .sample_rate
                            .context("Audio stream has no sample rate")?,
                        resolved.audio.base.duration_ms.unwrap_or_default() as f64 / 1000.0,
                    );
                    Some((
                        FileMetadata {
                            path: prepared.raw_node.path.clone(),
                            metadata,
                        },
                        resolved,
                    ))
                };
                Ok((prepared, metadata, digest, size))
            })
            .await
            .context("Metadata worker failed")?;
            let (prepared, mut metadata, digest, size) = match prepared {
                Ok(value) => value,
                Err(e) => {
                    error!("Failed to prepare {}: {e:#}", description.file_name);
                    outcome.failed += 1;
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
                outcome.failed += 1;
                rejected.insert((description.directory.clone(), description.file_name.clone()));
                continue;
            }
            if let Some((_, resolved)) = &mut metadata {
                resolved.discover_resources(fsio).await;
            }
            let mut moved_from = None;
            let release_track = metadata.as_ref().and_then(|(_, resolved)| {
                resolved_metadata::external_uuid(&resolved.audio.base, "musicbrainz:release-track")
            });
            if existing.is_none()
                && let Some((moved, previous_album)) =
                    find_moved_file(fsio, main_db, &prepared, &digest, size, release_track).await?
            {
                existing = Some(moved);
                moved_from = previous_album;
            }
            *description = prepared;

            ready.push((description, existing, metadata, digest, size, moved_from));
        }
        if ready.is_empty() {
            continue;
        }
        let batch_txn = main_db.begin().await?;
        store.lock(&batch_txn).await?;
        for (description, existing, metadata, digest, size, moved_from) in ready {
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
            let mut current = media_files::Entity::find()
                .filter(media_files::Column::Directory.eq(&description.directory))
                .filter(media_files::Column::FileName.eq(&description.file_name))
                .one(&batch_txn)
                .await?;
            if current.is_none()
                && let Some(old) = &existing
                && (old.directory != description.directory
                    || old.file_name != description.file_name)
            {
                // A detached remote row has no former location to check.
                let detached =
                    !old.local_available && old.directory.is_empty() && old.file_name.is_empty();
                let still_absent = description.lib_path.as_deref().is_some_and(|root| {
                    matches!(
                        fsio.exists(&root.join(&old.directory).join(&old.file_name)),
                        Ok(false)
                    )
                });
                if detached || still_absent {
                    current = media_files::Entity::find_by_id(old.id)
                        .one(&batch_txn)
                        .await?;
                }
            }
            if current != existing {
                error!(
                    "File changed during metadata scan: {}",
                    description.file_name
                );
                outcome.failed += 1;
                rejected.insert((description.directory.clone(), description.file_name.clone()));
                continue;
            }
            let txn = batch_txn.begin().await?;
            let result: Result<()> = async {
                let content_changed = existing.as_ref().is_some_and(|file| {
                    file.content_hash
                        .as_deref()
                        .is_some_and(|old_hash| old_hash != digest.as_str())
                        || file.content_size.is_some_and(|old_size| old_size != size)
                });
                match (&existing, &metadata) {
                    (Some(file), Some((metadata, _))) => {
                        update_file_codec_information(fsio, &txn, file, description).await?;
                        update_file_metadata(fsio, &txn, node_id, file, description, metadata)
                            .await?;
                        unlink_cover_art(&txn, file).await?;
                    }
                    (Some(file), None) => update_last_modified(&txn, file, description).await?,
                    (None, Some((metadata, _))) => {
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

                let id = author_track(&txn, store, &description.directory, &description.file_name)
                    .await?;
                if let Some((_, resolved)) = &metadata {
                    let file = media_files::Entity::find_by_id(id)
                        .one(&txn)
                        .await?
                        .context("scanned file disappeared")?;
                    resolved_metadata::store(&txn, &file, resolved, moved_from.as_deref()).await?;
                }

                // Analysis describes the bytes it read, so a changed source
                // invalidates it before the next analysis run.
                if content_changed {
                    media_analysis::Entity::delete_many()
                        .filter(media_analysis::Column::FileId.eq(id))
                        .exec(&txn)
                        .await?;
                }

                if let Some(options) = scan {
                    anyhow::ensure!(
                        resolved_metadata::portable::publish_track(
                            &txn,
                            id,
                            options.cancel_token.as_ref(),
                            store,
                            metadata.as_ref().map(|(_, resolved)| resolved),
                        )
                        .await
                        .context("Unable to index file")?,
                        "Missing metadata summary for file {id}"
                    );
                }
                Ok(())
            }
            .await;
            match result {
                Ok(()) => {
                    txn.commit().await?;
                    outcome.saved += 1;
                }
                Err(e) => {
                    txn.rollback().await.with_context(|| {
                        format!("Failed to roll back {} after {e:#}", description.file_name)
                    })?;
                    if scan
                        .and_then(|options| options.cancel_token.as_ref())
                        .is_some_and(|token| token.is_cancelled())
                    {
                        batch_txn
                            .rollback()
                            .await
                            .context("Failed to roll back cancelled metadata batch")?;
                        return Ok(outcome);
                    }
                    error!("Failed to save {}: {e:#}", description.file_name);
                    if is_fatal_scan_error(&e) {
                        batch_txn.rollback().await.with_context(|| {
                            format!("Failed to roll back metadata batch after {e:#}")
                        })?;
                        return Err(e);
                    }
                    outcome.failed += 1;
                    rejected.insert((description.directory.clone(), description.file_name.clone()));
                }
            }
        }
        resolved_metadata::reconcile_recordings(&batch_txn).await?;
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
    Ok(outcome)
}

async fn find_moved_file(
    fsio: &FsIo,
    db: &DatabaseConnection,
    description: &FileDescription,
    digest: &str,
    size: i64,
    release_track: Option<Uuid>,
) -> Result<Option<(media_files::Model, Option<String>)>> {
    use sea_orm::QuerySelect;

    let Some(root) = description.lib_path.as_deref() else {
        return Ok(None);
    };
    let mut candidates = media_files::Entity::find()
        .filter(media_files::Column::ContentHash.eq(digest))
        .filter(media_files::Column::ContentSize.eq(size))
        .filter(
            Condition::any()
                .add(media_files::Column::LocalAvailable.eq(true))
                .add(
                    Condition::all()
                        .add(media_files::Column::Directory.eq(""))
                        .add(media_files::Column::FileName.eq("")),
                ),
        )
        .limit(2)
        .all(db)
        .await?;

    // An exact edition-track identifier can follow the file through a tag rewrite.
    if candidates.is_empty()
        && let Some(id) = release_track
    {
        let tagged_files = Query::select()
            .column(catalog_audio::Column::FileId)
            .from(catalog_audio::Entity)
            .and_where(catalog_audio::Column::ReleaseTrackMbid.eq(id.to_string()))
            .to_owned();
        candidates = media_files::Entity::find()
            .filter(Expr::col(media_files::Column::Id.as_column_ref()).in_subquery(tagged_files))
            .filter(media_files::Column::LocalAvailable.eq(true))
            .limit(2)
            .all(db)
            .await?;
    }
    let [old] = candidates.as_slice() else {
        return Ok(None);
    };

    // A unique exact match can attach a local copy to a synced track with no assigned path.
    if !old.local_available && old.directory.is_empty() && old.file_name.is_empty() {
        return Ok(Some((old.clone(), None)));
    }
    // A live original makes the new file a copy. Check the old location outside the transaction.
    if is_managed_directory(&old.directory) || Path::new(&old.directory).is_absolute() {
        return Ok(None);
    }
    let old_path = root.join(&old.directory).join(&old.file_name);
    let album_dir = resolved_metadata::track_album_directory(db, old.id)
        .await?
        .filter(|path| Path::new(path).starts_with(root));
    let fsio = fsio.clone();
    let (absent, absent_album) = task::spawn_blocking(move || {
        let absent = matches!(fsio.exists(&old_path), Ok(false));
        let absent_album =
            album_dir.filter(|path| absent && matches!(fsio.exists(Path::new(path)), Ok(false)));
        (absent, absent_album)
    })
    .await?;
    Ok(absent.then(|| (old.clone(), absent_album)))
}

async fn author_track(
    txn: &sea_orm::DatabaseTransaction,
    store: &crate::sync::SeaOrmSyncStore,
    directory: &str,
    file_name: &str,
) -> Result<i32> {
    let file = media_files::Entity::find()
        .filter(media_files::Column::Directory.eq(directory))
        .filter(media_files::Column::FileName.eq(file_name))
        .one(txn)
        .await?
        .context("scanned file missing")?;
    let id = file.id;
    author_track_row(txn, store, file).await?;
    Ok(id)
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
    active_model.directory = ActiveValue::Set(description.directory.clone());
    active_model.file_name = ActiveValue::Set(description.file_name.clone());
    active_model.extension = ActiveValue::Set(description.extension.clone());

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
    type Tags = OwnedListPolicy<source_tags::Entity, MetadataMap>;
    let owner = [file_id.into()];
    let previous = Tags::read_on(tx, &owner, Tags::read_ceiling()).await?;
    let mut used = HashSet::new();
    let mut compatible: Vec<_> = tags
        .iter()
        .filter(|(key, value)| key.len() <= 256 && value.len() <= 16384)
        .collect();
    if compatible.len() > 128 {
        // The rich catalog keeps additional fields beyond the sync list budget.
        compatible.sort_by_key(|(key, _)| {
            !matches!(
                key.as_str(),
                "track_title"
                    | "album"
                    | "artist"
                    | "album_artist"
                    | "genre"
                    | "track_number"
                    | "disc_number"
            )
        });
        compatible.truncate(128);
    }
    let incoming = compatible
        .into_iter()
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

#[derive(Debug, thiserror::Error)]
#[error(
    "The library folder appears empty. Missing tracks were kept. Reconnect the folder and scan again."
)]
pub struct EmptyLibraryCleanupDeferred;

pub struct EmptyLibraryCleanupPreview {
    pub count: usize,
    pub fingerprint: String,
}

fn root_has_only_index_entries(entries: &[FsNode]) -> bool {
    entries
        .iter()
        .all(|entry| entry.is_dir && entry.filename == RUNE_INDEX_DIRECTORY)
}

async fn empty_local_root(fsio: &FsIo, root_path: &Path) -> bool {
    fsio.storage() == StorageLocation::Local
        && fsio
            .read_dir(root_path)
            .await
            .is_ok_and(|entries| root_has_only_index_entries(&entries))
}

async fn empty_library_cleanup_candidates(
    fsio: &FsIo,
    db: &impl ConnectionTrait,
    root_path: &Path,
) -> Result<Option<Vec<media_files::Model>>> {
    if !empty_local_root(fsio, root_path).await {
        return Ok(None);
    }
    let mut tracks = media_files::Entity::find()
        .filter(media_files::Column::LocalAvailable.eq(true))
        .all(db)
        .await?;
    tracks.retain(|track| !is_managed_directory(&track.directory));
    tracks.sort_unstable_by_key(|track| track.id);
    let fsio = fsio.clone();
    let root_path = root_path.to_path_buf();
    task::spawn_blocking(move || {
        let missing = tracks.iter().all(|track| {
            matches!(
                fsio.exists(&root_path.join(&track.directory).join(&track.file_name)),
                Ok(false)
            )
        });
        Ok((missing && !tracks.is_empty()).then_some(tracks))
    })
    .await?
}

fn empty_library_cleanup_fingerprint(tracks: &[media_files::Model]) -> String {
    let mut hasher = Hasher::new();
    for track in tracks {
        hasher.update(&track.id.to_le_bytes());
        for value in [&track.source_uuid, &track.directory, &track.file_name] {
            hasher.update(&(value.len() as u64).to_le_bytes());
            hasher.update(value.as_bytes());
        }
    }
    hasher.finalize().to_hex().to_string()
}

pub async fn preview_empty_library_cleanup(
    fsio: &FsIo,
    db: &DatabaseConnection,
    root_path: &Path,
) -> Result<Option<EmptyLibraryCleanupPreview>> {
    Ok(empty_library_cleanup_candidates(fsio, db, root_path)
        .await?
        .map(|tracks| EmptyLibraryCleanupPreview {
            count: tracks.len(),
            fingerprint: empty_library_cleanup_fingerprint(&tracks),
        }))
}

pub async fn confirm_empty_library_cleanup(
    fsio: &FsIo,
    db: &DatabaseConnection,
    root_path: &Path,
    store: &crate::sync::SeaOrmSyncStore,
    fingerprint: &str,
    still_current: impl Fn() -> bool,
    begin_commit: impl FnOnce() -> bool,
) -> Result<bool> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let Some(tracks) = empty_library_cleanup_candidates(fsio, &tx, root_path).await? else {
        tx.rollback().await?;
        return Ok(false);
    };
    if empty_library_cleanup_fingerprint(&tracks) != fingerprint || !still_current() {
        tx.rollback().await?;
        return Ok(false);
    }
    for track in tracks {
        remove_track(&tx, store, track.id).await?;
    }
    if !empty_local_root(fsio, root_path).await || !still_current() || !begin_commit() {
        tx.rollback().await?;
        return Ok(false);
    }
    tx.commit().await?;
    Ok(true)
}

async fn clean_up_database(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    root_path: &Path,
    policy: MissingHostFilePolicy,
    store: &crate::sync::SeaOrmSyncStore,
    cancel_token: Option<&CancellationToken>,
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
    let cancel_missing = cancel_token.cloned();
    let missing: Vec<_> = task::spawn_blocking(move || {
        files
            .into_iter()
            .take_while(|_| {
                !cancel_missing
                    .as_ref()
                    .is_some_and(CancellationToken::is_cancelled)
            })
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
    if cancel_token.is_some_and(CancellationToken::is_cancelled) {
        return Ok(());
    }
    let host_missing = missing
        .iter()
        .filter(|file| !is_managed_directory(&file.directory))
        .count();
    let mass_missing = host_missing > 5 && host_missing * 2 > host_total;
    if mass_missing {
        log::warn!("Skipping missing host file removals because the library looks unmounted");
    }
    let root_listing = fsio.read_dir(root_path).await;
    let root_listable = root_listing.is_ok();
    let empty_root = root_listing.is_ok_and(|entries| root_has_only_index_entries(&entries));
    let defer_host_cleanup = empty_root
        && fsio.storage() == StorageLocation::Local
        && host_missing > 0
        && policy == MissingHostFilePolicy::RemoveEverywhere;
    for file in missing {
        if cancel_token.is_some_and(CancellationToken::is_cancelled) {
            break;
        }
        let managed = is_managed_directory(&file.directory);
        if defer_host_cleanup && !managed {
            continue;
        }
        let result: Result<()> = async {
            if !managed
                && policy == MissingHostFilePolicy::RemoveEverywhere
                && root_listable
                && !mass_missing
            {
                let tx = main_db.begin().await?;
                let removal: Result<bool> = async {
                    store.lock(&tx).await?;
                    let current = media_files::Entity::find_by_id(file.id).one(&tx).await?;
                    let unchanged = current.as_ref().is_some_and(|row| {
                        row.local_available
                            && row.directory == file.directory
                            && row.file_name == file.file_name
                    });
                    if !unchanged || cancel_token.is_some_and(CancellationToken::is_cancelled) {
                        return Ok(false);
                    }
                    remove_track(&tx, store, file.id).await?;

                    // A file can return while cleanup waits for the database. Confirm
                    // absence before the catalog and sync removals become visible.
                    let fsio = fsio.clone();
                    let path = root_path.join(&file.directory).join(&file.file_name);
                    let still_missing =
                        task::spawn_blocking(move || matches!(fsio.exists(&path), Ok(false)))
                            .await?;
                    Ok(still_missing && !cancel_token.is_some_and(CancellationToken::is_cancelled))
                }
                .await;
                match removal {
                    Ok(true) => tx.commit().await?,
                    Ok(false) => tx.rollback().await?,
                    Err(error) => {
                        if let Err(rollback_error) = tx.rollback().await {
                            return Err(error.context(format!(
                                "Failed to roll back cleanup of missing file {}: {rollback_error:#}",
                                file.id
                            )));
                        }
                        return Err(error);
                    }
                }
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
            let detail = format!("Failed to clean up missing file {}: {error:#}", file.id);
            error!("{detail}");
            if let Err(log_error) = insert_log(
                main_db,
                LogLevel::Warning,
                "actions::metadata::clean_up_database".to_string(),
                detail,
            )
            .await
            {
                error!(
                    "Failed to log cleanup failure for missing file {}: {log_error:#}",
                    file.id
                );
            }
        }
    }
    if defer_host_cleanup && !cancel_token.is_some_and(CancellationToken::is_cancelled) {
        return Err(EmptyLibraryCleanupDeferred.into());
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
    let description = task::spawn_blocking(move || {
        let node = fsio_for_describe.canonicalize(&path_for_describe)?;
        // A local file may have been reached through another spelling of the
        // root (for example an 8.3 path). Keep already-matching backend paths.
        let root_for_describe = if node.path.starts_with(&root_for_describe) {
            root_for_describe
        } else {
            fsio_for_describe.canonicalize_path(&root_for_describe)?
        };
        let description = describe_file(&fsio_for_describe, &node, &Some(root_for_describe))?;
        Ok::<_, anyhow::Error>(description)
    })
    .await??;
    let mut descriptions = vec![Some(description)];
    // Single-file scans publish the same catalog records inside the scan transaction.
    let options = ScanOptions {
        cleanup: false,
        force: false,
        missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
        cancel_token: None,
    };
    sync_file_descriptions_inner(
        fsio,
        main_db,
        node_id,
        &mut descriptions,
        false,
        Some(&options),
        store,
    )
    .await?;
    let ids = get_file_ids_by_descriptions(main_db, &descriptions).await?;
    let id = *ids.first().context("Audio file was not indexed")?;
    if let Some(description) = descriptions.first().and_then(Option::as_ref) {
        let root = description.lib_path.as_deref().unwrap_or(lib_path);
        let directory = &description.directory;
        let layouts = async {
            let paths = fsio
                .read_dir(&root.join(directory))
                .await?
                .into_iter()
                .filter(::metadata::scanner::is_cue_file)
                .map(|node| node.path)
                .collect();
            resolved_metadata::cue::reconcile_directory(fsio, main_db, store, directory, paths)
                .await
        };
        if let Err(error) = layouts.await {
            warn!("Cannot index CUE layouts in {directory:?}: {error:#}");
        }
    }
    resolved_metadata::reconcile_releases(main_db, store, None).await?;
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
        ref cancel_token,
    } = options;
    let fsio_for_scan = fsio.clone();
    let root_for_scan = lib_path.to_path_buf();
    // A failed walk reports none of the paths it skipped, so cleanup waits for a
    // later scan, as it does when the walk skips paths.
    let mut scanner =
        task::spawn_blocking(move || AudioScanner::new(&fsio_for_scan, &root_for_scan)).await??;

    for path in scanner.skipped() {
        let detail = format!("Skipped {path:?}: its path is not valid Unicode");
        warn!("{detail}");
        if let Err(error) = insert_log(
            main_db,
            LogLevel::Warning,
            "actions::metadata::scan_audio_library".to_string(),
            detail,
        )
        .await
        {
            error!("Failed to log a skipped path: {error:#}");
        }
    }
    // Cleanup cannot tell a track under a skipped path from a deleted one, so it
    // waits for a walk that skips nothing and leaves the rows as they are until
    // then. Marking them unavailable instead would hide them from later
    // cleanups, which only check available tracks.
    let walk_complete = scanner.skipped().is_empty();
    if cleanup && !walk_complete {
        warn!("Leaving cleanup for a scan that can read every path");
    }

    info!("Starting audio library scan with last-modified pre-filtering");

    let mut processed_files = 0;
    let mut total_files_scanned = 0;
    let mut skipped_files = 0;
    let mut scanned_directories = HashSet::new();

    // Read audio files in larger batches for better performance
    const BATCH_SIZE: usize = 48;

    while !scanner.has_ended() {
        // Check if the cancellation token has been triggered
        if let Some(token) = cancel_token
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
            let description = task::spawn_blocking(move || {
                describe_file(&fsio_for_describe, &file, &Some(root_for_describe))
            })
            .await?;
            match description {
                Ok(value) => {
                    scanned_directories.insert(value.directory.clone());
                    descriptions.push(Some(value));
                }
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

            // Batch query the stored change tokens of existing files
            match get_existing_files_modified(main_db, file_keys).await {
                Ok(existing_change_tokens) => {
                    // Filter out unchanged files
                    let (filtered_descriptions, skipped_count) =
                        filter_modified_files(descriptions, &existing_change_tokens);

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

        let outcome = sync_file_descriptions_inner(
            fsio,
            main_db,
            node_id,
            &mut descriptions,
            force,
            Some(&options),
            store,
        )
        .await
        .context("Unable to describe files")?;
        if outcome.failed > 0 {
            info!("Failed to scan {} files in this batch", outcome.failed);
        }
        debug!("Finished one batch with {} saved files", outcome.saved);

        if cancel_token
            .as_ref()
            .is_some_and(|token| token.is_cancelled())
        {
            return Ok(processed_files);
        }
        // Update the number of processed files
        processed_files += outcome.saved;

        // Call the progress callback with total files scanned
        progress_callback(total_files_scanned);
    }

    if skipped_files > 0 {
        info!("Pre-filtering skipped {} unchanged files", skipped_files);
    }

    if cleanup
        && walk_complete
        && !cancel_token
            .as_ref()
            .is_some_and(|token| token.is_cancelled())
    {
        info!("Starting cleanup process.");
        match clean_up_database(
            fsio,
            main_db,
            lib_path,
            policy,
            store,
            cancel_token.as_ref(),
        )
        .await
        .with_context(|| "Unable to cleanup database")
        {
            Ok(_) => info!("Cleanup completed successfully."),
            Err(e) if e.is::<EmptyLibraryCleanupDeferred>() => return Err(e),
            Err(e) => error!("{e:#?}"),
        }
    }

    // Revisit sidecars even if only the CUE changed and every audio file was unchanged.
    if walk_complete {
        resolved_metadata::cue::reconcile_scan(
            fsio,
            main_db,
            store,
            lib_path,
            &scanned_directories,
            scanner.take_cue_files(),
            cancel_token.as_ref(),
        )
        .await?;
    }
    // Local sidecars can change while audio remains unchanged. Remote normal scans
    // keep the audio-based skip; a Force Scan refreshes their covers on demand.
    if walk_complete && !force && fsio.storage() == StorageLocation::Local {
        super::cover_art::refresh_checked_external_artwork(
            fsio,
            main_db,
            store,
            lib_path,
            scanned_directories,
            scanner.take_external_cover_directories(),
            cancel_token.as_ref(),
        )
        .await?;
    }
    // Reconcile even when a previous scan was interrupted after committing its files.
    resolved_metadata::reconcile_releases(main_db, store, cancel_token.as_ref()).await?;
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
    use sea_orm::{QueryOrder, QuerySelect};
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

    // Join summaries by the catalog file identity instead of reading the source-tag projection.
    use catalog_audio::Column as Audio;
    let order: HashMap<i32, (Option<i32>, Option<i32>)> = catalog_audio::Entity::find()
        .select_only()
        .columns([Audio::FileId, Audio::DiscNumber, Audio::TrackNumber])
        .filter(Audio::FileId.is_in(file_ids.iter().copied()))
        .into_tuple::<(i32, Option<i32>, Option<i32>)>()
        .all(db)
        .await?
        .into_iter()
        .map(|(file, disc, track)| (file, (disc, track)))
        .collect();
    let titles = resolved_metadata::summary::track_titles(db, &file_ids).await?;

    // Display the original primary credit spellings and the resolved list of genres.
    use catalog_credits::Column as Credit;
    let artist_rows = catalog_credits::Entity::find()
        .select_only()
        .columns([Credit::FileId, Credit::CreditedAs])
        .filter(Credit::FileId.is_in(file_ids.iter().copied()))
        .filter(Credit::Scope.eq(resolved_metadata::CreditScope::Audio as i32))
        .filter(Credit::Role.eq("Primary Artist"))
        .order_by_asc(Credit::Position)
        .into_tuple::<(i32, String)>()
        .all(db)
        .await?;
    let artist_names = names_by_file(artist_rows);

    use catalog_values::Column as Value;
    let genre_rows = catalog_values::Entity::find()
        .select_only()
        .columns([Value::FileId, Value::Value])
        .filter(Value::FileId.is_in(file_ids.iter().copied()))
        .filter(Value::Scope.eq(resolved_metadata::CreditScope::Audio as i32))
        .filter(Value::Field.eq("genres"))
        .order_by_asc(Value::Position)
        .into_tuple::<(i32, String)>()
        .all(db)
        .await?;
    let genres = names_by_file(genre_rows);

    // Keep file paths, identity and download state on their owning file rows.
    let mut results = Vec::with_capacity(files.len());
    for file in files {
        let file_id = file.id;
        let (title, album) = titles
            .get(&file_id)
            .map(|(title, album)| (Some(title), album.as_ref()))
            .unwrap_or_default();
        let (disc, track) = order.get(&file_id).copied().unwrap_or_default();
        let track_number =
            track.and_then(|track| disc.unwrap_or(0).checked_mul(1000)?.checked_add(track));

        let cover_art_id = file
            .cover_art_uuid
            .as_ref()
            .and_then(|id| artwork_ids.get(id))
            .copied();

        let summary = MetadataSummary {
            id: file_id,
            directory: file.directory.clone(),
            file_name: file.file_name.clone(),
            artist: artist_names
                .get(&file_id)
                .map_or_else(String::new, |names| names.join(", ")),
            album: album.cloned().unwrap_or_default(),
            genre: genres
                .get(&file_id)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .join("\\\\")
                .to_uppercase(),
            title: title
                .filter(|title| !title.is_empty())
                .cloned()
                .unwrap_or_else(|| file.file_name.clone()),
            track_number,
            duration: file
                .duration
                .to_f64()
                .expect("Unable to convert track duration"),
            cover_art_id,
            file_hash: file.file_hash.clone(),
            file_uuid: file.source_uuid.clone(),
        };

        results.push(summary);
    }

    Ok(results)
}

fn names_by_file(rows: Vec<(i32, String)>) -> HashMap<i32, Vec<String>> {
    let mut names = HashMap::<_, Vec<_>>::new();
    for (file, name) in rows {
        let names_for_file = names.entry(file).or_default();
        if !names_for_file.contains(&name) {
            names_for_file.push(name);
        }
    }
    names
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
) -> Result<(
    MetadataSummary,
    Vec<portable_artists::Model>,
    Option<portable_albums::Model>,
)> {
    let file = get_metadata_summary_by_file_id(db, file_id).await?;

    // Credits preserve tag spellings while artist navigation follows confirmed identities.
    use crate::entities::audio_artists as links;
    let targets = Query::select()
        .column(links::Column::ArtistUuid)
        .from(links::Entity)
        .and_where(links::Column::TrackUuid.eq(&file.file_uuid))
        .to_owned();
    let artists = portable_artists::Entity::find()
        .filter(
            Expr::col(portable_artists::Column::SourceUuid.as_column_ref()).in_subquery(targets),
        )
        .all(db)
        .await?;

    let album = super::browse::album_for_file(db, &file.file_uuid).await?;
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
