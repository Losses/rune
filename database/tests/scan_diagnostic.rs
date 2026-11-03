//! Opt-in diagnostic using real audio and an isolated, disposable database.
//! RUNE_SCAN_ROOT=/absolute/music/path cargo test -p database --test scan_diagnostic scan_real_library -- --ignored --nocapture
//! RUNE_SCAN_NORMALIZE_MTIME=1 converts only the temporary database's timestamps
//! before the second cover pass to demonstrate the timestamp mismatch.
//! Self-contained regressions run normally; the real-library probe is opt-in.
use std::{path::PathBuf, sync::Arc, time::Instant};

use anyhow::{Context, Result};
use database::{
    actions::{cover_art::scan_cover_arts, metadata::scan_audio_library},
    connection::connect_main_db,
    entities::media_files,
    sync::LibrarySyncStore,
};
use fsio::FsIo;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNE_SCAN_ROOT; reads real audio, writes only a temporary database"]
async fn scan_real_library() -> Result<()> {
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("database=info,metadata=warn"),
    )
    .try_init();
    let root = PathBuf::from(std::env::var("RUNE_SCAN_ROOT").context("set RUNE_SCAN_ROOT")?)
        .canonicalize()?;
    let temp = tempfile::tempdir()?;
    let fs = Arc::new(FsIo::new());
    let node = uuid::Uuid::new_v4();
    let start = Instant::now();
    eprintln!(
        "DIAG root={} database={}",
        root.display(),
        temp.path().display()
    );
    let db = connect_main_db(&fs, temp.path().to_str().unwrap(), None, &node.to_string()).await?;
    let library = LibrarySyncStore::open(db.clone(), node, None).await?;
    library.import_existing(node).await?;
    eprintln!(
        "DIAG initialized elapsed={:.3}s",
        start.elapsed().as_secs_f64()
    );
    for pass in 1..=2 {
        let stage = Instant::now();
        eprintln!("DIAG metadata-start pass={pass}");
        let result = scan_audio_library(
            &fs,
            &db,
            &node.to_string(),
            &root,
            true,
            false,
            |count| {
                eprintln!(
                    "DIAG metadata pass={pass} count={count} elapsed={:.3}s",
                    stage.elapsed().as_secs_f64()
                )
            },
            None,
            &library.record_store(),
            database::actions::metadata::MissingHostFilePolicy::RemoveEverywhere,
        )
        .await;
        eprintln!(
            "DIAG metadata-end pass={pass} result={result:?} rows={} elapsed={:.3}s",
            media_files::Entity::find().count(&db).await?,
            stage.elapsed().as_secs_f64()
        );
        result?;
        // Optional counterfactual: change only this disposable database's mtime
        // encoding to the format currently expected by the cover scanner.
        if pass == 2 && std::env::var_os("RUNE_SCAN_NORMALIZE_MTIME").is_some() {
            for file in media_files::Entity::find().all(&db).await? {
                if let Ok(seconds) = file.last_modified.parse::<i64>() {
                    let timestamp = chrono::DateTime::from_timestamp(seconds, 0)
                        .context("mtime outside chrono range")?
                        .to_rfc3339();
                    media_files::Entity::update_many()
                        .col_expr(
                            media_files::Column::LastModified,
                            sea_orm::sea_query::Expr::value(timestamp),
                        )
                        .filter(media_files::Column::Id.eq(file.id))
                        .exec(&db)
                        .await?;
                }
            }
            eprintln!("DIAG normalized temporary database mtimes before second cover scan");
        }
        let stage = Instant::now();
        eprintln!("DIAG covers-start pass={pass}");
        let result = scan_cover_arts(
            fs.clone(),
            &db,
            &root,
            16,
            move |count, total| {
                if count <= 3 || count % 48 == 0 || count == total {
                    eprintln!(
                        "DIAG covers pass={pass} count={count}/{total} elapsed={:.3}s",
                        stage.elapsed().as_secs_f64()
                    );
                }
            },
            None,
            &library.record_store(),
        )
        .await;
        eprintln!(
            "DIAG covers-end pass={pass} result={result:?} elapsed={:.3}s",
            stage.elapsed().as_secs_f64()
        );
        result?;
    }
    eprintln!(
        "DIAG complete elapsed={:.3}s",
        start.elapsed().as_secs_f64()
    );
    db.close().await?;
    Ok(())
}

fn write_tagged_wav(path: &std::path::Path, title: &str) -> Result<()> {
    fn chunk(output: &mut Vec<u8>, id: &[u8; 4], value: &[u8]) {
        output.extend(id);
        output.extend((value.len() as u32).to_le_bytes());
        output.extend(value);
        if !value.len().is_multiple_of(2) {
            output.push(0);
        }
    }
    let mut body = b"WAVE".to_vec();
    let mut format = Vec::new();
    format.extend(1u16.to_le_bytes()); // PCM
    format.extend(1u16.to_le_bytes()); // mono
    format.extend(44100u32.to_le_bytes());
    format.extend(88200u32.to_le_bytes());
    format.extend(2u16.to_le_bytes());
    format.extend(16u16.to_le_bytes());
    chunk(&mut body, b"fmt ", &format);
    let mut info = b"INFO".to_vec();
    let mut title = title.as_bytes().to_vec();
    title.push(0);
    chunk(&mut info, b"INAM", &title);
    chunk(&mut body, b"LIST", &info);
    chunk(&mut body, b"data", &vec![0; 8820]);
    let mut file = b"RIFF".to_vec();
    file.extend((body.len() as u32).to_le_bytes());
    file.extend(body);
    std::fs::write(path, file)?;
    Ok(())
}

#[tokio::test]
async fn unchanged_cover_scan_does_not_reinspect() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    std::fs::create_dir(root.join("with-cover"))?;
    std::fs::create_dir(root.join("without-cover"))?;
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/source/lotus.png"),
        root.join("with-cover/cover.png"),
    )?;
    write_tagged_wav(&root.join("with-cover/track.wav"), "with cover")?;
    write_tagged_wav(&root.join("without-cover/track.wav"), "without cover")?;
    let fs = Arc::new(FsIo::new());
    let node = uuid::Uuid::new_v4();
    let db = connect_main_db(&fs, root.to_str().unwrap(), None, &node.to_string()).await?;
    let library = LibrarySyncStore::open(db.clone(), node, None).await?;
    let store = library.record_store();
    scan_audio_library(
        &fs,
        &db,
        &node.to_string(),
        root,
        true,
        false,
        |_| {},
        None,
        &store,
        database::actions::metadata::MissingHostFilePolicy::RemoveEverywhere,
    )
    .await?;
    scan_cover_arts(fs.clone(), &db, root, 16, |_, _| {}, None, &store).await?;
    let before = media_files::Entity::find().all(&db).await?;
    assert_eq!(before.len(), 2);
    assert_eq!(
        before
            .iter()
            .filter(|file| file.cover_art_uuid.is_some())
            .count(),
        1
    );
    assert!(
        before
            .iter()
            .all(|file| file.cover_art_checked_at.is_some())
    );
    let head = sync_orm::causal_progress::head_on(&db, store.namespace()).await?;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    scan_cover_arts(fs, &db, root, 16, |_, _| {}, None, &store).await?;
    let after = media_files::Entity::find().all(&db).await?;
    assert_eq!(before, after);
    assert_eq!(
        head,
        sync_orm::causal_progress::head_on(&db, store.namespace()).await?
    );
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn rejected_artwork_assignment_does_not_stop_other_files_and_retries() -> Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let temp = tempfile::tempdir()?;
    let root = temp.path();
    for (directory, image) in [
        ("rejected", "source/lotus.png"),
        ("healthy", "available_ms_store.png"),
    ] {
        std::fs::create_dir(root.join(directory))?;
        std::fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../assets")
                .join(image),
            root.join(directory).join("cover.png"),
        )?;
        write_tagged_wav(&root.join(directory).join("track.wav"), directory)?;
    }
    let rejected_hash = blake3::hash(&std::fs::read(root.join("rejected/cover.png"))?)
        .to_hex()
        .to_string();
    let fs = Arc::new(FsIo::new());
    let node = uuid::Uuid::new_v4();
    let db = connect_main_db(&fs, root.to_str().unwrap(), None, &node.to_string()).await?;
    let library = LibrarySyncStore::open(db.clone(), node, None).await?;
    let store = library.record_store();
    assert_eq!(
        scan_audio_library(
            &fs,
            &db,
            &node.to_string(),
            root,
            true,
            false,
            |_| {},
            None,
            &store,
            database::actions::metadata::MissingHostFilePolicy::RemoveEverywhere
        )
        .await?,
        2
    );
    db.execute_unprepared(&format!(
        "CREATE TRIGGER reject_one_artwork BEFORE INSERT ON media_cover_art WHEN NEW.content_hash = '{rejected_hash}' BEGIN SELECT RAISE(ABORT, 'rejected artwork'); END"
    )).await?;
    let visited = Arc::new(AtomicUsize::new(0));
    let progress = visited.clone();
    scan_cover_arts(
        fs.clone(),
        &db,
        root,
        2,
        move |count, _| {
            progress.store(count, Ordering::Relaxed);
        },
        None,
        &store,
    )
    .await?;
    assert_eq!(visited.load(Ordering::Relaxed), 2);
    let rows = media_files::Entity::find().all(&db).await?;
    let rejected = rows
        .iter()
        .find(|file| file.directory.ends_with("rejected"))
        .context("missing rejected track")?;
    let healthy = rows
        .iter()
        .find(|file| file.directory.ends_with("healthy"))
        .context("missing healthy track")?;
    assert!(rejected.cover_art_uuid.is_none());
    assert!(rejected.cover_art_checked_at.is_none());
    assert!(healthy.cover_art_uuid.is_some());
    assert!(healthy.cover_art_checked_at.is_some());
    let healthy_before = healthy.clone();
    db.execute_unprepared("DROP TRIGGER reject_one_artwork")
        .await?;
    scan_cover_arts(fs, &db, root, 2, |_, _| {}, None, &store).await?;
    let retried = media_files::Entity::find_by_id(rejected.id)
        .one(&db)
        .await?
        .context("missing retried track")?;
    assert!(retried.cover_art_uuid.is_some());
    assert!(retried.cover_art_checked_at.is_some());
    assert_eq!(
        media_files::Entity::find_by_id(healthy_before.id)
            .one(&db)
            .await?,
        Some(healthy_before)
    );
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn oversized_tag_does_not_abort_library_scan() -> Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    for (name, title) in [
        ("before.wav", "ordinary title".to_owned()),
        ("oversized.wav", "t".repeat(16385)),
        ("after.wav", "another title".to_owned()),
    ] {
        write_tagged_wav(&root.join(name), &title)?;
    }
    let fs = FsIo::new();
    let node = uuid::Uuid::new_v4();
    let db = connect_main_db(&fs, root.to_str().unwrap(), None, &node.to_string()).await?;
    let library = LibrarySyncStore::open(db.clone(), node, None).await?;
    let progress = AtomicUsize::new(0);
    let result = scan_audio_library(
        &fs,
        &db,
        &node.to_string(),
        root,
        true,
        false,
        |n| {
            progress.store(n, Ordering::Relaxed);
        },
        None,
        &library.record_store(),
        database::actions::metadata::MissingHostFilePolicy::RemoveEverywhere,
    )
    .await;
    let saved = media_files::Entity::find().count(&db).await?;
    eprintln!(
        "DIAG rejected-tag result={result:?} saved={saved} progress={}",
        progress.load(Ordering::Relaxed)
    );
    assert_eq!(saved, 2, "healthy neighboring files should be saved");
    assert_eq!(result?, 2);
    assert_eq!(progress.load(Ordering::Relaxed), 3);
    db.close().await?;
    Ok(())
}
