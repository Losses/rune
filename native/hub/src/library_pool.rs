use std::{
    collections::HashMap,
    fs,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, OnceLock, Weak},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use database::{
    connection::{MainDbConnection, RecommendationDbConnection, StorageMode, check_storage_mode},
    library_registry::{
        DbPlacement, LibraryAccess, LibraryDescriptor, LibraryRegistry, LibraryStorage,
    },
    sync::LibrarySyncStore,
};
use fsio::FsIo;
use tokio::{sync::Mutex, time::Instant};
use uuid::Uuid;

use crate::utils::{initialize_databases, initialize_library_replica};

const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
static POOLS: OnceLock<StdMutex<HashMap<PathBuf, Weak<LibraryPool>>>> = OnceLock::new();

/// Connections and storage belonging to one library. Keep a handle alive while using them.
pub struct LibraryResources {
    pub main_db: Arc<MainDbConnection>,
    pub library_sync: LibrarySyncStore,
    pub recommend_db: Arc<RecommendationDbConnection>,
    pub fsio: Arc<FsIo>,
    pub media_root: String,
    pub descriptor: LibraryDescriptor,
}

struct Entry {
    resources: Arc<LibraryResources>,
    ui_users: usize,
    last_released: Arc<StdMutex<Instant>>,
}

/// A lease which prevents idle closure until all its clones have been dropped.
#[derive(Clone)]
pub struct LibraryHandle {
    resources: Arc<LibraryResources>,
    pool: Arc<LibraryPool>,
    last_released: Arc<StdMutex<Instant>>,
}

impl LibraryHandle {
    /// The node-level pool owning this connection.
    pub fn pool(&self) -> Arc<LibraryPool> {
        self.pool.clone()
    }
}

impl Deref for LibraryHandle {
    type Target = LibraryResources;

    fn deref(&self) -> &Self::Target {
        &self.resources
    }
}

impl Drop for LibraryHandle {
    fn drop(&mut self) {
        *self.last_released.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }
}

/// One connection manager per node configuration, shared across library switches.
pub struct LibraryPool {
    config_dir: PathBuf,
    db_path: PathBuf,
    node_id: String,
    idle_timeout: Duration,
    entries: Mutex<HashMap<Uuid, Entry>>,
}

impl LibraryPool {
    /// Get this node's process-wide pool, with a five-minute idle timeout.
    pub fn new(config_dir: &Path, db_path: &Path, node_id: &str) -> Result<Arc<Self>> {
        Self::with_idle_timeout(config_dir, db_path, node_id, DEFAULT_IDLE_TIMEOUT)
    }

    /// Configure the idle timeout when first creating a node's pool.
    pub fn with_idle_timeout(
        config_dir: &Path,
        db_path: &Path,
        node_id: &str,
        idle_timeout: Duration,
    ) -> Result<Arc<Self>> {
        fs::create_dir_all(config_dir).context("Failed to create node configuration directory")?;
        let config_dir = config_dir.canonicalize()?;
        fs::create_dir_all(db_path).context("Failed to create database directory")?;
        let db_path = db_path.canonicalize()?;
        let mut pools = POOLS
            .get_or_init(StdMutex::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(pool) = pools.get(&config_dir).and_then(Weak::upgrade) {
            ensure!(
                pool.node_id == node_id && pool.db_path == db_path,
                "Node pool configuration changed"
            );
            return Ok(pool);
        }
        let pool = Arc::new(Self {
            config_dir: config_dir.clone(),
            db_path,
            node_id: node_id.to_owned(),
            idle_timeout,
            entries: Mutex::new(HashMap::new()),
        });
        pools.insert(config_dir, Arc::downgrade(&pool));
        let weak = Arc::downgrade(&pool);
        let interval = idle_timeout.clamp(Duration::from_millis(1), Duration::from_secs(30));
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let Some(pool) = weak.upgrade() else { return };
                if let Err(error) = pool.close_idle().await {
                    log::warn!("Could not close an idle library: {error:#}");
                }
            }
        });
        Ok(pool)
    }

    fn handle(self: &Arc<Self>, entry: &Entry) -> LibraryHandle {
        LibraryHandle {
            resources: entry.resources.clone(),
            pool: self.clone(),
            last_released: entry.last_released.clone(),
        }
    }

    /// Libraries enabled for background work, including any library registered by the UI.
    pub async fn served_library_ids(&self) -> Result<Vec<Uuid>> {
        let mut ids: Vec<_> = LibraryRegistry::load(&self.config_dir)?
            .libraries
            .into_iter()
            .filter(|library| library.background_sync)
            .map(|library| library.library_id)
            .collect();
        for (id, entry) in self.entries.lock().await.iter() {
            if entry.ui_users > 0 && !ids.contains(id) {
                ids.push(*id);
            }
        }
        Ok(ids)
    }

    /// Resolve the current serving policy before acquiring a connection.
    pub async fn acquire_served(
        self: &Arc<Self>,
        library_id: Uuid,
    ) -> Result<Option<LibraryHandle>> {
        if !self.served_library_ids().await?.contains(&library_id) {
            return Ok(None);
        }
        self.acquire(library_id).await.map(Some)
    }

    /// Acquire an existing connection or open the registered library once.
    pub async fn acquire(self: &Arc<Self>, library_id: Uuid) -> Result<LibraryHandle> {
        // Keep initialization alive if a worker is cancelled while waiting for it. Dropping
        // the future while migrations are in flight would release the serialization lock
        // before SQLite has necessarily stopped executing the migration statements.
        let pool = self.clone();
        tokio::spawn(async move { pool.acquire_inner(library_id).await })
            .await
            .context("Library acquisition task failed")?
    }

    async fn acquire_inner(self: &Arc<Self>, library_id: Uuid) -> Result<LibraryHandle> {
        let mut entries = self.entries.lock().await;
        let descriptor = LibraryRegistry::load(&self.config_dir)?
            .libraries
            .into_iter()
            .find(|library| library.library_id == library_id)
            .context("Library is not registered on this node")?;
        ensure!(
            descriptor.access_level != Some(LibraryAccess::NotShared),
            "Library access revoked"
        );
        if let Some(entry) = entries.get(&library_id) {
            return Ok(self.handle(entry));
        }
        let LibraryStorage::Local { path } = &descriptor.storage;
        #[cfg(not(target_os = "android"))]
        let fsio = Arc::new(FsIo::new());
        #[cfg(target_os = "android")]
        let fsio = Arc::new(FsIo::new(Path::new(".rune/.android-fs.db"), path)?);
        let mode = match descriptor.db_placement {
            DbPlacement::Portable => StorageMode::Portable,
            DbPlacement::Redirected { storage_key } => StorageMode::Redirected(storage_key),
        };
        ensure!(
            check_storage_mode(&fsio, path)? == mode,
            "Library database placement disagrees with its descriptor"
        );
        let db_path = self
            .db_path
            .to_str()
            .context("Database path is not UTF-8")?;
        let databases = initialize_databases(&fsio, path, Some(db_path), &self.node_id).await?;
        let library_sync =
            initialize_library_replica(&databases.main_db, &self.node_id, library_id).await?;
        let entry = Entry {
            resources: Arc::new(LibraryResources {
                main_db: databases.main_db,
                recommend_db: databases.recommend_db,
                library_sync,
                fsio,
                media_root: path.clone(),
                descriptor,
            }),
            ui_users: 0,
            last_released: Arc::new(StdMutex::new(Instant::now())),
        };
        let handle = self.handle(&entry);
        entries.insert(library_id, entry);
        Ok(handle)
    }

    /// Open for the UI, reusing any connection already acquired in the background.
    pub async fn acquire_ui(self: &Arc<Self>, library_id: Uuid) -> Result<LibraryHandle> {
        self.forget_moved(library_id).await?;
        let handle = self.acquire(library_id).await?;
        let mut entries = self.entries.lock().await;
        let entry = entries
            .get_mut(&library_id)
            .context("Library connection disappeared")?;
        entry.ui_users += 1;
        Ok(handle)
    }

    /// Forget a cached connection whose library was reopened at another path, so
    /// the UI gets storage under the path it opened (a portable library on a
    /// drive mounted elsewhere). Holders of the old handle keep it until dropped.
    async fn forget_moved(&self, library_id: Uuid) -> Result<()> {
        let registered = LibraryRegistry::load(&self.config_dir)?
            .libraries
            .into_iter()
            .find(|library| library.library_id == library_id);
        let Some(LibraryDescriptor {
            storage: LibraryStorage::Local { path },
            ..
        }) = registered
        else {
            return Ok(());
        };
        let mut entries = self.entries.lock().await;
        if entries
            .get(&library_id)
            .is_some_and(|entry| entry.resources.media_root != path)
        {
            entries.remove(&library_id);
        }
        Ok(())
    }

    /// Test seam for registering an already-open UI connection. Production opens through `acquire`.
    pub async fn register_ui(
        self: &Arc<Self>,
        resources: LibraryResources,
    ) -> Result<LibraryHandle> {
        let mut entries = self.entries.lock().await;
        let id = resources.descriptor.library_id;
        if let Some(entry) = entries.get_mut(&id) {
            ensure!(
                Arc::ptr_eq(&entry.resources.main_db, &resources.main_db),
                "Library already has a different open connection"
            );
            entry.ui_users += 1;
            return Ok(self.handle(entry));
        }
        let entry = Entry {
            resources: Arc::new(resources),
            ui_users: 1,
            last_released: Arc::new(StdMutex::new(Instant::now())),
        };
        let handle = self.handle(&entry);
        entries.insert(id, entry);
        Ok(handle)
    }

    /// Release the UI's registration after its listeners and workers have stopped.
    pub async fn unregister_ui(&self, library_id: Uuid) {
        if let Some(entry) = self.entries.lock().await.get_mut(&library_id) {
            entry.ui_users = entry.ui_users.saturating_sub(1);
            *entry
                .last_released
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Instant::now();
        }
    }

    async fn close_idle(&self) -> Result<()> {
        let mut entries = self.entries.lock().await;
        let idle: Vec<_> = entries
            .iter()
            .filter(|(_, entry)| {
                entry.ui_users == 0
                    && Arc::strong_count(&entry.resources) == 1
                    && entry
                        .last_released
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .elapsed()
                        >= self.idle_timeout
            })
            .map(|(id, _)| *id)
            .collect();
        for id in idle {
            if let Some(entry) = entries.get(&id) {
                entry
                    .resources
                    .main_db
                    .close_by_ref()
                    .await
                    .context("Failed to close library database")?;
            }
            entries.remove(&id);
        }
        Ok(())
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    use database::{connection::create_redirect, library_registry::resolve_library};
    use sea_orm::TransactionTrait;
    use tempfile::TempDir;
    use tokio::time::{sleep, timeout};

    struct Fixture {
        dir: TempDir,
        pool: Arc<LibraryPool>,
        descriptor: LibraryDescriptor,
        fsio: Arc<FsIo>,
        root: String,
        node_id: String,
    }

    impl Fixture {
        async fn new(redirected: bool) -> Result<Self> {
            let dir = TempDir::new()?;
            let root = dir.path().join("media").to_string_lossy().into_owned();
            let fsio = Arc::new(FsIo::new());
            if redirected {
                create_redirect(&fsio, &root).await?;
            }
            let mode = check_storage_mode(&fsio, &root)?;
            let mut registry = LibraryRegistry::load(dir.path())?;
            let descriptor = resolve_library(&fsio, &mut registry, &root, "Test", mode).await?;
            let node_id = Uuid::new_v4().to_string();
            let pool = LibraryPool::with_idle_timeout(
                dir.path(),
                dir.path(),
                &node_id,
                Duration::from_millis(50),
            )?;
            Ok(Self {
                dir,
                pool,
                descriptor,
                fsio,
                root,
                node_id,
            })
        }

        async fn open_resources(&self) -> Result<LibraryResources> {
            let databases = initialize_databases(
                &self.fsio,
                &self.root,
                self.dir.path().to_str(),
                &self.node_id,
            )
            .await?;
            let library_sync = initialize_library_replica(
                &databases.main_db,
                &self.node_id,
                self.descriptor.library_id,
            )
            .await?;
            Ok(LibraryResources {
                main_db: databases.main_db,
                recommend_db: databases.recommend_db,
                library_sync,
                fsio: self.fsio.clone(),
                media_root: self.root.clone(),
                descriptor: self.descriptor.clone(),
            })
        }

        async fn wait_closed(&self) -> Result<()> {
            timeout(Duration::from_secs(3), async {
                while self
                    .pool
                    .entries
                    .lock()
                    .await
                    .contains_key(&self.descriptor.library_id)
                {
                    sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .context("Idle library was not closed")
        }
    }

    #[tokio::test]
    async fn acquire_reuses_connection_and_namespace() -> Result<()> {
        let f = Fixture::new(false).await?;
        let id = f.descriptor.library_id;
        let (a, b) = tokio::join!(f.pool.acquire(id), f.pool.acquire(id));
        let (a, b) = (a?, b?);
        assert!(Arc::ptr_eq(&a.main_db, &b.main_db));
        assert_eq!(a.library_sync.record_store().namespace(), id);
        assert_eq!(a.descriptor, f.descriptor);
        assert_eq!(a.media_root, f.root);
        let same_pool = LibraryPool::new(f.dir.path(), f.dir.path(), &f.node_id)?;
        assert!(Arc::ptr_eq(&same_pool, &f.pool));
        assert!(Arc::ptr_eq(
            &a.main_db,
            &same_pool.acquire(id).await?.main_db
        ));
        assert!(f.pool.acquire(Uuid::new_v4()).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn ui_open_follows_a_moved_library() -> Result<()> {
        let f = Fixture::new(false).await?;
        let id = f.descriptor.library_id;
        let background = f.pool.acquire(id).await?;
        let moved = f.dir.path().join("moved").to_string_lossy().into_owned();
        fs::rename(&f.root, &moved)?;
        let mode = check_storage_mode(&f.fsio, &moved)?;
        let mut registry = LibraryRegistry::load(f.dir.path())?;
        let reopened = resolve_library(&f.fsio, &mut registry, &moved, "Test", mode).await?;
        assert_eq!(reopened.library_id, id);
        let ui = f.pool.acquire_ui(id).await?;
        assert_eq!(ui.media_root, moved);
        assert_eq!(background.media_root, f.root);
        assert_eq!(f.pool.acquire(id).await?.media_root, moved);
        Ok(())
    }

    #[tokio::test]
    async fn ui_registration_preserves_connection_and_prevents_idle_close() -> Result<()> {
        let f = Fixture::new(false).await?;
        let resources = f.open_resources().await?;
        let connection = Arc::downgrade(&resources.main_db);
        let ui = f.pool.register_ui(resources).await?;
        drop(ui);
        sleep(f.pool.idle_timeout * 3).await;
        let acquired = f.pool.acquire(f.descriptor.library_id).await?;
        assert!(Arc::ptr_eq(
            &connection.upgrade().unwrap(),
            &acquired.main_db
        ));
        f.pool.unregister_ui(f.descriptor.library_id).await;
        sleep(f.pool.idle_timeout * 3).await;
        assert!(
            f.pool
                .entries
                .lock()
                .await
                .contains_key(&f.descriptor.library_id)
        );
        drop(acquired);
        f.wait_closed().await?;
        assert!(connection.upgrade().is_none());
        Ok(())
    }

    #[tokio::test]
    async fn idle_close_waits_for_last_handle_then_reopens() -> Result<()> {
        let f = Fixture::new(false).await?;
        let first = f.pool.acquire(f.descriptor.library_id).await?;
        let last = first.clone();
        let connection = Arc::downgrade(&first.main_db);
        drop(first);
        sleep(f.pool.idle_timeout * 3).await;
        assert!(connection.upgrade().is_some());
        let held = f.pool.acquire(f.descriptor.library_id).await?;
        assert!(Arc::ptr_eq(&last.main_db, &held.main_db));
        drop(held);
        drop(last);
        f.wait_closed().await?;
        assert!(connection.upgrade().is_none());
        let reopened = f.pool.acquire(f.descriptor.library_id).await?;
        assert!(!Weak::ptr_eq(
            &connection,
            &Arc::downgrade(&reopened.main_db)
        ));
        assert_eq!(
            reopened.library_sync.record_store().namespace(),
            f.descriptor.library_id
        );
        Ok(())
    }

    #[tokio::test]
    async fn acquire_respects_redirected_placement() -> Result<()> {
        let f = Fixture::new(true).await?;
        let handle = f.pool.acquire(f.descriptor.library_id).await?;
        let DbPlacement::Redirected { storage_key } = handle.descriptor.db_placement else {
            panic!("redirected fixture");
        };
        assert!(
            f.dir
                .path()
                .join(storage_key.to_string())
                .join(".0.db")
                .exists()
        );
        assert!(!Path::new(&f.root).join(".rune/.0.db").exists());
        drop(handle);
        f.wait_closed().await?;
        let reopened = f.pool.acquire(f.descriptor.library_id).await?;
        assert_eq!(reopened.descriptor, f.descriptor);
        Ok(())
    }

    #[tokio::test]
    async fn closing_waits_for_in_flight_transaction_before_reopening() -> Result<()> {
        let f = Fixture::new(false).await?;
        let handle = f.pool.acquire(f.descriptor.library_id).await?;
        let transaction = handle.main_db.begin().await?;
        drop(handle);
        {
            let entries = f.pool.entries.lock().await;
            let entry = entries.get(&f.descriptor.library_id).unwrap();
            *entry.last_released.lock().unwrap() = Instant::now() - f.pool.idle_timeout;
        }
        let pool = f.pool.clone();
        let closing = tokio::spawn(async move { pool.close_idle().await });
        sleep(Duration::from_millis(30)).await;
        assert!(!closing.is_finished());
        let pool = f.pool.clone();
        let id = f.descriptor.library_id;
        let reopening = tokio::spawn(async move { pool.acquire(id).await });
        sleep(Duration::from_millis(30)).await;
        assert!(!reopening.is_finished());
        transaction.commit().await?;
        timeout(Duration::from_secs(3), closing).await???;
        let reopened = timeout(Duration::from_secs(3), reopening).await???;
        assert_eq!(reopened.descriptor.library_id, id);
        Ok(())
    }
}
