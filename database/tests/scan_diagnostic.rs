//! Opt-in diagnostic using real audio and an isolated, disposable database.
//! RUNE_SCAN_ROOT=/absolute/music/path cargo test -p database --test scan_diagnostic scan_real_library -- --ignored --nocapture
//! RUNE_SCAN_NORMALIZE_MTIME=1 converts only the temporary database's timestamps
//! before the second cover pass to demonstrate the timestamp mismatch.
//! Self-contained regressions run normally; the real-library probe is opt-in.

use anyhow::{Context, Result};
use chrono::DateTime;
use database::actions::metadata::MissingHostFilePolicy;
use database::test_fixtures::fixtures;
use database::{
    actions::{
        cover_art::scan_cover_arts,
        metadata::{ScanOptions, scan_audio_library},
    },
    connection::connect_main_db,
    entities::media_files,
};
use env_logger::Builder;
use env_logger::Env;
use fsio::FsIo;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter};
use std::env;
use std::fs;
use std::path as std_path;
use std::{path::PathBuf, sync::Arc, time::Instant};
use sync_orm::causal_progress;
use uuid::Uuid;

#[path = "scan_diagnostic/upgrade.rs"]
mod upgrade;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNE_SCAN_ROOT; reads real audio, writes only a temporary database"]
async fn scan_real_library() -> Result<()> {
    let _ = Builder::from_env(Env::default().default_filter_or("database=info,metadata=warn"))
        .try_init();
    let root =
        PathBuf::from(env::var("RUNE_SCAN_ROOT").context("set RUNE_SCAN_ROOT")?).canonicalize()?;
    let temp = tempfile::tempdir()?;
    let fs = Arc::new(FsIo::new());
    let node = Uuid::new_v4();
    let start = Instant::now();
    eprintln!(
        "DIAG root={} database={}",
        root.display(),
        temp.path().display()
    );
    let db = connect_main_db(&fs, temp.path().to_str().unwrap(), None, &node.to_string()).await?;
    let library = fixtures::open_library(db.clone(), node, None).await?;
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
            ScanOptions {
                cleanup: true,
                force: false,
                missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
                cancel_token: None,
            },
            |count| {
                eprintln!(
                    "DIAG metadata pass={pass} count={count} elapsed={:.3}s",
                    stage.elapsed().as_secs_f64()
                )
            },
            &library.record_store(),
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
        if pass == 2 && env::var_os("RUNE_SCAN_NORMALIZE_MTIME").is_some() {
            for file in media_files::Entity::find().all(&db).await? {
                if let Ok(seconds) = file.last_modified.parse::<i64>() {
                    let timestamp = DateTime::from_timestamp(seconds, 0)
                        .context("mtime outside chrono range")?
                        .to_rfc3339();
                    media_files::Entity::update_many()
                        .col_expr(media_files::Column::LastModified, Expr::value(timestamp))
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

fn write_tagged_wav(path: &std_path::Path, title: &str) -> Result<()> {
    write_wav_with_info(path, &[(b"INAM", title)])
}

fn write_wav_with_info(path: &std_path::Path, tags: &[(&[u8; 4], &str)]) -> Result<()> {
    fn chunk(output: &mut Vec<u8>, id: &[u8; 4], value: &[u8]) {
        output.extend(id);
        output.extend((value.len() as u32).to_le_bytes());
        output.extend(value);
        if !value.len().is_multiple_of(2) {
            output.push(0);
        }
    }

    // Describe the silent audio as 16-bit mono PCM.
    let mut body = b"WAVE".to_vec();
    let mut format = Vec::new();
    format.extend(1u16.to_le_bytes()); // PCM
    format.extend(1u16.to_le_bytes()); // mono
    format.extend(44100u32.to_le_bytes());
    format.extend(88200u32.to_le_bytes());
    format.extend(2u16.to_le_bytes());
    format.extend(16u16.to_le_bytes());
    chunk(&mut body, b"fmt ", &format);

    // RIFF INFO values are NUL-terminated strings.
    let mut info = b"INFO".to_vec();
    for &(id, value) in tags {
        let mut value = value.as_bytes().to_vec();
        value.push(0);
        chunk(&mut info, id, &value);
    }
    chunk(&mut body, b"LIST", &info);
    chunk(&mut body, b"data", &vec![0; 8820]);

    // Wrap the chunks in the outer RIFF container.
    let mut file = b"RIFF".to_vec();
    file.extend((body.len() as u32).to_le_bytes());
    file.extend(body);
    fs::write(path, file)?;
    Ok(())
}

#[tokio::test]
async fn scanned_artist_lists_preserve_legacy_splitting() -> Result<()> {
    use database::actions::{index::index_audio_library, resolved_metadata::CreditScope};
    use database::entities::{audio_artists as media_file_artists, portable_artists as artists};
    use sea_orm::{
        ColumnTrait, ExprTrait,
        sea_query::{Alias, Order, Query},
    };

    // Write one ARTIST tag per audio file, as it appears on disk.
    let mut artist_tags = [
        "Alpha feat. Beta",
        "Gamma & Delta",
        "Epsilon, Zeta",
        "Eta / Theta",
        "Iota",
    ];
    let temp = tempfile::tempdir()?;
    for (index, artist) in artist_tags.iter().enumerate() {
        write_wav_with_info(
            &temp.path().join(format!("track-{index}.wav")),
            &[(b"IART", artist)],
        )?;
    }

    // Use the library's normal scan and index entry points.
    let actor = Uuid::new_v4();
    let actor_id = actor.to_string();
    let db = fixtures::memory_database(actor).await?;
    let library = fixtures::open_library(db.clone(), actor, None).await?;
    let store = library.record_store();
    let scanned = fixtures::scan_library(&FsIo::new(), &db, &actor_id, temp.path(), &store).await?;
    assert_eq!(scanned, artist_tags.len());

    // Both entry points must link the expected artists to each individual file.
    use artists::Column as Artist;
    use media_file_artists::Column as Link;
    use media_files::Column as File;
    let memberships = Query::select()
        .columns([File::FileName.as_column_ref(), Artist::Name.as_column_ref()])
        .from(media_file_artists::Entity)
        .inner_join(
            media_files::Entity,
            Expr::col(Link::TrackUuid.as_column_ref())
                .eq(Expr::col(File::SourceUuid.as_column_ref())),
        )
        .inner_join(
            artists::Entity,
            Expr::col(Link::ArtistUuid.as_column_ref())
                .eq(Expr::col(Artist::SourceUuid.as_column_ref())),
        )
        .order_by(File::FileName.as_column_ref(), Order::Asc)
        .order_by(Artist::Name.as_column_ref(), Order::Asc)
        .to_owned();
    let expected_links = [
        ("track-0.wav", "Alpha"),
        ("track-0.wav", "Beta"),
        ("track-1.wav", "Delta"),
        ("track-1.wav", "Gamma"),
        ("track-2.wav", "Epsilon"),
        ("track-2.wav", "Zeta"),
        ("track-3.wav", "Eta"),
        ("track-3.wav", "Theta"),
        ("track-4.wav", "Iota"),
    ];
    for reindex in [false, true] {
        if reindex {
            index_audio_library(&db, &actor_id, 50, None, &store).await?;
        }
        let rows = db.query_all(&memberships).await?;
        let names = rows
            .iter()
            .map(|row| {
                Ok((
                    row.try_get::<String>("", "file_name")?,
                    row.try_get::<String>("", "name")?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let links = names
            .iter()
            .map(|(file, artist)| (file.as_str(), artist.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(links, expected_links, "after reindex={reindex}");
        upgrade::assert_recordings(&db, artist_tags.len() as u64).await?;
    }

    // The artist catalog and its browse projection expose the same names.
    let expected = [
        "Alpha", "Beta", "Delta", "Epsilon", "Eta", "Gamma", "Iota", "Theta", "Zeta",
    ];
    use database::actions::collection::{CollectionQuery, CollectionQueryListMode};
    let mut names = artists::Model::list(&db, 100, CollectionQueryListMode::Forward)
        .await?
        .into_iter()
        .map(|row| row.name)
        .collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(names, expected);

    // Structured credits retain the exact tag values used for attribution.
    artist_tags.sort_unstable();
    let credited_rows = db
        .query_all(
            Query::select()
                .column(Alias::new("credited_as"))
                .from(Alias::new("music_metadata_credits"))
                .and_where(Expr::col(Alias::new("scope")).eq(CreditScope::Audio as i32))
                .and_where(Expr::col(Alias::new("role")).eq("Primary Artist"))
                .order_by(Alias::new("credited_as"), Order::Asc),
        )
        .await?;

    // Check the full list so missing or extra credits also fail the regression.
    let credited_names = credited_rows
        .iter()
        .map(|row| row.try_get::<String>("", "credited_as"))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(credited_names, artist_tags);
    Ok(())
}

#[tokio::test]
async fn unchanged_cover_scan_does_not_reinspect() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    fs::create_dir(root.join("with-cover"))?;
    fs::create_dir(root.join("without-cover"))?;
    fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../assets/available_app_store.png"
        ),
        root.join("with-cover/cover.png"),
    )?;
    write_tagged_wav(&root.join("with-cover/track.wav"), "with cover")?;
    write_tagged_wav(&root.join("without-cover/track.wav"), "without cover")?;
    let fs = Arc::new(FsIo::new());
    let node = Uuid::new_v4();
    let db = connect_main_db(&fs, root.to_str().unwrap(), None, &node.to_string()).await?;
    let library = fixtures::open_library(db.clone(), node, None).await?;
    let store = library.record_store();
    fixtures::cleanup_scan(
        &fs,
        &db,
        &node.to_string(),
        root,
        &store,
        MissingHostFilePolicy::RemoveEverywhere,
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
    let head = causal_progress::head_on(&db, store.namespace()).await?;
    scan_cover_arts(fs, &db, root, 16, |_, _| {}, None, &store).await?;
    let after = media_files::Entity::find().all(&db).await?;
    assert_eq!(before, after);
    assert_eq!(
        head,
        causal_progress::head_on(&db, store.namespace()).await?
    );
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn normal_local_scan_refreshes_external_album_artwork() -> Result<()> {
    use database::actions::cover_art::bake_cover_art_by_media_files;

    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let tracks = [root.join("first.wav"), root.join("second.wav")];
    for (track, title) in tracks.iter().zip(["First", "Second"]) {
        write_wav_with_info(track, &[(b"INAM", title), (b"IPRD", "Same album")])?;
        fs::File::open(track)?
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000))?;
    }
    let original_audio = tracks
        .iter()
        .map(|track| Ok((fs::metadata(track)?.modified()?, fs::read(track)?)))
        .collect::<Result<Vec<_>>>()?;
    let cover = root.join("cover.png");
    let initial = include_bytes!("../../assets/available_app_store.png").as_slice();
    let replacement = include_bytes!("../../assets/available_ms_store.png").as_slice();
    fs::write(&cover, initial)?;

    let fs = Arc::new(FsIo::new());
    let node = Uuid::new_v4();
    let node_id = node.to_string();
    let db = connect_main_db(&fs, root.to_str().context("library path")?, None, &node_id).await?;
    let library = fixtures::open_library(db.clone(), node, None).await?;
    let store = library.record_store();
    let scan = |force| {
        let fs = Arc::clone(&fs);
        let db = &db;
        let node_id = &node_id;
        let store = &store;
        async move {
            let scanned = scan_audio_library(
                &fs,
                db,
                node_id,
                root,
                ScanOptions {
                    cleanup: true,
                    force,
                    missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
                    cancel_token: None,
                },
                |_| {},
                store,
            )
            .await?;
            assert_eq!(
                scan_cover_arts(fs, db, root, 16, |_, _| {}, None, store).await?,
                2
            );
            Ok::<_, anyhow::Error>(scanned)
        }
    };
    let inspect = |stage, expected: Option<&'static [u8]>| {
        let fs = &fs;
        let db = &db;
        async move {
            let files = media_files::Entity::find().all(db).await?;
            assert_eq!(files.len(), 2);
            assert!(files.iter().all(|file| file.cover_art_checked_at.is_some()));
            let baked = bake_cover_art_by_media_files(fs, db, files.clone()).await?;
            let mut matches = true;
            for file in files {
                let path = &baked[&file.id];
                let bytes = (!path.is_empty())
                    .then(|| std::fs::read(path))
                    .transpose()?;
                let digest = bytes
                    .as_deref()
                    .map(|bytes| blake3::hash(bytes).to_hex().to_string());
                println!(
                    "{stage}: {} {:?} {digest:?}",
                    file.file_name, file.cover_art_uuid
                );
                matches &= file.cover_art_uuid.is_some() == expected.is_some()
                    && bytes.as_deref() == expected;
            }
            Ok::<_, anyhow::Error>(matches)
        }
    };

    assert_eq!(scan(false).await?, 2);
    assert!(inspect("initial", Some(initial)).await?);
    let mut stale = Vec::new();

    fs::write(&cover, replacement)?;
    assert_eq!(scan(false).await?, 0);
    if !inspect("normal replacement", Some(replacement)).await? {
        stale.push("replacement");
    }
    assert_eq!(scan(true).await?, 2);
    assert!(inspect("forced replacement", Some(replacement)).await?);

    fs::remove_file(&cover)?;
    assert_eq!(scan(false).await?, 0);
    if !inspect("normal removal", None).await? {
        stale.push("removal");
    }
    assert_eq!(scan(true).await?, 2);
    assert!(inspect("forced removal", None).await?);

    fs::write(&cover, initial)?;
    assert_eq!(scan(false).await?, 0);
    if !inspect("normal addition after checked absence", Some(initial)).await? {
        stale.push("addition after checked absence");
    }
    assert_eq!(scan(true).await?, 2);
    assert!(inspect("forced addition after checked absence", Some(initial)).await?);

    for (track, (modified, bytes)) in tracks.iter().zip(original_audio) {
        assert_eq!(fs::metadata(track)?.modified()?, modified);
        assert_eq!(fs::read(track)?, bytes);
    }
    let before = media_files::Entity::find().all(&db).await?;
    let head = causal_progress::head_on(&db, store.namespace()).await?;
    assert_eq!(scan(false).await?, 0);
    assert_eq!(media_files::Entity::find().all(&db).await?, before);
    assert_eq!(
        causal_progress::head_on(&db, store.namespace()).await?,
        head
    );
    assert!(
        stale.is_empty(),
        "normal scan left external artwork stale: {stale:?}"
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
        ("rejected", "available_app_store.png"),
        ("healthy", "available_ms_store.png"),
    ] {
        fs::create_dir(root.join(directory))?;
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../assets")
                .join(image),
            root.join(directory).join("cover.png"),
        )?;
        write_tagged_wav(&root.join(directory).join("track.wav"), directory)?;
    }
    let rejected_hash = blake3::hash(&fs::read(root.join("rejected/cover.png"))?)
        .to_hex()
        .to_string();
    let fs = Arc::new(FsIo::new());
    let node = Uuid::new_v4();
    let db = connect_main_db(&fs, root.to_str().unwrap(), None, &node.to_string()).await?;
    let library = fixtures::open_library(db.clone(), node, None).await?;
    let store = library.record_store();
    assert_eq!(
        scan_audio_library(
            &fs,
            &db,
            &node.to_string(),
            root,
            ScanOptions {
                cleanup: true,
                force: false,
                missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
                cancel_token: None
            },
            |_| {},
            &store,
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
async fn oversized_tag_stays_in_catalog_without_exceeding_sync_limit() -> Result<()> {
    use database::actions::metadata::get_metadata_summary_by_files;
    use database::entities::source_tags as media_metadata;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let oversized_title = "t".repeat(16_385);
    for (name, title) in [
        ("before.wav", "ordinary title"),
        ("oversized.wav", oversized_title.as_str()),
        ("after.wav", "another title"),
    ] {
        write_tagged_wav(&root.join(name), title)?;
    }
    let fs = FsIo::new();
    let node = Uuid::new_v4();
    let db = connect_main_db(&fs, root.to_str().unwrap(), None, &node.to_string()).await?;
    let library = fixtures::open_library(db.clone(), node, None).await?;
    let progress = AtomicUsize::new(0);
    let result = scan_audio_library(
        &fs,
        &db,
        &node.to_string(),
        root,
        ScanOptions {
            cleanup: true,
            force: false,
            missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
            cancel_token: None,
        },
        |n| {
            progress.store(n, Ordering::Relaxed);
        },
        &library.record_store(),
    )
    .await?;
    assert_eq!(result, 3);
    assert_eq!(progress.load(Ordering::Relaxed), 3);

    // Keep the complete title in the catalog while bounding the portable payload.
    let files = media_files::Entity::find().all(&db).await?;
    let summaries = get_metadata_summary_by_files(&db, files).await?;
    let oversized = summaries
        .iter()
        .find(|file| file.file_name == "oversized.wav")
        .context("missing file with oversized title")?;
    assert_eq!(oversized.title, oversized_title);
    assert_eq!(
        media_metadata::Entity::find()
            .filter(media_metadata::Column::FileId.eq(oversized.id))
            .filter(media_metadata::Column::MetaKey.eq("track_title"))
            .count(&db)
            .await?,
        0
    );
    db.close().await?;
    Ok(())
}

/// A path that is not valid Unicode cannot be stored without loss, so the scan
/// skips it, reports it in the library log and indexes the tracks beside it. A
/// folder with such a name is skipped whole: its tracks would otherwise be
/// stored as if they sat in the library root.
#[cfg(any(windows, all(unix, not(target_vendor = "apple"))))]
#[tokio::test]
async fn paths_that_are_not_unicode_are_skipped_and_reported() -> Result<()> {
    use database::actions::logging::list_log;
    use fsio::test_fixtures::non_unicode_name;

    async fn indexed(db: &sea_orm::DatabaseConnection) -> Result<Vec<(String, String)>> {
        Ok(media_files::Entity::find()
            .all(db)
            .await?
            .into_iter()
            .map(|file| (file.directory, file.file_name))
            .collect())
    }

    let temp = tempfile::tempdir()?;
    let root = temp.path();
    write_tagged_wav(&root.join("track.wav"), "kept")?;
    let track = root.join(non_unicode_name("track-", ".wav"));
    write_tagged_wav(&track, "skipped track")?;
    let folder = root.join(non_unicode_name("folder-", ""));
    fs::create_dir(&folder)?;
    write_tagged_wav(&folder.join("track.wav"), "skipped folder")?;
    let fs = FsIo::new();
    let node = Uuid::new_v4();
    let db = connect_main_db(&fs, root.to_str().unwrap(), None, &node.to_string()).await?;
    let library = fixtures::open_library(db.clone(), node, None).await?;
    let store = library.record_store();
    let options = || ScanOptions {
        cleanup: true,
        force: false,
        missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
        cancel_token: None,
    };
    let kept = [(String::new(), "track.wav".to_owned())];

    scan_audio_library(&fs, &db, &node.to_string(), root, options(), |_| {}, &store).await?;
    assert_eq!(indexed(&db).await?, kept);
    let mut reported: Vec<_> = list_log(&db, 0, 100)
        .await?
        .into_iter()
        .filter(|log| log.level == "WARNING")
        .map(|log| log.detail)
        .collect();
    reported.sort();
    let mut expected =
        [track, folder].map(|path| format!("Skipped {path:?}: its path is not valid Unicode"));
    expected.sort();
    assert_eq!(reported, expected);

    // A retry keeps the track and leaves the skipped paths out again.
    scan_audio_library(&fs, &db, &node.to_string(), root, options(), |_| {}, &store).await?;
    assert_eq!(indexed(&db).await?, kept);
    db.close().await?;
    Ok(())
}

/// A walk that skipped a folder cannot tell whether the tracks indexed from it
/// still exist, so cleanup leaves them as they are.
#[cfg(any(windows, all(unix, not(target_vendor = "apple"))))]
#[tokio::test]
async fn folder_renamed_to_a_non_unicode_name_keeps_its_tracks() -> Result<()> {
    use fsio::test_fixtures::non_unicode_name;

    async fn rows(db: &sea_orm::DatabaseConnection) -> Result<Vec<(i32, bool)>> {
        Ok(media_files::Entity::find()
            .all(db)
            .await?
            .into_iter()
            .map(|file| (file.id, file.local_available))
            .collect())
    }

    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let album = root.join("album");
    fs::create_dir(&album)?;
    write_tagged_wav(&album.join("track.wav"), "renamed folder")?;
    let fs = FsIo::new();
    let node = Uuid::new_v4();
    let db = connect_main_db(&fs, root.to_str().unwrap(), None, &node.to_string()).await?;
    let library = fixtures::open_library(db.clone(), node, None).await?;
    let store = library.record_store();
    let options = || ScanOptions {
        cleanup: true,
        force: false,
        missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
        cancel_token: None,
    };

    scan_audio_library(&fs, &db, &node.to_string(), root, options(), |_| {}, &store).await?;
    let track = media_files::Entity::find()
        .one(&db)
        .await?
        .context("the track was not indexed")?;
    assert_eq!(rows(&db).await?, [(track.id, true)]);

    let renamed = root.join(non_unicode_name("album-", ""));
    fs::rename(&album, &renamed)?;
    scan_audio_library(&fs, &db, &node.to_string(), root, options(), |_| {}, &store).await?;
    assert_eq!(
        rows(&db).await?,
        [(track.id, true)],
        "a scan that skipped a folder must leave its tracks alone"
    );

    fs::rename(&renamed, &album)?;
    scan_audio_library(&fs, &db, &node.to_string(), root, options(), |_| {}, &store).await?;
    assert_eq!(rows(&db).await?, [(track.id, true)]);
    db.close().await?;
    Ok(())
}

/// A track deleted while the walk skips an unrelated path is removed by the
/// next scan that reads every path.
#[cfg(any(windows, all(unix, not(target_vendor = "apple"))))]
#[tokio::test]
async fn removal_waits_for_a_walk_that_skips_nothing() -> Result<()> {
    use fsio::test_fixtures::non_unicode_name;

    async fn names(db: &sea_orm::DatabaseConnection) -> Result<Vec<String>> {
        let mut names: Vec<_> = media_files::Entity::find()
            .all(db)
            .await?
            .into_iter()
            .map(|file| file.file_name)
            .collect();
        names.sort();
        Ok(names)
    }

    let temp = tempfile::tempdir()?;
    let root = temp.path();
    write_tagged_wav(&root.join("deleted.wav"), "deleted")?;
    write_tagged_wav(&root.join("kept.wav"), "kept")?;
    let fs = FsIo::new();
    let node = Uuid::new_v4();
    let db = connect_main_db(&fs, root.to_str().unwrap(), None, &node.to_string()).await?;
    let library = fixtures::open_library(db.clone(), node, None).await?;
    let store = library.record_store();
    let options = || ScanOptions {
        cleanup: true,
        force: false,
        missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
        cancel_token: None,
    };

    scan_audio_library(&fs, &db, &node.to_string(), root, options(), |_| {}, &store).await?;
    assert_eq!(names(&db).await?, ["deleted.wav", "kept.wav"]);

    fs::remove_file(root.join("deleted.wav"))?;
    let stray = root.join(non_unicode_name("stray-", ".txt"));
    fs::write(&stray, b"")?;
    scan_audio_library(&fs, &db, &node.to_string(), root, options(), |_| {}, &store).await?;
    assert_eq!(names(&db).await?, ["deleted.wav", "kept.wav"]);

    fs::remove_file(&stray)?;
    scan_audio_library(&fs, &db, &node.to_string(), root, options(), |_| {}, &store).await?;
    assert_eq!(
        names(&db).await?,
        ["kept.wav"],
        "the first scan that reads every path removes the deleted track"
    );
    db.close().await?;
    Ok(())
}

/// A walk that fails reports none of the paths it skipped, so the scan leaves
/// cleanup to a later scan and keeps the tracks of a folder renamed to a name
/// that is not valid Unicode.
#[cfg(all(unix, not(target_vendor = "apple")))]
#[tokio::test]
async fn failed_walk_keeps_tracks_below_a_skipped_folder() -> Result<()> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    use fsio::test_fixtures::non_unicode_name;

    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let album = root.join("album");
    fs::create_dir(&album)?;
    write_tagged_wav(&album.join("track.wav"), "renamed folder")?;
    let fs = FsIo::new();
    let node = Uuid::new_v4();
    let actor = node.to_string();
    let db = connect_main_db(&fs, root.to_str().unwrap(), None, &actor).await?;
    let library = fixtures::open_library(db.clone(), node, None).await?;
    let store = library.record_store();
    let scan = || {
        fixtures::cleanup_scan(
            &fs,
            &db,
            &actor,
            root,
            &store,
            MissingHostFilePolicy::RemoveEverywhere,
        )
    };

    scan().await?;
    let indexed = media_files::Entity::find().all(&db).await?;
    assert_eq!(indexed.len(), 1);

    fs::rename(&album, root.join(non_unicode_name("album-", "")))?;
    // The walk lists a folder without search permission, but cannot read the
    // metadata of its entries, so it fails.
    let blocked = root.join("blocked");
    fs::create_dir(&blocked)?;
    fs::write(blocked.join("unreadable"), b"")?;
    fs::set_permissions(&blocked, Permissions::from_mode(0o444))?;
    let failed = scan().await;
    fs::set_permissions(&blocked, Permissions::from_mode(0o755))?;

    assert!(
        failed.is_err(),
        "the walk should fail on the entry it cannot read"
    );
    assert_eq!(
        media_files::Entity::find().all(&db).await?,
        indexed,
        "a failed walk must not clean up tracks below a skipped folder"
    );
    db.close().await?;
    Ok(())
}
