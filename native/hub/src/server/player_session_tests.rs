use std::sync::Mutex as StdMutex;
#[cfg(target_os = "linux")]
use std::{env, process::Command};

use database::actions::{
    downloads::cleanup_unselected_downloads,
    file::{catalog_track, file_uuids_by_id},
    mixes::query_mix_media_files,
    playback_queue::{list_legacy_playback_queue, replace_playback_queue},
};
use database::entities::media_file_stats;
use database::entities::media_files;
use database::entities::media_metadata;
use crate::{
    handlers::playback::{playing_item_from_request, playing_item_requests},
    messages::{
        InLibraryPlayingItem, LibraryItemRequest, PlayingItemRequest, RequestErrorKind,
    },
    request_error::classify,
    session::store::SessionState,
};
use playback::{
    PlaybackMode,
    player::{LibraryItem, PlaybackSource, PlaybackState, PlayerStatus, PlayingItem, UnavailableReason},
    strategies::AddMode,
};
use sea_orm::sea_query::Expr;
use sea_orm::{EntityTrait, Set};
#[cfg(target_os = "linux")]
use tokio::runtime::Builder;
use tokio::time::{Instant, sleep, timeout};

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

async fn library_item_for_id(f: &Fixture, id: i32) -> Result<LibraryItem> {
    let library = f.global_params.test_library();
    let file_uuid = file_uuids_by_id(&library.main_db, &[id]).await?
        .get(&id).copied().context("file uuid")?;
    Ok(LibraryItem { library_id: library.descriptor.library_id, file_uuid })
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
    let items = futures::future::join_all(ids.iter().map(|id| library_item_for_id(f, *id))).await.into_iter().collect::<Result<Vec<_>>>()?;
    let player = f.global_params.player.lock().await;
    player.add_to_playlist_with_sources(
        items.into_iter().map(|item| (PlayingItem::InLibrary(item), PlaybackSource::LibraryStream))
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

    let queue = vec![
        PlayingItem::InLibrary(library_item_for_id(&a, a1).await?),
        PlayingItem::InLibrary(library_item_for_id(&b, b1).await?),
        PlayingItem::InLibrary(library_item_for_id(&a, a2).await?),
    ];
    a.global_params.player.lock().await.add_to_playlist_with_sources(
        queue.iter()
            .cloned()
            .map(|item| (item, PlaybackSource::LibraryStream))
            .collect(),
        AddMode::AppendToEnd,
    );
    wait_player(&a, |status| status.playlist == queue).await?;

    a.global_params.player.lock().await.switch(0);
    wait_player(&a, |status| status.item == Some(queue[0].clone()) && status.state == PlaybackState::Playing).await?;
    a.global_params.player.lock().await.next();
    wait_player(&a, |status| status.item == Some(queue[1].clone()) && status.state == PlaybackState::Playing).await?;
    a.global_params.player.lock().await.next();
    wait_player(&a, |status| status.item == Some(queue[2].clone()) && status.state == PlaybackState::Playing).await?;

    let initial = playlist_update_with_titles(&signals, &["A1", "B1", "A2"]).await?;
    assert!(initial.items.iter().all(|item| item.unavailable.is_none()));

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
            .map(|item| item.unavailable.is_some())
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

#[tokio::test]
async fn session_store_v1_migrates_stable_file_uuids_and_cursor() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let f = fixture(dir.path()).await?;
    let first_id = session_audio(&f, "v1-first.ogg").await?;
    let second_id = session_audio(&f, "v1-second.ogg").await?;
    let library_id = f.global_params.library_id.context("library")?;
    let first_uuid = library_item_for_id(&f, first_id).await?.file_uuid;
    let second_uuid = library_item_for_id(&f, second_id).await?.file_uuid;
    let session_path = dir.path().join("v1-session.json");
    let state = serde_json::json!({
        "version": 1,
        "migrated": true,
        "queue": [
            {"type":"in_library","id":first_id,"library_id":Uuid::new_v4().to_string()},
            {"type":"in_library","id":first_id,"library_id":library_id.to_string()},
            {"type":"in_library","id":second_id,"library_id":library_id.to_string()}
        ],
        "index": 2,
        "position": serde_json::to_value(Duration::from_secs(0))?,
        "mode": 0
    });
    std::fs::write(&session_path, serde_json::to_vec(&state)?)?;
    let migrated = SessionState::read(&session_path, &f.global_params.node.library_pool).await?;
    assert_eq!(migrated.index, Some(1));
    let tracks = migrated.tracks();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].0, PlayingItem::InLibrary(LibraryItem { library_id, file_uuid: first_uuid }));
    assert_eq!(tracks[1].0, PlayingItem::InLibrary(LibraryItem { library_id, file_uuid: second_uuid }));
    let written: serde_json::Value = serde_json::from_slice(&std::fs::read(&session_path)?)?;
    assert_eq!(written["version"], 3);
    assert!(written["queue"][0].get("file_uuid").is_some());
    let mut current_dropped = state.clone();
    current_dropped["index"] = serde_json::json!(0);
    std::fs::write(&session_path, serde_json::to_vec(&current_dropped)?)?;
    let migrated = SessionState::read(&session_path, &f.global_params.node.library_pool).await?;
    assert_eq!(migrated.index, None, "a dropped current item must clear the cursor");
    let mut unknown = state;
    unknown["version"] = serde_json::json!(77);
    std::fs::write(&session_path, serde_json::to_vec(&unknown)?)?;
    assert!(SessionState::read(&session_path, &f.global_params.node.library_pool).await.is_err());
    f.global_params.node.shutdown().await
}

#[tokio::test]
async fn playback_boundary_uses_current_library_and_maps_online_to_unknown_shape() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let f = fixture(dir.path()).await?;
    let id = session_audio(&f, "boundary.ogg").await?;
    let expected = library_item_for_id(&f, id).await?;
    let library = f.global_params.test_library();
    let parsed = playing_item_from_request(
        PlayingItemRequest {
            in_library: Some(InLibraryPlayingItem { file_id: id }),
            independent_file: None,
            library_item: None,
        },
        Some(library),
    ).await?;
    assert_eq!(parsed, PlayingItem::InLibrary(expected));
    let current_library_id = library.descriptor.library_id;
    let other = LibraryItem {
        library_id: Uuid::new_v4(),
        file_uuid: expected.file_uuid,
    };
    let outgoing = playing_item_requests(
        &[
            PlayingItem::InLibrary(expected),
            PlayingItem::InLibrary(other),
            PlayingItem::Online("https://example.invalid/audio".into(), None),
        ],
        &f.global_params.node.library_pool,
        Some(current_library_id),
    ).await;
    assert_eq!(outgoing[0].in_library.as_ref().map(|item| item.file_id), Some(id));
    assert!(outgoing[0].library_item.is_none());
    assert!(outgoing[1].in_library.is_none());
    assert_eq!(outgoing[1].library_item.as_ref().map(|item| item.library_id.clone()), Some(other.library_id.to_string()));
    assert_eq!(outgoing[1].library_item.as_ref().map(|item| item.file_uuid.clone()), Some(other.file_uuid.to_string()));
    assert!(outgoing[2].in_library.is_none());
    assert!(outgoing[2].independent_file.is_none());
    assert!(outgoing[2].library_item.is_none());

    let without_ui = playing_item_requests(
        &[PlayingItem::InLibrary(expected)],
        &f.global_params.node.library_pool,
        None,
    ).await;
    assert!(without_ui[0].in_library.is_none());
    assert!(without_ui[0].library_item.is_some());

    let other_request = PlayingItemRequest {
        in_library: None,
        independent_file: None,
        library_item: Some(LibraryItemRequest {
            library_id: other.library_id.to_string(),
            file_uuid: other.file_uuid.to_string(),
        }),
    };
    assert_eq!(
        playing_item_from_request(other_request, None).await?,
        PlayingItem::InLibrary(other),
    );
    let malformed = PlayingItemRequest {
        in_library: None,
        independent_file: None,
        library_item: Some(LibraryItemRequest {
            library_id: other.library_id.to_string(),
            file_uuid: "bad-file-uuid".into(),
        }),
    };
    let error = playing_item_from_request(malformed, None).await.unwrap_err();
    assert_eq!(classify(&error), RequestErrorKind::InvalidInput);
    f.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn playlist_update_uses_library_item_after_switching_ui_library() -> Result<()> {
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
    let a_id = session_audio(&a, "boundary-switch.ogg").await?;
    let a_item = library_item_for_id(&a, a_id).await?;
    let b = fixture_open_replica(&a, bdir.path()).await?;
    assert_ne!(a_item.library_id, b.global_params.test_library().descriptor.library_id);

    a.global_params.player.lock().await.add_to_playlist_with_sources(
        vec![(PlayingItem::InLibrary(a_item), PlaybackSource::LibraryStream)],
        AddMode::AppendToEnd,
    );
    wait_player(&a, |status| status.playlist == [PlayingItem::InLibrary(a_item)]).await?;
    a.global_params.player.lock().await.switch(0);
    wait_player(&a, |status| status.item == Some(PlayingItem::InLibrary(a_item))).await?;
    let update = timeout(Duration::from_secs(5), async {
        loop {
            if let Some(update) = signals.0.lock().unwrap().iter().rev().find_map(
                |(name, bytes)| {
                    (name == "PlaylistUpdate")
                        .then(|| rinf::deserialize::<PlaylistUpdate>(bytes).ok())
                        .flatten()
                        .filter(|update| {
                            update.items.iter().any(|item| {
                                item.item.library_item.as_ref().is_some_and(|identity| {
                                    identity.library_id == a_item.library_id.to_string()
                                        && identity.file_uuid == a_item.file_uuid.to_string()
                                })
                            })
                        })
                },
            ) {
                break update;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("playlist update with stable library item did not arrive")?;
    assert_eq!(update.items.len(), 1);
    assert!(update.items[0].item.in_library.is_none());
    let library_item = update.items[0]
        .item
        .library_item
        .as_ref()
        .context("other-library queue item was not sent by stable identity")?;
    assert_eq!(library_item.library_id, a_item.library_id.to_string());
    assert_eq!(library_item.file_uuid, a_item.file_uuid.to_string());
    let status = timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = signals.0.lock().unwrap().iter().rev().find_map(
                |(name, bytes)| {
                    (name == "PlaybackStatus")
                        .then(|| rinf::deserialize::<PlaybackStatus>(bytes).ok())
                        .flatten()
                        .filter(|status| {
                            status.item_request.as_ref().is_some_and(|request| {
                                request.library_item.as_ref().is_some_and(|identity| {
                                    identity.library_id == a_item.library_id.to_string()
                                        && identity.file_uuid == a_item.file_uuid.to_string()
                                })
                            })
                        })
                },
            ) {
                break status;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("playback status with stable library item did not arrive")?;
    assert!(status.item_request.unwrap().in_library.is_none());
    a.global_params.node.shutdown().await
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
    use crate::messages::PlaylistUnavailableReason;
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
    let good = PlayingItem::InLibrary(library_item_for_id(&b, bid).await?);
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
    assert_eq!(
        status.unavailable_reasons.get(&bad),
        Some(&UnavailableReason::LibraryUnavailable)
    );
    assert_eq!(status.playlist.len(), 2);
    timeout(Duration::from_secs(3), async {
        loop {
            let reported = signals.0.lock().unwrap().iter().any(|(name, bytes)| {
                name == "PlaylistUpdate"
                    && rinf::deserialize::<PlaylistUpdate>(bytes).is_ok_and(|update| {
                        update.items.len() == 2
                            && update.items[0].unavailable == Some(PlaylistUnavailableReason::LibraryUnavailable)
                            && update.items[1].unavailable.is_none()
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
                        update.items.len() == 2 && update.items[0].unavailable.is_none()
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
    let good = PlayingItem::InLibrary(library_item_for_id(&b, good_id).await?);
    let bad = PlayingItem::InLibrary(library_item_for_id(&a, bad_id).await?);
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
    b.global_params.player.lock().await.add_to_playlist_with_sources(
        futures::future::join_all(ids.iter().map(|id| library_item_for_id(&a, *id))).await.into_iter().collect::<Result<Vec<_>>>()?.into_iter()
            .map(|item| (PlayingItem::InLibrary(item), PlaybackSource::LibraryStream))
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
    let settled = b.global_params.player.lock().await.get_status();
    assert_eq!(settled.item, None);
    assert_eq!(settled.index, None);
    sleep(Duration::from_millis(300)).await;
    let after_quiet_period = b.global_params.player.lock().await.get_status();
    assert_eq!(after_quiet_period.state, PlaybackState::Stopped);
    assert_eq!(after_quiet_period.index, settled.index);
    assert_eq!(after_quiet_period.item, settled.item);
    assert_eq!(after_quiet_period.playlist, settled.playlist);
    assert_eq!(after_quiet_period.unavailable, settled.unavailable);
    assert_eq!(after_quiet_period.unavailable.len(), ids.len());
    a.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deleted_queued_uuid_is_unreadable_and_playback_advances() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let f = fixture(dir.path()).await?;
    let ids = [
        session_audio(&f, "delete-first.ogg").await?,
        session_audio(&f, "delete-middle.ogg").await?,
        session_audio(&f, "delete-last.ogg").await?,
    ];
    let queued = futures::future::join_all(ids.iter().map(|id| library_item_for_id(&f, *id))).await.into_iter().collect::<Result<Vec<_>>>()?;
    f.global_params.player.lock().await.add_to_playlist_with_sources(
        queued.iter().copied().map(|item| (PlayingItem::InLibrary(item), PlaybackSource::LibraryStream)).collect(),
        AddMode::AppendToEnd,
    );
    wait_player(&f, |status| status.playlist.len() == 3).await?;
    media_files::Entity::delete_by_id(ids[1]).exec(&*f.global_params.test_library().main_db).await?;
    f.global_params.player.lock().await.switch(0);
    wait_player(&f, |status| status.item == Some(PlayingItem::InLibrary(queued[0])) && status.state == PlaybackState::Playing).await?;
    f.global_params.player.lock().await.next();
    let advanced = wait_player(&f, |status| status.item == Some(PlayingItem::InLibrary(queued[2])) && status.state == PlaybackState::Playing && status.unavailable.contains(&PlayingItem::InLibrary(queued[1]))).await?;
    assert_eq!(advanced.unavailable_reasons.get(&PlayingItem::InLibrary(queued[1])), Some(&UnavailableReason::Unreadable));
    f.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scenario_revoking_host_grant_stops_remote_playback() -> Result<()> {
    use crate::server::{sync, sync_worker};
    use crate::access_policy::LibraryLevel;
    use crate::backends::remote::{connect_remote, decode_message};
    use crate::messages::{PlaylistUnavailableReason, PlaylistUpdate};
    use crate::server::sync_worker::{PeerEndpoint, test_peer_pass};
    use futures::StreamExt;

    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let c_dir = tempfile::tempdir()?;
    let b_signals = Arc::new(RecordedSignals::default());
    let a = fixture(a_dir.path()).await?;
    let b = fixture_with_broadcaster(
        b_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        b_signals.clone(),
    )
    .await?;
    let c = fixture(c_dir.path()).await?;
    let c_tracks = ["revoke-c1.ogg", "revoke-c2.ogg"];
    let mut c_ids = Vec::new();
    let mut c_track_uuids = Vec::new();
    for track in c_tracks {
        let source = if track == "revoke-c1.ogg" {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg")
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_1.ogg")
        };
        let target = c_dir.path().join(track);
        fs::copy(source, &target)?;
        scan_audio_file(
            &c.global_params.test_library().fsio,
            &c.global_params.test_library().main_db,
            &c.global_params.node_id,
            c_dir.path(),
            &target,
            &c.global_params.test_library().library_sync.record_store(),
        )
        .await?;
        let row = media_files::Entity::find()
                .filter(Column::FileName.eq(track))
                .one(&*c.global_params.test_library().main_db)
                .await?
                .context("host fixture track")?;
        c_ids.push(row.id);
        c_track_uuids.push(row.hlc_uuid);
    }
    let b_local_id = session_audio(&b, "revoke-b-local.ogg").await?;

    let c_addr = peer_step("pair B with C", paired_full(&c, &b)).await?;
    let c_info = peer_step("read C identity", info(&c)).await?;
    let b_info = peer_step("read B identity", info(&b)).await?;
    let c_library_id = c.global_params.library_id.context("C library id")?;
    b.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &c_info.fingerprint,
            vec![format!("127.0.0.1:{}", c_addr.port())],
        )
        .await?;
    let mut initial_grant = c
        .global_params
        .node
        .access_policy
        .grants(&b_info.fingerprint)?;
    initial_grant
        .libraries
        .insert(c_library_id, LibraryLevel::ReadWrite);
    c.global_params
        .node
        .access_policy
        .set(&b_info.fingerprint, initial_grant)?;
    let listed = peer_step(
        "list C libraries on B",
        request_peer_libraries(&b, &c_info.fingerprint),
    )
    .await?;
    let hosted = listed
        .libraries
        .iter()
        .find(|library| library.library_id == c.global_params.library_id.unwrap().to_string())
        .with_context(|| {
            format!(
                "C library was not listed (error {:?}, libraries {:?})",
                listed.error_kind, listed.libraries
            )
        })?;
    let replica_path = peer_step("add C library replica on B", async {
        request_add_library(&b, &c_info.fingerprint, hosted).await
    })
    .await?;
    let b_replica = fixture_open_replica(&b, Path::new(&replica_path)).await?;
    let c_endpoint = PeerEndpoint {
        fingerprint: c_info.fingerprint.clone(),
        trusted_host: format!("127.0.0.1:{}", c_addr.port()),
        host: "127.0.0.1".into(),
        port: c_addr.port(),
    };
    peer_step_with_timeout("sync C metadata to B", Duration::from_secs(30), async {
        loop {
            match test_peer_pass(
                b_replica.global_params.clone(),
                &b_replica.private_key,
                c_endpoint.clone(),
                true,
            )
            .await?
            {
                PeerPass::Complete => break,
                PeerPass::Incomplete => tokio::task::yield_now().await,
                result => anyhow::bail!("unexpected C metadata pass: {result:?}"),
            }
        }
        Ok(())
    })
    .await?;
    let b_c_ids = peer_step("resolve C replica track ids on B", async {
        let mut ids = Vec::new();
        for track_uuid in &c_track_uuids {
            ids.push(
                media_files::Entity::find()
                    .filter(Column::HlcUuid.eq(track_uuid))
                    .one(&*b_replica.global_params.test_library().main_db)
                    .await?
                    .context("replicated host track")?
                    .id,
            );
        }
        Ok(ids)
    })
    .await?;
    for id in &b_c_ids {
        let row = media_files::Entity::find_by_id(*id)
            .one(&*b_replica.global_params.test_library().main_db)
            .await?
            .context("replicated host row")?;
        assert!(!row.local_available, "C audio must remain remote only on B");
    }

    let a_addr = peer_step("pair A with B", paired_full(&b, &a)).await?;
    let mut controller = peer_step("connect A controller socket", async {
        Ok(connect_remote(
            "127.0.0.1",
            a_addr.port(),
            &a.global_params.config_path,
            a.global_params.cert_validator.read().await.clone(),
        )
        .await?)
    })
    .await?;
    let mut grant = c.global_params.node.access_policy.grants(&b_info.fingerprint)?;
    grant.libraries.insert(c_library_id, LibraryLevel::NotShared);
    let c_blob = catalog_track(
        &c.global_params.test_library().main_db,
        c_ids[0],
    )
    .await?
    .context("host audio blob")?;
    let (base, b_client) = peer_step("authenticate B audio client before playback", async {
        sync_worker::authenticated_client(
            &b.global_params.node,
            &b_info.fingerprint,
            &b.private_key,
            &c_info.fingerprint,
            "127.0.0.1",
            c_addr.port(),
        )
        .await
    })
    .await?;
    let audio_url = format!(
        "{}/sync/blobs/{}?track_id={}",
        sync::library_base(&base, c_library_id),
        c_blob.1.digest,
        c_blob.0
    );
    peer_step("verify B can request C audio before playback", async {
        let response = b_client
            .get(&audio_url)
            .header(reqwest::header::RANGE, "bytes=0-31")
            .send()
            .await?;
        anyhow::ensure!(
            response.status() == reqwest::StatusCode::PARTIAL_CONTENT,
            "pre-revocation range request returned {}",
            response.status()
        );
        Ok(())
    })
    .await?;
    let queue = futures::future::join_all(b_c_ids.iter().map(|id| library_item_for_id(&c, *id))).await.into_iter().collect::<Result<Vec<_>>>()?.into_iter().map(|item| (PlayingItem::InLibrary(item), PlaybackSource::LibraryStream)).collect();
    b.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(queue, AddMode::AppendToEnd);
    b.global_params.player.lock().await.switch(0);
    let first = PlayingItem::InLibrary(LibraryItem { library_id: c_library_id, file_uuid: library_item_for_id(&c, b_c_ids[0]).await?.file_uuid });
    let second = PlayingItem::InLibrary(LibraryItem { library_id: c_library_id, file_uuid: library_item_for_id(&c, b_c_ids[1]).await?.file_uuid });
    let playing_result = peer_step_with_timeout("B starts remote C track", Duration::from_secs(35), async {
        loop {
            let status = b.global_params.player.lock().await.get_status();
            if status.item == Some(first.clone()) && status.state == PlaybackState::Playing {
                return Ok(status);
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    let playing = match playing_result {
        Ok(playing) => playing,
        Err(error) => {
            let status = b.global_params.player.lock().await.get_status();
            let crashed = b_signals
                .0
                .lock()
                .unwrap()
                .iter()
                .any(|(name, _)| name == "CrashResponse");
            return Err(error.context(format!(
                "latest B player status: {status:?}; crash response recorded: {crashed}"
            )));
        }
    };
    assert_eq!(playing.index, Some(0));
    assert!(playing
        .path
        .as_deref()
        .is_some_and(|path| path.as_os_str().is_empty()));
    let first_status_item = first.to_string();
    peer_step("revoke B read grant on C library", async {
        c.global_params
            .node
            .access_policy
            .set(&b_info.fingerprint, grant)?;
        Ok(())
    })
    .await?;
    peer_step_with_timeout("B stops after C stream revocation", Duration::from_secs(20), async {
        let deadline = Instant::now() + Duration::from_secs(19);
        loop {
            let status = b.global_params.player.lock().await.get_status();
            if status.state == PlaybackState::Stopped
                && status.item.is_none()
                && status.unavailable.contains(&first)
                && status.unavailable.contains(&second)
            {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                anyhow::bail!("player did not settle after revocation: {status:?}");
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    peer_step_with_timeout("A receives B playing status", Duration::from_secs(5), async {
        loop {
            let frame = controller
                .next()
                .await
                .context("controller socket closed")??;
            if let Some((name, payload, _)) = decode_message(&frame.into_data())
                && name == "PlaybackStatus"
            {
                let status: PlaybackStatus = rinf::deserialize(&payload)?;
                if status.state == "Playing"
                    && status.item.as_deref() == Some(first_status_item.as_str())
                {
                    return Ok(());
                }
            }
        }
    })
    .await?;
    let controller_update = peer_step_with_timeout(
        "A receives stopped status or unavailable playlist update",
        Duration::from_secs(5),
        async {
            loop {
                let frame = controller
                    .next()
                    .await
                    .context("controller socket closed")??;
                if let Some((name, payload, _)) = decode_message(&frame.into_data()) {
                    if name == "PlaybackStatus" {
                        let status: PlaybackStatus = rinf::deserialize(&payload)?;
                        if status.state == "Stopped" && status.item.is_none() {
                            return Ok(());
                        }
                    } else if name == "PlaylistUpdate" {
                        let update: PlaylistUpdate = rinf::deserialize(&payload)?;
                        if update.items.iter().filter(|item| item.unavailable == Some(PlaylistUnavailableReason::AccessRevoked)).count() == 2 {
                            return Ok(());
                        }
                    }
                }
            }
        },
    )
    .await;
    if let Err(error) = controller_update {
        let local_statuses: Vec<_> = b_signals
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == "PlaybackStatus")
            .filter_map(|(_, payload)| rinf::deserialize::<PlaybackStatus>(payload).ok())
            .map(|status| (status.state, status.item, status.ready))
            .collect();
        return Err(error.context(format!(
            "B's locally recorded playback status signals: {local_statuses:?}"
        )));
    }
    peer_step("C denies a new ranged audio request from B", async {
        let response = b_client
            .get(&audio_url)
            .header(reqwest::header::RANGE, "bytes=0-31")
            .send()
            .await?;
        anyhow::ensure!(
            response.status() == reqwest::StatusCode::FORBIDDEN
                || response.status() == reqwest::StatusCode::UNAUTHORIZED,
            "revoked range request returned {}",
            response.status()
        );
        Ok(())
    })
    .await?;
    assert!(
        b_signals
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|(name, _)| name != "CrashResponse"),
        "expected access revocation must not crash the player"
    );

    let local = PlayingItem::InLibrary(library_item_for_id(&b, b_local_id).await?);
    b.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            vec![(local.clone(), PlaybackSource::LibraryStream)],
            AddMode::AppendToEnd,
        );
    b.global_params.player.lock().await.switch(2);
    peer_step_with_timeout("B player continues with local audio", Duration::from_secs(15), async {
        loop {
            let status = b.global_params.player.lock().await.get_status();
            if status.item == Some(local.clone()) && status.state == PlaybackState::Playing {
                return Ok(());
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    peer_step_with_timeout("A receives post-revocation playback update", Duration::from_secs(5), async {
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(2), controller.next())
                .await
                .context("controller update timed out")?
                .context("controller socket closed")??;
            if let Some((name, payload, _)) = decode_message(&frame.into_data())
                && name == "PlaybackStatus"
            {
                let status: PlaybackStatus = rinf::deserialize(&payload)?;
                if status.item == Some(local.to_string()) {
                    return Ok(());
                }
            }
        }
    })
    .await?;
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    c.global_params.node.shutdown().await?;
    Ok(())
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
    a.global_params.player.lock().await.add_to_playlist_with_sources(
        vec![(
            PlayingItem::InLibrary(library_item_for_id(&a, second).await?),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_c2a_revision_conflicts_and_identity_persist() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let track = session_audio(&fixture, "revision-track.ogg").await?;
    let item = PlayingItem::InLibrary(library_item_for_id(&fixture, track).await?);
    let session = fixture.global_params.node.session()?;
    let session_id = session.session_id().await;
    session
        .edit_queue(None, |player| {
            player.add_to_playlist_with_sources(
                vec![(item.clone(), PlaybackSource::LibraryStream)],
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    wait_player(&fixture, |status| status.playlist == [item.clone()]).await?;
    assert_eq!(session.queue_revision().await, 1);

    fixture.global_params.player.lock().await.play();
    fixture.global_params.player.lock().await.pause();
    fixture.global_params.player.lock().await.seek(0.0);
    assert_eq!(session.queue_revision().await, 1);

    let stale = session
        .edit_queue(Some(0), |player| {
            player.remove_from_playlist(0);
            true
        })
        .await
        .unwrap_err();
    assert_eq!(classify(&stale), RequestErrorKind::Conflict);
    assert_eq!(fixture.global_params.player.lock().await.get_playlist(), [item]);

    session
        .edit_queue(None, |player| {
            player.remove_from_playlist(0);
            true
        })
        .await?;
    wait_player(&fixture, |status| status.playlist.is_empty()).await?;
    assert_eq!(session.queue_revision().await, 2);

    let session_path = dir.path().join("sessions/local.json");
    fixture.global_params.node.shutdown().await?;
    let reopened = NodeContext::new(
        &fixture.global_params.config_path,
        &fixture.global_params.config_path,
        Arc::new(Quiet),
        false,
    )
    .await?;
    assert_eq!(reopened.session()?.session_id().await, session_id);

    let mut legacy: serde_json::Value = serde_json::from_slice(&fs::read(session_path)?)?;
    legacy["version"] = 2.into();
    legacy.as_object_mut().context("session JSON object")?.remove("session_id");
    legacy.as_object_mut().context("session JSON object")?.remove("queue_revision");
    let v2_path = dir.path().join("sessions/v2.json");
    fs::write(&v2_path, serde_json::to_vec(&legacy)?)?;
    let migrated = SessionState::read(&v2_path, &reopened.library_pool).await?;
    assert_ne!(migrated.session_id, Uuid::nil());
    assert_eq!(migrated.queue_revision, 0);
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&fs::read(v2_path)?)?["version"], 3);
    reopened.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_c2a_two_controllers_observe_conflict_and_same_queue() -> Result<()> {
    use crate::backends::remote::encode_message;
    use crate::messages::{MovePlaylistItemRequest, PlaylistUpdate, RemoteRequestFailed, RemoveRequest};
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    async fn request_and_updates(
        socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        name: &str,
        payload: Vec<u8>,
    ) -> Result<(String, Vec<u8>, Vec<PlaylistUpdate>)> {
        let request_id = Uuid::new_v4();
        socket
            .send(Message::Binary(
                encode_message(name, &payload, Some(request_id)).into(),
            ))
            .await?;
        let mut updates = Vec::new();
        timeout(Duration::from_secs(5), async {
            loop {
                let frame = socket.next().await.context("controller socket closed")??;
                let Some((response_name, payload, response_id)) =
                    decode_message(&frame.into_data())
                else {
                    continue;
                };
                if response_name == "PlaylistUpdate" {
                    updates.push(rinf::deserialize(&payload)?);
                }
                if response_id == request_id {
                    return Ok::<_, anyhow::Error>((response_name, payload, updates));
                }
            }
        })
        .await?
    }

    async fn wait_revision(
        socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        mut updates: Vec<PlaylistUpdate>,
        revision: u64,
    ) -> Result<PlaylistUpdate> {
        timeout(Duration::from_secs(5), async {
            loop {
                if let Some(index) = updates.iter().position(|update| update.revision == revision) {
                    return Ok(updates.swap_remove(index));
                }
                let frame = socket.next().await.context("controller socket closed")??;
                if let Some((name, payload, _)) = decode_message(&frame.into_data())
                    && name == "PlaylistUpdate"
                {
                    updates.push(rinf::deserialize(&payload)?);
                }
            }
        })
        .await?
    }

    let renderer_dir = tempfile::tempdir()?;
    let first_controller_dir = tempfile::tempdir()?;
    let second_controller_dir = tempfile::tempdir()?;
    let renderer = fixture(renderer_dir.path()).await?;
    let first_controller = fixture(first_controller_dir.path()).await?;
    let second_controller = fixture(second_controller_dir.path()).await?;
    let (mut first, mut second) =
        connect_two_controllers(&renderer, &first_controller, &second_controller).await?;

    let first_track = session_audio(&renderer, "conflict-first.ogg").await?;
    let second_track = session_audio(&renderer, "conflict-second.ogg").await?;
    queue_session(&renderer, &[first_track, second_track]).await?;

    let (first_name, _, first_updates) = request_and_updates(
        &mut first,
        "MovePlaylistItemRequest",
        rinf::serialize(&MovePlaylistItemRequest {
            old_index: 0,
            new_index: 1,
            expected_revision: Some(0),
        })?,
    )
    .await?;
    assert_eq!(first_name, "");
    let (second_name, failure_payload, second_updates) = request_and_updates(
        &mut second,
        "RemoveRequest",
        rinf::serialize(&RemoveRequest {
            index: 0,
            expected_revision: Some(0),
        })?,
    )
    .await?;
    assert_eq!(second_name, "RemoteRequestFailed");
    let failure: RemoteRequestFailed = rinf::deserialize(&failure_payload)?;
    assert_eq!(failure.kind, RequestErrorKind::Conflict);
    let first_final = wait_revision(&mut first, first_updates, 1).await?;
    let second_final = wait_revision(&mut second, second_updates, 1).await?;
    assert_eq!(first_final.items.len(), 2);
    assert_eq!(first_final.items.len(), second_final.items.len());
    assert_eq!(first_final.session_id, second_final.session_id);
    assert_eq!(
        first_final.items.iter().map(|item| &item.title).collect::<Vec<_>>(),
        second_final.items.iter().map(|item| &item.title).collect::<Vec<_>>(),
    );
    assert_eq!(renderer.global_params.node.session()?.queue_revision().await, 1);
    renderer.global_params.node.shutdown().await?;
    first_controller.global_params.node.shutdown().await?;
    second_controller.global_params.node.shutdown().await?;
    Ok(())
}

/// Pairs both controllers with `renderer` and opens a controller WebSocket from each.
async fn connect_two_controllers(
    renderer: &Fixture,
    first: &Fixture,
    second: &Fixture,
) -> Result<(ControllerSocket, ControllerSocket)> {
    let addr = paired_full(renderer, first).await?;
    pair(renderer, second).await?;
    let mut sockets = Vec::with_capacity(2);
    for controller in [first, second] {
        sockets.push(
            connect_remote(
                "127.0.0.1",
                addr.port(),
                &controller.global_params.config_path,
                controller.global_params.cert_validator.read().await.clone(),
            )
            .await?,
        );
    }
    let second_socket = sockets.pop().context("second controller socket")?;
    let first_socket = sockets.pop().context("first controller socket")?;
    Ok((first_socket, second_socket))
}

type ControllerSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn wait_controller_count(node: &NodeContext, count: usize) -> Result<()> {
    timeout(Duration::from_secs(3), async {
        loop {
            if node.session()?.attached_controllers().len() == count {
                return Ok::<_, anyhow::Error>(());
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_c2b_attached_controllers_list_detach_and_disconnect() -> Result<()> {
    use crate::backends::remote::encode_message;
    use crate::messages::{
        DetachControllerRequest, ListControllersRequest, MovePlaylistItemRequest,
    };
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let renderer_dir = tempfile::tempdir()?;
    let first_controller_dir = tempfile::tempdir()?;
    let second_controller_dir = tempfile::tempdir()?;
    let renderer = fixture(renderer_dir.path()).await?;
    let first_controller = fixture(first_controller_dir.path()).await?;
    let second_controller = fixture(second_controller_dir.path()).await?;
    let (mut first, mut second) =
        connect_two_controllers(&renderer, &first_controller, &second_controller).await?;
    let renderer_node = &renderer.global_params.node;
    wait_controller_count(renderer_node, 2).await?;
    let list_request = ListControllersRequest {};
    let controllers = list_request
        .handle(renderer_node.clone(), None, &list_request)
        .await?
        .context("owner controller list response")?
        .controllers;
    let first_fingerprint = info(&first_controller).await?.fingerprint;
    let second_fingerprint = info(&second_controller).await?.fingerprint;
    let first_entry = controllers
        .iter()
        .find(|controller| controller.fingerprint == first_fingerprint)
        .context("first controller missing from owner list")?;
    assert!(!first_entry.alias.is_empty());
    assert!(first_entry.connected_at > 0);
    assert!(controllers
        .iter()
        .any(|controller| controller.fingerprint == second_fingerprint));

    let unknown_detach = DetachControllerRequest {
        id: Uuid::new_v4().to_string(),
    };
    let error = unknown_detach
        .handle(renderer_node.clone(), None, &unknown_detach)
        .await
        .unwrap_err();
    assert_eq!(classify(&error), RequestErrorKind::NotFound);
    let detach = DetachControllerRequest {
        id: first_entry.id.clone(),
    };
    detach
        .handle(renderer_node.clone(), None, &detach)
        .await?;
    let frame = timeout(Duration::from_secs(3), first.next())
        .await?
        .context("detached socket closed without a frame")??;
    assert!(matches!(frame, Message::Close(Some(frame)) if u16::from(frame.code) == 4409 && frame.reason == "detached"));
    wait_controller_count(renderer_node, 1).await?;

    let first_track = session_audio(&renderer, "attached-controller-first.ogg").await?;
    let second_track = session_audio(&renderer, "attached-controller-second.ogg").await?;
    queue_session(&renderer, &[first_track, second_track]).await?;
    let request_id = Uuid::new_v4();
    second
        .send(Message::Binary(
            encode_message(
                "MovePlaylistItemRequest",
                &rinf::serialize(&MovePlaylistItemRequest {
                    old_index: 0,
                    new_index: 1,
                    expected_revision: Some(0),
                })?,
                Some(request_id),
            )
            .into(),
        ))
        .await?;
    timeout(Duration::from_secs(5), async {
        loop {
            let frame = second.next().await.context("second controller disconnected")??;
            if let Some((name, payload, _)) = decode_message(&frame.into_data())
                && name == "PlaylistUpdate"
            {
                let update: PlaylistUpdate = rinf::deserialize(&payload)?;
                if update.revision == 1 {
                    assert_eq!(update.items.len(), 2);
                    return Ok::<_, anyhow::Error>(());
                }
            }
        }
    })
    .await??;
    assert_eq!(renderer_node.session()?.attached_controllers().len(), 1);

    drop(second);
    wait_controller_count(renderer_node, 0).await?;
    renderer.global_params.node.shutdown().await?;
    first_controller.global_params.node.shutdown().await?;
    second_controller.global_params.node.shutdown().await?;
    Ok(())
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
