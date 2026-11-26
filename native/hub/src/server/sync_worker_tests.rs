use super::*;
use crate::handlers::sync::update_sync_settings;
use crate::library_pool::{LibraryHandle, LibraryPool, LibraryResources};
use crate::messages::{
    OpenDalLibraryService, OpenDalShareDataDetection, RegisterOpenDalLibraryRequest,
    ShareDataChoice,
};
use crate::node::NodeContext;
use crate::server::utils::device::RunMode;
use crate::test_wait::{TEST_DEADLINE, wait_until};
use crate::utils::nid::get_or_create_node_id;
use crate::utils::{
    Broadcaster, RinfRustSignal, detect_opendal_share_data, register_opendal_library_on_node,
};
use chrono::Utc;
use database::connection::{MainDbConnection, RecommendationDbConnection, check_storage_mode};
use database::credentials::{Credential, CredentialRef, CredentialStore, Secret};
use database::library_registry::{
    CatalogPlacement, FsAdapter, LibraryRecordId, ProtocolAdapter, resolve_library,
};
use database::sync::LibrarySyncStore;
use database::sync::model::{Collection, ObjectKey, RuneModel};
use database::sync::share::{BaseCutRequest, ShareWriter, cut_base};
use database::sync::share::{load_or_create_state, save_state};
use fsio::FsIo;
use fsio::test_fixtures::{S3Fixture, WebDavFixture};
use fsio::{OpenDalConfig, OpenDalObjectStore};
use futures::future::try_join_all;
use rcgen::CertificateParams;
use rcgen::KeyPair;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::path as std_path;
use std::path::PathBuf;
use std::sync as std_sync;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use sync::SyncModel;
use sync::protocol::BoxFuture as SyncBoxFuture;
use sync::share::{
    Checkpoint, ConditionalGet, DevicePrefix, FORMAT_VERSION, Head, LibraryFile, MemoryObjectStore,
    MemoryStoreConfig, ObjectKey as ShareObjectKey, ObjectStore, ObjectTag, ObjectValue,
    RETIREMENT_MS, StoreError, StoreOperation, base_key, decode_head, encode_library, library_key,
};
use tokio::sync::{Barrier, Notify};
use tokio::task;
use tokio::time as tokio_time;
use tokio::time::Instant;
use uuid::Uuid;
use visualizer::Visualizer;

#[path = "share_worker_tests/support.rs"]
mod support;

#[path = "share_worker_tests/pre_head_write.rs"]
mod pre_head_write;

struct MemoryShareFactory {
    store: Arc<MemoryObjectStore>,
}

impl ShareStoreFactory for MemoryShareFactory {
    fn open<'a>(
        &'a self,
        _record: &'a LibraryRecord,
        _prefix: &'a str,
        _credentials: &'a CredentialStore,
    ) -> BoxFuture<'a, Result<ShareStoreSession, ShareSyncFailure>> {
        let store: Arc<dyn ObjectStore> = self.store.clone();
        Box::pin(async move {
            Ok(ShareStoreSession {
                store,
                capabilities: ShareCapabilities::default(),
            })
        })
    }
}

struct FixedShareFactory {
    session: ShareStoreSession,
}

struct RoutedShareFactory {
    stores: HashMap<Uuid, Arc<MemoryObjectStore>>,
    opened: Arc<Mutex<HashMap<Uuid, usize>>>,
}

impl ShareStoreFactory for FixedShareFactory {
    fn open<'a>(
        &'a self,
        _record: &'a LibraryRecord,
        _prefix: &'a str,
        _credentials: &'a CredentialStore,
    ) -> BoxFuture<'a, Result<ShareStoreSession, ShareSyncFailure>> {
        let session = self.session.clone();
        Box::pin(async move { Ok(session) })
    }
}

impl ShareStoreFactory for RoutedShareFactory {
    fn open<'a>(
        &'a self,
        record: &'a LibraryRecord,
        _prefix: &'a str,
        _credentials: &'a CredentialStore,
    ) -> BoxFuture<'a, Result<ShareStoreSession, ShareSyncFailure>> {
        let Some(library_id) = record.library_id else {
            return Box::pin(async { Err(ShareSyncFailure::ProbeFailed) });
        };
        let Some(store) = self.stores.get(&library_id).cloned() else {
            return Box::pin(async { Err(ShareSyncFailure::ProbeFailed) });
        };
        *self
            .opened
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(library_id)
            .or_default() += 1;
        let store: Arc<dyn ObjectStore> = store;
        Box::pin(async move {
            Ok(ShareStoreSession {
                store,
                capabilities: ShareCapabilities::default(),
            })
        })
    }
}

fn test_record(library_id: Uuid) -> LibraryRecord {
    LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: "share-test".into(),
        library_id: Some(library_id),
        protocol: ProtocolAdapter::Native,
        fs: vec![FsAdapter::WebDav {
            endpoint: "http://localhost".into(),
            root: "/share".into(),
            username: "user".into(),
            credential: CredentialRef(Uuid::new_v4()),
        }],
        catalog: CatalogPlacement::InShare(library_id),
    }
}

async fn memory_session(store: &Arc<MemoryObjectStore>) -> ShareStoreSession {
    let factory = MemoryShareFactory {
        store: store.clone(),
    };
    let directory = tempfile::tempdir().unwrap();
    let credentials = CredentialStore::open(directory.path());
    factory
        .open(&test_record(Uuid::new_v4()), "probe-prefix", &credentials)
        .await
        .unwrap()
}

async fn sync_pass(
    library: &LibrarySyncStore,
    library_id: Uuid,
    session: &ShareStoreSession,
) -> Result<(), ShareSyncFailure> {
    let cache = env::temp_dir().join(format!("hub-share-{}", Uuid::new_v4()));
    share_sync_pass(
        library,
        library_id,
        cache,
        session,
        SharePass::PullAndPush,
        || {},
    )
    .await
    .map(|_| ())
}

async fn detailed_sync_pass(
    library: &LibrarySyncStore,
    library_id: Uuid,
    session: &ShareStoreSession,
) -> Result<()> {
    let cache = env::temp_dir().join(format!("hub-share-{}", Uuid::new_v4()));
    library
        .pull_share(
            session.store.clone(),
            library_id,
            cache.clone(),
            session.capabilities,
        )
        .await
        .map_err(|error| anyhow::anyhow!("pull: {error:?}"))?;
    library
        .push_share(
            session.store.clone(),
            library_id,
            cache,
            session.capabilities,
        )
        .await
        .map_err(|error| anyhow::anyhow!("push: {error:?}"))?;
    Ok(())
}

/// A playlist name whose compressed bytes keep a base well above a small update's quarter, so a
/// worker test that needs a segment keeps publishing one under the base-fraction rule.
fn large_playlist_name() -> String {
    (0..400).map(|_| Uuid::new_v4().to_string()).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShareBackend {
    WebDav,
    S3,
}

enum ShareFixture {
    WebDav { _server: WebDavFixture },
    S3 { _server: S3Fixture },
}

impl ShareFixture {
    fn conflict_next_conditional_put(&self) {
        if let Self::S3 { _server } = self {
            _server.conflict_next_conditional_put();
        }
    }

    fn conditional_put_conflict_pending(&self) -> bool {
        match self {
            Self::S3 { _server } => _server.conditional_put_conflict_pending(),
            Self::WebDav { .. } => false,
        }
    }

    fn reject_writes(&self) {
        match self {
            Self::WebDav { _server } => _server.reject_writes(),
            Self::S3 { _server } => _server.reject_writes(),
        }
    }
}

async fn share_fixture(backend: ShareBackend) -> (ShareFixture, OpenDalConfig) {
    match backend {
        ShareBackend::WebDav => {
            let fixture = WebDavFixture::start(Vec::new()).await.unwrap();
            let config = OpenDalConfig::WebDav {
                endpoint: fixture.endpoint.clone(),
                root: "/dav/library/".into(),
                username: "user".into(),
                password: "pass".into(),
            };
            (ShareFixture::WebDav { _server: fixture }, config)
        }
        ShareBackend::S3 => {
            let fixture = S3Fixture::start(Vec::new()).await.unwrap();
            let config = OpenDalConfig::S3 {
                endpoint: fixture.endpoint.clone(),
                bucket: "bucket".into(),
                region: "us-east-1".into(),
                root: "/library/".into(),
                access_key_id: "access".into(),
                secret_access_key: "secret".into(),
            };
            (ShareFixture::S3 { _server: fixture }, config)
        }
    }
}

async fn real_share(backend: ShareBackend) -> (ShareFixture, ShareStoreSession) {
    let (fixture, config) = share_fixture(backend).await;
    (fixture, opendal_share_session(config).await)
}

/// A session on the OpenDAL share `config` names, with the conditional features its probe found.
async fn opendal_share_session(config: OpenDalConfig) -> ShareStoreSession {
    let store = Arc::new(OpenDalObjectStore::new(config).unwrap());
    let capabilities = store.probe("devices/probe.generation").await.unwrap();
    ShareStoreSession {
        store,
        capabilities: ShareCapabilities {
            conditional_create: capabilities.conditional_create,
            conditional_head_write: capabilities.conditional_put,
            conditional_get: capabilities.conditional_get,
        },
    }
}

/// The storage adapter of a library kept in the share at `config`, with the share's
/// secret saved in `credentials` as the Add Library guide saves it.
async fn share_adapter(config: OpenDalConfig, credentials: &CredentialStore) -> Result<FsAdapter> {
    Ok(match config {
        OpenDalConfig::WebDav {
            endpoint,
            root,
            username,
            password,
        } => FsAdapter::WebDav {
            credential: credentials
                .put(Credential {
                    username: username.clone(),
                    secret: Secret::new(password.into_bytes()),
                })
                .await?,
            endpoint,
            root,
            username,
        },
        OpenDalConfig::S3 {
            endpoint,
            bucket,
            region,
            root,
            access_key_id,
            secret_access_key,
        } => FsAdapter::S3 {
            credential: credentials
                .put(Credential {
                    username: access_key_id.clone(),
                    secret: Secret::new(secret_access_key.into_bytes()),
                })
                .await?,
            endpoint,
            bucket,
            region,
            root,
            access_key_id,
        },
    })
}

async fn hubs_on_share(namespace: Uuid, count: usize) -> Vec<support::SqlLibrary> {
    let mut hubs = Vec::with_capacity(count);
    for _ in 0..count {
        hubs.push(
            support::SqlLibrary::new(namespace, Uuid::new_v4())
                .await
                .unwrap(),
        );
    }
    hubs
}

#[derive(Default)]
struct QuietBroadcaster {
    messages: std_sync::Mutex<Vec<String>>,
    share_statuses: std_sync::Mutex<Vec<Option<ShareSyncFailure>>>,
    share_statuses_by_library: std_sync::Mutex<Vec<(String, Option<ShareSyncFailure>)>>,
    successful_share_passes: AtomicUsize,
    successful_share_pass: Notify,
}

impl Broadcaster for QuietBroadcaster {
    fn broadcast(&self, message: &dyn RinfRustSignal) {
        if let Some(status) = message.as_any().downcast_ref::<LibrarySyncStatusUpdated>() {
            self.share_statuses
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(status.share_failure);
            self.share_statuses_by_library
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push((status.library_id.clone(), status.share_failure));
            if status.share_failure.is_none() {
                self.successful_share_passes.fetch_add(1, Ordering::SeqCst);
                self.successful_share_pass.notify_waiters();
            }
        }
        let status = if message.name() == "LibrarySyncStatusUpdated" {
            format!(
                " status_tail={:?}",
                message.encode_to_vec().map(|bytes| bytes
                    .into_iter()
                    .rev()
                    .take(8)
                    .collect::<Vec<_>>())
            )
        } else {
            String::new()
        };
        self.messages
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(format!("{}{status}", message.name()));
    }
}

struct WorkerHub {
    _directory: tempfile::TempDir,
    node: Arc<NodeContext>,
    namespace: Uuid,
    library: support::SqlLibrary,
    broadcaster: Arc<QuietBroadcaster>,
    handle: Option<LibraryHandle>,
    cancel: CancellationToken,
    task: Option<task::JoinHandle<()>>,
    factory: Arc<dyn ShareStoreFactory>,
    timers: ShareWorkerTimers,
}

impl WorkerHub {
    fn start_worker(&mut self) {
        let worker_cancel = self.cancel.clone();
        let worker_node = Arc::downgrade(&self.node);
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        self.task = Some(tokio::spawn(run_share_worker(
            worker_node,
            notify,
            worker_cancel,
            self.factory.clone(),
            self.timers,
        )));
    }

    async fn stop_worker(&mut self) -> Result<()> {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            task.await?;
        }
        Ok(())
    }

    async fn restart_node(&mut self) -> Result<()> {
        self.stop_worker().await?;
        self.handle.take();
        self.node.shutdown().await?;
        self._directory = tempfile::tempdir()?;
        let (node, broadcaster, handle) =
            register_worker_library(&self._directory, test_record(self.namespace), &self.library)
                .await?;
        self.node = node;
        self.broadcaster = broadcaster;
        self.handle = Some(handle);
        self.cancel = CancellationToken::new();
        let worker_cancel = self.cancel.clone();
        let worker_node = Arc::downgrade(&self.node);
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        self.task = Some(tokio::spawn(run_share_worker(
            worker_node,
            notify,
            worker_cancel,
            self.factory.clone(),
            self.timers,
        )));
        Ok(())
    }

    async fn stop(self) -> Result<()> {
        let mut this = self;
        this.stop_worker().await?;
        this.handle.take();
        this.node.shutdown().await?;
        Ok(())
    }
}

/// Stops the worker a node starts for itself, so that the share worker a test runs is the only
/// one: a share worker holds the node's share sync requests for as long as it runs, and the
/// node's own worker would also work and report on the test's libraries.
async fn stop_own_worker(node: &NodeContext) -> Result<()> {
    node.manager()?.stop_worker().await
}

/// A node generates an RSA device certificate on first start, which takes seconds in a
/// debug build, and the share worker never uses it. Starting from an elliptic-curve
/// identity written beforehand, made in milliseconds, avoids that wait.
fn write_quick_identity(directory: &std_path::Path) -> Result<()> {
    let key = KeyPair::generate()?;
    let certificate = CertificateParams::new(vec!["share-test".to_owned()])?.self_signed(&key)?;
    fs::write(directory.join("cid"), "R-share-test")?;
    fs::write(directory.join("certificate.pem"), certificate.pem())?;
    fs::write(directory.join("private_key.pem"), key.serialize_pem())?;
    Ok(())
}

async fn register_worker_library(
    directory: &tempfile::TempDir,
    record: LibraryRecord,
    library: &support::SqlLibrary,
) -> Result<(Arc<NodeContext>, Arc<QuietBroadcaster>, LibraryHandle)> {
    let namespace = record
        .catalog_library_id()
        .context("share test record has no library id")?;
    let path = directory.path().to_string_lossy().into_owned();
    write_quick_identity(directory.path())?;
    crate::test_node::disable_media_server(directory.path())?;
    let broadcaster = Arc::new(QuietBroadcaster::default());
    let node = NodeContext::new(
        &path,
        &path,
        broadcaster.clone(),
        false,
        RunMode::App,
        Arc::new(Visualizer::new()),
    )
    .await?;
    // Stopped while it has no library to work on, so nothing of it is cut off half way.
    stop_own_worker(&node).await?;
    fs::write(
        directory.path().join("libraries.json"),
        serde_json::to_vec(&json!({
            "version": 3,
            "libraries": [record],
            "last_open_record": null,
        }))?,
    )?;
    let resources = LibraryResources {
        library_id: namespace,
        display_name: "SH15 test library".into(),
        main_db: Arc::new(library.db.clone()),
        library_sync: library.sync_store.clone(),
        recommend_db: Arc::new(RecommendationDbConnection::new()),
        fsio: Arc::new(FsIo::new()),
        media_root: library.root.path().to_string_lossy().into_owned(),
        descriptor: record,
    };
    let handle = node.library_pool.register_ui(resources).await?;
    anyhow::ensure!(
        node.library_pool.served_library_ids().await? == [namespace],
        "share test library is not served by its hub"
    );
    Ok((node, broadcaster, handle))
}

async fn worker_hubs_on_share_sessions(
    namespace: Uuid,
    sessions: Vec<ShareStoreSession>,
) -> Result<Vec<WorkerHub>> {
    let mut hubs = Vec::with_capacity(sessions.len());
    for session in sessions {
        let library = support::SqlLibrary::new_persistent(namespace, Uuid::new_v4()).await?;
        hubs.push(worker_hub_on_share(namespace, session, library).await?);
    }
    Ok(hubs)
}

async fn worker_hub_on_share(
    namespace: Uuid,
    session: ShareStoreSession,
    library: support::SqlLibrary,
) -> Result<WorkerHub> {
    let mut hub = worker_hub_on_share_prepared(namespace, session, library).await?;
    hub.start_worker();
    Ok(hub)
}

fn fast_share_timers() -> ShareWorkerTimers {
    ShareWorkerTimers {
        tick: Duration::from_millis(25),
        foreground_pull: Duration::from_millis(50),
        background_pull: Duration::from_millis(50),
        push_debounce: Duration::from_millis(10),
        statistics_push: Duration::from_millis(50),
        idle_push: Duration::from_millis(100),
    }
}

async fn worker_hub_on_share_prepared(
    namespace: Uuid,
    session: ShareStoreSession,
    library: support::SqlLibrary,
) -> Result<WorkerHub> {
    let timers = fast_share_timers();
    let factory: Arc<dyn ShareStoreFactory> = Arc::new(FixedShareFactory { session });
    let directory = tempfile::tempdir()?;
    let (node, broadcaster, handle) =
        register_worker_library(&directory, test_record(namespace), &library).await?;
    let cancel = CancellationToken::new();
    Ok(WorkerHub {
        _directory: directory,
        node,
        namespace,
        library,
        broadcaster,
        handle: Some(handle),
        cancel,
        task: None,
        factory,
        timers,
    })
}

#[tokio::test]
async fn share_worker_fixtures_do_not_start_the_media_server() -> Result<()> {
    let session = memory_session(&Arc::new(MemoryObjectStore::default())).await;
    let hubs =
        worker_hubs_on_share_sessions(Uuid::new_v4(), vec![session.clone(), session]).await?;
    let mut listeners = Vec::new();
    for hub in &hubs {
        let manager = hub.node.manager()?;
        listeners.push((
            hub.node.sync_settings.read().await.media_server,
            manager.get_address().await,
        ));
    }
    for hub in hubs {
        hub.stop().await?;
    }
    assert_eq!(
        listeners,
        [(false, None), (false, None)],
        "share workers must not request or start the media server"
    );
    Ok(())
}

/// A node with an ordinary, background-synced local library whose pool closes it after a
/// controlled idle timeout. The pool is created with the timeout before the node, which reuses
/// it, and the library opens through its registry record like a backgrounded real library.
struct IdlePatrolNode {
    _directory: tempfile::TempDir,
    node: Arc<NodeContext>,
    // The node reuses this pool only while the strong reference lives, so the harness keeps it.
    pool: Arc<LibraryPool>,
    library_id: Uuid,
    cancel: CancellationToken,
    task: Option<task::JoinHandle<()>>,
}

impl IdlePatrolNode {
    async fn new(idle_timeout: Duration) -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let root = directory
            .path()
            .join("media")
            .to_string_lossy()
            .into_owned();
        let fsio = FsIo::new();
        let mode = check_storage_mode(&fsio, &root)?;
        let mut registry = LibraryRegistry::load(directory.path())?;
        let record = resolve_library(&fsio, &mut registry, &root, "Idle test", mode).await?;
        let library_id = record.catalog_library_id().context("local library id")?;
        let path = directory.path().to_string_lossy().into_owned();
        write_quick_identity(directory.path())?;
        crate::test_node::disable_media_server(directory.path())?;
        get_or_create_node_id(&path).await?;
        let credentials = Arc::new(CredentialStore::open(directory.path()));
        let node_id = node_id_file(directory.path())?;
        let pool = LibraryPool::with_idle_timeout_and_credentials(
            directory.path(),
            directory.path(),
            &node_id,
            idle_timeout,
            credentials,
        )?;
        let broadcaster = Arc::new(QuietBroadcaster::default());
        let node = NodeContext::new(
            &path,
            &path,
            broadcaster,
            false,
            RunMode::App,
            Arc::new(Visualizer::new()),
        )
        .await?;
        anyhow::ensure!(
            Arc::ptr_eq(&pool, &node.library_pool),
            "the node did not reuse the controlled-timeout pool"
        );
        Ok(Self {
            _directory: directory,
            node,
            pool,
            library_id,
            cancel: CancellationToken::new(),
            task: None,
        })
    }

    fn start_worker(&mut self) {
        let cancel = self.cancel.clone();
        let node = Arc::downgrade(&self.node);
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        self.task = Some(tokio::spawn(run_share_worker(
            node,
            notify,
            cancel,
            Arc::new(MemoryShareFactory {
                store: Arc::new(MemoryObjectStore::default()),
            }),
            fast_share_timers(),
        )));
    }

    /// The pool's connection to the library, acquired as background work so the probe itself
    /// cannot refresh the idle clock. Reuses the open entry or reopens the library.
    async fn probe_connection(&self) -> Result<Arc<MainDbConnection>> {
        Ok(self
            .pool
            .acquire_background(self.library_id)
            .await?
            .main_db
            .clone())
    }

    /// The connection changes identity when the pool closed the idle database and the next
    /// access reopened it.
    async fn wait_for_reopen_after(&self, connection: &Arc<MainDbConnection>) -> Result<()> {
        wait_until("the idle library to close and reopen", || async {
            let probe = self.probe_connection().await?;
            Ok((!Arc::ptr_eq(&probe, connection)).then_some(()))
        })
        .await
    }

    async fn stop(mut self) -> Result<()> {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            task.await?;
        }
        self.node.shutdown().await?;
        Ok(())
    }
}

fn node_id_file(config_dir: &std_path::Path) -> Result<String> {
    Ok(fs::read_to_string(config_dir.join("nid"))?
        .trim()
        .to_owned())
}

#[tokio::test]
async fn share_patrol_of_an_unshared_library_allows_idle_closure() -> Result<()> {
    let idle_timeout = Duration::from_millis(400);
    let mut fixture = IdlePatrolNode::new(idle_timeout).await?;
    let library_id = fixture.library_id;
    fixture.start_worker();

    // The patrol does not visit a library that is not stored in a shared folder, so this
    // probe is what opens it.
    let initial = fixture.probe_connection().await?;
    initial
        .execute_unprepared("CREATE TABLE share_idle_probe (value INTEGER NOT NULL)")
        .await?;
    initial
        .execute_unprepared("INSERT INTO share_idle_probe (value) VALUES (42)")
        .await?;

    // Background visits continue, yet once external use stops the library still closes after
    // the idle timeout, and the next access reopens it.
    fixture.wait_for_reopen_after(&initial).await?;
    let reopened = fixture.probe_connection().await?;
    let result = reopened
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT value FROM share_idle_probe".to_owned(),
        ))
        .await?;
    assert_eq!(
        result
            .context("persisted probe data")?
            .try_get::<i64>("", "value")?,
        42
    );

    // An active lease keeps the library open across several idle periods. Eviction is only
    // time-driven, so this negative observation needs one bounded wait rather than a condition.
    let lease = fixture.node.library_pool.acquire(library_id).await?;
    let leased_connection = lease.main_db.clone();
    tokio_time::sleep(idle_timeout * 5).await;
    let probe = fixture.probe_connection().await?;
    assert!(
        Arc::ptr_eq(&probe, &leased_connection),
        "the idle library closed while a lease was held"
    );
    drop(lease);
    drop(probe);
    fixture.wait_for_reopen_after(&leased_connection).await?;

    fixture.stop().await?;
    Ok(())
}

async fn wait_for_key_on_hubs(hubs: &[WorkerHub], key: &ObjectKey) -> Result<()> {
    wait_until("share key to reach every hub", || async {
        let records = try_join_all(hubs.iter().map(|hub| hub.library.sync_store.get(key))).await?;
        let present = records.iter().all(Option::is_some);
        Ok(present.then_some(()))
    })
    .await
}

async fn wait_for_keys_on_hubs(hubs: &[WorkerHub], keys: &[ObjectKey]) -> Result<()> {
    wait_until("share keys to reach every hub", || async {
        let records = try_join_all(
            hubs.iter()
                .flat_map(|hub| keys.iter().map(|key| hub.library.sync_store.get(key))),
        )
        .await?;
        let present = records.iter().all(Option::is_some);
        Ok(present.then_some(()))
    })
    .await
}

async fn key_presence_on_hubs(hubs: &[WorkerHub], key: &ObjectKey) -> Result<String> {
    let mut states = Vec::with_capacity(hubs.len());
    for (index, hub) in hubs.iter().enumerate() {
        let replica = hub.library.peer.identity().await?.replica;
        let present = hub.library.sync_store.get(key).await?.is_some();
        states.push(format!("hub#{index} replica={replica} present={present}"));
    }
    Ok(states.join(", "))
}

async fn key_presence_matrix(hubs: &[WorkerHub], keys: &[ObjectKey]) -> Result<String> {
    let mut states = Vec::with_capacity(hubs.len() * keys.len());
    for (writer, key) in keys.iter().enumerate() {
        for (index, hub) in hubs.iter().enumerate() {
            let replica = hub.library.peer.identity().await?.replica;
            let present = hub.library.sync_store.get(key).await?.is_some();
            states.push(format!(
                "writer#{writer} key={} hub#{index} replica={replica} present={present}",
                key.id
            ));
        }
    }
    Ok(states.join(", "))
}

async fn wait_for_replica_head(store: &dyn ObjectStore, replica: Uuid) -> Result<()> {
    wait_until("share replica head to appear", || async {
        for key in store
            .list_prefix("devices/")
            .await
            .map_err(|error| anyhow::anyhow!("list replica heads: {error:?}"))?
        {
            if !key.contains("/head.") {
                continue;
            }
            let value = store
                .get(&key)
                .await
                .map_err(|error| anyhow::anyhow!("get replica head: {error:?}"))?;
            if decode_head(&value.bytes)?.local_replica == replica {
                return Ok(Some(()));
            }
        }
        Ok(None)
    })
    .await
}

struct CrashBetweenSegmentAndHead {
    inner: Arc<dyn ObjectStore>,
    armed: AtomicBool,
    segment_written: AtomicBool,
    failed: AtomicBool,
    reached: Notify,
    resume: Notify,
}

struct DeleteBaseBeforeRead {
    inner: Arc<dyn ObjectStore>,
    deleted: AtomicBool,
}

struct RaceHeadPutStore {
    inner: Arc<dyn ObjectStore>,
    barrier: Barrier,
    waiting: AtomicUsize,
}

impl CrashBetweenSegmentAndHead {
    fn arm(&self) {
        self.segment_written.store(false, Ordering::SeqCst);
        self.failed.store(false, Ordering::SeqCst);
        self.armed.store(true, Ordering::SeqCst);
    }

    async fn wait_for_head_failure(&self) {
        self.reached.notified().await;
    }

    fn resume_after_crash(&self) {
        self.armed.store(false, Ordering::SeqCst);
        self.resume.notify_one();
    }

    fn intercept(&self, key: &str) -> bool {
        self.armed.load(Ordering::SeqCst)
            && self.segment_written.load(Ordering::SeqCst)
            && key.starts_with("devices/")
            && key.contains("/head.")
            && !self.failed.swap(true, Ordering::SeqCst)
    }
}

impl ObjectStore for CrashBetweenSegmentAndHead {
    fn get<'a>(&'a self, key: &'a str) -> SyncBoxFuture<'a, ObjectValue, StoreError> {
        self.inner.get(key)
    }

    fn get_if_none_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
    ) -> SyncBoxFuture<'a, ConditionalGet, StoreError> {
        self.inner.get_if_none_match(key, tag)
    }

    fn put<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        if self.intercept(key) {
            return Box::pin(async move {
                self.reached.notify_one();
                self.resume.notified().await;
                Err(StoreError::Transient("simulated process crash".into()))
            });
        }
        Box::pin(async move {
            let tag = self.inner.put(key, bytes).await?;
            if key.contains("/seg/") {
                self.segment_written.store(true, Ordering::SeqCst);
            }
            Ok(tag)
        })
    }

    fn create<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        if self.intercept(key) {
            return Box::pin(async move {
                self.reached.notify_one();
                self.resume.notified().await;
                Err(StoreError::Transient("simulated process crash".into()))
            });
        }
        Box::pin(async move {
            let tag = self.inner.create(key, bytes).await?;
            if key.contains("/seg/") {
                self.segment_written.store(true, Ordering::SeqCst);
            }
            Ok(tag)
        })
    }

    fn put_if_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        if self.intercept(key) {
            return Box::pin(async move {
                self.reached.notify_one();
                self.resume.notified().await;
                Err(StoreError::Transient("simulated process crash".into()))
            });
        }
        Box::pin(async move {
            let updated = self.inner.put_if_match(key, tag, bytes).await?;
            if key.contains("/seg/") {
                self.segment_written.store(true, Ordering::SeqCst);
            }
            Ok(updated)
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> SyncBoxFuture<'a, (), StoreError> {
        self.inner.delete(key)
    }

    fn list_prefix<'a>(&'a self, prefix: &'a str) -> SyncBoxFuture<'a, Vec<String>, StoreError> {
        self.inner.list_prefix(prefix)
    }
}

struct CountNewChunkObjects {
    inner: Arc<dyn ObjectStore>,
    attempted_chunk_writes: AtomicUsize,
}

impl ObjectStore for CountNewChunkObjects {
    fn get<'a>(&'a self, key: &'a str) -> SyncBoxFuture<'a, ObjectValue, StoreError> {
        self.inner.get(key)
    }

    fn get_if_none_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
    ) -> SyncBoxFuture<'a, ConditionalGet, StoreError> {
        self.inner.get_if_none_match(key, tag)
    }

    fn put<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            if key.contains("/obj/") {
                self.attempted_chunk_writes.fetch_add(1, Ordering::SeqCst);
            }
            self.inner.put(key, bytes).await
        })
    }

    fn create<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            if key.contains("/obj/") {
                self.attempted_chunk_writes.fetch_add(1, Ordering::SeqCst);
            }
            self.inner.create(key, bytes).await
        })
    }

    fn put_if_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            if key.contains("/obj/") {
                self.attempted_chunk_writes.fetch_add(1, Ordering::SeqCst);
            }
            self.inner.put_if_match(key, tag, bytes).await
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> SyncBoxFuture<'a, (), StoreError> {
        self.inner.delete(key)
    }

    fn list_prefix<'a>(&'a self, prefix: &'a str) -> SyncBoxFuture<'a, Vec<String>, StoreError> {
        self.inner.list_prefix(prefix)
    }
}

impl ObjectStore for RaceHeadPutStore {
    fn get<'a>(&'a self, key: &'a str) -> SyncBoxFuture<'a, ObjectValue, StoreError> {
        self.inner.get(key)
    }

    fn get_if_none_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
    ) -> SyncBoxFuture<'a, ConditionalGet, StoreError> {
        self.inner.get_if_none_match(key, tag)
    }

    fn put<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            if key.contains("/head.") {
                self.waiting.fetch_add(1, Ordering::SeqCst);
                self.barrier.wait().await;
            }
            self.inner.put(key, bytes).await
        })
    }

    fn create<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        self.inner.create(key, bytes)
    }

    fn put_if_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            if key.contains("/head.") {
                self.waiting.fetch_add(1, Ordering::SeqCst);
                self.barrier.wait().await;
            }
            self.inner.put_if_match(key, tag, bytes).await
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> SyncBoxFuture<'a, (), StoreError> {
        self.inner.delete(key)
    }

    fn list_prefix<'a>(&'a self, prefix: &'a str) -> SyncBoxFuture<'a, Vec<String>, StoreError> {
        self.inner.list_prefix(prefix)
    }
}

impl DeleteBaseBeforeRead {
    fn intercept(&self, key: &str) -> bool {
        key.contains("/base/") && !self.deleted.swap(true, Ordering::SeqCst)
    }
}

impl ObjectStore for DeleteBaseBeforeRead {
    fn get<'a>(&'a self, key: &'a str) -> SyncBoxFuture<'a, ObjectValue, StoreError> {
        if self.intercept(key) {
            return Box::pin(async move {
                self.inner.delete(key).await?;
                self.inner.get(key).await
            });
        }
        self.inner.get(key)
    }

    fn get_if_none_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
    ) -> SyncBoxFuture<'a, ConditionalGet, StoreError> {
        if self.intercept(key) {
            return Box::pin(async move {
                self.inner.delete(key).await?;
                self.inner.get_if_none_match(key, tag).await
            });
        }
        self.inner.get_if_none_match(key, tag)
    }

    fn put<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        self.inner.put(key, bytes)
    }

    fn create<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        self.inner.create(key, bytes)
    }

    fn put_if_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        self.inner.put_if_match(key, tag, bytes)
    }

    fn delete<'a>(&'a self, key: &'a str) -> SyncBoxFuture<'a, (), StoreError> {
        self.inner.delete(key)
    }

    fn list_prefix<'a>(&'a self, prefix: &'a str) -> SyncBoxFuture<'a, Vec<String>, StoreError> {
        self.inner.list_prefix(prefix)
    }
}

#[tokio::test]
async fn worker_factory_share_passes_converge_and_relay_edits() -> Result<()> {
    let namespace = Uuid::new_v4();
    let first = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let second = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    let session = memory_session(&store).await;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    };
    first
        .sync_store
        .write_local(key.clone(), support::playlist("from first device"))
        .await?;
    sync_pass(&first.sync_store, namespace, &session)
        .await
        .unwrap();
    sync_pass(&second.sync_store, namespace, &session)
        .await
        .unwrap();
    assert!(second.sync_store.get(&key).await?.is_some());

    let next_key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    };
    first
        .sync_store
        .write_local(next_key.clone(), support::playlist("later edit"))
        .await?;
    sync_pass(&first.sync_store, namespace, &session)
        .await
        .unwrap();
    sync_pass(&second.sync_store, namespace, &session)
        .await
        .unwrap();
    assert!(second.sync_store.get(&next_key).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn read_only_share_reports_typed_failure_without_objects() -> Result<()> {
    let namespace = Uuid::new_v4();
    let library = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    store.inject_error(StoreOperation::Put, StoreError::PermissionDenied);
    let session = memory_session(&store).await;
    let result = sync_pass(&library.sync_store, namespace, &session).await;
    assert_eq!(result, Err(ShareSyncFailure::ReadOnlyShare));
    assert!(store.list_prefix("").await.unwrap().is_empty());
    Ok(())
}

#[tokio::test]
async fn share_worker_skips_redirected_catalog_before_opening_a_store() -> Result<()> {
    let redirected_id = Uuid::new_v4();
    let share_id = Uuid::new_v4();
    let mut redirected_record = test_record(redirected_id);
    redirected_record.catalog = CatalogPlacement::Redirected(Uuid::new_v4());
    let share_record = test_record(share_id);
    let redirected_library =
        support::SqlLibrary::new_persistent(redirected_id, Uuid::new_v4()).await?;
    let share_library = support::SqlLibrary::new_persistent(share_id, Uuid::new_v4()).await?;
    let directory = tempfile::tempdir()?;
    let path = directory.path().to_string_lossy().into_owned();
    let broadcaster = Arc::new(QuietBroadcaster::default());
    crate::test_node::disable_media_server(directory.path())?;
    let node = NodeContext::new(
        &path,
        &path,
        broadcaster,
        false,
        RunMode::App,
        Arc::new(Visualizer::new()),
    )
    .await?;
    stop_own_worker(&node).await?;
    fs::write(
        directory.path().join("libraries.json"),
        serde_json::to_vec(&json!({
            "version": 3,
            "libraries": [redirected_record.clone(), share_record.clone()],
            "last_open_record": null,
        }))?,
    )?;
    let make_resources =
        |library_id: Uuid, library: &support::SqlLibrary, record: LibraryRecord| LibraryResources {
            library_id,
            display_name: record.alias.clone(),
            main_db: Arc::new(library.db.clone()),
            library_sync: library.sync_store.clone(),
            recommend_db: Arc::new(RecommendationDbConnection::new()),
            fsio: Arc::new(FsIo::new()),
            media_root: library.root.path().to_string_lossy().into_owned(),
            descriptor: record,
        };
    let redirected_handle = node
        .library_pool
        .register_ui(make_resources(
            redirected_id,
            &redirected_library,
            redirected_record,
        ))
        .await?;
    let share_handle = node
        .library_pool
        .register_ui(make_resources(share_id, &share_library, share_record))
        .await?;
    anyhow::ensure!(
        node.library_pool.served_library_ids().await? == [redirected_id, share_id],
        "redirected catalog must precede the share status barrier"
    );

    let redirected_store = Arc::new(MemoryObjectStore::default());
    let share_store = Arc::new(MemoryObjectStore::default());
    let opened = Arc::new(Mutex::new(HashMap::new()));
    let factory: Arc<dyn ShareStoreFactory> = Arc::new(RoutedShareFactory {
        stores: HashMap::from([
            (redirected_id, redirected_store.clone()),
            (share_id, share_store.clone()),
        ]),
        opened: opened.clone(),
    });
    let cancel = CancellationToken::new();
    let task = tokio::spawn(run_share_worker(
        Arc::downgrade(&node),
        Arc::new(|| {}),
        cancel.clone(),
        factory,
        ShareWorkerTimers {
            tick: Duration::from_millis(25),
            foreground_pull: Duration::from_millis(50),
            background_pull: Duration::from_millis(50),
            push_debounce: Duration::from_millis(10),
            statistics_push: Duration::from_millis(50),
            idle_push: Duration::from_millis(100),
        },
    ));
    wait_until("the later in-share catalog store request", || async {
        Ok((opened
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&share_id)
            .copied()
            .unwrap_or_default()
            > 0
            && !share_store.calls().is_empty())
        .then_some(()))
    })
    .await?;
    assert!(redirected_store.calls().is_empty());
    assert_eq!(
        opened
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&redirected_id),
        None
    );

    cancel.cancel();
    task.await?;
    drop((redirected_handle, share_handle));
    node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn read_only_webdav_in_share_probe_reports_typed_state_without_persisting_writes()
-> Result<()> {
    let server = WebDavFixture::start_read_only(Vec::new()).await?;
    let directory = tempfile::tempdir()?;
    let credentials = CredentialStore::open(directory.path());
    let credential = credentials
        .put(Credential {
            username: "user".into(),
            secret: Secret::new(b"pass".to_vec()),
        })
        .await?;
    let mut record = test_record(Uuid::new_v4());
    record.catalog = CatalogPlacement::InShare(Uuid::new_v4());
    record.fs = vec![FsAdapter::WebDav {
        endpoint: server.endpoint.clone(),
        root: "/dav/library/".into(),
        username: "user".into(),
        credential,
    }];

    let prefix = DevicePrefix {
        stream: Uuid::new_v4(),
        generation: Uuid::new_v4(),
    };
    let result = OpenDalShareStoreFactory
        .open(&record, &prefix.to_string(), &credentials)
        .await;
    assert_eq!(
        result.as_ref().err().copied(),
        Some(ShareSyncFailure::ReadOnlyShare),
        "factory returned {:?}; recorded operations: {:?}",
        result.as_ref().err(),
        server.write_requests()
    );
    let refused_operations = server.write_requests();
    assert!(
        !refused_operations.is_empty(),
        "share writability probe did not run"
    );
    assert!(
        refused_operations
            .iter()
            .all(|operation| matches!(operation.as_str(), "MKCOL" | "PUT" | "DELETE")),
        "unexpected write operation in share probe: {refused_operations:?}"
    );
    Ok(())
}

/// The share worker opens a WebDAV or S3 share through `OpenDalShareStoreFactory` with this
/// device's `<stream>.<generation>` prefix. Its first pass has to get through the store
/// probe and publish the device's head in its own folder of the share.
#[tokio::test]
async fn share_worker_syncs_webdav_and_s3_shares_through_the_opendal_factory() -> Result<()> {
    for backend in [ShareBackend::WebDav, ShareBackend::S3] {
        let (_fixture, config) = share_fixture(backend).await;
        let namespace = Uuid::new_v4();
        let library = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        let directory = tempfile::tempdir()?;
        let mut record = test_record(namespace);
        record.fs =
            vec![share_adapter(config.clone(), &CredentialStore::open(directory.path())).await?];
        // The node's own sync worker runs here: its share worker opens stores through
        // `OpenDalShareStoreFactory`, as in the app.
        let (node, broadcaster, handle) =
            register_worker_library(&directory, record, &library).await?;
        node.manager()?.start_worker().await?;
        let first_status = wait_until("the share worker's first pass", || async {
            Ok(broadcaster
                .share_statuses
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .first()
                .copied())
        })
        .await?;
        assert_eq!(
            first_status, None,
            "{backend:?}: the share worker's first pass failed"
        );

        let prefix = library
            .sync_store
            .share_prefix(
                PathBuf::from(node.config_path.as_str())
                    .join("share-cache")
                    .join(namespace.to_string()),
            )
            .await?;
        let store =
            OpenDalObjectStore::new(config).map_err(|error| anyhow::anyhow!("{error:?}"))?;
        let objects = store
            .list_prefix(&format!("devices/{prefix}/"))
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert!(
            objects
                .iter()
                .any(|key| key.starts_with(&format!("devices/{prefix}/head."))),
            "{backend:?}: no head in devices/{prefix}/ of the share: {objects:?}"
        );
        drop(handle);
        node.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn sh15_three_hub_concurrent_push_obeys_backend_capabilities() -> Result<()> {
    for backend in [ShareBackend::WebDav, ShareBackend::S3] {
        let started = Instant::now();
        let namespace = Uuid::new_v4();
        let (_fixture, session) = real_share(backend).await;
        let seed_library = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        let seed_hub = worker_hub_on_share(namespace, session.clone(), seed_library).await?;
        let seed_replica = seed_hub.library.peer.identity().await?.replica;
        if let Err(error) = wait_for_replica_head(&*session.store, seed_replica).await {
            anyhow::bail!(
                "{backend:?} waiting for initial seed writer hub#0 replica {seed_replica}: {error:#}"
            );
        }
        let seed_key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        seed_hub
            .library
            .sync_store
            .write_local(seed_key.clone(), support::playlist("three hub shared seed"))
            .await?;
        let mut hubs = vec![seed_hub];
        for _ in 1..3 {
            let library = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
            hubs.push(worker_hub_on_share(namespace, session.clone(), library).await?);
        }
        let seed_wait_started = Instant::now();
        if let Err(error) = wait_for_key_on_hubs(&hubs, &seed_key).await {
            anyhow::bail!(
                "{backend:?} waiting for seed catch-up: {error:#}; {}",
                key_presence_on_hubs(&hubs, &seed_key).await?
            );
        }
        let seed_wait = seed_wait_started.elapsed();
        let mut initial_head_waits = Vec::with_capacity(hubs.len());
        for (index, hub) in hubs.iter().enumerate().skip(1) {
            let replica = hub.library.peer.identity().await?.replica;
            let wait_started = Instant::now();
            if let Err(error) = wait_for_replica_head(&*session.store, replica).await {
                anyhow::bail!(
                    "{backend:?} waiting for initial head from hub#{index} replica {replica}: {error:#}"
                );
            }
            initial_head_waits.push((index, replica, wait_started.elapsed()));
        }

        let first_key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        let second_key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        let third_key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        let (first_write, second_write, third_write) = tokio::join!(
            hubs[0]
                .library
                .sync_store
                .write_local(first_key.clone(), support::playlist("writer one")),
            hubs[1]
                .library
                .sync_store
                .write_local(second_key.clone(), support::playlist("writer two")),
            hubs[2]
                .library
                .sync_store
                .write_local(third_key.clone(), support::playlist("writer three")),
        );
        first_write?;
        second_write?;
        third_write?;
        let keys = [first_key, second_key, third_key];
        if backend == ShareBackend::WebDav {
            assert!(!session.capabilities.conditional_head_write);
            let keys = session
                .store
                .list_prefix("devices/")
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            let mut head_count = 0;
            for key in keys.iter().filter(|key| key.contains("/head.")) {
                let value = session
                    .store
                    .get(key)
                    .await
                    .map_err(|error| anyhow::anyhow!("{error:?}"))?;
                decode_head(&value.bytes)?;
                head_count += 1;
            }
            assert!(
                head_count > 0,
                "competing WebDAV writes produced no head slots"
            );
        } else {
            assert!(session.capabilities.conditional_head_write);
        }
        let key_wait_started = Instant::now();
        if let Err(error) = wait_for_keys_on_hubs(&hubs, &keys).await {
            anyhow::bail!(
                "{backend:?} waiting for all three writer keys to converge: {error:#}; {}",
                key_presence_matrix(&hubs, &keys).await?
            );
        }
        let key_wait = key_wait_started.elapsed();
        eprintln!(
            "SH15 concurrent {backend:?}: seed_wait={seed_wait:?}, initial_head_waits={initial_head_waits:?}, key_wait={key_wait:?}, total={:?}",
            started.elapsed()
        );
        for hub in hubs {
            hub.stop().await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn sh15_read_only_real_share_reports_typed_status_and_writes_nothing() -> Result<()> {
    for backend in [ShareBackend::WebDav, ShareBackend::S3] {
        let namespace = Uuid::new_v4();
        let (fixture, session) = real_share(backend).await;
        fixture.reject_writes();
        let hubs = hubs_on_share(namespace, 3).await;
        assert_eq!(
            sync_pass(&hubs[0].sync_store, namespace, &session).await,
            Err(ShareSyncFailure::ReadOnlyShare)
        );
        let objects = session.store.list_prefix("devices/").await.unwrap();
        assert!(objects.iter().all(|key| key.ends_with('/')));
        assert!(matches!(
            session.store.get(&library_key()).await,
            Err(StoreError::NotFound)
        ));
    }
    Ok(())
}

#[tokio::test]
async fn sh15_crash_after_segment_put_recovers_without_a_gap_or_duplicate() -> Result<()> {
    for backend in [ShareBackend::WebDav, ShareBackend::S3] {
        let backend_started = Instant::now();
        let namespace = Uuid::new_v4();
        let (_fixture, session) = real_share(backend).await;
        let fixture_started = backend_started.elapsed();
        let crash_store = Arc::new(CrashBetweenSegmentAndHead {
            inner: session.store.clone(),
            armed: AtomicBool::new(false),
            segment_written: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            reached: Notify::new(),
            resume: Notify::new(),
        });
        let crash_session = ShareStoreSession {
            store: crash_store.clone(),
            capabilities: session.capabilities,
        };
        let seed = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        let seed_key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        seed.sync_store
            .write_local(seed_key.clone(), support::playlist(&large_playlist_name()))
            .await?;
        let push_started = Instant::now();
        detailed_sync_pass(&seed.sync_store, namespace, &session).await?;
        let seed_push = push_started.elapsed();
        let hub_start_started = Instant::now();
        let mut hubs = worker_hubs_on_share_sessions(namespace, vec![crash_session]).await?;
        let hub_started = hub_start_started.elapsed();
        let push_started = Instant::now();
        wait_for_key_on_hubs(&hubs, &seed_key).await?;
        for hub in &hubs {
            wait_for_replica_head(&*session.store, hub.library.peer.identity().await?.replica)
                .await?;
        }
        let writer_seed_push = push_started.elapsed();
        let receiver = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        let key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        crash_store.arm();
        let crash_started = Instant::now();
        hubs[0]
            .library
            .sync_store
            .write_local(key.clone(), support::playlist("committed after restart"))
            .await?;
        let cache_path = PathBuf::from(hubs[0].node.config_path.as_str())
            .join("share-cache")
            .join(namespace.to_string());
        let pending = hubs[0]
            .library
            .sync_store
            .share_pending_changes(cache_path)
            .await?;
        anyhow::ensure!(
            pending.is_some(),
            "local edit was not visible to share worker"
        );
        let local_push = crash_started.elapsed();
        let crash_wait_started = Instant::now();
        if tokio_time::timeout(TEST_DEADLINE, crash_store.wait_for_head_failure())
            .await
            .is_err()
        {
            let objects = session
                .store
                .list_prefix("devices/")
                .await
                .map_err(|error| anyhow::anyhow!("list after missing crash: {error:?}"))?;
            let messages = hubs[0]
                .broadcaster
                .messages
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            anyhow::bail!(
                "worker did not reach the head after segment write; objects: {objects:?}; messages: {messages:?}"
            );
        }
        let crash = crash_wait_started.elapsed();
        assert!(
            session
                .store
                .list_prefix("devices/")
                .await
                .unwrap()
                .iter()
                .any(|object| object.contains("/seg/"))
        );
        hubs[0].stop_worker().await?;
        crash_store.resume_after_crash();
        let recovery_started = Instant::now();
        hubs[0].restart_node().await?;
        let recovery_restart = recovery_started.elapsed();
        let recovery_push_started = Instant::now();
        detailed_sync_pass(&receiver.sync_store, namespace, &session).await?;
        anyhow::ensure!(
            receiver.sync_store.get(&seed_key).await?.is_some(),
            "recovery left a gap before the changed playlist"
        );
        anyhow::ensure!(
            receiver.sync_store.get(&key).await?.is_some(),
            "recovery did not reach the independent replica"
        );
        let recovery_push = recovery_push_started.elapsed();
        let assertion_started = Instant::now();
        let objects = session
            .store
            .list_prefix("devices/")
            .await
            .map_err(|error| anyhow::anyhow!("list recovered writer objects: {error:?}"))?;
        let segments = objects
            .iter()
            .filter(|object| object.contains("/seg/"))
            .count();
        assert_eq!(segments, 1, "recovered local event was duplicated");
        let assertions = assertion_started.elapsed();
        eprintln!(
            "SH15 {backend:?}: fixture={fixture_started:?}, seed_push={seed_push:?}, hub_started={hub_started:?}, writer_seed_push={writer_seed_push:?}, local_push={local_push:?}, crash={crash:?}, recovery_restart={recovery_restart:?}, recovery_push={recovery_push:?}, assertions={assertions:?}, total={:?}",
            backend_started.elapsed()
        );
        for hub in hubs {
            hub.stop().await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn sh15_torn_older_head_slot_is_survived_on_webdav_and_s3() -> Result<()> {
    for backend in [ShareBackend::WebDav, ShareBackend::S3] {
        let namespace = Uuid::new_v4();
        let (_fixture, session) = real_share(backend).await;
        let hubs = hubs_on_share(namespace, 3).await;
        detailed_sync_pass(&hubs[0].sync_store, namespace, &session).await?;
        let key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        hubs[0]
            .sync_store
            .write_local(
                key.clone(),
                support::playlist("head survives a torn peer slot"),
            )
            .await?;
        sync_pass(&hubs[0].sync_store, namespace, &session)
            .await
            .map_err(|failure| anyhow::anyhow!("{failure:?}"))?;
        let head_zero = session
            .store
            .list_prefix("devices/")
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?
            .into_iter()
            .find(|object| object.ends_with("/head.0.json"))
            .context("writer did not publish its initial head slot")?;
        session
            .store
            .put(&head_zero, b"torn head")
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;

        sync_pass(&hubs[1].sync_store, namespace, &session)
            .await
            .map_err(|failure| anyhow::anyhow!("{failure:?}"))?;
        assert!(hubs[1].sync_store.get(&key).await?.is_some());
    }
    Ok(())
}

#[tokio::test]
async fn sh15_new_device_catches_up_from_a_shared_base_on_webdav_and_s3() -> Result<()> {
    for backend in [ShareBackend::WebDav, ShareBackend::S3] {
        let namespace = Uuid::new_v4();
        let (_fixture, session) = real_share(backend).await;
        let source = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        let key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        source
            .sync_store
            .write_local(key.clone(), support::playlist("new device base catch-up"))
            .await?;
        detailed_sync_pass(&source.sync_store, namespace, &session).await?;
        let source_identity = source.peer.identity().await?;
        let (source_head_key, source_head) =
            latest_head_for_replica(&*session.store, source_identity.replica).await?;
        let source_prefix = source_head_key
            .strip_prefix("devices/")
            .and_then(|key| key.rsplit_once("/head.").map(|(prefix, _)| prefix))
            .context("invalid source head key")?
            .parse::<DevicePrefix>()?;

        let base_owner = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        detailed_sync_pass(&base_owner.sync_store, namespace, &session).await?;
        assert!(base_owner.sync_store.get(&key).await?.is_some());
        let base_identity = base_owner.peer.identity().await?;
        let (_, base_head) =
            latest_head_for_replica(&*session.store, base_identity.replica).await?;
        let mut base_state = load_or_create_state(
            &base_owner.db,
            base_identity.replica,
            namespace,
            base_owner.root.path().join("share-cache"),
            Utc::now().timestamp_millis(),
        )
        .await?;
        let base_endpoint = base_owner.sync_store.endpoint();
        let writer = ShareWriter::default();
        let base_head = cut_base(BaseCutRequest {
            db: &base_owner.db,
            state: &mut base_state,
            writer: &writer,
            store: &*session.store,
            identity: &base_identity,
            source: &base_endpoint,
            current_head: Some(&base_head),
            covers: BTreeMap::from([(
                source_prefix.to_string(),
                Checkpoint {
                    seq: source_head.head.seq,
                    anchor: source_head.head.anchor,
                },
            )]),
            capabilities: session.capabilities,
        })
        .await
        .map_err(|error| anyhow::anyhow!("publish shared catch-up base: {error:?}"))?;
        anyhow::ensure!(base_head.bases.len() > 1);

        let source_base_key =
            base_key(&source_prefix.to_string(), &source_head.floor_base.base_id)?;
        session
            .store
            .delete(&source_base_key)
            .await
            .map_err(|error| anyhow::anyhow!("delete source floor base: {error:?}"))?;
        let new_device = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        detailed_sync_pass(&new_device.sync_store, namespace, &session).await?;
        assert!(new_device.sync_store.get(&key).await?.is_some());
    }
    Ok(())
}

async fn latest_head_for_replica(store: &dyn ObjectStore, replica: Uuid) -> Result<(String, Head)> {
    let mut latest = None;
    for key in store
        .list_prefix("devices/")
        .await
        .map_err(|error| anyhow::anyhow!("list device heads: {error:?}"))?
        .into_iter()
        .filter(|key| key.contains("/head."))
    {
        let object = store
            .get(&key)
            .await
            .map_err(|error| anyhow::anyhow!("get device head: {error:?}"))?;
        let head = decode_head(&object.bytes)?;
        if head.local_replica == replica
            && latest
                .as_ref()
                .is_none_or(|(_, current): &(String, Head)| head.counter > current.counter)
        {
            latest = Some((key, head));
        }
    }
    latest.context("device did not publish a head")
}

#[tokio::test]
async fn sh15_conditional_head_put_retries_a_s3_409() -> Result<()> {
    let namespace = Uuid::new_v4();
    let (fixture, session) = real_share(ShareBackend::S3).await;
    let library = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    detailed_sync_pass(&library.sync_store, namespace, &session).await?;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    };
    // The first edit creates the alternate slot. Only a subsequent publish
    // overwrites an existing slot with If-Match, which is what the fixture faults.
    library
        .sync_store
        .write_local(key.clone(), support::playlist("prime the alternate slot"))
        .await?;
    detailed_sync_pass(&library.sync_store, namespace, &session).await?;
    let edited = support::playlist("retry conditional head conflict");
    library
        .sync_store
        .write_local(key.clone(), edited.clone())
        .await?;
    fixture.conflict_next_conditional_put();
    detailed_sync_pass(&library.sync_store, namespace, &session).await?;
    anyhow::ensure!(
        !fixture.conditional_put_conflict_pending(),
        "share writer never encountered the injected conditional 409"
    );
    let reader = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    detailed_sync_pass(&reader.sync_store, namespace, &session).await?;
    assert_eq!(
        reader
            .sync_store
            .get(&key)
            .await?
            .context("retried edit missing")?
            .value,
        edited
    );
    Ok(())
}

#[tokio::test]
async fn sh15_reader_recovers_from_another_live_base_after_floor_base_delete() -> Result<()> {
    for backend in [ShareBackend::WebDav, ShareBackend::S3] {
        let namespace = Uuid::new_v4();
        let (_fixture, session) = real_share(backend).await;
        let first = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        let key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        first
            .sync_store
            .write_local(
                key.clone(),
                support::playlist("preserved by another owner's base"),
            )
            .await?;
        detailed_sync_pass(&first.sync_store, namespace, &session).await?;

        let second = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        detailed_sync_pass(&second.sync_store, namespace, &session).await?;
        assert!(second.sync_store.get(&key).await?.is_some());

        let first_replica = first.peer.identity().await?.replica;
        let listed = session
            .store
            .list_prefix("devices/")
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        let mut first_head = None;
        for key in listed.iter().filter(|key| key.contains("/head.")) {
            let value = session
                .store
                .get(key)
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            let head = decode_head(&value.bytes)?;
            if head.local_replica == first_replica
                && first_head
                    .as_ref()
                    .is_none_or(|(_, current): &(String, Head)| head.counter > current.counter)
            {
                first_head = Some((key.clone(), head));
            }
        }
        let (_, first_head) = first_head.context("first owner did not publish a head")?;
        let mut covers = BTreeMap::new();
        for key in listed.iter().filter(|key| key.contains("/head.")) {
            let value = session
                .store
                .get(key)
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            let head = decode_head(&value.bytes)?;
            if head.local_replica == first_replica {
                continue;
            }
            let prefix = key
                .strip_prefix("devices/")
                .and_then(|key| key.rsplit_once("/head.").map(|(prefix, _)| prefix))
                .context("invalid peer head key")?;
            let prefix = prefix.parse::<DevicePrefix>()?.to_string();
            let checkpoint = Checkpoint {
                seq: head.head.seq,
                anchor: head.head.anchor,
            };
            if covers
                .get(&prefix)
                .is_none_or(|current: &Checkpoint| checkpoint.seq > current.seq)
            {
                covers.insert(prefix, checkpoint);
            }
        }
        let first_identity = first.peer.identity().await?;
        let mut state = load_or_create_state(
            &first.db,
            first_identity.replica,
            namespace,
            first.root.path().join("share-cache"),
            Utc::now().timestamp_millis(),
        )
        .await?;
        let endpoint = first.sync_store.endpoint();
        let writer = ShareWriter::default();
        let next_head = cut_base(BaseCutRequest {
            db: &first.db,
            state: &mut state,
            writer: &writer,
            store: &*session.store,
            identity: &first_identity,
            source: &endpoint,
            current_head: Some(&first_head),
            covers,
            capabilities: session.capabilities,
        })
        .await
        .map_err(|error| anyhow::anyhow!("publish newer floor base: {error:?}"))?;
        assert!(next_head.bases.len() > first_head.bases.len());
        let delete_store = Arc::new(DeleteBaseBeforeRead {
            inner: session.store.clone(),
            deleted: AtomicBool::new(false),
        });
        let racing_session = ShareStoreSession {
            store: delete_store.clone(),
            capabilities: session.capabilities,
        };

        let reader = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        detailed_sync_pass(&reader.sync_store, namespace, &racing_session).await?;
        assert!(delete_store.deleted.load(Ordering::SeqCst));
        assert!(reader.sync_store.get(&key).await?.is_some());
    }
    Ok(())
}

#[tokio::test]
async fn sh15_clone_restore_and_double_restore_use_new_generations() -> Result<()> {
    for backend in [ShareBackend::WebDav, ShareBackend::S3] {
        let namespace = Uuid::new_v4();
        let (_fixture, session) = real_share(backend).await;
        let library = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        detailed_sync_pass(&library.sync_store, namespace, &session).await?;
        let identity = library.peer.identity().await?;
        let cache_path = library.root.path().join("share-cache");
        let state = load_or_create_state(
            &library.db,
            identity.replica,
            namespace,
            cache_path,
            Utc::now().timestamp_millis(),
        )
        .await?;
        let backup = state.clone();
        let previous_generation = backup.generation;
        let backup_writer_session = backup.writer_session;
        let key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        library
            .sync_store
            .write_local(
                key,
                support::playlist("original writer advances after clone"),
            )
            .await?;
        detailed_sync_pass(&library.sync_store, namespace, &session).await?;
        let mut active_state = load_or_create_state(
            &library.db,
            identity.replica,
            namespace,
            library.root.path().join("share-cache"),
            Utc::now().timestamp_millis(),
        )
        .await?;
        active_state.generation = Uuid::new_v4();
        active_state.writer_session = Uuid::new_v4();
        active_state.head_counter = 0;
        active_state.local_cursor = 0;
        active_state.observed_objects.clear();
        save_state(&library.db, &active_state).await?;
        detailed_sync_pass(&library.sync_store, namespace, &session).await?;

        let restored = support::SqlLibrary::new(namespace, identity.replica).await?;
        save_state(&restored.db, &backup).await?;

        detailed_sync_pass(&restored.sync_store, namespace, &session).await?;
        let rotated = load_or_create_state(
            &restored.db,
            identity.replica,
            namespace,
            restored.root.path().join("share-cache"),
            Utc::now().timestamp_millis(),
        )
        .await?;
        assert_ne!(rotated.generation, previous_generation);
        assert_ne!(rotated.writer_session, backup_writer_session);

        // Restoring the same backup a second time must also be recognized as a
        // restore, even though the share already contains a newer generation.
        save_state(&restored.db, &backup).await?;
        detailed_sync_pass(&restored.sync_store, namespace, &session).await?;
        let restored_again = load_or_create_state(
            &restored.db,
            identity.replica,
            namespace,
            restored.root.path().join("share-cache"),
            Utc::now().timestamp_millis(),
        )
        .await?;
        assert_ne!(restored_again.generation, previous_generation);
        assert_ne!(restored_again.generation, rotated.generation);
        assert_ne!(restored_again.writer_session, backup_writer_session);
    }
    Ok(())
}

#[tokio::test]
async fn sh15_two_writers_race_one_head_slot_on_each_backend() -> Result<()> {
    for backend in [ShareBackend::WebDav, ShareBackend::S3] {
        let namespace = Uuid::new_v4();
        let (_fixture, session) = real_share(backend).await;
        match backend {
            ShareBackend::WebDav => anyhow::ensure!(
                !session.capabilities.conditional_create
                    && !session.capabilities.conditional_head_write,
                "OpenDAL WebDAV must report its missing conditional write capabilities"
            ),
            ShareBackend::S3 => anyhow::ensure!(
                session.capabilities.conditional_create
                    && session.capabilities.conditional_head_write,
                "S3 head race requires conditional create and overwrite"
            ),
        }
        let actor = Uuid::new_v4();
        let first = support::SqlLibrary::new(namespace, actor).await?;
        detailed_sync_pass(&first.sync_store, namespace, &session).await?;
        let second =
            support::fixtures::open_library(first.db.clone(), actor, Some(namespace)).await?;
        let prime_key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        first
            .sync_store
            .write_local(prime_key, support::playlist("prime both head slots"))
            .await?;
        detailed_sync_pass(&first.sync_store, namespace, &session).await?;
        let existing_heads = session
            .store
            .list_prefix("devices/")
            .await
            .map_err(|error| anyhow::anyhow!("list primed heads: {error:?}"))?
            .into_iter()
            .filter(|key| key.contains("/head."))
            .collect::<Vec<_>>();
        anyhow::ensure!(
            existing_heads
                .iter()
                .any(|key| key.ends_with("/head.0.json"))
                && existing_heads
                    .iter()
                    .any(|key| key.ends_with("/head.1.json")),
            "the raced conditional write needs both head slots to exist"
        );
        let race_store = Arc::new(RaceHeadPutStore {
            inner: session.store.clone(),
            barrier: Barrier::new(2),
            waiting: AtomicUsize::new(0),
        });
        let race_session = ShareStoreSession {
            store: race_store.clone(),
            capabilities: session.capabilities,
        };
        let key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        first
            .sync_store
            .write_local(
                key.clone(),
                support::playlist("both writers race one prefix"),
            )
            .await?;
        let (first_result, second_result) = tokio::join!(
            sync_pass(&first.sync_store, namespace, &race_session),
            sync_pass(&second, namespace, &race_session),
        );
        anyhow::ensure!(first_result.is_ok() || second_result.is_ok());
        assert!(
            race_store.waiting.load(Ordering::SeqCst) >= 2,
            "writers did not meet in the conditional head overwrite window"
        );
        detailed_sync_pass(&first.sync_store, namespace, &race_session).await?;
        let head_keys = session
            .store
            .list_prefix("devices/")
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?
            .into_iter()
            .filter(|key| key.contains("/head."))
            .collect::<Vec<_>>();
        anyhow::ensure!(head_keys.iter().any(|key| key.ends_with("/head.0.json")));
        let reader = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        detailed_sync_pass(&reader.sync_store, namespace, &session).await?;
        assert!(reader.sync_store.get(&key).await?.is_some());
    }
    Ok(())
}

#[tokio::test]
async fn sh15_new_base_reuses_every_unchanged_chunk() -> Result<()> {
    for backend in [ShareBackend::WebDav, ShareBackend::S3] {
        let namespace = Uuid::new_v4();
        let (_fixture, session) = real_share(backend).await;
        let library = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        let key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        library
            .sync_store
            .write_local(key, support::playlist("reused base chunk"))
            .await?;
        detailed_sync_pass(&library.sync_store, namespace, &session).await?;
        let listed = session
            .store
            .list_prefix("devices/")
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        let head_key = listed
            .iter()
            .find(|object| object.ends_with("/head.0.json"))
            .context("base owner did not publish its first head")?;
        let value = session
            .store
            .get(head_key)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        let head = decode_head(&value.bytes)?;
        let prefix = head_key
            .strip_prefix("devices/")
            .and_then(|key| key.strip_suffix("/head.0.json"))
            .context("invalid head key")?;
        let chunk_prefix = format!("devices/{prefix}/obj/");
        let chunks_before = session
            .store
            .list_prefix(&chunk_prefix)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        anyhow::ensure!(!chunks_before.is_empty(), "fixture base had no chunks");
        let counting_store = Arc::new(CountNewChunkObjects {
            inner: session.store.clone(),
            attempted_chunk_writes: AtomicUsize::new(0),
        });

        let identity = library.peer.identity().await?;
        let mut state = load_or_create_state(
            &library.db,
            identity.replica,
            namespace,
            library.root.path().join("share-cache"),
            Utc::now().timestamp_millis(),
        )
        .await?;
        let endpoint = library.sync_store.endpoint();
        let writer = ShareWriter::default();
        let next = cut_base(BaseCutRequest {
            db: &library.db,
            state: &mut state,
            writer: &writer,
            store: &*counting_store,
            identity: &identity,
            source: &endpoint,
            current_head: Some(&head),
            covers: BTreeMap::new(),
            capabilities: session.capabilities,
        })
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        anyhow::ensure!(next.bases.len() > head.bases.len());
        assert_eq!(
            counting_store.attempted_chunk_writes.load(Ordering::SeqCst),
            0,
            "new base rewrote an existing chunk"
        );
        let chunks_after = session
            .store
            .list_prefix(&chunk_prefix)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(chunks_after, chunks_before);
    }
    Ok(())
}

#[tokio::test]
async fn sh15_initial_base_cut_races_own_prefix_gc_safely() -> Result<()> {
    for backend in [ShareBackend::WebDav, ShareBackend::S3] {
        let namespace = Uuid::new_v4();
        let (_fixture, session) = real_share(backend).await;
        let library = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        let key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        library
            .sync_store
            .write_local(
                key.clone(),
                support::playlist("base cut and GC share one writer"),
            )
            .await?;
        let (first, second) = tokio::join!(
            sync_pass(&library.sync_store, namespace, &session),
            sync_pass(&library.sync_store, namespace, &session),
        );
        anyhow::ensure!(first.is_ok() || second.is_ok());
        let library_file: LibraryFile = serde_json::from_slice(
            &session
                .store
                .get(&library_key())
                .await
                .map_err(|error| {
                    anyhow::anyhow!("read concurrently created library.json: {error:?}")
                })?
                .bytes,
        )?;
        anyhow::ensure!(
            library_file.library_id == namespace,
            "non-conditional library.json creation left an invalid binding"
        );
        detailed_sync_pass(&library.sync_store, namespace, &session).await?;
        assert!(library.sync_store.get(&key).await?.is_some());
        let heads = session
            .store
            .list_prefix("devices/")
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?
            .into_iter()
            .filter(|key| key.contains("/head."))
            .collect::<Vec<_>>();
        assert!(!heads.is_empty());
    }
    Ok(())
}

#[tokio::test]
async fn sh15_retired_device_return_uses_a_fresh_generation() -> Result<()> {
    for backend in [ShareBackend::WebDav, ShareBackend::S3] {
        let namespace = Uuid::new_v4();
        let (_fixture, session) = real_share(backend).await;
        let first = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        let first_identity = first.peer.identity().await?;
        let shared_key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        first
            .sync_store
            .write_local(shared_key, support::playlist(&large_playlist_name()))
            .await?;
        detailed_sync_pass(&first.sync_store, namespace, &session).await?;
        let returning = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        detailed_sync_pass(&returning.sync_store, namespace, &session).await?;
        let returning_identity = returning.peer.identity().await?;
        let offline_key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::new_v4(),
        };
        first
            .sync_store
            .write_local(
                offline_key.clone(),
                support::playlist("published while the device was offline"),
            )
            .await?;
        detailed_sync_pass(&first.sync_store, namespace, &session).await?;
        let listed = session
            .store
            .list_prefix("devices/")
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        let mut first_head = None;
        let mut returning_head = None;
        for key in listed.iter().filter(|key| key.contains("/head.")) {
            let value = session
                .store
                .get(key)
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            let head = decode_head(&value.bytes)?;
            if head.local_replica == first_identity.replica
                && first_head
                    .as_ref()
                    .is_none_or(|(_, current): &(String, Head)| head.counter > current.counter)
            {
                first_head = Some((key.clone(), head.clone()));
            }
            if head.local_replica == returning_identity.replica
                && returning_head
                    .as_ref()
                    .is_none_or(|(_, current): &(String, Head)| head.counter > current.counter)
            {
                returning_head = Some((key.clone(), head));
            }
        }
        let (first_head_key, first_head) = first_head.context("first device head missing")?;
        let (returning_head_key, returning_head) =
            returning_head.context("returning device head missing")?;
        let first_prefix = first_head_key
            .strip_prefix("devices/")
            .and_then(|key| key.rsplit_once("/head."))
            .map(|(prefix, _)| prefix)
            .context("invalid first device head key")?
            .to_owned();
        let returning_prefix = returning_head_key
            .strip_prefix("devices/")
            .and_then(|key| key.rsplit_once("/head."))
            .map(|(prefix, _)| prefix)
            .context("invalid returning device head key")?
            .to_owned();
        let returning_prefix = returning_prefix.parse::<DevicePrefix>()?;
        let now_ms = Utc::now().timestamp_millis();
        let mut first_state = load_or_create_state(
            &first.db,
            first_identity.replica,
            namespace,
            first.root.path().join("share-cache"),
            now_ms,
        )
        .await?;
        let endpoint = first.sync_store.endpoint();
        let writer = ShareWriter::default();
        let first_head = cut_base(BaseCutRequest {
            db: &first.db,
            state: &mut first_state,
            writer: &writer,
            store: &*session.store,
            identity: &first_identity,
            source: &endpoint,
            current_head: Some(&first_head),
            covers: BTreeMap::from([(
                returning_prefix.to_string(),
                Checkpoint {
                    seq: returning_head.head.seq,
                    anchor: returning_head.head.anchor,
                },
            )]),
            capabilities: session.capabilities,
        })
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        anyhow::ensure!(first_head.bases.len() > 1);
        for (key, observed) in &mut first_state.observed_objects {
            if matches!(key, ShareObjectKey::Head { prefix, .. } if prefix == &returning_prefix) {
                observed.last_change_ms = now_ms - RETIREMENT_MS as i64 - 1;
            }
        }
        save_state(&first.db, &first_state).await?;
        detailed_sync_pass(&first.sync_store, namespace, &session).await?;
        detailed_sync_pass(&first.sync_store, namespace, &session).await?;
        let first_heads = session
            .store
            .list_prefix(&format!("devices/{first_prefix}/"))
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        let mut retired = false;
        for key in first_heads.iter().filter(|key| key.contains("/head.")) {
            let value = session
                .store
                .get(key)
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            retired |= decode_head(&value.bytes)?
                .retired
                .contains(&returning_prefix.to_string());
        }
        assert!(retired, "keeper did not retire the offline prefix");
        let returning_objects = session
            .store
            .list_prefix(&format!("devices/{}/", returning_prefix))
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert!(returning_objects.is_empty());

        let old_generation = returning_head.generation;
        detailed_sync_pass(&returning.sync_store, namespace, &session).await?;
        let state = load_or_create_state(
            &returning.db,
            returning_identity.replica,
            namespace,
            returning.root.path().join("share-cache"),
            Utc::now().timestamp_millis(),
        )
        .await?;
        assert_ne!(state.generation, old_generation);
        assert!(returning.sync_store.get(&offline_key).await?.is_some());
    }
    Ok(())
}

#[tokio::test]
async fn replaced_library_file_is_a_typed_share_change() -> Result<()> {
    let namespace = Uuid::new_v4();
    let library = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    let session = memory_session(&store).await;
    sync_pass(&library.sync_store, namespace, &session)
        .await
        .unwrap();
    let replacement = LibraryFile {
        format: FORMAT_VERSION,
        library_id: Uuid::new_v4(),
        model: RuneModel::model_id().to_owned(),
        schema: RuneModel::schema_id().to_owned(),
    };
    store
        .put(&library_key(), &encode_library(&replacement).unwrap())
        .await
        .unwrap();
    assert_eq!(
        sync_pass(&library.sync_store, namespace, &session).await,
        Err(ShareSyncFailure::ShareChanged)
    );
    assert_eq!(
        store.get(&library_key()).await.unwrap().bytes,
        encode_library(&replacement).unwrap()
    );
    Ok(())
}

/// A node on a device that has not added any library yet.
async fn node_without_libraries(directory: &tempfile::TempDir) -> Result<Arc<NodeContext>> {
    write_quick_identity(directory.path())?;
    crate::test_node::disable_media_server(directory.path())?;
    let path = directory.path().to_string_lossy().into_owned();
    NodeContext::new(
        &path,
        &path,
        Arc::new(QuietBroadcaster::default()),
        false,
        RunMode::App,
        Arc::new(Visualizer::new()),
    )
    .await
}

/// Registers the share on `node` the way the Add Library guide does after detecting what the
/// share holds, and returns the stored record.
async fn register_share_library(
    node: &Arc<NodeContext>,
    service: &OpenDalLibraryService,
    share_data_choice: ShareDataChoice,
) -> Result<LibraryRecord> {
    let response = register_opendal_library_on_node(
        &RegisterOpenDalLibraryRequest {
            service: service.clone(),
            alias: "Shared music".into(),
            share_data_choice,
            call_id: None,
        },
        node,
    )
    .await?;
    let record_id = response
        .record
        .context("registered share record")?
        .record_id;
    LibraryRegistry::load(Path::new(node.config_path.as_str()))?
        .libraries
        .into_iter()
        .find(|record| record.record_id.0.to_string() == record_id)
        .context("stored share record")
}

/// A device that adds a WebDAV folder where another device keeps a library, through the steps
/// of the Add Library guide, joins that library: it detects the library the share names,
/// registers the same identity, and its share pass accepts the share and receives the library.
/// An empty folder still gets a new library, and a library with another identity is still
/// refused without replacing the share's identity.
#[tokio::test]
async fn a_new_device_joins_a_published_share_with_its_identity() -> Result<()> {
    let server = WebDavFixture::start(vec![("track.ogg".into(), Vec::new())]).await?;
    let service = OpenDalLibraryService::WebDav {
        endpoint: server.endpoint.clone(),
        root: "/dav/library/".into(),
        username: "user".into(),
        password: "pass".into(),
    };
    let session = opendal_share_session(OpenDalConfig::WebDav {
        endpoint: server.endpoint.clone(),
        root: "/dav/library/".into(),
        username: "user".into(),
        password: "pass".into(),
    })
    .await;

    // The first device finds the folder empty and keeps the new library's data in the share.
    let first_directory = tempfile::tempdir()?;
    let first_node = node_without_libraries(&first_directory).await?;
    assert_eq!(
        detect_opendal_share_data(&service).await,
        Ok(OpenDalShareDataDetection::WritableEmpty)
    );
    let created =
        register_share_library(&first_node, &service, ShareDataChoice::KeepInShare).await?;
    assert!(matches!(created.catalog, CatalogPlacement::InShare(_)));
    let library_id = created.library_id.context("new share library id")?;
    let first = support::SqlLibrary::new(library_id, Uuid::new_v4()).await?;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    };
    first
        .sync_store
        .write_local(key.clone(), support::playlist("kept in the share"))
        .await?;
    sync_pass(&first.sync_store, library_id, &session)
        .await
        .unwrap();

    // A second device with an empty registry adds the same folder.
    let second_directory = tempfile::tempdir()?;
    let second_node = node_without_libraries(&second_directory).await?;
    let published = Ok(OpenDalShareDataDetection::ExistingLibrary {
        library_id: library_id.to_string(),
    });
    assert_eq!(detect_opendal_share_data(&service).await, published);
    // For a share that holds a library the guide asks nothing and sends this device only;
    // registration joins the library in the share regardless.
    let joined =
        register_share_library(&second_node, &service, ShareDataChoice::ThisDeviceOnly).await?;
    assert_eq!(joined.library_id, Some(library_id));
    assert!(matches!(joined.catalog, CatalogPlacement::InShare(_)));
    let second = support::SqlLibrary::new(library_id, Uuid::new_v4()).await?;
    sync_pass(&second.sync_store, library_id, &session)
        .await
        .unwrap();
    assert!(second.sync_store.get(&key).await?.is_some());

    // A library with another identity cannot take the share over.
    let other_id = Uuid::new_v4();
    let other = support::SqlLibrary::new(other_id, Uuid::new_v4()).await?;
    assert_eq!(
        sync_pass(&other.sync_store, other_id, &session).await,
        Err(ShareSyncFailure::ShareChanged)
    );
    assert_eq!(detect_opendal_share_data(&service).await, published);

    first_node.shutdown().await?;
    second_node.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn cancellation_drops_an_in_flight_share_pass() -> Result<()> {
    let namespace = Uuid::new_v4();
    let library = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    store.configure(MemoryStoreConfig {
        delay: Duration::from_millis(10),
        partial_write_bytes: None,
    });
    let session = memory_session(&store).await;
    let cache = env::temp_dir().join(format!("hub-share-{}", Uuid::new_v4()));
    let cancel = CancellationToken::new();
    let library_sync = library.sync_store.clone();
    let session = session.clone();
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        select_share_cancellation(
            task_cancel,
            share_sync_pass(
                &library_sync,
                namespace,
                cache,
                &session,
                SharePass::PullAndPush,
                || {},
            ),
        )
        .await
    });
    wait_until("share pass to begin its put", || async {
        Ok(store.calls().contains(&StoreOperation::Put).then_some(()))
    })
    .await?;
    cancel.cancel();
    assert!(task.await?.is_none());
    Ok(())
}

#[test]
fn repeated_share_failures_back_off_to_the_cap() {
    let mut previous = None;
    let mut observed = Vec::new();
    for _ in 0..8 {
        let (wait, next) = next_share_backoff(previous);
        observed.push(wait);
        previous = Some(next);
    }
    assert_eq!(
        observed,
        [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(16),
            Duration::from_secs(30),
            Duration::from_secs(30),
            Duration::from_secs(30),
        ]
    );
}

mod revocation_tests {
    use super::*;
    use discovery::server::UserStatus;
    use discovery::utils::DeviceType;
    use tokio::task;
    use tokio::time as tokio_time;
    use tokio::time::Instant;
    use visualizer::Visualizer;

    /// A node with `fingerprint` approved and `host` its trusted host.
    async fn node_with_trusted_peer(
        directory: &tempfile::TempDir,
        fingerprint: &str,
        host: &str,
    ) -> Result<Arc<NodeContext>> {
        let path = directory.path().to_string_lossy().into_owned();
        crate::test_node::disable_media_server(directory.path())?;
        let node = NodeContext::new(
            &path,
            &path,
            Arc::new(QuietBroadcaster::default()),
            false,
            RunMode::App,
            Arc::new(Visualizer::new()),
        )
        .await?;
        {
            let permissions = node.permission_manager.read().await;
            permissions
                .add_user(
                    "key".into(),
                    fingerprint.into(),
                    "peer".into(),
                    "test".into(),
                    DeviceType::Desktop,
                    "127.0.0.1".into(),
                )
                .await?;
            permissions
                .change_user_status(fingerprint, UserStatus::Approved)
                .await?;
        }
        node.cert_validator
            .read()
            .await
            .replace_hosts_for_fingerprint(fingerprint, vec![host.into()])
            .await?;
        Ok(node)
    }

    /// Starts the watcher, and checks it keeps waiting while nothing changes.
    async fn watch(node: &Arc<NodeContext>, fingerprint: &str, host: &str) -> task::JoinHandle<()> {
        let weak = Arc::downgrade(node);
        let (fingerprint, host) = (fingerprint.to_owned(), host.to_owned());
        let watcher =
            tokio::spawn(async move { wait_for_revocation(weak, &fingerprint, &host).await });
        for _ in 0..20 {
            task::yield_now().await;
        }
        assert!(!watcher.is_finished(), "an approved peer is not revoked");
        watcher
    }

    /// With the clock stopped from here on, only a change can end the watcher before a second of
    /// the clock has passed: a timer would need it to move.
    async fn ends_within_a_tick(watcher: &task::JoinHandle<()>) -> Result<()> {
        let started = Instant::now();
        wait_until("the watcher to end", || async {
            Ok(watcher.is_finished().then_some(()))
        })
        .await?;
        let waited = started.elapsed();
        anyhow::ensure!(
            waited < Duration::from_millis(500),
            "the watcher ended only after {waited:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn revoking_a_peer_ends_the_watcher_at_once() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let node = node_with_trusted_peer(&directory, "peer-a", "127.0.0.1:1").await?;
        let watcher = watch(&node, "peer-a", "127.0.0.1:1").await;
        tokio_time::pause();

        node.permission_manager
            .read()
            .await
            .change_user_status("peer-a", UserStatus::Blocked)
            .await?;

        ends_within_a_tick(&watcher).await?;
        node.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn removing_the_trusted_host_ends_the_watcher_at_once() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let node = node_with_trusted_peer(&directory, "peer-b", "127.0.0.1:2").await?;
        let watcher = watch(&node, "peer-b", "127.0.0.1:2").await;
        tokio_time::pause();

        node.cert_validator
            .read()
            .await
            .replace_hosts_for_fingerprint("peer-b", vec!["127.0.0.1:3".into()])
            .await?;

        ends_within_a_tick(&watcher).await?;
        node.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn the_watcher_ends_with_its_node() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let node = node_with_trusted_peer(&directory, "peer-c", "127.0.0.1:4").await?;
        let watcher = watch(&node, "peer-c", "127.0.0.1:4").await;
        tokio_time::pause();

        node.shutdown().await?;
        drop(node);

        ends_within_a_tick(&watcher).await?;
        Ok(())
    }
}

impl SyncTokenStore {
    pub(crate) fn insert_stale_for_test(&self, fingerprint: &str, base: &str) {
        self.insert(fingerprint, base, "unknown-to-peer".to_owned());
    }
}

#[derive(Default)]
pub(in crate::server) struct TestPeerSession {
    locks: ReplicaLocks,
}

impl TestPeerSession {
    pub(in crate::server) async fn pass(
        &self,
        params: Arc<GlobalParams>,
        private_key: &str,
        endpoint: PeerEndpoint,
        metadata: bool,
    ) -> Result<PeerPass> {
        let global = params.node.clone();
        let credentials = LocalCredentials {
            fingerprint: global.device_info().await?.fingerprint,
            private_key: private_key.into(),
        };
        let library = global
            .library_pool
            .acquire_background(params.library_id.unwrap())
            .await?;
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        classify_peer_pass(
            sync_one(
                &global,
                &library,
                &notify,
                &endpoint,
                PassOptions {
                    credentials: &credentials,
                    metadata,
                    locks: &self.locks,
                    cancel: &params.node.token,
                    download_progress: &mut DownloadProgress::default(),
                },
            )
            .await,
        )
    }
}

pub(in crate::server) async fn test_peer_pass(
    params: Arc<GlobalParams>,
    private_key: &str,
    endpoint: PeerEndpoint,
    metadata: bool,
) -> Result<PeerPass> {
    TestPeerSession::default()
        .pass(params, private_key, endpoint, metadata)
        .await
}

impl QuietBroadcaster {
    /// The share sync statuses broadcast for one library, in order.
    fn share_statuses_of(&self, library_id: Uuid) -> Vec<Option<ShareSyncFailure>> {
        self.share_statuses_by_library
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|(id, _)| *id == library_id.to_string())
            .map(|(_, failure)| *failure)
            .collect()
    }
}

/// A node with two libraries stored in shared folders, each with its own store: `ui` is open
/// in the UI, and `background` is open in the pool only, the way a library is that some
/// other work holds.
struct ShareSwitchFixture {
    _directory: tempfile::TempDir,
    node: Arc<NodeContext>,
    broadcaster: Arc<QuietBroadcaster>,
    ui: Uuid,
    background: Uuid,
    background_library: support::SqlLibrary,
    background_store: Arc<MemoryObjectStore>,
    /// Another device that keeps `background` in the same shared folder.
    other_device: support::SqlLibrary,
    opened: Arc<Mutex<HashMap<Uuid, usize>>>,
    _handles: Vec<LibraryHandle>,
    _ui_library: support::SqlLibrary,
    cancel: CancellationToken,
    task: task::JoinHandle<()>,
}

impl ShareSwitchFixture {
    async fn start(auto_share_sync: bool) -> Result<Self> {
        let ui = Uuid::new_v4();
        let background = Uuid::new_v4();
        let ui_library = support::SqlLibrary::new_persistent(ui, Uuid::new_v4()).await?;
        let background_library =
            support::SqlLibrary::new_persistent(background, Uuid::new_v4()).await?;
        let other_device = support::SqlLibrary::new(background, Uuid::new_v4()).await?;
        let directory = tempfile::tempdir()?;
        let path = directory.path().to_string_lossy().into_owned();
        write_quick_identity(directory.path())?;
        crate::test_node::disable_media_server(directory.path())?;
        let broadcaster = Arc::new(QuietBroadcaster::default());
        let node = NodeContext::new(
            &path,
            &path,
            broadcaster.clone(),
            false,
            RunMode::App,
            Arc::new(Visualizer::new()),
        )
        .await?;
        stop_own_worker(&node).await?;
        update_sync_settings(&node, |settings| settings.auto_share_sync = auto_share_sync).await?;
        let records = [test_record(ui), test_record(background)];
        fs::write(
            directory.path().join("libraries.json"),
            serde_json::to_vec(&json!({
                "version": 3,
                "libraries": records,
                "last_open_record": null,
            }))?,
        )?;
        let mut handles = Vec::new();
        for (library, record) in [&ui_library, &background_library].into_iter().zip(records) {
            let library_id = record.library_id.context("share test record library id")?;
            handles.push(
                node.library_pool
                    .register_ui(LibraryResources {
                        library_id,
                        display_name: record.alias.clone(),
                        main_db: Arc::new(library.db.clone()),
                        library_sync: library.sync_store.clone(),
                        recommend_db: Arc::new(RecommendationDbConnection::new()),
                        fsio: Arc::new(FsIo::new()),
                        media_root: library.root.path().to_string_lossy().into_owned(),
                        descriptor: record,
                    })
                    .await?,
            );
        }
        // The held handle keeps `background` open in the pool after the UI lets go of it.
        node.library_pool.unregister_ui(background).await;
        anyhow::ensure!(
            node.library_pool.ui_library_ids().await == [ui],
            "only one share test library may be open in the UI"
        );

        let ui_store = Arc::new(MemoryObjectStore::default());
        let background_store = Arc::new(MemoryObjectStore::default());
        let opened = Arc::new(Mutex::new(HashMap::new()));
        let factory: Arc<dyn ShareStoreFactory> = Arc::new(RoutedShareFactory {
            stores: HashMap::from([(ui, ui_store), (background, background_store.clone())]),
            opened: opened.clone(),
        });
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run_share_worker(
            Arc::downgrade(&node),
            Arc::new(|| {}),
            cancel.clone(),
            factory,
            ShareWorkerTimers {
                tick: Duration::from_millis(25),
                foreground_pull: Duration::from_millis(50),
                background_pull: Duration::from_millis(50),
                push_debounce: Duration::from_millis(10),
                statistics_push: Duration::from_millis(50),
                idle_push: Duration::from_millis(100),
            },
        ));
        Ok(Self {
            _directory: directory,
            node,
            broadcaster,
            ui,
            background,
            background_library,
            background_store,
            other_device,
            opened,
            _handles: handles,
            _ui_library: ui_library,
            cancel,
            task,
        })
    }

    /// How many times the worker opened a store session for `library_id`.
    fn sessions_opened(&self, library_id: Uuid) -> usize {
        self.opened
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&library_id)
            .copied()
            .unwrap_or_default()
    }

    /// One pull and push by the other device that keeps `background` in the same share.
    async fn other_device_pass(&self) -> Result<()> {
        let session = memory_session(&self.background_store).await;
        sync_pass(&self.other_device.sync_store, self.background, &session)
            .await
            .map_err(|failure| anyhow::anyhow!("other device share pass: {failure:?}"))
    }

    /// Waits until the worker has run `count` more scheduled passes over the UI library, so
    /// that it has also had that many chances to schedule `background`.
    async fn scheduled_ui_passes(&self, count: usize) -> Result<()> {
        let before = self.broadcaster.share_statuses_of(self.ui).len();
        wait_until("scheduled passes over the UI share library", || async {
            Ok((self.broadcaster.share_statuses_of(self.ui).len() >= before + count).then_some(()))
        })
        .await
    }

    async fn set_auto_share_sync(&self, enabled: bool) -> Result<()> {
        update_sync_settings(&self.node, |settings| settings.auto_share_sync = enabled).await
    }

    async fn request_sync(&self, library_id: String) -> Result<Option<ShareSyncFailure>> {
        use crate::Signal;
        use crate::messages::SyncShareLibraryRequest;

        let request = SyncShareLibraryRequest {
            library_id,
            call_id: None,
        };
        Ok(request
            .handle(self.node.clone(), None, &request)
            .await?
            .context("share sync response")?
            .share_failure)
    }

    async fn stop(self) -> Result<()> {
        self.cancel.cancel();
        self.task.await?;
        drop(self._handles);
        self.node.shutdown().await
    }
}

#[tokio::test]
async fn a_share_sync_request_ends_when_the_node_stops() -> Result<()> {
    let requests = ShareSyncRequests::default();
    let shutdown = CancellationToken::new();
    let pending = requests.sync(Uuid::new_v4(), &shutdown);
    tokio::pin!(pending);
    // No worker runs here, so nothing ever takes the request.
    assert!(futures::poll!(&mut pending).is_pending());

    shutdown.cancel();
    assert!(pending.await.is_err());
    Ok(())
}

#[tokio::test]
async fn the_last_publication_leaves_out_a_library_that_only_syncs_on_request() -> Result<()> {
    let mut fixture = ShareSwitchFixture::start(false).await?;
    let background = fixture.background;
    // The worker is stopped first: whether its own last publication runs depends on where the
    // stop finds it, so the publication is called here with the session a request leaves.
    fixture.cancel.cancel();
    (&mut fixture.task).await?;
    let after_the_request = new_playlist_key();
    fixture
        .node
        .library_pool
        .acquire_background(background)
        .await?
        .library_sync
        .write_local(
            after_the_request.clone(),
            support::playlist("after the request"),
        )
        .await?;
    let sessions = HashMap::from([(background, memory_session(&fixture.background_store).await)]);
    let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});

    flush_share_worker(&Arc::downgrade(&fixture.node), &sessions, &notify).await;

    fixture.other_device_pass().await?;
    assert!(
        fixture
            .other_device
            .sync_store
            .get(&after_the_request)
            .await?
            .is_none(),
        "the last publication pushed a library that automatic sync does not cover"
    );

    // With automatic sync on, the same publication covers it.
    fixture.set_auto_share_sync(true).await?;
    flush_share_worker(&Arc::downgrade(&fixture.node), &sessions, &notify).await;
    fixture.other_device_pass().await?;
    assert!(
        fixture
            .other_device
            .sync_store
            .get(&after_the_request)
            .await?
            .is_some()
    );
    let ShareSwitchFixture { node, _handles, .. } = fixture;
    drop(_handles);
    node.shutdown().await
}

fn new_playlist_key() -> ObjectKey {
    ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    }
}

#[tokio::test]
async fn share_library_not_open_in_the_ui_syncs_only_on_request_while_automatic_share_sync_is_off()
-> Result<()> {
    let fixture = ShareSwitchFixture::start(false).await?;
    let background = fixture.background;
    let from_other = new_playlist_key();
    fixture
        .other_device
        .sync_store
        .write_local(
            from_other.clone(),
            support::playlist("from the other device"),
        )
        .await?;
    fixture.other_device_pass().await?;
    let from_here = new_playlist_key();
    fixture
        .background_library
        .sync_store
        .write_local(from_here.clone(), support::playlist("from this device"))
        .await?;

    // The library open in the UI is scheduled whatever the switch says. The other one is not.
    fixture.scheduled_ui_passes(3).await?;
    assert_eq!(fixture.sessions_opened(fixture.ui), 1);
    assert_eq!(
        fixture.sessions_opened(background),
        0,
        "a library that is not open in the UI was scheduled"
    );
    assert!(fixture.broadcaster.share_statuses_of(background).is_empty());
    assert!(
        fixture
            .background_library
            .sync_store
            .get(&from_other)
            .await?
            .is_none()
    );

    // A request runs one pass that both pulls and pushes, and is reported like a scheduled one.
    assert_eq!(
        fixture
            .node
            .share_sync_requests
            .sync(background, &fixture.node.token)
            .await?,
        Ok(())
    );
    assert_eq!(fixture.sessions_opened(background), 1);
    assert!(
        fixture
            .background_library
            .sync_store
            .get(&from_other)
            .await?
            .is_some(),
        "the requested pass did not pull"
    );
    fixture.other_device_pass().await?;
    assert!(
        fixture
            .other_device
            .sync_store
            .get(&from_here)
            .await?
            .is_some(),
        "the requested pass did not push"
    );
    assert_eq!(fixture.broadcaster.share_statuses_of(background), [None]);

    // The request the UI sends gets the same pass, and nothing is scheduled afterwards.
    assert_eq!(fixture.request_sync(background.to_string()).await?, None);
    let reported = fixture.broadcaster.share_statuses_of(background).len();
    fixture.scheduled_ui_passes(3).await?;
    assert_eq!(
        fixture.broadcaster.share_statuses_of(background).len(),
        reported
    );

    // With the switch on, the same library is scheduled.
    fixture.set_auto_share_sync(true).await?;
    let later = new_playlist_key();
    fixture
        .other_device
        .sync_store
        .write_local(
            later.clone(),
            support::playlist("later, from the other device"),
        )
        .await?;
    fixture.other_device_pass().await?;
    wait_until(
        "a scheduled pass to pull into the background library",
        || async {
            Ok(fixture
                .background_library
                .sync_store
                .get(&later)
                .await?
                .map(|_| ()))
        },
    )
    .await?;

    // Switching it off again keeps the store session for the next request.
    fixture.set_auto_share_sync(false).await?;
    fixture.scheduled_ui_passes(2).await?;
    assert_eq!(
        fixture
            .node
            .share_sync_requests
            .sync(background, &fixture.node.token)
            .await?,
        Ok(())
    );
    assert_eq!(fixture.sessions_opened(background), 1);
    fixture.stop().await
}

#[tokio::test]
async fn manual_share_sync_answers_with_its_failure_and_runs_again_during_backoff() -> Result<()> {
    let fixture = ShareSwitchFixture::start(false).await?;
    let background = fixture.background;
    let from_here = new_playlist_key();
    fixture
        .background_library
        .sync_store
        .write_local(from_here.clone(), support::playlist("from this device"))
        .await?;
    fixture
        .background_store
        .inject_error(StoreOperation::Put, StoreError::PermissionDenied);

    // Every failure doubles the wait a scheduled pass would observe: after these four it is
    // eight seconds. Each request still runs a pass at once, which is how it gets the failure.
    let started = Instant::now();
    for _ in 0..3 {
        assert_eq!(
            fixture
                .node
                .share_sync_requests
                .sync(background, &fixture.node.token)
                .await?,
            Err(ShareSyncFailure::ReadOnlyShare)
        );
    }
    assert_eq!(
        fixture.request_sync(background.to_string()).await?,
        Some(ShareSyncFailure::ReadOnlyShare)
    );
    // The answer is the report. A broadcast as well would show the failure twice.
    assert!(fixture.broadcaster.share_statuses_of(background).is_empty());

    fixture.background_store.clear_error(StoreOperation::Put);
    assert_eq!(
        fixture
            .node
            .share_sync_requests
            .sync(background, &fixture.node.token)
            .await?,
        Ok(())
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "the requests took longer than the backoff they are meant to ignore"
    );
    fixture.other_device_pass().await?;
    assert!(
        fixture
            .other_device
            .sync_store
            .get(&from_here)
            .await?
            .is_some(),
        "the pass requested during backoff did not push"
    );
    assert_eq!(fixture.broadcaster.share_statuses_of(background), [None]);
    fixture.stop().await
}

#[tokio::test]
async fn manual_share_sync_refuses_a_library_that_is_not_stored_in_a_shared_folder() -> Result<()> {
    use crate::messages::RequestErrorKind;
    use crate::request_error::classify;

    let fixture = ShareSwitchFixture::start(false).await?;
    // The same record as a library whose files are remote and whose catalog is local.
    let redirected = fixture.background;
    let config = fixture.node.config_path.clone();
    LibraryRegistry::update(Path::new(config.as_str()), |registry| {
        let record = registry
            .libraries
            .iter_mut()
            .find(|record| record.library_id == Some(redirected))
            .context("background share record")?;
        record.catalog = CatalogPlacement::Redirected(Uuid::new_v4());
        Ok(())
    })
    .await?;

    for library_id in [redirected, Uuid::new_v4()] {
        let error = fixture
            .node
            .share_sync_requests
            .sync(library_id, &fixture.node.token)
            .await
            .expect_err("a library outside a shared folder was synced with one");
        assert_eq!(classify(&error), RequestErrorKind::InvalidInput);
    }
    let error = fixture
        .request_sync(redirected.to_string())
        .await
        .expect_err("the request for a library outside a shared folder was answered");
    assert_eq!(classify(&error), RequestErrorKind::InvalidInput);
    let error = fixture
        .request_sync("not a library id".into())
        .await
        .expect_err("a library id that is not a UUID was accepted");
    assert_eq!(classify(&error), RequestErrorKind::InvalidInput);
    assert_eq!(
        fixture.sessions_opened(redirected),
        0,
        "a store was opened for a library that is not kept in one"
    );
    assert!(fixture.broadcaster.share_statuses_of(redirected).is_empty());
    fixture.stop().await
}

#[test]
fn record_summary_marks_libraries_stored_in_a_shared_folder() {
    use crate::messages::LibraryRecordSummary;

    let in_share = test_record(Uuid::new_v4());
    assert!(LibraryRecordSummary::from(&in_share).share_sync);
    let mut redirected = test_record(Uuid::new_v4());
    redirected.catalog = CatalogPlacement::Redirected(Uuid::new_v4());
    assert!(!LibraryRecordSummary::from(&redirected).share_sync);
}

/// Adds an analysis row whose file is gone, which metadata maintenance removes.
async fn insert_orphan_analysis(library: &LibraryHandle) -> Result<()> {
    use database::entities::media_analysis;
    use sea_orm::{EntityTrait, Set};

    media_analysis::Entity::insert(media_analysis::ActiveModel {
        file_id: Set(i32::MAX),
        analysis_version: Set(1),
        source_uuid: Set(Uuid::new_v4().to_string()),
        ..Default::default()
    })
    .exec(&*library.main_db)
    .await?;
    Ok(())
}

async fn wait_for_maintenance_pass(library: &LibraryHandle) -> Result<()> {
    use database::entities::media_analysis;
    use sea_orm::{EntityTrait, PaginatorTrait};

    insert_orphan_analysis(library).await?;
    wait_until("metadata maintenance to clean the library", || async {
        Ok((media_analysis::Entity::find()
            .count(&*library.main_db)
            .await?
            == 0)
            .then_some(()))
    })
    .await
}

/// Metadata maintenance and audio verification work on the libraries that are open. With
/// the whole worker running on a node that has no paired device and no library in a shared
/// folder, they are the only work that looks at a library that is not open in the UI.
#[tokio::test]
async fn maintenance_and_audio_verification_never_open_a_library_or_keep_one_open() -> Result<()> {
    use crate::library_pool::LibraryPool;
    use crate::utils::nid::get_or_create_node_id;
    use database::connection::StorageMode;
    use database::library_registry::resolve_library;

    let directory = tempfile::tempdir()?;
    let config = directory.path();
    let path = config.to_string_lossy().into_owned();
    write_quick_identity(config)?;
    crate::test_node::disable_media_server(config)?;
    let node_id = get_or_create_node_id(&path).await?.to_string();
    let _ = migration::initialize_node_id(node_id.clone());
    // The node takes this pool over, and with it the short idle timeout.
    let pool = LibraryPool::with_idle_timeout_and_credentials(
        config,
        config,
        &node_id,
        Duration::from_millis(100),
        Arc::new(CredentialStore::open(config)),
    )?;
    let mut registry = LibraryRegistry::load(config)?;
    let mut ids = Vec::new();
    for name in ["ui", "held"] {
        let root = config.join(name);
        fs::create_dir_all(&root)?;
        let record = resolve_library(
            &FsIo::new(),
            &mut registry,
            &root.to_string_lossy(),
            name,
            StorageMode::Portable,
        )
        .await?;
        ids.push(record.catalog_library_id().context("library id")?);
    }
    let (ui_id, held_id) = (ids[0], ids[1]);
    let held_database = config.join("held/.rune/.0.db");
    let node = NodeContext::new(
        &path,
        &path,
        Arc::new(QuietBroadcaster::default()),
        false,
        RunMode::App,
        Arc::new(Visualizer::new()),
    )
    .await?;
    anyhow::ensure!(
        Arc::ptr_eq(&node.library_pool, &pool),
        "the node did not take over the test pool"
    );
    let ui = pool.acquire_ui(ui_id).await?;
    let mut served = pool.served_library_ids().await?;
    served.sort();
    let mut registered = vec![ui_id, held_id];
    registered.sort();
    assert_eq!(served, registered);

    // Maintenance goes through every served library on each pass, so after two passes over
    // the UI library it has gone past the closed one. Verification runs five times as often.
    wait_for_maintenance_pass(&ui).await?;
    wait_for_maintenance_pass(&ui).await?;
    assert!(
        pool.acquire_open_background(held_id).await.is_none(),
        "background work opened a library"
    );
    assert!(
        !held_database.exists(),
        "background work created the database of a library nobody opened"
    );

    // Once something else has the library open, maintenance works on it too.
    let held = pool.acquire(held_id).await?;
    assert!(held_database.exists());
    wait_for_maintenance_pass(&held).await?;
    // It still closes when that user is gone, although maintenance and verification keep
    // taking it while it is open.
    drop(held);
    wait_until("the held library to idle close", || async {
        Ok(pool
            .acquire_open_background(held_id)
            .await
            .is_none()
            .then_some(()))
    })
    .await?;
    wait_for_maintenance_pass(&ui).await?;
    wait_for_maintenance_pass(&ui).await?;
    assert!(
        pool.acquire_open_background(held_id).await.is_none(),
        "background work reopened a library that had idle closed"
    );

    drop(ui);
    node.shutdown().await
}
