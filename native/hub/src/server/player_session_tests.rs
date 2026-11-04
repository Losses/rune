use std::sync::Mutex as StdMutex;
#[cfg(target_os = "linux")]
use std::{env, process::Command};

use database::actions::{
    downloads::cleanup_unselected_downloads,
    mixes::query_mix_media_files,
    playback_queue::{list_legacy_playback_queue, replace_playback_queue},
};
use database::entities::media_file_stats;
use database::entities::media_metadata;
use playback::{
    PlaybackMode,
    player::{PlaybackSource, PlaybackState, PlayerStatus, PlayingItem},
    strategies::AddMode,
};
use sea_orm::sea_query::Expr;
use sea_orm::{EntityTrait, Set};
#[cfg(target_os = "linux")]
use tokio::runtime::Builder;
use tokio::time::{sleep, timeout};

#[cfg(target_os = "linux")]
#[test]
fn session_threads_stop_with_node() -> Result<()> {
    const CHILD: &str = "RUNE_SESSION_TEARDOWN_CASE";
    if let Ok(case) = env::var(CHILD) {
        // Retire harness blocking workers before counting the node's threads.
        return Builder::new_multi_thread()
            .enable_all()
            .thread_keep_alive(Duration::from_millis(100))
            .build()?
            .block_on(assert_node_threads_stop(&case));
    }
    let (_, module) = module_path!().split_once("::").context("test module")?;
    let test = format!("{module}::session_threads_stop_with_node");
    for case in [
        "node-drop",
        "node-shutdown",
        "fixture-drop",
        "fixture-shutdown",
    ] {
        let status = Command::new("timeout")
            .arg("90")
            .arg(env::current_exe()?)
            .args(["--exact", &test, "--nocapture"])
            .env(CHILD, case)
            .status()?;
        assert!(status.success(), "teardown case {case} failed: {status}");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn assert_node_threads_stop(case: &str) -> Result<()> {
    // Warm up process-wide I/O services before measuring owned threads.
    let warm = tempfile::tempdir()?;
    let node = fixture(warm.path()).await?;
    node.global_params.node.shutdown().await?;
    drop(node);
    sleep(Duration::from_millis(300)).await;
    let baseline = process_thread_count()?;
    let before = process_threads()?;
    let dir = tempfile::tempdir()?;
    let node = if case.starts_with("fixture") {
        let fixture = fixture(dir.path()).await?;
        let node = fixture.global_params.node.clone();
        drop(fixture);
        node
    } else {
        let path = dir.path().to_str().context("node path")?;
        NodeContext::new(path, path, Arc::new(Quiet), false).await?
    };
    let session = node.session()?;
    let weak = Arc::downgrade(&node);
    if case.ends_with("shutdown") {
        node.shutdown().await?;
    }
    let held_player = if case.ends_with("drop") {
        Some(session.player.lock().await)
    } else {
        None
    };
    drop(node);
    timeout(Duration::from_secs(5), async {
        loop {
            let threads = process_thread_count()?;
            if weak.upgrade().is_none() && threads <= baseline {
                return Ok::<_, anyhow::Error>(());
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .with_context(|| {
        format!(
            "{case}: node or player threads survived (baseline {baseline}, now {:?}, node owners {}, before {before:?}, after {:?})",
            process_thread_count(),
            weak.strong_count(),
            process_threads(),
        )
    })??;
    // A caller may retain the session/player after the node has gone.
    drop(held_player);
    drop(session);
    Ok(())
}

#[cfg(target_os = "linux")]
fn process_thread_count() -> Result<usize> {
    Ok(fs::read_dir("/proc/self/task")?.count())
}

#[cfg(target_os = "linux")]
fn process_threads() -> Result<Vec<(String, String)>> {
    fs::read_dir("/proc/self/task")?
        .map(|entry| {
            let path = entry?.path();
            Ok((
                path.display().to_string(),
                fs::read_to_string(path.join("comm"))?,
            ))
        })
        .collect()
}

#[derive(Default)]
struct RecordedSignals(StdMutex<Vec<(String, Vec<u8>)>>);
impl Broadcaster for RecordedSignals {
    fn broadcast(&self, message: &dyn RinfRustSignal) {
        self.0
            .lock()
            .unwrap()
            .push((message.name(), message.encode_to_vec().unwrap()));
    }
}

async fn session_audio(f: &Fixture, name: &str) -> Result<i32> {
    let library = f.global_params.test_library();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
    let target = Path::new(&library.media_root).join(name);
    fs::copy(source, &target)?;
    scan_audio_file(
        &library.fsio,
        &library.main_db,
        &f.global_params.node_id,
        Path::new(&library.media_root),
        &target,
        &library.library_sync.record_store(),
    )
    .await?;
    Ok(media_files::Entity::find()
        .filter(Column::FileName.eq(name))
        .one(&*library.main_db)
        .await?
        .context("scanned session audio")?
        .id)
}

async fn wait_player(
    f: &Fixture,
    predicate: impl Fn(&PlayerStatus) -> bool,
) -> Result<PlayerStatus> {
    timeout(Duration::from_secs(10), async {
        loop {
            let status = f.global_params.player.lock().await.get_status();
            if predicate(&status) {
                return status;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("session did not reach expected state")
}

async fn queue_session(f: &Fixture, ids: &[i32]) -> Result<()> {
    let library_id = f.global_params.library_id.context("library")?.to_string();
    let player = f.global_params.player.lock().await;
    player.add_to_playlist_with_sources(
        ids.iter()
            .map(|id| {
                (
                    PlayingItem::InLibrary(*id, library_id.clone()),
                    PlaybackSource::LibraryStream,
                )
            })
            .collect(),
        AddMode::AppendToEnd,
    );
    drop(player);
    wait_player(f, |status| status.playlist.len() == ids.len()).await?;
    Ok(())
}

async fn set_session_title(f: &Fixture, id: i32, title: &str) -> Result<()> {
    media_metadata::Entity::delete_many()
        .filter(media_metadata::Column::FileId.eq(id))
        .exec(&*f.global_params.test_library().main_db)
        .await?;
    media_metadata::Entity::insert(media_metadata::ActiveModel {
        file_id: Set(id),
        position: Set(0),
        meta_key: Set("track_title".into()),
        meta_value: Set(title.into()),
        hlc_uuid: Set(Uuid::new_v4().to_string()),
        ..Default::default()
    })
    .exec(&*f.global_params.test_library().main_db)
    .await?;
    Ok(())
}

async fn playlist_update_with_titles(
    signals: &RecordedSignals,
    titles: &[&str],
) -> Result<PlaylistUpdate> {
    timeout(Duration::from_secs(5), async {
        loop {
            if let Some(update) = signals.0.lock().unwrap().iter().rev().find_map(|(name, bytes)| {
                (name == "PlaylistUpdate")
                    .then(|| rinf::deserialize::<PlaylistUpdate>(bytes).ok())
                    .flatten()
                    .filter(|update| {
                        update.items.iter().map(|item| item.title.as_str()).collect::<Vec<_>>()
                            == titles
                    })
            }) {
                return update;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("playlist update with expected title order did not arrive")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_f2_batches_playlist_metadata_per_library_in_queue_order() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let a1 = session_audio(&a, "f2-a1.ogg").await?;
    let a2 = session_audio(&a, "f2-a2.ogg").await?;
    let b = fixture_open_replica(&a, bdir.path()).await?;
    let b1 = session_audio(&b, "f2-b1.ogg").await?;
    set_session_title(&a, a1, "A1").await?;
    set_session_title(&b, b1, "B1").await?;
    set_session_title(&a, a2, "A2").await?;

    let a_library_id = a.global_params.library_id.context("library A")?.to_string();
    let b_library_id = b.global_params.library_id.context("library B")?.to_string();
    let queue = vec![
        PlayingItem::InLibrary(a1, a_library_id),
        PlayingItem::InLibrary(b1, b_library_id),
        PlayingItem::InLibrary(a2, a.global_params.library_id.context("library A")?.to_string()),
    ];
    a.global_params.player.lock().await.add_to_playlist_with_sources(
        queue.iter()
            .cloned()
            .map(|item| (item, PlaybackSource::LibraryStream))
            .collect(),
        AddMode::AppendToEnd,
    );
    wait_player(&a, |status| status.playlist == queue).await?;

    let initial = playlist_update_with_titles(&signals, &["A1", "B1", "A2"]).await?;
    assert!(initial.items.iter().all(|item| !item.unavailable));

    let removed_id = a.global_params.library_id.context("library A")?;
    LibraryRegistry::update(dir.path(), |registry| {
        registry.libraries.retain(|library| library.library_id != removed_id);
        Ok(())
    })
    .await?;
    a.global_params.player.lock().await.clear_playlist();
    a.global_params.player.lock().await.add_to_playlist_with_sources(
        queue.iter()
            .cloned()
            .map(|item| (item, PlaybackSource::LibraryStream))
            .collect(),
        AddMode::AppendToEnd,
    );
    wait_player(&a, |status| status.playlist == queue).await?;

    let unavailable = playlist_update_with_titles(
        &signals,
        &["Unknown Title", "B1", "Unknown Title"],
    )
    .await?;
    assert_eq!(
        unavailable
            .items
            .iter()
            .map(|item| item.unavailable)
            .collect::<Vec<_>>(),
        vec![true, false, true]
    );
    assert!(unavailable.items[0].artist == "Unknown Artist");
    assert!(unavailable.items[0].album == "Unknown Album");
    assert_eq!(unavailable.items[0].duration, 0.0);
    a.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_d1_survives_library_switch() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let a =
        fixture_with_library_timeout(dir.path(), Uuid::new_v4(), Duration::from_millis(50)).await?;
    a.stop_worker().await?;
    let node = a.global_params.node.clone();
    let connection = Arc::downgrade(&a.global_params.test_library().main_db);
    let id = session_audio(&a, "first.ogg").await?;
    let next = session_audio(&a, "next.ogg").await?;
    queue_session(&a, &[id, next]).await?;
    a.global_params.player.lock().await.switch(0);
    wait_player(&a, |s| s.state == PlaybackState::Playing).await?;
    a.global_params.player.lock().await.seek(0.1);
    let before = wait_player(&a, |s| s.position >= Duration::from_millis(80)).await?;
    close_ui(&a.global_params).await?;
    let b = fixture_open_replica(&a, bdir.path()).await?;
    assert!(Arc::ptr_eq(
        &a.global_params.player,
        &b.global_params.player
    ));
    let after = b.global_params.player.lock().await.get_status();
    assert_eq!(before.playlist, after.playlist);
    assert_eq!(before.item, after.item);
    assert_eq!(after.state, PlaybackState::Playing);
    assert!(after.position >= before.position);
    b.global_params.player.lock().await.pause();
    wait_player(&b, |s| s.state == PlaybackState::Paused).await?;
    drop(a);
    timeout(Duration::from_secs(5), async {
        while connection.upgrade().is_some() {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("closed UI library was not evicted from pool")?;
    b.global_params.player.lock().await.next();
    wait_player(&b, |s| {
        s.index == Some(1) && s.state == PlaybackState::Playing
    })
    .await?;
    node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_d2_restart_restores_state() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let a = fixture(dir.path()).await?;
    let id = session_audio(&a, "restart.ogg").await?;
    queue_session(&a, &[id, id]).await?;
    {
        let mut player = a.global_params.player.lock().await;
        player.set_playback_mode(PlaybackMode::RepeatAll);
        player.load(1);
    }
    wait_player(&a, |s| s.index == Some(1) && s.ready).await?;
    a.global_params.player.lock().await.seek(0.1);
    let before = wait_player(&a, |s| s.position >= Duration::from_millis(80)).await?;
    let b = reopen_fixture(&a).await?;
    let after = wait_player(&b, |s| {
        s.index == Some(1) && s.ready && s.position >= Duration::from_millis(80)
    })
    .await?;
    assert_eq!(before.playlist, after.playlist);
    assert_eq!(after.playback_mode, PlaybackMode::RepeatAll);
    assert!(after.position.abs_diff(before.position) < Duration::from_millis(50));
    b.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_d3_imports_legacy_queue_once() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let a = fixture(dir.path()).await?;
    let id = session_audio(&a, "legacy.ogg").await?;
    replace_playback_queue(&a.global_params.test_library().main_db, vec![id, id]).await?;
    a.global_params
        .node
        .session()?
        .migrate_legacy(a.global_params.test_library())
        .await?;
    wait_player(&a, |s| s.playlist.len() == 2).await?;
    a.global_params.player.lock().await.clear_playlist();
    wait_player(&a, |s| s.playlist.is_empty()).await?;
    let b = reopen_fixture(&a).await?;
    b.global_params
        .node
        .session()?
        .migrate_legacy(b.global_params.test_library())
        .await?;
    sleep(Duration::from_millis(400)).await;
    assert!(
        b.global_params
            .player
            .lock()
            .await
            .get_playlist()
            .is_empty()
    );
    // The legacy rows remain untouched; they are no longer the live queue.
    assert_eq!(
        list_legacy_playback_queue(&b.global_params.test_library().main_db).await?,
        vec![id, id]
    );
    b.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_d4_queue_queries_and_download_rules_are_library_scoped() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let a = fixture(dir.path()).await?;
    let aid = session_audio(&a, "queued.ogg").await?;
    let b = fixture_open_replica(&a, bdir.path()).await?;
    let bid = session_audio(&b, "other.ogg").await?;
    assert_eq!(aid, bid, "exercise colliding catalog integer IDs");
    queue_session(&a, &[aid]).await?;
    // A stale catalog queue must not make B's track appear queued.
    replace_playback_queue(&b.global_params.test_library().main_db, vec![bid]).await?;
    sleep(Duration::from_millis(400)).await;
    for (f, expected) in [(&a, vec![aid]), (&b, vec![])] {
        let library = f.global_params.test_library();
        let file = media_files::Entity::find()
            .one(&*library.main_db)
            .await?
            .context("download track")?;
        let managed = Path::new(&library.media_root).join("Rune Sync");
        fs::create_dir_all(&managed)?;
        let target = managed.join(&file.file_name);
        fs::copy(
            Path::new(&library.media_root).join(&file.file_name),
            &target,
        )?;
        media_files::Entity::update_many()
            .col_expr(Column::Directory, Expr::value("Rune Sync"))
            .filter(Column::Id.eq(file.id))
            .exec(&*library.main_db)
            .await?;
        let queue = f
            .global_params
            .node
            .session_queue_for_library(library.descriptor.library_id)
            .await?;
        assert_eq!(queue, expected);
        let queries = vec![("lib::queue".into(), "true".into())];
        let rows = query_mix_media_files(
            &library.main_db,
            &library.recommend_db,
            queries.clone(),
            0,
            100,
            &queue,
        )
        .await?;
        assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), expected);
        set_download_rule(&library.main_db, queries, None, true).await?;
        // Cleanup uses the actual download-rule query path. The queued file must remain selected.
        let report = cleanup_unselected_downloads(
            &library.main_db,
            &library.recommend_db,
            &library.fsio,
            Path::new(&library.media_root),
            &library.library_sync.record_store(),
            &queue,
        )
        .await?;
        assert_eq!(report.delete_failures, 0);
        assert_eq!(report.removed, usize::from(expected.is_empty()));
        assert_eq!(target.exists(), !expected.is_empty());
    }
    a.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_f4_nodes_with_matching_sqlite_filenames_keep_queues_separate() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let a = fixture(dir.path()).await?;
    let aid = session_audio(&a, "queue-a.ogg").await?;
    let b = fixture_open_replica(&a, bdir.path()).await?;
    let bid = session_audio(&b, "queue-b.ogg").await?;
    assert_eq!(aid, bid, "exercise colliding catalog integer IDs");

    let filename = |fixture: &Fixture| {
        fixture
            .global_params
            .test_library()
            .main_db
            .get_sqlite_connection_pool()
            .connect_options()
            .get_filename()
            .file_name()
            .map(ToOwned::to_owned)
    };
    assert_eq!(filename(&a), filename(&b));

    queue_session(&a, &[aid]).await?;
    sleep(Duration::from_millis(400)).await;
    for (fixture, expected) in [(&a, vec![aid]), (&b, vec![])] {
        let library = fixture.global_params.test_library();
        let queue = fixture
            .global_params
            .node
            .session_queue_for_library(library.descriptor.library_id)
            .await?;
        assert_eq!(queue, expected);
        let rows = query_mix_media_files(
            &library.main_db,
            &library.recommend_db,
            vec![("lib::queue".into(), "true".into())],
            0,
            10,
            &queue,
        )
        .await?;
        assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), expected);
    }
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_d5_stats_follow_item_library() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let a = fixture(dir.path()).await?;
    let id = session_audio(&a, "stats.ogg").await?;
    queue_session(&a, &[id]).await?;
    close_ui(&a.global_params).await?;
    let b = fixture_open_replica(&a, bdir.path()).await?;
    session_audio(&b, "collision.ogg").await?;
    b.global_params.player.lock().await.switch(0);
    timeout(Duration::from_secs(15), async {
        loop {
            let rows = media_file_stats::Entity::find()
                .all(&*a.global_params.test_library().main_db)
                .await?;
            if rows.iter().any(|s| s.played_through > 0) {
                return Ok::<_, anyhow::Error>(());
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await??;
    let wrong = media_file_stats::Entity::find()
        .all(&*b.global_params.test_library().main_db)
        .await?;
    assert!(wrong.iter().all(|s| s.played_through == 0));
    a.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_d6_removed_library_is_unavailable_without_crash() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let aid = session_audio(&a, "removed.ogg").await?;
    let b = fixture_open_replica(&a, bdir.path()).await?;
    let bid = session_audio(&b, "available.ogg").await?;
    queue_session(&a, &[aid]).await?;
    let good = PlayingItem::InLibrary(
        bid,
        b.global_params.library_id.context("library")?.to_string(),
    );
    b.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            vec![(good.clone(), PlaybackSource::LibraryStream)],
            AddMode::AppendToEnd,
        );
    wait_player(&b, |s| s.playlist.len() == 2).await?;
    let bad = a.global_params.player.lock().await.get_playlist()[0].clone();
    let descriptor = a.global_params.test_library().descriptor.clone();
    LibraryRegistry::update(dir.path(), |registry| {
        registry
            .libraries
            .retain(|lib| Some(lib.library_id) != a.global_params.library_id);
        Ok(())
    })
    .await?;
    b.global_params.player.lock().await.switch(0);
    let status = wait_player(&b, |s| {
        s.item == Some(good.clone()) && s.state == PlaybackState::Playing
    })
    .await?;
    assert!(status.unavailable.contains(&bad));
    assert_eq!(status.playlist.len(), 2);
    timeout(Duration::from_secs(3), async {
        loop {
            let reported = signals.0.lock().unwrap().iter().any(|(name, bytes)| {
                name == "PlaylistUpdate"
                    && rinf::deserialize::<PlaylistUpdate>(bytes).is_ok_and(|update| {
                        update.items.len() == 2
                            && update.items[0].unavailable
                            && !update.items[1].unavailable
                    })
            });
            if reported {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    assert!(
        signals
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|(name, _)| name != "CrashResponse")
    );
    LibraryRegistry::update(dir.path(), |registry| {
        registry.libraries.push(descriptor);
        Ok(())
    })
    .await?;
    b.global_params.player.lock().await.load(0);
    let recovered = wait_player(&b, |s| s.item == Some(bad.clone()) && s.ready).await?;
    assert!(!recovered.unavailable.contains(&bad));
    b.global_params.player.lock().await.play();
    let playing = wait_player(&b, |s| {
        s.item == Some(bad.clone()) && s.state == PlaybackState::Playing
    })
    .await?;
    assert!(!playing.unavailable.contains(&bad));
    timeout(Duration::from_secs(3), async {
        loop {
            let reported = signals.0.lock().unwrap().iter().any(|(name, bytes)| {
                name == "PlaylistUpdate"
                    && rinf::deserialize::<PlaylistUpdate>(bytes).is_ok_and(|update| {
                        update.items.len() == 2 && !update.items[0].unavailable
                    })
            });
            if reported {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    a.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_f1_repeat_all_wraps_after_failed_last_item() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let a = fixture(dir.path()).await?;
    let bad_id = session_audio(&a, "repeat-bad.ogg").await?;
    let b = fixture_open_replica(&a, bdir.path()).await?;
    let good_id = session_audio(&b, "repeat-good.ogg").await?;
    let source_library_id = a.global_params.library_id.context("library")?.to_string();
    let replica_library_id = b.global_params.library_id.context("library")?.to_string();
    let good = PlayingItem::InLibrary(good_id, replica_library_id);
    let bad = PlayingItem::InLibrary(bad_id, source_library_id);
    b.global_params.player.lock().await.add_to_playlist_with_sources(
        vec![
            (good.clone(), PlaybackSource::LibraryStream),
            (bad.clone(), PlaybackSource::LibraryStream),
        ],
        AddMode::AppendToEnd,
    );
    wait_player(&b, |s| s.playlist.len() == 2).await?;
    LibraryRegistry::update(dir.path(), |registry| {
        registry.libraries.retain(|lib| Some(lib.library_id) != a.global_params.library_id);
        Ok(())
    }).await?;
    b.global_params.player.lock().await.set_playback_mode(PlaybackMode::RepeatAll);
    b.global_params.player.lock().await.switch(1);
    wait_player(&b, |s| {
        s.item == Some(good.clone())
            && s.index == Some(0)
            && s.state == PlaybackState::Playing
    }).await?;
    a.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_f1_all_failed_items_stop_without_hanging() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bdir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let ids = [
        session_audio(&a, "all-fail-a.ogg").await?,
        session_audio(&a, "all-fail-b.ogg").await?,
    ];
    let b = fixture_open_replica(&a, bdir.path()).await?;
    let source_library_id = a.global_params.library_id.context("library")?.to_string();
    b.global_params.player.lock().await.add_to_playlist_with_sources(
        ids.iter()
            .map(|id| (PlayingItem::InLibrary(*id, source_library_id.clone()), PlaybackSource::LibraryStream))
            .collect(),
        AddMode::AppendToEnd,
    );
    wait_player(&b, |s| s.playlist.len() == ids.len()).await?;
    LibraryRegistry::update(dir.path(), |registry| {
        registry.libraries.retain(|lib| Some(lib.library_id) != a.global_params.library_id);
        Ok(())
    }).await?;
    b.global_params.player.lock().await.set_playback_mode(PlaybackMode::RepeatAll);
    b.global_params.player.lock().await.switch(0);
    let status = timeout(Duration::from_secs(5), async {
        loop {
            let status = b.global_params.player.lock().await.get_status();
            if status.unavailable.len() == ids.len() && status.state == PlaybackState::Stopped {
                return status;
            }
            sleep(Duration::from_millis(20)).await;
        }
    }).await.context("all failed tracks did not stop playback")?;
    assert_eq!(status.unavailable.len(), ids.len());
    timeout(Duration::from_secs(3), async {
        loop {
            let count = signals.0.lock().unwrap().iter().filter(|(name, bytes)| {
                name == "PlaylistUpdate"
                    && rinf::deserialize::<PlaylistUpdate>(bytes)
                        .is_ok_and(|update| update.items.iter().filter(|item| item.unavailable).count() == ids.len())
            }).count();
            if count >= 1 {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    }).await.context("unavailable playlist updates did not arrive")?;
    let updates_before = signals.0.lock().unwrap().iter().filter(|(name, bytes)| {
        name == "PlaylistUpdate"
            && rinf::deserialize::<PlaylistUpdate>(bytes)
                .is_ok_and(|update| update.items.iter().filter(|item| item.unavailable).count() == ids.len())
    }).count();
    sleep(Duration::from_millis(300)).await;
    let updates_after = signals.0.lock().unwrap().iter().filter(|(name, bytes)| {
        name == "PlaylistUpdate"
            && rinf::deserialize::<PlaylistUpdate>(bytes)
                .is_ok_and(|update| update.items.iter().filter(|item| item.unavailable).count() == ids.len())
    }).count();
    assert_eq!(updates_after, updates_before, "failed tracks kept auto-advancing");
    a.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_f3_persists_changes_without_rewriting_while_idle() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let a = fixture(dir.path()).await?;
    let id = session_audio(&a, "persist-first.ogg").await?;
    queue_session(&a, &[id]).await?;
    let store = dir.path().join("sessions/local.json");
    wait_persisted_queue(&store, 1)
        .await
        .context("initial session queue was not persisted")?;
    a.global_params.player.lock().await.switch(0);
    wait_player(&a, |status| status.state == PlaybackState::Playing).await?;
    a.global_params.player.lock().await.pause();
    wait_player(&a, |status| status.state == PlaybackState::Paused).await?;
    sleep(Duration::from_millis(350)).await;
    let before = fs::read(&store)?;
    let modified_before = fs::metadata(&store)?.modified()?;
    sleep(Duration::from_secs(1)).await;
    assert_eq!(fs::metadata(&store)?.modified()?, modified_before);
    assert_eq!(fs::read(&store)?, before);

    let second = session_audio(&a, "persist-second.ogg").await?;
    let library_id = a.global_params.library_id.context("library")?.to_string();
    a.global_params.player.lock().await.add_to_playlist_with_sources(
        vec![(
            PlayingItem::InLibrary(second, library_id),
            PlaybackSource::LibraryStream,
        )],
        AddMode::AppendToEnd,
    );
    wait_player(&a, |status| status.playlist.len() == 2).await?;
    wait_persisted_queue(&store, 2)
        .await
        .context("changed queue was not persisted within one second")?;
    a.global_params.node.shutdown().await
}

/// Waits up to a second for the session file at `store` to hold a queue of `len` items.
async fn wait_persisted_queue(store: &Path, len: usize) -> Result<()> {
    timeout(Duration::from_secs(1), async {
        loop {
            let contents = fs::read(store).unwrap_or_default();
            if serde_json::from_slice::<serde_json::Value>(&contents)
                .is_ok_and(|state| state["queue"].as_array().is_some_and(|queue| queue.len() == len))
            {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    Ok(())
}
