//! Read-only local access state for catalog files.
use crate::entities::{media_files, sync_download_jobs};
use anyhow::{Result, ensure};
use fsio::FsIo;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use std::path::Component;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioAvailability {
    Local,
    RemoteOnly,
    Unavailable,
    ImportPending,
}

fn row_path(fsio: &FsIo, root: &Path, row: &media_files::Model) -> Result<Option<PathBuf>> {
    if !row.local_available || row.file_name.is_empty() {
        return Ok(None);
    }
    let relative = PathBuf::from(&row.directory).join(&row.file_name);
    ensure!(
        relative
            .components()
            .all(|c| matches!(c, Component::Normal(_))),
        "invalid local path"
    );
    #[cfg(not(target_os = "android"))]
    let managed = relative.starts_with(super::content::MANAGED_AUDIO_DIRECTORY);
    let path = root.join(relative);
    #[cfg(not(target_os = "android"))]
    {
        let Ok(canonical) = fsio.canonicalize_path(&path) else {
            return Ok(None);
        };
        ensure!(
            !managed || canonical.starts_with(fsio.canonicalize_path(root)?),
            "local path escapes library root"
        );
    }
    if !fsio.exists(&path)? || !fsio.canonicalize(&path)?.is_file {
        return Ok(None);
    }
    Ok(Some(path))
}

pub async fn materialized_path<C: ConnectionTrait>(
    db: &C,
    fsio: &FsIo,
    root: &Path,
    id: i32,
) -> Result<Option<PathBuf>> {
    match media_files::Entity::find_by_id(id).one(db).await? {
        Some(row) => row_path(fsio, root, &row),
        None => Ok(None),
    }
}

pub async fn audio_availability<C: ConnectionTrait>(
    db: &C,
    fsio: &FsIo,
    root: &Path,
    ids: &[i32],
) -> Result<HashMap<i32, AudioAvailability>> {
    let rows = media_files::Entity::find()
        .filter(media_files::Column::Id.is_in(ids.iter().copied()))
        .all(db)
        .await?;
    let uuids: Vec<_> = rows.iter().map(|row| row.hlc_uuid.clone()).collect();
    let pending: HashSet<_> = sync_download_jobs::Entity::find()
        .filter(sync_download_jobs::Column::FileUuid.is_in(uuids))
        .all(db)
        .await?
        .into_iter()
        .map(|job| job.file_uuid)
        .collect();
    rows.into_iter()
        .map(|row| {
            let state = if row_path(fsio, root, &row)?.is_some() {
                AudioAvailability::Local
            } else if pending.contains(&row.hlc_uuid) {
                AudioAvailability::ImportPending
            } else if row.content_hash.is_some() && row.content_size.is_some() {
                AudioAvailability::RemoteOnly
            } else {
                AudioAvailability::Unavailable
            };
            Ok((row.id, state))
        })
        .collect()
}

pub async fn is_materialized<C: ConnectionTrait>(
    db: &C,
    fsio: &FsIo,
    root: &Path,
    id: i32,
) -> Result<bool> {
    Ok(materialized_path(db, fsio, root, id).await?.is_some())
}

pub async fn resolve_local_id<C: ConnectionTrait>(db: &C, id: i32) -> Result<Option<i32>> {
    Ok(media_files::Entity::find_by_id(id)
        .one(db)
        .await?
        .map(|row| row.id))
}

pub async fn remote_required_ids<C: ConnectionTrait>(
    db: &C,
    fsio: &FsIo,
    root: &Path,
    ids: &[i32],
) -> Result<HashSet<i32>> {
    Ok(audio_availability(db, fsio, root, ids)
        .await?
        .into_iter()
        .filter_map(|(id, availability)| (availability != AudioAvailability::Local).then_some(id))
        .collect())
}
