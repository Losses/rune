use crate::macros::non_local_request_names;
mod access_policy_tests {
    use super::*;
    use super::player_session_tests::replicate_host_library_at;
    use crate::access_policy::{ConnectionLevel, DeviceGrants, SyncLevel};
    use crate::backends::remote::connect_remote;
    use crate::macros::{
        RequestAccess, request_access, request_scope,
    };
    use crate::messages::{
        DeviceGrantsChange, DeviceGrantsInfo, RemoveTrustedClientRequest,
    };
    use crate::request_error::ExpectedRequestError;
    use crate::messages::{CastLibraryState, RendererAvailability};
    use crate::server::libraries::{
        cast_library_state, list_peer_libraries, list_renderer_candidates,
    };
    use crate::server::sync::client::LibrarySession;
    use crate::server::sync_worker::TestPeerSession;
    use database::actions::stats::{get_liked, set_liked};
    use database::entities::media_cover_art;
    use database::library_registry::{CatalogPlacement, FsAdapter, ProtocolAdapter, RuneForwardMode};
    use http_request::sync::MetadataPeer;
    use sea_orm::{
        ActiveValue,
        sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions},
    };
    use std::fs;
    use sync::protocol::SyncError;
    use tempfile::TempDir;
    use tokio::{
        task::yield_now,
        time::{sleep, timeout},
    };

    struct PolicyPair {
        _first: TempDir,
        _second: TempDir,
        a: Fixture,
        b: Fixture,
        addr: SocketAddr,
        b_addr: SocketAddr,
        client: reqwest::Client,
        id: Uuid,
        owner_session: TestPeerSession,
        replica_session: TestPeerSession,
    }

    fn mark_hosted(record: &mut LibraryRecord, fingerprint: String) -> Result<()> {
        anyhow::ensure!(record.library_id.is_some(), "record is not a catalog");
        let local_path = record.local_folder_path().map(str::to_owned);
        let mode = match &record.protocol {
            ProtocolAdapter::RuneForward { mode: RuneForwardMode::Replica { access_level, connection_access }, .. } => RuneForwardMode::Replica { access_level: *access_level, connection_access: *connection_access },
            _ => RuneForwardMode::Replica { access_level: None, connection_access: None },
        };
        record.protocol = ProtocolAdapter::RuneForward { owner_fingerprint: fingerprint.clone(), mode };
        record.fs = local_path.into_iter().map(|path| FsAdapter::Local { path }).chain([FsAdapter::RunePeer { fingerprint }]).collect();
        if record.catalog == CatalogPlacement::None { record.catalog = CatalogPlacement::ConfigReplica; }
        Ok(())
    }

    fn set_grants(libraries: Vec<DeviceLibraryGrant>) -> DeviceGrantsChange {
        DeviceGrantsChange::Set(DeviceGrantsInfo {
            control: false,
            all_connection: None,
            all_sync: None,
            libraries,
            administer: false,
        })
    }

    impl PolicyPair {
        async fn new_replica() -> Result<Self> {
            let first = tempfile::tempdir()?;
            let second = tempfile::tempdir()?;
            let id = Uuid::new_v4();
            let a = fixture_with_library(first.path(), id).await?;
            let peer = fixture(second.path()).await?;
            let addr = paired_full(&a, &peer).await?;
            let (_, b) = replicate_host_library_at(&a, &peer, addr).await?;
            a.stop_worker().await?;
            peer.stop_worker().await?;
            let client = Self::client_for(&a, &b, addr).await?;
            Ok(Self {
                _first: first,
                _second: second,
                a,
                b,
                addr,
                b_addr: addr,
                client,
                id,
                owner_session: TestPeerSession::default(),
                replica_session: TestPeerSession::default(),
            })
        }

        async fn new() -> Result<Self> {
            let first = tempfile::tempdir()?;
            let second = tempfile::tempdir()?;
            let id = Uuid::new_v4();
            let a = fixture_with_library(first.path(), id).await?;
            let b = fixture_with_library(second.path(), id).await?;
            let addr = paired_full(&a, &b).await?;
            let b_addr = paired_full(&b, &a).await?;
            a.stop_worker().await?;
            b.stop_worker().await?;
            let client = Self::client_for(&a, &b, addr).await?;
            Ok(Self {
                _first: first,
                _second: second,
                a,
                b,
                addr,
                b_addr,
                client,
                id,
                owner_session: TestPeerSession::default(),
                replica_session: TestPeerSession::default(),
            })
        }
        async fn client_for(a: &Fixture, b: &Fixture, addr: SocketAddr) -> Result<reqwest::Client> {
            let peer_fingerprint = info(a).await?.fingerprint;
            sync_worker::invalidate_sync_token(
                &b.global_params.node,
                &peer_fingerprint,
                "127.0.0.1",
                addr.port(),
            );
            let (_, client) = authenticated_client(
                &b.global_params.node,
                &info(b).await?.fingerprint,
                &b.private_key,
                &peer_fingerprint,
                "127.0.0.1",
                addr.port(),
            )
            .await?;
            Ok(client)
        }
        async fn grants(&self, level: DeviceSyncLevel) -> Result<()> {
            self.grants_for(&self.a, &self.b, level).await
        }
        async fn grants_for(
            &self,
            local: &Fixture,
            peer: &Fixture,
            level: DeviceSyncLevel,
        ) -> Result<()> {
            let request = SetDeviceGrantsRequest {
            call_id: None,
                fingerprint: info(peer).await?.fingerprint,
                grants: set_grants(vec![DeviceLibraryGrant {
                        library_id: self.id.to_string(),
                        connection: match level {
                            DeviceSyncLevel::None => DeviceConnectionLevel::None,
                            DeviceSyncLevel::Pull => DeviceConnectionLevel::View,
                            DeviceSyncLevel::PullPush => DeviceConnectionLevel::ViewEdit,
                        },
                        sync: level,
                    }]),
            };
            request
                .handle(request.extract_params(&local.global_params), None, &request)
                .await?;
            Ok(())
        }
        async fn pass(&self, owner: bool) -> Result<PeerPass> {
            self.pass_with_timeout(owner, Duration::from_secs(15)).await
        }
        async fn pass_with_timeout(&self, owner: bool, limit: Duration) -> Result<PeerPass> {
            let (local, remote, addr) = if owner {
                (&self.a, &self.b, self.b_addr)
            } else {
                (&self.b, &self.a, self.addr)
            };
            let session = if owner {
                &self.owner_session
            } else {
                &self.replica_session
            };
            let endpoint = PeerEndpoint {
                fingerprint: info(remote).await?.fingerprint,
                trusted_host: "127.0.0.1".into(),
                host: "127.0.0.1".into(),
                port: addr.port(),
            };
            timeout(limit, async {
                loop {
                    match session
                        .pass(
                            local.global_params.clone(),
                            &local.private_key,
                            endpoint.clone(),
                            true,
                        )
                        .await?
                    {
                        PeerPass::Incomplete => yield_now().await,
                        result => return Ok::<_, anyhow::Error>(result),
                    }
                }
            })
            .await?
        }
        async fn listings(&self) -> Result<serde_json::Value> {
            Ok(self
                .client
                .get(format!("https://{}/libraries", self.addr))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?)
        }
        async fn close(self) -> Result<()> {
            self.a.global_params.node.shutdown().await?;
            self.b.global_params.node.shutdown().await
        }
    }

    async fn lock_replica_library(fixture: &Fixture) -> Result<SqliteConnection> {
        let library = fixture.global_params.test_library();
        let path = library
            .main_db
            .get_sqlite_connection_pool()
            .connect_options()
            .get_filename()
            .to_path_buf();
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(false)
            .busy_timeout(Duration::ZERO);
        let mut connection = SqliteConnection::connect_with(&options).await?;
        sea_orm::sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut connection)
            .await?;
        let mut contender = SqliteConnection::connect_with(&options).await?;
        let error = sea_orm::sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut contender)
            .await
            .expect_err("second writer must be blocked by the held immediate transaction");
        anyhow::ensure!(
            error
                .as_database_error()
                .and_then(|error| error.code())
                .is_some_and(|code| code == "5" || code == "6"),
            "second writer failed for a reason other than SQLite busy/locked: {error}"
        );
        Ok(connection)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn sync_recovers_after_replica_write_lock_is_released() -> Result<()> {
        let pair = PolicyPair::new_replica().await?;
        let host = pair.a.global_params.test_library();
        let key = write_album(host, "busy recovery").await?;
        let mut lock = lock_replica_library(&pair.b).await?;
        let pass = tokio::spawn(async move {
            let result = pair.pass_with_timeout(false, Duration::from_secs(30)).await;
            (result, pair)
        });

        // The default SQLite busy timeout is five seconds. The first metadata pass also
        // performs authenticated HTTP setup before reaching its write; release just after
        // that first busy result so the retry can complete.
        sleep(Duration::from_millis(11_250)).await;
        sea_orm::sqlx::query("ROLLBACK")
            .execute(&mut lock)
            .await?;
        let (result, pair) = pass.await?;
        assert_eq!(result?, PeerPass::Complete);
        let replica = pair.b.global_params.test_library();
        assert!(replica.library_sync.get(&key).await?.is_some());
        pair.close().await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn sync_reports_storage_busy_after_retry_cap() -> Result<()> {
        let pair = PolicyPair::new_replica().await?;
        let host = pair.a.global_params.test_library();
        write_album(host, "busy cap").await?;
        let mut lock = lock_replica_library(&pair.b).await?;

        let result = pair
            .pass_with_timeout(false, Duration::from_secs(45))
            .await;
        sea_orm::sqlx::query("ROLLBACK")
            .execute(&mut lock)
            .await?;
        let error = result.expect_err("held SQLite lock must exhaust the retry cap");
        assert_eq!(error.downcast_ref::<SyncError>(), Some(&SyncError::StorageBusy));
        pair.close().await
    }

    /// Real files in the library root and the cover cache, so a 403 means the
    /// policy refused, not that the file handler found nothing to serve.
    async fn served_files(pair: &PolicyPair, name: &str) -> Result<[String; 2]> {
        let library = pair.a.global_params.library_pool.acquire(pair.id).await?;
        fs::write(Path::new(&library.media_root).join(name), b"audio")?;
        let cover_hash = blake3::hash(b"cover").to_hex().to_string();
        media_cover_art::Entity::insert(media_cover_art::ActiveModel {
            file_hash: ActiveValue::Set(cover_hash.clone()),
            binary: ActiveValue::Set(b"cover".to_vec()),
            content_hash: ActiveValue::Set(Some(cover_hash.clone())),
            content_size: ActiveValue::Set(Some(5)),
            local_available: ActiveValue::Set(true),
            hlc_uuid: ActiveValue::Set(Uuid::new_v4().to_string()),
            ..Default::default()
        })
        .exec(&*library.main_db)
        .await?;
        fs::create_dir_all(&*COVER_TEMP_DIR)?;
        fs::write(COVER_TEMP_DIR.join(&cover_hash), b"cover")?;
        Ok([
            format!("files/library/{name}"),
            format!("files/cache/{cover_hash}?library_id={}", pair.id),
        ])
    }

    async fn status(pair: &PolicyPair, path: &str) -> Result<reqwest::StatusCode> {
        Ok(pair
            .client
            .get(format!("https://{}/{path}", pair.addr))
            .send()
            .await?
            .status())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn access_policy_migration() -> Result<()> {
        let mut pair = PolicyPair::new().await?;
        let fingerprint = info(&pair.b).await?.fingerprint;
        let config = Path::new(&*pair.a.global_params.config_path).to_path_buf();
        fs::remove_file(config.join("grants.json"))?;
        pair.a = reopen_fixture(&pair.a).await?;
        assert_eq!(
            pair.a
                .global_params
                .node
                .access_policy
                .grants(&fingerprint)?,
            DeviceGrants::my_device()
        );
        pair.addr = paired_full(&pair.a, &pair.b).await?;
        pair.a.stop_worker().await?;
        pair.client = PolicyPair::client_for(&pair.a, &pair.b, pair.addr).await?;
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(config.join("grants.json"))?)?;
        assert_eq!(json["version"], 2);
        assert_eq!(
            json["devices"][&fingerprint],
            serde_json::to_value(DeviceGrants::my_device())?
        );
        let mut socket = connect_remote(
            "127.0.0.1",
            pair.addr.port(),
            &pair.b.global_params.config_path,
            pair.b.global_params.cert_validator.read().await.clone(),
            &pair.b.global_params.node.sync_tokens,
        )
        .await?;
        let (name, _) = socket_request(
            &mut socket,
            "PauseRequest",
            &rinf::serialize(&PauseRequest {
            call_id: None,})?,
        )
        .await?;
        assert_eq!(name, "");
        let left = pair.a.global_params.library_pool.acquire(pair.id).await?;
        let right = pair.b.global_params.library_pool.acquire(pair.id).await?;
        let forward = write_album(&left, "migration forward").await?;
        let reverse = write_album(&right, "migration reverse").await?;
        assert_eq!(pair.pass(false).await?, PeerPass::Complete);
        assert!(right.library_sync.get(&forward).await?.is_some());
        assert!(left.library_sync.get(&reverse).await?.is_some());
        fs::write(
            Path::new(&pair.a.global_params.test_library().media_root).join("migration.txt"),
            b"migration",
        )?;
        let response = pair
            .client
            .get(format!("https://{}/files/library/migration.txt", pair.addr))
            .send()
            .await?;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.bytes().await?.as_ref(), b"migration");
        socket.close(None).await?;
        pair.close().await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn stale_peer_token_recovers_across_library_requests() -> Result<()> {
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let id = Uuid::new_v4();
        let a = fixture_with_library(first.path(), id).await?;
        let b = fixture_with_library(second.path(), id).await?;
        let a_addr = paired_full(&a, &b).await?;
        let b_addr = paired_full(&b, &a).await?;
        announce_peer(&b, &info(&a).await?, a_addr).await?;
        announce_peer(&a, &info(&b).await?, b_addr).await?;
        a.stop_worker().await?;
        b.stop_worker().await?;
        let a_fingerprint = info(&a).await?.fingerprint;
        let b_fingerprint = info(&b).await?.fingerprint;
        let store = &b.global_params.node.sync_tokens;
        let base = sync_worker::peer_base("127.0.0.1", a_addr.port());

        store.insert_stale_for_test(&a_fingerprint, &base);
        let candidates = list_renderer_candidates(b.global_params.node.clone(), vec![id]).await?;
        assert!(candidates.iter().any(|candidate| {
            candidate.fingerprint == a_fingerprint
                && candidate.availability == RendererAvailability::Available
        }));

        LibraryRegistry::update(Path::new(&*b.global_params.config_path), |registry| {
            let library = registry
                .libraries
                .iter_mut()
                .find(|library| library.catalog_library_id() == Some(id))
                .context("replica library")?;
            mark_hosted(library, a_fingerprint.clone())?;
            Ok(())
        })
        .await?;
        store.insert_stale_for_test(&a_fingerprint, &base);
        assert_eq!(
            cast_library_state(&b.global_params.node, id).await,
            CastLibraryState::Ready
        );

        store.insert_stale_for_test(&a_fingerprint, &base);
        let listed =
            list_peer_libraries(&b.global_params.node, &b.private_key, &a_fingerprint).await?;
        assert!(
            listed
                .iter()
                .any(|library| library.library_id == id.to_string())
        );

        a.global_params
            .permission_manager
            .write()
            .await
            .change_user_status(&b_fingerprint, UserStatus::Blocked)
            .await?;
        store.insert_stale_for_test(&a_fingerprint, &base);
        let candidates = list_renderer_candidates(b.global_params.node.clone(), vec![id]).await?;
        assert!(candidates.iter().any(|candidate| {
            candidate.fingerprint == a_fingerprint
                && candidate.availability == RendererAvailability::NotApproved
        }));
        a.global_params.node.shutdown().await?;
        b.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn access_policy_read_only_replica() -> Result<()> {
        let pair = PolicyPair::new().await?;
        pair.grants(DeviceSyncLevel::Pull).await?;
        let owner_fingerprint = info(&pair.a).await?.fingerprint;
        LibraryRegistry::update(Path::new(&*pair.b.global_params.config_path), |registry| {
            let replica = registry
                .libraries
                .iter_mut()
                .find(|library| library.catalog_library_id() == Some(pair.id))
                .context("replica descriptor")?;
            mark_hosted(replica, owner_fingerprint)?;
            Ok(())
        })
        .await?;
        assert_eq!(pair.listings().await?[0]["connection"], "View");
        assert_eq!(pair.listings().await?[0]["sync"], "Pull");
        let original = scan_held_track(&pair.a, pair.id, "readonly.ogg").await?;
        let left = pair.a.global_params.library_pool.acquire(pair.id).await?;
        let right = pair.b.global_params.library_pool.acquire(pair.id).await?;
        let album = write_album(&left, "read-only pull").await?;
        assert_eq!(pair.pass(false).await?, PeerPass::Complete);
        let registry = LibraryRegistry::load(Path::new(&*pair.b.global_params.config_path))?;
        let replica = registry
            .catalog(pair.id)
            .context("replica descriptor")?;
        assert_eq!(
            serde_json::to_value(replica)?["protocol"]["mode"]["access_level"],
            "Read"
        );
        assert_eq!(
            serde_json::to_value(replica)?["protocol"]["mode"]["connection_access"],
            "View"
        );
        assert!(right.library_sync.get(&album).await?.is_some());
        let remote = media_files::Entity::find()
            .filter(Column::HlcUuid.eq(&original.hlc_uuid))
            .one(&*right.main_db)
            .await?
            .context("replicated track")?;
        set_liked(
            &right.main_db,
            &right.library_sync.record_store(),
            remote.id,
            true,
        )
        .await?;
        assert!(get_liked(&*right.main_db, remote.id).await?);
        // A client ignoring the advertised level must be refused by the owner itself.
        let peer = MetadataPeer::<RuneModel>::new(
            format!("https://{}/libraries/{}", pair.addr, pair.id),
            pair.client.clone(),
        );
        let mut session = LibrarySession::new(right.library_sync.clone())?;
        let result = timeout(Duration::from_secs(10), async {
            loop {
                if session.advance(&peer, Arc::new(|| {})).await? {
                    return Ok::<(), anyhow::Error>(());
                }
            }
        })
        .await?;
        let error = result.expect_err("read-only push was accepted");
        assert!(matches!(
            error.downcast_ref::<SyncError>(),
            Some(SyncError::Forbidden)
        ));
        for _ in 0..3 {
            sleep(Duration::from_secs(1)).await;
            assert_eq!(
                pair.pass(false).await?,
                PeerPass::Complete,
                "read-only worker retried a push"
            );
        }
        // The hosted replica is absent from listings. A push 403 must be remembered,
        // and the owner's worker must not pull unauthorized edits around the gate.
        pair.grants_for(&pair.b, &pair.a, DeviceSyncLevel::Pull)
            .await?;
        for _ in 0..3 {
            assert_eq!(pair.pass(true).await?, PeerPass::Complete);
        }
        assert!(!get_liked(&*left.main_db, original.id).await?);
        pair.grants(DeviceSyncLevel::PullPush).await?;
        assert_eq!(pair.pass(false).await?, PeerPass::Complete);
        let registry = LibraryRegistry::load(Path::new(&*pair.b.global_params.config_path))?;
        let replica = registry
            .catalog(pair.id)
            .context("replica descriptor")?;
        assert_eq!(
            serde_json::to_value(replica)?["protocol"]["mode"]["access_level"],
            "ReadWrite"
        );
        pair.close().await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn removing_a_device_stops_both_sync_directions_and_keeps_replica() -> Result<()> {
        let pair = PolicyPair::new().await?;
        let fingerprint_b = info(&pair.b).await?.fingerprint;
        let source = pair.a.global_params.library_pool.acquire(pair.id).await?;
        let album = write_album(&source, "retained after removal").await?;
        assert_eq!(pair.pass(false).await?, PeerPass::Complete);
        let replica = pair.b.global_params.library_pool.acquire(pair.id).await?;
        assert!(replica.library_sync.get(&album).await?.is_some());

        let request = RemoveTrustedClientRequest {
            call_id: None,
            fingerprint: fingerprint_b.clone(),
        };
        let response = request
            .handle(
                request.extract_params(&pair.a.global_params),
                None,
                &request,
            )
            .await?
            .context("remove device response")?;
        assert!(response.success, "{}", response.error);

        assert!(
            pair.a
                .global_params
                .cert_validator
                .read()
                .await
                .get_hosts_for_fingerprint(&fingerprint_b)
                .await
                .is_empty()
        );
        assert!(
            pair.a
                .global_params
                .permission_manager
                .read()
                .await
                .verify_by_fingerprint(&fingerprint_b)
                .await
                .is_none()
        );
        let own = info(&pair.a).await?.fingerprint;
        assert!(
            sync_worker::peer_endpoints(&pair.a.global_params.node, &own)
                .await
                .is_empty()
        );
        assert!(
            pair.pass(false).await.is_err(),
            "removed host accepted a pull"
        );
        assert!(replica.library_sync.get(&album).await?.is_some());
        pair.close().await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn non_authority_changes_require_sync_push_grant() -> Result<()> {
        let pair = PolicyPair::new().await?;
        let third_dir = tempfile::tempdir()?;
        let c = fixture_with_library(third_dir.path(), pair.id).await?;
        let a_fingerprint = info(&pair.a).await?.fingerprint;
        let c_addr = paired_full(&c, &pair.b).await?;
        c.stop_worker().await?;
        let c_fingerprint = info(&c).await?.fingerprint;

        for fixture in [&pair.b, &c] {
            LibraryRegistry::update(Path::new(&*fixture.global_params.config_path), |registry| {
                let library = registry
                    .libraries
                    .iter_mut()
                    .find(|library| library.catalog_library_id() == Some(pair.id))
                    .context("replica library")?;
                mark_hosted(library, a_fingerprint.clone())?;
                Ok(())
            })
            .await?;
        }
        pair.b
            .global_params
            .node
            .access_policy
            .set(&c_fingerprint, DeviceGrants::guest())?;
        let third_library = c.global_params.library_pool.acquire(pair.id).await?;
        let injected = write_album(&third_library, "third party change").await?;
        let replica = pair.b.global_params.library_pool.acquire(pair.id).await?;
        let own_change =
            write_album(&replica, "change kept from a peer without a sync grant").await?;
        let session = TestPeerSession::default();
        let endpoint = PeerEndpoint {
            fingerprint: c_fingerprint.clone(),
            trusted_host: "127.0.0.1".into(),
            host: "127.0.0.1".into(),
            port: c_addr.port(),
        };
        let pass = || async {
            timeout(Duration::from_secs(15), async {
                loop {
                    let result = session
                        .pass(
                            pair.b.global_params.clone(),
                            &pair.b.private_key,
                            endpoint.clone(),
                            true,
                        )
                        .await?;
                    if result != PeerPass::Incomplete {
                        return Ok::<_, anyhow::Error>(result);
                    }
                    yield_now().await;
                }
            })
            .await?
        };
        assert_eq!(pass().await?, PeerPass::Complete);
        assert!(replica.library_sync.get(&injected).await?.is_none());
        // B has not granted C SyncPull either, so B's own change is not sent to C.
        assert!(third_library.library_sync.get(&own_change).await?.is_none());

        pair.grants_for(&pair.b, &c, DeviceSyncLevel::PullPush)
            .await?;
        assert_eq!(pass().await?, PeerPass::Complete);
        assert!(replica.library_sync.get(&injected).await?.is_some());
        assert!(third_library.library_sync.get(&own_change).await?.is_some());

        pair.a.global_params.node.shutdown().await?;
        pair.b.global_params.node.shutdown().await?;
        c.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn access_policy_not_shared() -> Result<()> {
        let pair = PolicyPair::new().await?;
        pair.grants(DeviceSyncLevel::None).await?;
        assert_eq!(pair.listings().await?, serde_json::json!([]));
        let metadata = pair
            .client
            .post(format!(
                "https://{}/libraries/{}/sync/metadata",
                pair.addr, pair.id
            ))
            .json(&serde_json::json!({"operation": "identity"}))
            .send()
            .await?;
        assert_eq!(metadata.status(), reqwest::StatusCode::FORBIDDEN);
        assert_eq!(
            metadata.json::<serde_json::Value>().await?,
            serde_json::json!({"error": "forbidden"})
        );
        let [audio, cover] = served_files(&pair, "not-shared.txt").await?;
        for path in [
            audio.as_str(),
            cover.as_str(),
            "media/metadata/1",
            "media/cover/1",
        ] {
            assert_eq!(
                pair.client
                    .get(format!("https://{}/{path}", pair.addr))
                    .send()
                    .await?
                    .status(),
                reqwest::StatusCode::FORBIDDEN,
                "{path}"
            );
        }
        pair.close().await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn access_policy_guest_browses_but_cannot_pull() -> Result<()> {
        use crate::messages::CreatePlaylistRequest;

        let pair = PolicyPair::new().await?;
        let request = SetDeviceGrantsRequest {
            call_id: None,
            fingerprint: info(&pair.b).await?.fingerprint,
            grants: DeviceGrantsChange::Preset(DevicePreset::Guest),
        };
        request
            .handle(
                request.extract_params(&pair.a.global_params),
                None,
                &request,
            )
            .await?;
        let mut socket = connect_remote(
            "127.0.0.1",
            pair.addr.port(),
            &pair.b.global_params.config_path,
            pair.b.global_params.cert_validator.read().await.clone(),
            &pair.b.global_params.node.sync_tokens,
        )
        .await?;
        let (name, _) = socket_request(
            &mut socket,
            "GetMediaFilesCountRequest",
            &rinf::serialize(&GetMediaFilesCountRequest {
            call_id: None,})?,
        )
        .await?;
        assert_eq!(name, "GetMediaFilesCountResponse");
        let (name, payload) = socket_request(
            &mut socket,
            "CreatePlaylistRequest",
            &rinf::serialize(&CreatePlaylistRequest {
            call_id: None,
                name: "guest denied".into(),
                group: "test".into(),
            })?,
        )
        .await?;
        assert_eq!(name, "RemoteRequestFailed");
        let failure: RemoteRequestFailed = rinf::deserialize(&payload)?;
        assert_eq!(failure.kind, RequestErrorKind::PermissionDenied);
        assert_eq!(pair.listings().await?[0]["connection"], "View");
        assert_eq!(pair.listings().await?[0]["sync"], "None");
        let metadata = pair
            .client
            .post(format!(
                "https://{}/libraries/{}/sync/metadata",
                pair.addr, pair.id
            ))
            .json(&serde_json::json!({"operation": "identity"}))
            .send()
            .await?;
        assert_eq!(metadata.status(), reqwest::StatusCode::FORBIDDEN);
        let source = pair.a.global_params.library_pool.acquire(pair.id).await?;
        let album = write_album(&source, "guest cannot replicate").await?;
        assert_eq!(pair.pass(false).await?, PeerPass::LibraryNotServed);
        let replica = pair.b.global_params.library_pool.acquire(pair.id).await?;
        assert!(replica.library_sync.get(&album).await?.is_none());
        let [audio, cover] = served_files(&pair, "guest-browse.txt").await?;
        assert_eq!(status(&pair, &audio).await?, reqwest::StatusCode::FORBIDDEN);
        assert_eq!(status(&pair, &cover).await?, reqwest::StatusCode::OK);
        let digest = blake3::hash(b"guest blob").to_hex();
        assert_eq!(
            pair.client
                .get(format!(
                    "https://{}/libraries/{}/sync/blobs/{digest}",
                    pair.addr, pair.id
                ))
                .send()
                .await?
                .status(),
            reqwest::StatusCode::FORBIDDEN
        );
        socket.close(None).await?;
        pair.close().await
    }

    #[tokio::test]
    async fn set_device_grants_preset_applies_the_preset_grants() -> Result<()> {
        let pair = PolicyPair::new().await?;
        let fingerprint = info(&pair.b).await?.fingerprint;
        let request = SetDeviceGrantsRequest {
            call_id: None,
            fingerprint: fingerprint.clone(),
            grants: DeviceGrantsChange::Preset(DevicePreset::MyDevice),
        };
        request
            .handle(
                request.extract_params(&pair.a.global_params),
                None,
                &request,
            )
            .await?;
        assert_eq!(
            pair.a
                .global_params
                .node
                .access_policy
                .grants(&fingerprint)?,
            DeviceGrants::my_device()
        );
        pair.close().await
    }

    #[tokio::test]
    async fn set_device_grants_replaces_previous_library_grants() -> Result<()> {
        let pair = PolicyPair::new().await?;
        let fingerprint = info(&pair.b).await?.fingerprint;
        let mut existing = DeviceGrants::guest();
        existing.connection.libraries.insert(pair.id, ConnectionLevel::View);
        existing.sync.libraries.insert(pair.id, SyncLevel::Pull);
        pair.a
            .global_params
            .node
            .access_policy
            .set(&fingerprint, existing)?;

        let request = SetDeviceGrantsRequest {
            call_id: None,
            fingerprint: fingerprint.clone(),
            grants: set_grants(Vec::new()),
        };
        let result = request
            .handle(
                request.extract_params(&pair.a.global_params),
                None,
                &request,
            )
            .await;
        assert!(result.is_ok());
        let actual = pair
            .a
            .global_params
            .node
            .access_policy
            .grants(&fingerprint)?;
        assert_eq!(actual.connection_level(pair.id), ConnectionLevel::None);
        assert_eq!(actual.sync_level(pair.id), SyncLevel::None);
        pair.close().await
    }

    #[tokio::test]
    async fn set_device_grants_returns_unknown_library_as_typed_error() -> Result<()> {
        let pair = PolicyPair::new().await?;
        let request = SetDeviceGrantsRequest {
            call_id: None,
            fingerprint: info(&pair.b).await?.fingerprint,
            grants: set_grants(vec![DeviceLibraryGrant {
                    library_id: Uuid::new_v4().to_string(),
                    connection: DeviceConnectionLevel::View,
                    sync: DeviceSyncLevel::None,
                }]),
        };
        let result = request
            .handle(
                request.extract_params(&pair.a.global_params),
                None,
                &request,
            )
            .await;
        let error = result
            .err()
            .unwrap_or_else(|| panic!("unknown library must fail"));
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::InvalidInput { .. })
        ));
        pair.close().await
    }

    #[tokio::test]
    async fn set_device_grants_returns_unapproved_device_as_typed_error() -> Result<()> {
        let pair = PolicyPair::new().await?;
        let request = SetDeviceGrantsRequest {
            call_id: None,
            fingerprint: "unapproved-device".into(),
            grants: set_grants(Vec::new()),
        };
        let result = request
            .handle(
                request.extract_params(&pair.a.global_params),
                None,
                &request,
            )
            .await;
        let error = result
            .err()
            .unwrap_or_else(|| panic!("unapproved device must fail"));
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::InvalidInput { .. })
        ));
        pair.close().await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn control_without_library_grants_does_not_allow_library_reads() -> Result<()> {
        let pair = PolicyPair::new().await?;
        let fingerprint = info(&pair.b).await?.fingerprint;
        let mut grants = DeviceGrants::guest();
        grants.connection.all_libraries = Some(ConnectionLevel::None);
        pair.a
            .global_params
            .node
            .access_policy
            .set(&fingerprint, grants)?;
        let mut socket = connect_remote(
            "127.0.0.1",
            pair.addr.port(),
            &pair.b.global_params.config_path,
            pair.b.global_params.cert_validator.read().await.clone(),
            &pair.b.global_params.node.sync_tokens,
        )
        .await?;
        let (name, _) = socket_request(
            &mut socket,
            "PauseRequest",
            &rinf::serialize(&PauseRequest {
            call_id: None,})?,
        )
        .await?;
        assert_ne!(name, "RemoteRequestFailed");
        let (name, payload) = socket_request(
            &mut socket,
            "GetMediaFilesCountRequest",
            &rinf::serialize(&GetMediaFilesCountRequest {
            call_id: None,})?,
        )
        .await?;
        assert_eq!(name, "RemoteRequestFailed");
        let failure: RemoteRequestFailed = rinf::deserialize(&payload)?;
        assert_eq!(failure.kind, RequestErrorKind::PermissionDenied);
        socket.close(None).await?;
        pair.close().await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn connection_edit_without_sync_allows_websocket_writes_only() -> Result<()> {
        use crate::messages::CreatePlaylistRequest;

        let pair = PolicyPair::new().await?;
        let fingerprint = info(&pair.b).await?.fingerprint;
        let mut grants = DeviceGrants::guest();
        grants.control = false;
        grants.connection.all_libraries = Some(ConnectionLevel::ViewEdit);
        grants.sync.all_libraries = Some(SyncLevel::None);
        pair.a
            .global_params
            .node
            .access_policy
            .set(&fingerprint, grants)?;
        let mut socket = connect_remote(
            "127.0.0.1",
            pair.addr.port(),
            &pair.b.global_params.config_path,
            pair.b.global_params.cert_validator.read().await.clone(),
            &pair.b.global_params.node.sync_tokens,
        )
        .await?;
        let (name, _) = socket_request(
            &mut socket,
            "CreatePlaylistRequest",
            &rinf::serialize(&CreatePlaylistRequest {
            call_id: None,
                name: "axis edit".into(),
                group: "test".into(),
            })?,
        )
        .await?;
        assert_eq!(name, "CreatePlaylistResponse");
        assert_eq!(
            pair.client
                .post(format!(
                    "https://{}/libraries/{}/sync/metadata",
                    pair.addr, pair.id
                ))
                .json(&serde_json::json!({"operation": "identity"}))
                .send()
                .await?
                .status(),
            reqwest::StatusCode::FORBIDDEN
        );
        socket.close(None).await?;
        pair.close().await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn sync_pull_without_connection_refuses_websocket_reads() -> Result<()> {
        let pair = PolicyPair::new().await?;
        let fingerprint = info(&pair.b).await?.fingerprint;
        let mut grants = DeviceGrants::guest();
        grants.control = false;
        grants.connection.all_libraries = Some(ConnectionLevel::None);
        grants.sync.all_libraries = Some(SyncLevel::Pull);
        pair.a
            .global_params
            .node
            .access_policy
            .set(&fingerprint, grants)?;
        assert_eq!(pair.listings().await?[0]["connection"], "None");
        assert_eq!(pair.listings().await?[0]["sync"], "Pull");
        let source = pair.a.global_params.library_pool.acquire(pair.id).await?;
        let album = write_album(&source, "pull only").await?;
        assert_eq!(pair.pass(false).await?, PeerPass::Complete);
        let replica = pair.b.global_params.library_pool.acquire(pair.id).await?;
        assert!(replica.library_sync.get(&album).await?.is_some());
        let mut socket = connect_remote(
            "127.0.0.1",
            pair.addr.port(),
            &pair.b.global_params.config_path,
            pair.b.global_params.cert_validator.read().await.clone(),
            &pair.b.global_params.node.sync_tokens,
        )
        .await?;
        let (name, payload) = socket_request(
            &mut socket,
            "GetMediaFilesCountRequest",
            &rinf::serialize(&GetMediaFilesCountRequest {
            call_id: None,})?,
        )
        .await?;
        assert_eq!(name, "RemoteRequestFailed");
        let failure: RemoteRequestFailed = rinf::deserialize(&payload)?;
        assert_eq!(failure.kind, RequestErrorKind::PermissionDenied);
        socket.close(None).await?;
        pair.close().await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cover_cache_requires_the_addressed_library_to_own_the_cover() -> Result<()> {
        let pair = PolicyPair::new().await?;
        pair.grants(DeviceSyncLevel::Pull).await?;
        let name = blake3::hash(b"cover from another library")
            .to_hex()
            .to_string();
        fs::create_dir_all(&*COVER_TEMP_DIR)?;
        fs::write(COVER_TEMP_DIR.join(&name), b"cover from another library")?;

        let from_other_library = format!("files/cache/{name}?library_id={}", Uuid::new_v4());
        let from_allowed_library = format!("files/cache/{name}?library_id={}", pair.id);
        for path in [&from_other_library, &from_allowed_library] {
            let response = pair
                .client
                .get(format!("https://{}/{path}", pair.addr))
                .send()
                .await?;
            assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
            assert_ne!(
                response.bytes().await?.as_ref(),
                b"cover from another library"
            );
        }

        let cover = served_files(&pair, "same-library-audio.txt").await?[1].clone();
        let response = pair
            .client
            .get(format!("https://{}/{cover}", pair.addr))
            .send()
            .await?;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.bytes().await?.as_ref(), b"cover");
        pair.close().await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn access_policy_live_revocation() -> Result<()> {
        let pair = PolicyPair::new().await?;
        let mut socket = connect_remote(
            "127.0.0.1",
            pair.addr.port(),
            &pair.b.global_params.config_path,
            pair.b.global_params.cert_validator.read().await.clone(),
            &pair.b.global_params.node.sync_tokens,
        )
        .await?;
        // Hold every connection while the metadata handler's head query waits on the pool.
        let library = pair.a.global_params.library_pool.acquire(pair.id).await?;
        let pool = library.main_db.get_sqlite_connection_pool();
        let mut connections = Vec::new();
        for _ in 0..pool.options().get_max_connections() {
            connections.push(pool.acquire().await?);
        }
        let request = pair
            .client
            .post(format!(
                "https://{}/libraries/{}/sync/metadata",
                pair.addr, pair.id
            ))
            .json(&serde_json::json!({"operation": "head"}))
            .send();
        tokio::pin!(request);
        tokio::select! {
            response = &mut request => panic!("sync did not remain in flight: {:?}", response.map(|r| r.status())),
            _ = sleep(Duration::from_millis(200)) => {},
        }
        pair.grants(DeviceSyncLevel::None).await?;
        let (name, payload) = socket_request(
            &mut socket,
            "GetMediaFilesCountRequest",
            &rinf::serialize(&GetMediaFilesCountRequest {
            call_id: None,})?,
        )
        .await?;
        assert_eq!(name, "RemoteRequestFailed");
        let failure: RemoteRequestFailed = rinf::deserialize(&payload)?;
        assert_eq!(failure.kind, RequestErrorKind::PermissionDenied);
        assert_eq!(
            timeout(Duration::from_secs(2), request).await??.status(),
            reqwest::StatusCode::FORBIDDEN
        );
        drop(connections);
        pair.close().await
    }

    fn old_request_access(name: &str) -> Option<RequestAccess> {
        Some(match name {
            "AttachRendererRequest" => RequestAccess::NodeLocal,
            "DetachRendererRequest" => RequestAccess::NodeLocal,
            "ListRenderersRequest" => RequestAccess::NodeLocal,
            "HandoffRequest" => RequestAccess::NodeLocal,
            "GetDeviceGrantsRequest" => RequestAccess::NodeLocal,
            "SetDeviceGrantsRequest" => RequestAccess::NodeLocal,
            "ListControllersRequest" => RequestAccess::NodeLocal,
            "DetachControllerRequest" => RequestAccess::NodeLocal,
            "ListLocalLibrariesRequest" => RequestAccess::NodeLocal,
            "ListLibraryRecordsRequest" => RequestAccess::NodeLocal,
            "RemoveLibraryRecordRequest" => RequestAccess::NodeLocal,
            "SetLibraryBackgroundSyncRequest" => RequestAccess::NodeLocal,
            "RegisterLocalLibraryRequest" => RequestAccess::NodeLocal,
            "RegisterRemoteControlRequest" => RequestAccess::NodeLocal,
            "RegisterSubsonicLibraryRequest" => RequestAccess::NodeLocal,
            "ReplaceSubsonicCredentialRequest" => RequestAccess::NodeLocal,
            "RegisterOpenDalLibraryRequest" => RequestAccess::NodeLocal,
            "OpenLibraryRequest" => RequestAccess::NodeLocal,
            "InitializeNodeRequest" => RequestAccess::NodeLocal,
            "RunFsSelfTestRequest" => RequestAccess::NodeLocal,
            "ListPeerLibrariesRequest" => RequestAccess::NodeLocal,
            "AddRemoteLibraryRequest" => RequestAccess::NodeLocal,
            "GetSyncSettingsRequest" => RequestAccess::NodeLocal,
            "SetSyncSettingsRequest" => RequestAccess::NodeLocal,
            "SetDownloadRuleRequest" => RequestAccess::NodeLocal,
            "RemoveDownloadRuleRequest" => RequestAccess::NodeLocal,
            "TestLibraryInitializedRequest" => RequestAccess::NodeLocal,
            "CloseLibraryRequest" => RequestAccess::NodeLocal,
            "CancelTaskRequest" => RequestAccess::Administer,
            "ScanAudioLibraryRequest" => RequestAccess::Administer,
            "AnalyzeAudioLibraryRequest" => RequestAccess::Administer,
            "DeduplicateAudioLibraryRequest" => RequestAccess::Administer,
            "VolumeRequest" => RequestAccess::Control,
            "LoadRequest" => RequestAccess::Control,
            "PlayRequest" => RequestAccess::Control,
            "PauseRequest" => RequestAccess::Control,
            "NextRequest" => RequestAccess::Control,
            "PreviousRequest" => RequestAccess::Control,
            "SwitchRequest" => RequestAccess::Control,
            "SeekRequest" => RequestAccess::Control,
            "RemoveRequest" => RequestAccess::Control,
            "SetPlaybackModeRequest" => RequestAccess::Control,
            "MovePlaylistItemRequest" => RequestAccess::Control,
            "SetRealtimeFFTEnabledRequest" => RequestAccess::Control,
            "SetAdaptiveSwitchingEnabledRequest" => RequestAccess::Control,
            "SetMixWithOthersRequest" => RequestAccess::NodeLocal,
            "StartVisualizerRequest" => RequestAccess::NodeLocal,
            "StopVisualizerRequest" => RequestAccess::NodeLocal,
            "ResizeVisualizerRequest" => RequestAccess::NodeLocal,
            "SetVisualizerPresetDirsRequest" => RequestAccess::NodeLocal,
            "SetVisualizerAccentRequest" => RequestAccess::NodeLocal,
            "SetVisualizerVisibleRequest" => RequestAccess::NodeLocal,
            "LoadVisualizerPresetRequest" => RequestAccess::NodeLocal,
            "NextVisualizerPresetRequest" => RequestAccess::NodeLocal,
            "PreviousVisualizerPresetRequest" => RequestAccess::NodeLocal,
            "RandomVisualizerPresetRequest" => RequestAccess::NodeLocal,
            "ImportVisualizerSceneRequest" => RequestAccess::NodeLocal,
            "RemoveVisualizerSceneRequest" => RequestAccess::NodeLocal,
            "SetVisualizerScenesRequest" => RequestAccess::NodeLocal,
            "SetVisualizerLogRequest" => RequestAccess::NodeLocal,
            "SetVisualizerLabRequest" => RequestAccess::NodeLocal,
            "VisualizerLabCommandRequest" => RequestAccess::NodeLocal,
            "VisualizerLabRequest" => RequestAccess::NodeLocal,
            "VisualizerPlanRequest" => RequestAccess::NodeLocal,
            "VisualizerSupportRequest" => RequestAccess::NodeLocal,
            "VisualizerStatusRequest" => RequestAccess::NodeLocal,
            "ListVisualizerPresetsRequest" => RequestAccess::NodeLocal,
            "ListVisualizerCreditsRequest" => RequestAccess::NodeLocal,
            "VisualizerTextureShownRequest" => RequestAccess::NodeLocal,
            "SfxPlayRequest" => RequestAccess::NodeLocal,
            "IfAnalyzeExistsRequest" => RequestAccess::ConnectionView,
            "GetAnalyzeCountRequest" => RequestAccess::ConnectionView,
            "FetchMediaFilesRequest" => RequestAccess::ConnectionView,
            "FetchMediaFileByIdsRequest" => RequestAccess::ConnectionView,
            "FetchParsedMediaFileRequest" => RequestAccess::ConnectionView,
            "SearchMediaFileSummaryRequest" => RequestAccess::ConnectionView,
            "GetMediaFilesCountRequest" => RequestAccess::ConnectionView,
            "GetLyricByTrackIdRequest" => RequestAccess::ConnectionView,
            "FetchCollectionGroupSummaryRequest" => RequestAccess::ConnectionView,
            "FetchCollectionGroupsRequest" => RequestAccess::ConnectionView,
            "FetchCollectionByIdsRequest" => RequestAccess::ConnectionView,
            "SearchCollectionSummaryRequest" => RequestAccess::ConnectionView,
            "GetCoverArtIdsByMixQueriesRequest" => RequestAccess::ConnectionView,
            "GetPrimaryColorByTrackIdRequest" => RequestAccess::ConnectionView,
            "FetchAllPlaylistsRequest" => RequestAccess::ConnectionView,
            "CreatePlaylistRequest" => RequestAccess::ConnectionEdit,
            "CreateM3u8PlaylistRequest" => RequestAccess::ConnectionEdit,
            "UpdatePlaylistRequest" => RequestAccess::ConnectionEdit,
            "RemovePlaylistRequest" => RequestAccess::ConnectionEdit,
            "AddItemToPlaylistRequest" => RequestAccess::ConnectionEdit,
            "ReorderPlaylistItemPositionRequest" => RequestAccess::ConnectionEdit,
            "FetchPlaylistOccurrencesRequest" => RequestAccess::ConnectionView,
            "GetPlaylistByIdRequest" => RequestAccess::ConnectionView,
            "FetchAllMixesRequest" => RequestAccess::ConnectionView,
            "CreateMixRequest" => RequestAccess::ConnectionEdit,
            "UpdateMixRequest" => RequestAccess::ConnectionEdit,
            "RemoveMixRequest" => RequestAccess::ConnectionEdit,
            "AddItemToMixRequest" => RequestAccess::ConnectionEdit,
            "GetMixByIdRequest" => RequestAccess::ConnectionView,
            "MixQueryRequest" => RequestAccess::ConnectionView,
            "FetchMixQueriesRequest" => RequestAccess::ConnectionView,
            "ResolveMixQueryRequest" => RequestAccess::ConnectionView,
            "LoadResolvedItemsRequest" => RequestAccess::Control,
            "PlaylistRevisionRequest" => RequestAccess::Control,
            "CastResolvedItemsRequest" | "RemotePlaylistRevisionRequest" => RequestAccess::Control,
            "CastPreflightRequest" => RequestAccess::Control,
            "SessionSnapshotRequest" | "RefreshSessionRequest" => RequestAccess::Control,
            "SetLikedRequest" => RequestAccess::ConnectionEdit,
            "GetLikedRequest" => RequestAccess::ConnectionView,
            "ComplexQueryRequest" => RequestAccess::ConnectionView,
            "SearchForRequest" => RequestAccess::ConnectionView,
            "FetchDirectoryTreeRequest" => RequestAccess::ConnectionView,
            "AuthenticateSingleServiceRequest" => RequestAccess::NodeLocal,
            "AuthenticateMultipleServiceRequest" => RequestAccess::NodeLocal,
            "LogoutSingleServiceRequest" => RequestAccess::NodeLocal,
            "ListLogRequest" => RequestAccess::NodeLocal,
            "ClearLogRequest" => RequestAccess::NodeLocal,
            "RemoveLogRequest" => RequestAccess::NodeLocal,
            "SystemInfoRequest" => RequestAccess::NodeLocal,
            "RegisterLicenseRequest" => RequestAccess::NodeLocal,
            "ValidateLicenseRequest" => RequestAccess::NodeLocal,
            "StartBroadcastRequest" => RequestAccess::NodeLocal,
            "StopBroadcastRequest" => RequestAccess::NodeLocal,
            "StartListeningRequest" => RequestAccess::NodeLocal,
            "StopListeningRequest" => RequestAccess::NodeLocal,
            "GetDiscoveredDeviceRequest" => RequestAccess::NodeLocal,
            "StartServerRequest" => RequestAccess::NodeLocal,
            "StopServerRequest" => RequestAccess::NodeLocal,
            "ListClientsRequest" => RequestAccess::NodeLocal,
            "GetSslCertificateFingerprintRequest" => RequestAccess::NodeLocal,
            "AddTrustedServerRequest" => RequestAccess::NodeLocal,
            "RemoveTrustedClientRequest" => RequestAccess::NodeLocal,
            "UpdateClientStatusRequest" => RequestAccess::NodeLocal,
            "EditHostsRequest" => RequestAccess::NodeLocal,
            "RemoveTrustedServerRequest" => RequestAccess::NodeLocal,
            "ServerAvailabilityTestRequest" => RequestAccess::NodeLocal,
            "RegisterDeviceOnServerRequest" => RequestAccess::NodeLocal,
            "CheckDeviceOnServerRequest" => RequestAccess::NodeLocal,
            "ConnectRequest" => RequestAccess::NodeLocal,
            "FetchServerCertificateRequest" => RequestAccess::NodeLocal,
            "FetchRemoteFileRequest" => RequestAccess::NodeLocal,
            "RemoveItemFromPlaylistRequest" => RequestAccess::ConnectionEdit,
            _ => return None,
        })
    }

    #[test]
    fn request_catalogue_access_matches_frozen_classification() {
        macro_rules! assert_catalogue {
            ($($request:ident),*) => {
                $(
                    let name = stringify!($request);
                    assert_eq!(request_access(name), old_request_access(name), "{name}");
                    assert!(request_scope(name).is_some(), "{name} has no scope");
                )*
            };
        }
        for_all_requests0!(assert_catalogue);
        for name in non_local_request_names() {
            assert_ne!(request_access(name), Some(RequestAccess::NodeLocal), "{name}");
        }
    }

    #[tokio::test]
    async fn access_policy_presets_overrides_and_trust_lifecycle() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let a = fixture(dir.path()).await?;
        let fingerprint = "preset-test".to_owned();
        let permissions = a.global_params.permission_manager.write().await;
        permissions
            .add_user(
                "key".into(),
                fingerprint.clone(),
                "alias".into(),
                "test".into(),
                DeviceType::Desktop,
                "127.0.0.3".into(),
            )
            .await?;
        drop(permissions);
        let request = UpdateClientStatusRequest {
            call_id: None,
            fingerprint: fingerprint.clone(),
            status: ClientStatus::Approved,
            preset: Some(DevicePreset::Guest),
        };
        assert!(
            request
                .handle(request.extract_params(&a.global_params), None, &request)
                .await?
                .context("approval")?
                .success
        );
        assert_eq!(
            a.global_params.node.access_policy.grants(&fingerprint)?,
            DeviceGrants::guest()
        );
        assert_eq!(
            DeviceGrants::guest().connection_level(Uuid::new_v4()),
            ConnectionLevel::View
        );
        assert_eq!(
            DeviceGrants::guest().sync_level(Uuid::new_v4()),
            SyncLevel::None
        );
        let request = GetDeviceGrantsRequest {
            call_id: None,
            fingerprint: fingerprint.clone(),
        };
        let response = request
            .handle(request.extract_params(&a.global_params), None, &request)
            .await?
            .context("get grants")?;
        assert_eq!(response.libraries.len(), 1);
        assert!(matches!(
            response.libraries[0].connection,
            DeviceConnectionLevel::View
        ));
        let mut grants = DeviceGrants::my_device();
        let later = Uuid::new_v4();
        assert_eq!(grants.sync_level(later), SyncLevel::PullPush);
        grants
            .connection
            .libraries
            .insert(later, ConnectionLevel::None);
        grants.sync.libraries.insert(later, SyncLevel::None);
        // Per-library overrides apply independently on both axes.
        assert_eq!(grants.connection_level(later), ConnectionLevel::None);
        assert_eq!(grants.sync_level(later), SyncLevel::None);
        assert!(!grants.administer);
        a.global_params
            .permission_manager
            .write()
            .await
            .change_user_status(&fingerprint, UserStatus::Blocked)
            .await?;
        assert!(
            a.global_params
                .node
                .access_policy
                .grants(&fingerprint)
                .is_err()
        );
        a.global_params
            .permission_manager
            .write()
            .await
            .change_user_status(&fingerprint, UserStatus::Approved)
            .await?;
        assert_eq!(
            a.global_params.node.access_policy.grants(&fingerprint)?,
            DeviceGrants::my_device()
        );
        a.global_params
            .permission_manager
            .write()
            .await
            .remove_user(&fingerprint)
            .await?;
        assert!(
            a.global_params
                .node
                .access_policy
                .grants(&fingerprint)
                .is_err()
        );
        a.global_params.node.shutdown().await
    }
}
