use super::*;
use std::{env, slice};
use tokio::time::timeout;

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

async fn wait_for_first_successful_share_pass(hub: &WorkerHub) -> Result<()> {
    wait_until("existing hub's first successful share pass", || async {
        Ok((hub
            .broadcaster
            .successful_share_passes
            .load(Ordering::SeqCst)
            > 0)
        .then_some(()))
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_write_before_first_head_publication_reaches_every_hub() -> Result<()> {
    let (webdav, s3) = tokio::join!(
        local_write_before_first_head_publication_reaches_every_hub_on(ShareBackend::WebDav),
        local_write_before_first_head_publication_reaches_every_hub_on(ShareBackend::S3),
    );
    webdav?;
    s3?;
    Ok(())
}

async fn local_write_before_first_head_publication_reaches_every_hub_on(
    backend: ShareBackend,
) -> Result<()> {
    let namespace = Uuid::new_v4();
    let (_fixture, session) = real_share(backend).await;

    let existing_library = support::SqlLibrary::new_persistent(namespace, Uuid::new_v4()).await?;
    let seed_key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    };
    existing_library
        .library
        .write_local(seed_key.clone(), support::playlist("s"))
        .await?;
    let library = support::SqlLibrary::new_persistent(namespace, Uuid::new_v4()).await?;
    let prefix = library
        .library
        .share_prefix(env::temp_dir().join(format!("hub-share-{}", Uuid::new_v4())))
        .await?;
    let gate = Arc::new(FirstHeadGate {
        inner: session.store.clone(),
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
        worker_hub_on_share(namespace, session.clone(), existing_library),
        worker_hub_on_share_prepared(namespace, gated_session, library),
    );
    let existing_hub = existing_hub?;
    let mut joining_hub = joining_hub?;
    wait_for_first_successful_share_pass(&existing_hub).await?;
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
        joining_hub.library.library.get(&seed_key).await?.is_some(),
        "joining hub reached first head publication before pulling the seed"
    );
    let write_key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    };
    joining_hub
        .library
        .library
        .write_local(write_key.clone(), support::playlist("w"))
        .await?;
    gate.resume();

    let hubs = [existing_hub, joining_hub];
    if let Err(error) = wait_for_keys_on_hubs(&hubs, slice::from_ref(&write_key)).await {
        anyhow::bail!(
            "{backend:?} write before first head did not converge: {error:#}; {}",
            key_presence_on_hubs(&hubs, &write_key).await?
        );
    }
    let [existing_hub, joining_hub] = hubs;
    let (existing_result, joining_result) = tokio::join!(existing_hub.stop(), joining_hub.stop());
    existing_result?;
    joining_result?;
    Ok(())
}
