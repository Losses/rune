use database::sync::{LibrarySyncStore, model::*};
use database::{
    actions::{
        downloads::set_download_rule,
        file::{blob_path_for_track, catalog_track, hash_file, is_materialized},
        metadata::{MissingHostFilePolicy, ScanOptions, scan_audio_library},
    },
    connection::{RecommendationDbConnection, initialize_db},
    entities::{albums, media_files, sync_download_jobs, sync_download_jobs::Column},
    sync::model::RuneModel,
};
use hub::server::sync::{
    client::sync_library,
    downloads::download_requested_audio,
    http::{BlobContext, SharedLibrary, blob_router},
    library_router,
};
type MetadataPeer = http_request::sync::MetadataPeer<RuneModel>;
#[path = "support/causal_library.rs"]
mod support;
use anyhow::{Context, Result};
use axum::{
    body::{Body, Bytes, to_bytes},
    extract::{Request, State},
    http::header,
    middleware::Next,
    response::Response,
};
use fsio::FsIo;
use futures::stream;
use sea_orm::{
    ColumnTrait, Database, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicUsize, Ordering},
    },
    time::Duration,
};
use sync::{CheckpointKey, PageLimits, ReceiveStore};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

const NORMAL: u8 = 0;
const FRAGMENT: u8 = 1;
const PAGE_EOF: u8 = 2;
const PAGE_RESET: u8 = 3;
const LOST_APPLY: u8 = 4;
const AUDIO_RESET: u8 = 5;
const AUDIO_STALL: u8 = 6;
const SLOW_LINK: u8 = 7;

#[derive(Default)]
struct Faults {
    mode: AtomicU8,
    page: AtomicUsize,
    blobs: AtomicUsize,
    applies: AtomicUsize,
}
impl Faults {
    fn set(&self, mode: u8) {
        self.page.store(0, Ordering::SeqCst);
        self.mode.store(mode, Ordering::SeqCst);
    }
}

async fn perturb(State(faults): State<Arc<Faults>>, request: Request, next: Next) -> Response {
    let mut path = request.uri().path().to_owned();
    let request = if path == "/sync/metadata" {
        let (parts, body) = request.into_parts();
        let bytes = to_bytes(body, 9 * 1024 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        match value["operation"].as_str() {
            Some("read_page") => path = "/sync/page".into(),
            Some("apply_page" | "finish_session") => path = "/sync/apply".into(),
            _ => (),
        }
        Request::from_parts(parts, Body::from(bytes))
    } else {
        request
    };
    let method = request.method().clone();
    let mode = faults.mode.load(Ordering::SeqCst);
    if path == "/sync/page" {
        faults.page.fetch_add(1, Ordering::SeqCst);
    }
    if path == "/sync/apply" {
        faults.applies.fetch_add(1, Ordering::SeqCst);
    }
    if path.starts_with("/sync/blobs/") && method == axum::http::Method::GET {
        faults.blobs.fetch_add(1, Ordering::SeqCst);
    }
    if mode == FRAGMENT {
        tokio::time::sleep(Duration::from_millis(5)).await;
    } else if mode == SLOW_LINK {
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
    let response = next.run(request).await;
    let is_page = path == "/sync/page";
    let is_blob = path.starts_with("/sync/blobs/") && method == axum::http::Method::GET;
    let alter_page = is_page
        && faults.page.load(Ordering::SeqCst) == 2
        && (mode == PAGE_EOF || mode == PAGE_RESET);
    let lose_apply = path == "/sync/apply" && mode == LOST_APPLY;
    if !alter_page
        && !lose_apply
        && !(is_blob && (mode == AUDIO_RESET || mode == AUDIO_STALL))
        && mode != FRAGMENT
        && mode != SLOW_LINK
    {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let bytes = to_bytes(body, 4 * 1024 * 1024).await.unwrap();
    if alter_page || lose_apply {
        faults.mode.store(NORMAL, Ordering::SeqCst);
        parts.headers.remove(header::CONTENT_LENGTH);
    }
    let body = if alter_page && mode == PAGE_EOF {
        Body::from(bytes.slice(..bytes.len().saturating_div(2)))
    } else if (alter_page && mode == PAGE_RESET) || lose_apply || (is_blob && mode == AUDIO_RESET) {
        let prefix = if lose_apply {
            Bytes::new()
        } else {
            bytes.slice(..bytes.len().min(1024))
        };
        Body::from_stream(stream::iter(vec![
            Ok::<_, std::io::Error>(prefix),
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "injected reset",
            )),
        ]))
    } else if is_blob && mode == AUDIO_STALL {
        let prefix = bytes.slice(..bytes.len().min(1024));
        let rest = bytes.slice(bytes.len().min(1024)..);
        Body::from_stream(stream::unfold(
            (0u8, prefix, rest),
            |(step, prefix, rest)| async move {
                match step {
                    0 => Some((Ok::<_, std::io::Error>(prefix.clone()), (1, prefix, rest))),
                    1 => {
                        tokio::time::sleep(Duration::from_millis(800)).await;
                        Some((Ok(rest.clone()), (2, prefix, rest)))
                    }
                    _ => None,
                }
            },
        ))
    } else {
        Body::from_stream(stream::unfold(
            (bytes, 0usize),
            move |(bytes, offset)| async move {
                if offset >= bytes.len() {
                    return None;
                }
                let (chunk_size, delay) = if mode == SLOW_LINK {
                    (256, [5, 15, 25, 35][(offset / 256) % 4])
                } else {
                    (1024, if (offset / 1024) % 2 == 0 { 1 } else { 3 })
                };
                tokio::time::sleep(Duration::from_millis(delay)).await;
                let end = (offset + chunk_size).min(bytes.len());
                Some((
                    Ok::<_, std::io::Error>(bytes.slice(offset..end)),
                    (bytes, end),
                ))
            },
        ))
    };
    Response::from_parts(parts, body)
}

struct Replica {
    _root: tempfile::TempDir,
    db: DatabaseConnection,
    store: Arc<LibrarySyncStore>,
    node: Uuid,
    fsio: Arc<FsIo>,
}
impl Replica {
    async fn new(library: Option<Uuid>) -> Result<Self> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("replica.sqlite");
        let db = Database::connect(format!("sqlite://{}?mode=rwc", path.display())).await?;
        let node = Uuid::new_v4();
        initialize_db(&db, &node.to_string()).await?;
        let fsio = Arc::new(FsIo::new());
        let store = Arc::new(LibrarySyncStore::open(db.clone(), node, library).await?);
        Ok(Self {
            _root: root,
            db,
            store,
            node,
            fsio,
        })
    }
    fn root(&self) -> std::path::PathBuf {
        self._root.path().into()
    }
    async fn album(&self, name: &str) -> Result<ObjectKey> {
        let key = ObjectKey {
            collection: Collection::Albums,
            id: migration::catalog_identity::parent(
                migration::catalog_identity::ParentKind::Album,
                name,
            ),
        };
        self.store
            .write_local(
                key.clone(),
                Some(Payload::Album(Album {
                    name: name.into(),
                    group: "network".into(),
                })),
            )
            .await?;
        Ok(key)
    }
}
async fn serve(
    replica: &Replica,
    faults: Arc<Faults>,
) -> Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>)> {
    serve_at(replica, faults, "127.0.0.1:0".parse()?).await
}
async fn serve_at(
    replica: &Replica,
    faults: Arc<Faults>,
    address: std::net::SocketAddr,
) -> Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>)> {
    let shared = Arc::new(RwLock::new((*replica.store).clone()));
    let app = library_router(SharedLibrary(shared.clone()), Arc::new(|| {}))
        .merge(blob_router(BlobContext {
            store: shared,
            db: replica.db.clone(),
            fsio: replica.fsio.clone(),
            root: replica.root(),
            node_id: replica.node,
            upload_lock: Arc::new(Mutex::new(())),
            notify: Arc::new(|| {}),
        }))
        .layer(axum::middleware::from_fn_with_state(faults, perturb));
    let listener = tokio::net::TcpListener::bind(address).await?;
    let address = listener.local_addr()?;
    Ok((
        address,
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }),
    ))
}
fn client() -> reqwest::Client {
    client_with_read_timeout(Duration::from_millis(300))
}
fn client_with_read_timeout(read_timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .pool_max_idle_per_host(0)
        .connect_timeout(Duration::from_millis(500))
        .read_timeout(read_timeout)
        .build()
        .unwrap()
}
fn url(address: std::net::SocketAddr) -> String {
    format!("http://{address}")
}
fn album_scope() -> Scope {
    Scope {
        collections: [Collection::Albums].into_iter().collect(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn page_interruptions_preserve_prefix_and_peer_progress() -> Result<()> {
    let a = Replica::new(None).await?;
    let library = a.store.identity().await?.namespace;
    let b = Replica::new(Some(library)).await?;
    let c = Replica::new(Some(library)).await?;
    let scope = album_scope();
    initialize(&a, &c, &scope).await?;
    initialize(&b, &c, &scope).await?;
    let keys = [
        a.album("one").await?,
        a.album("two").await?,
        a.album("three").await?,
    ];
    let b_key = b.album("independent").await?;
    let fa = Arc::new(Faults::default());
    let fb = Arc::new(Faults::default());
    let (addr_a, server_a) = serve(&a, fa.clone()).await?;
    let (addr_b, server_b) = serve(&b, fb.clone()).await?;
    let limits = PageLimits {
        max_scanned: 1,
        max_bytes: 8192,
    };
    let identity = a.store.identity().await?;
    let checkpoint = CheckpointKey {
        namespace: identity.namespace,
        source_replica: identity.replica,
        source_generation: identity.generation,
        scope_id: sync::scope_id(&scope)?,
    };
    let remote_a = MetadataPeer::new(url(addr_a), client());
    fa.set(PAGE_EOF);
    assert!(
        pull(c.store.as_ref(), &remote_a, &scope, limits)
            .await
            .is_err()
    );
    assert_eq!(
        c.store
            .endpoint()
            .checkpoint(&checkpoint)
            .await?
            .unwrap()
            .sequence,
        1
    );
    assert!(c.store.get(&keys[0]).await?.is_some());
    assert_eq!(
        albums::Entity::find()
            .filter(albums::Column::Name.eq("one"))
            .count(&c.db)
            .await?,
        1
    );
    assert_eq!(
        albums::Entity::find()
            .filter(albums::Column::Name.eq("two"))
            .count(&c.db)
            .await?,
        0
    );
    assert!(c.store.get(&keys[1]).await?.is_none());
    fa.set(PAGE_RESET);
    assert!(
        pull(c.store.as_ref(), &remote_a, &scope, limits)
            .await
            .is_err()
    );
    assert_eq!(
        c.store
            .endpoint()
            .checkpoint(&checkpoint)
            .await?
            .unwrap()
            .sequence,
        2
    );
    assert!(c.store.get(&keys[1]).await?.is_some());
    let before = c.store.head().await?;
    fa.set(FRAGMENT);
    pull(c.store.as_ref(), &remote_a, &scope, limits).await?;
    assert_eq!(c.store.head().await?.sequence, before.sequence + 1);
    let complete = c.store.head().await?;
    pull(c.store.as_ref(), &remote_a, &scope, limits).await?;
    assert_eq!(c.store.head().await?, complete);
    for key in &keys {
        assert!(c.store.get(key).await?.is_some());
    }
    // A refused connection must leave A's checkpoint unchanged; B remains usable.
    server_a.abort();
    let _ = server_a.await;
    let fresh_a = MetadataPeer::new(url(addr_a), client());
    assert!(
        pull(c.store.as_ref(), &fresh_a, &scope, limits)
            .await
            .is_err()
    );
    assert_eq!(
        c.store
            .endpoint()
            .checkpoint(&checkpoint)
            .await?
            .unwrap()
            .sequence,
        3
    );
    let remote_b = MetadataPeer::new(url(addr_b), client());
    pull(c.store.as_ref(), &remote_b, &scope, limits).await?;
    assert!(c.store.get(&b_key).await?.is_some());
    // Recreate A at the same address and confirm no log or progress is duplicated.
    let shared = Arc::new(RwLock::new((*a.store).clone()));
    let listener = tokio::net::TcpListener::bind(addr_a).await?;
    let restarted = tokio::spawn(async move {
        axum::serve(
            listener,
            library_router(SharedLibrary(shared), Arc::new(|| {})),
        )
        .await
        .unwrap()
    });
    server_b.abort();
    let _ = server_b.await;
    let b_identity = b.store.identity().await?;
    let b_checkpoint = CheckpointKey {
        namespace: b_identity.namespace,
        source_replica: b_identity.replica,
        source_generation: b_identity.generation,
        scope_id: sync::scope_id(&scope)?,
    };
    let b_before = c.store.endpoint().checkpoint(&b_checkpoint).await?;
    assert!(
        pull(
            c.store.as_ref(),
            &MetadataPeer::new(url(addr_b), client()),
            &scope,
            limits
        )
        .await
        .is_err()
    );
    assert_eq!(
        c.store.endpoint().checkpoint(&b_checkpoint).await?,
        b_before
    );
    let previous = c.store.head().await?;
    pull(
        c.store.as_ref(),
        &MetadataPeer::new(url(addr_a), client()),
        &scope,
        limits,
    )
    .await?;
    assert_eq!(c.store.head().await?, previous);
    let shared_b = Arc::new(RwLock::new((*b.store).clone()));
    let listener_b = tokio::net::TcpListener::bind(addr_b).await?;
    let restored_b = tokio::spawn(async move {
        axum::serve(
            listener_b,
            library_router(SharedLibrary(shared_b), Arc::new(|| {})),
        )
        .await
        .unwrap()
    });
    pull(
        c.store.as_ref(),
        &MetadataPeer::new(url(addr_b), client()),
        &scope,
        limits,
    )
    .await?;
    assert_eq!(c.store.head().await?, previous);
    restarted.abort();
    restored_b.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lost_commit_ack_and_audio_interruptions_retry_without_duplicates() -> Result<()> {
    let source = Replica::new(None).await?;
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg"),
        source.root().join("one.ogg"),
    )?;
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_1.ogg"),
        source.root().join("two.ogg"),
    )?;
    scan_audio_library(
        &source.fsio,
        &source.db,
        &source.node.to_string(),
        &source.root(),
        ScanOptions {
            cleanup: false,
            force: false,
            missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
            cancel_token: None,
        },
        |_| {},
        &source.store.record_store(),
    )
    .await?;
    let source_files = media_files::Entity::find().all(&source.db).await?;
    assert_eq!(source_files.len(), 2);
    source.store.import_existing(source.node).await?;
    let library = source.store.identity().await?.namespace;
    let target = Replica::new(Some(library)).await?;
    let faults = Arc::new(Faults::default());
    let (address, server) = serve(&source, faults.clone()).await?;
    let base = url(address);
    // Reply-loss injection is independent of the audio-stall deadline.
    let metadata_client = client_with_read_timeout(Duration::from_secs(3));
    sync_library(
        target.db.clone(),
        target.node,
        &base,
        metadata_client.clone(),
    )
    .await?;
    assert_eq!(
        faults.blobs.load(Ordering::SeqCst),
        0,
        "metadata sync must not request audio"
    );
    let local_key = target.album("local after initial round").await?;
    faults.set(LOST_APPLY);
    assert!(
        sync_library(
            target.db.clone(),
            target.node,
            &base,
            metadata_client.clone()
        )
        .await
        .is_err()
    );
    assert!(
        source.store.get(&local_key).await?.is_some(),
        "server committed before response loss"
    );
    assert_eq!(
        albums::Entity::find()
            .filter(albums::Column::Name.eq("local after initial round"))
            .count(&source.db)
            .await?,
        1
    );
    let head = source.store.head().await?;
    let applies = faults.applies.load(Ordering::SeqCst);
    sync_library(
        target.db.clone(),
        target.node,
        &base,
        metadata_client.clone(),
    )
    .await?;
    assert_eq!(
        source.store.head().await?,
        head,
        "lost response retry duplicated journal events"
    );
    assert_eq!(
        faults.applies.load(Ordering::SeqCst),
        applies + 1,
        "an idle retry sends only a completion acknowledgement"
    );
    let selected = media_files::Entity::find()
        .filter(media_files::Column::LocalAvailable.eq(false))
        .order_by_asc(media_files::Column::Id)
        .one(&target.db)
        .await?
        .context("no catalog")?
        .id;
    let other = media_files::Entity::find()
        .filter(media_files::Column::LocalAvailable.eq(false))
        .order_by_desc(media_files::Column::Id)
        .one(&target.db)
        .await?
        .context("no other catalog")?
        .id;
    let (track_id, blob, extension) = catalog_track(&target.db, selected)
        .await?
        .context("missing blob")?;
    let final_path = blob_path_for_track(
        &target.fsio,
        &target.root(),
        track_id,
        &blob.digest,
        &extension,
    )?;
    set_download_rule(
        &target.db,
        vec![("lib::track".into(), selected.to_string())],
        None,
        true,
    )
    .await?;
    let counter = sync_download_jobs::Entity::find().count(&target.db).await?;
    assert_eq!(counter, 0);
    for mode in [AUDIO_RESET, AUDIO_STALL] {
        // Advance the persisted retry deadline between independently injected faults.
        sync_download_jobs::Entity::update_many()
            .col_expr(
                Column::RetryAt,
                sea_orm::sea_query::Expr::value("2000-01-01T00:00:00Z"),
            )
            .exec(&target.db)
            .await?;
        faults.set(mode);
        let start = std::time::Instant::now();
        let result = download_requested_audio(
            target.db.clone(),
            &RecommendationDbConnection::new(),
            target.fsio.clone(),
            target.root(),
            target.node,
            &http_request::sync::MetadataPeer::new(&base, client()),
            Arc::new(|| {}),
        )
        .await;
        let result = result?;
        assert_eq!(result.downloaded, 0);
        assert_eq!(result.pending, 1, "audio failure must remain queued");
        assert_eq!(
            sync_download_jobs::Entity::find()
                .filter(sync_download_jobs::Column::LastError.is_not_null())
                .filter(sync_download_jobs::Column::Attempts.gt(0))
                .count(&target.db)
                .await?,
            1,
            "the failed job must retain its error"
        );
        if mode == AUDIO_STALL {
            assert!(start.elapsed() >= Duration::from_millis(220));
        }
        assert!(
            !final_path.exists(),
            "incomplete audio must not be published"
        );
        assert!(!is_materialized(&target.db, &target.fsio, target._root.path(), selected).await?);
        assert!(!is_materialized(&target.db, &target.fsio, target._root.path(), other).await?);
        assert_eq!(
            sync_download_jobs::Entity::find().count(&target.db).await?,
            1
        );
    }
    faults.set(FRAGMENT);
    sync_download_jobs::Entity::update_many()
        .col_expr(
            Column::RetryAt,
            sea_orm::sea_query::Expr::value("2000-01-01T00:00:00Z"),
        )
        .exec(&target.db)
        .await?;
    let report = download_requested_audio(
        target.db.clone(),
        &RecommendationDbConnection::new(),
        target.fsio.clone(),
        target.root(),
        target.node,
        &http_request::sync::MetadataPeer::new(&base, client()),
        Arc::new(|| {}),
    )
    .await?;
    assert_eq!(report.downloaded, 1);
    assert!(is_materialized(&target.db, &target.fsio, target._root.path(), selected).await?);
    assert!(!is_materialized(&target.db, &target.fsio, target._root.path(), other).await?);
    assert_eq!(
        sync_download_jobs::Entity::find().count(&target.db).await?,
        0
    );
    let row = media_files::Entity::find_by_id(selected)
        .one(&target.db)
        .await?
        .context("missing materialized row")?;
    let path = target.root().join(row.directory).join(row.file_name);
    assert_eq!(hash_file(&target.fsio, &path)?, blob);
    let stable = target.store.head().await?;
    sync_library(
        target.db.clone(),
        target.node,
        &base,
        metadata_client.clone(),
    )
    .await?;
    assert_eq!(target.store.head().await?, stable);
    server.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_link_converges_after_three_outages_with_edits() -> Result<()> {
    let started = std::time::Instant::now();
    let source = Replica::new(None).await?;
    let target = Replica::new(Some(source.store.identity().await?.namespace)).await?;
    let faults = Arc::new(Faults::default());
    let scope = album_scope();
    initialize(&source, &target, &scope).await?;
    initialize(&target, &source, &scope).await?;
    faults.set(SLOW_LINK);
    let (address, mut server) = serve(&source, faults.clone()).await?;
    let scope = album_scope();
    let limits = PageLimits {
        max_scanned: 1,
        max_bytes: 8192,
    };
    let identity = source.store.identity().await?;
    let checkpoint = CheckpointKey {
        namespace: identity.namespace,
        source_replica: identity.replica,
        source_generation: identity.generation,
        scope_id: sync::scope_id(&scope)?,
    };
    let mut keys = vec![source.album("before outages").await?];
    pull(
        target.store.as_ref(),
        &MetadataPeer::new(url(address), client()),
        &scope,
        limits,
    )
    .await?;
    let mut local_key = None;
    for round in 0..3 {
        server.abort();
        let _ = server.await;
        let before = target.store.endpoint().checkpoint(&checkpoint).await?;
        keys.push(source.album(&format!("offline edit {round}")).await?);
        if round == 1 {
            local_key = Some(target.album("other replica edit").await?);
        }
        assert!(
            pull(
                target.store.as_ref(),
                &MetadataPeer::new(url(address), client()),
                &scope,
                limits
            )
            .await
            .is_err()
        );
        assert_eq!(
            target.store.endpoint().checkpoint(&checkpoint).await?,
            before,
            "outage advanced progress"
        );
        (_, server) = serve_at(&source, faults.clone(), address).await?;
        pull(
            target.store.as_ref(),
            &MetadataPeer::new(url(address), client()),
            &scope,
            limits,
        )
        .await?;
        assert_eq!(
            target.store.endpoint().checkpoint(&checkpoint).await?,
            Some(source.store.head().await?)
        );
        for key in &keys {
            assert_eq!(target.store.get(key).await?, source.store.get(key).await?);
        }
    }
    let local_key = local_key.context("missing independent edit")?;
    let base = url(address);
    let remote = MetadataPeer::new(&base, client());
    let local = target.store.endpoint();
    let mut coordinator = sync::Coordinator::new(scope.clone(), support::limits())?;
    for step in 0..20000 {
        match coordinator
            .advance(&remote, &remote, &remote, &local, &local)
            .await?
        {
            sync::SessionAdvance::Complete(_) => break,
            sync::SessionAdvance::BudgetExhausted => coordinator.start_pass(),
            sync::SessionAdvance::Incomplete => (),
            sync::SessionAdvance::Cancelled => anyhow::bail!("unexpected cancellation"),
        }
        anyhow::ensure!(step < 19999, "push did not finish");
    }
    assert_eq!(
        source.store.get(&local_key).await?,
        target.store.get(&local_key).await?
    );
    let source_head = source.store.head().await?;
    let target_head = target.store.head().await?;
    let remote = MetadataPeer::new(&base, client());
    let local = target.store.endpoint();
    let mut coordinator = sync::Coordinator::new(scope.clone(), support::limits())?;
    for step in 0..20000 {
        match coordinator
            .advance(&remote, &remote, &remote, &local, &local)
            .await?
        {
            sync::SessionAdvance::Complete(_) => break,
            sync::SessionAdvance::BudgetExhausted => coordinator.start_pass(),
            sync::SessionAdvance::Incomplete => (),
            sync::SessionAdvance::Cancelled => anyhow::bail!("unexpected cancellation"),
        }
        anyhow::ensure!(step < 19999, "push did not finish");
    }
    assert_eq!(source.store.head().await?, source_head);
    assert_eq!(target.store.head().await?, target_head);
    for key in &keys {
        assert_eq!(target.store.get(key).await?, source.store.get(key).await?);
    }
    assert_eq!(albums::Entity::find().count(&target.db).await?, 5);
    assert_eq!(faults.blobs.load(Ordering::SeqCst), 0);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "slow profile exceeded its bounded workload"
    );
    server.abort();
    Ok(())
}

async fn initialize(source: &Replica, target: &Replica, scope: &Scope) -> Result<()> {
    support::exchange_scoped(
        &(Arc::new(source.store.endpoint()) as support::Peer),
        &(Arc::new(target.store.endpoint()) as support::Peer),
        scope.clone(),
    )
    .await?;
    Ok(())
}
async fn pull(
    local: &LibrarySyncStore,
    remote: &MetadataPeer,
    scope: &Scope,
    pages: PageLimits,
) -> Result<()> {
    let endpoint = local.endpoint();
    let mut limits = support::limits();
    limits.pages = pages;
    let mut coordinator = sync::Coordinator::new(scope.clone(), limits)?;
    for _ in 0..20000 {
        match coordinator
            .advance(&endpoint, &endpoint, &endpoint, remote, remote)
            .await?
        {
            sync::SessionAdvance::Complete(_) => return Ok(()),
            sync::SessionAdvance::BudgetExhausted => coordinator.start_pass(),
            sync::SessionAdvance::Incomplete => (),
            sync::SessionAdvance::Cancelled => anyhow::bail!("unexpected cancellation"),
        }
    }
    anyhow::bail!("pull did not finish")
}
