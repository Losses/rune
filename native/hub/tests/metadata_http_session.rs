//! Real TCP metadata sessions with acknowledged writes whose replies are lost.
#[path = "support/causal_library.rs"]
mod support;
use anyhow::{Context, Result, ensure};
use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use database::sync::model::{Collection, Payload, Playlist, RuneModel};
use http_request::sync::router;
type MetadataPeer = http_request::sync::MetadataPeer<RuneModel>;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::Duration,
};
use support::{Peer, SqlLibrary};
use sync::protocol as p;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Copy, Debug)]
enum Disturbance {
    Truncate,
    Reset,
    Timeout,
    Slow,
}

#[derive(Default)]
struct Faults {
    remaining: BTreeMap<String, usize>,
    calls: BTreeMap<String, usize>,
    dropped: usize,
    disturbances: BTreeMap<String, VecDeque<Disturbance>>,
    exercised: Vec<Disturbance>,
}

async fn lose_reply(
    State(faults): State<Arc<Mutex<Faults>>>,
    request: Request,
    next: Next,
) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, 9 * 1024 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let operation = json["operation"].as_str().unwrap().to_owned();
    let response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    let mut faults = faults.lock().await;
    *faults.calls.entry(operation.clone()).or_default() += 1;
    if let Some(remaining) = faults.remaining.get_mut(&operation)
        && *remaining > 0
        && response.status().is_success()
    {
        *remaining -= 1;
        faults.dropped += 1;
        // The real handler already completed. Only its successful response is lost.
        return (StatusCode::BAD_GATEWAY, "injected lost acknowledgement").into_response();
    }
    let disturbance = if response.status().is_success() {
        faults
            .disturbances
            .get_mut(&operation)
            .and_then(VecDeque::pop_front)
    } else {
        None
    };
    if let Some(disturbance) = disturbance {
        faults.exercised.push(disturbance);
    }
    drop(faults);
    let Some(disturbance) = disturbance else {
        return response;
    };
    let (mut parts, body) = response.into_parts();
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    let bytes = to_bytes(body, 32 * 1024 * 1024).await.unwrap();
    let body = match disturbance {
        Disturbance::Truncate => Body::from(bytes.slice(..bytes.len() / 2)),
        Disturbance::Reset => Body::from_stream(futures::stream::iter(vec![
            Ok(bytes.slice(..bytes.len().min(32))),
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "injected connection reset",
            )),
        ])),
        Disturbance::Timeout => {
            tokio::time::sleep(Duration::from_millis(700)).await;
            Body::from(bytes)
        }
        Disturbance::Slow => Body::from_stream(futures::stream::unfold(
            (bytes, 0),
            |(bytes, offset)| async move {
                if offset == bytes.len() {
                    return None;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
                let end = (offset + 512).min(bytes.len());
                Some((
                    Ok::<_, std::io::Error>(bytes.slice(offset..end)),
                    (bytes, end),
                ))
            },
        )),
    };
    Response::from_parts(parts, body)
}

struct Endpoint {
    peer: MetadataPeer,
    faults: Arc<Mutex<Faults>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Endpoint {
    async fn start(store: Peer, operations: &[&str]) -> Result<Self> {
        let faults = Arc::new(Mutex::new(Faults {
            remaining: operations.iter().map(|name| ((*name).into(), 1)).collect(),
            ..Default::default()
        }));
        let app = router(store).layer(middleware::from_fn_with_state(faults.clone(), lose_reply));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Ok(Self {
            peer: MetadataPeer::new(
                base,
                reqwest::Client::builder()
                    .timeout(Duration::from_millis(500))
                    .build()?,
            ),
            faults,
            task,
        })
    }
}

async fn replica(namespace: Uuid, index: u128) -> Result<SqlLibrary> {
    SqlLibrary::new(namespace, Uuid::from_u128(index)).await
}
fn scope() -> p::Scope<RuneModel> {
    p::Scope {
        collections: [Collection::Playlists].into_iter().collect(),
    }
}
fn limits(page_bytes: u64) -> p::SessionLimits {
    p::SessionLimits {
        pages: p::PageLimits {
            max_scanned: 8,
            max_bytes: page_bytes,
        },
        stages: p::StageLimits {
            max_total_bytes: p::DEFAULT_STAGE_BYTES,
            max_stages: p::DEFAULT_STAGES,
            max_event_bytes: p::DEFAULT_LOGICAL_BYTES,
            max_event_items: p::DEFAULT_LOGICAL_ITEMS,
            max_fragment_bytes: p::DEFAULT_PAGE_BYTES,
        },
        apply: p::ApplyLimits {
            max_projection_items: 8,
            max_projection_bytes: 65536,
            max_mutations: 4096,
        },
        work: p::WorkLimits {
            max_requests: 64,
            max_bytes: p::DEFAULT_ROUND_BYTES,
            max_depth: 32,
            max_rows_scanned: 128,
            max_mutations: 4096,
        },
    }
}
fn playlist(name: &str) -> Option<Payload> {
    Some(Payload::Playlist(Playlist {
        name: name.into(),
        group: String::new(),
    }))
}
async fn direction(
    receiver: &dyn p::ReceiveStore<RuneModel>,
    sink: &dyn p::SnapshotSink<RuneModel>,
    receiver_source: &dyn p::SnapshotSource<RuneModel>,
    source: &dyn p::SyncPeer<RuneModel>,
    source_snapshot: &dyn p::SnapshotSource<RuneModel>,
    page_bytes: u64,
) -> Result<usize> {
    let mut coordinator = p::Coordinator::new(scope(), limits(page_bytes))?;
    let mut lost = 0;
    for step in 0..4096 {
        match coordinator
            .advance(receiver, sink, receiver_source, source, source_snapshot)
            .await
        {
            Ok(p::SessionAdvance::Complete(_)) => return Ok(lost),
            Ok(p::SessionAdvance::Incomplete) => {}
            Ok(p::SessionAdvance::BudgetExhausted) => coordinator.start_pass(),
            Ok(other) => anyhow::bail!("unexpected session state: {other:?}"),
            Err(p::SyncError::Transport(_)) => lost += 1,
            Err(error) => {
                return Err(error).with_context(|| format!("direction failed at step {step}"));
            }
        }
    }
    anyhow::bail!("session did not finish within 4096 operations")
}
async fn push(source: &Peer, target: &Endpoint, page_bytes: u64) -> Result<usize> {
    direction(
        &target.peer,
        &target.peer,
        &target.peer,
        &p::StorePeer(source.clone()),
        &**source,
        page_bytes,
    )
    .await
}
async fn pull(target: &Peer, source: &Endpoint, page_bytes: u64) -> Result<usize> {
    direction(
        &**target,
        &**target,
        &**target,
        &source.peer,
        &source.peer,
        page_bytes,
    )
    .await
}

#[tokio::test]
async fn first_contact_and_fragmented_tail_retry_committed_http_operations() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source_library = replica(namespace, 1).await?;
    let target_library = replica(namespace, 2).await?;
    let source = source_library.peer.clone();
    let target = target_library.peer.clone();
    let key = p::ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    };
    source.write_local(key.clone(), playlist("initial")).await?;
    let endpoint = Endpoint::start(
        target.clone(),
        &[
            "open_snapshot",
            "begin_reconciliation",
            "stage_context_page",
            "stage_leaf_page",
            "commit_complete_range",
            "complete_reconciliation",
            "finish_session",
        ],
    )
    .await?;
    assert_eq!(push(&source, &endpoint, 3072).await?, 7);
    assert_eq!(target.get(&key).await?.unwrap().value, playlist("initial"));
    let source_identity = source.identity().await?;
    let checkpoint = p::CheckpointKey {
        namespace,
        source_replica: source_identity.replica,
        source_generation: source_identity.generation,
        scope_id: p::scope_id(&scope())?,
    };
    assert_eq!(
        target.checkpoint(&checkpoint).await?,
        Some(source.head().await?)
    );

    source
        .write_local(key.clone(), playlist(&"x".repeat(6000)))
        .await?;
    {
        let mut faults = endpoint.faults.lock().await;
        for operation in ["begin_stage", "stage_fragment", "commit_fragmented_event"] {
            faults.remaining.insert(operation.into(), 1);
        }
    }
    assert_eq!(push(&source, &endpoint, 3072).await?, 3);
    assert_eq!(
        target.get(&key).await?.unwrap().value,
        playlist(&"x".repeat(6000))
    );
    assert_eq!(
        target.checkpoint(&checkpoint).await?,
        Some(source.head().await?)
    );
    let head = target.head().await?;
    assert_eq!(push(&source, &endpoint, 3072).await?, 0);
    assert_eq!(
        target.head().await?,
        head,
        "acknowledged replay published history again"
    );
    let faults = endpoint.faults.lock().await;
    assert_eq!(faults.dropped, 10);
    ensure!(
        faults.remaining.values().all(|remaining| *remaining == 0),
        "a fault was not exercised"
    );
    Ok(())
}

#[tokio::test]
async fn concurrent_updates_survive_observed_removal_and_http_triangle_reaches_steady_state()
-> Result<()> {
    let namespace = Uuid::new_v4();
    let a_library = replica(namespace, 11).await?;
    let a = a_library.peer.clone();
    let b_library = replica(namespace, 12).await?;
    let b = b_library.peer.clone();
    let c_library = replica(namespace, 13).await?;
    let c = c_library.peer.clone();
    let a_endpoint = Endpoint::start(a.clone(), &[]).await?;
    let b_endpoint = Endpoint::start(b.clone(), &[]).await?;
    let c_endpoint = Endpoint::start(c.clone(), &[]).await?;
    let key = p::ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    };
    a.write_local(key.clone(), playlist("common")).await?;
    pull(&b, &a_endpoint, 65536).await?;
    push(&b, &c_endpoint, 65536).await?;
    a.write_local(key.clone(), playlist("offline A")).await?;
    b.write_local(key.clone(), playlist("offline B")).await?;
    c.write_local(key.clone(), None).await?;
    assert!(c.get(&key).await?.is_none());
    let mut previous = None;
    let mut stable = false;
    for _ in 0..12 {
        push(&a, &b_endpoint, 65536).await?;
        pull(&a, &b_endpoint, 65536).await?;
        push(&b, &c_endpoint, 65536).await?;
        pull(&b, &c_endpoint, 65536).await?;
        push(&c, &a_endpoint, 65536).await?;
        pull(&c, &a_endpoint, 65536).await?;
        let heads = [a.head().await?, b.head().await?, c.head().await?];
        if previous.as_ref() == Some(&heads) {
            stable = true;
            break;
        }
        previous = Some(heads);
    }
    assert!(
        stable,
        "repeated all-pair exchange kept publishing new history"
    );
    let mut expected = None;
    for store in [&a, &b, &c] {
        let page = store
            .candidates(
                &key,
                None,
                p::PageLimits {
                    max_scanned: 8,
                    max_bytes: 65536,
                },
            )
            .await?;
        assert!(page.done);
        assert_eq!(
            page.values.len(),
            2,
            "the unseen concurrent assignment was removed"
        );
        if let Some(expected) = &expected {
            assert_eq!(&page.values, expected);
        } else {
            expected = Some(page.values);
        }
    }
    assert_eq!(a.get(&key).await?, b.get(&key).await?);
    assert_eq!(b.get(&key).await?, c.get(&key).await?);
    Ok(())
}

#[tokio::test]
async fn sqlite_sessions_resume_through_truncation_reset_timeout_and_slow_responses() -> Result<()>
{
    use database::entities::playlists;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let namespace = Uuid::new_v4();
    let source = replica(namespace, 21).await?;
    let target = replica(namespace, 22).await?;
    let source_http = Endpoint::start(source.peer.clone(), &[]).await?;
    let target_http = Endpoint::start(target.peer.clone(), &[]).await?;
    let key = p::ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    };
    source
        .peer
        .write_local(key.clone(), playlist("baseline"))
        .await?;
    push(&source.peer, &target_http, 65536).await?;
    for index in 0..20 {
        source
            .peer
            .write_local(key.clone(), playlist(&format!("tail-{index}")))
            .await?;
    }
    source_http.faults.lock().await.disturbances.insert(
        "read_page".into(),
        [
            Disturbance::Truncate,
            Disturbance::Reset,
            Disturbance::Timeout,
            Disturbance::Slow,
        ]
        .into(),
    );
    target_http.faults.lock().await.disturbances.insert(
        "apply_page".into(),
        [
            Disturbance::Truncate,
            Disturbance::Reset,
            Disturbance::Timeout,
            Disturbance::Slow,
        ]
        .into(),
    );
    let mut coordinator = p::Coordinator::new(scope(), limits(65536))?;
    let mut errors = 0;
    let mut suspended = false;
    let mut complete = false;
    for step in 0..4096 {
        if step == 7 {
            // Retain the caller's continuation across a period with no scheduled work.
            tokio::time::sleep(Duration::from_millis(60)).await;
            suspended = true;
        }
        match coordinator
            .advance(
                &target_http.peer,
                &target_http.peer,
                &target_http.peer,
                &source_http.peer,
                &source_http.peer,
            )
            .await
        {
            Ok(p::SessionAdvance::Complete(_)) => {
                complete = true;
                break;
            }
            Ok(p::SessionAdvance::BudgetExhausted) => coordinator.start_pass(),
            Ok(p::SessionAdvance::Incomplete) => (),
            Ok(other) => anyhow::bail!("unexpected session state: {other:?}"),
            Err(p::SyncError::Transport(_)) => errors += 1,
            Err(error) => return Err(error.into()),
        }
    }
    ensure!(complete && suspended, "session failed to resume");
    assert_eq!(errors, 6);
    assert_eq!(source_http.faults.lock().await.exercised.len(), 4);
    assert_eq!(target_http.faults.lock().await.exercised.len(), 4);
    assert_eq!(
        target.peer.get(&key).await?.unwrap().value,
        playlist("tail-19")
    );
    let row = playlists::Entity::find()
        .filter(playlists::Column::HlcUuid.eq(key.id.to_string()))
        .one(&target.db)
        .await?
        .context("SQL projection missing")?;
    assert_eq!(row.name, "tail-19");
    let stable = target.peer.head().await?;
    direction(
        &target_http.peer,
        &target_http.peer,
        &target_http.peer,
        &source_http.peer,
        &source_http.peer,
        65536,
    )
    .await?;
    assert_eq!(
        target.peer.head().await?,
        stable,
        "retry appended duplicate accepted history"
    );
    Ok(())
}

#[derive(Default)]
struct BatchReplyFault {
    open_calls: usize,
    dropped: Option<(p::WorkCursor, p::WorkCursor)>,
}

async fn lose_batched_snapshot_reply(
    State(fault): State<Arc<Mutex<BatchReplyFault>>>,
    request: Request,
    next: Next,
) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, 9 * 1024 * 1024).await.unwrap();
    let operation: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let is_open = operation["operation"] == "open_snapshot";
    let resume: Option<p::WorkCursor> = if is_open {
        serde_json::from_value(operation["resume"].clone()).unwrap()
    } else {
        None
    };
    let response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    if !is_open || !response.status().is_success() {
        return response;
    }
    let (parts, body) = response.into_parts();
    let bytes = to_bytes(body, 32 * 1024 * 1024).await.unwrap();
    let result: p::Result<p::WorkOutcome<p::SnapshotDescriptor<RuneModel>>> =
        serde_json::from_slice(&bytes).unwrap();
    let mut state = fault.lock().await;
    state.open_calls += 1;
    if let (Some(before), Ok(p::WorkOutcome::Incomplete { cursor: after })) = (resume, result)
        && state.dropped.is_none()
        && after.position > before.position + 1
    {
        state.dropped = Some((before, after));
        return (StatusCode::BAD_GATEWAY, "injected lost batched reply").into_response();
    }
    drop(state);
    Response::from_parts(parts, Body::from(bytes))
}

#[tokio::test]
async fn lost_batched_snapshot_reply_retries_old_cursor_and_preserves_view() -> Result<()> {
    use p::SnapshotSource;
    let namespace = Uuid::new_v4();
    let library = replica(namespace, 41).await?;
    let source = library.peer.clone();
    let mut expected = BTreeMap::new();
    for index in 1..=20_u128 {
        let key = p::ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::from_u128(index),
        };
        let name = format!("captured-{index}");
        source.write_local(key.clone(), playlist(&name)).await?;
        expected.insert(key.id, playlist(&name).unwrap());
    }
    let captured_head = source.head().await?;
    let fault = Arc::new(Mutex::new(BatchReplyFault::default()));
    let app = router(source.clone()).layer(middleware::from_fn_with_state(
        fault.clone(),
        lose_batched_snapshot_reply,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let peer = MetadataPeer::new(base, reqwest::Client::new());
    let work = p::WorkLimits {
        max_requests: 64,
        max_bytes: 1024 * 1024,
        max_depth: 32,
        max_rows_scanned: 1024,
        max_mutations: 65536,
    };
    let request = Uuid::new_v4();
    let mut cursor = match peer.open_snapshot(request, scope(), work, None).await? {
        p::WorkOutcome::Incomplete { cursor } => cursor,
        _ => anyhow::bail!("expected admitted incomplete snapshot"),
    };
    source
        .write_local(
            p::ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(1),
            },
            playlist("changed"),
        )
        .await?;
    source
        .write_local(
            p::ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(2),
            },
            None,
        )
        .await?;
    source
        .write_local(
            p::ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(21),
            },
            playlist("added"),
        )
        .await?;
    let mut lost_input = None;
    let snapshot = loop {
        match peer
            .open_snapshot(request, scope(), work, Some(cursor.clone()))
            .await
        {
            Ok(p::WorkOutcome::Incomplete { cursor: next }) => cursor = next,
            Ok(p::WorkOutcome::Ready(snapshot)) => break snapshot,
            Err(p::SyncError::Transport(_)) if lost_input.is_none() => {
                let state = fault.lock().await;
                let (before, after) = state.dropped.as_ref().context("missing batched drop")?;
                assert_eq!(before, &cursor);
                assert!(after.position - before.position > 1);
                lost_input = Some(cursor.clone());
            }
            Err(error) => return Err(error.into()),
        }
    };
    let old_cursor = lost_input.context("no batched reply was dropped")?;
    assert_eq!(snapshot.head, captured_head);
    let mut values = BTreeMap::new();
    let mut after = None;
    loop {
        let page = peer
            .leaf(
                snapshot.snapshot_id,
                snapshot.roots[0].range.clone(),
                after,
                p::PageLimits {
                    max_scanned: 64,
                    max_bytes: 65536,
                },
            )
            .await?;
        for candidate in page.candidates {
            values.insert(candidate.key.id, candidate.payload);
        }
        if page.done {
            break;
        }
        after = page.next;
        ensure!(after.is_some(), "incomplete page did not advance");
    }
    assert_eq!(values, expected);
    let replay = peer
        .open_snapshot(request, scope(), work, Some(old_cursor))
        .await?;
    assert_eq!(replay, p::WorkOutcome::Ready(snapshot));
    let cancel_id = Uuid::new_v4();
    let cancel = match peer.open_snapshot(cancel_id, scope(), work, None).await? {
        p::WorkOutcome::Incomplete { cursor } => cursor,
        _ => anyhow::bail!("expected cancellable admitted snapshot"),
    };
    peer.cancel_snapshot_build(cancel.clone()).await?;
    assert!(
        peer.open_snapshot(cancel_id, scope(), work, Some(cancel))
            .await
            .is_err()
    );
    assert!(fault.lock().await.open_calls >= 3);
    server.abort();
    Ok(())
}
