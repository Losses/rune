/// Source handing off to a target whose copy of the track does not decode, with the target's
/// own track playing. The handoff's rollback is the moment the tests cut the transport.
struct HandoffReconcileFixture {
    _dirs: Vec<TempDir>,
    source: Fixture,
    target: Fixture,
    host: Fixture,
    _target_replica: Fixture,
    target_fingerprint: String,
    target_address: SocketAddr,
    target_file: PathBuf,
    valid_audio: Vec<u8>,
    handed_off: PlayingItem,
    expected_item: PlayingItem,
}

async fn handoff_reconcile_fixture(name: &str) -> Result<HandoffReconcileFixture> {
    use crate::access_policy::{ConnectionLevel, DeviceGrants, LibraryGrants, SyncLevel};
    let host_dir = tempfile::tempdir()?;
    let source_dir = tempfile::tempdir()?;
    let target_dir = tempfile::tempdir()?;
    let host = fixture(host_dir.path()).await?;
    let source = fixture(source_dir.path()).await?;
    let target = fixture(target_dir.path()).await?;
    let file_name = format!("{name}.ogg");
    let host_id = session_audio(&host, &file_name).await?;
    let (host_address, source_replica) = replicate_host_library(&host, &source)
        .await
        .context("replicate source library")?;
    pair(&host, &target).await?;
    pair(&target, &host).await?;
    let host_library_id = host.global_params.library_id.context("host library id")?;
    let target_fingerprint = info(&target).await?.fingerprint;
    host.global_params.node.access_policy.set(
        &target_fingerprint,
        DeviceGrants {
            control: true,
            connection: LibraryGrants {
                all_libraries: None,
                libraries: [(host_library_id, ConnectionLevel::ViewEdit)]
                    .into_iter()
                    .collect(),
            },
            sync: LibraryGrants {
                all_libraries: None,
                libraries: [(host_library_id, SyncLevel::PullPush)]
                    .into_iter()
                    .collect(),
            },
            administer: false,
        },
    )?;
    let (_, target_replica) = replicate_host_library_at(&host, &target, host_address)
        .await
        .context("replicate target library")?;
    let source_file =
        Path::new(&source_replica.global_params.test_library().media_root).join(&file_name);
    fs::copy(
        Path::new(&host.global_params.test_library().media_root).join(&file_name),
        &source_file,
    )?;
    let host_item = library_track_for_id(&host, host_id).await?;
    let source_item = player::LibraryItem {
        library_id: target_replica
            .global_params
            .library_id
            .context("target replica library id")?,
        file_uuid: host_item.file_uuid,
    };
    source
        .global_params
        .node
        .session()?
        .edit_queue(None, |player| {
            player.add_to_playlist_with_sources(
                vec![(
                    PlayingItem::from(source_item),
                    PlaybackSource::Local(source_file.clone()),
                )],
                AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    source
        .global_params
        .player
        .lock()
        .await
        .set_playback_mode(PlaybackMode::RepeatAll);
    source.global_params.player.lock().await.load(0);
    source.global_params.player.lock().await.play();
    let source_before =
        wait_player(&source, |status| status.state == PlayerPlaybackState::Playing).await?;
    let expected_item = source_before.item.clone().context("source item")?;

    let source_fingerprint = info(&source).await?.fingerprint;
    let host_fingerprint = info(&host).await?.fingerprint;
    target
        .global_params
        .node
        .access_policy
        .set(&source_fingerprint, DeviceGrants::my_device())?;
    target
        .global_params
        .node
        .access_policy
        .set(&host_fingerprint, DeviceGrants::my_device())?;
    let target_address = paired_full(&target, &source)
        .await
        .context("pair target renderer")?;
    source
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &target_fingerprint,
            vec![format!("127.0.0.1:{}", target_address.port())],
        )
        .await?;

    // The target plays its own track: its original queue must survive every rollback.
    let target_id = session_audio(&target, &format!("{name}-target.ogg")).await?;
    queue_session(&target, &[target_id]).await?;
    target.global_params.player.lock().await.load(0);
    target.global_params.player.lock().await.play();
    wait_player(&target, |status| status.state == PlayerPlaybackState::Playing).await?;

    // Give the replica a valid copy first, then corrupt it in place: the load is accepted
    // while the decode fails, so the handoff initiates its rollback.
    let target_track_id = file_ids_by_uuid(
        &target_replica.global_params.test_library().main_db,
        &[host_item.file_uuid],
    )
    .await?
    .get(&host_item.file_uuid)
    .copied()
    .context("target replica track id")?;
    let host_file = Path::new(&host.global_params.test_library().media_root).join(&file_name);
    materialize_replica_file(&target_replica, target_track_id, &host_file, &file_name).await?;
    let target_file =
        Path::new(&target_replica.global_params.test_library().media_root).join(&file_name);
    let valid_audio = fs::read(&target_file)?;
    fs::write(&target_file, b"not an audio file")?;

    let handed_off = PlayingItem::LibraryTrack {
        library_id: target_replica
            .global_params
            .library_id
            .context("target replica library id")?,
        track: host_item.file_uuid.to_string(),
    };
    Ok(HandoffReconcileFixture {
        _dirs: vec![host_dir, source_dir, target_dir],
        source,
        target,
        host,
        _target_replica: target_replica,
        target_fingerprint,
        target_address,
        target_file,
        valid_audio,
        handed_off,
        expected_item,
    })
}

impl HandoffReconcileFixture {
    /// The target's take-back endpoint reports the request to the test. What the override does
    /// with it is `cut`: either it never applies the take-back, or it applies it and drops the
    /// answer.
    async fn install_take_back_cut(&self, apply: bool) -> Arc<Notify> {
        let rollback_entered = Arc::new(Notify::new());
        let entered = rollback_entered.clone();
        let cut_once = Arc::new(AtomicBool::new(false));
        let target_node = Arc::downgrade(&self.target.global_params.node);
        let take_back = "TakeBackHandoffBlockRequest";
        let entered_clone = entered.clone();
        self.target
            .global_params
            .node
            .websocket_service
            .override_handler(take_back, move |payload, session, library_id| {
                let target_node = target_node.clone();
                let entered = entered_clone.clone();
                let cut_once = cut_once.clone();
                async move {
                    let node = target_node.upgrade().expect("target node");
                    if cut_once.swap(true, Ordering::SeqCst) {
                        return dispatch_remote(take_back, &payload, session, library_id, &node)
                            .await
                            .expect("real handoff rollback handler");
                    }
                    if apply {
                        let response =
                            dispatch_remote(take_back, &payload, session, library_id, &node)
                                .await
                                .expect("real handoff rollback handler");
                        // The block is taken back now, but this answer is never sent: the
                        // transport goes down with it still inside the handler.
                        let _kept = response;
                        // Report only once the take-back is applied: stopping the server on
                        // an earlier signal could still drop the handler mid-apply, and the
                        // test would exercise the unapplied case it does not name.
                    }
                    entered.notify_one();
                    std::future::pending::<()>().await;
                    unreachable!("the stopped server drops this pending handler")
                }
            })
            .await;
        rollback_entered
    }

    /// The target's take-back endpoint answers its first request with a refusal rather than a
    /// transport loss: the connection carrying it stays usable, but the reconciled block is
    /// neither applied nor confirmed absent. Later requests reach the real handler.
    async fn install_take_back_refusal(&self) -> Arc<Notify> {
        let reconcile_refused = Arc::new(Notify::new());
        let refused = reconcile_refused.clone();
        let refuse_once = Arc::new(AtomicBool::new(false));
        let target_node = Arc::downgrade(&self.target.global_params.node);
        let take_back = "TakeBackHandoffBlockRequest";
        self.target
            .global_params
            .node
            .websocket_service
            .override_handler(take_back, move |payload, session, library_id| {
                let target_node = target_node.clone();
                let refused = refused.clone();
                let refuse_once = refuse_once.clone();
                async move {
                    let node = target_node.upgrade().expect("target node");
                    if refuse_once.swap(true, Ordering::SeqCst) {
                        return dispatch_remote(take_back, &payload, session, library_id, &node)
                            .await
                            .expect("real handoff rollback handler");
                    }
                    refused.notify_one();
                    (
                        String::new(),
                        Err(anyhow::Error::new(ExpectedRequestError::invalid_input(
                            "test: the reconciling take-back is refused",
                        ))),
                    )
                }
            })
            .await;
        reconcile_refused
    }

    async fn shutdown(&self) -> Result<()> {
        self.source.global_params.node.shutdown().await?;
        self.target.global_params.node.shutdown().await?;
        self.host.global_params.node.shutdown().await?;
        Ok(())
    }
}

/// After the rollback's transport died, the retry must not stack the new load on top of whatever
/// the target still holds. `block_retained` is whether the cut kept the take-back from applying.
async fn assert_retry_reconciled(f: &HandoffReconcileFixture, block_retained: bool) -> Result<()> {
    use crate::messages::HandoffRequest;

    let target_before = f.target.global_params.player.lock().await.get_status();
    let original_items: Vec<_> = target_before
        .playlist
        .iter()
        .filter(|entry| entry.item != f.handed_off)
        .map(|entry| entry.item.clone())
        .collect();
    assert!(
        !original_items.is_empty(),
        "the target should have its own queue before the handoff"
    );

    let _target_after_failure = if block_retained {
        // The rollback never applied: the target still holds the handed-off block.
        let status = wait_player(&f.target, |status| {
            status
                .playlist
                .iter()
                .any(|entry| entry.item == f.handed_off)
        })
        .await
        .context("the unapplied rollback leaves the handoff block on the target")?;
        assert!(
            status
                .playlist
                .iter()
                .all(|entry| original_items.contains(&entry.item) || entry.item == f.handed_off),
            "the target's original queue must survive the failed rollback: {:?}",
            status.playlist
        );
        status
    } else {
        // The rollback applied, but its answer never left: the block is already gone.
        wait_player(&f.target, |status| {
            !status
                .playlist
                .iter()
                .any(|entry| entry.item == f.handed_off)
        })
        .await
        .context("the applied rollback removes the handoff block on the target")?
    };
    wait_player(&f.source, |status| {
        status.state == PlayerPlaybackState::Playing && status.item == Some(f.expected_item.clone())
    })
    .await
    .context("the source resumes after the failed handoff")?;

    // Connectivity returns; the retried handoff is the first connection to the target.
    f.target
        .manager
        .clone()
        .start(
            f.target_address,
            DiscoveryParams {
                device_info: info(&f.target).await?,
            },
            true,
        )
        .await?;
    fs::write(&f.target_file, &f.valid_audio)?;
    let retry = HandoffRequest {
        ticket: None,
        call_id: None,
        target: Some(f.target_fingerprint.clone()),
    };
    timeout(
        TEST_DEADLINE,
        dispatch_gui_request(&f.source.global_params, &retry),
    )
    .await
    .context("retried handoff did not finish")?
    .context("retried handoff should succeed")?;

    let target_after_retry = wait_player(&f.target, |status| {
        status.item == Some(f.handed_off.clone()) && status.state == PlayerPlaybackState::Playing
    })
    .await
    .context("the retried handoff should play on the target")?;
    let handed_off_count = target_after_retry
        .playlist
        .iter()
        .filter(|entry| entry.item == f.handed_off)
        .count();
    assert_eq!(
        handed_off_count, 1,
        "the retry must not duplicate the handed-off track on the target's queue: {:?}",
        target_after_retry.playlist
    );
    assert!(
        original_items
            .iter()
            .all(|item| target_after_retry.playlist.iter().any(|entry| &entry.item == item)),
        "the target's original queue must survive the retry: {:?}",
        target_after_retry.playlist
    );
    wait_player(&f.source, |status| status.state == PlayerPlaybackState::Paused)
        .await
        .context("only the target should play after the retried handoff")?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn handoff_rollback_transport_loss_reconciles_on_reconnect() -> Result<()> {
    use crate::messages::HandoffRequest;

    let f = handoff_reconcile_fixture("handoff-rollback-cut").await?;
    // The cut lands before the take-back applies: the target keeps the block, and whether it
    // does is exactly what the source cannot know.
    let rollback_entered = f.install_take_back_cut(false).await;
    let source_params = f.source.global_params.clone();
    let handoff = HandoffRequest {
        ticket: None,
        call_id: None,
        target: Some(f.target_fingerprint.clone()),
    };
    let handoff_task = tokio::spawn(async move { dispatch_gui_request(&source_params, &handoff).await });
    timeout(TEST_DEADLINE, rollback_entered.notified())
        .await
        .context("the decode failure should request a handoff rollback")?;
    f.target.manager.stop().await?;
    let result = handoff_task
        .await
        .context("join cut-rollback handoff")?
        .context("dispatch cut-rollback handoff");
    assert!(
        result.as_ref().is_err(),
        "a handoff whose rollback loses its connection must fail: {result:?}"
    );
    assert_retry_reconciled(&f, true).await?;
    f.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn handoff_rollback_answer_loss_reconciles_on_reconnect() -> Result<()> {
    use crate::messages::HandoffRequest;

    let f = handoff_reconcile_fixture("handoff-rollback-answer-cut").await?;
    // The cut lands after the take-back applied but before its answer left: the target no
    // longer holds the block, and the reconciling take-back must read the target's `Unavailable`
    // answer as exactly that, not as another transport failure.
    let rollback_entered = f.install_take_back_cut(true).await;
    let source_params = f.source.global_params.clone();
    let handoff = HandoffRequest {
        ticket: None,
        call_id: None,
        target: Some(f.target_fingerprint.clone()),
    };
    let handoff_task = tokio::spawn(async move { dispatch_gui_request(&source_params, &handoff).await });
    timeout(TEST_DEADLINE, rollback_entered.notified())
        .await
        .context("the decode failure should request a handoff rollback")?;
    f.target.manager.stop().await?;
    let result = handoff_task
        .await
        .context("join answer-cut handoff")?
        .context("dispatch answer-cut handoff");
    assert!(
        result.as_ref().is_err(),
        "a handoff whose rollback answer is lost must fail: {result:?}"
    );
    assert_retry_reconciled(&f, false).await?;
    f.shutdown().await
}

/// A reconcile attempt whose answer never arrives must end, not retry the dead request inside
/// the same connection. The target's first take-back after the restart is cut in flight, so the
/// reconcile fails with a transport error: the connection must still finish, and the block must
/// stay pending for the next connection to reconcile.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn handoff_rollback_failed_reconcile_retries_on_next_connection() -> Result<()> {
    use crate::messages::HandoffRequest;

    let f = handoff_reconcile_fixture("handoff-rollback-failed-reconcile").await?;
    // The rollback never applied: the target keeps the block, and the source records its
    // outcome as unknown.
    let rollback_entered = f.install_take_back_cut(false).await;
    let source_params = f.source.global_params.clone();
    let handoff = HandoffRequest {
        ticket: None,
        call_id: None,
        target: Some(f.target_fingerprint.clone()),
    };
    let handoff_task = tokio::spawn(async move { dispatch_gui_request(&source_params, &handoff).await });
    timeout(TEST_DEADLINE, rollback_entered.notified())
        .await
        .context("the decode failure should request a handoff rollback")?;
    f.target.manager.stop().await?;
    let result = handoff_task
        .await
        .context("join cut-rollback handoff")?
        .context("dispatch cut-rollback handoff");
    assert!(
        result.as_ref().is_err(),
        "a handoff whose rollback loses its connection must fail: {result:?}"
    );

    // Connectivity returns; this time the reconciling take-back is itself cut in flight, so
    // its caller sees a transport error rather than a refusal. The connection carrying it must
    // finish anyway: retrying the dead request inside the same connection would never end.
    let reconcile_entered = f.install_take_back_cut(false).await;
    f.target
        .manager
        .clone()
        .start(
            f.target_address,
            DiscoveryParams {
                device_info: info(&f.target).await?,
            },
            true,
        )
        .await?;
    let source_node = f.source.global_params.node.clone();
    let target_fingerprint = f.target_fingerprint.clone();
    let connect = tokio::spawn(async move {
        source_node
            .connect_renderer(&target_fingerprint, RendererLinkUse::Cast)
            .await
    });
    timeout(TEST_DEADLINE, reconcile_entered.notified())
        .await
        .context("the reconnecting reconcile should request its take-back")?;
    f.target.manager.stop().await?;
    let _ = timeout(TEST_DEADLINE, connect)
        .await
        .context("a failed reconcile must not hold its connection open")?
        .context("join failed-reconcile connect")?;

    // The failed attempt keeps the block pending: with the server answering normally again,
    // the next connection reconciles it and the handoff can be retried end to end.
    f.target
        .manager
        .clone()
        .start(
            f.target_address,
            DiscoveryParams {
                device_info: info(&f.target).await?,
            },
            true,
        )
        .await?;
    timeout(
        TEST_DEADLINE,
        f.source
            .global_params
            .node
            .connect_renderer(&f.target_fingerprint, RendererLinkUse::Cast),
    )
    .await
    .context("the reconciling connection should finish")?
    .context("the reconciling connection should succeed")?;
    f.target.manager.stop().await?;
    fs::write(&f.target_file, &f.valid_audio)?;
    assert_retry_reconciled(&f, false).await?;
    f.shutdown().await
}

/// A reconciling take-back that is refused rather than lost still leaves its connection usable,
/// so the retried handoff reaches the load step on it. The target must not receive that load
/// while the old block is unresolved: the handoff fails, the target's queue stays as the failed
/// rollback left it, and a later reconciliation clears the way for the retry.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn handoff_rollback_pending_blocks_retry_until_reconciled() -> Result<()> {
    use crate::messages::HandoffRequest;

    let f = handoff_reconcile_fixture("handoff-rollback-pending-retry").await?;
    // The rollback never applies: the target keeps the block, and the source records its
    // outcome as unknown.
    let rollback_entered = f.install_take_back_cut(false).await;
    let source_params = f.source.global_params.clone();
    let handoff = HandoffRequest {
        ticket: None,
        call_id: None,
        target: Some(f.target_fingerprint.clone()),
    };
    let handoff_task = tokio::spawn(async move { dispatch_gui_request(&source_params, &handoff).await });
    timeout(TEST_DEADLINE, rollback_entered.notified())
        .await
        .context("the decode failure should request a handoff rollback")?;
    f.target.manager.stop().await?;
    let result = handoff_task
        .await
        .context("join cut-rollback handoff")?
        .context("dispatch cut-rollback handoff");
    assert!(
        result.as_ref().is_err(),
        "a handoff whose rollback loses its connection must fail: {result:?}"
    );

    // The rollback never applied, so the target still holds the handed-off block on top of its
    // original queue.
    let held = wait_player(&f.target, |status| {
        status
            .playlist
            .iter()
            .any(|entry| entry.item == f.handed_off)
    })
    .await
    .context("the unapplied rollback leaves the handoff block on the target")?;
    let original_items: Vec<_> = held
        .playlist
        .iter()
        .filter(|entry| entry.item != f.handed_off)
        .map(|entry| entry.item.clone())
        .collect();
    assert!(
        !original_items.is_empty(),
        "the target should have its own queue before the handoff"
    );
    let held_items: Vec<_> = held
        .playlist
        .iter()
        .map(|entry| entry.item.clone())
        .collect();

    // Connectivity returns, and the reconciling take-back is refused with a non-transport
    // failure: the connection stays open and answers, but the block stays unresolved. The retry
    // is immediate: no other connection reconciles first.
    let reconcile_refused = f.install_take_back_refusal().await;
    f.target
        .manager
        .clone()
        .start(
            f.target_address,
            DiscoveryParams {
                device_info: info(&f.target).await?,
            },
            true,
        )
        .await?;
    fs::write(&f.target_file, &f.valid_audio)?;
    let retry = HandoffRequest {
        ticket: None,
        call_id: None,
        target: Some(f.target_fingerprint.clone()),
    };
    let result = timeout(
        TEST_DEADLINE,
        dispatch_gui_request(&f.source.global_params, &retry),
    )
    .await
    .context("the refused-reconcile retry did not finish")?;
    timeout(TEST_DEADLINE, reconcile_refused.notified())
        .await
        .context("the retry should reconcile the unresolved block before its load")?;
    assert!(
        result.as_ref().is_err(),
        "a handoff onto a renderer with an unresolved rollback must fail: {result:?}"
    );

    // No new load reached the target: the queue is exactly what the failed rollback left.
    let after_refusal = f.target.global_params.player.lock().await.get_status();
    let after_items: Vec<_> = after_refusal
        .playlist
        .iter()
        .map(|entry| entry.item.clone())
        .collect();
    assert_eq!(
        after_items, held_items,
        "the target must receive no new load while the old block is unresolved"
    );
    assert!(
        original_items
            .iter()
            .all(|item| after_items.contains(item)),
        "the target's original queue must remain intact: {after_items:?}"
    );

    // With the target answering normally again, the next connection reconciles the block and
    // the retried handoff goes through end to end.
    let retry = HandoffRequest {
        ticket: None,
        call_id: None,
        target: Some(f.target_fingerprint.clone()),
    };
    timeout(
        TEST_DEADLINE,
        dispatch_gui_request(&f.source.global_params, &retry),
    )
    .await
    .context("the reconciled retry did not finish")?
    .context("the reconciled retry should succeed")?;
    let target_after_retry = wait_player(&f.target, |status| {
        status.item == Some(f.handed_off.clone()) && status.state == PlayerPlaybackState::Playing
    })
    .await
    .context("the reconciled retry should play on the target")?;
    let handed_off_count = target_after_retry
        .playlist
        .iter()
        .filter(|entry| entry.item == f.handed_off)
        .count();
    assert_eq!(
        handed_off_count, 1,
        "the retry must not duplicate the handed-off track on the target's queue: {:?}",
        target_after_retry.playlist
    );
    assert!(
        original_items
            .iter()
            .all(|item| target_after_retry.playlist.iter().any(|entry| &entry.item == item)),
        "the target's original queue must survive the retry: {:?}",
        target_after_retry.playlist
    );
    wait_player(&f.source, |status| status.state == PlayerPlaybackState::Paused)
        .await
        .context("only the target should play after the reconciled retry")?;
    f.shutdown().await
}
