use super::*;
use std::{env, slice};
use sync::share::{
    FORMAT_VERSION, LibraryFile, chunk_key, decode_chunk, decode_manifest, decode_segment,
    encode_library, library_key, parse_key,
};
use tokio::time::timeout;

#[derive(Clone)]
struct StoreTrace {
    events: Arc<Mutex<Vec<String>>>,
}

impl StoreTrace {
    fn record_key(&self, name: &str, operation: &str, key: &str, result: bool) {
        if key.starts_with("devices/") {
            self.record(format!("{name} {operation} {key}: {result}"));
        }
    }

    fn record(&self, event: impl Into<String>) {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(event.into());
    }

    fn snapshot(&self) -> String {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .join("\n    ")
    }
}

struct ObservedStore {
    inner: Arc<dyn ObjectStore>,
    name: &'static str,
    trace: StoreTrace,
}

impl ObjectStore for ObservedStore {
    fn get<'a>(&'a self, key: &'a str) -> SyncBoxFuture<'a, ObjectValue, StoreError> {
        Box::pin(async move {
            let result = self.inner.get(key).await;
            self.trace.record_key(self.name, "get", key, result.is_ok());
            result
        })
    }

    fn get_if_none_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
    ) -> SyncBoxFuture<'a, ConditionalGet, StoreError> {
        Box::pin(async move {
            let result = self.inner.get_if_none_match(key, tag).await;
            self.trace
                .record_key(self.name, "conditional get", key, result.is_ok());
            result
        })
    }

    fn put<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            let result = self.inner.put(key, bytes).await;
            self.trace.record_key(self.name, "put", key, result.is_ok());
            result
        })
    }

    fn create<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            let result = self.inner.create(key, bytes).await;
            self.trace
                .record_key(self.name, "create", key, result.is_ok());
            result
        })
    }

    fn put_if_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            let result = self.inner.put_if_match(key, tag, bytes).await;
            self.trace
                .record_key(self.name, "conditional put", key, result.is_ok());
            result
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> SyncBoxFuture<'a, (), StoreError> {
        Box::pin(async move {
            let result = self.inner.delete(key).await;
            self.trace
                .record_key(self.name, "delete", key, result.is_ok());
            result
        })
    }

    fn list_prefix<'a>(&'a self, prefix: &'a str) -> SyncBoxFuture<'a, Vec<String>, StoreError> {
        Box::pin(async move {
            let result = self.inner.list_prefix(prefix).await;
            if prefix == "devices/" || prefix.starts_with("devices/") {
                self.trace
                    .record(format!("{} list {prefix}: {result:?}", self.name));
            }
            result
        })
    }
}

struct FirstHeadGate {
    inner: Arc<dyn ObjectStore>,
    prefix: String,
    blocked: AtomicBool,
    reached: Notify,
    resume: Notify,
}

impl FirstHeadGate {
    async fn stop_at_first_head(&self, key: &str) {
        if key.starts_with(&format!("devices/{}/head.", self.prefix))
            && !self.blocked.swap(true, Ordering::SeqCst)
        {
            self.reached.notify_one();
            self.resume.notified().await;
        }
    }

    async fn wait_until_blocked(&self) {
        self.reached.notified().await;
    }

    fn resume(&self) {
        self.resume.notify_one();
    }
}

impl ObjectStore for FirstHeadGate {
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
            self.stop_at_first_head(key).await;
            self.inner.put(key, bytes).await
        })
    }

    fn create<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> SyncBoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            self.stop_at_first_head(key).await;
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
            self.stop_at_first_head(key).await;
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

async fn wait_for_successful_share_pass(hub: &WorkerHub, target: usize) -> Result<()> {
    timeout(TEST_DEADLINE, async {
        loop {
            let changed = hub.broadcaster.successful_share_pass.notified();
            if hub
                .broadcaster
                .successful_share_passes
                .load(Ordering::SeqCst)
                >= target
            {
                return;
            }
            changed.await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("existing hub's first successful share pass timed out"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_write_before_first_head_publication_reaches_every_hub_webdav() -> Result<()> {
    local_write_before_first_head_publication_reaches_every_hub_on(ShareBackend::WebDav).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_write_before_first_head_publication_reaches_every_hub_s3() -> Result<()> {
    local_write_before_first_head_publication_reaches_every_hub_on(ShareBackend::S3).await
}

async fn local_write_before_first_head_publication_reaches_every_hub_on(
    backend: ShareBackend,
) -> Result<()> {
    let namespace = Uuid::new_v4();
    let (_fixture, session) = real_share(backend).await;
    // This test is about the window before a head, not about creating the share, so the
    // library file is already there.
    let library_file = LibraryFile {
        format: FORMAT_VERSION,
        library_id: namespace,
        model: RuneModel::model_id().to_owned(),
        schema: RuneModel::schema_id().to_owned(),
    };
    session
        .store
        .put(&library_key(), &encode_library(&library_file)?)
        .await
        .map_err(|error| anyhow::anyhow!("write library file: {error:?}"))?;

    let (existing_library, library) = tokio::join!(
        support::SqlLibrary::new(namespace, Uuid::new_v4()),
        support::SqlLibrary::new(namespace, Uuid::new_v4()),
    );
    let existing_library = existing_library?;
    let library = library?;
    let seed_key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    };
    let (seed_write, prefix, existing_prefix) = tokio::join!(
        existing_library
            .sync_store
            .write_local(seed_key.clone(), support::playlist("")),
        library
            .sync_store
            .share_prefix(env::temp_dir().join(format!("hub-share-{}", Uuid::new_v4()))),
        existing_library
            .sync_store
            .share_prefix(env::temp_dir().join(format!("hub-share-{}", Uuid::new_v4()))),
    );
    seed_write?;
    let prefix = prefix?;
    let existing_prefix = existing_prefix?;
    let trace = StoreTrace {
        events: Arc::new(Mutex::new(Vec::new())),
    };
    let existing_trace = StoreTrace {
        events: Arc::new(Mutex::new(Vec::new())),
    };
    let observed_existing: Arc<dyn ObjectStore> = Arc::new(ObservedStore {
        inner: session.store.clone(),
        name: "hub#0",
        trace: existing_trace.clone(),
    });
    let observed_joining: Arc<dyn ObjectStore> = Arc::new(ObservedStore {
        inner: session.store.clone(),
        name: "hub#1",
        trace: trace.clone(),
    });
    let gate = Arc::new(FirstHeadGate {
        inner: observed_joining,
        prefix,
        blocked: AtomicBool::new(false),
        reached: Notify::new(),
        resume: Notify::new(),
    });
    let gated_session = ShareStoreSession {
        store: gate.clone(),
        capabilities: session.capabilities,
    };
    let (existing_hub, joining_hub) = tokio::join!(
        worker_hub_on_share(
            namespace,
            ShareStoreSession {
                store: observed_existing,
                capabilities: session.capabilities,
            },
            existing_library,
        ),
        worker_hub_on_share_prepared(namespace, gated_session, library),
    );
    let existing_hub = existing_hub?;
    let mut joining_hub = joining_hub?;
    wait_for_successful_share_pass(&existing_hub, 1).await?;
    joining_hub.start_worker();

    if timeout(TEST_DEADLINE, gate.wait_until_blocked())
        .await
        .is_err()
    {
        let messages = joining_hub
            .broadcaster
            .messages
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let objects = session
            .store
            .list_prefix("devices/")
            .await
            .map_err(|error| anyhow::anyhow!("list share devices: {error:?}"))?;
        anyhow::bail!(
            "{backend:?} joining hub did not reach first head publication; messages: {messages:?}; objects: {objects:?}"
        );
    }
    let joining_objects = session
        .store
        .list_prefix(&format!("devices/{}/", gate.prefix))
        .await
        .map_err(|error| anyhow::anyhow!("list joining hub objects: {error:?}"))?;
    anyhow::ensure!(
        joining_objects.iter().all(|key| !key.contains("/head.")),
        "joining hub published a head before the gate"
    );
    anyhow::ensure!(
        trace
            .snapshot()
            .contains(&format!("hub#1 get devices/{existing_prefix}/head.")),
        "joining hub reached first head publication without fetching the existing hub's head"
    );
    anyhow::ensure!(
        joining_hub
            .library
            .sync_store
            .get(&seed_key)
            .await?
            .is_some(),
        "joining hub reached first head publication before pulling the seed"
    );
    let write_key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    };
    joining_hub
        .library
        .sync_store
        .write_local(write_key.clone(), support::playlist(""))
        .await?;
    let last_passes: Vec<_> = hubs_successful_passes(&[&existing_hub, &joining_hub]);
    gate.resume();
    let hubs = [existing_hub, joining_hub];
    if let Err(error) =
        wait_for_keys_after_worker_events(&hubs, slice::from_ref(&write_key), last_passes).await
    {
        let observations = format!(
            "store snapshot: {}\n  hub#0 store operations:\n    {}\n  hub#1 store operations:\n    {}\n  hub state: {}",
            describe_share_objects(session.store.as_ref(), &gate.prefix, &write_key).await?,
            existing_trace.snapshot(),
            trace.snapshot(),
            describe_hub_states(&hubs).await?,
        );
        anyhow::bail!(
            "{backend:?} write before first head did not converge: {error:#}; {}; {observations}",
            key_presence_on_hubs(&hubs, &write_key).await?,
        );
    }
    let [existing_hub, joining_hub] = hubs;
    let (existing_result, joining_result) = tokio::join!(existing_hub.stop(), joining_hub.stop());
    existing_result?;
    joining_result?;
    Ok(())
}

fn hubs_successful_passes(hubs: &[&WorkerHub]) -> Vec<usize> {
    hubs.iter()
        .map(|hub| {
            hub.broadcaster
                .successful_share_passes
                .load(Ordering::SeqCst)
        })
        .collect()
}

async fn wait_for_keys_after_worker_events(
    hubs: &[WorkerHub],
    keys: &[ObjectKey],
    mut last_passes: Vec<usize>,
) -> Result<()> {
    timeout(TEST_DEADLINE, async {
        let mut observed_passes = vec![false; hubs.len()];
        loop {
            let first_event = hubs[0].broadcaster.successful_share_pass.notified();
            let second_event = hubs[1].broadcaster.successful_share_pass.notified();
            let current = hubs_successful_passes(&hubs.iter().collect::<Vec<_>>());
            for (index, (now, previous)) in current.iter().zip(&last_passes).enumerate() {
                observed_passes[index] |= now > previous;
            }
            last_passes = current;
            if observed_passes.iter().all(|observed| *observed) {
                wait_until("share keys to reach every hub", || async {
                    let records =
                        try_join_all(hubs.iter().flat_map(|hub| {
                            keys.iter().map(|key| hub.library.sync_store.get(key))
                        }))
                        .await?;
                    Ok(records.iter().all(Option::is_some).then_some(()))
                })
                .await?;
                return Ok::<_, anyhow::Error>(());
            }
            tokio::select! {
                _ = first_event => {},
                _ = second_event => {},
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("timed out waiting for worker events to converge share keys"))??;
    Ok(())
}

async fn describe_share_objects(
    store: &dyn ObjectStore,
    joining_prefix: &str,
    write_key: &ObjectKey,
) -> Result<String> {
    let keys = store
        .list_prefix("devices/")
        .await
        .map_err(|error| anyhow::anyhow!("list share devices: {error:?}"))?;
    let mut details = Vec::new();
    for key in keys {
        if !key.starts_with(&format!("devices/{joining_prefix}/")) {
            continue;
        }
        let Ok(value) = store.get(&key).await else {
            details.push(format!("{key}: disappeared before diagnostic read"));
            continue;
        };
        match parse_key(&key)? {
            ShareObjectKey::Head { .. } => {
                let head = decode_head(&value.bytes)?;
                details.push(format!(
                    "{key}: published counter={} journal_head={} local_cursor={} segments={:?} bases={:?}",
                    head.counter, head.head.seq, head.local_cursor, head.segments, head.bases
                ));
                for segment_ref in &head.segments {
                    let segment_path = format!("devices/{joining_prefix}/seg/{}", segment_ref.name);
                    let segment_value = store
                        .get(&segment_path)
                        .await
                        .map_err(|error| anyhow::anyhow!("get {segment_path}: {error:?}"))?;
                    let segment = decode_segment::<RuneModel>(&segment_value.bytes)?;
                    details.push(format!(
                        "{segment_path}: links={:?}",
                        segment
                            .links
                            .iter()
                            .map(|link| (link.seq, &link.collection, &link.event))
                            .collect::<Vec<_>>()
                    ));
                }
                for base_ref in &head.bases {
                    let base_path = base_key(joining_prefix, &base_ref.base_id)?;
                    let base_value = store
                        .get(&base_path)
                        .await
                        .map_err(|error| anyhow::anyhow!("get {base_path}: {error:?}"))?;
                    let manifest =
                        decode_manifest::<RuneModel>(&base_value.bytes, &base_ref.base_id)?;
                    details.push(format!(
                        "{base_path}: base covers={:?} collections={:?}",
                        manifest.covers,
                        manifest.collections.keys().collect::<Vec<_>>()
                    ));
                    for summary in manifest.collections.values() {
                        for bucket in &summary.buckets {
                            let chunk_path = chunk_key(joining_prefix, &bucket.chunk)?;
                            let chunk_value = store
                                .get(&chunk_path)
                                .await
                                .map_err(|error| anyhow::anyhow!("get {chunk_path}: {error:?}"))?;
                            let chunk = decode_chunk(&bucket.chunk, &chunk_value.bytes)?;
                            let has_write = String::from_utf8_lossy(&chunk.canonical_bytes)
                                .contains(&write_key.id.to_string());
                            details.push(format!("{chunk_path}: contains_write_key={has_write}"));
                        }
                    }
                }
            }
            _ => details.push(format!("{key}: present ({} bytes)", value.bytes.len())),
        }
    }
    Ok(details.join("; "))
}

async fn describe_hub_states(hubs: &[WorkerHub]) -> Result<String> {
    let mut states = Vec::with_capacity(hubs.len());
    for (index, hub) in hubs.iter().enumerate() {
        let identity = hub.library.peer.identity().await?;
        let head = hub.library.peer.head().await?;
        states.push(format!(
            "hub#{index} replica={} journal_head={}",
            identity.replica, head.sequence
        ));
    }
    Ok(states.join(", "))
}
