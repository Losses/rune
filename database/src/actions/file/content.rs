//! Verified, bounded transfer of audio blobs.
use crate::sync::model::BlobRef;
use anyhow::{Context, Result, ensure};
use fsio::FsIo;
use std::{
    io::Read,
    path::{Path, PathBuf},
};
use uuid::Uuid;

/// Maximum accepted audio object size (8 GiB).
pub const MAX_BLOB_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// Reserved application-managed content; ordinary library scans do not author it.
pub const MANAGED_AUDIO_DIRECTORY: &str = "Rune Sync";

/// Validate the full lowercase BLAKE3 digest used to identify audio.
pub fn validate_digest(digest: &str) -> Result<()> {
    ensure!(
        digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid blob digest"
    );
    Ok(())
}

/// Validate a filename extension supplied by a peer.
pub fn validate_extension(extension: &str) -> Result<()> {
    ensure!(
        !extension.is_empty()
            && extension.len() <= 12
            && extension
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()),
        "invalid audio extension"
    );
    Ok(())
}

/// Compute a full digest without loading the file into memory.
pub fn hash_file(fsio: &FsIo, path: &Path) -> Result<BlobRef> {
    let mut input = fsio.open(path, "r")?;
    let mut hasher = blake3::Hasher::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size = size.checked_add(n as u64).context("blob size overflow")?;
        ensure!(size <= MAX_BLOB_BYTES, "blob too large");
        hasher.update(&buffer[..n]);
    }
    Ok(BlobRef {
        digest: hasher.finalize().to_hex().to_string(),
        size,
    })
}

/// Return the local download path after validating all remote components.
pub fn blob_path(fsio: &FsIo, root: &Path, digest: &str, extension: &str) -> Result<PathBuf> {
    validate_digest(digest)?;
    validate_extension(extension)?;
    ensure!(
        !root
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)),
        "invalid library root"
    );
    let dir = root.join(MANAGED_AUDIO_DIRECTORY);
    fsio.create_dir_all(&dir)?;
    #[cfg(not(target_os = "android"))]
    {
        let canonical_root = fsio
            .canonicalize_path(root)
            .context("invalid library root")?;
        let canonical_dir = fsio.canonicalize_path(&dir)?;
        ensure!(
            canonical_dir.starts_with(&canonical_root),
            "download directory escapes library root"
        );
        Ok(canonical_dir.join(format!("{digest}.{extension}")))
    }
    #[cfg(target_os = "android")]
    Ok(dir.join(format!("{digest}.{extension}")))
}

/// Return a distinct managed path for a track even when other tracks share its content.
pub fn blob_path_for_track(
    fsio: &FsIo,
    root: &Path,
    track_id: Uuid,
    digest: &str,
    extension: &str,
) -> Result<PathBuf> {
    let base = blob_path(fsio, root, digest, extension)?;
    let directory = base
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing download directory"))?
        .join(track_id.to_string());
    fsio.create_dir_all(&directory)?;
    #[cfg(not(target_os = "android"))]
    {
        let parent = fsio.canonicalize_path(base.parent().unwrap())?;
        let canonical = fsio.canonicalize_path(&directory)?;
        ensure!(
            canonical.starts_with(parent),
            "track directory escapes library root"
        );
        Ok(canonical.join(format!("{digest}.{extension}")))
    }
    #[cfg(target_os = "android")]
    Ok(directory.join(format!("{digest}.{extension}")))
}

use super::access::materialized_path;
use crate::{
    entities::{media_files, sync_download_jobs},
    sync::SeaOrmSyncStore,
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait};

pub async fn catalog_track(
    db: &DatabaseConnection,
    local_file_id: i32,
) -> Result<Option<(Uuid, BlobRef, String)>> {
    let Some(file) = media_files::Entity::find_by_id(local_file_id)
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    Ok(match (file.content_hash, file.content_size) {
        (Some(digest), Some(size)) => {
            validate_digest(&digest)?;
            Some((
                file.hlc_uuid.parse()?,
                BlobRef {
                    digest,
                    size: u64::try_from(size)?,
                },
                file.extension,
            ))
        }
        _ => None,
    })
}

/// Identify rows that carry a valid portable audio content identity.
pub async fn remote_content_ids(
    db: &DatabaseConnection,
    ids: &[i32],
) -> Result<std::collections::HashSet<i32>> {
    let mut found = std::collections::HashSet::new();
    for chunk in ids.chunks(512) {
        let rows = media_files::Entity::find()
            .filter(media_files::Column::Id.is_in(chunk.iter().copied()))
            .filter(media_files::Column::ContentHash.is_not_null())
            .filter(media_files::Column::ContentSize.is_not_null())
            .all(db)
            .await?;
        found.extend(rows.into_iter().filter_map(|row| {
            match (row.content_hash.as_deref(), row.content_size) {
                (Some(digest), Some(size)) if size >= 0 && validate_digest(digest).is_ok() => {
                    Some(row.id)
                }
                _ => None,
            }
        }));
    }
    Ok(found)
}

pub async fn local_audio_path(
    db: &DatabaseConnection,
    fsio: &FsIo,
    root: &Path,
    id: Uuid,
    blob: &BlobRef,
) -> Result<Option<PathBuf>> {
    validate_digest(&blob.digest)?;
    let Some(file) = media_files::Entity::find()
        .filter(media_files::Column::HlcUuid.eq(id.to_string()))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    if file.content_hash.as_deref() != Some(&blob.digest)
        || file.content_size != Some(i64::try_from(blob.size)?)
    {
        return Ok(None);
    }
    let Some(path) = materialized_path(db, fsio, root, file.id).await? else {
        return Ok(None);
    };
    if hash_file(fsio, &path).ok().as_ref() == Some(blob) {
        Ok(Some(path))
    } else {
        Ok(None)
    }
}

pub async fn cached_blob_path(
    db: &DatabaseConnection,
    fsio: &FsIo,
    root: &Path,
    digest: &str,
) -> Result<Option<PathBuf>> {
    validate_digest(digest)?;
    for file in media_files::Entity::find()
        .filter(media_files::Column::ContentHash.eq(digest))
        .filter(media_files::Column::LocalAvailable.eq(true))
        .all(db)
        .await?
    {
        let Some(size) = file.content_size else {
            continue;
        };
        let blob = BlobRef {
            digest: digest.to_owned(),
            size: u64::try_from(size)?,
        };
        if let Some(path) = local_audio_path(db, fsio, root, file.hlc_uuid.parse()?, &blob).await? {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

pub async fn attach_download(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    fsio: &FsIo,
    root: &Path,
    track_id: Uuid,
    blob: &BlobRef,
    path: &Path,
) -> Result<()> {
    #[cfg(not(target_os = "android"))]
    {
        let canonical_root = fsio.canonicalize_path(root)?;
        let canonical = fsio.canonicalize_path(path)?;
        ensure!(
            canonical.starts_with(&canonical_root),
            "download escapes library root"
        );
    }
    ensure!(
        &hash_file(fsio, path)? == blob,
        "download digest or size mismatch"
    );
    let relative = path.strip_prefix(root)?;
    ensure!(
        relative
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_))),
        "invalid download path"
    );
    let directory = relative
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_string_lossy()
        .into_owned();
    let name = relative
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("missing filename"))?
        .to_string_lossy()
        .into_owned();
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    store.identity_on(&tx).await?;
    let row = media_files::Entity::find()
        .filter(media_files::Column::HlcUuid.eq(track_id.to_string()))
        .one(&tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("downloaded file no longer exists"))?;
    ensure!(
        row.content_hash.as_deref() == Some(&blob.digest)
            && row.content_size == Some(i64::try_from(blob.size)?),
        "downloaded file changed during transfer"
    );
    let job = sync_download_jobs::Entity::find()
        .filter(sync_download_jobs::Column::FileUuid.eq(track_id.to_string()))
        .filter(sync_download_jobs::Column::Digest.eq(&blob.digest))
        .one(&tx)
        .await?;
    let Some(job) = job else {
        ensure!(
            row.local_available && row.directory == directory && row.file_name == name,
            "missing download job"
        );
        tx.commit().await?;
        return Ok(());
    };
    ensure!(
        job.directory == directory
            && job.file_name == name
            && job.size == i64::try_from(blob.size)?,
        "download job mismatch"
    );
    media_files::Entity::update_many()
        .col_expr(
            media_files::Column::Directory,
            sea_orm::sea_query::Expr::value(directory),
        )
        .col_expr(
            media_files::Column::FileName,
            sea_orm::sea_query::Expr::value(name),
        )
        .col_expr(
            media_files::Column::LocalAvailable,
            sea_orm::sea_query::Expr::value(true),
        )
        .filter(media_files::Column::Id.eq(row.id))
        .exec(&tx)
        .await?;
    sync_download_jobs::Entity::delete_by_id(job.id)
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{Database, Set};

    #[tokio::test]
    async fn portable_identity_is_independent_of_local_availability() -> Result<()> {
        let _ = migration::initialize_node_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into());
        let db = Database::connect("sqlite::memory:").await?;
        Migrator::up(&db, None).await?;
        let digest = "a".repeat(64);
        for (id, local, hash, size) in [
            (1, true, Some(digest.clone()), Some(7)),
            (2, false, Some(digest.clone()), Some(7)),
            (3, false, Some("invalid".to_string()), Some(7)),
            (4, false, Some(digest.clone()), Some(-1)),
            (5, false, None, None),
        ] {
            media_files::Entity::insert(media_files::ActiveModel {
                id: Set(id),
                file_name: Set(format!("{id}.ogg")),
                directory: Set(String::new()),
                extension: Set("ogg".into()),
                file_hash: Set(String::new()),
                last_modified: Set(String::new()),
                sample_rate: Set(1),
                duration: Set(1.into()),
                hlc_uuid: Set(Uuid::new_v4().to_string()),
                local_available: Set(local),
                content_hash: Set(hash),
                content_size: Set(size),
                ..Default::default()
            })
            .exec(&db)
            .await?;
        }
        assert!(catalog_track(&db, 1).await?.is_some());
        assert!(catalog_track(&db, 2).await?.is_some());
        assert!(catalog_track(&db, 3).await.is_err());
        assert!(catalog_track(&db, 4).await.is_err());
        assert!(catalog_track(&db, 5).await?.is_none());
        assert_eq!(
            remote_content_ids(&db, &[1, 2, 3, 4, 5, 99]).await?,
            [1, 2].into_iter().collect()
        );
        Ok(())
    }
}
