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
use database::{actions::analysis::analysis_audio_library, entities::media_analysis};
use fsio::{FileIoError, FsIo, OpenDalConfig, test_fixtures::WebDavFixture};
use playback::{
    player::{Playable, PlaybackSource, PlaybackState, Player, PlayingItem},
    strategies::AddMode,
};
use sea_orm::{DatabaseConnection, EntityTrait, PaginatorTrait};
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
        player.terminate();
        assert!(server.write_requests().is_empty());
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("concurrent WebDAV playback and analysis timed out")??;
    Ok(())
}
