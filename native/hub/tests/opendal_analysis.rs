#[path = "support/fixtures.rs"]
mod fixtures;

use std::time::Duration;
use std::{
    io::Error,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use analysis::{analysis::analyze_audio, utils::computing_device::ComputingDevice};
use anyhow::{Context, Result};
use database::{
    actions::{
        analysis::analysis_audio_library, cover_art::scan_cover_arts, mixes::query_mix_media_files,
    },
    connection::RecommendationDbConnection,
    entities::{media_analysis, media_files},
};
use fsio::{
    FileIoError, FsIo, OpenDalConfig,
    test_fixtures::{S3Fixture, WebDavFixture},
};
use playback::{
    player::{Playable, PlaybackSource, PlaybackState, Player, PlayingItem},
    strategies::AddMode,
};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter};
use tokio::{runtime::Handle, task::spawn_blocking, time::timeout};
use uuid::Uuid;

const DEADLINE: Duration = Duration::from_secs(30);

fn webdav_config(server: &WebDavFixture) -> OpenDalConfig {
    OpenDalConfig::WebDav {
        endpoint: server.endpoint.clone(),
        root: "/dav/library/".into(),
        username: "user".into(),
        password: "pass".into(),
    }
}

fn s3_config(server: &S3Fixture) -> OpenDalConfig {
    OpenDalConfig::S3 {
        endpoint: server.endpoint.clone(),
        bucket: "bucket".into(),
        region: "us-east-1".into(),
        root: "/library/".into(),
        access_key_id: "access".into(),
        secret_access_key: "secret".into(),
    }
}

async fn assert_scan_change_detection(
    backend: &str,
    fs: FsIo,
    changed: Vec<u8>,
    set_file: impl Fn(Vec<u8>),
    read_count: impl Fn() -> usize,
) -> Result<()> {
    let actor = Uuid::new_v4();
    let db = fixtures::memory_database(actor).await?;
    let store = fixtures::open_library(db.clone(), actor, None).await?;
    fixtures::scan_library(
        &fs,
        &db,
        &actor.to_string(),
        Path::new(""),
        &store.record_store(),
    )
    .await?;
    let original = media_files::Entity::find()
        .one(&db)
        .await?
        .context("missing initial file")?;
    assert!(
        original.last_modified.parse::<i64>().is_ok(),
        "{backend} last_modified must remain a modification time, got {:?}",
        original.last_modified
    );
    let original_change_token = original.change_token.clone();
    assert!(
        original_change_token.is_some(),
        "{backend} ETag was not stored"
    );
    media_files::Entity::update_many()
        .col_expr(
            media_files::Column::CoverArtCheckedAt,
            Expr::value(Some("2025-01-02T00:00:00Z".to_owned())),
        )
        .filter(media_files::Column::Id.eq(original.id))
        .exec(&db)
        .await?;
    let reads_before_current_cover_check = read_count();
    scan_cover_arts(
        Arc::new(fs.clone()),
        &db,
        Path::new(""),
        4,
        |_, _| {},
        None,
        &store.record_store(),
    )
    .await?;
    assert_eq!(
        read_count(),
        reads_before_current_cover_check,
        "{backend} cover inspection was not considered current"
    );
    let after_initial = read_count();
    fixtures::scan_library(
        &fs,
        &db,
        &actor.to_string(),
        Path::new(""),
        &store.record_store(),
    )
    .await?;
    assert_eq!(
        read_count(),
        after_initial,
        "unchanged object was read again"
    );
    set_file(changed);
    let check_fs = fs.clone();
    let prior_token = original_change_token.clone().context("missing token")?;
    assert_ne!(
        spawn_blocking(move || check_fs.change_token(Path::new("track.ogg"))).await??,
        prior_token,
        "{backend} fixture did not change its ETag"
    );
    let changed_token = spawn_blocking({
        let fs = fs.clone();
        move || fs.change_token(Path::new("track.ogg"))
    })
    .await??;
    let rescanned = fixtures::scan_library(
        &fs,
        &db,
        &actor.to_string(),
        Path::new(""),
        &store.record_store(),
    )
    .await?;
    let updated = media_files::Entity::find()
        .one(&db)
        .await?
        .context("missing rescanned file")?;
    assert_ne!(
        original_change_token, updated.change_token,
        "{backend} ETag did not change: {:?} -> {:?}, current token {changed_token}, processed {rescanned}",
        original_change_token, updated.change_token
    );
    assert!(read_count() > after_initial, "changed object was not read");
    Ok(())
}

fn large_wav() -> Vec<u8> {
    let data_length = 36 * 1024 * 1024;
    let mut bytes = Vec::with_capacity(data_length + 44);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((data_length + 36) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&192_000_u32.to_le_bytes());
    bytes.extend_from_slice(&768_000_u32.to_le_bytes());
    bytes.extend_from_slice(&4_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_length as u32).to_le_bytes());
    bytes.resize(data_length + 44, 0);
    bytes
}

async fn assert_large_audio_playback_and_analysis(
    fs: FsIo,
    ranges: impl Fn() -> Vec<Option<(usize, usize)>>,
    actor: Uuid,
) -> Result<()> {
    let db = fixtures::memory_database(actor).await?;
    let store = fixtures::open_library(db.clone(), actor, None).await?;
    fixtures::scan_library(
        &fs,
        &db,
        &actor.to_string(),
        Path::new(""),
        &store.record_store(),
    )
    .await?;
    assert_eq!(media_files::Entity::find().count(&db).await?, 1);

    let player = Player::new(Arc::new(fs.clone()), None);
    player.add_to_playlist_with_sources(
        vec![(
            PlayingItem::IndependentFile("large.wav".into()),
            PlaybackSource::Local("large.wav".into()),
        )],
        AddMode::AppendToEnd,
    );
    player.load(0);
    player.play();
    tokio::time::timeout(Duration::from_secs(10), async {
        while player.get_status().state != PlaybackState::Playing {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("large remote playback did not start")?;
    player.seek(45.0);

    let analysis_fs = Arc::new(fs);
    assert_eq!(
        analysis_audio_library(
            analysis_fs,
            &db,
            Path::new(""),
            1,
            ComputingDevice::Cpu,
            |_, _| {},
            None,
        )
        .await?,
        1,
    );
    assert!(media_analysis::Entity::find().count(&db).await? > 0);
    let ranges = ranges();
    assert!(
        ranges.iter().all(Option::is_some),
        "non-ranged audio read: {ranges:?}"
    );
    assert!(
        ranges
            .iter()
            .flatten()
            .any(|(start, _)| *start > 32 * 1024 * 1024),
        "playback seek did not read near the end: {ranges:?}"
    );
    player.terminate();
    Ok(())
}

fn change_ogg_vendor(bytes: &mut [u8]) {
    let marker = bytes
        .windows(4)
        .position(|window| window == b"Lavf")
        .unwrap();
    let page_start = bytes[..marker]
        .windows(4)
        .rposition(|window| window == b"OggS")
        .unwrap();
    bytes[marker] = b'M';
    let segment_count = bytes[page_start + 26] as usize;
    let body_length = bytes[page_start + 27..page_start + 27 + segment_count]
        .iter()
        .map(|length| usize::from(*length))
        .sum::<usize>();
    let page_end = page_start + 27 + segment_count + body_length;
    bytes[page_start + 22..page_start + 26].fill(0);
    let mut checksum = 0_u32;
    for byte in &bytes[page_start..page_end] {
        checksum ^= u32::from(*byte) << 24;
        for _ in 0..8 {
            checksum = if checksum & 0x8000_0000 == 0 {
                checksum << 1
            } else {
                (checksum << 1) ^ 0x04c1_1db7
            };
        }
    }
    bytes[page_start + 22..page_start + 26].copy_from_slice(&checksum.to_le_bytes());
}

async fn scanned_database(fs: &FsIo, root: &Path, actor: Uuid) -> Result<DatabaseConnection> {
    let db = fixtures::memory_database(actor).await?;
    let store = fixtures::open_library(db.clone(), actor, None).await?;
    fixtures::scan_library(fs, &db, &actor.to_string(), root, &store.record_store()).await?;
    Ok(db)
}

#[tokio::test]
async fn webdav_analysis_matches_local_and_never_writes_to_the_share() -> Result<()> {
    timeout(DEADLINE, async {
        let bytes = std::fs::read(fixtures::audio_fixture())?;
        let server = WebDavFixture::start(vec![("track.ogg".into(), bytes)]).await?;
        let remote_fs = Arc::new(FsIo::new_opendal(
            webdav_config(&server),
            Handle::current(),
        )?);
        let remote_db = scanned_database(&remote_fs, Path::new(""), Uuid::from_u128(0x201)).await?;

        let local_root = tempfile::tempdir()?;
        std::fs::copy(
            fixtures::audio_fixture(),
            local_root.path().join("track.ogg"),
        )?;
        let local_fs = FsIo::new();
        let local_db =
            scanned_database(&local_fs, local_root.path(), Uuid::from_u128(0x202)).await?;

        assert_eq!(
            analysis_audio_library(
                remote_fs,
                &remote_db,
                Path::new(""),
                1,
                ComputingDevice::Cpu,
                |_, _| {},
                None,
            )
            .await?,
            1,
        );
        assert_eq!(
            analysis_audio_library(
                Arc::new(local_fs),
                &local_db,
                local_root.path(),
                1,
                ComputingDevice::Cpu,
                |_, _| {},
                None,
            )
            .await?,
            1,
        );

        let remote = media_analysis::Entity::find()
            .one(&remote_db)
            .await?
            .context("missing WebDAV analysis")?;
        let local = media_analysis::Entity::find()
            .one(&local_db)
            .await?
            .context("missing local analysis")?;
        assert_eq!(remote, local);
        assert!(server.write_requests().is_empty());
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("WebDAV analysis timed out")??;
    Ok(())
}

#[tokio::test]
async fn rejected_webdav_read_is_a_typed_per_file_failure_and_does_not_stop_analysis() -> Result<()>
{
    timeout(DEADLINE, async {
        let bytes = std::fs::read(fixtures::audio_fixture())?;
        let server = WebDavFixture::start(vec![
            ("first.ogg".into(), bytes.clone()),
            ("second.ogg".into(), bytes),
        ])
        .await?;
        let fs = Arc::new(FsIo::new_opendal(
            webdav_config(&server),
            Handle::current(),
        )?);
        let actor = Uuid::from_u128(0x203);
        let db = scanned_database(&fs, Path::new(""), actor).await?;
        let rejected = Arc::new(server.clone());
        let progress_count = Arc::new(AtomicUsize::new(0));
        let callback_count = Arc::clone(&progress_count);
        let root = PathBuf::new();
        assert_eq!(
            analysis_audio_library(
                Arc::clone(&fs),
                &db,
                &root,
                1,
                ComputingDevice::Cpu,
                move |processed, _| {
                    callback_count.store(processed, Ordering::SeqCst);
                    if processed == 1 {
                        rejected.reject_reads();
                    }
                },
                None,
            )
            .await?,
            2,
        );
        assert_eq!(progress_count.load(Ordering::SeqCst), 2);
        assert_eq!(media_analysis::Entity::find().count(&db).await?, 1);

        let failure_fs = Arc::clone(&fs);
        let result = spawn_blocking(move || {
            analyze_audio(
                &failure_fs,
                "second.ogg",
                1024,
                512,
                ComputingDevice::Cpu,
                None,
            )
        })
        .await?;
        let error = result.expect_err("rejected credentials must fail the file analysis");
        assert!(
            matches!(
                error
                    .chain()
                    .find_map(|cause| cause.downcast_ref::<Error>())
                    .and_then(|io_error| io_error.get_ref())
                    .and_then(|cause| cause.downcast_ref::<FileIoError>()),
                Some(FileIoError::PermissionDenied(_))
            ),
            "{error:#}"
        );
        assert!(server.write_requests().is_empty());
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("credential rejection analysis timed out")??;
    Ok(())
}

#[tokio::test]
async fn playback_and_analysis_can_read_the_same_webdav_library_concurrently() -> Result<()> {
    timeout(DEADLINE, async {
        let bytes = std::fs::read(fixtures::audio_fixture())?;
        let server = WebDavFixture::start(vec![("track.ogg".into(), bytes)]).await?;
        let fs = Arc::new(FsIo::new_opendal(
            webdav_config(&server),
            Handle::current(),
        )?);
        let actor = Uuid::from_u128(0x204);
        let db = scanned_database(&fs, Path::new(""), actor).await?;

        let player = Player::new(Arc::clone(&fs), None);
        player.add_to_playlist_with_sources(
            vec![(
                PlayingItem::IndependentFile("track.ogg".into()),
                PlaybackSource::Local("track.ogg".into()),
            )],
            AddMode::AppendToEnd,
        );
        player.load(0);
        player.play();

        let player_for_status = &player;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if player_for_status.get_status().state == PlaybackState::Playing {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("WebDAV playback did not start")?;
        player.seek(0.5);
        let analysis_fs = Arc::clone(&fs);
        let analysis_db = db.clone();
        let analysis_progress = Arc::new(AtomicUsize::new(0));
        let callback_progress = Arc::clone(&analysis_progress);
        let analysis_task = tokio::spawn(async move {
            analysis_audio_library(
                analysis_fs,
                &analysis_db,
                Path::new(""),
                1,
                ComputingDevice::Cpu,
                move |processed, _| callback_progress.store(processed, Ordering::SeqCst),
                None,
            )
            .await
        });
        assert_eq!(analysis_task.await??, 1);
        assert_eq!(analysis_progress.load(Ordering::SeqCst), 1);
        assert!(media_analysis::Entity::find().count(&db).await? > 0);
        let ranges = server.read_ranges();
        assert!(
            !ranges.is_empty(),
            "audio playback/analysis made no GET requests"
        );
        assert!(
            ranges.iter().all(Option::is_some),
            "whole-file GET observed: {ranges:?}"
        );
        player.terminate();
        assert!(server.write_requests().is_empty());
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("concurrent WebDAV playback and analysis timed out")??;
    Ok(())
}

#[tokio::test]
async fn paged_audio_library_scan_is_complete_on_both_backends() -> Result<()> {
    timeout(Duration::from_secs(120), async {
        let audio = std::fs::read(fixtures::audio_fixture())?;
        let files = (0..11)
            .map(|index| (format!("track-{index:02}.ogg"), audio.clone()))
            .collect::<Vec<_>>();
        let webdav = WebDavFixture::start(files.clone()).await?;
        let s3 = S3Fixture::start(files).await?;
        for (fs, actor) in [
            (
                FsIo::new_opendal(webdav_config(&webdav), Handle::current())?,
                Uuid::from_u128(0x211),
            ),
            (
                FsIo::new_opendal(s3_config(&s3), Handle::current())?,
                Uuid::from_u128(0x212),
            ),
        ] {
            let db = scanned_database(&fs, Path::new(""), actor).await?;
            assert_eq!(media_files::Entity::find().count(&db).await?, 11);
        }
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("paged library scan timed out")??;
    Ok(())
}

#[tokio::test]
async fn large_audio_is_played_seeked_and_analyzed_by_both_backends() -> Result<()> {
    timeout(Duration::from_secs(180), async {
        let audio = large_wav();
        let webdav = WebDavFixture::start(vec![("large.wav".into(), audio.clone())]).await?;
        let s3 = S3Fixture::start(vec![("large.wav".into(), audio)]).await?;
        let webdav_ranges = webdav.clone();
        assert_large_audio_playback_and_analysis(
            FsIo::new_opendal(webdav_config(&webdav), Handle::current())?,
            move || webdav_ranges.read_ranges(),
            Uuid::from_u128(0x213),
        )
        .await?;
        let s3_ranges = s3.clone();
        assert_large_audio_playback_and_analysis(
            FsIo::new_opendal(s3_config(&s3), Handle::current())?,
            move || s3_ranges.read_ranges(),
            Uuid::from_u128(0x214),
        )
        .await?;
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("large audio playback and analysis timed out")??;
    Ok(())
}

#[tokio::test]
async fn object_store_scan_uses_etag_and_skips_unchanged_content() -> Result<()> {
    timeout(DEADLINE, async {
        let bytes = std::fs::read(fixtures::audio_fixture())?;
        let webdav = WebDavFixture::start(vec![("track.ogg".into(), bytes.clone())]).await?;
        let s3 = S3Fixture::start(vec![("track.ogg".into(), bytes.clone())]).await?;
        let mut changed = bytes.clone();
        change_ogg_vendor(&mut changed);
        assert_eq!(changed.len(), bytes.len());

        let webdav_set = webdav.clone();
        let webdav_count = webdav.clone();
        assert_scan_change_detection(
            "WebDAV",
            FsIo::new_opendal(webdav_config(&webdav), Handle::current())?,
            changed.clone(),
            move |data| webdav_set.set_file("track.ogg", data),
            move || webdav_count.read_ranges().len(),
        )
        .await?;
        let s3_set = s3.clone();
        let s3_count = s3.clone();
        assert_scan_change_detection(
            "S3",
            FsIo::new_opendal(s3_config(&s3), Handle::current())?,
            changed,
            move |data| s3_set.set_file("track.ogg", data),
            move || s3_count.read_ranges().len(),
        )
        .await?;
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("object store scan timed out")??;
    Ok(())
}

#[tokio::test]
async fn last_modified_mix_sort_uses_opendal_modification_times() -> Result<()> {
    let audio = std::fs::read(fixtures::audio_fixture())?;
    let webdav = WebDavFixture::start(vec![
        ("later.ogg".into(), audio.clone()),
        ("earlier.ogg".into(), audio),
    ])
    .await?;
    let fs = FsIo::new_opendal(webdav_config(&webdav), Handle::current())?;
    let actor = Uuid::from_u128(0x215);
    let db = scanned_database(&fs, Path::new(""), actor).await?;
    let files = media_files::Entity::find().all(&db).await?;
    assert_eq!(files.len(), 2);
    assert!(
        files
            .iter()
            .all(|file| file.last_modified.parse::<i64>().is_ok())
    );
    for file in &files {
        let modified = if file.file_name == "earlier.ogg" {
            "1735689500"
        } else {
            "1735689600"
        };
        media_files::Entity::update_many()
            .col_expr(media_files::Column::LastModified, Expr::value(modified))
            .filter(media_files::Column::Id.eq(file.id))
            .exec(&db)
            .await?;
    }
    let sorted = query_mix_media_files(
        &db,
        &RecommendationDbConnection::new(),
        vec![
            ("lib::all".into(), "true".into()),
            ("sort::last_modified".into(), "true".into()),
        ],
        0,
        10,
        &[],
    )
    .await?;
    assert_eq!(
        sorted
            .iter()
            .map(|file| file.file_name.as_str())
            .collect::<Vec<_>>(),
        ["earlier.ogg", "later.ogg"]
    );
    Ok(())
}

#[tokio::test]
async fn webdav_file_without_stored_change_token_is_processed_on_each_scan() -> Result<()> {
    let bytes = std::fs::read(fixtures::audio_fixture())?;
    let webdav = WebDavFixture::start(vec![("track.ogg".into(), bytes)]).await?;
    let fs = FsIo::new_opendal(webdav_config(&webdav), Handle::current())?;
    let actor = Uuid::from_u128(0x216);
    let db = fixtures::memory_database(actor).await?;
    let store = fixtures::open_library(db.clone(), actor, None).await?;
    fixtures::scan_library(
        &fs,
        &db,
        &actor.to_string(),
        Path::new(""),
        &store.record_store(),
    )
    .await?;
    let file = media_files::Entity::find()
        .one(&db)
        .await?
        .context("missing initially scanned file")?;
    media_files::Entity::update_many()
        .col_expr(
            media_files::Column::ChangeToken,
            Expr::value(Option::<String>::None),
        )
        .filter(media_files::Column::Id.eq(file.id))
        .exec(&db)
        .await?;
    let initial_reads = webdav.read_ranges().len();
    fixtures::scan_library(
        &fs,
        &db,
        &actor.to_string(),
        Path::new(""),
        &store.record_store(),
    )
    .await?;
    assert!(
        webdav.read_ranges().len() > initial_reads,
        "file without a change token was skipped on its next scan"
    );
    Ok(())
}
