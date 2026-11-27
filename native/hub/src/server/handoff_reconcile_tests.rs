/// Source handing off to a target whose copy of the track does not decode, with the target's
/// own track playing. Every connection the source makes to the target goes through a TLS
/// man-in-the-middle that presents the target's own certificate, so pairing, trust, and the
/// websocket protocol are exactly production. The proxy is the fault injection point: armed
/// with a cut kind, it turns the first take-back request frame into a transport failure whose
/// timing is decided by content, not by sleeps. `cut` also stops the target's original queue
/// from being replaced, because every cut leaves the rollback outcome unknown.
struct HandoffReconcileFixture {
    _dirs: Vec<TempDir>,
    source: Fixture,
    target: Fixture,
    host: Fixture,
    _target_replica: Fixture,
    target_fingerprint: String,
    target_file: PathBuf,
    valid_audio: Vec<u8>,
    handed_off: PlayingItem,
    expected_item: PlayingItem,
    cut: mitm::MitmCut,
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

    // The target's own certificate terminates TLS at the proxy: the pinned-certificate trust
    // path the source uses is byte-for-byte production.
    let cert_pem = fs::read_to_string(
        Path::new(target.global_params.node.config_path.as_str()).join("certificate.pem"),
    )
    .context("read target certificate")?;
    let key_pem = fs::read_to_string(
        Path::new(target.global_params.node.config_path.as_str()).join("private_key.pem"),
    )
    .context("read target private key")?;
    let cut = mitm::MitmCut::new(target_address, &cert_pem, &key_pem).await?;
    source
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &target_fingerprint,
            vec![format!("127.0.0.1:{}", cut.listen_addr.port())],
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
        target_file,
        valid_audio,
        handed_off,
        expected_item,
        cut,
    })
}

impl HandoffReconcileFixture {
    /// The handed-off track is on the target's queue: the handoff load applied there.
    async fn wait_block_held(&self) -> Result<playback::player::PlayerStatus> {
        wait_player(&self.target, |status| {
            status
                .playlist
                .iter()
                .any(|entry| entry.item == self.handed_off)
        })
        .await
        .context("the target should hold the handed-off block")
    }

    /// The handed-off track left the target's queue: the take-back applied there.
    async fn wait_block_taken_back(&self) -> Result<playback::player::PlayerStatus> {
        wait_player(&self.target, |status| {
            !status
                .playlist
                .iter()
                .any(|entry| entry.item == self.handed_off)
        })
        .await
        .context("the target should have taken the block back")
    }

    async fn shutdown(&self) -> Result<()> {
        self.source.global_params.node.shutdown().await?;
        self.target.global_params.node.shutdown().await?;
        self.host.global_params.node.shutdown().await?;
        Ok(())
    }
}

/// Runs one handoff whose rollback is cut the way `kind` says, and returns the dispatch
/// result. The cut fires exactly when the proxy sees the take-back frame; the handoff's own
/// outcome then proves whether the source could learn what happened.
async fn run_cut_handoff(f: &HandoffReconcileFixture, kind: mitm::CutKind) -> Result<Result<()>> {
    use crate::messages::HandoffRequest;

    f.cut.arm(kind);
    let source_params = f.source.global_params.clone();
    let handoff = HandoffRequest {
        ticket: None,
        call_id: None,
        target: Some(f.target_fingerprint.clone()),
    };
    let handoff_task =
        tokio::spawn(async move { dispatch_gui_request(&source_params, &handoff).await });
    timeout(TEST_DEADLINE, f.cut.wait_fired())
        .await
        .context("the decode failure should send a take-back through the proxy")?;
    Ok(handoff_task
        .await
        .context("join cut-rollback handoff")?
        .map(|_| ()))
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
        let status = f.wait_block_held().await?;
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
        f.wait_block_taken_back().await?
    };
    wait_player(&f.source, |status| {
        status.state == PlayerPlaybackState::Playing && status.item == Some(f.expected_item.clone())
    })
    .await
    .context("the source resumes after the failed handoff")?;

    // The server never went down: only the cut connection died, so the retried handoff's fresh
    // connection reconciles the pending block.
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
    let f = handoff_reconcile_fixture("handoff-rollback-cut").await?;
    // The cut swallows the take-back before it reaches the target: the target keeps the block,
    // and whether it does is exactly what the source cannot know.
    let result = run_cut_handoff(&f, mitm::CutKind::BeforeApply).await?;
    assert!(
        result.as_ref().is_err(),
        "a handoff whose rollback loses its connection must fail: {result:?}"
    );
    assert_retry_reconciled(&f, true).await?;
    f.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn handoff_rollback_answer_loss_reconciles_on_reconnect() -> Result<()> {
    let f = handoff_reconcile_fixture("handoff-rollback-answer-cut").await?;
    // The cut passes the take-back whole and then holds the response direction: the target
    // applies it, and the answer can never pass, because it cannot exist before the request
    // did. The handoff runs while the connection is still up, so the source waits inside its
    // rollback; killing the connection is what ends the wait with the outcome unknown.
    f.cut.arm(mitm::CutKind::AfterApply);
    let source_params = f.source.global_params.clone();
    let handoff = crate::messages::HandoffRequest {
        ticket: None,
        call_id: None,
        target: Some(f.target_fingerprint.clone()),
    };
    let handoff_task =
        tokio::spawn(async move { dispatch_gui_request(&source_params, &handoff).await });
    timeout(TEST_DEADLINE, f.cut.wait_fired())
        .await
        .context("the decode failure should send a take-back through the proxy")?;
    // Proof by server-side state, not by timing: the block is gone, so the take-back applied.
    f.wait_block_taken_back().await?;
    f.cut.kill_fired_connection();
    let result = handoff_task
        .await
        .context("join answer-cut handoff")?
        .map(|_| ());
    assert!(
        result.as_ref().is_err(),
        "a handoff whose rollback answer is lost must fail: {result:?}"
    );
    assert_retry_reconciled(&f, false).await?;
    f.shutdown().await
}

/// A reconcile attempt whose answer never arrives must end, not retry the dead request inside
/// the same connection. The reconnect's own take-back is cut before it reaches the target, so
/// the reconcile fails with a transport error: the connection must still finish, the block must
/// stay pending, and a handoff attempted onto the target must fail rather than load on top of
/// the unresolved block. The next connection reconciles it and the handoff can be retried end
/// to end.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn handoff_rollback_failed_reconcile_retries_on_next_connection() -> Result<()> {
    use crate::messages::HandoffRequest;

    let f = handoff_reconcile_fixture("handoff-rollback-failed-reconcile").await?;
    // The rollback never applied: the target keeps the block, and the source records its
    // outcome as unknown.
    let result = run_cut_handoff(&f, mitm::CutKind::BeforeApply).await?;
    assert!(
        result.as_ref().is_err(),
        "a handoff whose rollback loses its connection must fail: {result:?}"
    );

    // The rollback never applied, so the target still holds the handed-off block on top of its
    // original queue.
    let held = f.wait_block_held().await?;
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

    // The reconnect's reconcile is itself cut in flight: the connection carrying it must
    // finish anyway, because retrying the dead request inside the same connection would never
    // end. Production keeps the block pending and never fails the connection for it.
    f.cut.arm(mitm::CutKind::BeforeApply);
    let connect = timeout(
        TEST_DEADLINE,
        f.source
            .global_params
            .node
            .connect_renderer(&f.target_fingerprint, RendererLinkUse::Cast),
    )
    .await
    .context("a failed reconcile must not hold its connection open")?;
    timeout(TEST_DEADLINE, f.cut.wait_fired())
        .await
        .context("the reconnecting reconcile should send its take-back through the proxy")?;
    assert!(
        connect.is_ok(),
        "a failed reconcile must not fail its connection: {}",
        connect.as_ref().err().map_or("ok".into(), |e| format!("{e:#}"))
    );
    assert!(
        f.source
            .global_params
            .node
            .has_pending_handoff_rollbacks(&f.target_fingerprint)
            .await,
        "the failed reconcile must keep the block pending"
    );

    // The block is still unresolved, so a handoff onto the target must not load: it fails with
    // the typed unresolved-rollback error, and the target's queue is exactly what the failed
    // rollback left.
    fs::write(&f.target_file, &f.valid_audio)?;
    f.cut.arm(mitm::CutKind::BeforeApply);
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
    .context("the pending-rollback retry did not finish")?;
    timeout(TEST_DEADLINE, f.cut.wait_fired())
        .await
        .context("the retry's reconcile should send its take-back through the proxy")?;
    let error = result.expect_err("a handoff onto a renderer with an unresolved rollback must fail");
    assert!(
        format!("{error:#}").contains("unresolved handoff rollback"),
        "the retry must fail on the unresolved rollback: {error:#}"
    );
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

    // With the proxy no longer armed, the next connection reconciles the block and the retried
    // handoff goes through end to end.
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
    assert!(
        !f.source
            .global_params
            .node
            .has_pending_handoff_rollbacks(&f.target_fingerprint)
            .await,
        "the reconciling connection must clear the pending block"
    );
    assert_retry_reconciled(&f, false).await?;
    f.shutdown().await
}
