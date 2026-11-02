use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use once_cell::sync::Lazy;
use sea_orm::{
    ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QuerySelect,
};
use tokio_util::sync::CancellationToken;

use ::fsio::FsIo;
use ::metadata::cover_art::{CoverArt, extract_cover_art_binary, get_primary_color};
use uuid::Uuid;

use crate::entities::{media_cover_art, media_files};

pub async fn get_magic_cover_art(
    main_db: &DatabaseConnection,
) -> Result<Option<media_cover_art::Model>, sea_orm::DbErr> {
    media_cover_art::Entity::find()
        .filter(media_cover_art::Column::FileHash.eq(String::new()))
        .one(main_db)
        .await
}

pub async fn get_magic_cover_art_id(main_db: &DatabaseConnection) -> Option<i32> {
    let magic_cover_art = get_magic_cover_art(main_db);

    magic_cover_art.await.ok().flatten().map(|s| s.id)
}

pub async fn ensure_magic_cover_art(
    main_db: &DatabaseConnection,
    node_id: &str,
) -> Result<media_cover_art::Model> {
    if let Some(magic_cover_art) = get_magic_cover_art(main_db).await? {
        Ok(magic_cover_art)
    } else {
        // This local placeholder has no portable artwork identity or body.
        let new_magic_cover_art = media_cover_art::ActiveModel {
            id: ActiveValue::NotSet,
            file_hash: ActiveValue::Set(String::new()),
            binary: ActiveValue::Set(Vec::new()),
            primary_color: ActiveValue::Set(Some(0)),
            content_hash: ActiveValue::Set(None),
            content_size: ActiveValue::Set(None),
            local_available: ActiveValue::Set(false),
            hlc_uuid: ActiveValue::Set("00000000-0000-0000-0000-000000000000".to_owned()),
            created_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
            updated_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
            created_at_hlc_ver: ActiveValue::Set(0),
            updated_at_hlc_ver: ActiveValue::Set(0),
            created_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
            updated_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
        };

        let insert_result = media_cover_art::Entity::insert(new_magic_cover_art)
            .exec(main_db)
            .await
            .with_context(|| "Failed to insert the magic cover art")?;

        let inserted_magic_cover_art =
            media_cover_art::Entity::find_by_id(insert_result.last_insert_id)
                .one(main_db)
                .await?
                .with_context(|| "Inserted magic cover art not found")?;

        Ok(inserted_magic_cover_art)
    }
}

pub async fn ensure_magic_cover_art_id(main_db: &DatabaseConnection, node_id: &str) -> Result<i32> {
    let magic_cover_art = ensure_magic_cover_art(main_db, node_id).await?;
    Ok(magic_cover_art.id)
}

pub static COVER_TEMP_DIR: Lazy<PathBuf> =
    Lazy::new(|| env::temp_dir().join("rune").join("cover_arts"));

fn bake_cover_art_by_cover_arts(
    _fsio: &FsIo,
    cover_arts: Vec<media_cover_art::Model>,
) -> Result<HashMap<i32, String>> {
    let mut cover_art_id_to_path: HashMap<i32, String> = HashMap::new();

    fs::create_dir_all(&*COVER_TEMP_DIR)?;

    for cover_art in cover_arts.iter() {
        let id: i32 = cover_art.id;
        if !cover_art.local_available || cover_art.binary.is_empty() {
            continue;
        }
        let digest = blake3::hash(&cover_art.binary).to_hex().to_string();
        if migration::catalog_identity::artwork(&digest).to_string() != cover_art.hlc_uuid
            || cover_art.content_hash.as_deref() != Some(digest.as_str())
            || cover_art.content_size != Some(cover_art.binary.len() as i64)
        {
            continue;
        }
        let path: PathBuf = COVER_TEMP_DIR.clone().join(digest);

        if fs::read(&path).ok().as_deref() != Some(cover_art.binary.as_slice()) {
            fs::write(&path, &cover_art.binary)?;
        }

        cover_art_id_to_path.insert(id, path.to_str().unwrap_or_default().to_string());
    }

    Ok(cover_art_id_to_path)
}

pub async fn bake_cover_art_by_cover_art_ids(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    cover_art_ids: Vec<i32>,
) -> Result<HashMap<i32, String>> {
    let cover_arts: Vec<media_cover_art::Model> = media_cover_art::Entity::find()
        .filter(media_cover_art::Column::Id.is_in(cover_art_ids))
        .all(main_db)
        .await?;

    bake_cover_art_by_cover_arts(fsio, cover_arts)
}

pub async fn bake_cover_art_by_media_files(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    files: Vec<media_files::Model>,
) -> Result<HashMap<i32, String>> {
    let ids: std::collections::BTreeSet<_> = files
        .iter()
        .filter_map(|file| file.cover_art_uuid.as_deref())
        .collect();
    let mut paths = HashMap::new();
    for ids in ids.into_iter().collect::<Vec<_>>().chunks(4) {
        let artwork = media_cover_art::Entity::find()
            .filter(media_cover_art::Column::HlcUuid.is_in(ids.iter().copied()))
            .filter(media_cover_art::Column::LocalAvailable.eq(true))
            .all(main_db)
            .await?;
        let identity: Vec<_> = artwork
            .iter()
            .map(|row| (row.id, row.hlc_uuid.clone()))
            .collect();
        let baked = bake_cover_art_by_cover_arts(fsio, artwork)?;
        for (id, uuid) in identity {
            if let Some(path) = baked.get(&id) {
                paths.insert(uuid, path.clone());
            }
        }
    }
    Ok(files
        .into_iter()
        .map(|file| {
            let path = file
                .cover_art_uuid
                .as_ref()
                .and_then(|id| paths.get(id))
                .cloned()
                .unwrap_or_default();
            (file.id, path)
        })
        .collect())
}

pub async fn bake_cover_art_by_file_ids(
    fsio: &FsIo,
    main_db: &DatabaseConnection,
    file_ids: Vec<i32>,
) -> Result<HashMap<i32, String>> {
    let files = media_files::Entity::find()
        .filter(media_files::Column::Id.is_in(file_ids))
        .all(main_db)
        .await?;

    bake_cover_art_by_media_files(fsio, main_db, files).await
}

pub fn extract_cover_art_by_file_id(
    fsio: &FsIo,
    lib_path: &Path,
    file: &media_files::Model,
) -> Option<CoverArt> {
    let file_path = Path::new(lib_path)
        .join(file.directory.clone())
        .join(file.file_name.clone());

    // Check if the file actually exists and get its modification time
    if !fsio.exists(&file_path).ok()? {
        return None;
    }

    // If cover_art_id is empty, it means the file has not been checked before
    extract_cover_art_binary(fsio, Some(lib_path), &file_path)
}

/// Batch query to get files with cover art and their update times
/// Inspect local artwork and author only confirmed results in the source transaction.
pub async fn scan_cover_arts<F>(
    fsio: Arc<FsIo>,
    db: &DatabaseConnection,
    root: &Path,
    batch_size: usize,
    progress: F,
    cancel: Option<CancellationToken>,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<usize>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    use sea_orm::QueryOrder;
    anyhow::ensure!(batch_size > 0, "batch size must be positive");
    let total = media_files::Entity::find()
        .filter(media_files::Column::LocalAvailable.eq(true))
        .count(db)
        .await? as usize;
    let mut cursor = 0;
    let mut processed = 0;
    loop {
        let rows = media_files::Entity::find()
            .filter(media_files::Column::Id.gt(cursor))
            .filter(media_files::Column::LocalAvailable.eq(true))
            .order_by_asc(media_files::Column::Id)
            .limit(batch_size as u64)
            .all(db)
            .await?;
        if rows.is_empty() {
            break;
        }
        for file in rows {
            cursor = file.id;
            if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                return Ok(processed);
            }
            if let Some(checked) = &file.cover_art_checked_at
                && let (Ok(modified), Ok(checked)) = (
                    chrono::DateTime::parse_from_rfc3339(&file.last_modified),
                    chrono::DateTime::parse_from_rfc3339(checked),
                )
                && modified <= checked
            {
                processed += 1;
                progress(processed, total);
                continue;
            }
            let path = root.join(&file.directory).join(&file.file_name);
            let expected = match (&file.content_hash, file.content_size) {
                (Some(hash), Some(size)) if size >= 0 => Some(crate::sync::model::BlobRef {
                    digest: hash.clone(),
                    size: size as u64,
                }),
                _ => None,
            };
            if let Some(expected) = &expected
                && crate::actions::file::hash_file(&fsio, &path).ok().as_ref() != Some(expected)
            {
                log::warn!("artwork source changed before inspection: {}", file.id);
                processed += 1;
                progress(processed, total);
                continue;
            }
            let recheck_path = path.clone();
            let root = root.to_path_buf();
            let fs = fsio.clone();
            let inspection = tokio::task::spawn_blocking(move || {
                metadata::cover_art::extract_cover_art_binary_checked(
                    fs.as_ref(),
                    Some(&root),
                    &path,
                )
            })
            .await?;
            if let Some(expected) = &expected
                && crate::actions::file::hash_file(&fsio, &recheck_path)
                    .ok()
                    .as_ref()
                    != Some(expected)
            {
                log::warn!("artwork source changed during inspection: {}", file.id);
                processed += 1;
                progress(processed, total);
                continue;
            }
            match inspection {
                Ok(art) => assign_inspected(db, store, &file, art).await?,
                Err(error) => {
                    log::warn!("artwork inspection for file {} failed: {error:#}", file.id)
                }
            }
            processed += 1;
            progress(processed, total);
        }
    }
    Ok(processed)
}

/// Returns a map of file_id -> (file_last_modified, cover_art_updated_at)
pub async fn get_files_with_cover_art_timestamps(
    main_db: &DatabaseConnection,
    file_ids: Vec<i32>,
) -> Result<HashMap<i32, (String, String)>> {
    let mut result = HashMap::new();
    for ids in file_ids.chunks(500) {
        let rows: Vec<(i32, String, String)> = media_files::Entity::find()
            .select_only()
            .columns([
                media_files::Column::Id,
                media_files::Column::LastModified,
                media_files::Column::CoverArtCheckedAt,
            ])
            .filter(media_files::Column::Id.is_in(ids.iter().copied()))
            .filter(media_files::Column::CoverArtCheckedAt.is_not_null())
            .into_tuple()
            .all(main_db)
            .await?;
        result.extend(
            rows.into_iter()
                .map(|(id, modified, checked)| (id, (modified, checked))),
        );
    }
    Ok(result)
}

/// Clear a track's artwork assignment and publish the source change atomically.
pub async fn remove_cover_art_by_file_id(
    main_db: &DatabaseConnection,
    store: &crate::sync::SeaOrmSyncStore,
    file_id: i32,
) -> Result<()> {
    use sea_orm::TransactionTrait;
    let tx = main_db.begin().await?;
    store.lock(&tx).await?;
    if let Some(file) = media_files::Entity::find_by_id(file_id).one(&tx).await? {
        let id = file.hlc_uuid.parse()?;
        let mut active: media_files::ActiveModel = file.into();
        active.cover_art_uuid = ActiveValue::Set(None);
        active.cover_art_checked_at = ActiveValue::Set(None);
        media_files::Entity::update(active).exec(&tx).await?;
        crate::sync::model::author_source_on(
            &tx,
            store,
            crate::sync::model::ObjectKey {
                collection: crate::sync::model::Collection::Tracks,
                id,
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn get_cover_art_id_by_track_id(
    main_db: &DatabaseConnection,
    file_id: i32,
) -> Result<Option<i32>> {
    let file: Option<media_files::Model> = media_files::Entity::find_by_id(file_id)
        .one(main_db)
        .await?;

    if let Some(file) = file
        && let Some(artwork) = file.cover_art_uuid
    {
        return Ok(media_cover_art::Entity::find()
            .select_only()
            .column(media_cover_art::Column::Id)
            .filter(media_cover_art::Column::HlcUuid.eq(artwork))
            .into_tuple::<i32>()
            .one(main_db)
            .await?);
    }

    Ok(None)
}

pub async fn get_cover_art_by_id(main_db: &DatabaseConnection, id: i32) -> Result<Option<Vec<u8>>> {
    let result = media_cover_art::Entity::find()
        .filter(media_cover_art::Column::Id.eq(id))
        .filter(media_cover_art::Column::LocalAvailable.eq(true))
        .one(main_db)
        .await?;

    match result {
        Some(result) => Ok(Some(result.binary)),
        _none => Ok(None),
    }
}

pub async fn get_primary_color_by_cover_art_id(
    main_db: &DatabaseConnection,
    cover_art_id: i32,
) -> Result<i32> {
    // Step 1: Retrieve the cover art record from the database
    let cover_art = media_cover_art::Entity::find_by_id(cover_art_id)
        .one(main_db)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Cover art not found"))?;

    // Step 2: Check if the primary color is null
    if let Some(primary_color) = cover_art.primary_color {
        return Ok(primary_color);
    }

    // Reads may compute a missing value but do not author a source change.
    get_primary_color(&cover_art.binary).ok_or_else(|| anyhow::anyhow!("No primary color found"))
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{Database, EntityTrait, Set};

    #[tokio::test]
    async fn matching_crc_does_not_merge_distinct_bodies() -> anyhow::Result<()> {
        let node = uuid::Uuid::new_v4();
        let _ = migration::initialize_node_id(node.to_string());
        let db = Database::connect("sqlite::memory:").await?;
        Migrator::up(&db, None).await?;
        for (id, name, hash, source) in [
            (1, "a", "x", "11111111-1111-4111-8111-111111111111"),
            (2, "b", "y", "22222222-2222-4222-8222-222222222222"),
        ] {
            media_files::Entity::insert(media_files::ActiveModel {
                id: Set(id),
                file_name: Set(name.into()),
                directory: Set(String::new()),
                extension: Set("ogg".into()),
                file_hash: Set(hash.into()),
                last_modified: Set("2020".into()),
                sample_rate: Set(44100),
                duration: Set(1.into()),
                hlc_uuid: Set(source.into()),
                local_available: Set(true),
                ..Default::default()
            })
            .exec(&db)
            .await?;
        }
        let store =
            crate::sync::SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
        for (id, data) in [(1, vec![1, 2, 3]), (2, vec![4, 5, 6])] {
            let file = media_files::Entity::find_by_id(id).one(&db).await?.unwrap();
            assign_inspected(
                &db,
                &store,
                &file,
                Some(CoverArt {
                    crc: "collision".into(),
                    data,
                    primary_color: 0,
                }),
            )
            .await?;
        }
        let files = media_files::Entity::find().all(&db).await?;
        assert_ne!(files[0].cover_art_uuid, files[1].cover_art_uuid);
        let artwork = media_cover_art::Entity::find().all(&db).await?;
        let ids = files
            .iter()
            .map(|file| {
                artwork
                    .iter()
                    .find(|art| Some(&art.hlc_uuid) == file.cover_art_uuid.as_ref())
                    .unwrap()
                    .id
            })
            .collect::<Vec<_>>();
        let baked = bake_cover_art_by_cover_art_ids(&FsIo::new(), &db, ids.clone()).await?;
        assert_ne!(baked[&ids[0]], baked[&ids[1]]);
        assert_eq!(std::fs::read(&baked[&ids[0]])?, vec![1, 2, 3]);
        assert_eq!(std::fs::read(&baked[&ids[1]])?, vec![4, 5, 6]);
        for file in files {
            let art = media_cover_art::Entity::find()
                .filter(media_cover_art::Column::HlcUuid.eq(file.cover_art_uuid.unwrap()))
                .one(&db)
                .await?
                .unwrap();
            let digest = blake3::hash(&art.binary).to_hex().to_string();
            assert_eq!(
                art.hlc_uuid,
                migration::catalog_identity::artwork(&digest).to_string()
            );
        }
        Ok(())
    }
}

pub const MAX_ARTWORK_BYTES: usize = 8 * 1024 * 1024;

/// Check the complete content identity before an artwork body may be stored.
pub fn verify_body(id: Uuid, digest: &str, size: u64, body: &[u8]) -> Result<()> {
    ensure!(
        size > 0 && size <= MAX_ARTWORK_BYTES as u64,
        "invalid artwork size"
    );
    ensure!(body.len() as u64 == size, "artwork size mismatch");
    ensure!(
        digest.len() == 64
            && digest
                .bytes()
                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v)),
        "invalid artwork digest"
    );
    ensure!(
        blake3::hash(body).to_hex().as_str() == digest,
        "artwork digest mismatch"
    );
    ensure!(
        migration::catalog_identity::artwork(digest) == id,
        "artwork identity mismatch"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_wrong_size_digest_and_identity() {
        let body = b"artwork bytes";
        let digest = blake3::hash(body).to_hex().to_string();
        let id = migration::catalog_identity::artwork(&digest);
        assert!(verify_body(id, &digest, body.len() as u64, body).is_ok());
        assert!(verify_body(id, &digest, body.len() as u64 + 1, body).is_err());
        assert!(verify_body(id, &digest, body.len() as u64, b"corrupt data!").is_err());
        assert!(verify_body(Uuid::new_v4(), &digest, body.len() as u64, body).is_err());
        assert!(verify_body(id, &digest, MAX_ARTWORK_BYTES as u64 + 1, body).is_err());
    }
}

/// Store a verified body in the primary artwork table within the caller transaction.
async fn attach_body(
    tx: &sea_orm::DatabaseTransaction,
    id: Uuid,
    digest: &str,
    size: u64,
    body: Vec<u8>,
    primary_color: Option<i32>,
) -> Result<i32> {
    use crate::entities::media_cover_art;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
    verify_body(id, digest, size, &body)?;
    let rows = media_cover_art::Entity::find()
        .filter(media_cover_art::Column::HlcUuid.eq(id.to_string()))
        .all(tx)
        .await?;
    ensure!(rows.len() <= 1, "duplicate artwork identity");
    if let Some(row) = rows.into_iter().next() {
        if row.binary != body {
            ensure!(
                verify_body(id, digest, size, &row.binary).is_err(),
                "artwork identity collision"
            );
        }
        if row.binary != body
            || row.primary_color != primary_color
            || !row.local_available
            || row.content_hash.as_deref() != Some(digest)
            || row.content_size != Some(size as i64)
        {
            use sea_orm::sea_query::Expr;
            media_cover_art::Entity::update_many()
                .col_expr(media_cover_art::Column::Binary, Expr::value(body))
                .col_expr(
                    media_cover_art::Column::ContentHash,
                    Expr::value(Some(digest.to_owned())),
                )
                .col_expr(
                    media_cover_art::Column::ContentSize,
                    Expr::value(Some(size as i64)),
                )
                .col_expr(media_cover_art::Column::LocalAvailable, Expr::value(true))
                .col_expr(
                    media_cover_art::Column::PrimaryColor,
                    Expr::value(primary_color),
                )
                .filter(media_cover_art::Column::Id.eq(row.id))
                .exec(tx)
                .await?;
        }
        return Ok(row.id);
    }
    Ok(
        media_cover_art::Entity::insert(media_cover_art::ActiveModel {
            file_hash: Set(format!("blake3:{digest}")),
            binary: Set(body),
            content_hash: Set(Some(digest.to_owned())),
            content_size: Set(Some(size as i64)),
            local_available: Set(true),
            primary_color: Set(primary_color),
            hlc_uuid: Set(id.to_string()),
            ..Default::default()
        })
        .exec(tx)
        .await?
        .last_insert_id,
    )
}

/// Commit a successful artwork inspection and its assignments together.
/// `None` must be supplied only after the caller has confirmed absence.
pub async fn assign_inspected(
    db: &sea_orm::DatabaseConnection,
    store: &crate::sync::SeaOrmSyncStore,
    observed: &crate::entities::media_files::Model,
    art: Option<metadata::cover_art::CoverArt>,
) -> Result<()> {
    use crate::entities::media_files;
    use crate::sync::model::{Collection, ObjectKey, author_source_on};
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
    let file_id: Uuid = observed.hlc_uuid.parse()?;
    ensure!(!file_id.is_nil(), "missing file source identity");
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let current = media_files::Entity::find_by_id(observed.id)
        .one(&tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("inspected file disappeared"))?;
    ensure!(
        current.hlc_uuid == observed.hlc_uuid
            && current.last_modified == observed.last_modified
            && current.content_hash == observed.content_hash
            && current.content_size == observed.content_size
            && current.local_available,
        "file changed during artwork inspection"
    );
    let artwork_id = if let Some(art) = art {
        let digest = blake3::hash(&art.data).to_hex().to_string();
        let id = migration::catalog_identity::artwork(&digest);
        let size = art.data.len() as u64;
        attach_body(&tx, id, &digest, size, art.data, Some(art.primary_color)).await?;
        let key = ObjectKey {
            collection: Collection::Artworks,
            id,
        };
        author_source_on(&tx, store, key).await?;
        Some(id)
    } else {
        None
    };
    media_files::Entity::update_many()
        .col_expr(
            media_files::Column::CoverArtUuid,
            Expr::value(artwork_id.map(|id| id.to_string())),
        )
        .col_expr(
            media_files::Column::CoverArtCheckedAt,
            Expr::value(Some(chrono::Utc::now().to_rfc3339())),
        )
        .filter(media_files::Column::Id.eq(current.id))
        .exec(&tx)
        .await?;
    author_source_on(
        &tx,
        store,
        ObjectKey {
            collection: Collection::Tracks,
            id: file_id,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Return whether the local body fails the accepted artwork's complete identity.
pub async fn artwork_body_needed(
    db: &DatabaseConnection,
    id: Uuid,
    expected: &crate::sync::model::Artwork,
) -> Result<bool> {
    let body = expected.body()?;
    let local = media_cover_art::Entity::find()
        .filter(media_cover_art::Column::HlcUuid.eq(id.to_string()))
        .one(db)
        .await?;
    Ok(local.is_none_or(|row| {
        !row.local_available
            || row.content_hash.as_deref() != Some(body.digest.as_str())
            || row.content_size != Some(body.size as i64)
            || verify_body(id, &body.digest, body.size, &row.binary).is_err()
    }))
}

/// Attach received bytes only while the accepted assignment still matches.
/// The caller completes external I/O before entering this transaction.
pub async fn attach_received_body(
    db: &DatabaseConnection,
    store: &crate::sync::SeaOrmSyncStore,
    id: Uuid,
    expected: &crate::sync::model::Artwork,
    bytes: Vec<u8>,
) -> Result<()> {
    use crate::sync::{
        accepted::AcceptedState,
        model::{Collection, ObjectKey, Payload},
    };
    use sea_orm::TransactionTrait;
    let body = expected.body()?;
    verify_body(id, &body.digest, body.size, &bytes)?;
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let current = AcceptedState::new(&tx, store)
        .get(&ObjectKey {
            collection: Collection::Artworks,
            id,
        })
        .await?;
    ensure!(
        current
            .as_ref()
            .is_some_and(|record| record.value == Some(Payload::Artwork(expected.clone()))),
        "artwork assignment changed during fetch"
    );
    attach_body(
        &tx,
        id,
        &body.digest,
        body.size,
        bytes,
        expected.primary_color,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Read a bounded page of accepted artwork assignments, then release the read transaction.
pub async fn artwork_candidates(
    db: &DatabaseConnection,
    store: &crate::sync::SeaOrmSyncStore,
    cursor: Option<Uuid>,
) -> Result<sync_orm::causal_store::ProjectedPage<crate::sync::model::RuneModel>> {
    use sea_orm::TransactionTrait;
    let tx = db.begin().await?;
    let page = crate::sync::accepted::AcceptedState::new(&tx, store)
        .page(
            crate::sync::model::Collection::Artworks,
            cursor,
            sync::PageLimits {
                max_scanned: 128,
                max_bytes: 4 * 1024 * 1024,
            },
        )
        .await?;
    tx.commit().await?;
    Ok(page)
}

#[cfg(test)]
mod received_body_tests {
    use super::*;
    use crate::sync::{
        LibrarySyncStore,
        model::{Artwork, Collection, ObjectKey, Payload},
    };
    use sea_orm::{Database, sea_query::Expr};

    #[tokio::test]
    async fn attachment_rechecks_assignment_and_repairs_local_availability() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        let actor = Uuid::new_v4();
        migration::up_with_actor(&db, Some(&actor.to_string()), None).await?;
        let library = LibrarySyncStore::open(db.clone(), actor, None).await?;
        let bytes = b"verified artwork bytes".to_vec();
        let digest = blake3::hash(&bytes).to_hex().to_string();
        let id = migration::catalog_identity::artwork(&digest);
        let key = ObjectKey {
            collection: Collection::Artworks,
            id,
        };
        let expected = Artwork {
            digest,
            size: bytes.len() as i64,
            primary_color: Some(1),
        };
        library
            .write_local(key.clone(), Some(Payload::Artwork(expected.clone())))
            .await?;
        let store = library.record_store();
        assert!(artwork_body_needed(&db, id, &expected).await?);
        let changed = Artwork {
            primary_color: Some(2),
            ..expected.clone()
        };
        library
            .write_local(key.clone(), Some(Payload::Artwork(changed.clone())))
            .await?;
        let head = library.head().await?;
        assert!(
            attach_received_body(&db, &store, id, &expected, bytes.clone())
                .await
                .is_err()
        );
        let row = media_cover_art::Entity::find().one(&db).await?.unwrap();
        assert!(!row.local_available);
        assert!(row.binary.is_empty());
        assert_eq!(row.primary_color, Some(2));
        attach_received_body(&db, &store, id, &changed, bytes.clone()).await?;
        assert!(!artwork_body_needed(&db, id, &changed).await?);
        media_cover_art::Entity::update_many()
            .col_expr(media_cover_art::Column::LocalAvailable, Expr::value(false))
            .filter(media_cover_art::Column::HlcUuid.eq(id.to_string()))
            .exec(&db)
            .await?;
        assert!(artwork_body_needed(&db, id, &changed).await?);
        attach_received_body(&db, &store, id, &changed, bytes).await?;
        assert!(!artwork_body_needed(&db, id, &changed).await?);
        assert_eq!(
            library.head().await?,
            head,
            "body attachment does not author metadata"
        );
        library.write_local(key, None).await?;
        assert!(
            attach_received_body(
                &db,
                &store,
                id,
                &changed,
                b"verified artwork bytes".to_vec()
            )
            .await
            .is_err()
        );
        assert!(media_cover_art::Entity::find().one(&db).await?.is_none());
        Ok(())
    }
}
