//! Deterministic business commands interleaved with bounded metadata sessions.
use anyhow::{Context, Result, ensure};
use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use database::{
    actions::{mixes, playlists, stats},
    entities::{media_file_stats, media_files, playlists as playlist_rows},
    sync::{LibrarySyncStore, model::RuneModel},
};
use http_request::sync::{MetadataPeer, MetadataStore, router};
use migration::{Migrator, MigratorTrait};
use sea_orm::{ColumnTrait, Database, DatabaseConnection, EntityTrait, QueryFilter, Set};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use sync::{Coordinator, SessionAdvance, SyncModel, SyncPeer};
use uuid::Uuid;

struct Replica {
    db: DatabaseConnection,
    library: LibrarySyncStore,
    actor: Uuid,
    url: String,
    client: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
    lose_reply: Arc<AtomicBool>,
    lost: Arc<AtomicUsize>,
    retry: Arc<AtomicBool>,
    gate: Arc<AtomicBool>,
    gate_head: Arc<AtomicBool>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    _directory: tempfile::TempDir,
}
impl Drop for Replica {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[derive(Clone)]
struct Fault {
    enabled: Arc<AtomicBool>,
    count: Arc<AtomicUsize>,
    retry: Arc<AtomicBool>,
    gate: Arc<AtomicBool>,
    gate_head: Arc<AtomicBool>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
async fn lose_first_page(State(fault): State<Fault>, request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, 9 * 1024 * 1024).await.unwrap();
    let operation = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| {
            v.get("operation")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        });
    if (operation.as_deref() == Some("read_page") && fault.gate.swap(false, Ordering::SeqCst))
        || (operation.as_deref() == Some("head") && fault.gate_head.swap(false, Ordering::SeqCst))
    {
        fault.entered.notify_one();
        fault.release.notified().await;
    }
    let response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    if operation.as_deref() == Some("read_page")
        && response.status().is_success()
        && fault.enabled.swap(false, Ordering::SeqCst)
    {
        fault.count.fetch_add(1, Ordering::SeqCst);
        fault.retry.store(true, Ordering::SeqCst);
        return (StatusCode::BAD_GATEWAY, "injected lost response").into_response();
    }
    response
}
impl Replica {
    async fn new(index: u128, namespace: Uuid) -> Result<Self> {
        Self::new_with_pool(index, namespace, 1).await
    }
    async fn new_with_pool(index: u128, namespace: Uuid, pool_size: u32) -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let url = format!(
            "sqlite://{}?mode=rwc",
            directory.path().join("library.db").display()
        );
        let mut options = sea_orm::ConnectOptions::new(url);
        options
            .max_connections(pool_size)
            .min_connections(pool_size)
            .idle_timeout(None)
            .max_lifetime(None);
        let db = Database::connect(options).await?;
        Migrator::up(&db, None).await?;
        let actor = Uuid::from_u128(1000 + index);
        let library = LibrarySyncStore::open(db.clone(), actor, Some(namespace)).await?;
        let lose_reply = Arc::new(AtomicBool::new(false));
        let lost = Arc::new(AtomicUsize::new(0));
        let retry = Arc::new(AtomicBool::new(false));
        let gate = Arc::new(AtomicBool::new(false));
        let gate_head = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let peer: Arc<dyn MetadataStore<RuneModel>> = Arc::new(library.endpoint());
        let app = router(peer).layer(middleware::from_fn_with_state(
            Fault {
                enabled: lose_reply.clone(),
                count: lost.clone(),
                retry: retry.clone(),
                gate: gate.clone(),
                gate_head: gate_head.clone(),
                entered: entered.clone(),
                release: release.clone(),
            },
            lose_first_page,
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Ok(Self {
            db,
            library,
            actor,
            url,
            client: reqwest::Client::new(),
            server,
            lose_reply,
            lost,
            retry,
            gate,
            gate_head,
            entered,
            release,
            _directory: directory,
        })
    }
    fn peer(&self) -> MetadataPeer<RuneModel> {
        MetadataPeer::new(&self.url, self.client.clone())
    }
    async fn file(&self, uuid: Uuid) -> Result<i32> {
        Ok(media_files::Entity::find()
            .filter(media_files::Column::HlcUuid.eq(uuid.to_string()))
            .one(&self.db)
            .await?
            .context("track is missing")?
            .id)
    }
    async fn playlist(&self, uuid: Uuid) -> Result<Option<playlist_rows::Model>> {
        Ok(playlists::get_all_playlists(&self.db)
            .await?
            .into_iter()
            .find(|row| row.hlc_uuid == uuid.to_string()))
    }
}
fn coordinator() -> Result<Coordinator<RuneModel>> {
    let scope = sync::Scope {
        collections: RuneModel::collections().into_iter().collect(),
    };
    let mut limits = database::sync::session_limits();
    limits.pages.max_scanned = 3;
    limits.apply.max_projection_items = 3;
    limits.work.max_requests = 8;
    Ok(Coordinator::new(scope, limits)?)
}
async fn step(
    session: &mut Coordinator<RuneModel>,
    source: &Replica,
    target: &Replica,
) -> Result<bool> {
    let endpoint = target.library.endpoint();
    let peer = source.peer();
    match session
        .advance(&endpoint, &endpoint, &endpoint, &peer, &peer)
        .await
    {
        Ok(SessionAdvance::Complete(_)) => Ok(true),
        Ok(SessionAdvance::BudgetExhausted) => {
            session.start_pass();
            Ok(false)
        }
        Ok(SessionAdvance::Incomplete) => Ok(false),
        Ok(SessionAdvance::Cancelled) => anyhow::bail!("unexpected cancellation"),
        Err(sync::SyncError::Transport(_)) if source.retry.swap(false, Ordering::SeqCst) => {
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}
async fn finish(
    session: &mut Coordinator<RuneModel>,
    source: &Replica,
    target: &Replica,
) -> Result<usize> {
    for count in 0..20000 {
        if step(session, source, target).await? {
            return Ok(count + 1);
        }
        if count % 8 == 0 {
            check_reads(target).await?;
        }
    }
    anyhow::bail!("session did not complete")
}
async fn exchange(source: &Replica, target: &Replica) -> Result<()> {
    finish(&mut coordinator()?, source, target).await?;
    Ok(())
}
async fn check_reads(replica: &Replica) -> Result<()> {
    for row in playlists::get_all_playlists(&replica.db).await? {
        let page = playlists::list_occurrences(&replica.db, row.id, None, 128).await?;
        let mut previous = None;
        for occurrence in page.entries {
            ensure!(
                occurrence.file.is_some(),
                "a visible occurrence lost its track"
            );
            let current = (occurrence.position, occurrence.occurrence_id);
            ensure!(
                previous.as_ref().is_none_or(|prior| prior < &current),
                "occurrences are out of order"
            );
            previous = Some(current);
        }
    }
    for file in database::actions::file::get_media_files(&replica.db, 0, 128).await? {
        let _ = stats::get_liked(&replica.db, file.id).await?;
    }
    for mix in mixes::get_all_mixes(&replica.db).await? {
        let _ = mixes::get_mix_queries_by_mix_id(&replica.db, mix.id).await?;
    }
    for row in media_file_stats::Entity::find().all(&replica.db).await? {
        ensure!(
            row.skipped >= 0 && row.played_through >= 0,
            "negative statistic"
        );
    }
    Ok(())
}
async fn heads(replicas: &[Replica]) -> Result<Vec<sync::ReceiveProgress>> {
    let mut result = Vec::new();
    for replica in replicas {
        result.push(replica.peer().head().await?);
    }
    Ok(result)
}

#[tokio::test]
async fn business_mutations_during_partitioned_http_sessions_converge() -> Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(600), business_scenario())
        .await
        .context("business scenario exceeded its deadline")?
}

async fn business_scenario() -> Result<()> {
    eprintln!("business: seed and initial exchange");
    let namespace = Uuid::from_u128(0xabc);
    let _ = migration::initialize_node_id(Uuid::from_u128(1000).to_string());
    let mut replicas = Vec::new();
    for index in 0..4 {
        replicas.push(Replica::new(index, namespace).await?);
    }
    let tracks = [Uuid::from_u128(101), Uuid::from_u128(102)];
    for (index, id) in tracks.iter().enumerate() {
        media_files::Entity::insert(media_files::ActiveModel {
            id: Set(index as i32 + 1),
            hlc_uuid: Set(id.to_string()),
            file_name: Set(format!("track-{index}.ogg")),
            directory: Set(String::new()),
            extension: Set("ogg".into()),
            file_hash: Set(format!("seed-{index}")),
            last_modified: Set("seed".into()),
            sample_rate: Set(44100),
            duration: Set(1.into()),
            local_available: Set(true),
            ..Default::default()
        })
        .exec(&replicas[0].db)
        .await?;
    }
    replicas[0]
        .library
        .import_existing(replicas[0].actor)
        .await?;
    for target in replicas.iter().skip(1) {
        exchange(&replicas[0], target).await?;
    }
    for replica in &replicas {
        assert!(replica.file(tracks[0]).await? > 0);
        assert!(replica.file(tracks[1]).await? > 0);
        check_reads(replica).await?;
    }

    eprintln!("business: interleaved commands and isolated partitions");
    let a = &replicas[0];
    let b = &replicas[1];
    let c = &replicas[2];
    let d = &replicas[3];
    let a_store = a.library.record_store();
    let b_store = b.library.record_store();
    let c_store = c.library.record_store();
    let d_store = d.library.record_store();
    let a1 = a.file(tracks[0]).await?;
    let shared = playlists::create_playlist(
        &a.db,
        &a_store,
        a.actor,
        "shared original".into(),
        "s".into(),
    )
    .await?;
    let shared_uuid: Uuid = shared.hlc_uuid.parse()?;
    let removed =
        playlists::add_item_to_playlist(&a.db, &a_store, a.actor, shared.id, a1, None).await?;
    let mut in_flight = coordinator()?;
    assert!(
        !step(&mut in_flight, a, b).await?,
        "fixture must pause an unfinished session"
    );
    a.gate.store(true, Ordering::SeqCst);
    a.lose_reply.store(true, Ordering::SeqCst);
    let kept = tokio::time::timeout(std::time::Duration::from_secs(90), async {
    let mut kept = None;
    for _ in 0..20000 {
        let mut request = Box::pin(step(&mut in_flight, a, b));
        tokio::select! {
            result = &mut request => {
                ensure!(!result?, "session completed before the held request");
                check_reads(b).await?;
            }
            _ = a.entered.notified() => {
                // The HTTP request is waiting at the source while these commands commit.
                let commands: Result<_> = async {
                    let entry = playlists::add_item_to_playlist(&a.db, &a_store, a.actor, shared.id, a1, None).await?;
                    assert_ne!(removed.hlc_uuid, entry.hlc_uuid);
                    stats::set_liked(&a.db, &a_store, a1, true).await?;
                    stats::increase_skipped(&a.db, &a_store, a1, a.actor).await?;
                    assert!(stats::get_liked(&a.db, a1).await?);
                    assert_eq!(playlists::list_occurrences(&a.db, shared.id, None, 128).await?.entries.len(), 2);
                    Ok(entry)
                }.await;
                a.release.notify_one();
                let _ = request.await?;
                kept = Some(commands?);
                break;
            }
        }
    }
    kept.context("the held request was never reached")
    }).await.context("held request or commands timed out")??;
    finish(&mut in_flight, a, b).await?;
    // The first session may have opened its source view before the later commands.
    exchange(a, b).await?;
    assert_eq!(
        a.lost.load(Ordering::SeqCst),
        1,
        "lost page response was not exercised"
    );
    let shared_b = b
        .playlist(shared_uuid)
        .await?
        .context("shared playlist missing")?;
    assert_eq!(
        playlists::list_occurrences(&b.db, shared_b.id, None, 128)
            .await?
            .entries
            .len(),
        2
    );
    let b1 = b.file(tracks[0]).await?;
    let b2 = b.file(tracks[1]).await?;
    let mut while_b_writes = coordinator()?;
    assert!(!step(&mut while_b_writes, b, a).await?);
    playlists::update_playlist(
        &b.db,
        &b_store,
        b.actor,
        shared_b.id,
        Some("shared final".into()),
        None,
    )
    .await?;
    playlists::remove_item_from_playlist(
        &b.db,
        &b_store,
        b.actor,
        shared_b.id,
        removed.hlc_uuid.parse()?,
    )
    .await?;
    playlists::reorder_playlist_item_position(
        &b.db,
        &b_store,
        b.actor,
        shared_b.id,
        kept.hlc_uuid.parse()?,
        5,
    )
    .await?;
    let extra =
        playlists::add_item_to_playlist(&b.db, &b_store, b.actor, shared_b.id, b2, Some(2)).await?;
    stats::increase_played_through(&b.db, &b_store, b1, b.actor).await?;
    let local_entries = playlists::list_occurrences(&b.db, shared_b.id, None, 128)
        .await?
        .entries;
    assert_eq!(
        local_entries
            .iter()
            .map(|row| row.track_id)
            .collect::<Vec<_>>(),
        [tracks[1], tracks[0]]
    );
    finish(&mut while_b_writes, b, a).await?;
    assert!(
        c.playlist(shared_uuid).await?.is_none(),
        "partition unexpectedly delivered a playlist"
    );

    // C and D form the other partition and modify distinct objects while A and B remain isolated.
    let c2 = c.file(tracks[1]).await?;
    let d1 = d.file(tracks[0]).await?;
    let transient =
        playlists::create_playlist(&c.db, &c_store, c.actor, "remove me".into(), "x".into())
            .await?;
    let transient_uuid: Uuid = transient.hlc_uuid.parse()?;
    playlists::update_playlist(
        &c.db,
        &c_store,
        c.actor,
        transient.id,
        Some("renamed then removed".into()),
        None,
    )
    .await?;
    let retained =
        playlists::create_playlist(&d.db, &d_store, d.actor, "remote list".into(), "r".into())
            .await?;
    let retained_uuid: Uuid = retained.hlc_uuid.parse()?;
    let retained_entry =
        playlists::add_item_to_playlist(&d.db, &d_store, d.actor, retained.id, d1, None).await?;
    let mix = mixes::create_mix(
        &c.db,
        &c_store,
        &c.actor.to_string(),
        "temporary mix".into(),
        "m".into(),
        false,
        0,
        false,
        vec![("lib::track".into(), c2.to_string())],
    )
    .await?;
    mixes::update_mix(
        &c.db,
        &c_store,
        &c.actor.to_string(),
        mix.id,
        Some("updated mix".into()),
        None,
        None,
        None,
        None,
        None,
    )
    .await?;
    let dangling_mix = mixes::create_mix(
        &c.db,
        &c_store,
        &c.actor.to_string(),
        "missing reference".into(),
        "m".into(),
        false,
        0,
        false,
        vec![("lib::playlist".into(), transient.id.to_string())],
    )
    .await?;
    let dangling_uuid = dangling_mix.hlc_uuid.clone();
    let removed_mix_uuid = mix.hlc_uuid.clone();
    stats::set_liked(&c.db, &c_store, c2, true).await?;
    stats::increase_skipped(&c.db, &c_store, c2, c.actor).await?;
    stats::increase_played_through(&c.db, &c_store, c2, c.actor).await?;
    stats::increase_skipped(&d.db, &d_store, d1, d.actor).await?;
    let mut cd = coordinator()?;
    assert!(!step(&mut cd, c, d).await?);
    playlists::remove_playlist(&c.db, &c_store, transient.id).await?;
    mixes::remove_mix(&c.db, &c_store, mix.id).await?;
    assert!(c.playlist(transient_uuid).await?.is_none());
    assert!(
        !mixes::get_all_mixes(&c.db)
            .await?
            .iter()
            .any(|row| row.hlc_uuid == removed_mix_uuid)
    );
    let views = mixes::get_mix_queries_by_mix_id(&c.db, dangling_mix.id).await?;
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].target_uuid, Some(transient_uuid));
    assert!(!views[0].available);
    assert!(
        mixes::executable_mix_queries(&c.db, dangling_mix.id)
            .await
            .is_err()
    );
    finish(&mut cd, c, d).await?;
    exchange(c, d).await?;
    exchange(d, c).await?;
    assert!(
        a.playlist(retained_uuid).await?.is_none(),
        "partition unexpectedly delivered the second playlist"
    );
    assert!(stats::get_liked(&c.db, c2).await?);

    eprintln!("business: reconnect and all-pair exchange");
    // Reconnect through a chain and then exercise every directed pair.
    for round in 0..3 {
        eprintln!("chain round {}", round + 1);
        let before = heads(&replicas).await?;
        for (from, to) in [(0, 1), (1, 0), (1, 2), (2, 1), (2, 3), (3, 2)] {
            exchange(&replicas[from], &replicas[to]).await?;
        }
        if heads(&replicas).await? == before {
            break;
        }
    }
    for from in 0..4 {
        for to in 0..4 {
            if from != to {
                exchange(&replicas[from], &replicas[to]).await?;
            }
        }
    }
    for replica in &replicas {
        check_reads(replica).await?;
        let rows = playlists::get_all_playlists(&replica.db).await?;
        assert_eq!(rows.len(), 2);
        let row = replica
            .playlist(shared_uuid)
            .await?
            .context("shared playlist absent after reunion")?;
        assert_eq!(row.name, "shared final");
        assert!(replica.playlist(transient_uuid).await?.is_none());
        let entries = playlists::list_occurrences(&replica.db, row.id, None, 128)
            .await?
            .entries;
        assert_eq!(
            entries.iter().map(|row| row.track_id).collect::<Vec<_>>(),
            [tracks[1], tracks[0]]
        );
        assert_eq!(
            entries
                .iter()
                .map(|row| row.occurrence_id.as_str())
                .collect::<Vec<_>>(),
            [extra.hlc_uuid.as_str(), kept.hlc_uuid.as_str()]
        );
        let retained_row = replica
            .playlist(retained_uuid)
            .await?
            .context("retained playlist absent")?;
        let entries = playlists::list_occurrences(&replica.db, retained_row.id, None, 128)
            .await?
            .entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].occurrence_id, retained_entry.hlc_uuid);
        assert_eq!(entries[0].track_id, tracks[0]);
        let all_mixes = mixes::get_all_mixes(&replica.db).await?;
        assert!(!all_mixes.iter().any(|row| row.hlc_uuid == removed_mix_uuid));
        let dangling = all_mixes
            .iter()
            .find(|row| row.hlc_uuid == dangling_uuid)
            .context("missing-reference mix is absent")?;
        let views = mixes::get_mix_queries_by_mix_id(&replica.db, dangling.id).await?;
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].target_uuid, Some(transient_uuid));
        assert!(!views[0].available);
        assert!(
            mixes::executable_mix_queries(&replica.db, dangling.id)
                .await
                .is_err()
        );
        assert!(stats::get_liked(&replica.db, replica.file(tracks[0]).await?).await?);
        assert!(stats::get_liked(&replica.db, replica.file(tracks[1]).await?).await?);
        let mut observed = BTreeMap::new();
        for row in media_file_stats::Entity::find().all(&replica.db).await? {
            observed.insert(
                (row.track_uuid, row.device_id),
                (row.skipped, row.played_through),
            );
        }
        let expected = BTreeMap::from([
            ((tracks[0].to_string(), a.actor.to_string()), (1, 0)),
            ((tracks[0].to_string(), b.actor.to_string()), (0, 1)),
            ((tracks[1].to_string(), c.actor.to_string()), (1, 1)),
            ((tracks[0].to_string(), d.actor.to_string()), (1, 0)),
        ]);
        assert_eq!(observed, expected);
    }
    eprintln!("business: verify idle all-pair exchange");
    let before = heads(&replicas).await?;
    for from in 0..4 {
        for to in 0..4 {
            if from != to {
                exchange(&replicas[from], &replicas[to]).await?;
            }
        }
    }
    assert_eq!(
        heads(&replicas).await?,
        before,
        "settled all-pair reads changed history"
    );
    Ok(())
}

#[tokio::test]
async fn repeated_partitioned_commands_preserve_unseen_edit_and_pagination() -> Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(600), repeated_scenario())
        .await
        .context("repeated scenario exceeded its deadline")?
}

async fn repeated_scenario() -> Result<()> {
    eprintln!("repeated: seed and initial exchange");
    let namespace = Uuid::from_u128(0xdef);
    let _ = migration::initialize_node_id(Uuid::from_u128(1000).to_string());
    let mut replicas = Vec::new();
    for index in 0..4 {
        replicas.push(Replica::new(index, namespace).await?);
    }
    let track = Uuid::from_u128(201);
    media_files::Entity::insert(media_files::ActiveModel {
        id: Set(1),
        hlc_uuid: Set(track.to_string()),
        file_name: Set("shared.ogg".into()),
        directory: Set(String::new()),
        extension: Set("ogg".into()),
        file_hash: Set("seed".into()),
        last_modified: Set("seed".into()),
        sample_rate: Set(44100),
        duration: Set(1.into()),
        local_available: Set(true),
        ..Default::default()
    })
    .exec(&replicas[0].db)
    .await?;
    let a = &replicas[0];
    let b = &replicas[1];
    a.library.import_existing(a.actor).await?;
    let common = playlists::create_playlist(
        &a.db,
        &a.library.record_store(),
        a.actor,
        "before partition".into(),
        "c".into(),
    )
    .await?;
    let common_uuid: Uuid = common.hlc_uuid.parse()?;
    for target in replicas.iter().skip(1) {
        exchange(a, target).await?;
    }
    let b_common = b
        .playlist(common_uuid)
        .await?
        .context("common playlist absent")?;
    let mut ab = coordinator()?;
    let mut ba = coordinator()?;
    let mut cd = coordinator()?;
    let mut dc = coordinator()?;
    assert!(!step(&mut ab, a, b).await?);
    assert!(!step(&mut ba, b, a).await?);
    assert!(!step(&mut cd, &replicas[2], &replicas[3]).await?);
    assert!(!step(&mut dc, &replicas[3], &replicas[2]).await?);
    eprintln!("repeated: unseen delete and edit");
    // Each edit observes the initial assignment, but neither sees the other command.
    playlists::remove_playlist(&a.db, &a.library.record_store(), common.id).await?;
    playlists::update_playlist(
        &b.db,
        &b.library.record_store(),
        b.actor,
        b_common.id,
        Some("unseen edit survives".into()),
        None,
    )
    .await?;
    assert!(a.playlist(common_uuid).await?.is_none());
    assert_eq!(
        b.playlist(common_uuid).await?.unwrap().name,
        "unseen edit survives"
    );

    let mut expected = BTreeMap::new();
    let mut kept_entries = Vec::new();
    let b_file = b.file(track).await?;
    for position in 0..7 {
        let entry = playlists::add_item_to_playlist(
            &b.db,
            &b.library.record_store(),
            b.actor,
            b_common.id,
            b_file,
            Some(position),
        )
        .await?;
        kept_entries.push(entry.hlc_uuid);
    }
    // A small page size guarantees several real continuation reads over duplicates.
    let mut cursor = None;
    let mut paged = Vec::new();
    loop {
        let page = playlists::list_occurrences(&b.db, b_common.id, cursor, 2).await?;
        paged.extend(page.entries.into_iter().map(|entry| entry.occurrence_id));
        if page.next.is_none() {
            break;
        }
        cursor = page.next;
    }
    assert_eq!(paged, kept_entries);

    eprintln!("repeated: six rounds of partitioned commands");
    for round in 0..6 {
        for (index, replica) in replicas.iter().enumerate() {
            let store = replica.library.record_store();
            let name = format!("replica-{index}-round-{round}");
            let row = playlists::create_playlist(
                &replica.db,
                &store,
                replica.actor,
                name.clone(),
                "r".into(),
            )
            .await?;
            let id: Uuid = row.hlc_uuid.parse()?;
            let final_name = format!("{name}-updated");
            playlists::update_playlist(
                &replica.db,
                &store,
                replica.actor,
                row.id,
                Some(final_name.clone()),
                None,
            )
            .await?;
            if round % 2 == 0 {
                playlists::remove_playlist(&replica.db, &store, row.id).await?;
            } else {
                expected.insert(id, final_name);
            }
            let file = replica.file(track).await?;
            stats::increase_skipped(&replica.db, &store, file, replica.actor).await?;
            if round % 2 == 0 {
                stats::increase_played_through(&replica.db, &store, file, replica.actor).await?;
            }
            stats::set_liked(&replica.db, &store, file, round % 2 == 1).await?;
            assert_eq!(stats::get_liked(&replica.db, file).await?, round % 2 == 1);
            check_reads(replica).await?;
        }
        // Advance only the two links that remain connected during the partition.
        if step(&mut ab, a, b).await? {
            ab = coordinator()?;
        }
        if step(&mut ba, b, a).await? {
            ba = coordinator()?;
        }
        if step(&mut cd, &replicas[2], &replicas[3]).await? {
            cd = coordinator()?;
        }
        if step(&mut dc, &replicas[3], &replicas[2]).await? {
            dc = coordinator()?;
        }
        for replica in &replicas {
            check_reads(replica).await?;
        }
    }
    eprintln!("repeated: reconnect and all-pair exchange");
    finish(&mut ab, a, b).await?;
    finish(&mut ba, b, a).await?;
    finish(&mut cd, &replicas[2], &replicas[3]).await?;
    finish(&mut dc, &replicas[3], &replicas[2]).await?;
    for round in 0..3 {
        eprintln!("chain round {}", round + 1);
        let before = heads(&replicas).await?;
        for (from, to) in [(0, 1), (1, 0), (1, 2), (2, 1), (2, 3), (3, 2)] {
            exchange(&replicas[from], &replicas[to]).await?;
        }
        if heads(&replicas).await? == before {
            break;
        }
    }
    for from in 0..4 {
        for to in 0..4 {
            if from != to {
                exchange(&replicas[from], &replicas[to]).await?;
            }
        }
    }
    for replica in &replicas {
        check_reads(replica).await?;
        let common = replica
            .playlist(common_uuid)
            .await?
            .context("unseen edit was lost")?;
        assert_eq!(common.name, "unseen edit survives");
        let rows = playlists::get_all_playlists(&replica.db).await?;
        let observed: BTreeMap<Uuid, String> = rows
            .into_iter()
            .filter(|row| row.hlc_uuid != common_uuid.to_string())
            .map(|row| Ok((row.hlc_uuid.parse()?, row.name)))
            .collect::<Result<_>>()?;
        assert_eq!(observed, expected);
        let mut cursor = None;
        let mut entries = Vec::new();
        loop {
            let page = playlists::list_occurrences(&replica.db, common.id, cursor, 2).await?;
            assert!(page.entries.iter().all(|entry| entry.track_id == track));
            entries.extend(page.entries.into_iter().map(|entry| entry.occurrence_id));
            if page.next.is_none() {
                break;
            }
            cursor = page.next;
        }
        assert_eq!(entries, kept_entries);
        let rows = media_file_stats::Entity::find().all(&replica.db).await?;
        let actual: BTreeMap<_, _> = rows
            .into_iter()
            .map(|row| (row.device_id, (row.skipped, row.played_through)))
            .collect();
        let counts: BTreeMap<_, _> = replicas
            .iter()
            .map(|r| (r.actor.to_string(), (6, 3)))
            .collect();
        assert_eq!(actual, counts);
        assert!(stats::get_liked(&replica.db, replica.file(track).await?).await?);
    }
    eprintln!("repeated: verify idle all-pair exchange");
    let stable = heads(&replicas).await?;
    for from in 0..4 {
        for to in 0..4 {
            if from != to {
                exchange(&replicas[from], &replicas[to]).await?;
            }
        }
    }
    assert_eq!(heads(&replicas).await?, stable, "settled history changed");
    Ok(())
}

#[tokio::test]
async fn concurrent_actions_and_http_while_writer_is_held() -> Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(180), concurrent_scenario())
        .await
        .context("concurrent scenario exceeded its deadline")?
}

async fn concurrent_scenario() -> Result<()> {
    use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait, sea_query::Expr};
    let namespace = Uuid::from_u128(0xbeef);
    let _ = migration::initialize_node_id(Uuid::from_u128(1000).to_string());
    let a = Replica::new_with_pool(0, namespace, 4).await?;
    let b = Replica::new_with_pool(1, namespace, 4).await?;
    let row =
        a.db.query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA journal_mode".to_owned(),
        ))
        .await?
        .context("missing journal mode")?;
    let journal: String = row.try_get("", "journal_mode")?;
    eprintln!("concurrent: pool_size=4 sqlite_journal_mode={journal}");
    let track = Uuid::from_u128(301);
    media_files::Entity::insert(media_files::ActiveModel {
        id: Set(1),
        hlc_uuid: Set(track.to_string()),
        file_name: Set("shared.ogg".into()),
        directory: Set(String::new()),
        extension: Set("ogg".into()),
        file_hash: Set("seed".into()),
        last_modified: Set("seed".into()),
        sample_rate: Set(44100),
        duration: Set(1.into()),
        local_available: Set(true),
        ..Default::default()
    })
    .exec(&a.db)
    .await?;
    let store = a.library.record_store();
    a.library.import_existing(a.actor).await?;
    let playlist =
        playlists::create_playlist(&a.db, &store, a.actor, "committed".into(), "c".into()).await?;
    let playlist_uuid: Uuid = playlist.hlc_uuid.parse()?;
    exchange(&a, &b).await?;
    let mut session = coordinator()?;
    assert!(!step(&mut session, &a, &b).await?);

    let tx = a.db.begin().await?;
    store.lock(&tx).await?;
    playlist_rows::Entity::update_many()
        .col_expr(playlist_rows::Column::Name, Expr::value("uncommitted"))
        .filter(playlist_rows::Column::Id.eq(playlist.id))
        .exec(&tx)
        .await?;
    a.gate_head.store(true, Ordering::SeqCst);
    let peer = a.peer();
    let head_request = tokio::spawn(async move { peer.head().await });
    tokio::time::timeout(std::time::Duration::from_secs(10), a.entered.notified())
        .await
        .context("metadata request did not reach source")?;
    assert!(!head_request.is_finished());

    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let stats_started = Arc::new(tokio::sync::Notify::new());
    let playlist_started = Arc::new(tokio::sync::Notify::new());
    let stats_action = {
        let db = a.db.clone();
        let store = store.clone();
        let barrier = barrier.clone();
        let started = stats_started.clone();
        let actor = a.actor;
        tokio::spawn(async move {
            barrier.wait().await;
            started.notify_one();
            stats::increase_skipped(&db, &store, 1, actor).await
        })
    };
    let playlist_action = {
        let db = a.db.clone();
        let store = store.clone();
        let barrier = barrier.clone();
        let started = playlist_started.clone();
        let actor = a.actor;
        let id = playlist.id;
        tokio::spawn(async move {
            barrier.wait().await;
            started.notify_one();
            playlists::update_playlist(&db, &store, actor, id, Some("after lock".into()), None)
                .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), barrier.wait())
        .await
        .context("business actions did not start")?;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(stats_started.notified(), playlist_started.notified());
    })
    .await
    .context("business actions were not polled")?;
    let visible = a
        .playlist(playlist_uuid)
        .await?
        .context("committed playlist disappeared")?;
    assert_eq!(visible.name, "committed", "uncommitted edit became visible");
    assert!(
        media_file_stats::Entity::find()
            .all(&a.db)
            .await?
            .is_empty()
    );
    assert!(!head_request.is_finished(), "request escaped the gate");
    assert!(
        !stats_action.is_finished(),
        "statistics action escaped the held writer"
    );
    assert!(
        !playlist_action.is_finished(),
        "playlist action escaped the held writer"
    );
    tx.rollback().await?;
    a.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(20), head_request)
        .await
        .context("metadata request did not finish")???;
    tokio::time::timeout(std::time::Duration::from_secs(20), stats_action)
        .await
        .context("statistics action did not finish")???;
    tokio::time::timeout(std::time::Duration::from_secs(20), playlist_action)
        .await
        .context("playlist action did not finish")???;
    assert_eq!(a.playlist(playlist_uuid).await?.unwrap().name, "after lock");
    let stats_rows = media_file_stats::Entity::find().all(&a.db).await?;
    assert_eq!(stats_rows.len(), 1);
    assert_eq!(
        (stats_rows[0].skipped, stats_rows[0].played_through),
        (1, 0)
    );

    let before = a.peer().head().await?;
    a.db.execute_unprepared("CREATE TRIGGER reject_test_journal BEFORE INSERT ON sync_journal BEGIN SELECT RAISE(ABORT, 'injected failure'); END").await?;
    assert!(
        stats::increase_skipped(&a.db, &store, 1, a.actor)
            .await
            .is_err()
    );
    a.db.execute_unprepared("DROP TRIGGER reject_test_journal")
        .await?;
    assert_eq!(a.peer().head().await?, before);
    assert_eq!(
        media_file_stats::Entity::find()
            .one(&a.db)
            .await?
            .unwrap()
            .skipped,
        1
    );
    finish(&mut session, &a, &b).await?;
    exchange(&a, &b).await?;
    exchange(&b, &a).await?;
    assert_eq!(b.playlist(playlist_uuid).await?.unwrap().name, "after lock");
    let observed = media_file_stats::Entity::find().all(&b.db).await?;
    assert_eq!(observed.len(), 1);
    assert_eq!((observed[0].skipped, observed[0].played_through), (1, 0));
    Ok(())
}
