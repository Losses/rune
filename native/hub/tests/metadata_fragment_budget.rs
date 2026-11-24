//! Persistent three-node HTTP router metadata sync regression coverage for large fragmented events.

use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::Request,
    http::{
        StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
    middleware::{self, Next},
    response::Response,
};
use database::sync::{
    LibrarySyncStore,
    model::{Collection, MetadataTag, ObjectKey, Payload, RuneModel, Track},
    session_limits,
};
use http_request::sync::router;
use migration::{Migrator, MigratorTrait, initialize_node_id};
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::result;
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use sync::{
    self as p, ApplyLimits, ApplyResult, BoxFuture, CandidateCursor, ChangePage, CheckpointKey,
    Coordinator, CoveragePage, CoverageProgress, Fragment, FragmentProgress, HistoryStatus,
    KeyRange, LeafPage, LeafProgress, ManifestRef, PageLimits, PageRead, PageRequest,
    ProjectionBatch, PromoteProgress, RangeSummary, ReceiveProgress, ReceiveStore, Scope,
    SessionAdvance, SessionLimits, SnapshotDescriptor, SnapshotSink, SnapshotSource,
    StageDescriptor, StageLimits, StageOrigin, StoreIdentity, SyncError, SyncModel, SyncPeer,
    WorkCursor, WorkLimits, WorkOutcome,
};
use tempfile::TempDir;
use tower::ServiceExt;
use uuid::Uuid;

const AUTH_HEADER: &str = "Bearer owned-rv355-fixture";
const REQUEST_BYTES: usize = 9 * 1024 * 1024;
const RESPONSE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug)]
struct WireExchange {
    operation: String,
    status: u16,
    request_bytes: usize,
    response_bytes: usize,
}

struct NodeHandle {
    db: DatabaseConnection,
    library: LibrarySyncStore,
}

impl NodeHandle {
    async fn open(root: &Path, actor: Uuid, namespace: Uuid) -> Result<Self> {
        fs::create_dir_all(root)?;
        let _ = initialize_node_id(actor.to_string());
        let sqlite_path = root.join("library.sqlite");
        let fresh = !sqlite_path.exists();
        if fresh {
            File::create(&sqlite_path)?;
        }
        let mut options = ConnectOptions::new(format!("sqlite://{}", sqlite_path.display()));
        options.max_connections(1).sqlx_logging(false);
        let db = Database::connect(options).await?;
        if fresh {
            Migrator::up(&db, None).await?;
        }
        let library = LibrarySyncStore::open(db.clone(), actor, Some(namespace)).await?;
        Ok(Self { db, library })
    }

    async fn close(self) -> Result<()> {
        self.library.clean_close().await?;
        drop(self.library);
        self.db.close().await?;
        Ok(())
    }
}

/// Drives `http_request::sync::router::<RuneModel>` directly over HTTP `Request`/`Response`
/// frames with Bearer authentication and wire-size capture.
#[derive(Clone)]
struct HttpRouterPeer {
    app: Router,
    auth: Option<String>,
    wire: Arc<Mutex<Vec<WireExchange>>>,
}

impl HttpRouterPeer {
    fn authorized(library: &LibrarySyncStore) -> Self {
        let wire: Arc<Mutex<Vec<WireExchange>>> = Arc::new(Mutex::new(Vec::new()));
        let recorded = wire.clone();
        let auth_and_wire = middleware::from_fn(move |req: Request, next: Next| {
            let recorded = recorded.clone();
            async move {
                let authorized = req
                    .headers()
                    .get(AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    == Some(AUTH_HEADER);
                if !authorized {
                    return Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body(Body::empty())
                        .unwrap();
                }
                let (parts, body) = req.into_parts();
                let req_bytes = to_bytes(body, 10 * 1024 * 1024).await.unwrap();
                let operation = serde_json::from_slice::<Value>(&req_bytes)
                    .ok()
                    .and_then(|v| {
                        v.get("operation")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .unwrap_or_default();
                let res = next
                    .run(Request::from_parts(parts, Body::from(req_bytes.clone())))
                    .await;
                let (res_parts, res_body) = res.into_parts();
                let res_bytes = to_bytes(res_body, 33 * 1024 * 1024).await.unwrap();
                recorded.lock().unwrap().push(WireExchange {
                    operation,
                    status: res_parts.status.as_u16(),
                    request_bytes: req_bytes.len(),
                    response_bytes: res_bytes.len(),
                });
                Response::from_parts(res_parts, Body::from(res_bytes))
            }
        });
        let app = router::<RuneModel>(Arc::new(library.endpoint())).layer(auth_and_wire);
        Self {
            app,
            auth: Some(AUTH_HEADER.to_owned()),
            wire,
        }
    }

    fn without_auth(&self) -> Self {
        Self {
            app: self.app.clone(),
            auth: None,
            wire: self.wire.clone(),
        }
    }

    async fn call<T: DeserializeOwned>(&self, payload: Value) -> p::Result<T> {
        let bytes =
            serde_json::to_vec(&payload).map_err(|err| SyncError::Invalid(err.to_string()))?;
        if bytes.len() > REQUEST_BYTES {
            return Err(SyncError::LimitTooSmall {
                required: bytes.len() as u64,
            });
        }
        let mut builder = Request::builder()
            .method("POST")
            .uri("/sync/metadata")
            .header(CONTENT_TYPE, "application/json");
        if let Some(auth) = &self.auth {
            builder = builder.header(AUTHORIZATION, auth);
        }
        let request = builder
            .body(Body::from(bytes))
            .map_err(|err| SyncError::Transport(err.to_string()))?;
        let response = match self.app.clone().oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        };
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            return Err(SyncError::Unauthorized);
        }
        if status == StatusCode::FORBIDDEN {
            return Err(SyncError::Forbidden);
        }
        let body = to_bytes(response.into_body(), RESPONSE_BYTES + 1)
            .await
            .map_err(|_| SyncError::LogicalLimit)?;
        if body.len() > RESPONSE_BYTES {
            return Err(SyncError::LogicalLimit);
        }
        if !status.is_success() {
            if let Ok(Err(error)) = serde_json::from_slice::<p::Result<Value>>(&body) {
                return Err(error);
            }
            return Err(SyncError::Transport(format!(
                "metadata HTTP status {status}"
            )));
        }
        serde_json::from_slice::<p::Result<T>>(&body)
            .map_err(|err| SyncError::Invalid(format!("invalid metadata response: {err}")))?
    }
}

impl SyncPeer<RuneModel> for HttpRouterPeer {
    fn identity(&self) -> BoxFuture<'_, StoreIdentity> {
        Box::pin(async move { self.call(json!({ "operation": "identity" })).await })
    }
    fn head(&self) -> BoxFuture<'_, ReceiveProgress> {
        Box::pin(async move { self.call(json!({ "operation": "head" })).await })
    }
    fn history_status<'a>(
        &'a self,
        key: &'a CheckpointKey,
        progress: Option<ReceiveProgress>,
    ) -> BoxFuture<'a, HistoryStatus> {
        let key = key.clone();
        Box::pin(async move {
            self.call(json!({
                "operation": "history",
                "key": key,
                "progress": progress,
            }))
            .await
        })
    }
    fn read_page(&self, request: PageRequest<RuneModel>) -> BoxFuture<'_, PageRead<RuneModel>> {
        Box::pin(async move {
            self.call(json!({
                "operation": "read_page",
                "request": request,
            }))
            .await
        })
    }
    fn read_fragment(
        &self,
        manifest: ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> BoxFuture<'_, Fragment> {
        Box::pin(async move {
            self.call(json!({
                "operation": "read_fragment",
                "manifest": manifest,
                "ordinal": ordinal,
                "max_bytes": max_bytes,
            }))
            .await
        })
    }
}

impl ReceiveStore<RuneModel> for HttpRouterPeer {
    fn identity(&self) -> BoxFuture<'_, StoreIdentity> {
        Box::pin(async move { self.call(json!({ "operation": "identity" })).await })
    }
    fn checkpoint<'a>(&'a self, key: &'a CheckpointKey) -> BoxFuture<'a, Option<ReceiveProgress>> {
        let key = key.clone();
        Box::pin(async move {
            self.call(json!({
                "operation": "checkpoint",
                "key": key,
            }))
            .await
        })
    }
    fn apply_page<'a>(
        &'a self,
        page: &'a ChangePage<RuneModel>,
        limits: ApplyLimits,
    ) -> BoxFuture<'a, ApplyResult<RuneModel>> {
        let page = page.clone();
        Box::pin(async move {
            self.call(json!({
                "operation": "apply_page",
                "page": page,
                "limits": limits,
            }))
            .await
        })
    }
    fn begin_stage(
        &self,
        descriptor: StageDescriptor<RuneModel>,
        limits: StageLimits,
    ) -> BoxFuture<'_, FragmentProgress> {
        Box::pin(async move {
            self.call(json!({
                "operation": "begin_stage",
                "descriptor": descriptor,
                "limits": limits,
            }))
            .await
        })
    }
    fn stage_fragment(&self, stage: Uuid, fragment: Fragment) -> BoxFuture<'_, FragmentProgress> {
        Box::pin(async move {
            self.call(json!({
                "operation": "stage_fragment",
                "stage": stage,
                "fragment": fragment,
            }))
            .await
        })
    }
    fn stage_status(&self, stage: Uuid) -> BoxFuture<'_, FragmentProgress> {
        Box::pin(async move {
            self.call(json!({
                "operation": "stage_status",
                "stage": stage,
            }))
            .await
        })
    }
    fn abandon_stage(&self, stage: Uuid) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.call(json!({
                "operation": "abandon_stage",
                "stage": stage,
            }))
            .await
        })
    }
    fn abandon_origin(&self, origin: StageOrigin<RuneModel>) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.call(json!({
                "operation": "abandon_origin",
                "origin": origin,
            }))
            .await
        })
    }
    fn commit_fragmented_event(
        &self,
        key: CheckpointKey,
        sequence: u64,
        stage: Uuid,
        limits: ApplyLimits,
    ) -> BoxFuture<'_, ApplyResult<RuneModel>> {
        Box::pin(async move {
            self.call(json!({
                "operation": "commit_fragmented_event",
                "key": key,
                "sequence": sequence,
                "stage": stage,
                "limits": limits,
            }))
            .await
        })
    }
    fn finish_session(&self, key: CheckpointKey, through: ReceiveProgress) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.call(json!({
                "operation": "finish_session",
                "key": key,
                "through": through,
            }))
            .await
        })
    }
}

impl SnapshotSource<RuneModel> for HttpRouterPeer {
    fn release_request(&self, request_id: Uuid) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.call(json!({
                "operation": "release_request",
                "request_id": request_id,
            }))
            .await
        })
    }
    fn open_snapshot(
        &self,
        request_id: Uuid,
        scope: Scope<RuneModel>,
        limits: WorkLimits,
        resume: Option<WorkCursor>,
    ) -> BoxFuture<'_, WorkOutcome<SnapshotDescriptor<RuneModel>>> {
        Box::pin(async move {
            self.call(json!({
                "operation": "open_snapshot",
                "request_id": request_id,
                "scope": scope,
                "limits": limits,
                "resume": resume,
            }))
            .await
        })
    }
    fn summarize(
        &self,
        snapshot: Uuid,
        range: KeyRange<RuneModel>,
        limits: WorkLimits,
        resume: Option<WorkCursor>,
    ) -> BoxFuture<'_, WorkOutcome<RangeSummary<RuneModel>>> {
        Box::pin(async move {
            self.call(json!({
                "operation": "summarize",
                "snapshot": snapshot,
                "range": range,
                "limits": limits,
                "resume": resume,
            }))
            .await
        })
    }
    fn split(
        &self,
        snapshot: Uuid,
        range: KeyRange<RuneModel>,
        limits: WorkLimits,
        resume: Option<WorkCursor>,
    ) -> BoxFuture<'_, WorkOutcome<Vec<KeyRange<RuneModel>>>> {
        Box::pin(async move {
            self.call(json!({
                "operation": "split",
                "snapshot": snapshot,
                "range": range,
                "limits": limits,
                "resume": resume,
            }))
            .await
        })
    }
    fn leaf(
        &self,
        snapshot: Uuid,
        range: KeyRange<RuneModel>,
        after: Option<CandidateCursor<RuneModel>>,
        limits: PageLimits,
    ) -> BoxFuture<'_, LeafPage<RuneModel>> {
        Box::pin(async move {
            self.call(json!({
                "operation": "leaf",
                "snapshot": snapshot,
                "range": range,
                "after": after,
                "limits": limits,
            }))
            .await
        })
    }
    fn read_fragment(
        &self,
        snapshot: Uuid,
        manifest: ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> BoxFuture<'_, Fragment> {
        Box::pin(async move {
            self.call(json!({
                "operation": "snapshot_fragment",
                "snapshot": snapshot,
                "manifest": manifest,
                "ordinal": ordinal,
                "max_bytes": max_bytes,
            }))
            .await
        })
    }
    fn close_snapshot(&self, snapshot: Uuid) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.call(json!({
                "operation": "close_snapshot",
                "snapshot": snapshot,
            }))
            .await
        })
    }
    fn cancel_snapshot_build(&self, cursor: WorkCursor) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.call(json!({
                "operation": "cancel_snapshot_build",
                "cursor": cursor,
            }))
            .await
        })
    }
}

impl SnapshotSink<RuneModel> for HttpRouterPeer {
    fn cancel_reconciliation(&self, session: Uuid) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.call(json!({
                "operation": "cancel_reconciliation",
                "session": session,
            }))
            .await
        })
    }
    fn discard_range(&self, session: Uuid, range: KeyRange<RuneModel>) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.call(json!({
                "operation": "discard_range",
                "session": session,
                "range": range,
            }))
            .await
        })
    }
    fn begin_reconciliation(
        &self,
        request_id: Uuid,
        remote: SnapshotDescriptor<RuneModel>,
        local: SnapshotDescriptor<RuneModel>,
    ) -> BoxFuture<'_, Uuid> {
        Box::pin(async move {
            self.call(json!({
                "operation": "begin_reconciliation",
                "request_id": request_id,
                "remote": remote,
                "local": local,
            }))
            .await
        })
    }
    fn coverage(
        &self,
        session: Uuid,
        ledger_revision: Option<u64>,
        after: u64,
        limits: PageLimits,
    ) -> BoxFuture<'_, CoveragePage<RuneModel>> {
        Box::pin(async move {
            self.call(json!({
                "operation": "coverage",
                "session": session,
                "ledger_revision": ledger_revision,
                "after": after,
                "limits": limits,
            }))
            .await
        })
    }
    fn split_range(
        &self,
        session: Uuid,
        parent: KeyRange<RuneModel>,
        children: Vec<RangeSummary<RuneModel>>,
    ) -> BoxFuture<'_, CoverageProgress> {
        Box::pin(async move {
            self.call(json!({
                "operation": "split_range",
                "session": session,
                "parent": parent,
                "children": children,
            }))
            .await
        })
    }
    fn accept_equal_range(
        &self,
        session: Uuid,
        source: RangeSummary<RuneModel>,
    ) -> BoxFuture<'_, CoverageProgress> {
        Box::pin(async move {
            self.call(json!({
                "operation": "accept_equal_range",
                "session": session,
                "source": source,
            }))
            .await
        })
    }
    fn leaf_progress(
        &self,
        session: Uuid,
        range: KeyRange<RuneModel>,
    ) -> BoxFuture<'_, LeafProgress<RuneModel>> {
        Box::pin(async move {
            self.call(json!({
                "operation": "leaf_progress",
                "session": session,
                "range": range,
            }))
            .await
        })
    }
    fn stage_leaf_page(
        &self,
        session: Uuid,
        page: LeafPage<RuneModel>,
    ) -> BoxFuture<'_, LeafProgress<RuneModel>> {
        Box::pin(async move {
            self.call(json!({
                "operation": "stage_leaf_page",
                "session": session,
                "page": page,
            }))
            .await
        })
    }
    fn commit_complete_range(
        &self,
        session: Uuid,
        range: KeyRange<RuneModel>,
        limits: ApplyLimits,
    ) -> BoxFuture<'_, (CoverageProgress, ProjectionBatch<RuneModel>)> {
        Box::pin(async move {
            self.call(json!({
                "operation": "commit_complete_range",
                "session": session,
                "range": range,
                "limits": limits,
            }))
            .await
        })
    }
    fn promote_step(&self, session: Uuid, limits: ApplyLimits) -> BoxFuture<'_, PromoteProgress> {
        Box::pin(async move {
            self.call(json!({
                "operation": "promote_step",
                "session": session,
                "limits": limits,
            }))
            .await
        })
    }
    fn complete_reconciliation(&self, session: Uuid) -> BoxFuture<'_, ReceiveProgress> {
        Box::pin(async move {
            self.call(json!({
                "operation": "complete_reconciliation",
                "session": session,
            }))
            .await
        })
    }
}

fn scope() -> Scope<RuneModel> {
    Scope {
        collections: RuneModel::collections().into_iter().collect(),
    }
}

fn deterministic_actor(node: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_DNS, format!("RV355-{node}").as_bytes())
}

fn deterministic_track_id(index: usize) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_DNS,
        format!("RV355-track-{index:03}").as_bytes(),
    )
}

async fn write_tracks(library: &LibrarySyncStore, items: &[(Uuid, usize)]) -> Result<()> {
    for &(id, title_bytes) in items {
        ensure!(
            title_bytes <= 16_384,
            "fixture title exceeds supported field cap"
        );
        let duration = "1"
            .to_owned()
            .try_into()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        library
            .write_local(
                ObjectKey {
                    collection: Collection::Tracks,
                    id,
                },
                Some(Payload::Track(Track {
                    extension: "wav".into(),
                    content_hash: None,
                    content_size: None,
                    duration,
                    sample_rate: 48_000,
                    cover_art_id: None,
                    metadata: vec![MetadataTag {
                        id: Uuid::from_u128(id.as_u128() ^ 0x355),
                        position: 0,
                        key: "track_title".into(),
                        value: "T".repeat(title_bytes),
                    }],
                })),
            )
            .await?;
    }
    Ok(())
}

async fn live_candidate_bodies(db: &DatabaseConnection) -> Result<Vec<(usize, Value)>> {
    let stmt = Statement::from_string(
        DbBackend::Sqlite,
        "SELECT body FROM sync_candidates WHERE live_until IS NULL ORDER BY object",
    );
    let rows = db.query_all_raw(stmt).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let body: Vec<u8> = row.try_get_by_index(0)?;
        let parsed: Value = serde_json::from_slice(&body)?;
        out.push((body.len(), parsed));
    }
    Ok(out)
}

async fn calibrate_64_tracks_on_node(node: &NodeHandle) -> Result<Vec<(Uuid, usize)>> {
    let mut items: Vec<(Uuid, usize)> = (0..64)
        .map(|i| (deterministic_track_id(i), 15_500))
        .collect();
    write_tracks(&node.library, &items).await?;
    for _ in 0..6 {
        let rows = live_candidate_bodies(&node.db).await?;
        if rows.len() == 64 && rows.iter().all(|(len, _)| *len == 16_384) {
            return Ok(items);
        }
        let mut next_items = Vec::with_capacity(rows.len());
        for (len, parsed) in rows {
            let id = Uuid::parse_str(parsed["key"]["id"].as_str().context("candidate key.id")?)?;
            let current_title_len = parsed["payload"]["value"]["metadata"][0]["value"]
                .as_str()
                .context("candidate metadata title")?
                .len();
            let adjusted = current_title_len + 16_384 - len;
            next_items.push((id, adjusted));
        }
        items = next_items;
        write_tracks(&node.library, &items).await?;
    }
    anyhow::bail!("candidate size calibration did not converge to 64 x 16384 bytes");
}

async fn sync_over_http(
    source: &HttpRouterPeer,
    target: &HttpRouterPeer,
    limits: SessionLimits,
) -> result::Result<ReceiveProgress, SyncError> {
    let mut coordinator = Coordinator::new(scope(), limits)?;
    let max_bytes = coordinator.limits.work.max_bytes;
    let max_requests = coordinator.limits.work.max_requests;
    for _ in 0..10_000 {
        let step = coordinator
            .advance(target, target, target, source, source)
            .await;
        let usage = coordinator.pass_usage();
        assert!(
            usage.requests <= max_requests && usage.encoded_bytes <= max_bytes,
            "pass usage {usage:?} exceeded work limits"
        );
        match step {
            Ok(SessionAdvance::Complete(progress)) => return Ok(progress),
            Ok(SessionAdvance::BudgetExhausted) => coordinator.start_pass(),
            Ok(SessionAdvance::Incomplete) => (),
            Ok(SessionAdvance::Cancelled) => {
                return Err(SyncError::Transport("unexpected cancellation".into()));
            }
            Err(err) => return Err(err),
        }
    }
    Err(SyncError::Transport(
        "coordinator step bound exceeded".into(),
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn large_legal_metadata_event_syncs_over_http_and_persists_across_cold_restart() -> Result<()>
{
    let temp = TempDir::new()?;
    let namespace = Uuid::new_v4();
    let dir_a: PathBuf = temp.path().join("a");
    let dir_b: PathBuf = temp.path().join("b");
    let dir_c: PathBuf = temp.path().join("c");

    let node_a = NodeHandle::open(&dir_a, deterministic_actor("a"), namespace).await?;
    let calibrated = calibrate_64_tracks_on_node(&node_a).await?;
    let a_candidates = live_candidate_bodies(&node_a.db).await?;
    assert_eq!(a_candidates.len(), 64);
    assert_eq!(
        a_candidates.iter().map(|(n, _)| *n).sum::<usize>(),
        64 * 16_384
    );

    let node_b = NodeHandle::open(&dir_b, deterministic_actor("b"), namespace).await?;
    let node_c = NodeHandle::open(&dir_c, deterministic_actor("c"), namespace).await?;

    let peer_a = HttpRouterPeer::authorized(&node_a.library);
    let peer_b = HttpRouterPeer::authorized(&node_b.library);
    let peer_c = HttpRouterPeer::authorized(&node_c.library);

    // Negative authorization control: unauthenticated metadata request is rejected with 401.
    assert_eq!(
        SyncPeer::identity(&peer_a.without_auth()).await,
        Err(SyncError::Unauthorized)
    );

    // Baseline empty sync B -> C establishes C's incremental checkpoint for B at sequence 0.
    let baseline = sync_over_http(&peer_b, &peer_c, session_limits())
        .await
        .expect("baseline empty sync B -> C");
    assert_eq!(baseline.sequence, 0);

    // Snapshot reconciliation A -> B writes a single 64-candidate MergeEvent (1,048,714 bytes) on B.
    sync_over_http(&peer_a, &peer_b, session_limits())
        .await
        .expect("snapshot sync A -> B");
    assert_eq!(live_candidate_bodies(&node_b.db).await?.len(), 64);

    // Incremental journal sync B -> C transfers the fragmented 64-candidate MergeEvent over HTTP.
    let expected_head = node_b.library.head().await?;
    let c_progress = sync_over_http(&peer_b, &peer_c, session_limits())
        .await
        .expect("fragmented journal sync B -> C must succeed within default session limits");
    assert_eq!(c_progress, expected_head);
    assert_eq!(live_candidate_bodies(&node_c.db).await?.len(), 64);

    {
        let b_wire = peer_b.wire.lock().unwrap();
        let read_fragments: Vec<&WireExchange> = b_wire
            .iter()
            .filter(|entry| entry.operation == "read_fragment")
            .collect();
        assert!(read_fragments.iter().all(|entry| entry.status == 200));
        let large: Vec<&&WireExchange> = read_fragments
            .iter()
            .filter(|entry| entry.response_bytes > 1_048_832)
            .collect();
        assert!(large.len() == 2, "expected two large fragment responses");
        for exchange in large {
            assert_eq!(exchange.status, 200);
            assert!(exchange.request_bytes <= REQUEST_BYTES);
            assert!(exchange.response_bytes <= RESPONSE_BYTES);
        }
        let c_wire = peer_c.wire.lock().unwrap();
        let stage_fragments: Vec<&WireExchange> = c_wire
            .iter()
            .filter(|entry| entry.operation == "stage_fragment" && entry.request_bytes > 1_000_000)
            .collect();
        assert_eq!(stage_fragments.len(), 2);
        for exchange in stage_fragments {
            assert_eq!(exchange.status, 200);
            assert!(exchange.request_bytes <= REQUEST_BYTES);
            assert!(exchange.response_bytes <= RESPONSE_BYTES);
        }
    }

    // Cold restart of node C: cleanly close router + SQLite, reopen from disk, verify all 64 rows,
    // and verify a cold retry sync B -> C remains idempotent.
    drop(peer_c);
    node_c.close().await?;

    let node_c_reopened = NodeHandle::open(&dir_c, deterministic_actor("c"), namespace).await?;
    let peer_c_reopened = HttpRouterPeer::authorized(&node_c_reopened.library);

    let c_cold_candidates = live_candidate_bodies(&node_c_reopened.db).await?;
    assert_eq!(c_cold_candidates.len(), 64);
    assert!(c_cold_candidates.iter().all(|(len, _)| *len == 16_384));

    for &(track_id, title_bytes) in &calibrated {
        let record = node_c_reopened
            .library
            .get(&ObjectKey {
                collection: Collection::Tracks,
                id: track_id,
            })
            .await?
            .context("reopened node C missing synced track")?;
        let Some(Payload::Track(track)) = record.value else {
            anyhow::bail!("unexpected payload variant on reopened node C");
        };
        assert_eq!(track.metadata.len(), 1);
        assert_eq!(track.metadata[0].value.len(), title_bytes);
    }

    let head_before_retry = node_c_reopened.library.head().await?;
    let retry_progress = sync_over_http(&peer_b, &peer_c_reopened, session_limits())
        .await
        .expect("cold retry sync B -> C");
    assert_eq!(retry_progress, node_b.library.head().await?);
    assert_eq!(node_c_reopened.library.head().await?, head_before_retry);

    // Also verify too-small journal and snapshot fragment budgets over HTTP still return typed LimitTooSmall.
    let dir_d = temp.path().join("d");
    let node_d = NodeHandle::open(&dir_d, deterministic_actor("d"), namespace).await?;
    let peer_d = HttpRouterPeer::authorized(&node_d.library);

    let mut small_snapshot_limits = session_limits();
    small_snapshot_limits.stages.max_fragment_bytes = 1;
    let snapshot_err = sync_over_http(&peer_b, &peer_d, small_snapshot_limits)
        .await
        .expect_err("1-byte snapshot fragment limit must reject snapshot context");
    assert!(
        matches!(snapshot_err, SyncError::LimitTooSmall { required } if required > 1),
        "expected typed LimitTooSmall for snapshot fragment, got {snapshot_err:?}"
    );

    let dir_e = temp.path().join("e");
    let node_e = NodeHandle::open(&dir_e, deterministic_actor("e"), namespace).await?;
    let dir_f = temp.path().join("f");
    let node_f = NodeHandle::open(&dir_f, deterministic_actor("f"), namespace).await?;
    let peer_e = HttpRouterPeer::authorized(&node_e.library);
    let peer_f = HttpRouterPeer::authorized(&node_f.library);
    sync_over_http(&peer_e, &peer_f, session_limits())
        .await
        .expect("baseline empty sync E -> F");
    sync_over_http(&peer_a, &peer_e, session_limits())
        .await
        .expect("snapshot sync A -> E");

    let mut small_journal_limits = session_limits();
    small_journal_limits.stages.max_fragment_bytes = 256 * 1024;
    let journal_err = sync_over_http(&peer_e, &peer_f, small_journal_limits)
        .await
        .expect_err("256 KiB journal fragment limit must reject 524,357-byte raw fragment");
    assert!(
        matches!(journal_err, SyncError::LimitTooSmall { required } if required >= 524_357),
        "expected typed LimitTooSmall for journal fragment, got {journal_err:?}"
    );

    drop(peer_a);
    drop(peer_b);
    drop(peer_c_reopened);
    drop(peer_d);
    drop(peer_e);
    drop(peer_f);
    node_a.close().await?;
    node_b.close().await?;
    node_c_reopened.close().await?;
    node_d.close().await?;
    node_e.close().await?;
    node_f.close().await?;
    Ok(())
}
