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
    credentials::{CredentialRef, CredentialStore},
    error::ExpectedError,
    library_registry::{DbPlacement, FsAdapter, LibraryAccess, LibraryRecord, LibraryRegistry},
    sync::LibrarySyncStore,
};
use fsio::{FsIo, OpenDalConfig};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tokio::{runtime::Handle, sync::Mutex, time::Instant};
use uuid::Uuid;

use crate::{
    request_error::ExpectedRequestError,
    utils::{initialize_databases, initialize_library_replica, initialize_opendal_databases},
};

const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
static POOLS: OnceLock<StdMutex<HashMap<PathBuf, Weak<LibraryPool>>>> = OnceLock::new();

/// Connections and storage belonging to one library. Keep a handle alive while using them.
pub struct LibraryResources {
    pub library_id: Uuid,
    pub display_name: String,
    pub main_db: Arc<MainDbConnection>,
    pub library_sync: LibrarySyncStore,
    pub recommend_db: Arc<RecommendationDbConnection>,
    pub fsio: Arc<FsIo>,
    pub media_root: String,
    pub descriptor: LibraryRecord,
}

impl LibraryResources {
    /// Task requests carry the local folder selected by the user.
    pub fn fs_path(&self, name: &str) -> String {
        name.to_owned()
    }
}

struct Entry {
    resources: Arc<LibraryResources>,
    ui_users: usize,
    last_released: Arc<StdMutex<Instant>>,
}

#[derive(Clone, Copy)]
enum LibraryUse {
    Request,
    Background,
}

/// A lease which prevents idle closure until all its clones have been dropped.
#[derive(Clone)]
pub struct LibraryHandle {
    resources: Arc<LibraryResources>,
    pool: Arc<LibraryPool>,
    last_released: Arc<StdMutex<Instant>>,
    library_use: LibraryUse,
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
        match self.library_use {
            LibraryUse::Request => {
                *self.last_released.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
            }
            LibraryUse::Background => {}
        }
    }
}

/// One connection manager per node configuration, shared across library switches.
pub struct LibraryPool {
    config_dir: PathBuf,
    db_path: PathBuf,
    node_id: String,
    credentials: Option<Arc<CredentialStore>>,
    idle_timeout: Duration,
    entries: Mutex<HashMap<Uuid, Entry>>,
}

impl LibraryPool {
    async fn opendal_secret(
        &self,
        credential: CredentialRef,
        expected_username: &str,
    ) -> Result<String> {
        let unavailable = || ExpectedRequestError::Unavailable {
            what: "OpenDAL credentials",
        };
        let credentials = self.credentials.as_ref().ok_or_else(unavailable)?;
        let credential = credentials
            .get(credential)
            .await
            .map_err(|_| unavailable())?;
        if credential.username != expected_username {
            return Err(unavailable().into());
        }
        String::from_utf8(credential.secret.as_bytes().to_vec()).map_err(|_| unavailable().into())
    }

    /// Get this node's process-wide pool, with a five-minute idle timeout.
    pub fn new(config_dir: &Path, db_path: &Path, node_id: &str) -> Result<Arc<Self>> {
        Self::with_idle_timeout(config_dir, db_path, node_id, DEFAULT_IDLE_TIMEOUT)
    }

    pub fn new_with_credentials(
        config_dir: &Path,
        db_path: &Path,
        node_id: &str,
        credentials: Arc<CredentialStore>,
    ) -> Result<Arc<Self>> {
        Self::with_idle_timeout_inner(
            config_dir,
            db_path,
            node_id,
            DEFAULT_IDLE_TIMEOUT,
            Some(credentials),
        )
    }

    /// Configure the idle timeout when first creating a node's pool.
    pub fn with_idle_timeout(
        config_dir: &Path,
        db_path: &Path,
        node_id: &str,
        idle_timeout: Duration,
    ) -> Result<Arc<Self>> {
        Self::with_idle_timeout_inner(config_dir, db_path, node_id, idle_timeout, None)
    }

    pub fn with_idle_timeout_and_credentials(
        config_dir: &Path,
        db_path: &Path,
        node_id: &str,
        idle_timeout: Duration,
        credentials: Arc<CredentialStore>,
    ) -> Result<Arc<Self>> {
        Self::with_idle_timeout_inner(
            config_dir,
            db_path,
            node_id,
            idle_timeout,
            Some(credentials),
        )
    }

    fn with_idle_timeout_inner(
        config_dir: &Path,
        db_path: &Path,
        node_id: &str,
        idle_timeout: Duration,
        credentials: Option<Arc<CredentialStore>>,
    ) -> Result<Arc<Self>> {
        fs::create_dir_all(config_dir).context("Failed to create node configuration directory")?;
        let config_dir = dunce::canonicalize(config_dir)?;
        fs::create_dir_all(db_path).context("Failed to create database directory")?;
        let db_path = dunce::canonicalize(db_path)?;
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
            credentials,
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
        self.handle_with_use(entry, LibraryUse::Request)
    }

    fn handle_with_use(self: &Arc<Self>, entry: &Entry, library_use: LibraryUse) -> LibraryHandle {
        LibraryHandle {
            resources: entry.resources.clone(),
            pool: self.clone(),
            last_released: entry.last_released.clone(),
            library_use,
        }
    }

    /// Libraries enabled for background work, including any library registered by the UI.
    pub async fn served_library_ids(&self) -> Result<Vec<Uuid>> {
        let mut ids: Vec<_> = LibraryRegistry::load(&self.config_dir)?
            .catalogs()
            .filter(|library| library.background_sync())
            .filter_map(LibraryRecord::catalog_library_id)
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

    /// Acquire a served library for background work without counting the pass as user activity.
    pub async fn acquire_served_background(
        self: &Arc<Self>,
        library_id: Uuid,
    ) -> Result<Option<LibraryHandle>> {
        if !self.served_library_ids().await?.contains(&library_id) {
            return Ok(None);
        }
        self.acquire_background(library_id).await.map(Some)
    }

    /// Acquire an existing connection or open the registered library once.
    pub async fn acquire(self: &Arc<Self>, library_id: Uuid) -> Result<LibraryHandle> {
        self.acquire_with_use(library_id, LibraryUse::Request).await
    }

    /// Acquire a library for one background pass without refreshing the UI idle clock.
    pub async fn acquire_background(self: &Arc<Self>, library_id: Uuid) -> Result<LibraryHandle> {
        self.acquire_with_use(library_id, LibraryUse::Background)
            .await
    }

    async fn acquire_with_use(
        self: &Arc<Self>,
        library_id: Uuid,
        library_use: LibraryUse,
    ) -> Result<LibraryHandle> {
        // Keep initialization alive if a worker is cancelled while waiting for it. Dropping
        // the future while migrations are in flight would release the serialization lock
        // before SQLite has necessarily stopped executing the migration statements.
        let pool = self.clone();
        tokio::spawn(async move { pool.acquire_inner(library_id, library_use).await })
            .await
            .context("Library acquisition task failed")?
    }

    /// Acquire a library for a WebSocket request with request-boundary errors.
    pub async fn acquire_for_request(self: &Arc<Self>, library_id: Uuid) -> Result<LibraryHandle> {
        let registry = LibraryRegistry::load(&self.config_dir)
            .context("Could not read the library registry")?;
        if !registry.catalog(library_id).is_ok() {
            return Err(ExpectedError::NotFound { what: "library" }.into());
        }
        self.acquire(library_id).await.map_err(|error| {
            anyhow::Error::new(ExpectedRequestError::Unavailable {
                what: "addressed library",
            })
            .context(error)
        })
    }

    async fn acquire_inner(
        self: &Arc<Self>,
        library_id: Uuid,
        library_use: LibraryUse,
    ) -> Result<LibraryHandle> {
        let mut entries = self.entries.lock().await;
        let descriptor = LibraryRegistry::load(&self.config_dir)?
            .catalog(library_id)?
            .clone();
        ensure!(
            descriptor.access_level() != Some(LibraryAccess::NotShared),
            "Library access revoked"
        );
        if let Some(entry) = entries.get(&library_id) {
            return Ok(self.handle_with_use(entry, library_use));
        }
        let root = descriptor
            .root(&self.config_dir)
            .context("Catalog has no storage root")?;
        let path = &root
            .to_str()
            .context("Library path is not UTF-8")?
            .to_owned();
        let fsio = Arc::new(match descriptor.fs.first() {
            Some(FsAdapter::WebDav {
                endpoint,
                root,
                username,
                credential,
            }) => {
                let password = self.opendal_secret(*credential, username).await?;
                FsIo::new_opendal(
                    OpenDalConfig::WebDav {
                        endpoint: endpoint.clone(),
                        root: root.clone(),
                        username: username.clone(),
                        password,
                    },
                    Handle::current(),
                )?
            }
            Some(FsAdapter::S3 {
                endpoint,
                bucket,
                region,
                root,
                access_key_id,
                credential,
            }) => {
                let secret_access_key = self.opendal_secret(*credential, access_key_id).await?;
                FsIo::new_opendal(
                    OpenDalConfig::S3 {
                        endpoint: endpoint.clone(),
                        bucket: bucket.clone(),
                        region: region.clone(),
                        root: root.clone(),
                        access_key_id: access_key_id.clone(),
                        secret_access_key,
                    },
                    Handle::current(),
                )?
            }
            _ => {
                #[cfg(not(target_os = "android"))]
                {
                    FsIo::new()
                }
                #[cfg(target_os = "android")]
                {
                    FsIo::new(Path::new(".rune/.android-fs.db"), path)?
                }
            }
        });
        let mode = match descriptor
            .db_placement()
            .context("Catalog has no database placement")?
        {
            DbPlacement::Portable => StorageMode::Portable,
            DbPlacement::Redirected { storage_key } => StorageMode::Redirected(storage_key),
        };
        if !matches!(
            descriptor.fs.first(),
            Some(FsAdapter::WebDav { .. } | FsAdapter::S3 { .. })
        ) {
            ensure!(
                check_storage_mode(&fsio, path)? == mode,
                "Library database placement disagrees with its descriptor"
            );
        }
        let databases = if matches!(
            descriptor.fs.first(),
            Some(FsAdapter::WebDav { .. } | FsAdapter::S3 { .. })
        ) {
            let DbPlacement::Redirected { storage_key } = descriptor
                .db_placement()
                .context("Catalog has no database placement")?
            else {
                anyhow::bail!("OpenDAL library has no redirected storage key");
            };
            initialize_opendal_databases(&self.db_path, storage_key, &self.node_id).await?
        } else {
            let db_path = self
                .db_path
                .to_str()
                .context("Database path is not UTF-8")?;
            initialize_databases(&fsio, path, Some(db_path), &self.node_id).await?
        };
        let library_sync = initialize_library_replica(
            &databases.main_db,
            &self.node_id,
            library_id,
            &self.config_dir,
        )
        .await?;
        let entry = Entry {
            resources: Arc::new(LibraryResources {
                library_id,
                display_name: descriptor
                    .display_name()
                    .context("Catalog has no app name")?,
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
        let handle = self.handle_with_use(&entry, library_use);
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
        let registered = LibraryRegistry::load(&self.config_dir)
            .ok()
            .and_then(|registry| registry.catalog(library_id).ok().cloned());
        let Some(descriptor) = registered else {
            return Ok(());
        };
        let Some(root) = descriptor.root(&self.config_dir) else {
            return Ok(());
        };
        let mut entries = self.entries.lock().await;
        if entries
            .get(&library_id)
            .is_some_and(|entry| Path::new(&entry.resources.media_root) != root)
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
        let id = resources.library_id;
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

    #[cfg(test)]
    pub(crate) async fn ui_user_count(&self, library_id: Uuid) -> usize {
        self.entries
            .lock()
            .await
            .get(&library_id)
            .map_or(0, |entry| entry.ui_users)
    }

    /// Drop the pool's background-only connection after a record is removed.
    pub async fn release_background(&self, library_id: Uuid) {
        let mut entries = self.entries.lock().await;
        if entries
            .get(&library_id)
            .is_some_and(|entry| entry.ui_users == 0)
        {
            entries.remove(&library_id);
        }
    }

    pub(crate) async fn close_idle(&self) -> Result<()> {
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
                    .library_sync
                    .clean_close()
                    .await
                    .context("Failed to persist writer counter before closing library")?;
                // Leave nothing only in -wal when the library is closed. A failure here is
                // logged and must not keep the other idle libraries open.
                if let Err(error) = checkpoint_wal(&entry.resources.main_db).await {
                    log::warn!("Could not checkpoint library {id} before closing: {error:#}");
                }
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

/// Fold a WAL database's log back into the main file. No-op in other journal modes.
async fn checkpoint_wal(db: &sea_orm::DatabaseConnection) -> Result<()> {
    let journal_mode = db
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA journal_mode".to_string(),
        ))
        .await?
        .context("SQLite did not return its journal mode")?
        .try_get::<String>("", "journal_mode")?;
    if journal_mode.eq_ignore_ascii_case("wal") {
        db.execute_unprepared("PRAGMA wal_checkpoint(TRUNCATE)")
            .await
            .context("Failed to checkpoint library WAL")?;
    }
    Ok(())
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    use database::{
        connection::create_redirect,
        library_registry::{create_replica, resolve_library},
    };
    use sea_orm::{
        TransactionTrait,
        sqlx::{
            ConnectOptions, Connection,
            sqlite::{SqliteConnectOptions, SqliteJournalMode},
        },
    };
    use tempfile::TempDir;
    use tokio::time::{sleep, timeout};

    struct Fixture {
        dir: TempDir,
        pool: Arc<LibraryPool>,
        descriptor: LibraryRecord,
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
                self.descriptor.catalog_library_id().unwrap(),
                self.dir.path(),
            )
            .await?;
            Ok(LibraryResources {
                library_id: self
                    .descriptor
                    .catalog_library_id()
                    .context("catalog fixture")?,
                display_name: self
                    .descriptor
                    .display_name()
                    .context("catalog fixture app name")?,
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
                    .contains_key(&self.descriptor.catalog_library_id().unwrap())
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
        let id = f.descriptor.catalog_library_id().unwrap();
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
        let id = f.descriptor.catalog_library_id().unwrap();
        let background = f.pool.acquire(id).await?;
        drop(background);
        sleep(f.pool.idle_timeout + Duration::from_millis(10)).await;
        f.pool.close_idle().await?;
        // A user moving the folder in the file manager cannot do so while Windows holds its database open.
        let moved = f.dir.path().join("moved").to_string_lossy().into_owned();
        fs::rename(&f.root, &moved)?;
        let mode = check_storage_mode(&f.fsio, &moved)?;
        let mut registry = LibraryRegistry::load(f.dir.path())?;
        let reopened = resolve_library(&f.fsio, &mut registry, &moved, "Test", mode).await?;
        assert_eq!(reopened.catalog_library_id().unwrap(), id);
        let ui = f.pool.acquire_ui(id).await?;
        assert_eq!(ui.media_root, moved);
        assert_eq!(f.pool.acquire(id).await?.media_root, moved);
        Ok(())
    }

    #[tokio::test]
    async fn replica_opens_from_a_moved_data_dir() -> Result<()> {
        let old = TempDir::new()?;
        let node_id = Uuid::new_v4().to_string();
        let id = Uuid::new_v4();
        let mut registry = LibraryRegistry::load(old.path())?;
        create_replica(old.path(), &mut registry, id, "Replica", "owner").await?;
        let pool = LibraryPool::with_idle_timeout(
            old.path(),
            old.path(),
            &node_id,
            Duration::from_millis(50),
        )?;
        let first = pool.acquire_ui(id).await?;
        assert_eq!(
            Path::new(&first.media_root),
            old.path().join("replicas").join(id.to_string())
        );
        pool.unregister_ui(id).await;
        drop(first);
        // A reinstall cannot relocate app data while Windows still has the library database open.
        sleep(pool.idle_timeout + Duration::from_millis(10)).await;
        pool.close_idle().await?;
        // A reinstall gives the app a new data directory holding the same files.
        let new = TempDir::new()?;
        fs::copy(
            old.path().join("libraries.json"),
            new.path().join("libraries.json"),
        )?;
        fs::rename(old.path().join("replicas"), new.path().join("replicas"))?;
        let moved = LibraryPool::new(new.path(), new.path(), &node_id)?;
        let reopened = moved.acquire_ui(id).await?;
        assert_eq!(
            Path::new(&reopened.media_root),
            new.path().join("replicas").join(id.to_string())
        );
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
        let acquired = f
            .pool
            .acquire(f.descriptor.catalog_library_id().unwrap())
            .await?;
        assert!(Arc::ptr_eq(
            &connection.upgrade().unwrap(),
            &acquired.main_db
        ));
        f.pool
            .unregister_ui(f.descriptor.catalog_library_id().unwrap())
            .await;
        sleep(f.pool.idle_timeout * 3).await;
        assert!(
            f.pool
                .entries
                .lock()
                .await
                .contains_key(&f.descriptor.catalog_library_id().unwrap())
        );
        drop(acquired);
        f.wait_closed().await?;
        assert!(connection.upgrade().is_none());
        Ok(())
    }

    #[tokio::test]
    async fn idle_close_waits_for_last_handle_then_reopens() -> Result<()> {
        let f = Fixture::new(false).await?;
        let first = f
            .pool
            .acquire(f.descriptor.catalog_library_id().unwrap())
            .await?;
        let last = first.clone();
        let connection = Arc::downgrade(&first.main_db);
        drop(first);
        sleep(f.pool.idle_timeout * 3).await;
        assert!(connection.upgrade().is_some());
        let held = f
            .pool
            .acquire(f.descriptor.catalog_library_id().unwrap())
            .await?;
        assert!(Arc::ptr_eq(&last.main_db, &held.main_db));
        drop(held);
        drop(last);
        f.wait_closed().await?;
        assert!(connection.upgrade().is_none());
        let reopened = f
            .pool
            .acquire(f.descriptor.catalog_library_id().unwrap())
            .await?;
        assert!(!Weak::ptr_eq(
            &connection,
            &Arc::downgrade(&reopened.main_db)
        ));
        assert_eq!(
            reopened.library_sync.record_store().namespace(),
            f.descriptor.catalog_library_id().unwrap()
        );
        Ok(())
    }

    #[tokio::test]
    async fn background_only_activity_does_not_refresh_idle_timeout() -> Result<()> {
        let f = Fixture::new(false).await?;
        let id = f.descriptor.catalog_library_id().unwrap();
        LibraryRegistry::set_background_sync(f.dir.path(), id, true).await?;

        let first = f
            .pool
            .acquire_served_background(id)
            .await?
            .context("served library")?;
        first
            .main_db
            .execute_unprepared("CREATE TABLE idle_release_probe (value INTEGER NOT NULL)")
            .await?;
        first
            .main_db
            .execute_unprepared("INSERT INTO idle_release_probe (value) VALUES (42)")
            .await?;
        drop(first);

        for _ in 0..2 {
            // Model background passes arriving 10 ms apart; this is cadence, not polling.
            sleep(Duration::from_millis(10)).await;
            let pass = f
                .pool
                .acquire_served_background(id)
                .await?
                .context("served library")?;
            drop(pass);
        }

        // This 40 ms window takes the original acquisition past the 50 ms timeout,
        // while the old behavior would still be within 50 ms of the final pass.
        sleep(Duration::from_millis(40)).await;
        f.pool.close_idle().await?;
        assert!(
            !f.pool.entries.lock().await.contains_key(&id),
            "background-only acquisitions kept the library open"
        );

        let reopened = f.pool.acquire(id).await?;
        let result = reopened
            .main_db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT value FROM idle_release_probe".to_owned(),
            ))
            .await?;
        assert_eq!(
            result
                .context("persisted background data")?
                .try_get::<i64>("", "value")?,
            42
        );
        Ok(())
    }

    #[tokio::test]
    async fn background_pass_holds_library_open_when_idle_timeout_fires() -> Result<()> {
        let f = Fixture::new(false).await?;
        let id = f.descriptor.catalog_library_id().unwrap();
        LibraryRegistry::set_background_sync(f.dir.path(), id, true).await?;
        let pass = f
            .pool
            .acquire_served_background(id)
            .await?
            .context("served library")?;
        {
            let entries = f.pool.entries.lock().await;
            let entry = entries.get(&id).context("open library")?;
            *entry
                .last_released
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Instant::now() - f.pool.idle_timeout;
        }

        f.pool.close_idle().await?;
        assert!(f.pool.entries.lock().await.contains_key(&id));
        drop(pass);
        f.pool.close_idle().await?;
        assert!(!f.pool.entries.lock().await.contains_key(&id));
        Ok(())
    }

    #[tokio::test]
    async fn acquire_respects_redirected_placement() -> Result<()> {
        let f = Fixture::new(true).await?;
        let handle = f
            .pool
            .acquire(f.descriptor.catalog_library_id().unwrap())
            .await?;
        let DbPlacement::Redirected { storage_key } = handle.descriptor.db_placement().unwrap()
        else {
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
        let reopened = f
            .pool
            .acquire(f.descriptor.catalog_library_id().unwrap())
            .await?;
        assert_eq!(reopened.descriptor, f.descriptor);
        Ok(())
    }

    #[tokio::test]
    async fn idle_close_truncates_redirected_database_wal() -> Result<()> {
        let f = Fixture::new(true).await?;
        let handle = f
            .pool
            .acquire(f.descriptor.catalog_library_id().unwrap())
            .await?;
        let DbPlacement::Redirected { storage_key } = handle.descriptor.db_placement().unwrap()
        else {
            panic!("redirected fixture");
        };
        handle
            .main_db
            .execute_unprepared("CREATE TABLE wal_close_probe (value INTEGER NOT NULL)")
            .await?;
        handle
            .main_db
            .execute_unprepared("INSERT INTO wal_close_probe (value) VALUES (1)")
            .await?;
        let wal_path = f.dir.path().join(storage_key.to_string()).join(".0.db-wal");
        assert!(wal_path.metadata()?.len() > 0);
        let database_path = f.dir.path().join(storage_key.to_string()).join(".0.db");
        let observer = SqliteConnectOptions::new()
            .filename(database_path)
            .journal_mode(SqliteJournalMode::Wal)
            .connect()
            .await?;
        drop(handle);
        {
            let entries = f.pool.entries.lock().await;
            let entry = entries
                .get(&f.descriptor.catalog_library_id().unwrap())
                .context("open library")?;
            *entry
                .last_released
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Instant::now() - f.pool.idle_timeout;
        }
        f.pool.close_idle().await?;
        assert!(wal_path.exists());
        assert_eq!(wal_path.metadata()?.len(), 0);
        observer.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn closing_waits_for_in_flight_transaction_before_reopening() -> Result<()> {
        let f = Fixture::new(false).await?;
        let handle = f
            .pool
            .acquire(f.descriptor.catalog_library_id().unwrap())
            .await?;
        let transaction = handle.main_db.begin().await?;
        drop(handle);
        {
            let entries = f.pool.entries.lock().await;
            let entry = entries
                .get(&f.descriptor.catalog_library_id().unwrap())
                .unwrap();
            *entry.last_released.lock().unwrap() = Instant::now() - f.pool.idle_timeout;
        }
        let pool = f.pool.clone();
        let closing = tokio::spawn(async move { pool.close_idle().await });
        sleep(Duration::from_millis(30)).await;
        assert!(!closing.is_finished());
        let pool = f.pool.clone();
        let id = f.descriptor.catalog_library_id().unwrap();
        let reopening = tokio::spawn(async move { pool.acquire(id).await });
        sleep(Duration::from_millis(30)).await;
        assert!(!reopening.is_finished());
        transaction.commit().await?;
        timeout(Duration::from_secs(3), closing).await???;
        let reopened = timeout(Duration::from_secs(3), reopening).await???;
        assert_eq!(reopened.library_id, id);
        Ok(())
    }
}
