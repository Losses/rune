use super::*;
use crate::library_pool::{LibraryHandle, LibraryResources};
use crate::node::NodeContext;
use crate::server::utils::device::RunMode;
use crate::test_wait::{TEST_DEADLINE, wait_until};
use crate::utils::{Broadcaster, RinfRustSignal};
use database::connection::RecommendationDbConnection;
use database::credentials::{Credential, CredentialRef, CredentialStore, Secret};
use database::library_registry::{CatalogPlacement, FsAdapter, LibraryRecordId, ProtocolAdapter};
use database::sync::LibrarySyncStore;
use database::sync::model::{Collection, ObjectKey, RuneModel};
use database::sync::share::{BaseCutRequest, ShareWriter, cut_base};
use database::sync::share::{load_or_create_state, save_state};
use fsio::test_fixtures::{S3Fixture, WebDavFixture};
use fsio::{OpenDalConfig, OpenDalObjectStore};
use futures::future::try_join_all;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
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
use tokio::time::Instant;
use uuid::Uuid;

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
        background_sync: true,
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
    let cache = std::env::temp_dir().join(format!("hub-share-{}", Uuid::new_v4()));
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
    let cache = std::env::temp_dir().join(format!("hub-share-{}", Uuid::new_v4()));
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

async fn real_share(backend: ShareBackend) -> (ShareFixture, ShareStoreSession) {
    let (fixture, config) = match backend {
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
    };
    let store = Arc::new(OpenDalObjectStore::new(config).unwrap());
    let capabilities = store.probe("devices/probe.generation").await.unwrap();
    let session = ShareStoreSession {
        store,
        capabilities: ShareCapabilities {
            conditional_create: capabilities.conditional_create,
            conditional_head_write: capabilities.conditional_put,
        },
    };
    (fixture, session)
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
    messages: std::sync::Mutex<Vec<String>>,
    successful_share_passes: AtomicUsize,
    successful_share_pass: Notify,
}

impl Broadcaster for QuietBroadcaster {
    fn broadcast(&self, message: &dyn RinfRustSignal) {
        if message
            .as_any()
            .downcast_ref::<LibrarySyncStatusUpdated>()
            .is_some_and(|status| status.share_failure.is_none())
        {
            self.successful_share_passes.fetch_add(1, Ordering::SeqCst);
            self.successful_share_pass.notify_waiters();
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
    task: Option<tokio::task::JoinHandle<()>>,
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
            register_worker_library(&self._directory, self.namespace, &self.library).await?;
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

/// A node generates an RSA device certificate on first start, which takes seconds in a
/// debug build, and the share worker never uses it. Starting from an elliptic-curve
/// identity written beforehand, made in milliseconds, avoids that wait.
fn write_quick_identity(directory: &std::path::Path) -> Result<()> {
    let key = rcgen::KeyPair::generate()?;
    let certificate =
        rcgen::CertificateParams::new(vec!["share-test".to_owned()])?.self_signed(&key)?;
    std::fs::write(directory.join("cid"), "R-share-test")?;
    std::fs::write(directory.join("certificate.pem"), certificate.pem())?;
    std::fs::write(directory.join("private_key.pem"), key.serialize_pem())?;
    Ok(())
}

async fn register_worker_library(
    directory: &tempfile::TempDir,
    namespace: Uuid,
    library: &support::SqlLibrary,
) -> Result<(Arc<NodeContext>, Arc<QuietBroadcaster>, LibraryHandle)> {
    let path = directory.path().to_string_lossy().into_owned();
    write_quick_identity(directory.path())?;
    let broadcaster = Arc::new(QuietBroadcaster::default());
    let node = NodeContext::new(
        &path,
        &path,
        broadcaster.clone(),
        false,
        RunMode::App,
        Arc::new(::visualizer::Visualizer::new()),
    )
    .await?;
    let record = test_record(namespace);
    std::fs::write(
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
        fsio: Arc::new(fsio::FsIo::new()),
        media_root: library.root.path().to_string_lossy().into_owned(),
        descriptor: test_record(namespace),
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

async fn worker_hub_on_share_prepared(
    namespace: Uuid,
    session: ShareStoreSession,
    library: support::SqlLibrary,
) -> Result<WorkerHub> {
    let timers = ShareWorkerTimers {
        tick: Duration::from_millis(25),
        foreground_pull: Duration::from_millis(50),
        background_pull: Duration::from_millis(50),
        push_debounce: Duration::from_millis(10),
        statistics_push: Duration::from_millis(50),
        idle_push: Duration::from_millis(100),
    };
    let factory: Arc<dyn ShareStoreFactory> = Arc::new(FixedShareFactory { session });
    let directory = tempfile::tempdir()?;
    let (node, broadcaster, handle) =
        register_worker_library(&directory, namespace, &library).await?;
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
    let node = NodeContext::new(
        &path,
        &path,
        broadcaster,
        false,
        RunMode::App,
        Arc::new(::visualizer::Visualizer::new()),
    )
    .await?;
    std::fs::write(
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
            fsio: Arc::new(fsio::FsIo::new()),
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

    let result = OpenDalShareStoreFactory
        .open(&record, "devices/fixture", &credentials)
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
            .write_local(seed_key.clone(), support::playlist("crash recovery seed"))
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
        if tokio::time::timeout(TEST_DEADLINE, crash_store.wait_for_head_failure())
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
            chrono::Utc::now().timestamp_millis(),
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
    session
        .store
        .list_prefix("devices/")
        .await
        .map_err(|error| anyhow::anyhow!("list devices after base: {error:?}"))?;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    };
    library
        .sync_store
        .write_local(key, support::playlist("retry conditional head conflict"))
        .await?;
    fixture.conflict_next_conditional_put();
    detailed_sync_pass(&library.sync_store, namespace, &session).await?;
    anyhow::ensure!(
        !fixture.conditional_put_conflict_pending(),
        "share writer never encountered the injected conditional 409"
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
            chrono::Utc::now().timestamp_millis(),
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
            chrono::Utc::now().timestamp_millis(),
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
            chrono::Utc::now().timestamp_millis(),
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
            chrono::Utc::now().timestamp_millis(),
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
            chrono::Utc::now().timestamp_millis(),
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
            chrono::Utc::now().timestamp_millis(),
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
            .write_local(
                shared_key,
                support::playlist("retired owner has shared data"),
            )
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
        let now_ms = chrono::Utc::now().timestamp_millis();
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
            chrono::Utc::now().timestamp_millis(),
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
    let cache = std::env::temp_dir().join(format!("hub-share-{}", Uuid::new_v4()));
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

    /// A node with `fingerprint` approved and `host` its trusted host.
    async fn node_with_trusted_peer(
        directory: &tempfile::TempDir,
        fingerprint: &str,
        host: &str,
    ) -> Result<Arc<NodeContext>> {
        let path = directory.path().to_string_lossy().into_owned();
        let node = NodeContext::new(
            &path,
            &path,
            Arc::new(QuietBroadcaster::default()),
            false,
            RunMode::App,
            Arc::new(::visualizer::Visualizer::new()),
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
    async fn watch(
        node: &Arc<NodeContext>,
        fingerprint: &str,
        host: &str,
    ) -> tokio::task::JoinHandle<()> {
        let weak = Arc::downgrade(node);
        let (fingerprint, host) = (fingerprint.to_owned(), host.to_owned());
        let watcher =
            tokio::spawn(async move { wait_for_revocation(weak, &fingerprint, &host).await });
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert!(!watcher.is_finished(), "an approved peer is not revoked");
        watcher
    }

    /// With the clock stopped from here on, only a change can end the watcher before a second of
    /// the clock has passed: a timer would need it to move.
    async fn ends_within_a_tick(watcher: &tokio::task::JoinHandle<()>) -> Result<()> {
        let started = tokio::time::Instant::now();
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
        tokio::time::pause();

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
        tokio::time::pause();

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
        tokio::time::pause();

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
