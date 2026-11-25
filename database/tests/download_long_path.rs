#![cfg(windows)]

use anyhow::Result;
use database::{
    actions::{
        downloads::prepare_download,
        file::{attach_download, is_materialized},
    },
    entities::media_files,
    sync::{SeaOrmSyncStore, model::BlobRef},
    test_fixtures::{causal_library, fixtures},
};
use fsio::FsIo;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use std::fs;
use uuid::Uuid;

#[tokio::test]
async fn attaches_download_when_only_the_file_needs_a_windows_verbatim_path() -> Result<()> {
    let fsio = FsIo::new();
    let tmp = tempfile::tempdir()?;
    let base = fsio.canonicalize_path(tmp.path())?;
    let width = 215usize.saturating_sub(base.as_os_str().len() + 1);
    let root = base.join("r".repeat(width.max(1)));
    fs::create_dir(&root)?;
    let owner = Uuid::new_v4();
    let db = fixtures::memory_database(owner).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), owner).await?;
    let track = Uuid::new_v4();
    let row = causal_library::local_file(&db, 1, track, "track.ogg").await?;
    let bytes = b"downloaded audio";
    let blob = BlobRef {
        digest: blake3::hash(bytes).to_hex().to_string(),
        size: bytes.len() as u64,
    };
    let mut incoming: media_files::ActiveModel = row.into();
    incoming.content_hash = Set(Some(blob.digest.clone()));
    incoming.content_size = Set(Some(blob.size as i64));
    incoming.local_available = Set(false);
    incoming.update(&db).await?;

    let path = prepare_download(&db, &fsio, &root, track, &blob, "ogg").await?;
    fs::write(&path, bytes)?;
    assert!(
        !fsio
            .canonicalize_path(&root)?
            .to_string_lossy()
            .starts_with(r"\\?\")
    );
    assert!(
        fsio.canonicalize_path(&path)?
            .to_string_lossy()
            .starts_with(r"\\?\")
    );

    let outside = base.join("outside.ogg");
    fs::write(&outside, bytes)?;
    let error = attach_download(&db, &store, &fsio, &root, track, &blob, &outside)
        .await
        .expect_err("matching bytes outside the library must not be attached");
    assert!(error.to_string().contains("download escapes library root"));

    attach_download(&db, &store, &fsio, &root, track, &blob, &path).await?;
    let attached = media_files::Entity::find_by_id(1).one(&db).await?.unwrap();
    assert!(attached.local_available);
    let stored_path = root.join(attached.directory).join(attached.file_name);
    assert_eq!(fs::canonicalize(stored_path)?, fs::canonicalize(path)?);
    assert!(is_materialized(&db, &fsio, &root, 1).await?);
    Ok(())
}
