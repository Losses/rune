use crate::macros::non_local_request_names;
mod access_policy_tests {
    use reqwest::StatusCode;
    use sea_orm::sqlx;
    use tokio::time as tokio_time;
    use url::Url;
    use super::player_session_tests::replicate_host_library_at;
    use super::*;
    use crate::Session;
    use crate::access_policy::{Action, ConnectionLevel, Denial, DeviceGrants, SyncLevel};
    use crate::backends::remote::connect_remote;
    use crate::macros::{
        BodyLibraries, RequestAccess, RequestScope, body_library_declarations, request_access,
        request_scope,
    };
    use crate::messages::{
        CastLibraryState, CastResolvedItemsRequest, CastResolvedItemsResponse, HandoffBlockId,
        HandoffLoadRequest, HandoffLoadResponse, LoadPlay, LoadResolvedItemsRequest,
        PlaylistOperateMode, RendererAvailability,
    };
    use crate::messages::{DeviceGrantsChange, DeviceGrantsInfo, RemoveTrustedClientRequest};
    use crate::messages::{
        GetLikedRequest, GetLyricByTrackIdRequest, GetPrimaryColorByTrackIdRequest,
        PlayingItemRef, SetLikedRequest,
    };
    use playback::player::PlayingItem;
    use crate::messages::{ExcludedItems, ResolveMixQueryRequest};
    use crate::request_error::ExpectedRequestError;
    use crate::server::api::remove_trusted_device;
    use crate::server::libraries::{
        cast_library_state, list_peer_libraries, list_renderer_candidates,
    };
    use crate::server::server_request::MAX_BODY_LIBRARIES;
    use crate::server::sync::client::LibrarySession;
    use crate::server::sync_worker::share_worker_tests::TestPeerSession;
    use crate::test_wait::{TEST_DEADLINE, wait_until};
    use database::actions::{file::file_ids_by_uuid, stats::{get_liked, set_liked}};
    use database::entities::{media_cover_art, media_files};
    use database::library_registry::{
        CatalogPlacement, FsAdapter, ProtocolAdapter, RuneForwardMode,
    };
    use http_request::sync::MetadataPeer;
    use sea_orm::{
        ActiveValue, EntityTrait, Set,
        sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions},
    };
    use sea_orm::prelude::Decimal;
    use std::fs;
    use std::collections::BTreeMap;
    use sync::protocol::SyncError;
    use tempfile::TempDir;
    use tokio::{sync::Mutex, time::sleep};

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn liked_requests_cannot_act_on_an_ungranted_body_library() -> Result<()> {
        let pair = PolicyPair::new().await?;
        let second_dir = tempfile::tempdir()?;
        let second = fixture_open_local_library(&pair.a, second_dir.path()).await?;
        let library_b = second.global_params.library_id.context("second library id")?;
        let library_b_handle = second
            .global_params
            .library
            .as_ref()
            .context("second library handle")?;
        let track = Uuid::new_v4();
        media_files::Entity::insert(media_files::ActiveModel {
            id: ActiveValue::NotSet,
            file_name: Set("track.flac".into()),
            directory: Set(String::new()),
            extension: Set("flac".into()),
            file_hash: Set("test-hash".into()),
            last_modified: Set(String::new()),
            change_token: Set(None),
            cover_art_uuid: Set(None),
            cover_art_checked_at: Set(None),
            sample_rate: Set(44_100),
            duration: Set(Decimal::ONE),
            local_available: Set(false),
            content_hash: Set(None),
            content_size: Set(None),
            source_uuid: Set(track.to_string()),
        })
        .exec(&*library_b_handle.main_db)
        .await?;
        let peer = info(&pair.b).await?;
        let mut grants = DeviceGrants::guest();
        grants.connection.all_libraries = None;
        grants.connection.libraries = BTreeMap::from([(pair.id, ConnectionLevel::ViewEdit)]);
        pair.a
            .global_params
            .node
            .access_policy
            .set(&peer.fingerprint, grants)?;
        let user = pair
            .a
            .global_params
            .node
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(&peer.fingerprint)
            .await
            .context("approved controller")?;
        let session = Session {
            fingerprint: user.fingerprint.clone(),
            public_key: user.public_key.clone(),
            base_url: Url::parse("https://127.0.0.1:7863/").unwrap(),
        };
        let second_session = Session {
            fingerprint: user.fingerprint.clone(),
            public_key: user.public_key.clone(),
            base_url: Url::parse("https://127.0.0.1:7863/").unwrap(),
        };
        let item = PlayingItemRef::LibraryTrack {
            library_id: library_b.to_string(),
            track: track.to_string(),
        };
        let node = Arc::clone(&pair.a.global_params.node);
        let set_request = SetLikedRequest {
            item: Some(item.clone()),
            liked: true,
            call_id: Some(101),
        };
        let (_, set_result) = node
            .websocket_service
            .handle_addressed_message(
                "SetLikedRequest",
                rinf::serialize(&set_request)?,
                Some(second_session),
                Some(pair.id),
                &node,
            )
            .await
            .context("SetLikedRequest route")?;
        let get_request = GetLikedRequest {
            item: Some(item),
            call_id: Some(102),
        };
        let (_, get_result) = node
            .websocket_service
            .handle_addressed_message(
                "GetLikedRequest",
                rinf::serialize(&get_request)?,
                Some(session),
                Some(pair.id),
                &node,
            )
            .await
            .context("GetLikedRequest route")?;
        let lyric_request = GetLyricByTrackIdRequest {
            item: Some(PlayingItemRef::LibraryTrack {
                library_id: library_b.to_string(),
                track: Uuid::new_v4().to_string(),
            }),
            call_id: Some(103),
        };
        let (_, lyric_result) = node
            .websocket_service
            .handle_addressed_message(
                "GetLyricByTrackIdRequest",
                rinf::serialize(&lyric_request)?,
                Some(Session {
                    fingerprint: user.fingerprint.clone(),
                    public_key: user.public_key.clone(),
                    base_url: Url::parse("https://127.0.0.1:7863/").unwrap(),
                }),
                Some(pair.id),
                &node,
            )
            .await
            .context("GetLyricByTrackIdRequest route")?;
        let color_request = GetPrimaryColorByTrackIdRequest {
            item: Some(PlayingItemRef::LibraryTrack {
                library_id: library_b.to_string(),
                track: Uuid::new_v4().to_string(),
            }),
            call_id: Some(104),
        };
        let (_, color_result) = node
            .websocket_service
            .handle_addressed_message(
                "GetPrimaryColorByTrackIdRequest",
                rinf::serialize(&color_request)?,
                Some(Session {
                    fingerprint: user.fingerprint,
                    public_key: user.public_key,
                    base_url: Url::parse("https://127.0.0.1:7863/").unwrap(),
                }),
                Some(pair.id),
                &node,
            )
            .await
            .context("GetPrimaryColorByTrackIdRequest route")?;

        for (name, result) in [
            ("SetLikedRequest", set_result),
            ("GetLikedRequest", get_result),
            ("GetLyricByTrackIdRequest", lyric_result),
            ("GetPrimaryColorByTrackIdRequest", color_result),
        ] {
            let error = result.expect_err(name);
            assert_eq!(classify(&error), RequestErrorKind::PermissionDenied, "{name}");
            assert!(matches!(
                error.downcast_ref::<Denial>(),
                Some(Denial::Action {
                    action: Action::ConnectionEdit { library_id }
                }) if *library_id == library_b
            ) || matches!(
                error.downcast_ref::<Denial>(),
                Some(Denial::Action {
                    action: Action::ConnectionView { library_id }
                }) if *library_id == library_b
            ), "{name}: {error:#}");
        }
        let track_id = file_ids_by_uuid(&library_b_handle.main_db, &[track])
            .await?
            .get(&track)
            .copied()
            .context("seeded second-library track")?;
        assert!(!get_liked(&*library_b_handle.main_db, track_id).await?);
        pair.close().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resolve_mix_query_cannot_name_an_item_of_an_ungranted_library() -> Result<()> {
        let pair = PolicyPair::new().await?;
        let second_dir = tempfile::tempdir()?;
        let second = fixture_open_local_library(&pair.a, second_dir.path()).await?;
        let library_b = second.global_params.library_id.context("second library id")?;
        let peer = info(&pair.b).await?;
        let mut grants = DeviceGrants::guest();
        grants.connection.all_libraries = None;
        grants.connection.libraries = BTreeMap::from([(pair.id, ConnectionLevel::View)]);
        pair.a
            .global_params
            .node
            .access_policy
            .set(&peer.fingerprint, grants)?;
        let user = pair
            .a
            .global_params
            .node
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(&peer.fingerprint)
            .await
            .context("approved controller")?;
        let request = ResolveMixQueryRequest {
            queries: Vec::new(),
            hint_position: -1,
            initial_playback_item: None,
            instantly_play: false,
            fallback_playing_items: vec![PlayingItemRef::LibraryTrack {
                library_id: library_b.to_string(),
                track: Uuid::new_v4().to_string(),
            }],
            excluded: ExcludedItems::default(),
            call_id: Some(105),
        };
        let node = Arc::clone(&pair.a.global_params.node);
        let (_, result) = node
            .websocket_service
            .handle_addressed_message(
                "ResolveMixQueryRequest",
                rinf::serialize(&request)?,
                Some(Session {
                    fingerprint: user.fingerprint,
                    public_key: user.public_key,
                    base_url: Url::parse("https://127.0.0.1:7863/").unwrap(),
                }),
                Some(pair.id),
                &node,
            )
            .await
            .context("ResolveMixQueryRequest route")?;

        let error = result.expect_err("an item of an ungranted library must be refused");
        assert_eq!(classify(&error), RequestErrorKind::PermissionDenied);
        assert!(matches!(
            error.downcast_ref::<Denial>(),
            Some(Denial::Action {
                action: Action::ConnectionView { library_id }
            }) if *library_id == library_b
        ), "{error:#}");
        pair.close().await?;
        Ok(())
    }

    /// A controller may control this player without being allowed to read every library it
    /// names. Cast, load and handoff must check each distinct named library for the calling
    /// session before the player reads a track or edits its queue.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn player_reads_requests_authorize_every_named_library() -> Result<()> {
        let pair = PolicyPair::new().await?;
        let second_dir = tempfile::tempdir()?;
        let second = fixture_open_local_library(&pair.b, second_dir.path()).await?;
        let granted_library = pair.id;
        let denied_library = second
            .global_params
            .library_id
            .context("second library id")?;

        let granted_track = Uuid::new_v4();
        let denied_track = Uuid::new_v4();
        seed_player_read_track(&pair.a, "granted.ogg", granted_track).await?;
        seed_player_read_track(&pair.b, "granted.ogg", granted_track).await?;
        seed_player_read_track(&second, "denied.ogg", denied_track).await?;

        let granted_item = PlayingItemRef::LibraryTrack {
            library_id: granted_library.to_string(),
            track: granted_track.to_string(),
        };
        let denied_item = PlayingItemRef::LibraryTrack {
            library_id: denied_library.to_string(),
            track: denied_track.to_string(),
        };
        let malformed_item = PlayingItemRef::LibraryTrack {
            library_id: "not-a-uuid".into(),
            track: granted_track.to_string(),
        };

        let controller = info(&pair.a).await?.fingerprint;
        set_controller_library_grants(&pair, &controller, &[granted_library])?;
        let user = pair
            .b
            .global_params
            .node
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(&controller)
            .await
            .context("approved controller")?;
        let session = Session {
            fingerprint: user.fingerprint,
            public_key: user.public_key,
            base_url: Url::parse("https://127.0.0.1:7863/").unwrap(),
        };
        let node = Arc::clone(&pair.b.global_params.node);

        let queue_before = pair.b.global_params.player.lock().await.get_playlist();
        let revision_before = node.session()?.queue_revision().await;
        let renderer_queue_before = pair.a.global_params.player.lock().await.get_playlist();
        let renderer_revision_before = pair.a.global_params.node.session()?.queue_revision().await;

        for (name, payload) in [
            (
                "LoadResolvedItemsRequest",
                rinf::serialize(&load_request(
                    vec![denied_item.clone()],
                    PlaylistOperateMode::Replace,
                ))?,
            ),
            (
                "HandoffLoadRequest",
                rinf::serialize(&handoff_request(vec![denied_item.clone()]))?,
            ),
            (
                "CastResolvedItemsRequest",
                rinf::serialize(&cast_request(&controller, vec![denied_item.clone()]))?,
            ),
        ] {
            let error = send_player_read(&node, name, payload, &session)
                .await
                .expect_err("a library the controller cannot read must be refused");
            assert_connection_view_denial(&error, denied_library);
            assert_eq!(
                pair.b.global_params.player.lock().await.get_playlist(),
                queue_before,
                "{name}"
            );
            assert_eq!(
                node.session()?.queue_revision().await,
                revision_before,
                "{name}"
            );
            assert_eq!(
                pair.a.global_params.player.lock().await.get_playlist(),
                renderer_queue_before,
                "{name}"
            );
            assert_eq!(
                pair.a.global_params.node.session()?.queue_revision().await,
                renderer_revision_before,
                "{name}"
            );
        }

        for (name, payload) in [
            (
                "LoadResolvedItemsRequest",
                rinf::serialize(&load_request(
                    vec![granted_item.clone(), denied_item.clone()],
                    PlaylistOperateMode::Replace,
                ))?,
            ),
            (
                "HandoffLoadRequest",
                rinf::serialize(&handoff_request(vec![
                    granted_item.clone(),
                    denied_item.clone(),
                ]))?,
            ),
            (
                "CastResolvedItemsRequest",
                rinf::serialize(&cast_request(
                    &controller,
                    vec![granted_item.clone(), denied_item.clone()],
                ))?,
            ),
        ] {
            let error = send_player_read(&node, name, payload, &session)
                .await
                .expect_err("a mixed request must refuse its ungranted library");
            assert_connection_view_denial(&error, denied_library);
            assert_eq!(
                pair.b.global_params.player.lock().await.get_playlist(),
                queue_before,
                "{name}"
            );
            assert_eq!(
                node.session()?.queue_revision().await,
                revision_before,
                "{name}"
            );
            assert_eq!(
                pair.a.global_params.player.lock().await.get_playlist(),
                renderer_queue_before,
                "{name}"
            );
            assert_eq!(
                pair.a.global_params.node.session()?.queue_revision().await,
                renderer_revision_before,
                "{name}"
            );
        }

        for (name, payload) in [
            (
                "LoadResolvedItemsRequest",
                rinf::serialize(&load_request(
                    vec![malformed_item.clone()],
                    PlaylistOperateMode::Replace,
                ))?,
            ),
            (
                "HandoffLoadRequest",
                rinf::serialize(&handoff_request(vec![malformed_item.clone()]))?,
            ),
            (
                "CastResolvedItemsRequest",
                rinf::serialize(&cast_request(&controller, vec![malformed_item.clone()]))?,
            ),
        ] {
            let error = send_player_read(&node, name, payload, &session)
                .await
                .expect_err("a malformed library id must be refused");
            assert_eq!(
                classify(&error),
                RequestErrorKind::InvalidInput,
                "{name}: {error:#}"
            );
            assert_eq!(
                pair.b.global_params.player.lock().await.get_playlist(),
                queue_before,
                "{name}"
            );
            assert_eq!(
                node.session()?.queue_revision().await,
                revision_before,
                "{name}"
            );
            assert_eq!(
                pair.a.global_params.player.lock().await.get_playlist(),
                renderer_queue_before,
                "{name}"
            );
            assert_eq!(
                pair.a.global_params.node.session()?.queue_revision().await,
                renderer_revision_before,
                "{name}"
            );
        }

        set_controller_library_grants(&pair, &controller, &[granted_library, denied_library])?;
        let allowed_load = load_request(
            vec![granted_item.clone(), denied_item.clone()],
            PlaylistOperateMode::Replace,
        );
        send_player_read(
            &node,
            "LoadResolvedItemsRequest",
            rinf::serialize(&allowed_load)?,
            &session,
        )
        .await?;
        let loaded_queue = pair.b.global_params.player.lock().await.get_playlist();
        assert!(loaded_queue.contains(&playing_item(granted_library, granted_track)));
        assert!(loaded_queue.contains(&playing_item(denied_library, denied_track)));
        let revision_after_load = node.session()?.queue_revision().await;
        assert!(revision_after_load > revision_before);

        let allowed_handoff = handoff_request(vec![granted_item.clone(), denied_item.clone()]);
        let payload = send_player_read(
            &node,
            "HandoffLoadRequest",
            rinf::serialize(&allowed_handoff)?,
            &session,
        )
        .await?;
        let handoff: HandoffLoadResponse = rinf::deserialize(&payload)?;
        assert_eq!(handoff.handoff_item, granted_item);
        let handed_off_queue = pair.b.global_params.player.lock().await.get_playlist();
        assert!(handed_off_queue.contains(&playing_item(granted_library, granted_track)));
        assert!(handed_off_queue.contains(&playing_item(denied_library, denied_track)));
        assert!(node.session()?.queue_revision().await > revision_after_load);

        // A is also the cast target: the cast goes to B, which forwards its load to A, and
        // both hold the same library id and track, so the allowed cast has a real queue to edit.
        let renderer_revision = pair.a.global_params.node.session()?.queue_revision().await;
        let controller_queue = pair.b.global_params.player.lock().await.get_playlist();
        let controller_revision = node.session()?.queue_revision().await;
        let allowed_cast = cast_request(&controller, vec![granted_item.clone()]);
        let payload = send_player_read(
            &node,
            "CastResolvedItemsRequest",
            rinf::serialize(&allowed_cast)?,
            &session,
        )
        .await?;
        let cast: CastResolvedItemsResponse = rinf::deserialize(&payload)?;
        assert!(cast.result.left_out_items.is_empty());
        assert!(
            pair.a
                .global_params
                .player
                .lock()
                .await
                .get_playlist()
                .contains(&playing_item(granted_library, granted_track))
        );
        assert!(
            pair.a.global_params.node.session()?.queue_revision().await > renderer_revision
        );
        assert_eq!(
            pair.b.global_params.player.lock().await.get_playlist(),
            controller_queue
        );
        assert_eq!(
            node.session()?.queue_revision().await,
            controller_revision
        );

        pair.close().await?;
        Ok(())
    }

    /// The library limit counts distinct libraries, and a request over it is refused before
    /// any per-library authorization or handler work.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn player_reads_requests_bound_the_number_of_named_libraries() -> Result<()> {
        let pair = PolicyPair::new().await?;
        let controller = info(&pair.a).await?.fingerprint;
        set_controller_library_grants(&pair, &controller, &[])?;
        let user = pair
            .b
            .global_params
            .node
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(&controller)
            .await
            .context("approved controller")?;
        let session = Session {
            fingerprint: user.fingerprint,
            public_key: user.public_key,
            base_url: Url::parse("https://127.0.0.1:7863/").unwrap(),
        };
        let node = Arc::clone(&pair.b.global_params.node);
        let queue_before = pair.b.global_params.player.lock().await.get_playlist();
        let revision_before = node.session()?.queue_revision().await;

        let at_limit = (0..MAX_BODY_LIBRARIES)
            .map(|_| PlayingItemRef::LibraryTrack {
                library_id: Uuid::new_v4().to_string(),
                track: Uuid::new_v4().to_string(),
            })
            .collect::<Vec<_>>();
        let error = send_player_read(
            &node,
            "LoadResolvedItemsRequest",
            rinf::serialize(&load_request(at_limit, PlaylistOperateMode::Replace))?,
            &session,
        )
        .await
        .expect_err("libraries without a grant must still be refused at the limit");
        assert_eq!(
            classify(&error),
            RequestErrorKind::PermissionDenied,
            "{error:#}"
        );
        assert_eq!(
            pair.b.global_params.player.lock().await.get_playlist(),
            queue_before
        );
        assert_eq!(node.session()?.queue_revision().await, revision_before);

        let over_limit = (0..=MAX_BODY_LIBRARIES)
            .map(|_| PlayingItemRef::LibraryTrack {
                library_id: Uuid::new_v4().to_string(),
                track: Uuid::new_v4().to_string(),
            })
            .collect::<Vec<_>>();
        let error = send_player_read(
            &node,
            "LoadResolvedItemsRequest",
            rinf::serialize(&load_request(over_limit, PlaylistOperateMode::Replace))?,
            &session,
        )
        .await
        .expect_err("a request over the library limit must be refused");
        assert_eq!(
            classify(&error),
            RequestErrorKind::InvalidInput,
            "{error:#}"
        );
        assert_eq!(
            pair.b.global_params.player.lock().await.get_playlist(),
            queue_before
        );
        assert_eq!(node.session()?.queue_revision().await, revision_before);

        pair.close().await?;
        Ok(())
    }

    async fn send_player_read(
        node: &Arc<NodeContext>,
        name: &str,
        payload: Vec<u8>,
        session: &Session,
    ) -> Result<Vec<u8>> {
        let (_, result) = node
            .websocket_service
            .handle_addressed_message(
                name,
                payload,
                Some(Session {
                    fingerprint: session.fingerprint.clone(),
                    public_key: session.public_key.clone(),
                    base_url: session.base_url.clone(),
                }),
                None,
                node,
            )
            .await
            .with_context(|| format!("{name} route"))?;
        result
    }

    fn assert_connection_view_denial(error: &anyhow::Error, library_id: Uuid) {
        assert_eq!(
            classify(error),
            RequestErrorKind::PermissionDenied,
            "{error:#}"
        );
        assert!(
            matches!(
                error.downcast_ref::<Denial>(),
                Some(Denial::Action {
                    action: Action::ConnectionView { library_id: denied }
                }) if *denied == library_id
            ),
            "{error:#}"
        );
    }

    fn set_controller_library_grants(
        pair: &PolicyPair,
        fingerprint: &str,
        libraries: &[Uuid],
    ) -> Result<()> {
        let mut grants = DeviceGrants::guest();
        grants.connection.all_libraries = None;
        grants.connection.libraries = libraries
            .iter()
            .map(|library_id| (*library_id, ConnectionLevel::View))
            .collect();
        pair.b
            .global_params
            .node
            .access_policy
            .set(fingerprint, grants)?;
        Ok(())
    }

    fn load_request(
        items: Vec<PlayingItemRef>,
        mode: PlaylistOperateMode,
    ) -> LoadResolvedItemsRequest {
        LoadResolvedItemsRequest {
            items,
            start_index: Some(0),
            mode,
            expected_revision: None,
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Pause,
            call_id: None,
        }
    }

    fn handoff_request(items: Vec<PlayingItemRef>) -> HandoffLoadRequest {
        HandoffLoadRequest {
            block: HandoffBlockId {
                uuid: Uuid::new_v4().to_string(),
            },
            load: load_request(items, PlaylistOperateMode::PlayNext),
            call_id: None,
        }
    }

    fn cast_request(fingerprint: &str, items: Vec<PlayingItemRef>) -> CastResolvedItemsRequest {
        CastResolvedItemsRequest {
            fingerprint: fingerprint.to_owned(),
            load: load_request(items, PlaylistOperateMode::Replace),
            call_id: None,
        }
    }

    fn playing_item(library_id: Uuid, track: Uuid) -> PlayingItem {
        PlayingItem::LibraryTrack {
            library_id,
            track: track.to_string(),
        }
    }

    async fn seed_player_read_track(fixture: &Fixture, name: &str, track: Uuid) -> Result<()> {
        let library = fixture.global_params.test_library();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
        fs::copy(source, Path::new(&library.media_root).join(name))?;
        media_files::Entity::insert(media_files::ActiveModel {
            id: ActiveValue::NotSet,
            file_name: Set(name.into()),
            directory: Set(String::new()),
            extension: Set("ogg".into()),
            file_hash: Set(track.to_string()),
            last_modified: Set(String::new()),
            change_token: Set(None),
            cover_art_uuid: Set(None),
            cover_art_checked_at: Set(None),
            sample_rate: Set(44_100),
            duration: Set(Decimal::ONE),
            local_available: Set(true),
            content_hash: Set(None),
            content_size: Set(None),
            source_uuid: Set(track.to_string()),
        })
        .exec(&*library.main_db)
        .await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn guest_set_liked_request_returns_typed_connection_edit_denial() -> Result<()> {
        let pair = PolicyPair::new().await?;
        let peer = info(&pair.b).await?;
        pair.a
            .global_params
            .node
            .access_policy
            .set(&peer.fingerprint, DeviceGrants::guest())?;
        let user = pair
            .a
            .global_params
            .node
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(&peer.fingerprint)
            .await
            .context("approved guest controller")?;
        let session = Session {
            fingerprint: user.fingerprint,
            public_key: user.public_key,
            base_url: Url::parse("https://127.0.0.1:7863/").unwrap(),
        };

        let node = Arc::clone(&pair.a.global_params.node);
        let (_, result) = node
            .websocket_service
            .handle_addressed_message(
                "SetLikedRequest",
                Vec::new(),
                Some(session),
                Some(pair.id),
                &node,
            )
            .await
            .context("SetLikedRequest route")?;
        let error = result.expect_err("guest must not be allowed to set liked state");
        assert_eq!(classify(&error), RequestErrorKind::PermissionDenied);
        assert!(matches!(
            error.downcast_ref::<Denial>(),
            Some(Denial::Action {
                action: Action::ConnectionEdit { library_id }
            }) if *library_id == pair.id
        ));
        pair.close().await?;
        Ok(())
    }

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
            ProtocolAdapter::RuneForward {
                mode:
                    RuneForwardMode::Replica {
                        access_level,
                        connection_access,
                    },
                ..
            } => RuneForwardMode::Replica {
                access_level: *access_level,
                connection_access: *connection_access,
            },
            _ => RuneForwardMode::Replica {
                access_level: None,
                connection_access: None,
            },
        };
        record.protocol = ProtocolAdapter::RuneForward {
            owner_fingerprint: fingerprint.clone(),
            mode,
        };
        record.fs = local_path
            .into_iter()
            .map(|path| FsAdapter::Local { path })
            .chain([FsAdapter::RunePeer { fingerprint }])
            .collect();
        if record.catalog == CatalogPlacement::None {
            record.catalog = CatalogPlacement::ConfigReplica;
        }
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
            )?;
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
            wait_until("peer sync pass to complete", || async {
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
                match session
                    .pass(
                        local.global_params.clone(),
                        &local.private_key,
                        endpoint.clone(),
                        true,
                    )
                    .await?
                {
                    PeerPass::Incomplete => Ok(None),
                    result => Ok(Some(result)),
                }
            })
            .await
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
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut connection)
            .await?;
        let mut contender = SqliteConnection::connect_with(&options).await?;
        let error = sqlx::query("BEGIN IMMEDIATE")
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

    async fn set_sqlite_busy_timeout(fixture: &Fixture) -> Result<()> {
        let pool = fixture
            .global_params
            .test_library()
            .main_db
            .get_sqlite_connection_pool();
        let mut connections = Vec::new();
        for _ in 0..pool.options().get_max_connections() {
            connections.push(pool.acquire().await?);
        }
        for connection in &mut connections {
            sqlx::query("PRAGMA busy_timeout = 100")
                .execute(&mut **connection)
                .await?;
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sync_recovers_after_replica_write_lock_is_released() -> Result<()> {
        let pair = PolicyPair::new_replica().await?;
        let host = pair.a.global_params.test_library();
        let key = write_album(host, "busy recovery").await?;
        let mut lock = lock_replica_library(&pair.b).await?;
        set_sqlite_busy_timeout(&pair.b).await?;

        let error = pair
            .pass(false)
            .await
            .expect_err("held SQLite lock must exhaust the bounded retry sequence");
        assert_eq!(
            error.downcast_ref::<SyncError>(),
            Some(&SyncError::StorageBusy)
        );
        sqlx::query("ROLLBACK").execute(&mut lock).await?;
        assert_eq!(pair.pass(false).await?, PeerPass::Complete);
        let replica = pair.b.global_params.test_library();
        assert!(replica.library_sync.get(&key).await?.is_some());
        pair.close().await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sync_reports_storage_busy_after_retry_cap() -> Result<()> {
        let pair = PolicyPair::new_replica().await?;
        let host = pair.a.global_params.test_library();
        write_album(host, "busy cap").await?;
        let mut lock = lock_replica_library(&pair.b).await?;
        set_sqlite_busy_timeout(&pair.b).await?;

        let result = pair.pass(false).await;
        sqlx::query("ROLLBACK").execute(&mut lock).await?;
        let error = result.expect_err("held SQLite lock must exhaust the retry cap");
        assert_eq!(
            error.downcast_ref::<SyncError>(),
            Some(&SyncError::StorageBusy)
        );
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
            source_uuid: ActiveValue::Set(Uuid::new_v4().to_string()),
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
        assert_eq!(json["version"], 3);
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
            &rinf::serialize(&PauseRequest { call_id: None })?,
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
        assert_eq!(response.status(), StatusCode::OK);
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
        let base = sync_worker::peer_base("127.0.0.1", a_addr.port())?;

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
    async fn approving_peer_lists_libraries_without_reverse_pairing() -> Result<()> {
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let id = Uuid::new_v4();
        let a = fixture_with_library(first.path(), id).await?;
        let b = fixture(second.path()).await?;
        // B paired with A and A approved it. A never paired with B, so B has no entry for A.
        let (_, a_fingerprint) = start_test_tls_server(&a).await?;
        pair(&a, &b).await?;
        let b_fingerprint = info(&b).await?.fingerprint;
        let a_permissions = &a.global_params.permission_manager;
        let b_permissions = &b.global_params.permission_manager;
        assert!(
            b_permissions
                .read()
                .await
                .verify_by_fingerprint(&a_fingerprint)
                .await
                .is_none()
        );
        let list = || list_peer_libraries(&b.global_params.node, &b.private_key, &a_fingerprint);

        let listed = list()
            .await
            .context("listing the libraries of a peer that approved this device")?;
        assert_eq!(
            listed
                .iter()
                .filter(|library| library.library_id == id.to_string())
                .count(),
            1,
            "{listed:?}"
        );

        // A refuses a device it does not approve.
        a_permissions
            .write()
            .await
            .change_user_status(&b_fingerprint, UserStatus::Pending)
            .await?;
        let refused = list().await;
        assert!(
            matches!(refused, Err(RemoteConnectError::NotAuthorized)),
            "{refused:?}"
        );
        a_permissions
            .write()
            .await
            .change_user_status(&b_fingerprint, UserStatus::Approved)
            .await?;

        // B refuses A once B holds an entry for A that is pending or blocked.
        b_permissions
            .write()
            .await
            .add_user(
                a.certificate.clone(),
                a_fingerprint.clone(),
                "paired".into(),
                "test".into(),
                DeviceType::Desktop,
                "127.0.0.2".into(),
            )
            .await?;
        for status in [UserStatus::Pending, UserStatus::Blocked] {
            b_permissions
                .write()
                .await
                .change_user_status(&a_fingerprint, status.clone())
                .await?;
            let refused = list().await;
            assert!(
                matches!(refused, Err(RemoteConnectError::NotAuthorized)),
                "{status:?}: {refused:?}"
            );
        }

        // Once B removes A, A still approving B is not enough.
        remove_trusted_device(
            &*b_permissions.read().await,
            &*b.global_params.cert_validator.read().await,
            &a_fingerprint,
        )
        .await?;
        let refused = list().await;
        assert!(
            matches!(refused, Err(RemoteConnectError::UntrustedEndpoint)),
            "{refused:?}"
        );
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
        let replica = registry.catalog(pair.id).context("replica descriptor")?;
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
            .filter(Column::SourceUuid.eq(&original.source_uuid))
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
        let session = Arc::new(Mutex::new(LibrarySession::new(
            right.library_sync.clone(),
        )?));
        let peer = Arc::new(peer);
        let result = wait_until(
            "read-only sync pass to reject its write",
            {
                let session = session.clone();
                let peer = peer.clone();
                move || {
                    let session = session.clone();
                    let peer = peer.clone();
                    async move {
                        match session.lock().await.advance(&peer, Arc::new(|| {})).await {
                            Ok(true) => Ok(Some(Ok(()))),
                            Ok(false) => Ok(None),
                            Err(error) => Ok(Some(Err(error))),
                        }
                    }
                }
            },
        )
        .await?;
        let error = result.expect_err("read-only push was accepted");
        assert!(matches!(
            error.downcast_ref::<SyncError>(),
            Some(SyncError::Forbidden)
        ));
        for _ in 0..3 {
            tokio_time::timeout(TEST_DEADLINE, sleep(Duration::from_secs(1))).await?;
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
        let replica = registry.catalog(pair.id).context("replica descriptor")?;
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
        request
            .handle(
                request.extract_params(&pair.a.global_params),
                None,
                &request,
            )
            .await?
            .context("remove device response")?;

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
            wait_until("third party sync pass to complete", || async {
                match session
                    .pass(
                        pair.b.global_params.clone(),
                        &pair.b.private_key,
                        endpoint.clone(),
                        true,
                    )
                    .await?
                {
                    PeerPass::Incomplete => Ok(None),
                    result => Ok(Some(result)),
                }
            })
            .await
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
        assert_eq!(metadata.status(), StatusCode::FORBIDDEN);
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
                StatusCode::FORBIDDEN,
                "{path}"
            );
        }
        pair.close().await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn access_policy_guest_browses_but_cannot_pull() -> Result<()> {
        use crate::messages::CreatePlaylistRequest;

        let pair = PolicyPair::new().await?;
        let fingerprint = info(&pair.b).await?.fingerprint;
        let request = SetDeviceGrantsRequest {
            call_id: None,
            fingerprint: fingerprint.clone(),
            grants: DeviceGrantsChange::Preset(DevicePreset::Guest),
        };
        request
            .handle(
                request.extract_params(&pair.a.global_params),
                None,
                &request,
            )
            .await?;
        let share_request = SetDeviceGrantsRequest {
            call_id: None,
            fingerprint,
            grants: DeviceGrantsChange::Set(DeviceGrantsInfo {
                control: true,
                all_connection: None,
                all_sync: None,
                libraries: vec![DeviceLibraryGrant {
                    library_id: pair.id.to_string(),
                    connection: DeviceConnectionLevel::View,
                    sync: DeviceSyncLevel::None,
                }],
                administer: false,
            }),
        };
        share_request
            .handle(
                share_request.extract_params(&pair.a.global_params),
                None,
                &share_request,
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
        let (name, _) = socket_request_for_library(
            &mut socket,
            "GetMediaFilesCountRequest",
            &rinf::serialize(&GetMediaFilesCountRequest { call_id: None })?,
            pair.id,
        )
        .await?;
        assert_eq!(name, "GetMediaFilesCountResponse");
        let (name, payload) = socket_request_for_library(
            &mut socket,
            "CreatePlaylistRequest",
            &rinf::serialize(&CreatePlaylistRequest {
                call_id: None,
                name: "guest denied".into(),
                group: "test".into(),
            })?,
            pair.id,
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
        assert_eq!(metadata.status(), StatusCode::FORBIDDEN);
        let source = pair.a.global_params.library_pool.acquire(pair.id).await?;
        let album = write_album(&source, "guest cannot replicate").await?;
        assert_eq!(pair.pass(false).await?, PeerPass::LibraryNotServed);
        let replica = pair.b.global_params.library_pool.acquire(pair.id).await?;
        assert!(replica.library_sync.get(&album).await?.is_none());
        let [audio, cover] = served_files(&pair, "guest-browse.txt").await?;
        assert_eq!(status(&pair, &audio).await?, StatusCode::FORBIDDEN);
        assert_eq!(status(&pair, &cover).await?, StatusCode::OK);
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
            StatusCode::FORBIDDEN
        );
        socket.close(None).await?;
        pair.close().await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn guest_sees_and_browses_only_explicitly_shared_libraries() -> Result<()> {
        use crate::backends::remote::RemoteStream;
        use crate::messages::{
            FetchAllPlaylistsRequest, GetOpenLibraryRequest, GetOpenLibraryResponse,
            SearchForRequest,
        };

        let mut pair = PolicyPair::new().await?;
        let shared_id = pair.id;
        LibraryRegistry::update(Path::new(&*pair.a.global_params.config_path), |registry| {
            let record = registry
                .libraries
                .iter_mut()
                .find(|record| record.catalog_library_id() == Some(shared_id))
                .context("shared library record")?;
            record.alias = "Shared Music".into();
            Ok(())
        })
        .await?;
        scan_held_track(&pair.a, shared_id, "shared.ogg").await?;

        let unshared_dir = tempfile::tempdir()?;
        let unshared_registered = register_local_library_on_node(
            &RegisterLocalLibraryRequest {
                call_id: None,
                path: unshared_dir
                    .path()
                    .to_str()
                    .context("unshared path")?
                    .into(),
                alias: "Private Archive".into(),
                mode: Some(LibraryInitializeMode::Portable),
            },
            &pair.a.global_params.node,
        )
        .await?
        .record
        .context("unshared library record")?;
        pair.a = fixture_open_record(&pair.a, unshared_registered.record_id).await?;
        let unshared_id = pair
            .a
            .global_params
            .library_id
            .context("unshared library id")?;
        pair.a.stop_worker().await?;
        scan_held_track(&pair.a, unshared_id, "private.ogg").await?;
        announce_peer(&pair.b, &info(&pair.a).await?, pair.addr).await?;

        let guest_fingerprint = info(&pair.b).await?.fingerprint;
        let host_fingerprint = info(&pair.a).await?.fingerprint;
        let preset_request = SetDeviceGrantsRequest {
            call_id: None,
            fingerprint: guest_fingerprint.clone(),
            grants: DeviceGrantsChange::Preset(DevicePreset::Guest),
        };
        preset_request
            .handle(
                preset_request.extract_params(&pair.a.global_params),
                None,
                &preset_request,
            )
            .await?;

        let load_grants = || {
            let node = pair.a.global_params.node.clone();
            let fingerprint = guest_fingerprint.clone();
            async move {
                let request = GetDeviceGrantsRequest {
                    call_id: None,
                    fingerprint,
                };
                request
                    .handle(node, None, &request)
                    .await?
                    .context("loaded guest grants")
            }
        };
        let save_library_grants = |libraries: Vec<DeviceLibraryGrant>| {
            let params = pair.a.global_params.clone();
            let fingerprint = guest_fingerprint.clone();
            async move {
                let current = load_grants().await?.grants;
                let request = SetDeviceGrantsRequest {
                    call_id: None,
                    fingerprint,
                    grants: DeviceGrantsChange::Set(DeviceGrantsInfo {
                        control: current.control,
                        all_connection: current.all_connection,
                        all_sync: current.all_sync,
                        libraries,
                        administer: current.administer,
                    }),
                };
                request
                    .handle(request.extract_params(&params), None, &request)
                    .await?;
                Ok::<_, anyhow::Error>(())
            }
        };
        let listed_pairs = || async {
            let value = pair.listings().await?;
            let entries = value.as_array().context("library list array")?;
            let mut pairs: Vec<(String, String, String, String)> = entries
                .iter()
                .map(|entry| {
                    (
                        entry["library_id"].as_str().unwrap_or_default().to_owned(),
                        entry["alias"].as_str().unwrap_or_default().to_owned(),
                        entry["connection"].as_str().unwrap_or_default().to_owned(),
                        entry["sync"].as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect();
            pairs.sort();
            Ok::<_, anyhow::Error>(pairs)
        };

        // Share only `shared_id` with browse-only access (`View`, sync `None`) on the Guest preset.
        save_library_grants(vec![DeviceLibraryGrant {
            library_id: shared_id.to_string(),
            connection: DeviceConnectionLevel::View,
            sync: DeviceSyncLevel::None,
        }])
        .await?;

        // Add a third native library after the Guest grant; it is not shared.
        let later_dir = tempfile::tempdir()?;
        let later_registered = register_local_library_on_node(
            &RegisterLocalLibraryRequest {
                call_id: None,
                path: later_dir.path().to_str().context("later path")?.into(),
                alias: "Added Later Secret".into(),
                mode: Some(LibraryInitializeMode::Portable),
            },
            &pair.a.global_params.node,
        )
        .await?
        .record
        .context("later library record")?;
        let later_id = LibraryRegistry::load(Path::new(&*pair.a.global_params.config_path))?
            .libraries
            .into_iter()
            .find(|record| record.record_id.0.to_string() == later_registered.record_id)
            .and_then(|record| record.catalog_library_id())
            .context("later library id")?;
        scan_held_track(&pair.a, later_id, "later.ogg").await?;

        assert_eq!(
            listed_pairs().await?,
            vec![(
                shared_id.to_string(),
                "Shared Music".to_owned(),
                "View".to_owned(),
                "None".to_owned(),
            )],
        );
        let peer_listed = list_peer_libraries(
            &pair.b.global_params.node,
            &pair.b.private_key,
            &host_fingerprint,
        )
        .await?;
        assert_eq!(peer_listed.len(), 1);
        assert_eq!(peer_listed[0].library_id, shared_id.to_string());
        assert_eq!(peer_listed[0].alias, "Shared Music");

        let mut socket = connect_remote(
            "127.0.0.1",
            pair.addr.port(),
            &pair.b.global_params.config_path,
            pair.b.global_params.cert_validator.read().await.clone(),
            &pair.b.global_params.node.sync_tokens,
        )
        .await?;

        // Host's active UI library is `unshared_id`; `GetOpenLibraryRequest` must not reveal it.
        let (open_name, open_payload) = socket_request(
            &mut socket,
            "GetOpenLibraryRequest",
            &rinf::serialize(&GetOpenLibraryRequest { call_id: None })?,
        )
        .await?;
        assert_eq!(open_name, "GetOpenLibraryResponse");
        let open_response: GetOpenLibraryResponse = rinf::deserialize(&open_payload)?;
        assert_eq!(open_response.library_id, None);

        async fn assert_catalog_allowed(socket: &mut RemoteStream, library_id: Uuid) -> Result<()> {
            let (name, payload) = socket_request_for_library(
                socket,
                "GetMediaFilesCountRequest",
                &rinf::serialize(&GetMediaFilesCountRequest { call_id: None })?,
                library_id,
            )
            .await?;
            assert_eq!(name, "GetMediaFilesCountResponse");
            let count: GetMediaFilesCountResponse = rinf::deserialize(&payload)?;
            assert_eq!(count.count, 1);

            let (name, _) = socket_request_for_library(
                socket,
                "FetchAllPlaylistsRequest",
                &rinf::serialize(&FetchAllPlaylistsRequest { call_id: None })?,
                library_id,
            )
            .await?;
            assert_eq!(name, "FetchAllPlaylistsResponse");

            let (name, _) = socket_request_for_library(
                socket,
                "SearchForRequest",
                &rinf::serialize(&SearchForRequest {
                    call_id: None,
                    query_str: "a".into(),
                    fields: Vec::new(),
                    n: 10,
                })?,
                library_id,
            )
            .await?;
            assert_eq!(name, "SearchForResponse");
            Ok(())
        }

        async fn assert_catalog_denied(socket: &mut RemoteStream, library_id: Uuid) -> Result<()> {
            for (request_name, payload) in [
                (
                    "GetMediaFilesCountRequest",
                    rinf::serialize(&GetMediaFilesCountRequest { call_id: None })?,
                ),
                (
                    "FetchAllPlaylistsRequest",
                    rinf::serialize(&FetchAllPlaylistsRequest { call_id: None })?,
                ),
                (
                    "SearchForRequest",
                    rinf::serialize(&SearchForRequest {
                        call_id: None,
                        query_str: "a".into(),
                        fields: Vec::new(),
                        n: 10,
                    })?,
                ),
            ] {
                let (name, reply) =
                    socket_request_for_library(socket, request_name, &payload, library_id).await?;
                assert_eq!(name, "RemoteRequestFailed", "{request_name} on {library_id}");
                let failure: RemoteRequestFailed = rinf::deserialize(&reply)?;
                assert_eq!(
                    failure.kind,
                    RequestErrorKind::PermissionDenied,
                    "{request_name} on {library_id}"
                );
            }
            Ok(())
        }

        assert_catalog_allowed(&mut socket, shared_id).await?;
        assert_catalog_denied(&mut socket, unshared_id).await?;
        assert_catalog_denied(&mut socket, later_id).await?;

        // Sync remains None on the shared library and forbidden on unshared libraries.
        for library_id in [shared_id, unshared_id, later_id] {
            let sync_response = pair
                .client
                .post(format!(
                    "https://{}/libraries/{library_id}/sync/metadata",
                    pair.addr
                ))
                .json(&serde_json::json!({"operation": "identity"}))
                .send()
                .await?;
            assert_eq!(sync_response.status(), StatusCode::FORBIDDEN);
        }

        // Explicitly share `later_id` as well; `unshared_id` remains hidden.
        save_library_grants(vec![
            DeviceLibraryGrant {
                library_id: shared_id.to_string(),
                connection: DeviceConnectionLevel::View,
                sync: DeviceSyncLevel::None,
            },
            DeviceLibraryGrant {
                library_id: later_id.to_string(),
                connection: DeviceConnectionLevel::View,
                sync: DeviceSyncLevel::None,
            },
        ])
        .await?;
        let mut expected_two = vec![
            (
                shared_id.to_string(),
                "Shared Music".to_owned(),
                "View".to_owned(),
                "None".to_owned(),
            ),
            (
                later_id.to_string(),
                "Added Later Secret".to_owned(),
                "View".to_owned(),
                "None".to_owned(),
            ),
        ];
        expected_two.sort();
        assert_eq!(listed_pairs().await?, expected_two);
        assert_catalog_allowed(&mut socket, later_id).await?;
        assert_catalog_denied(&mut socket, unshared_id).await?;

        // Revoke `shared_id` (`Not Shared`); visibility and catalog access drop immediately,
        // and `/access` does not leak `shared_id` or `unshared_id`.
        save_library_grants(vec![
            DeviceLibraryGrant {
                library_id: shared_id.to_string(),
                connection: DeviceConnectionLevel::None,
                sync: DeviceSyncLevel::None,
            },
            DeviceLibraryGrant {
                library_id: later_id.to_string(),
                connection: DeviceConnectionLevel::View,
                sync: DeviceSyncLevel::None,
            },
        ])
        .await?;
        assert_eq!(
            listed_pairs().await?,
            vec![(
                later_id.to_string(),
                "Added Later Secret".to_owned(),
                "View".to_owned(),
                "None".to_owned(),
            )],
        );
        let access_body: serde_json::Value = pair
            .client
            .get(format!("https://{}/access", pair.addr))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let access_text = access_body.to_string();
        assert!(!access_text.contains(&shared_id.to_string()));
        assert!(!access_text.contains(&unshared_id.to_string()));
        assert_catalog_denied(&mut socket, shared_id).await?;
        assert_catalog_allowed(&mut socket, later_id).await?;

        // Re-applying the pure Guest preset hides every library.
        preset_request
            .handle(
                preset_request.extract_params(&pair.a.global_params),
                None,
                &preset_request,
            )
            .await?;
        assert_eq!(listed_pairs().await?, Vec::new());
        assert_catalog_denied(&mut socket, later_id).await?;

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
        existing
            .connection
            .libraries
            .insert(pair.id, ConnectionLevel::View);
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
            &rinf::serialize(&PauseRequest { call_id: None })?,
        )
        .await?;
        assert_ne!(name, "RemoteRequestFailed");
        let (name, payload) = socket_request_for_library(
            &mut socket,
            "GetMediaFilesCountRequest",
            &rinf::serialize(&GetMediaFilesCountRequest { call_id: None })?,
            pair.id,
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
        let (name, _) = socket_request_for_library(
            &mut socket,
            "CreatePlaylistRequest",
            &rinf::serialize(&CreatePlaylistRequest {
                call_id: None,
                name: "axis edit".into(),
                group: "test".into(),
            })?,
            pair.id,
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
            StatusCode::FORBIDDEN
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
        let (name, payload) = socket_request_for_library(
            &mut socket,
            "GetMediaFilesCountRequest",
            &rinf::serialize(&GetMediaFilesCountRequest { call_id: None })?,
            pair.id,
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
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
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
        assert_eq!(response.status(), StatusCode::OK);
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
        let (name, payload) = socket_request_for_library(
            &mut socket,
            "GetMediaFilesCountRequest",
            &rinf::serialize(&GetMediaFilesCountRequest { call_id: None })?,
            pair.id,
        )
        .await?;
        assert_eq!(name, "RemoteRequestFailed");
        let failure: RemoteRequestFailed = rinf::deserialize(&payload)?;
        assert_eq!(failure.kind, RequestErrorKind::PermissionDenied);
        assert_eq!(
            tokio_time::timeout(TEST_DEADLINE, request).await??.status(),
            StatusCode::FORBIDDEN
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
            "DetectOpenDalShareDataRequest" => RequestAccess::NodeLocal,
            "OpenLibraryRequest" => RequestAccess::NodeLocal,
            "InitializeNodeRequest" => RequestAccess::NodeLocal,
            "RunFsSelfTestRequest" => RequestAccess::NodeLocal,
            "ListFileDialogPlacesRequest" => RequestAccess::NodeLocal,
            "ResolveFileDialogPathRequest" => RequestAccess::NodeLocal,
            "ListFileDialogFolderRequest" => RequestAccess::NodeLocal,
            "FetchFileDialogThumbnailRequest" => RequestAccess::NodeLocal,
            "CreateFileDialogFolderRequest" => RequestAccess::NodeLocal,
            "RenameFileDialogEntryRequest" => RequestAccess::NodeLocal,
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
            "PauseReportingRequest" => RequestAccess::Control,
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
            "LoadResolvedItemsRequest" | "HandoffLoadRequest" | "TakeBackHandoffBlockRequest" => {
                RequestAccess::Control
            }
            "PlaylistRevisionRequest" => RequestAccess::Control,
            "CastResolvedItemsRequest" | "RemotePlaylistRevisionRequest" => RequestAccess::Control,
            "CastPreflightRequest" => RequestAccess::Control,
            "SessionSnapshotRequest" | "RefreshSessionRequest" => RequestAccess::Control,
            "SetLikedRequest" => RequestAccess::ConnectionEdit,
            "GetLikedRequest" => RequestAccess::ConnectionView,
            "ComplexQueryRequest" => RequestAccess::ConnectionView,
            "SearchForRequest" => RequestAccess::ConnectionView,
            "FetchArtistPageRequest" => RequestAccess::ConnectionView,
            "FetchArtistSectionAlbumsRequest" => RequestAccess::ConnectionView,
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
            "ListPairedServersRequest" => RequestAccess::NodeLocal,
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
            "GetOpenLibraryRequest" => RequestAccess::Control,
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
            assert_ne!(
                request_access(name),
                Some(RequestAccess::NodeLocal),
                "{name}"
            );
        }
    }

    #[test]
    fn every_request_body_that_names_a_library_declares_its_rule() {
        for &(name, declared, names_library) in body_library_declarations() {
            if names_library {
                assert_ne!(
                    declared,
                    BodyLibraries::NoLibrary,
                    "{name}'s body names a library, but its catalogue entry declares no rule"
                );
            } else {
                assert_eq!(
                    declared,
                    BodyLibraries::NoLibrary,
                    "{name}'s body names no library, but its catalogue entry declares one"
                );
            }
            if declared == BodyLibraries::Authorized {
                assert_eq!(request_scope(name), Some(RequestScope::Library), "{name}");
            }
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
        request
            .handle(request.extract_params(&a.global_params), None, &request)
            .await?
            .context("approval")?;
        assert_eq!(
            a.global_params.node.access_policy.grants(&fingerprint)?,
            DeviceGrants::guest()
        );
        assert_eq!(
            DeviceGrants::guest().connection_level(Uuid::new_v4()),
            ConnectionLevel::None
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
            DeviceConnectionLevel::None
        ));
        assert!(matches!(
            response.libraries[0].sync,
            DeviceSyncLevel::None
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

    #[derive(Clone, Copy)]
    enum TrustMutation {
        Block,
        #[cfg(unix)]
        Remove,
    }

    impl TrustMutation {
        async fn apply(
            self,
            a: &Fixture,
            fingerprint: &str,
        ) -> Result<(), ::discovery::server::PermissionError> {
            let permissions = a.global_params.permission_manager.write().await;
            match self {
                Self::Block => {
                    permissions
                        .change_user_status(fingerprint, UserStatus::Blocked)
                        .await
                }
                #[cfg(unix)]
                Self::Remove => permissions.remove_user(fingerprint).await,
            }
        }

        /// What the trust list shows for the device once the mutation has taken effect.
        #[cfg(unix)]
        fn expected_status(self) -> Option<UserStatus> {
            match self {
                Self::Block => Some(UserStatus::Blocked),
                Self::Remove => None,
            }
        }
    }

    /// A node whose single approved device is restricted to the guest preset.
    async fn restricted_device_node(dir: &Path, fingerprint: &str) -> Result<Fixture> {
        let a = fixture(dir).await?;
        let permissions = a.global_params.permission_manager.write().await;
        permissions
            .add_user(
                "key".into(),
                fingerprint.into(),
                "alias".into(),
                "test".into(),
                DeviceType::Desktop,
                "127.0.0.3".into(),
            )
            .await?;
        permissions
            .change_user_status(fingerprint, UserStatus::Approved)
            .await?;
        drop(permissions);
        a.global_params
            .node
            .access_policy
            .set(fingerprint, DeviceGrants::guest())?;
        Ok(a)
    }

    async fn trust_status(a: &Fixture, fingerprint: &str) -> Option<UserStatus> {
        a.global_params
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(fingerprint)
            .await
            .map(|user| user.status)
    }

    /// Makes `path` read-only and proves it: a process that may write anyway (such as root)
    /// would otherwise let the failure scenarios pass without exercising a failed write.
    #[cfg(unix)]
    fn make_unwritable(path: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o444))?;
        assert!(
            fs::OpenOptions::new().write(true).open(path).is_err(),
            "{} is still writable; run this test as an unprivileged user",
            path.display()
        );
        Ok(())
    }

    #[cfg(unix)]
    fn make_writable(path: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o644))?;
        Ok(())
    }

    /// Reopens the trust list and grants from disk, as a restart would.
    #[cfg(unix)]
    async fn reload_policy(
        config: &Path,
    ) -> Result<(PermissionManager, crate::access_policy::RuneAccessPolicy)> {
        let permissions = PermissionManager::new(config)?;
        let policy =
            crate::access_policy::RuneAccessPolicy::load(config, &permissions.list_users().await)?;
        Ok((permissions, policy))
    }

    #[cfg(unix)]
    fn peer(fingerprint: &str) -> ::discovery::access_policy::PeerIdentity {
        ::discovery::access_policy::PeerIdentity {
            fingerprint: fingerprint.into(),
            public_key: "key".into(),
        }
    }

    #[cfg(unix)]
    async fn failed_trust_save_keeps_restricted_grants(mutation: TrustMutation) -> Result<()> {
        use ::discovery::access_policy::AccessPolicy;

        let dir = tempfile::tempdir()?;
        let fingerprint = "restricted-device";
        let a = restricted_device_node(dir.path(), fingerprint).await?;
        let trust_file = dir.path().join(".known-clients");
        let grants_file = dir.path().join("grants.json");
        let trust_before = fs::read(&trust_file)?;
        let grants_before = fs::read(&grants_file)?;

        make_unwritable(&trust_file)?;
        assert!(mutation.apply(&a, fingerprint).await.is_err());

        assert_eq!(
            trust_status(&a, fingerprint).await,
            Some(UserStatus::Approved)
        );
        assert_eq!(
            a.global_params.node.access_policy.grants(fingerprint)?,
            DeviceGrants::guest()
        );
        assert_eq!(fs::read(&trust_file)?, trust_before);
        assert_eq!(fs::read(&grants_file)?, grants_before);

        make_writable(&trust_file)?;
        let (_reloaded_permissions, reloaded_policy) = reload_policy(dir.path()).await?;
        assert_eq!(reloaded_policy.grants(fingerprint)?, DeviceGrants::guest());
        assert!(
            reloaded_policy
                .check(
                    &peer(fingerprint),
                    &Action::ConnectionEdit {
                        library_id: Uuid::new_v4()
                    }
                )
                .is_err()
        );

        mutation
            .apply(&a, fingerprint)
            .await
            .context("retry once the trust file is writable again")?;
        assert_eq!(
            trust_status(&a, fingerprint).await,
            mutation.expected_status()
        );
        assert!(
            a.global_params
                .node
                .access_policy
                .grants(fingerprint)
                .is_err()
        );
        a.global_params.node.shutdown().await
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn blocking_a_device_keeps_its_restricted_grants_when_the_trust_file_cannot_be_written()
    -> Result<()> {
        failed_trust_save_keeps_restricted_grants(TrustMutation::Block).await
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn removing_a_device_keeps_its_restricted_grants_when_the_trust_file_cannot_be_written()
    -> Result<()> {
        failed_trust_save_keeps_restricted_grants(TrustMutation::Remove).await
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn approving_a_device_grants_nothing_when_the_trust_file_cannot_be_written() -> Result<()>
    {
        use ::discovery::access_policy::AccessPolicy;

        let dir = tempfile::tempdir()?;
        let fingerprint = "pending-device";
        let a = fixture(dir.path()).await?;
        a.global_params
            .permission_manager
            .write()
            .await
            .add_user(
                "key".into(),
                fingerprint.into(),
                "alias".into(),
                "test".into(),
                DeviceType::Desktop,
                "127.0.0.3".into(),
            )
            .await?;
        let trust_file = dir.path().join(".known-clients");
        let trust_before = fs::read(&trust_file)?;

        make_unwritable(&trust_file)?;
        let approval = a
            .global_params
            .permission_manager
            .write()
            .await
            .change_user_status(fingerprint, UserStatus::Approved)
            .await;
        assert!(approval.is_err());

        assert_eq!(
            trust_status(&a, fingerprint).await,
            Some(UserStatus::Pending)
        );
        assert!(
            a.global_params
                .node
                .access_policy
                .grants(fingerprint)
                .is_err()
        );
        assert_eq!(fs::read(&trust_file)?, trust_before);

        make_writable(&trust_file)?;
        let (reloaded_permissions, reloaded_policy) = reload_policy(dir.path()).await?;
        assert_eq!(
            reloaded_permissions
                .verify_by_fingerprint(fingerprint)
                .await
                .map(|user| user.status),
            Some(UserStatus::Pending)
        );
        assert!(reloaded_policy.grants(fingerprint).is_err());
        assert!(
            reloaded_policy
                .check(
                    &peer(fingerprint),
                    &Action::ConnectionView {
                        library_id: Uuid::new_v4()
                    }
                )
                .is_err()
        );
        a.global_params.node.shutdown().await
    }

    #[tokio::test]
    async fn failed_grant_save_leaves_the_device_approved_and_can_be_retried() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let fingerprint = "restricted-device";
        let a = restricted_device_node(dir.path(), fingerprint).await?;
        let trust_file = dir.path().join(".known-clients");
        let grants_file = dir.path().join("grants.json");
        let trust_before = fs::read(&trust_file)?;
        let grants_before = fs::read(&grants_file)?;

        let blocker = dir.path().join("grants.json.tmp");
        fs::create_dir(&blocker)?;
        assert!(
            TrustMutation::Block
                .apply(&a, fingerprint)
                .await
                .is_err()
        );

        assert_eq!(
            trust_status(&a, fingerprint).await,
            Some(UserStatus::Approved)
        );
        assert_eq!(
            a.global_params.node.access_policy.grants(fingerprint)?,
            DeviceGrants::guest()
        );
        assert_eq!(fs::read(&trust_file)?, trust_before);
        assert_eq!(fs::read(&grants_file)?, grants_before);

        fs::remove_dir(&blocker)?;
        TrustMutation::Block
            .apply(&a, fingerprint)
            .await
            .context("retry once the grants file is writable again")?;
        assert_eq!(
            trust_status(&a, fingerprint).await,
            Some(UserStatus::Blocked)
        );
        assert!(
            a.global_params
                .node
                .access_policy
                .grants(fingerprint)
                .is_err()
        );
        a.global_params.node.shutdown().await
    }
}
