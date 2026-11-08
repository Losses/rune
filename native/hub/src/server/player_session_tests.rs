use super::gui_listener_tests::start;
use crate::Session;
use crate::backends::remote::open_remote_library;
use crate::macros::{
    RequestScope, capability_snapshot, request_scope, request_uses_controller_library,
};
use crate::messages::{
    CapabilityReason, CloseLibraryRequest, CloseLibraryResponse, CreatePlaylistRequest,
    FetchAllPlaylistsRequest, FetchAllPlaylistsResponse, GetPlaylistByIdRequest,
    LibraryContentChanged, ListLocalLibrariesRequest, MixQuery, OperatePlaybackWithMixQueryRequest,
    PlaybackStatus, PlaylistOperateMode, RemoteConnectionLost, RemoteLibraryReconnected,
    RemoteLibraryReconnecting, RemoteLossKind, RequestCapabilities, RequestCapability,
    RequestFailedResponse, ScanAudioLibraryProgress, ScanTaskType,
};
use crate::node::ReconnectConfig;
#[cfg(target_os = "linux")]
use std::{env, process::Command};
use std::{path::PathBuf, sync::Mutex as StdMutex};
use tempfile::TempDir;

use crate::{
    access_policy::{ConnectionLevel, SyncLevel},
    backends::remote::{RendererLink, RendererLinkUse},
    handlers::playback::{
        playing_item_from_request, playing_item_requests, session_snapshot_from_status,
    },
    messages::{
        CastLibraryState, CastPreflightRequest, GetMediaFilesCountRequest, InLibraryPlayingItem,
        LibraryItemRequest, ListControllersRequest, ListRenderersRequest, PlayingItemRequest,
        PlaylistUpdate, RendererAvailability, RendererCandidate, RequestErrorKind,
        RequestFailureCause, SwitchRequest, VolumeRequest,
    },
    request_error::{ExpectedRequestError, classify},
    rpc::{
        RpcTarget, Slot, UnsupportedReason, dispatch_gui_request, run_on_target,
    },
    server::api::fetch_device_info_at_port,
    session::store::SessionState,
};
use database::actions::{
    downloads::cleanup_unselected_downloads,
    file::{catalog_track, file_ids_by_uuid, file_uuids_by_id},
    mixes::query_mix_media_files,
    playback_queue::{list_legacy_playback_queue, replace_playback_queue},
};
use database::entities::media_file_stats;
use database::entities::media_files;
use database::entities::media_metadata;
use playback::{
    PlaybackMode,
    player::{
        LibraryItem, MockPlayer, Playable, PlaybackSource, PlaybackState, PlayerStatus,
        PlayingItem, UnavailableReason,
    },
    strategies::AddMode,
};
use sea_orm::sea_query::Expr;
use sea_orm::{EntityTrait, Set};
use serde::de::DeserializeOwned;
use tokio::net::TcpListener as TokioTcpListener;
#[cfg(target_os = "linux")]
use tokio::runtime::Builder;
use tokio::time::{Instant, sleep, timeout};
use tokio_tungstenite::{accept_async, connect_async};

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

impl RecordedSignals {
    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }

    /// Every signal named `name` recorded so far, decoded.
    fn decoded<T: DeserializeOwned>(&self, name: &str) -> Vec<T> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(signal, _)| signal == name)
            .filter_map(|(_, payload)| rinf::deserialize::<T>(payload).ok())
            .collect()
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
    let file_uuid = file_uuids_by_id(&library.main_db, &[id])
        .await?
        .get(&id)
        .copied()
        .context("file uuid")?;
    Ok(LibraryItem {
        library_id: library.descriptor.library_id,
        file_uuid,
    })
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
    let items = futures::future::join_all(ids.iter().map(|id| library_item_for_id(f, *id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let player = f.global_params.player.lock().await;
    player.add_to_playlist_with_sources(
        items
            .into_iter()
            .map(|item| (PlayingItem::InLibrary(item), PlaybackSource::LibraryStream))
            .collect(),
        AddMode::AppendToEnd,
    );
    drop(player);
    wait_player(f, |status| status.playlist.len() == ids.len()).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn renderer_link_forwards_controls_routes_status_and_keeps_local_queue() -> Result<()> {
    use crate::messages::{
        MovePlaylistItemRequest, PauseRequest, PlayRequest, RendererAttachmentChanged,
    };

    let first = tempfile::tempdir()?;
    let second = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = super::fixture_with_broadcaster(
        first.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let b = super::fixture(second.path()).await?;
    let b_address = super::paired_full(&b, &a).await?;
    let b_info = super::info(&b).await?;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &b_info.fingerprint,
            vec![format!("127.0.0.1:{}", b_address.port())],
        )
        .await?;

    let a_track = session_audio(&a, "local-renderer-link.ogg").await?;
    queue_session(&a, &[a_track]).await?;
    a.global_params.player.lock().await.play();
    wait_player(&a, |status| status.state == PlaybackState::Playing).await?;
    let b_tracks = [
        session_audio(&b, "remote-renderer-first.ogg").await?,
        session_audio(&b, "remote-renderer-second.ogg").await?,
    ];
    let b_items = futures::future::join_all(b_tracks.iter().map(|id| library_item_for_id(&b, *id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let expected_first_uuid = b_items[1].file_uuid;
    b.global_params
        .node
        .session()?
        .edit_queue(None, move |player| {
            player.add_to_playlist_with_sources(
                b_items
                    .into_iter()
                    .map(|item| (PlayingItem::InLibrary(item), PlaybackSource::LibraryStream))
                    .collect(),
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    wait_player(&b, |status| status.playlist.len() == 2).await?;

    signals.0.lock().unwrap().clear();
    a.global_params
        .node
        .attach_renderer(&b_info.fingerprint)
        .await?;
    let attached = signals
        .0
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find_map(|(name, payload)| {
            (name == "RendererAttachmentChanged")
                .then(|| rinf::deserialize::<RendererAttachmentChanged>(payload).ok())
                .flatten()
        })
        .context("attach signal")?;
    assert_eq!(
        attached
            .renderer
            .as_ref()
            .map(|renderer| renderer.fingerprint.as_str()),
        Some(b_info.fingerprint.as_str())
    );
    wait_player(&a, |status| status.state == PlaybackState::Paused).await?;
    let local_session_id = a.global_params.node.session()?.session_id().await;
    let remote_session_id = b.global_params.node.session()?.session_id().await;
    let local_queue_len = a
        .global_params
        .player
        .lock()
        .await
        .get_status()
        .playlist
        .len();
    assert_eq!(local_queue_len, 1);

    let local_volume = a
        .global_params
        .node
        .session()?
        .playback_snapshot()
        .await
        .0
        .volume;
    let volume = dispatch_gui_request(&a.global_params, &VolumeRequest { volume: 0.63 })
        .await?
        .context("renderer volume response")?;
    assert_eq!(volume.volume, 0.63);
    wait_player(&b, |status| status.volume == 0.63).await?;
    assert_eq!(
        a.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .volume,
        local_volume
    );

    dispatch_gui_request(&a.global_params, &PlayRequest {}).await?;
    wait_player(&b, |status| status.state == PlaybackState::Playing).await?;
    assert_eq!(
        a.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .state,
        PlaybackState::Paused
    );
    dispatch_gui_request(&a.global_params, &PauseRequest {}).await?;
    wait_player(&b, |status| status.state == PlaybackState::Paused).await?;

    let revision = b.global_params.node.session()?.queue_revision().await;
    dispatch_gui_request(
        &a.global_params,
        &MovePlaylistItemRequest {
            old_index: 0,
            new_index: 1,
            expected_revision: Some(revision),
        },
    )
    .await?;
    wait_player(&b, |status| {
        status.playlist.first().is_some_and(|item| {
            matches!(item, PlayingItem::InLibrary(library_item) if library_item.file_uuid == expected_first_uuid)
        })
    })
    .await?;

    timeout(Duration::from_secs(5), async {
        loop {
            let has_remote = signals.0.lock().unwrap().iter().any(|(name, payload)| {
                name == "PlaylistUpdate"
                    && rinf::deserialize::<PlaylistUpdate>(payload)
                        .is_ok_and(|update| update.session_id == remote_session_id)
            });
            if has_remote {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("renderer queue update did not reach local GUI")?;
    let updates = signals
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _)| name == "PlaylistUpdate")
        .filter_map(|(_, payload)| rinf::deserialize::<PlaylistUpdate>(payload).ok())
        .collect::<Vec<_>>();
    assert!(
        updates
            .iter()
            .any(|update| update.session_id == remote_session_id)
    );
    assert!(
        !updates
            .iter()
            .any(|update| update.session_id == local_session_id)
    );

    a.global_params.node.detach_renderer().await?;
    let detached = signals
        .0
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find_map(|(name, payload)| {
            (name == "RendererAttachmentChanged")
                .then(|| rinf::deserialize::<RendererAttachmentChanged>(payload).ok())
                .flatten()
        })
        .context("detach signal")?;
    assert!(detached.renderer.is_none());
    assert_eq!(
        a.global_params
            .player
            .lock()
            .await
            .get_status()
            .playlist
            .len(),
        1
    );
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn renderer_link_does_not_forward_its_controller_list() -> Result<()> {
    use crate::messages::{ControllersUpdated, ListControllersRequest};

    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let c_dir = tempfile::tempdir()?;
    let a_signals = Arc::new(RecordedSignals::default());
    let b_signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        a_signals.clone(),
    )
    .await?;
    let b = fixture_with_broadcaster(
        b_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        b_signals.clone(),
    )
    .await?;
    let c = fixture(c_dir.path()).await?;

    let b_address = super::paired_full(&b, &a).await?;
    let b_info = super::info(&b).await?;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &b_info.fingerprint,
            vec![format!("127.0.0.1:{}", b_address.port())],
        )
        .await?;
    a.global_params
        .node
        .attach_renderer(&b_info.fingerprint)
        .await?;
    assert_eq!(
        a.global_params
            .node
            .renderer_link()?
            .as_ref()
            .map(|link| link.fingerprint()),
        Some(b_info.fingerprint.as_str())
    );
    a_signals.clear();

    super::pair(&b, &c).await?;
    let mut c_socket = super::connect_remote(
        "127.0.0.1",
        b_address.port(),
        &c.global_params.config_path,
        c.global_params.cert_validator.read().await.clone(),
        &c.global_params.node.sync_tokens,
    )
    .await?;
    wait_controller_count(&b.global_params.node, 2).await?;

    let b_list = ListControllersRequest {};
    let b_controllers = b_list
        .handle(b.global_params.node.clone(), None, &b_list)
        .await?
        .context("renderer controller list")?
        .controllers;
    let a_fingerprint = super::info(&a).await?.fingerprint;
    let c_fingerprint = super::info(&c).await?.fingerprint;
    assert!(
        b_controllers
            .iter()
            .any(|entry| entry.fingerprint == a_fingerprint)
    );
    assert!(
        b_controllers
            .iter()
            .any(|entry| entry.fingerprint == c_fingerprint)
    );

    let b_updates = b_signals.decoded::<ControllersUpdated>("ControllersUpdated");
    assert!(b_updates.iter().any(|update| {
        update
            .controllers
            .iter()
            .any(|entry| entry.fingerprint == c_fingerprint)
    }));
    assert!(
        a_signals
            .decoded::<ControllersUpdated>("ControllersUpdated")
            .is_empty()
    );

    // Only updates sent after C leaves count: B's lists from before C joined lack C too.
    b_signals.clear();
    c_socket.close(None).await?;
    wait_controller_count(&b.global_params.node, 1).await?;
    timeout(Duration::from_secs(3), async {
        loop {
            let has_removed = b_signals
                .decoded::<ControllersUpdated>("ControllersUpdated")
                .iter()
                .any(|update| {
                    update
                        .controllers
                        .iter()
                        .all(|entry| entry.fingerprint != c_fingerprint)
                });
            if has_removed {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("renderer did not publish the disconnected controller list")?;
    assert!(
        a_signals
            .decoded::<ControllersUpdated>("ControllersUpdated")
            .is_empty()
    );

    drop(c_socket);
    for fixture in [&a, &b, &c] {
        fixture.global_params.node.shutdown().await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn renderer_attach_failures_are_typed() -> Result<()> {
    use crate::request_error::{ExpectedRequestError, classify};
    use database::error::ExpectedError;

    let dir = tempfile::tempdir()?;
    let a = super::fixture(dir.path()).await?;
    let unknown = a
        .global_params
        .node
        .attach_renderer("missing-renderer-fingerprint")
        .await
        .expect_err("unknown renderer should fail");
    assert!(unknown.chain().any(|cause| cause.is::<ExpectedError>()));
    assert_eq!(classify(&unknown), RequestErrorKind::NotFound);

    let fingerprint = "a".repeat(64);
    a.global_params
        .cert_validator
        .read()
        .await
        .add_trusted_domains(["127.0.0.1:1"], &fingerprint)
        .await?;
    let unreachable = timeout(
        Duration::from_secs(20),
        a.global_params.node.attach_renderer(&fingerprint),
    )
    .await
    .context("unreachable renderer attempt timed out")?
    .expect_err("unreachable renderer should fail");
    assert!(
        unreachable
            .chain()
            .any(|cause| cause.is::<ExpectedRequestError>())
    );
    assert_eq!(classify(&unreachable), RequestErrorKind::Unavailable);
    a.global_params.node.shutdown().await?;
    Ok(())
}

async fn attach_pair_with_signals(
    a: &Fixture,
    b: &Fixture,
    signals: &Arc<RecordedSignals>,
) -> Result<String> {
    let b_address = super::paired_full(b, a).await?;
    let b_info = super::info(b).await?;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &b_info.fingerprint,
            vec![format!("127.0.0.1:{}", b_address.port())],
        )
        .await?;
    signals.0.lock().unwrap().clear();
    a.global_params
        .node
        .attach_renderer(&b_info.fingerprint)
        .await?;
    Ok(b_info.fingerprint)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn detached_renderer_restores_local_session_signals() -> Result<()> {
    use crate::messages::{RendererAttachmentChanged, RendererLinkLost};

    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let b = fixture(b_dir.path()).await?;
    let b_alias = super::info(&b).await?.alias;
    attach_pair_with_signals(&a, &b, &signals).await?;
    let local_track = session_audio(&a, "restore-after-detach.ogg").await?;
    queue_session(&a, &[local_track]).await?;
    a.global_params.node.session()?.player.lock().await.pause();
    let local_session_id = a.global_params.node.session()?.session_id().await;
    let controller = b
        .global_params
        .node
        .session()?
        .attached_controllers()
        .into_iter()
        .next()
        .context("renderer did not register its controller")?;
    b.global_params
        .node
        .session()?
        .detach_controller(Uuid::parse_str(&controller.id)?)?;

    timeout(Duration::from_secs(5), async {
        loop {
            let (loss, attachment, local_queue) = {
                let records = signals.0.lock().unwrap();
                let loss = records.iter().position(|(name, bytes)| {
                    name == "RendererLinkLost"
                        && rinf::deserialize::<RendererLinkLost>(bytes)
                            .is_ok_and(|event| event.alias == b_alias)
                });
                let attachment = records.iter().position(|(name, bytes)| {
                    name == "RendererAttachmentChanged"
                        && rinf::deserialize::<RendererAttachmentChanged>(bytes)
                            .is_ok_and(|event| event.renderer.is_none())
                });
                let local_queue = records.iter().position(|(name, bytes)| {
                    name == "PlaylistUpdate"
                        && rinf::deserialize::<PlaylistUpdate>(bytes)
                            .is_ok_and(|event| event.session_id == local_session_id)
                });
                (loss, attachment, local_queue)
            };
            if let (Some(loss), Some(attachment), Some(queue)) = (loss, attachment, local_queue) {
                assert!(loss < attachment && attachment < queue);
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("detached renderer loss did not restore the local queue")?;
    assert!(a.global_params.node.renderer_link()?.is_none());
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn removing_renderer_trust_restores_local_session() -> Result<()> {
    use crate::messages::{RendererAttachmentChanged, RendererLinkLost};

    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let b = fixture(b_dir.path()).await?;
    let alias = super::info(&b).await?.alias;
    let fingerprint = attach_pair_with_signals(&a, &b, &signals).await?;
    let local_track = session_audio(&a, "trust-revoked.ogg").await?;
    queue_session(&a, &[local_track]).await?;
    let local_session_id = a.global_params.node.session()?.session_id().await;
    signals.0.lock().unwrap().clear();

    a.global_params.node.revoke_outgoing_for(&fingerprint).await;

    timeout(Duration::from_secs(5), async {
        loop {
            let complete = {
                let records = signals.0.lock().unwrap();
                let lost = records.iter().any(|(name, bytes)| {
                    name == "RendererLinkLost"
                        && rinf::deserialize::<RendererLinkLost>(bytes).is_ok_and(|event| {
                            event.fingerprint == fingerprint && event.alias == alias
                        })
                });
                let detached = records.iter().any(|(name, bytes)| {
                    name == "RendererAttachmentChanged"
                        && rinf::deserialize::<RendererAttachmentChanged>(bytes)
                            .is_ok_and(|event| event.renderer.is_none())
                });
                let local_session = records.iter().any(|(name, bytes)| {
                    name == "PlaylistUpdate"
                        && rinf::deserialize::<PlaylistUpdate>(bytes)
                            .is_ok_and(|event| event.session_id == local_session_id)
                });
                lost && detached && local_session
            };
            if complete {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("trust revocation did not restore the local session")?;
    assert!(a.global_params.node.renderer_link()?.is_none());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopped_renderer_clears_link_and_reports_server_stopped() -> Result<()> {
    use crate::messages::{
        PlaybackStatus, RemoteConnectionLost, RendererAttachmentChanged, RendererLinkLost,
    };

    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let b = fixture(b_dir.path()).await?;
    let b_alias = super::info(&b).await?.alias;
    attach_pair_with_signals(&a, &b, &signals).await?;
    let local_track = session_audio(&a, "renderer-server-stopped.ogg").await?;
    queue_session(&a, &[local_track]).await?;
    a.global_params.node.session()?.player.lock().await.pause();
    let local_session_id = a.global_params.node.session()?.session_id().await;
    let local_session_id_text = local_session_id.to_string();
    b.global_params.node.shutdown().await?;
    timeout(Duration::from_secs(5), async {
        loop {
            let observed = {
                let records = signals.0.lock().unwrap();
                let loss = records.iter().position(|(name, bytes)| {
                    name == "RendererLinkLost"
                        && rinf::deserialize::<RendererLinkLost>(bytes).is_ok_and(|event| {
                            event.alias == b_alias && !event.fingerprint.is_empty()
                        })
                });
                let attachment = records.iter().position(|(name, bytes)| {
                    name == "RendererAttachmentChanged"
                        && rinf::deserialize::<RendererAttachmentChanged>(bytes)
                            .is_ok_and(|event| event.renderer.is_none())
                });
                let status = records.iter().position(|(name, bytes)| {
                    name == "PlaybackStatus"
                        && rinf::deserialize::<PlaybackStatus>(bytes)
                            .is_ok_and(|event| event.session_id == local_session_id_text)
                });
                let queue = records.iter().position(|(name, bytes)| {
                    name == "PlaylistUpdate"
                        && rinf::deserialize::<PlaylistUpdate>(bytes)
                            .is_ok_and(|event| event.session_id == local_session_id)
                });
                let no_legacy_loss = !records.iter().any(|(name, bytes)| {
                    name == "RemoteConnectionLost"
                        && rinf::deserialize::<RemoteConnectionLost>(bytes).is_ok()
                });
                (loss, attachment, status, queue, no_legacy_loss)
            };
            if let (Some(loss), Some(attachment), Some(status), Some(queue), no_legacy_loss) =
                observed
            {
                assert!(loss < attachment && attachment < status && status < queue);
                assert!(no_legacy_loss);
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("renderer shutdown was not reported")?;
    assert!(a.global_params.node.renderer_link()?.is_none());
    assert!(a.global_params.ui_library.lock().await.is_some());
    a.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn old_renderer_reader_cannot_clear_a_new_link() -> Result<()> {
    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let b = fixture(b_dir.path()).await?;
    let fingerprint = attach_pair_with_signals(&a, &b, &signals).await?;
    let old = a
        .global_params
        .node
        .renderer_link()?
        .context("first link")?;
    a.global_params.node.detach_renderer().await?;
    a.global_params.node.attach_renderer(&fingerprint).await?;
    let current = a
        .global_params
        .node
        .renderer_link()?
        .context("second link")?;
    a.global_params.node.renderer_link_lost(old.id()).await;
    assert_eq!(
        a.global_params
            .node
            .renderer_link()?
            .as_ref()
            .map(|link| link.id()),
        Some(current.id())
    );
    a.global_params.node.detach_renderer().await?;
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mix_query_forwarding_leaves_it_for_local_resolution() -> Result<()> {
    use crate::messages::OperatePlaybackWithMixQueryRequest;

    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = fixture_with_broadcaster(
        a_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let b = fixture(b_dir.path()).await?;
    attach_pair_with_signals(&a, &b, &signals).await?;
    let id = session_audio(&b, "mix-query-renderer.ogg").await?;
    queue_session(&b, &[id]).await?;
    let before = b.global_params.player.lock().await.get_status().playlist;
    let request = OperatePlaybackWithMixQueryRequest {
        queries: Vec::new(),
        playback_mode: 0,
        hint_position: 0,
        initial_playback_item: None,
        instantly_play: false,
        operate_mode: crate::messages::PlaylistOperateMode::AppendToEnd,
        fallback_playing_items: Vec::new(),
        expected_revision: None,
        cast_to: None,
    };
    assert!(matches!(
        RpcTarget::select(&a.global_params.node, &request)?.slot(&request),
        Slot::Native
    ));
    assert_eq!(
        b.global_params.player.lock().await.get_status().playlist,
        before
    );

    let read = GetMediaFilesCountRequest {};
    assert!(matches!(
        RpcTarget::select(&a.global_params.node, &read)?.slot(&read),
        Slot::Native
    ));
    let response = dispatch_gui_request(&a.global_params, &read)
        .await?
        .context("local library count response")?;
    assert_eq!(response.count, 0);
    a.global_params.node.detach_renderer().await?;
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attached_mix_query_casts_and_stale_edits_are_rejected() -> Result<()> {
    use crate::Signal;
    use crate::messages::{MixQuery, OperatePlaybackWithMixQueryRequest, PlaylistOperateMode};
    use crate::request_error::{ExpectedRequestError, classify};
    use crate::utils::ParamsExtractor;

    let host_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let controller_dir = tempfile::tempdir()?;
    let host = fixture(host_dir.path()).await?;
    let renderer = fixture(renderer_dir.path()).await?;
    let signals = Arc::new(RecordedSignals::default());
    let controller = fixture_with_broadcaster(
        controller_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let ids = [
        session_audio(&host, "mix-cast-first.ogg").await?,
        session_audio(&host, "mix-cast-second.ogg").await?,
        session_audio(&host, "mix-cast-local-queue.ogg").await?,
    ];
    let host_items =
        futures::future::join_all(ids[..2].iter().map(|id| library_item_for_id(&host, *id)))
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
    let (_, renderer_replica) = replicate_host_library(&host, &renderer).await?;
    let host_addr = host.get_address().await.context("host listener address")?;
    pair(&host, &controller).await?;
    pair(&controller, &host).await?;
    let (_, controller_replica) = replicate_host_library_at(&host, &controller, host_addr).await?;
    attach_pair_with_signals(&controller_replica, &renderer_replica, &signals).await?;

    queue_session(&controller_replica, &[ids[2]]).await?;
    let local_queue = controller_replica
        .global_params
        .player
        .lock()
        .await
        .get_playlist();
    let local_revision = controller_replica
        .global_params
        .node
        .session()?
        .queue_revision()
        .await;
    let controller_file_ids = file_ids_by_uuid(
        &controller_replica.global_params.test_library().main_db,
        &host_items
            .iter()
            .map(|item| item.file_uuid)
            .collect::<Vec<_>>(),
    )
    .await?;
    let request = OperatePlaybackWithMixQueryRequest {
        queries: host_items
            .iter()
            .map(|item| MixQuery {
                operator: "lib::track".into(),
                parameter: controller_file_ids[&item.file_uuid].to_string(),
            })
            .collect(),
        playback_mode: 0,
        hint_position: -1,
        initial_playback_item: None,
        instantly_play: true,
        operate_mode: PlaylistOperateMode::Replace,
        fallback_playing_items: Vec::new(),
        expected_revision: Some(0),
        cast_to: None,
    };
    let params = request.extract_params(&controller_replica.global_params);
    let response = request
        .handle(params, None, &request)
        .await?
        .context("cast mix query response")?;
    assert_eq!(response.playing_items.len(), 2);
    let response_ids = response
        .playing_items
        .iter()
        .map(|item| {
            item.in_library
                .as_ref()
                .map(|item| item.file_id)
                .context("resolved mix item is not in the controller library")
        })
        .collect::<Result<Vec<_>>>()?;
    let response_uuids = file_uuids_by_id(
        &controller_replica.global_params.test_library().main_db,
        &response_ids,
    )
    .await?;
    let expected_items = response_ids
        .iter()
        .map(|file_id| {
            response_uuids
                .get(file_id)
                .copied()
                .map(|file_uuid| {
                    PlayingItem::InLibrary(LibraryItem {
                        library_id: controller_replica
                            .global_params
                            .library_id
                            .expect("replica library id"),
                        file_uuid,
                    })
                })
                .context("resolved mix item uuid")
        })
        .collect::<Result<Vec<_>>>()?;
    let renderer_status = wait_player(&renderer_replica, |status| {
        status.playlist == expected_items && status.item == expected_items.first().cloned()
    })
    .await?;
    renderer_replica.global_params.player.lock().await.pause();
    wait_player(&renderer_replica, |status| {
        status.state == PlaybackState::Paused
    })
    .await?;
    assert_eq!(renderer_status.playlist, expected_items);
    assert_eq!(
        controller_replica
            .global_params
            .player
            .lock()
            .await
            .get_playlist(),
        local_queue
    );
    assert_eq!(
        controller_replica
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        local_revision
    );

    let renderer_revision = renderer_replica
        .global_params
        .node
        .session()?
        .queue_revision()
        .await;
    let mut stale = request;
    stale.expected_revision = Some(renderer_revision.saturating_sub(1));
    let params = stale.extract_params(&controller_replica.global_params);
    let error = match stale.handle(params, None, &stale).await {
        Err(error) => error,
        Ok(_) => anyhow::bail!("stale mix cast unexpectedly succeeded"),
    };
    assert!(
        error
            .chain()
            .any(|cause| cause.is::<ExpectedRequestError>())
    );
    assert_eq!(classify(&error), RequestErrorKind::Conflict);
    assert_eq!(
        renderer_replica
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        renderer_revision
    );
    assert_eq!(
        renderer_replica
            .global_params
            .player
            .lock()
            .await
            .get_playlist(),
        renderer_status.playlist
    );

    renderer_replica.global_params.player.lock().await.stop();
    wait_player(&renderer_replica, |status| {
        status.state == PlaybackState::Stopped
    })
    .await?;
    controller_replica
        .global_params
        .node
        .detach_renderer()
        .await?;
    renderer_replica.global_params.node.shutdown().await?;
    controller_replica.global_params.node.shutdown().await?;
    host.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attached_mix_query_rejects_items_outside_renderer_libraries() -> Result<()> {
    use crate::Signal;
    use crate::messages::{
        IndependentFilePlayingItem, OperatePlaybackWithMixQueryRequest, PlayingItemRequest,
        PlaylistOperateMode,
    };
    use crate::utils::ParamsExtractor;

    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let controller = fixture_with_broadcaster(
        controller_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let renderer = fixture(renderer_dir.path()).await?;
    let id = session_audio(&controller, "mix-cast-unheld.ogg").await?;
    attach_pair_with_signals(&controller, &renderer, &signals).await?;
    let before_queue = renderer.global_params.player.lock().await.get_playlist();
    let before_revision = renderer
        .global_params
        .node
        .session()?
        .queue_revision()
        .await;
    let independent_dir = tempfile::tempdir()?;
    let independent_path = independent_dir.path().join("outside.ogg");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg"),
        &independent_path,
    )?;
    let independent_request = OperatePlaybackWithMixQueryRequest {
        queries: Vec::new(),
        playback_mode: 0,
        hint_position: -1,
        initial_playback_item: None,
        instantly_play: false,
        operate_mode: PlaylistOperateMode::Replace,
        fallback_playing_items: vec![PlayingItemRequest {
            independent_file: Some(IndependentFilePlayingItem {
                raw_path: independent_path.to_string_lossy().into_owned(),
            }),
            in_library: None,
            library_item: None,
        }],
        expected_revision: None,
        cast_to: None,
    };
    let params = independent_request.extract_params(&controller.global_params);
    let error = match independent_request
        .handle(params, None, &independent_request)
        .await
    {
        Err(error) => error,
        Ok(_) => anyhow::bail!("cast of an independent file unexpectedly succeeded"),
    };
    assert_eq!(classify(&error), RequestErrorKind::Unavailable);
    assert!(format!("{error:#}").contains("casting files outside a library"));
    assert_eq!(
        renderer.global_params.player.lock().await.get_playlist(),
        before_queue
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        before_revision
    );

    let item = library_item_for_id(&controller, id).await?;
    let request = OperatePlaybackWithMixQueryRequest {
        queries: Vec::new(),
        playback_mode: 0,
        hint_position: -1,
        initial_playback_item: None,
        instantly_play: false,
        operate_mode: PlaylistOperateMode::Replace,
        fallback_playing_items: playing_item_requests(
            &[PlayingItem::InLibrary(item)],
            &controller.global_params.library_pool,
            controller.global_params.library_id,
        )
        .await,
        expected_revision: None,
        cast_to: None,
    };
    let params = request.extract_params(&controller.global_params);
    let error = match request.handle(params, None, &request).await {
        Err(error) => error,
        Ok(_) => anyhow::bail!("cast of an unheld library unexpectedly succeeded"),
    };
    assert_eq!(classify(&error), RequestErrorKind::PermissionDenied);
    assert_eq!(
        renderer.global_params.player.lock().await.get_playlist(),
        before_queue
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        before_revision
    );
    controller.global_params.node.shutdown().await?;
    renderer.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chosen_renderer_cast_preserves_local_playback_and_can_switch_targets() -> Result<()> {
    use crate::Signal;
    use crate::backends::remote::RendererLinkUse;
    use crate::messages::{
        CastTarget, MixQuery, OperatePlaybackWithMixQueryRequest, PlaylistOperateMode,
        RendererAttachmentChanged,
    };
    use crate::utils::ParamsExtractor;

    let host_dir = tempfile::tempdir()?;
    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let other_renderer_dir = tempfile::tempdir()?;
    let host = fixture(host_dir.path()).await?;
    let signals = Arc::new(RecordedSignals::default());
    let controller = fixture_with_broadcaster(
        controller_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let renderer_node = fixture(renderer_dir.path()).await?;
    let other_renderer = fixture(other_renderer_dir.path()).await?;
    let track_ids = [
        session_audio(&host, "target-cast-local-playing.ogg").await?,
        session_audio(&host, "target-cast-next.ogg").await?,
    ];
    let items = track_ids
        .iter()
        .map(|id| library_item_for_id(&host, *id))
        .collect::<Vec<_>>();
    let host_items = futures::future::join_all(items)
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let (host_address, controller_replica) = replicate_host_library(&host, &controller).await?;
    super::pair(&host, &renderer_node).await?;
    super::pair(&renderer_node, &host).await?;
    let (_, renderer) = replicate_host_library_at(&host, &renderer_node, host_address).await?;
    let renderer_address = super::paired_full(&renderer_node, &controller).await?;
    let renderer_info = super::info(&renderer_node).await?;
    controller_replica
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &renderer_info.fingerprint,
            vec![format!("127.0.0.1:{}", renderer_address.port())],
        )
        .await?;
    let other_address = super::paired_full(&other_renderer, &controller).await?;
    let other_info = super::info(&other_renderer).await?;
    controller_replica
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &other_info.fingerprint,
            vec![format!("127.0.0.1:{}", other_address.port())],
        )
        .await?;
    let local_file_ids = file_ids_by_uuid(
        &controller_replica.global_params.test_library().main_db,
        &host_items
            .iter()
            .map(|item| item.file_uuid)
            .collect::<Vec<_>>(),
    )
    .await?;
    let local_track_id = local_file_ids[&host_items[0].file_uuid];
    materialize_replica_file(
        &controller_replica,
        local_track_id,
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg"),
        "cast-local-playing.ogg",
    )
    .await?;
    queue_session(&controller_replica, &[local_track_id]).await?;
    controller_replica.global_params.player.lock().await.load(0);
    controller_replica.global_params.player.lock().await.play();
    let local_before = wait_player(&controller_replica, |status| {
        status.state == PlaybackState::Playing && status.item.is_some()
    })
    .await?;
    let make_query =
        |file_id: i32, cast_to: Option<CastTarget>| OperatePlaybackWithMixQueryRequest {
            queries: vec![MixQuery {
                operator: "lib::track".into(),
                parameter: file_id.to_string(),
            }],
            playback_mode: 0,
            hint_position: -1,
            initial_playback_item: None,
            instantly_play: true,
            operate_mode: PlaylistOperateMode::Replace,
            fallback_playing_items: Vec::new(),
            expected_revision: None,
            cast_to,
        };
    let cast = make_query(
        local_file_ids[&host_items[0].file_uuid],
        Some(CastTarget {
            fingerprint: renderer_info.fingerprint.clone(),
            attach: false,
        }),
    );
    let response = cast
        .handle(
            cast.extract_params(&controller_replica.global_params),
            None,
            &cast,
        )
        .await?
        .context("detached cast response")?;
    assert_eq!(response.playing_items.len(), 1);
    let renderer_status = wait_player(&renderer, |status| {
        status.state == PlaybackState::Playing
            && status.item.is_some()
            && status.playlist.len() == 1
    })
    .await?;
    assert_eq!(renderer_status.playlist.len(), 1);
    renderer.global_params.player.lock().await.pause();
    wait_player(&renderer, |status| status.state == PlaybackState::Paused).await?;
    let local_after = controller_replica
        .global_params
        .player
        .lock()
        .await
        .get_status();
    assert_eq!(local_after.item, local_before.item);
    assert_eq!(local_after.state, PlaybackState::Playing);
    assert!(
        controller_replica
            .global_params
            .node
            .renderer_link()?
            .is_none()
    );
    assert!(
        !signals
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|(name, _)| name == "RendererAttachmentChanged")
    );
    // A one-off cast link must not show the renderer's session in this device's
    // UI: hold one open while the renderer's state changes.
    let renderer_session_id = renderer.global_params.node.session()?.session_id().await;
    let (cast_link, _) = controller_replica
        .global_params
        .node
        .connect_renderer(&renderer_info.fingerprint, RendererLinkUse::Cast)
        .await?;
    renderer.global_params.player.lock().await.play();
    wait_player(&renderer, |status| status.state == PlaybackState::Playing).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    renderer.global_params.player.lock().await.pause();
    wait_player(&renderer, |status| status.state == PlaybackState::Paused).await?;
    cast_link.close();
    drop(cast_link);
    assert!(
        !signals
            .0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(name, payload)| match name.as_str() {
                "PlaylistUpdate" => rinf::deserialize::<PlaylistUpdate>(payload)
                    .ok()
                    .map(|update| update.session_id),
                "PlaybackStatus" => rinf::deserialize::<PlaybackStatus>(payload)
                    .ok()
                    .map(|status| status.session_id),
                _ => None,
            })
            .any(|session_id| session_id == renderer_session_id)
    );

    let attach_cast = make_query(
        local_file_ids[&host_items[1].file_uuid],
        Some(CastTarget {
            fingerprint: renderer_info.fingerprint.clone(),
            attach: true,
        }),
    );
    attach_cast
        .handle(
            attach_cast.extract_params(&controller_replica.global_params),
            None,
            &attach_cast,
        )
        .await?;
    let attached_cast_status = wait_player(&renderer, |status| {
        status.state == PlaybackState::Playing
            && status.item == Some(PlayingItem::InLibrary(host_items[1]))
    })
    .await?;
    renderer.global_params.player.lock().await.pause();
    wait_player(&renderer, |status| status.state == PlaybackState::Paused).await?;
    assert_eq!(
        controller_replica
            .global_params
            .node
            .renderer_link()?
            .as_ref()
            .map(|link| link.fingerprint()),
        Some(renderer_info.fingerprint.as_str())
    );
    assert_eq!(
        attached_cast_status.item,
        Some(PlayingItem::InLibrary(host_items[1]))
    );

    let append = OperatePlaybackWithMixQueryRequest {
        queries: Vec::new(),
        playback_mode: 0,
        hint_position: -1,
        initial_playback_item: None,
        instantly_play: false,
        operate_mode: PlaylistOperateMode::PlayNext,
        fallback_playing_items: vec![PlayingItemRequest {
            library_item: Some(host_items[1].into()),
            in_library: None,
            independent_file: None,
        }],
        expected_revision: None,
        cast_to: Some(CastTarget {
            fingerprint: renderer_info.fingerprint.clone(),
            attach: false,
        }),
    };
    renderer.global_params.player.lock().await.play();
    let current = wait_player(&renderer, |status| status.state == PlaybackState::Playing).await?;
    append
        .handle(
            append.extract_params(&controller_replica.global_params),
            None,
            &append,
        )
        .await?;
    let after_next = wait_player(&renderer, |status| status.playlist.len() == 2).await?;
    assert_eq!(after_next.state, PlaybackState::Playing);
    assert_eq!(after_next.item, current.item);
    assert_eq!(
        after_next.playlist[1],
        PlayingItem::InLibrary(host_items[1])
    );
    renderer.global_params.player.lock().await.pause();
    wait_player(&renderer, |status| status.state == PlaybackState::Paused).await?;

    signals.0.lock().unwrap().clear();
    controller_replica
        .global_params
        .node
        .attach_renderer(&renderer_info.fingerprint)
        .await?;
    assert_eq!(
        signals
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == "RendererAttachmentChanged")
            .count(),
        1
    );

    signals.0.lock().unwrap().clear();
    controller_replica
        .global_params
        .node
        .attach_renderer(&other_info.fingerprint)
        .await?;
    let changes = signals
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _)| name == "RendererAttachmentChanged")
        .filter_map(|(_, payload)| rinf::deserialize::<RendererAttachmentChanged>(payload).ok())
        .collect::<Vec<_>>();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0]
            .renderer
            .as_ref()
            .map(|renderer| renderer.fingerprint.as_str()),
        Some(other_info.fingerprint.as_str())
    );
    wait_controller_count(&renderer.global_params.node, 0).await?;
    assert_eq!(
        controller_replica
            .global_params
            .node
            .renderer_link()?
            .as_ref()
            .map(|link| link.fingerprint()),
        Some(other_info.fingerprint.as_str())
    );

    let before_queue = other_renderer
        .global_params
        .player
        .lock()
        .await
        .get_playlist();
    let before_revision = other_renderer
        .global_params
        .node
        .session()?
        .queue_revision()
        .await;
    let denied_cast = make_query(
        local_file_ids[&host_items[0].file_uuid],
        Some(CastTarget {
            fingerprint: other_info.fingerprint.clone(),
            attach: false,
        }),
    );
    let error = match denied_cast
        .handle(
            denied_cast.extract_params(&controller_replica.global_params),
            None,
            &denied_cast,
        )
        .await
    {
        Err(error) => error,
        Ok(_) => anyhow::bail!("cast to a renderer without the library unexpectedly succeeded"),
    };
    assert_eq!(classify(&error), RequestErrorKind::PermissionDenied);
    assert_eq!(
        other_renderer
            .global_params
            .player
            .lock()
            .await
            .get_playlist(),
        before_queue
    );
    assert_eq!(
        other_renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        before_revision
    );
    assert_eq!(
        controller_replica
            .global_params
            .node
            .renderer_link()?
            .as_ref()
            .map(|link| link.fingerprint()),
        Some(other_info.fingerprint.as_str())
    );

    controller_replica
        .global_params
        .node
        .detach_renderer()
        .await?;
    renderer.global_params.node.shutdown().await?;
    other_renderer.global_params.node.shutdown().await?;
    host.global_params.node.shutdown().await?;
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
            if let Some(update) =
                signals
                    .0
                    .lock()
                    .unwrap()
                    .iter()
                    .rev()
                    .find_map(|(name, bytes)| {
                        (name == "PlaylistUpdate")
                            .then(|| rinf::deserialize::<PlaylistUpdate>(bytes).ok())
                            .flatten()
                            .filter(|update| {
                                update
                                    .items
                                    .iter()
                                    .map(|item| item.title.as_str())
                                    .collect::<Vec<_>>()
                                    == titles
                            })
                    })
            {
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
    a.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            queue
                .iter()
                .cloned()
                .map(|item| (item, PlaybackSource::LibraryStream))
                .collect(),
            AddMode::AppendToEnd,
        );
    wait_player(&a, |status| status.playlist == queue).await?;

    a.global_params.player.lock().await.switch(0);
    wait_player(&a, |status| {
        status.item == Some(queue[0].clone()) && status.state == PlaybackState::Playing
    })
    .await?;
    a.global_params.player.lock().await.next();
    wait_player(&a, |status| {
        status.item == Some(queue[1].clone()) && status.state == PlaybackState::Playing
    })
    .await?;
    a.global_params.player.lock().await.next();
    wait_player(&a, |status| {
        status.item == Some(queue[2].clone()) && status.state == PlaybackState::Playing
    })
    .await?;

    let initial = playlist_update_with_titles(&signals, &["A1", "B1", "A2"]).await?;
    assert!(initial.items.iter().all(|item| item.unavailable.is_none()));

    let removed_id = a.global_params.library_id.context("library A")?;
    LibraryRegistry::update(dir.path(), |registry| {
        registry
            .libraries
            .retain(|library| library.library_id != removed_id);
        Ok(())
    })
    .await?;
    a.global_params.player.lock().await.clear_playlist();
    a.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            queue
                .iter()
                .cloned()
                .map(|item| (item, PlaybackSource::LibraryStream))
                .collect(),
            AddMode::AppendToEnd,
        );
    wait_player(&a, |status| status.playlist == queue).await?;

    let unavailable =
        playlist_update_with_titles(&signals, &["Unknown Title", "B1", "Unknown Title"]).await?;
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
    assert_eq!(
        tracks[0].0,
        PlayingItem::InLibrary(LibraryItem {
            library_id,
            file_uuid: first_uuid
        })
    );
    assert_eq!(
        tracks[1].0,
        PlayingItem::InLibrary(LibraryItem {
            library_id,
            file_uuid: second_uuid
        })
    );
    let written: serde_json::Value = serde_json::from_slice(&std::fs::read(&session_path)?)?;
    assert_eq!(written["version"], 3);
    assert!(written["queue"][0].get("file_uuid").is_some());
    let mut current_dropped = state.clone();
    current_dropped["index"] = serde_json::json!(0);
    std::fs::write(&session_path, serde_json::to_vec(&current_dropped)?)?;
    let migrated = SessionState::read(&session_path, &f.global_params.node.library_pool).await?;
    assert_eq!(
        migrated.index, None,
        "a dropped current item must clear the cursor"
    );
    let mut unknown = state;
    unknown["version"] = serde_json::json!(77);
    std::fs::write(&session_path, serde_json::to_vec(&unknown)?)?;
    assert!(
        SessionState::read(&session_path, &f.global_params.node.library_pool)
            .await
            .is_err()
    );
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
    )
    .await?;
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
    )
    .await;
    assert_eq!(
        outgoing[0].in_library.as_ref().map(|item| item.file_id),
        Some(id)
    );
    assert!(outgoing[0].library_item.is_none());
    assert!(outgoing[1].in_library.is_none());
    assert_eq!(
        outgoing[1]
            .library_item
            .as_ref()
            .map(|item| item.library_id.clone()),
        Some(other.library_id.to_string())
    );
    assert_eq!(
        outgoing[1]
            .library_item
            .as_ref()
            .map(|item| item.file_uuid.clone()),
        Some(other.file_uuid.to_string())
    );
    assert!(outgoing[2].in_library.is_none());
    assert!(outgoing[2].independent_file.is_none());
    assert!(outgoing[2].library_item.is_none());

    let without_ui = playing_item_requests(
        &[PlayingItem::InLibrary(expected)],
        &f.global_params.node.library_pool,
        None,
    )
    .await;
    assert!(without_ui[0].in_library.is_none());
    assert!(without_ui[0].library_item.is_some());

    let other_request = PlayingItemRequest {
        in_library: None,
        independent_file: None,
        library_item: Some(other.into()),
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
    let error = playing_item_from_request(malformed, None)
        .await
        .unwrap_err();
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
    assert_ne!(
        a_item.library_id,
        b.global_params.test_library().descriptor.library_id
    );

    a.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            vec![(
                PlayingItem::InLibrary(a_item),
                PlaybackSource::LibraryStream,
            )],
            AddMode::AppendToEnd,
        );
    wait_player(&a, |status| {
        status.playlist == [PlayingItem::InLibrary(a_item)]
    })
    .await?;
    a.global_params.player.lock().await.switch(0);
    wait_player(&a, |status| {
        status.item == Some(PlayingItem::InLibrary(a_item))
    })
    .await?;
    let update = timeout(Duration::from_secs(5), async {
        loop {
            if let Some(update) =
                signals
                    .0
                    .lock()
                    .unwrap()
                    .iter()
                    .rev()
                    .find_map(|(name, bytes)| {
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
                    })
            {
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
            if let Some(status) =
                signals
                    .0
                    .lock()
                    .unwrap()
                    .iter()
                    .rev()
                    .find_map(|(name, bytes)| {
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
                    })
            {
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
                            && update.items[0].unavailable
                                == Some(PlaylistUnavailableReason::LibraryUnavailable)
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
    b.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            vec![
                (good.clone(), PlaybackSource::LibraryStream),
                (bad.clone(), PlaybackSource::LibraryStream),
            ],
            AddMode::AppendToEnd,
        );
    wait_player(&b, |s| s.playlist.len() == 2).await?;
    LibraryRegistry::update(dir.path(), |registry| {
        registry
            .libraries
            .retain(|lib| Some(lib.library_id) != a.global_params.library_id);
        Ok(())
    })
    .await?;
    b.global_params
        .player
        .lock()
        .await
        .set_playback_mode(PlaybackMode::RepeatAll);
    b.global_params.player.lock().await.switch(1);
    wait_player(&b, |s| {
        s.item == Some(good.clone()) && s.index == Some(0) && s.state == PlaybackState::Playing
    })
    .await?;
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
    b.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            futures::future::join_all(ids.iter().map(|id| library_item_for_id(&a, *id)))
                .await
                .into_iter()
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .map(|item| (PlayingItem::InLibrary(item), PlaybackSource::LibraryStream))
                .collect(),
            AddMode::AppendToEnd,
        );
    wait_player(&b, |s| s.playlist.len() == ids.len()).await?;
    LibraryRegistry::update(dir.path(), |registry| {
        registry
            .libraries
            .retain(|lib| Some(lib.library_id) != a.global_params.library_id);
        Ok(())
    })
    .await?;
    b.global_params
        .player
        .lock()
        .await
        .set_playback_mode(PlaybackMode::RepeatAll);
    b.global_params.player.lock().await.switch(0);
    let status = timeout(Duration::from_secs(5), async {
        loop {
            let status = b.global_params.player.lock().await.get_status();
            if status.unavailable.len() == ids.len() && status.state == PlaybackState::Stopped {
                return status;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("all failed tracks did not stop playback")?;
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
    let queued = futures::future::join_all(ids.iter().map(|id| library_item_for_id(&f, *id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    f.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
            queued
                .iter()
                .copied()
                .map(|item| (PlayingItem::InLibrary(item), PlaybackSource::LibraryStream))
                .collect(),
            AddMode::AppendToEnd,
        );
    wait_player(&f, |status| status.playlist.len() == 3).await?;
    media_files::Entity::delete_by_id(ids[1])
        .exec(&*f.global_params.test_library().main_db)
        .await?;
    f.global_params.player.lock().await.switch(0);
    wait_player(&f, |status| {
        status.item == Some(PlayingItem::InLibrary(queued[0]))
            && status.state == PlaybackState::Playing
    })
    .await?;
    f.global_params.player.lock().await.next();
    let advanced = wait_player(&f, |status| {
        status.item == Some(PlayingItem::InLibrary(queued[2]))
            && status.state == PlaybackState::Playing
            && status
                .unavailable
                .contains(&PlayingItem::InLibrary(queued[1]))
    })
    .await?;
    assert_eq!(
        advanced
            .unavailable_reasons
            .get(&PlayingItem::InLibrary(queued[1])),
        Some(&UnavailableReason::Unreadable)
    );
    f.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scenario_revoking_host_grant_stops_remote_playback() -> Result<()> {
    use crate::backends::remote::{connect_remote, decode_message};
    use crate::messages::{PlaylistUnavailableReason, PlaylistUpdate};
    use crate::server::{sync, sync_worker};
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

    let (c_addr, b_replica) = peer_step_with_timeout(
        "replicate C library on B",
        Duration::from_secs(45),
        replicate_host_library(&c, &b),
    )
    .await?;
    let c_info = info(&c).await?;
    let b_info = info(&b).await?;
    let c_library_id = c.global_params.library_id.context("C library id")?;
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
            &a.global_params.node.sync_tokens,
        )
        .await?)
    })
    .await?;
    let mut grant = c
        .global_params
        .node
        .access_policy
        .grants(&b_info.fingerprint)?;
    grant
        .connection
        .libraries
        .insert(c_library_id, ConnectionLevel::None);
    grant.sync.libraries.insert(c_library_id, SyncLevel::None);
    let c_blob = catalog_track(&c.global_params.test_library().main_db, c_ids[0])
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
    let queue = futures::future::join_all(b_c_ids.iter().map(|id| library_item_for_id(&c, *id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(|item| (PlayingItem::InLibrary(item), PlaybackSource::LibraryStream))
        .collect();
    b.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(queue, AddMode::AppendToEnd);
    b.global_params.player.lock().await.switch(0);
    let first = PlayingItem::InLibrary(LibraryItem {
        library_id: c_library_id,
        file_uuid: library_item_for_id(&c, b_c_ids[0]).await?.file_uuid,
    });
    let second = PlayingItem::InLibrary(LibraryItem {
        library_id: c_library_id,
        file_uuid: library_item_for_id(&c, b_c_ids[1]).await?.file_uuid,
    });
    let playing_result =
        peer_step_with_timeout("B starts remote C track", Duration::from_secs(35), async {
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
    assert!(
        playing
            .path
            .as_deref()
            .is_some_and(|path| path.as_os_str().is_empty())
    );
    let first_status_item = first.to_string();
    peer_step("revoke B read grant on C library", async {
        c.global_params
            .node
            .access_policy
            .set(&b_info.fingerprint, grant)?;
        Ok(())
    })
    .await?;
    peer_step_with_timeout(
        "B stops after C stream revocation",
        Duration::from_secs(20),
        async {
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
        },
    )
    .await?;
    peer_step_with_timeout(
        "A receives B playing status",
        Duration::from_secs(5),
        async {
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
        },
    )
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
                        if update
                            .items
                            .iter()
                            .filter(|item| {
                                item.unavailable == Some(PlaylistUnavailableReason::AccessRevoked)
                            })
                            .count()
                            == 2
                        {
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
    peer_step_with_timeout(
        "B player continues with local audio",
        Duration::from_secs(15),
        async {
            loop {
                let status = b.global_params.player.lock().await.get_status();
                if status.item == Some(local.clone()) && status.state == PlaybackState::Playing {
                    return Ok(());
                }
                sleep(Duration::from_millis(20)).await;
            }
        },
    )
    .await?;
    peer_step_with_timeout(
        "A receives post-revocation playback update",
        Duration::from_secs(5),
        async {
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
        },
    )
    .await?;
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    c.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cast_preflight_and_load_check_renderer_library_grants_over_websocket() -> Result<()> {
    use crate::backends::remote::{connect_remote, decode_message};
    use crate::messages::{
        CastLibraryState, CastLoadRequest, CastLoadResponse, CastPreflightResponse,
        LibraryItemRequest, PlaylistOperateMode, RemoteRequestFailed,
    };
    use futures::StreamExt;

    let host_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let controller_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let host = fixture(host_dir.path()).await?;
    let renderer = fixture_with_broadcaster(
        renderer_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals,
    )
    .await?;
    let controller = fixture(controller_dir.path()).await?;

    let host_ids = [
        session_audio(&host, "cast-first.ogg").await?,
        session_audio(&host, "cast-second.ogg").await?,
    ];
    let host_items =
        futures::future::join_all(host_ids.iter().map(|id| library_item_for_id(&host, *id)))
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
    let (_, _renderer_replica) = replicate_host_library(&host, &renderer).await?;
    let renderer_info = info(&renderer).await?;
    let host_library_id = host.global_params.library_id.context("host library id")?;

    let controller_addr = paired_full(&renderer, &controller).await?;
    let mut socket = connect_remote(
        "127.0.0.1",
        controller_addr.port(),
        &controller.global_params.config_path,
        controller.global_params.cert_validator.read().await.clone(),
        &controller.global_params.node.sync_tokens,
    )
    .await?;

    let preflight = CastPreflightRequest {
        library_ids: vec![host_library_id.to_string()],
    };
    let (name, payload, _) = controller_request(
        &mut socket,
        "CastPreflightRequest",
        rinf::serialize(&preflight)?,
    )
    .await?;
    assert_eq!(name, "CastPreflightResponse");
    let preflight: CastPreflightResponse = rinf::deserialize(&payload)?;
    assert_eq!(preflight.libraries[0].state, CastLibraryState::Ready);

    let cast_items = host_items
        .iter()
        .map(|item| (*item).into())
        .collect::<Vec<_>>();
    let load = CastLoadRequest {
        items: cast_items.clone(),
        start_index: Some(0),
        mode: PlaylistOperateMode::Replace,
        expected_revision: None,
        playback_mode: None,
        position_seconds: None,
        play: None,
    };
    let (name, payload, updates) =
        controller_request(&mut socket, "CastLoadRequest", rinf::serialize(&load)?).await?;
    assert_eq!(name, "CastLoadResponse");
    let _: CastLoadResponse = rinf::deserialize(&payload)?;
    wait_player(&renderer, |status| {
        status.item == Some(PlayingItem::InLibrary(host_items[0]))
    })
    .await?;
    renderer.global_params.player.lock().await.pause();
    let mut updates = updates;
    tokio::time::timeout(Duration::from_secs(8), async {
        while !updates.iter().any(|update| update.items.len() == 2) {
            let frame = socket.next().await.context("controller socket closed")??;
            if let Some((name, payload, _)) = decode_message(&frame.into_data())
                && name == "PlaylistUpdate"
            {
                updates.push(rinf::deserialize(&payload)?);
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("controller did not receive the cast queue")??;
    let update = updates
        .into_iter()
        .find(|update| update.items.len() == 2)
        .context("controller did not receive the cast queue")?;
    assert!(
        update
            .items
            .iter()
            .all(|item| { item.item.in_library.is_some() || item.item.library_item.is_some() })
    );
    assert_eq!(update.revision, 1);
    renderer.stop_worker().await?;
    host.stop_worker().await?;
    let revision = renderer
        .global_params
        .node
        .session()?
        .queue_revision()
        .await;
    let local_cast_id = session_audio(&renderer, "cast-local.ogg").await?;
    let local_cast_item = library_item_for_id(&renderer, local_cast_id).await?;
    let stale_load = CastLoadRequest {
        items: vec![local_cast_item.into()],
        expected_revision: Some(revision.saturating_sub(1)),
        start_index: Some(0),
        mode: PlaylistOperateMode::Replace,
        playback_mode: None,
        position_seconds: None,
        play: None,
    };
    let queue_before_stale = renderer.global_params.player.lock().await.get_playlist();
    let (name, payload, _) = controller_request(
        &mut socket,
        "CastLoadRequest",
        rinf::serialize(&stale_load)?,
    )
    .await?;
    assert_eq!(name, "RemoteRequestFailed");
    assert_eq!(
        rinf::deserialize::<RemoteRequestFailed>(&payload)?.kind,
        RequestErrorKind::Conflict
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        revision
    );
    assert_eq!(
        renderer.global_params.player.lock().await.get_playlist(),
        queue_before_stale
    );

    let not_held = LibraryItemRequest {
        library_id: controller
            .global_params
            .library_id
            .context("controller library id")?
            .to_string(),
        file_uuid: host_items[0].file_uuid.to_string(),
    };
    let (name, payload, _) = controller_request(
        &mut socket,
        "CastPreflightRequest",
        rinf::serialize(&CastPreflightRequest {
            library_ids: vec![not_held.library_id.clone()],
        })?,
    )
    .await?;
    let preflight: CastPreflightResponse = rinf::deserialize(&payload)?;
    assert_eq!(name, "CastPreflightResponse");
    assert_eq!(preflight.libraries[0].state, CastLibraryState::NotHeld);
    let queue_before_denial = renderer.global_params.player.lock().await.get_playlist();
    let (name, payload, _) = controller_request(
        &mut socket,
        "CastLoadRequest",
        rinf::serialize(&CastLoadRequest {
            items: vec![not_held.clone()],
            start_index: Some(0),
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(revision),
            playback_mode: None,
            position_seconds: None,
            play: None,
        })?,
    )
    .await?;
    assert_eq!(name, "RemoteRequestFailed");
    let failure = rinf::deserialize::<RemoteRequestFailed>(&payload)?;
    assert_eq!(failure.kind, RequestErrorKind::PermissionDenied);
    assert_eq!(
        failure.cause,
        Some(RequestFailureCause::CastNotPossible {
            library_id: not_held.library_id.clone(),
            state: CastLibraryState::NotHeld,
        })
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        revision
    );
    assert_eq!(
        renderer.global_params.player.lock().await.get_playlist(),
        queue_before_denial
    );

    set_library_grant(
        &host,
        &renderer_info.fingerprint,
        host_library_id,
        SyncLevel::None,
    )?;
    let (name, payload, _) = controller_request(
        &mut socket,
        "CastPreflightRequest",
        rinf::serialize(&preflight_request(host_library_id))?,
    )
    .await?;
    assert_eq!(name, "CastPreflightResponse");
    assert_eq!(
        rinf::deserialize::<CastPreflightResponse>(&payload)?.libraries[0].state,
        CastLibraryState::AccessRevoked
    );
    let queue_before_revoked_load = renderer.global_params.player.lock().await.get_playlist();
    let (name, payload, _) = controller_request(
        &mut socket,
        "CastLoadRequest",
        rinf::serialize(&CastLoadRequest {
            items: cast_items.clone(),
            start_index: None,
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(revision),
            playback_mode: None,
            position_seconds: None,
            play: None,
        })?,
    )
    .await?;
    assert_eq!(name, "RemoteRequestFailed");
    assert_eq!(
        rinf::deserialize::<RemoteRequestFailed>(&payload)?.kind,
        RequestErrorKind::PermissionDenied
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        revision
    );
    assert_eq!(
        renderer.global_params.player.lock().await.get_playlist(),
        queue_before_revoked_load
    );
    host.global_params.node.shutdown().await?;
    let (name, payload, _) = controller_request(
        &mut socket,
        "CastPreflightRequest",
        rinf::serialize(&preflight_request(host_library_id))?,
    )
    .await?;
    assert_eq!(name, "CastPreflightResponse");
    assert_eq!(
        rinf::deserialize::<CastPreflightResponse>(&payload)?.libraries[0].state,
        CastLibraryState::Unreachable
    );
    let (name, payload, _) = controller_request(
        &mut socket,
        "CastLoadRequest",
        rinf::serialize(&CastLoadRequest {
            items: cast_items,
            start_index: Some(0),
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(revision),
            playback_mode: None,
            position_seconds: None,
            play: None,
        })?,
    )
    .await?;
    assert_eq!(name, "RemoteRequestFailed");
    assert_eq!(
        rinf::deserialize::<RemoteRequestFailed>(&payload)?.kind,
        RequestErrorKind::Unavailable
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        revision
    );
    assert_eq!(
        renderer.global_params.player.lock().await.get_playlist(),
        queue_before_denial
    );

    let (name, payload, _) = controller_request(
        &mut socket,
        "CastPreflightRequest",
        rinf::serialize(&CastPreflightRequest {
            library_ids: vec!["malformed".into()],
        })?,
    )
    .await?;
    assert_eq!(name, "RemoteRequestFailed");
    assert_eq!(
        rinf::deserialize::<RemoteRequestFailed>(&payload)?.kind,
        RequestErrorKind::InvalidInput
    );

    let (name, payload, _) = controller_request(
        &mut socket,
        "CastLoadRequest",
        rinf::serialize(&CastLoadRequest {
            items: vec![LibraryItemRequest {
                library_id: host_library_id.to_string(),
                file_uuid: "malformed".into(),
            }],
            start_index: Some(0),
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(revision),
            playback_mode: None,
            position_seconds: None,
            play: None,
        })?,
    )
    .await?;
    assert_eq!(name, "RemoteRequestFailed");
    assert_eq!(
        rinf::deserialize::<RemoteRequestFailed>(&payload)?.kind,
        RequestErrorKind::InvalidInput
    );

    renderer.global_params.node.shutdown().await?;
    controller.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_renderers_reports_access_availability_and_attachment() -> Result<()> {
    let host_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let no_control_dir = tempfile::tempdir()?;
    let offline_dir = tempfile::tempdir()?;
    let controller_dir = tempfile::tempdir()?;
    let host = fixture(host_dir.path()).await?;
    let renderer = fixture(renderer_dir.path()).await?;
    let no_control = fixture(no_control_dir.path()).await?;
    let offline = fixture(offline_dir.path()).await?;
    let controller = fixture(controller_dir.path()).await?;

    replicate_host_library(&host, &renderer).await?;
    let renderer_address = paired_full(&renderer, &controller).await?;
    let no_control_address = paired_full(&no_control, &controller).await?;
    let offline_address = paired_full(&offline, &controller).await?;

    let controller_info = info(&controller).await?;
    let renderer_info = info(&renderer).await?;
    let no_control_info = info(&no_control).await?;
    let offline_info = info(&offline).await?;
    for (fingerprint, address) in [
        (&renderer_info.fingerprint, renderer_address),
        (&no_control_info.fingerprint, no_control_address),
        (&offline_info.fingerprint, offline_address),
    ] {
        controller
            .global_params
            .cert_validator
            .read()
            .await
            .replace_hosts_for_fingerprint(
                fingerprint,
                vec![format!("127.0.0.1:{}", address.port())],
            )
            .await?;
    }
    let renderer_validator = controller.global_params.cert_validator.read().await.clone();
    timeout(Duration::from_secs(3), async {
        loop {
            if fetch_device_info_at_port(
                "127.0.0.1",
                renderer_address.port(),
                Arc::new(Arc::new(renderer_validator.clone()).into_client_config()),
            )
            .await
            .is_ok()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .context("renderer server did not become ready")?;
    let host_library_id = host.global_params.library_id.context("host library id")?;
    let controller_library_id = controller
        .global_params
        .library_id
        .context("controller library id")?;
    let mut no_control_grants = no_control
        .global_params
        .node
        .access_policy
        .grants(&controller_info.fingerprint)?;
    no_control_grants.control = false;
    no_control
        .global_params
        .node
        .access_policy
        .set(&controller_info.fingerprint, no_control_grants)?;
    offline.global_params.node.shutdown().await?;

    async fn list(
        controller: &Fixture,
        library_ids: Vec<String>,
    ) -> Result<Vec<RendererCandidate>> {
        let request = ListRenderersRequest { library_ids };
        Ok(request
            .handle(
                request.extract_params(&controller.global_params),
                None,
                &request,
            )
            .await?
            .context("renderer list response")?
            .renderers)
    }

    let empty = list(&controller, Vec::new()).await?;
    fn candidate<'a>(
        renderers: &'a [RendererCandidate],
        fingerprint: &str,
    ) -> Result<&'a RendererCandidate> {
        renderers
            .iter()
            .find(|renderer| renderer.fingerprint == fingerprint)
            .with_context(|| format!("missing renderer candidate {fingerprint}"))
    }
    assert_eq!(
        candidate(&empty, &renderer_info.fingerprint)?.availability,
        RendererAvailability::Available
    );
    assert_eq!(
        candidate(&empty, &no_control_info.fingerprint)?.availability,
        RendererAvailability::NoControl
    );
    assert_eq!(
        candidate(&empty, &offline_info.fingerprint)?.availability,
        RendererAvailability::Offline
    );

    let host_library = list(&controller, vec![host_library_id.to_string()]).await?;
    assert_eq!(
        candidate(&host_library, &renderer_info.fingerprint)?.availability,
        RendererAvailability::Available
    );
    let own_library = list(&controller, vec![controller_library_id.to_string()]).await?;
    let blocked = candidate(&own_library, &renderer_info.fingerprint)?;
    assert_eq!(blocked.availability, RendererAvailability::CantPlay);
    let controller_library_id_text = controller_library_id.to_string();
    assert_eq!(
        blocked.blocked_library_id.as_deref(),
        Some(controller_library_id_text.as_str())
    );
    assert_eq!(blocked.library_state, Some(CastLibraryState::NotHeld));

    controller
        .global_params
        .node
        .attach_renderer(&renderer_info.fingerprint)
        .await?;
    let attached = list(&controller, Vec::new()).await?;
    assert!(candidate(&attached, &renderer_info.fingerprint)?.attached);

    controller.global_params.node.detach_renderer().await?;
    host.global_params.node.shutdown().await?;
    renderer.global_params.node.shutdown().await?;
    no_control.global_params.node.shutdown().await?;
    controller.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn renderer_link_request_round_trip_errors_and_cast_start_modes() -> Result<()> {
    use crate::backends::remote::RendererLink;
    use crate::messages::{
        CastLibraryState, CastLoadRequest, LibraryItemRequest, PlaylistOperateMode,
        RequestErrorKind,
    };
    use crate::request_error::{ExpectedRequestError, classify};

    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let controller_signals = Arc::new(RecordedSignals::default());
    let controller = fixture_with_broadcaster(
        controller_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        controller_signals.clone(),
    )
    .await?;
    let renderer = fixture(renderer_dir.path()).await?;
    attach_pair_with_signals(&controller, &renderer, &controller_signals).await?;
    let link: Arc<RendererLink> = controller
        .global_params
        .node
        .renderer_link()?
        .context("attached renderer link")?;
    let library_id = renderer
        .global_params
        .library_id
        .context("renderer library id")?;

    let preflight = link.call(&preflight_request(library_id)).await?;
    assert_eq!(preflight.libraries[0].state, CastLibraryState::Ready);

    let ids = [
        session_audio(&renderer, "request-anchor.ogg").await?,
        session_audio(&renderer, "request-no-switch.ogg").await?,
        session_audio(&renderer, "request-append-play.ogg").await?,
        session_audio(&renderer, "request-play-next.ogg").await?,
    ];
    let items = futures::future::join_all(ids.iter().map(|id| library_item_for_id(&renderer, *id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let anchor = items[0];
    renderer
        .global_params
        .node
        .session()?
        .edit_queue(None, move |player| {
            player.add_to_playlist_with_sources(
                vec![(
                    PlayingItem::InLibrary(anchor),
                    PlaybackSource::LibraryStream,
                )],
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    wait_player(&renderer, |status| status.playlist.len() == 1).await?;
    renderer.global_params.player.lock().await.switch(0);
    wait_player(&renderer, |status| {
        status.item == Some(PlayingItem::InLibrary(items[0]))
    })
    .await?;
    renderer.global_params.player.lock().await.pause();
    wait_player(&renderer, |status| {
        status.item == Some(PlayingItem::InLibrary(items[0]))
            && status.state == PlaybackState::Paused
    })
    .await?;

    let initial_revision = renderer
        .global_params
        .node
        .session()?
        .queue_revision()
        .await;
    let append_without_switch = CastLoadRequest {
        items: vec![items[1].into()],
        start_index: None,
        mode: PlaylistOperateMode::AppendToEnd,
        expected_revision: Some(initial_revision),
        playback_mode: None,
        position_seconds: None,
        play: None,
    };
    link.call(&append_without_switch).await?;
    wait_player(&renderer, |status| status.playlist.len() == 2).await?;
    assert_eq!(
        renderer.global_params.player.lock().await.get_status().item,
        Some(PlayingItem::InLibrary(items[0]))
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        initial_revision + 1
    );

    for (mode, item, expected_state) in [
        (
            PlaylistOperateMode::AppendToEnd,
            items[2],
            PlaybackState::Playing,
        ),
        (
            PlaylistOperateMode::PlayNext,
            items[3],
            PlaybackState::Playing,
        ),
    ] {
        let request = CastLoadRequest {
            items: vec![item.into()],
            start_index: Some(0),
            mode,
            expected_revision: Some(
                renderer
                    .global_params
                    .node
                    .session()?
                    .queue_revision()
                    .await,
            ),
            playback_mode: None,
            position_seconds: None,
            play: None,
        };
        link.call(&request).await?;
        wait_player(&renderer, |status| {
            status.item == Some(PlayingItem::InLibrary(item)) && status.state == expected_state
        })
        .await?;
        // Keep this test from decoding audio for the rest of its run; other tests in the
        // process share the machine and stream audio under 10 second deadlines.
        renderer.global_params.player.lock().await.pause();
    }

    // The player publishes the queue and the current item separately; wait for all four
    // items before taking the snapshot the rejected load must leave unchanged.
    let queue_before_invalid = wait_player(&renderer, |status| status.playlist.len() == 4)
        .await?
        .playlist;
    let revision_before_invalid = renderer
        .global_params
        .node
        .session()?
        .queue_revision()
        .await;
    let invalid = CastLoadRequest {
        items: vec![LibraryItemRequest {
            library_id: library_id.to_string(),
            file_uuid: "malformed".into(),
        }],
        start_index: None,
        mode: PlaylistOperateMode::AppendToEnd,
        expected_revision: None,
        playback_mode: None,
        position_seconds: None,
        play: None,
    };
    let error = match link.call(&invalid).await {
        Ok(_) => anyhow::bail!("malformed cast load unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(
        matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Remote {
                kind: RequestErrorKind::InvalidInput,
                cause: None,
                ..
            })
        ),
        "unexpected typed call error: {error:#}"
    );
    assert_eq!(classify(&error), RequestErrorKind::InvalidInput);

    let not_held = CastLoadRequest {
        items: vec![LibraryItemRequest {
            library_id: Uuid::new_v4().to_string(),
            file_uuid: Uuid::new_v4().to_string(),
        }],
        start_index: None,
        mode: PlaylistOperateMode::AppendToEnd,
        expected_revision: None,
        playback_mode: None,
        position_seconds: None,
        play: None,
    };
    let error = match link.call(&not_held).await {
        Ok(_) => anyhow::bail!("cast to a library the renderer does not hold succeeded"),
        Err(error) => error,
    };
    assert!(matches!(
        error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Remote {
            kind: RequestErrorKind::PermissionDenied,
            cause: Some(RequestFailureCause::CastNotPossible {
                state: CastLibraryState::NotHeld,
                ..
            }),
            ..
        })
    ));
    assert_eq!(
        renderer.global_params.player.lock().await.get_playlist(),
        queue_before_invalid
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        revision_before_invalid
    );
    assert!(
        !controller_signals
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|(name, _)| name == "RemoteRequestFailed")
    );

    renderer.global_params.node.shutdown().await?;
    timeout(Duration::from_secs(5), async {
        while controller.global_params.node.renderer_link()?.is_some() {
            sleep(Duration::from_millis(20)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("renderer link did not clear after renderer shutdown")??;
    let error = match link.call(&preflight_request(library_id)).await {
        Ok(_) => anyhow::bail!("request over a closed renderer link succeeded"),
        Err(error) => error,
    };
    assert_eq!(classify(&error), RequestErrorKind::Unavailable);
    assert!(controller.global_params.node.renderer_link()?.is_none());

    controller.global_params.node.shutdown().await?;
    Ok(())
}

fn preflight_request(library_id: Uuid) -> CastPreflightRequest {
    CastPreflightRequest {
        library_ids: vec![library_id.to_string()],
    }
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
    a.global_params
        .player
        .lock()
        .await
        .add_to_playlist_with_sources(
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
    assert_eq!(
        fixture.global_params.player.lock().await.get_playlist(),
        [item]
    );

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
    legacy
        .as_object_mut()
        .context("session JSON object")?
        .remove("session_id");
    legacy
        .as_object_mut()
        .context("session JSON object")?
        .remove("queue_revision");
    let v2_path = dir.path().join("sessions/v2.json");
    fs::write(&v2_path, serde_json::to_vec(&legacy)?)?;
    let migrated = SessionState::read(&v2_path, &reopened.library_pool).await?;
    assert_ne!(migrated.session_id, Uuid::nil());
    assert_eq!(migrated.queue_revision, 0);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(v2_path)?)?["version"],
        3
    );
    reopened.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn switch_request_rejects_stale_queue_revision_and_accepts_current_revision() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let first = session_audio(&fixture, "switch-revision-first.ogg").await?;
    let second = session_audio(&fixture, "switch-revision-second.ogg").await?;
    let items = [
        library_item_for_id(&fixture, first).await?,
        library_item_for_id(&fixture, second).await?,
    ];
    let session = fixture.global_params.node.session()?;
    session
        .edit_queue(None, |player| {
            player.add_to_playlist_with_sources(
                items
                    .iter()
                    .copied()
                    .map(|item| (PlayingItem::InLibrary(item), PlaybackSource::LibraryStream))
                    .collect(),
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    fixture.global_params.player.lock().await.switch(0);
    wait_player(&fixture, |status| status.index == Some(0)).await?;
    let before = fixture.global_params.player.lock().await.get_status().item;

    let stale = SwitchRequest {
        index: 1,
        expected_revision: Some(0),
    };
    let params = stale.extract_params(&fixture.global_params);
    let error = stale.handle(params, None, &stale).await.unwrap_err();
    assert_eq!(classify(&error), RequestErrorKind::Conflict);
    assert_eq!(
        fixture.global_params.player.lock().await.get_status().item,
        before
    );

    let current = SwitchRequest {
        index: 1,
        expected_revision: Some(1),
    };
    let params = current.extract_params(&fixture.global_params);
    current.handle(params, None, &current).await?;
    wait_player(&fixture, |status| status.index == Some(1)).await?;
    assert_eq!(
        fixture.global_params.player.lock().await.get_status().item,
        Some(PlayingItem::InLibrary(items[1])),
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_c2a_two_controllers_observe_conflict_and_same_queue() -> Result<()> {
    use crate::messages::{
        MovePlaylistItemRequest, PlaylistUpdate, RemoteRequestFailed, RemoveRequest,
    };
    use futures::StreamExt;

    async fn wait_revision(
        socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        mut updates: Vec<PlaylistUpdate>,
        revision: u64,
    ) -> Result<PlaylistUpdate> {
        timeout(Duration::from_secs(5), async {
            loop {
                if let Some(index) = updates
                    .iter()
                    .position(|update| update.revision == revision)
                {
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

    let (first_name, _, first_updates) = controller_request(
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
    let (second_name, failure_payload, second_updates) = controller_request(
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
        first_final
            .items
            .iter()
            .map(|item| &item.title)
            .collect::<Vec<_>>(),
        second_final
            .items
            .iter()
            .map(|item| &item.title)
            .collect::<Vec<_>>(),
    );
    assert_eq!(
        renderer
            .global_params
            .node
            .session()?
            .queue_revision()
            .await,
        1
    );
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
                &controller.global_params.node.sync_tokens,
            )
            .await?,
        );
    }
    let second_socket = sockets.pop().context("second controller socket")?;
    let first_socket = sockets.pop().context("first controller socket")?;
    Ok((first_socket, second_socket))
}

type ControllerSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Send one addressed request over a controller socket and collect queue updates until its reply.
async fn controller_request(
    socket: &mut ControllerSocket,
    name: &str,
    payload: Vec<u8>,
) -> Result<(String, Vec<u8>, Vec<PlaylistUpdate>)> {
    use crate::backends::remote::encode_message;
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let request_id = Uuid::new_v4();
    socket
        .send(Message::Binary(
            encode_message(name, &payload, Some(request_id)).into(),
        ))
        .await?;
    let mut updates = Vec::new();
    timeout(Duration::from_secs(8), async {
        loop {
            let frame = socket.next().await.context("controller socket closed")??;
            let Some((response_name, payload, response_id)) = decode_message(&frame.into_data())
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

/// Set one library's level in the grant `host` gives to `peer_fingerprint`.
fn set_library_grant(
    host: &Fixture,
    peer_fingerprint: &str,
    library_id: Uuid,
    level: SyncLevel,
) -> Result<()> {
    let policy = &host.global_params.node.access_policy;
    let mut grant = policy.grants(peer_fingerprint)?;
    grant.connection.libraries.insert(
        library_id,
        match level {
            SyncLevel::None => ConnectionLevel::None,
            SyncLevel::Pull => ConnectionLevel::View,
            SyncLevel::PullPush => ConnectionLevel::ViewEdit,
        },
    );
    grant.sync.libraries.insert(library_id, level);
    policy.set(peer_fingerprint, grant)?;
    Ok(())
}

/// Pair `peer` with `host`, grant it the host library, add the replica and sync its metadata.
async fn replicate_host_library(host: &Fixture, peer: &Fixture) -> Result<(SocketAddr, Fixture)> {
    let host_addr = paired_full(host, peer).await?;
    replicate_host_library_at(host, peer, host_addr).await
}

pub(super) async fn replicate_host_library_at(
    host: &Fixture,
    peer: &Fixture,
    host_addr: SocketAddr,
) -> Result<(SocketAddr, Fixture)> {
    use crate::server::sync_worker::{PeerEndpoint, TestPeerSession};

    let host_info = info(host).await?;
    let peer_info = info(peer).await?;
    let trusted_host = format!("127.0.0.1:{}", host_addr.port());
    peer.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(&host_info.fingerprint, vec![trusted_host.clone()])
        .await?;
    let host_library_id = host.global_params.library_id.context("host library id")?;
    set_library_grant(
        host,
        &peer_info.fingerprint,
        host_library_id,
        SyncLevel::PullPush,
    )?;
    let listed = request_peer_libraries(peer, &host_info.fingerprint).await?;
    let hosted = listed
        .libraries
        .iter()
        .find(|library| library.library_id == host_library_id.to_string())
        .with_context(|| {
            format!(
                "host library was not listed (error {:?}, libraries {:?})",
                listed.error_kind, listed.libraries
            )
        })?;
    let replica_path = request_add_library(peer, &host_info.fingerprint, hosted).await?;
    let replica = fixture_open_replica(peer, Path::new(&replica_path)).await?;
    let endpoint = PeerEndpoint {
        fingerprint: host_info.fingerprint,
        trusted_host,
        host: "127.0.0.1".into(),
        port: host_addr.port(),
    };
    // One session for the whole replication: an incomplete pass resumes its work on the
    // next pass instead of abandoning it, as the background worker does.
    let session = TestPeerSession::default();
    timeout(Duration::from_secs(30), async {
        loop {
            match session
                .pass(
                    replica.global_params.clone(),
                    &replica.private_key,
                    endpoint.clone(),
                    true,
                )
                .await?
            {
                PeerPass::Complete => return Ok::<_, anyhow::Error>(()),
                PeerPass::Incomplete => tokio::task::yield_now().await,
                other => anyhow::bail!("unexpected replica metadata pass: {other:?}"),
            }
        }
    })
    .await
    .context("replica metadata sync timed out")??;
    Ok((host_addr, replica))
}

/// Give a replica a local copy of one of its files, so it plays without streaming.
async fn materialize_replica_file(
    replica: &Fixture,
    file_id: i32,
    source: &Path,
    file_name: &str,
) -> Result<()> {
    let library = replica.global_params.test_library();
    fs::copy(source, Path::new(&library.media_root).join(file_name))?;
    media_files::Entity::update(media_files::ActiveModel {
        id: Set(file_id),
        directory: Set(String::new()),
        file_name: Set(file_name.into()),
        local_available: Set(true),
        ..Default::default()
    })
    .exec(&*library.main_db)
    .await?;
    Ok(())
}

async fn attached_handoff_source(
    hold_source_library: bool,
) -> Result<(Vec<TempDir>, Fixture, Fixture, String)> {
    let controller_dir = tempfile::tempdir()?;
    let source_dir = tempfile::tempdir()?;
    let controller = fixture(controller_dir.path()).await?;
    let source = fixture(source_dir.path()).await?;
    let source_id = session_audio(&source, "handoff-retry-source.ogg").await?;
    queue_session(&source, &[source_id]).await?;
    // The fixture track lasts a few seconds; repeat it so a slow run never sees it end.
    for node in [&controller, &source] {
        node.global_params
            .player
            .lock()
            .await
            .set_playback_mode(PlaybackMode::RepeatAll);
    }
    source.global_params.player.lock().await.load(0);
    source.global_params.player.lock().await.play();
    wait_player(&source, |status| status.state == PlaybackState::Playing).await?;
    let source_address = if hold_source_library {
        replicate_host_library(&source, &controller).await?.0
    } else {
        super::paired_full(&source, &controller).await?
    };
    let source_info = super::info(&source).await?;
    controller
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &source_info.fingerprint,
            vec![format!("127.0.0.1:{}", source_address.port())],
        )
        .await?;
    controller
        .global_params
        .node
        .attach_renderer(&source_info.fingerprint)
        .await?;
    Ok((
        vec![controller_dir, source_dir],
        controller,
        source,
        source_info.fingerprint,
    ))
}

async fn hand_off(controller: &Fixture, target: Option<String>) -> Result<()> {
    use crate::messages::HandoffRequest;

    let handoff = HandoffRequest { target };
    handoff
        .handle(
            handoff.extract_params(&controller.global_params),
            None,
            &handoff,
        )
        .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn handoff_pause_retries_after_destination_started() -> Result<()> {
    let (_dirs, controller, source, _) = attached_handoff_source(true).await?;
    controller.global_params.node.fail_next_handoff_pauses(1);
    hand_off(&controller, None).await?;
    wait_player(&controller, |status| status.state == PlaybackState::Playing).await?;
    let source_paused =
        wait_player(&source, |status| status.state == PlaybackState::Paused).await?;
    assert!(!source_paused.playlist.is_empty());
    controller.global_params.node.shutdown().await?;
    source.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn new_handoff_cancels_old_source_pause_retry() -> Result<()> {
    let (_dirs, controller, source, source_fingerprint) = attached_handoff_source(true).await?;
    controller.global_params.node.fail_next_handoff_pauses(1);
    hand_off(&controller, None).await?;
    wait_player(&controller, |status| status.state == PlaybackState::Playing).await?;

    hand_off(&controller, Some(source_fingerprint)).await?;
    wait_player(&source, |status| status.state == PlaybackState::Playing).await?;
    sleep(Duration::from_millis(1300)).await;
    assert_eq!(
        source.global_params.player.lock().await.get_status().state,
        PlaybackState::Playing
    );
    controller.global_params.node.shutdown().await?;
    source.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn failed_handoff_load_does_not_pause_source() -> Result<()> {
    let (_dirs, controller, source, _) = attached_handoff_source(false).await?;
    assert!(hand_off(&controller, None).await.is_err());
    assert_eq!(
        source.global_params.player.lock().await.get_status().state,
        PlaybackState::Playing
    );
    controller.global_params.node.shutdown().await?;
    source.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn handoff_moves_queue_and_position_and_preserves_source_sessions() -> Result<()> {
    use crate::Signal;
    use crate::messages::HandoffRequest;
    use crate::request_error::request_failure_cause;
    use crate::utils::ParamsExtractor;

    let h_dir = tempfile::tempdir()?;
    let g_dir = tempfile::tempdir()?;
    let a_dir = tempfile::tempdir()?;
    let b_dir = tempfile::tempdir()?;
    let c_dir = tempfile::tempdir()?;
    let h = fixture(h_dir.path()).await?;
    let g = fixture(g_dir.path()).await?;
    let a = fixture(a_dir.path()).await?;
    let b = fixture(b_dir.path()).await?;
    let c = fixture(c_dir.path()).await?;

    let h_ids = [
        session_audio(&h, "handoff-h1.ogg").await?,
        session_audio(&h, "handoff-h2.ogg").await?,
    ];
    let g_id = session_audio(&g, "handoff-g.ogg").await?;
    let (h_address, _a_h_replica) = replicate_host_library(&h, &a).await?;
    let (g_address, a_g_replica) = replicate_host_library(&g, &a).await?;
    pair(&h, &b).await?;
    pair(&b, &h).await?;
    pair(&g, &b).await?;
    pair(&b, &g).await?;
    let (_, _b_h) = replicate_host_library_at(&h, &b, h_address).await?;
    let (_, _b_g) = replicate_host_library_at(&g, &b, g_address).await?;
    pair(&h, &c).await?;
    pair(&c, &h).await?;
    let (_, _c_h) = replicate_host_library_at(&h, &c, h_address).await?;
    let b_fingerprint = info(&b).await?.fingerprint;
    let b_address = paired_full(&b, &a).await?;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &b_fingerprint,
            vec![format!("127.0.0.1:{}", b_address.port())],
        )
        .await?;
    let c_fingerprint = info(&c).await?.fingerprint;
    let c_address = paired_full(&c, &a).await?;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &c_fingerprint,
            vec![format!("127.0.0.1:{}", c_address.port())],
        )
        .await?;

    let h_items = futures::future::join_all(h_ids.iter().map(|id| library_item_for_id(&h, *id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let g_item = library_item_for_id(&g, g_id).await?;
    let queue = vec![h_items[0], g_item, h_items[1]];
    let g_source = g.global_params.test_library();
    let a_g_library = a_g_replica.global_params.test_library();
    let g_local_id = file_ids_by_uuid(&a_g_library.main_db, &[g_item.file_uuid])
        .await?
        .get(&g_item.file_uuid)
        .copied()
        .context("replica G file id")?;
    let controller = &a_g_replica;
    let g_row = media_files::Entity::find_by_id(g_id)
        .one(&*g_source.main_db)
        .await?
        .context("source G row")?;
    materialize_replica_file(
        controller,
        g_local_id,
        &Path::new(&g_source.media_root)
            .join(&g_row.directory)
            .join(&g_row.file_name),
        "handoff-materialized.ogg",
    )
    .await?;
    controller
        .global_params
        .node
        .session()?
        .edit_queue(None, move |player| {
            player.add_to_playlist_with_sources(
                queue
                    .into_iter()
                    .map(|item| (PlayingItem::InLibrary(item), PlaybackSource::LibraryStream))
                    .collect(),
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    wait_player(controller, |status| status.playlist.len() == 3).await?;
    controller.global_params.player.lock().await.switch(1);
    wait_player(controller, |status| status.index == Some(1)).await?;
    controller.global_params.player.lock().await.play();
    wait_player(controller, |status| status.state == PlaybackState::Playing).await?;
    let source_before = wait_player(controller, |status| {
        status.position >= Duration::from_millis(1900)
    })
    .await?;
    let source_queue = source_before.playlist.clone();
    let source_position = source_before.position;

    let handoff = HandoffRequest {
        target: Some(b_fingerprint.clone()),
    };
    handoff
        .handle(
            handoff.extract_params(&controller.global_params),
            None,
            &handoff,
        )
        .await?;
    let b_status = wait_player(&b, |status| {
        status.index == Some(1)
            && status.state == PlaybackState::Playing
            && status.playlist == source_queue
            && status.position.abs_diff(source_position) <= Duration::from_millis(1500)
    })
    .await?;
    assert_eq!(b_status.playlist, source_queue);
    assert!(b_status.position.abs_diff(source_position) <= Duration::from_millis(1500));
    let a_paused = wait_player(controller, |status| {
        status.state == PlaybackState::Paused && status.playlist == source_queue
    })
    .await?;
    assert_eq!(a_paused.playlist, source_queue);
    assert_eq!(a_paused.index, Some(1));
    assert_eq!(
        controller
            .global_params
            .node
            .renderer_link()?
            .as_ref()
            .map(|link| link.fingerprint()),
        Some(b_fingerprint.as_str())
    );

    b.global_params.player.lock().await.pause();
    wait_player(&b, |status| status.state == PlaybackState::Paused).await?;
    b.global_params.player.lock().await.play();
    wait_player(&b, |status| status.state == PlaybackState::Playing).await?;
    let b_before_return = b.global_params.player.lock().await.get_status();
    let b_queue = b_before_return.playlist.clone();
    let back = HandoffRequest { target: None };
    back.handle(back.extract_params(&controller.global_params), None, &back)
        .await?;
    let a_returned = wait_player(controller, |status| {
        status.index == Some(1)
            && status.state == PlaybackState::Playing
            && status.playlist == b_queue
            && status.position.abs_diff(b_before_return.position) <= Duration::from_millis(1500)
    })
    .await?;
    assert_eq!(a_returned.playlist, b_queue);
    assert!(a_returned.position.abs_diff(b_before_return.position) <= Duration::from_millis(1500));
    assert!(controller.global_params.node.renderer_link()?.is_none());
    let b_paused = wait_player(&b, |status| status.state == PlaybackState::Paused).await?;
    assert_eq!(b_paused.playlist, b_queue);
    controller.global_params.player.lock().await.pause();
    wait_player(controller, |status| status.state == PlaybackState::Paused).await?;

    controller.global_params.player.lock().await.play();
    let a_before_refused =
        wait_player(controller, |status| status.state == PlaybackState::Playing).await?;
    let c_before = c.global_params.player.lock().await.get_playlist();
    let refused = HandoffRequest {
        target: Some(c_fingerprint),
    };
    let error = refused
        .handle(
            refused.extract_params(&controller.global_params),
            None,
            &refused,
        )
        .await
        .unwrap_err();
    let g_library_id = g
        .global_params
        .library_id
        .context("G library id")?
        .to_string();
    assert_eq!(
        request_failure_cause(&error),
        Some(RequestFailureCause::CastNotPossible {
            library_id: g_library_id,
            state: CastLibraryState::NotHeld,
        })
    );
    assert!(controller.global_params.node.renderer_link()?.is_none());
    assert_eq!(
        controller
            .global_params
            .player
            .lock()
            .await
            .get_status()
            .state,
        PlaybackState::Playing
    );
    assert_eq!(
        controller
            .global_params
            .player
            .lock()
            .await
            .get_status()
            .playlist,
        a_before_refused.playlist
    );
    assert_eq!(c.global_params.player.lock().await.get_playlist(), c_before);
    controller.global_params.player.lock().await.pause();
    wait_player(controller, |status| status.state == PlaybackState::Paused).await?;

    controller
        .global_params
        .node
        .session()?
        .edit_queue(None, |player| {
            player.add_to_playlist_with_sources(
                vec![(
                    PlayingItem::IndependentFile("outside-library.ogg".into()),
                    PlaybackSource::Local(PathBuf::from("unused-handoff-test-path")),
                )],
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    let independent_before = wait_player(controller, |status| {
        status.state == PlaybackState::Paused && status.playlist.len() == 4
    })
    .await?;
    let b_before_independent = b.global_params.player.lock().await.get_status();
    let refused_independent = HandoffRequest {
        target: Some(b_fingerprint),
    };
    let error = refused_independent
        .handle(
            refused_independent.extract_params(&controller.global_params),
            None,
            &refused_independent,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unavailable {
            what: "handing off files outside a library"
        })
    ));
    let independent_after = controller.global_params.player.lock().await.get_status();
    assert_eq!(independent_after.state, PlaybackState::Paused);
    assert_eq!(independent_after.playlist, independent_before.playlist);
    assert_eq!(independent_after.index, independent_before.index);
    assert!(controller.global_params.node.renderer_link()?.is_none());
    let b_after_independent = b.global_params.player.lock().await.get_status();
    assert_eq!(b_after_independent.playlist, b_before_independent.playlist);
    assert_eq!(b_after_independent.index, b_before_independent.index);
    assert_eq!(b_after_independent.position, b_before_independent.position);

    controller.global_params.node.detach_renderer().await?;
    for fixture in [controller, &b, &c, &h, &g] {
        fixture.global_params.node.shutdown().await?;
    }
    Ok(())
}

#[test]
fn session_snapshot_rejects_independent_files() {
    let mut status = MockPlayer.get_status();
    status
        .playlist
        .push(PlayingItem::IndependentFile("outside-library.ogg".into()));
    let error = match session_snapshot_from_status(status, 0) {
        Ok(_) => panic!("independent file unexpectedly appeared in a handoff snapshot"),
        Err(error) => error,
    };
    assert!(matches!(
        error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unavailable {
            what: "handing off files outside a library"
        })
    ));
}

#[tokio::test]
async fn playback_snapshot_waits_for_accepted_queue_changes() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let fixture = fixture(temp.path()).await?;
    let track = session_audio(&fixture, "snapshot-barrier.ogg").await?;
    let item = library_item_for_id(&fixture, track).await?;
    let session = fixture.global_params.node.session()?;

    session
        .edit_queue(None, move |player| {
            player.add_to_playlist_with_sources(
                vec![(PlayingItem::InLibrary(item), PlaybackSource::LibraryStream)],
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;

    let (status, revision) = timeout(Duration::from_secs(2), session.playback_snapshot())
        .await
        .context("snapshot barrier did not return")?;
    assert_eq!(status.playlist, [PlayingItem::InLibrary(item)]);
    assert_eq!(revision, 1);
    fixture.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn playback_snapshot_on_idle_player_returns_promptly() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let fixture = fixture(temp.path()).await?;
    let session = fixture.global_params.node.session()?;

    let (status, revision) = timeout(Duration::from_secs(2), session.playback_snapshot())
        .await
        .context("idle snapshot barrier did not return")?;
    assert!(status.playlist.is_empty());
    assert_eq!(revision, 0);
    fixture.global_params.node.shutdown().await?;
    Ok(())
}

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
    assert!(
        controllers
            .iter()
            .any(|controller| controller.fingerprint == second_fingerprint)
    );

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
    detach.handle(renderer_node.clone(), None, &detach).await?;
    let frame = timeout(Duration::from_secs(3), first.next())
        .await?
        .context("detached socket closed without a frame")??;
    assert!(
        matches!(frame, Message::Close(Some(frame)) if u16::from(frame.code) == 4409 && frame.reason == "detached")
    );
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
            let frame = second
                .next()
                .await
                .context("second controller disconnected")??;
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
            if serde_json::from_slice::<serde_json::Value>(&contents).is_ok_and(|state| {
                state["queue"]
                    .as_array()
                    .is_some_and(|queue| queue.len() == len)
            }) {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_m3u8_and_independent_lyrics_cannot_read_outside_library() -> Result<()> {
    use crate::access_policy::{ConnectionLevel, DeviceGrants, LibraryGrants, SyncLevel};
    use crate::backends::remote::connect_remote;
    use crate::messages::{
        CreateM3u8PlaylistRequest, CreateM3u8PlaylistResponse, RemoteRequestFailed,
    };

    let host_dir = tempfile::tempdir()?;
    let controller_dir = tempfile::tempdir()?;
    let host = fixture(host_dir.path()).await?;
    let controller = fixture(controller_dir.path()).await?;
    let library_id = host.global_params.library_id.context("host library id")?;
    let track_id = session_audio(&host, "remote-import.ogg").await?;
    let media_root = Path::new(&host.global_params.test_library().media_root);
    let inside_playlist = media_root.join("inside.m3u8");
    fs::write(&inside_playlist, "remote-import.ogg\n")?;

    let addr = paired_full(&host, &controller).await?;
    let fingerprint = info(&controller).await?.fingerprint;
    host.global_params.node.access_policy.set(
        &fingerprint,
        DeviceGrants {
            control: true,
            connection: LibraryGrants {
                all_libraries: None,
                libraries: [(library_id, ConnectionLevel::ViewEdit)]
                    .into_iter()
                    .collect(),
            },
            sync: LibraryGrants {
                all_libraries: None,
                libraries: [(library_id, SyncLevel::PullPush)].into_iter().collect(),
            },
            administer: false,
        },
    )?;
    let mut socket = connect_remote(
        "127.0.0.1",
        addr.port(),
        &controller.global_params.config_path,
        controller.global_params.cert_validator.read().await.clone(),
        &controller.global_params.node.sync_tokens,
    )
    .await?;

    let outside_dir = tempfile::tempdir()?;
    let secret = "REMOTE_PATH_SECRET_MARKER";
    let outside_playlist = outside_dir.path().join("secret.m3u8");
    fs::write(&outside_playlist, format!("{secret}\n"))?;
    let outside_request = CreateM3u8PlaylistRequest {
        name: "outside remote".into(),
        group: "security".into(),
        path: outside_playlist.to_string_lossy().into_owned(),
    };
    let (response_name, response_payload) = socket_request_for_library(
        &mut socket,
        "CreateM3u8PlaylistRequest",
        &rinf::serialize(&outside_request)?,
        library_id,
    )
    .await?;
    assert_eq!(response_name, "RemoteRequestFailed");
    let failure: RemoteRequestFailed = rinf::deserialize(&response_payload)?;
    assert_eq!(failure.kind, RequestErrorKind::InvalidInput);
    assert!(!failure.detail.contains(secret));
    assert!(!String::from_utf8_lossy(&response_payload).contains(secret));

    let inside_request = CreateM3u8PlaylistRequest {
        name: "inside remote".into(),
        group: "security".into(),
        path: inside_playlist.to_string_lossy().into_owned(),
    };
    let (response_name, response_payload) = socket_request_for_library(
        &mut socket,
        "CreateM3u8PlaylistRequest",
        &rinf::serialize(&inside_request)?,
        library_id,
    )
    .await?;
    assert_eq!(response_name, "CreateM3u8PlaylistResponse");
    let imported: CreateM3u8PlaylistResponse = rinf::deserialize(&response_payload)?;
    assert!(imported.success);
    assert_eq!(imported.imported_count, Some(1));

    let local_request = CreateM3u8PlaylistRequest {
        name: "outside local".into(),
        group: "security".into(),
        path: outside_playlist.to_string_lossy().into_owned(),
    };
    let local_response = local_request
        .handle(
            (
                Some(host.global_params.test_library().clone()),
                Arc::new(Quiet),
            ),
            None,
            &local_request,
        )
        .await?
        .context("local playlist import returned no response")?;
    assert!(local_response.success);
    assert_eq!(local_response.imported_count, Some(0));

    assert!(track_id > 0);
    socket.close(None).await?;
    host.manager.stop().await?;
    host.global_params.node.shutdown().await?;
    controller.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_independent_file_lyrics_are_refused() -> Result<()> {
    use crate::access_policy::{ConnectionLevel, DeviceGrants, LibraryGrants, SyncLevel};
    use crate::backends::remote::connect_remote;
    use crate::messages::{
        GetLyricByTrackIdRequest, IndependentFilePlayingItem, RemoteRequestFailed,
    };

    let host_dir = tempfile::tempdir()?;
    let controller_dir = tempfile::tempdir()?;
    let host = fixture(host_dir.path()).await?;
    let controller = fixture(controller_dir.path()).await?;
    let library_id = host.global_params.library_id.context("host library id")?;
    let addr = paired_full(&host, &controller).await?;
    let fingerprint = info(&controller).await?.fingerprint;
    host.global_params.node.access_policy.set(
        &fingerprint,
        DeviceGrants {
            control: true,
            connection: LibraryGrants {
                all_libraries: None,
                libraries: [(library_id, ConnectionLevel::View)].into_iter().collect(),
            },
            sync: LibraryGrants {
                all_libraries: None,
                libraries: [(library_id, SyncLevel::Pull)].into_iter().collect(),
            },
            administer: false,
        },
    )?;
    let mut socket = connect_remote(
        "127.0.0.1",
        addr.port(),
        &controller.global_params.config_path,
        controller.global_params.cert_validator.read().await.clone(),
        &controller.global_params.node.sync_tokens,
    )
    .await?;

    let outside_dir = tempfile::tempdir()?;
    let secret = "REMOTE_LYRIC_SECRET_MARKER";
    let outside_audio = outside_dir.path().join("private-audio.ogg");
    fs::write(&outside_audio, secret)?;
    let request = GetLyricByTrackIdRequest {
        item: Some(PlayingItemRequest {
            in_library: None,
            independent_file: Some(IndependentFilePlayingItem {
                raw_path: outside_audio.to_string_lossy().into_owned(),
            }),
            library_item: None,
        }),
    };
    let (response_name, response_payload) = socket_request_for_library(
        &mut socket,
        "GetLyricByTrackIdRequest",
        &rinf::serialize(&request)?,
        library_id,
    )
    .await?;
    assert_eq!(response_name, "RemoteRequestFailed");
    let failure: RemoteRequestFailed = rinf::deserialize(&response_payload)?;
    assert_eq!(failure.kind, RequestErrorKind::InvalidInput);
    assert!(!failure.detail.contains(secret));
    assert!(!String::from_utf8_lossy(&response_payload).contains(secret));

    socket.close(None).await?;
    host.manager.stop().await?;
    host.global_params.node.shutdown().await?;
    controller.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn typed_renderer_call_rejects_wrong_reply_name() -> Result<()> {
    use crate::backends::remote::{
        RemoteStream, RendererLink, RendererLinkUse, decode_message, encode_message,
    };
    use crate::messages::{CastLoadRequest, PlaylistOperateMode};
    use futures::SinkExt;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, connect_async, tungstenite::Message};

    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let fake_renderer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        let mut socket = accept_async(stream).await?;
        let message = socket
            .next()
            .await
            .context("typed request was not received")??;
        let Message::Binary(payload) = message else {
            anyhow::bail!("expected a binary request frame")
        };
        let (_, _, request_id) = decode_message(&payload).context("invalid request frame")?;
        socket
            .send(Message::Binary(
                encode_message("SessionSnapshotResponse", &[], Some(request_id)).into(),
            ))
            .await?;
        Ok::<_, anyhow::Error>(())
    });
    let (stream, _) = connect_async(format!("ws://{address}")).await?;
    let stream: RemoteStream = stream;
    let link = RendererLink::new(
        stream,
        fixture.global_params.node.clone(),
        "fake-renderer".to_owned(),
        "Fake renderer".to_owned(),
        RendererLinkUse::Cast,
    )
    .await;
    let request = CastLoadRequest {
        items: Vec::new(),
        start_index: None,
        mode: PlaylistOperateMode::Replace,
        expected_revision: None,
        playback_mode: None,
        position_seconds: None,
        play: None,
    };
    let error = match timeout(Duration::from_secs(5), link.call(&request))
        .await
        .context("typed renderer call timed out")?
    {
        Ok(_) => anyhow::bail!("typed call accepted a response with the wrong name"),
        Err(error) => error,
    };
    assert!(
        matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::UnexpectedResponse { expected, actual })
                if expected == "CastLoadResponse" && actual == "SessionSnapshotResponse"
        ),
        "unexpected typed call error: {error:#}"
    );
    link.close();
    fake_renderer.await??;
    fixture.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn rpc_target_local_session_changes_local_state() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let local = fixture(dir.path()).await?;
    let request = VolumeRequest { volume: 0.37 };
    assert!(local.global_params.node.renderer_link()?.is_none());
    let response = dispatch_gui_request(&local.global_params, &request)
        .await?
        .context("native volume response")?;
    assert_eq!(response.volume, 0.37);
    wait_player(&local, |status| status.volume == 0.37).await?;
    local.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn rpc_target_session_and_library_catalogue_matches_registered_peer_handlers() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let peer = fixture(dir.path()).await?;
    peer.global_params
        .node
        .close_ui(&peer.global_params.main_token)
        .await;
    for (name, slot) in RpcTarget::rune_slots().scoped_slots() {
        // Session-only requests authorize without a UI; catalogue requests still need one.
        // An untrusted identity must never execute commands, including zero-argument ones.
        let session = Session {
            public_key: "untrusted key".into(),
            fingerprint: "untrusted peer".into(),
            host: "127.0.0.1".into(),
        };
        let dispatched =
            dispatch_remote(name, &[255], Some(session), None, &peer.global_params.node).await;
        match slot {
            Slot::Forward => {
                let (_, result) =
                    dispatched.with_context(|| format!("{name} has no peer handler"))?;
                let error = result.unwrap_err();
                if request_scope(name) == Some(RequestScope::Session)
                    && !request_uses_controller_library(name)
                {
                    assert_eq!(
                        classify(&error),
                        RequestErrorKind::PermissionDenied,
                        "{name}"
                    );
                } else {
                    assert_eq!(classify(&error), RequestErrorKind::Unavailable, "{name}");
                }
            }
            Slot::Unsupported { reason } => {
                assert_eq!(reason, UnsupportedReason::OwnerOnly);
                assert!(
                    dispatched.is_none(),
                    "{name} no longer needs its blacklist entry"
                );
            }
            _ => panic!("Rune scoped slot {name} is neither forwarded nor blacklisted"),
        }
    }
    peer.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn rpc_target_blacklisted_request_returns_typed_failure_without_sending() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let controller = fixture(dir.path()).await?;
    let listener = TokioTcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let accepting = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        Ok::<_, anyhow::Error>(accept_async(stream).await?)
    });
    let (stream, _) = connect_async(format!("ws://{address}")).await?;
    let mut peer = accepting.await??;
    let link = RendererLink::new(
        stream,
        controller.global_params.node.clone(),
        "blacklist-peer".to_owned(),
        "Blacklist peer".to_owned(),
        RendererLinkUse::Control,
    )
    .await;
    controller
        .global_params
        .node
        .install_renderer_link(link, "Blacklist peer".to_owned())?;
    // This node's own coordinators stay local while a renderer is attached.
    let controllers = dispatch_gui_request(&controller.global_params, &ListControllersRequest {})
        .await?
        .context("local controller list")?;
    assert!(controllers.controllers.is_empty());

    // A request the Rune target lists as unsupported fails before anything is sent.
    let link = controller
        .global_params
        .node
        .renderer_link()?
        .context("attached renderer")?;
    let target = RpcTarget::rune(link).with_slots(|slots| {
        slots.pause_request = Slot::Unsupported {
            reason: UnsupportedReason::OwnerOnly,
        }
    });
    let error = run_on_target(&target, &controller.global_params, &PauseRequest {})
        .await
        .err()
        .context("blacklisted request unexpectedly succeeded")?;
    assert!(matches!(
        error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unsupported {
            request: "PauseRequest",
            reason: UnsupportedReason::OwnerOnly,
        })
    ));
    assert_eq!(classify(&error), RequestErrorKind::Unavailable);
    assert!(
        timeout(Duration::from_millis(100), peer.next())
            .await
            .is_err(),
        "blacklisted request sent a frame"
    );
    controller.global_params.node.detach_renderer().await?;
    controller.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_library_routes_gui_edits_events_and_lifecycle_between_real_nodes() -> Result<()> {
    let first = tempfile::tempdir()?;
    let second = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let a = super::fixture_with_reconnect_config(
        first.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
        reconnect_config(Duration::from_millis(400)),
    )
    .await?;
    let b = fixture(second.path()).await?;
    let address = super::paired_full(&b, &a).await?;
    let fingerprint = super::info(&b).await?.fingerprint;
    a.global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(&fingerprint, vec![format!("127.0.0.1:{}", address.port())])
        .await?;
    async fn create(params: &GlobalParams, name: &str) -> Result<()> {
        dispatch_gui_request(
            params,
            &CreatePlaylistRequest {
                name: name.into(),
                group: "RR".into(),
            },
        )
        .await?;
        Ok(())
    }
    async fn names(params: &GlobalParams) -> Result<Vec<String>> {
        Ok(dispatch_gui_request(params, &FetchAllPlaylistsRequest {})
            .await?
            .context("playlists")?
            .playlists
            .into_iter()
            .map(|playlist| playlist.name)
            .collect())
    }
    create(&a.global_params, "only on A").await?;
    create(&b.global_params, "only on B").await?;
    let a_track = session_audio(&a, "rr-local.ogg").await?;
    queue_session(&a, &[a_track]).await?;
    let local_queue = a.global_params.player.lock().await.get_playlist();
    let b_track = session_audio(&b, "rr-remote.ogg").await?;
    let remote_session_id = b.global_params.node.session()?.session_id().await;
    let path = format!("ws://{address}");
    let remote = open_remote_library(a.global_params.node.clone(), &format!("@RR|{path}")).await?;
    assert!(Arc::ptr_eq(&remote.player, &a.global_params.player));
    assert!(Arc::ptr_eq(&remote.scrobbler, &a.global_params.scrobbler));
    assert!(remote.library.is_none());
    let capabilities = capability_snapshot(&a.global_params.node)?;
    assert_eq!(
        capabilities.fetch_all_playlists_request,
        RequestCapability::Available
    );
    assert_eq!(
        capabilities.close_library_request,
        RequestCapability::Available
    );
    assert_eq!(
        RpcTarget::select_type::<FetchAllPlaylistsRequest>(&a.global_params.node)?
            .slot_type::<FetchAllPlaylistsRequest>(),
        Slot::Forward
    );
    assert_eq!(
        RpcTarget::select_type::<CloseLibraryRequest>(&a.global_params.node)?
            .slot_type::<CloseLibraryRequest>(),
        Slot::Native
    );
    let mut browse = start::<FetchAllPlaylistsRequest>(&remote);
    browse.send(FetchAllPlaylistsRequest {});
    let response: FetchAllPlaylistsResponse = browse.receive("FetchAllPlaylistsResponse").await;
    assert_eq!(
        response
            .playlists
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["only on B"]
    );
    browse.stop().await;
    let mut missing = start::<GetPlaylistByIdRequest>(&remote);
    missing.send(GetPlaylistByIdRequest { playlist_id: -1 });
    let failure: RequestFailedResponse = missing.receive("RequestFailedResponse").await;
    assert_eq!(failure.kind, RequestErrorKind::NotFound);
    missing.stop().await;

    let local_volume = a
        .global_params
        .node
        .session()?
        .playback_snapshot()
        .await
        .0
        .volume;
    dispatch_gui_request(&remote, &VolumeRequest { volume: 0.61 }).await?;
    wait_player(&b, |status| status.volume == 0.61).await?;
    assert_eq!(
        a.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .volume,
        local_volume
    );
    let local = dispatch_gui_request(&remote, &ListLocalLibrariesRequest {})
        .await?
        .context("local libraries")?;
    assert!(local.libraries.iter().any(|library| {
        Some(library.library_id.as_str())
            == a.global_params
                .library_id
                .map(|id| id.to_string())
                .as_deref()
    }));
    assert!(!local.libraries.iter().any(|library| {
        Some(library.library_id.as_str())
            == b.global_params
                .library_id
                .map(|id| id.to_string())
                .as_deref()
    }));

    signals.0.lock().unwrap().clear();
    create(&remote, "edited through A").await?;
    assert!(
        names(&b.global_params)
            .await?
            .contains(&"edited through A".into())
    );
    // An explicitly addressed request still uses A's library, even while its GUI is remote.
    let (_, bytes) = dispatch_remote(
        "FetchAllPlaylistsRequest",
        &rinf::serialize(&FetchAllPlaylistsRequest {})?,
        None,
        a.global_params.library_id,
        &a.global_params.node,
    )
    .await
    .context("addressed request")?;
    let own: FetchAllPlaylistsResponse = rinf::deserialize(&bytes?)?;
    assert_eq!(
        own.playlists
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["only on A"]
    );
    dispatch_gui_request(
        &remote,
        &OperatePlaybackWithMixQueryRequest {
            queries: vec![MixQuery {
                operator: "lib::track".into(),
                parameter: b_track.to_string(),
            }],
            playback_mode: 0,
            hint_position: -1,
            initial_playback_item: None,
            instantly_play: true,
            operate_mode: PlaylistOperateMode::Replace,
            fallback_playing_items: Vec::new(),
            expected_revision: None,
            cast_to: None,
        },
    )
    .await?;
    wait_player(&b, |status| status.state == PlaybackState::Playing).await?;
    assert_eq!(
        a.global_params.player.lock().await.get_playlist(),
        local_queue
    );
    b.global_params
        .broadcaster
        .broadcast(&ScanAudioLibraryProgress {
            scan_id: "rr-scan".into(),
            path: "B".into(),
            progress: 3,
            total: 10,
            task: ScanTaskType::IndexFiles,
        });
    timeout(Duration::from_secs(10), async {
        loop {
            let ready = {
                let events = signals.0.lock().unwrap();
                let status = events.iter().any(|(name, body)| {
                    name == "PlaybackStatus"
                        && rinf::deserialize::<PlaybackStatus>(body)
                            .is_ok_and(|s| s.session_id == remote_session_id)
                });
                let content = events.iter().any(|(name, body)| {
                    name == "LibraryContentChanged"
                        && rinf::deserialize::<LibraryContentChanged>(body)
                            .is_ok_and(|s| s.path == b.global_params.test_library().media_root)
                });
                let scan = events.iter().any(|(name, body)| {
                    name == "ScanAudioLibraryProgress"
                        && rinf::deserialize::<ScanAudioLibraryProgress>(body)
                            .is_ok_and(|s| s.scan_id == "rr-scan")
                });
                status && content && scan
            };
            if ready {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("RR pushed status/content/scan events missing")?;

    let mut close = start::<CloseLibraryRequest>(&remote);
    close.send(CloseLibraryRequest { path: path.clone() });
    let closed: CloseLibraryResponse = close.receive("CloseLibraryResponse").await;
    assert_eq!(closed.path, path);
    close.stop().await;
    assert!(a.global_params.node.renderer_link()?.is_none());
    assert!(!a.global_params.node.remote_ui_library()?);
    assert!(a.global_params.node.current_ui().await.is_err());
    let local = Arc::new(
        a.global_params
            .node
            .params_for_library(a.global_params.test_library().clone())?,
    );
    a.global_params.node.attach_ui(&local).await;
    assert_eq!(names(&local).await?, ["only on A"]);
    dispatch_gui_request(&local, &VolumeRequest { volume: 0.27 }).await?;
    wait_player(&a, |status| status.volume == 0.27).await?;
    assert_eq!(
        b.global_params
            .node
            .session()?
            .playback_snapshot()
            .await
            .0
            .volume,
        0.61
    );

    // Switching directly to a local library also leaves RR.
    let remote = open_remote_library(a.global_params.node.clone(), &path).await?;
    let replacement = Arc::new(
        a.global_params
            .node
            .params_for_library(a.global_params.test_library().clone())?,
    );
    a.global_params.node.attach_ui(&replacement).await;
    assert!(remote.main_token.is_cancelled());
    assert!(a.global_params.node.renderer_link()?.is_none());
    assert_eq!(names(&replacement).await?, ["only on A"]);

    let remote = open_remote_library(a.global_params.node.clone(), &path).await?;
    signals.0.lock().unwrap().clear();
    b.global_params.node.manager()?.stop().await?;
    timeout(Duration::from_secs(10), async {
        loop {
            let found = signals.0.lock().unwrap().iter().any(|(name, body)| {
                name == "RemoteConnectionLost"
                    && rinf::deserialize::<RemoteConnectionLost>(body)
                        .is_ok_and(|loss| loss.kind == RemoteLossKind::ServerStopped)
            });
            if found {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("RR connection-lost notice missing")?;
    assert!(a.global_params.node.renderer_link()?.is_none());
    let failure = dispatch_gui_request(&remote, &FetchAllPlaylistsRequest {})
        .await
        .err()
        .context("lost link silently used local library")?;
    assert_eq!(classify(&failure), RequestErrorKind::Unavailable);
    // This is the existing Dart loss handler's final step: close and await its response.
    let mut close = start::<CloseLibraryRequest>(&remote);
    close.send(CloseLibraryRequest { path: path.clone() });
    let _: CloseLibraryResponse = close.receive("CloseLibraryResponse").await;
    close.stop().await;
    assert!(!a.global_params.node.remote_ui_library()?);
    assert!(a.global_params.node.current_ui().await.is_err());
    a.global_params.node.shutdown().await?;
    b.global_params.node.shutdown().await?;
    Ok(())
}

fn reconnect_config(deadline: Duration) -> ReconnectConfig {
    ReconnectConfig {
        delays: [Duration::from_millis(50); 4],
        interval: Duration::from_millis(80),
        deadline,
    }
}

struct ReconnectFixture {
    a: Fixture,
    b: Fixture,
    remote: Arc<GlobalParams>,
    signals: Arc<RecordedSignals>,
    address: SocketAddr,
    fingerprint: String,
    _directories: [TempDir; 2],
}

impl ReconnectFixture {
    async fn new(deadline: Duration) -> Result<Self> {
        let directories = [tempfile::tempdir()?, tempfile::tempdir()?];
        let signals = Arc::new(RecordedSignals::default());
        let a = super::fixture_with_reconnect_config(
            directories[0].path(),
            Uuid::new_v4(),
            Duration::from_secs(300),
            signals.clone(),
            reconnect_config(deadline),
        )
        .await?;
        let b = fixture(directories[1].path()).await?;
        let address = super::paired_full(&b, &a).await?;
        let fingerprint = super::info(&b).await?.fingerprint;
        a.global_params
            .cert_validator
            .read()
            .await
            .replace_hosts_for_fingerprint(
                &fingerprint,
                vec![format!("127.0.0.1:{}", address.port())],
            )
            .await?;
        let track = session_audio(&b, "reconnect.ogg").await?;
        queue_session(&b, &[track]).await?;
        let remote =
            open_remote_library(a.global_params.node.clone(), &format!("ws://{address}")).await?;
        signals.clear();
        Ok(Self {
            a,
            b,
            remote,
            signals,
            address,
            fingerprint,
            _directories: directories,
        })
    }

    async fn wait_signal<T: DeserializeOwned>(&self, name: &str) -> Result<T> {
        timeout(Duration::from_secs(5), async {
            loop {
                if let Some(event) = self.signals.decoded(name).into_iter().next() {
                    return event;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .with_context(|| format!("{name} missing"))
    }

    async fn disconnect(&self) -> Result<()> {
        self.b.manager.stop().await?;
        let event: RemoteLibraryReconnecting =
            self.wait_signal("RemoteLibraryReconnecting").await?;
        assert_eq!(event.fingerprint, self.fingerprint);
        assert_eq!(event.alias, super::info(&self.b).await?.alias);
        assert!(!self.remote.main_token.is_cancelled());
        assert!(self.a.global_params.node.remote_ui_library()?);
        assert!(
            self.signals
                .decoded::<RemoteConnectionLost>("RemoteConnectionLost")
                .is_empty()
        );
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        self.a.global_params.node.shutdown().await?;
        self.b.global_params.node.shutdown().await?;
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_reconnect_restores_real_node_requests() -> Result<()> {
    let f = ReconnectFixture::new(Duration::from_secs(10)).await?;
    f.disconnect().await?;
    let mut browse = start::<FetchAllPlaylistsRequest>(&f.remote);
    browse.send(FetchAllPlaylistsRequest {});
    let failure: RequestFailedResponse = timeout(
        Duration::from_millis(500),
        browse.receive("RequestFailedResponse"),
    )
    .await?;
    assert_eq!(failure.kind, RequestErrorKind::Unavailable);
    assert_eq!(
        failure.cause,
        Some(RequestFailureCause::Reconnecting {
            alias: super::info(&f.b).await?.alias
        })
    );
    browse.stop().await;
    let failure = timeout(
        Duration::from_millis(500),
        dispatch_gui_request(&f.remote, &VolumeRequest { volume: 0.7 }),
    )
    .await?
    .err()
    .context("session request must fail")?;
    assert!(matches!(
        failure.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Reconnecting { .. })
    ));
    dispatch_gui_request(&f.remote, &ListLocalLibrariesRequest {})
        .await?
        .context("node requests stay local")?;
    f.signals.clear();
    f.a.global_params
        .node
        .session()?
        .player
        .lock()
        .await
        .set_volume(0.2);
    sleep(Duration::from_millis(50)).await;
    assert!(
        f.signals
            .decoded::<PlaybackStatus>("PlaybackStatus")
            .is_empty(),
        "local session replaced the remote view"
    );
    f.b.manager
        .clone()
        .start(
            f.address,
            DiscoveryParams {
                device_info: super::info(&f.b).await?,
            },
        )
        .await?;
    let event: RemoteLibraryReconnected = f.wait_signal("RemoteLibraryReconnected").await?;
    assert_eq!(event.fingerprint, f.fingerprint);
    assert_eq!(event.alias, super::info(&f.b).await?.alias);
    let session_id = f.b.global_params.node.session()?.session_id().await;
    let status: PlaybackStatus = f.wait_signal("PlaybackStatus").await?;
    let queue: PlaylistUpdate = f.wait_signal("PlaylistUpdate").await?;
    assert_eq!(status.session_id, session_id);
    assert_eq!(queue.session_id, session_id);
    assert_eq!(queue.items.len(), 1);
    dispatch_gui_request(&f.remote, &FetchAllPlaylistsRequest {})
        .await?
        .context("remote request")?;
    dispatch_gui_request(&f.remote, &VolumeRequest { volume: 0.7 }).await?;
    wait_player(&f.b, |status| status.volume == 0.7).await?;
    f.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_reconnect_deadline_uses_existing_loss_and_close() -> Result<()> {
    let f = ReconnectFixture::new(Duration::from_millis(400)).await?;
    let started = Instant::now();
    f.disconnect().await?;
    let loss: RemoteConnectionLost = f.wait_signal("RemoteConnectionLost").await?;
    assert_eq!(loss.kind, RemoteLossKind::ServerStopped);
    assert!(started.elapsed() >= Duration::from_millis(400));
    assert!(f.a.global_params.node.reconnect_attempt_count() > 0);
    let mut close = start::<CloseLibraryRequest>(&f.remote);
    close.send(CloseLibraryRequest {
        path: format!("ws://{}", f.address),
    });
    let _: CloseLibraryResponse = close.receive("CloseLibraryResponse").await;
    close.stop().await;
    assert!(!f.a.global_params.node.remote_ui_library()?);
    assert!(f.a.global_params.node.current_ui().await.is_err());
    assert!(f.remote.main_token.is_cancelled());
    assert!(
        f.signals
            .decoded::<RemoteLibraryReconnected>("RemoteLibraryReconnected")
            .is_empty()
    );
    f.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_reconnect_leaving_stops_attempts() -> Result<()> {
    for exit in ["close", "switch", "shutdown"] {
        let f = ReconnectFixture::new(Duration::from_secs(2)).await?;
        f.disconnect().await?;
        timeout(Duration::from_secs(1), async {
            while f.a.global_params.node.reconnect_attempt_count() == 0 {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        let replacement = Arc::new(
            f.a.global_params
                .node
                .params_for_library(f.a.global_params.test_library().clone())?,
        );
        match exit {
            "close" => f.a.global_params.node.close_ui(&f.remote.main_token).await,
            "switch" => f.a.global_params.node.attach_ui(&replacement).await,
            _ => f.a.global_params.node.shutdown().await?,
        }
        let attempts = f.a.global_params.node.reconnect_attempt_count();
        f.b.manager
            .clone()
            .start(
                f.address,
                DiscoveryParams {
                    device_info: super::info(&f.b).await?,
                },
            )
            .await?;
        sleep(Duration::from_millis(350)).await;
        assert_eq!(
            f.a.global_params.node.reconnect_attempt_count(),
            attempts,
            "attempt after {exit}"
        );
        assert!(f.a.global_params.node.renderer_link()?.is_none());
        assert!(
            f.signals
                .decoded::<RemoteLibraryReconnected>("RemoteLibraryReconnected")
                .is_empty()
        );
        assert!(
            f.signals
                .decoded::<RemoteConnectionLost>("RemoteConnectionLost")
                .is_empty()
        );
        f.shutdown().await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_capability_snapshot_marks_every_request_available() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let fixture = fixture_with_broadcaster(
        dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;

    assert_eq!(
        capability_snapshot(&fixture.global_params.node)?,
        RequestCapabilities::all_available()
    );
    assert_eq!(
        signals
            .decoded::<RequestCapabilities>("RequestCapabilities")
            .len(),
        1
    );
    fixture.global_params.node.publish_capabilities()?;
    assert_eq!(
        signals
            .decoded::<RequestCapabilities>("RequestCapabilities")
            .len(),
        1
    );
    fixture.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attached_renderer_selects_rune_slots_and_suppresses_equal_capabilities() -> Result<()> {
    use crate::messages::{HandoffRequest, PauseRequest};

    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let controller = fixture_with_broadcaster(
        controller_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals.clone(),
    )
    .await?;
    let renderer = super::fixture(renderer_dir.path()).await?;
    let renderer_address = super::paired_full(&renderer, &controller).await?;
    let renderer_info = super::info(&renderer).await?;
    controller
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &renderer_info.fingerprint,
            vec![format!("127.0.0.1:{}", renderer_address.port())],
        )
        .await?;

    signals.clear();
    assert_eq!(
        RpcTarget::select_type::<PauseRequest>(&controller.global_params.node)?
            .slot_type::<PauseRequest>(),
        Slot::Native
    );
    controller
        .global_params
        .node
        .attach_renderer(&renderer_info.fingerprint)
        .await?;
    assert_eq!(
        RpcTarget::select_type::<PauseRequest>(&controller.global_params.node)?
            .slot_type::<PauseRequest>(),
        Slot::Forward
    );
    assert_eq!(
        RpcTarget::select_type::<HandoffRequest>(&controller.global_params.node)?
            .slot_type::<HandoffRequest>(),
        Slot::Native
    );
    let capabilities = capability_snapshot(&controller.global_params.node)?;
    assert_eq!(capabilities.pause_request, RequestCapability::Available);
    assert_eq!(capabilities.handoff_request, RequestCapability::Available);
    let snapshots = signals.decoded::<RequestCapabilities>("RequestCapabilities");
    assert!(
        snapshots.is_empty(),
        "equal capability snapshots are suppressed"
    );
    controller.global_params.node.publish_capabilities()?;
    assert_eq!(
        signals
            .decoded::<RequestCapabilities>("RequestCapabilities")
            .len(),
        0,
        "a no-op publication must be suppressed"
    );

    controller.global_params.node.detach_renderer().await?;
    assert_eq!(
        signals
            .decoded::<RequestCapabilities>("RequestCapabilities")
            .len(),
        0,
        "detach restores the same capability snapshot and is suppressed"
    );
    controller.global_params.node.shutdown().await?;
    renderer.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rr_capability_snapshot_uses_rune_library_and_local_close_slots() -> Result<()> {
    let local_dir = tempfile::tempdir()?;
    let remote_dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let local = fixture_with_broadcaster(
        local_dir.path(),
        Uuid::new_v4(),
        Duration::from_secs(300),
        signals,
    )
    .await?;
    let remote = super::fixture(remote_dir.path()).await?;
    let address = super::paired_full(&remote, &local).await?;
    let remote_info = super::info(&remote).await?;
    local
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &remote_info.fingerprint,
            vec![format!("127.0.0.1:{}", address.port())],
        )
        .await?;
    let remote_params = open_remote_library(
        local.global_params.node.clone(),
        &format!("@RR|ws://{address}"),
    )
    .await?;

    let snapshot = capability_snapshot(&local.global_params.node)?;
    assert_eq!(
        snapshot.fetch_all_playlists_request,
        RequestCapability::Available
    );
    assert_eq!(snapshot.close_library_request, RequestCapability::Available);
    assert_eq!(
        RpcTarget::select_type::<FetchAllPlaylistsRequest>(&local.global_params.node)?
            .slot_type::<FetchAllPlaylistsRequest>(),
        Slot::Forward
    );
    assert_eq!(
        RpcTarget::select_type::<CloseLibraryRequest>(&local.global_params.node)?
            .slot_type::<CloseLibraryRequest>(),
        Slot::Native
    );

    local
        .global_params
        .node
        .close_ui(&remote_params.main_token)
        .await;
    local.global_params.node.shutdown().await?;
    remote.global_params.node.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn capability_snapshot_agrees_with_unsupported_dispatch_by_scope() -> Result<()> {
    use crate::messages::{FetchAllPlaylistsRequest, ListLocalLibrariesRequest, PauseRequest};

    let dir = tempfile::tempdir()?;
    let fixture = super::fixture(dir.path()).await?;
    let params = &fixture.global_params;
    let unsupported = Slot::Unsupported {
        reason: UnsupportedReason::NotImplemented,
    };
    assert_eq!(
        RequestCapability::from(Slot::Translate),
        RequestCapability::Available
    );
    assert_eq!(
        RequestCapability::from(Slot::Unsupported {
            reason: UnsupportedReason::OwnerOnly,
        }),
        RequestCapability::Disabled {
            reason: CapabilityReason::OwnerOnly,
        }
    );

    let node_request = ListLocalLibrariesRequest {};
    let node_target = RpcTarget::local().with_slots(|slots| {
        slots.list_local_libraries_request = unsupported;
    });
    assert_eq!(
        RequestCapabilities::from_target(&node_target).list_local_libraries_request,
        RequestCapability::Hidden
    );
    let node_error = run_on_target(&node_target, params, &node_request)
        .await
        .err()
        .context("unsupported node request should fail")?;
    assert!(matches!(
        node_error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unsupported {
            reason: UnsupportedReason::NotImplemented,
            ..
        })
    ));
    assert!(dispatch_gui_request(params, &node_request).await.is_ok());

    let session_request = PauseRequest {};
    let session_target = RpcTarget::local().with_slots(|slots| {
        slots.pause_request = unsupported;
    });
    assert_eq!(
        RequestCapabilities::from_target(&session_target).pause_request,
        RequestCapability::Hidden
    );
    let session_error = run_on_target(&session_target, params, &session_request)
        .await
        .err()
        .context("unsupported session request should fail")?;
    assert!(matches!(
        session_error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unsupported {
            reason: UnsupportedReason::NotImplemented,
            ..
        })
    ));
    assert!(dispatch_gui_request(params, &session_request).await.is_ok());

    let library_request = FetchAllPlaylistsRequest {};
    let library_target = RpcTarget::local().with_slots(|slots| {
        slots.fetch_all_playlists_request = unsupported;
    });
    assert_eq!(
        RequestCapabilities::from_target(&library_target).fetch_all_playlists_request,
        RequestCapability::Hidden
    );
    let library_error = run_on_target(&library_target, params, &library_request)
        .await
        .err()
        .context("unsupported library request should fail")?;
    assert!(matches!(
        library_error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unsupported {
            reason: UnsupportedReason::NotImplemented,
            ..
        })
    ));
    assert!(dispatch_gui_request(params, &library_request).await.is_ok());

    fixture.global_params.node.shutdown().await?;
    Ok(())
}
