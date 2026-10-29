use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use anyhow::{Context, Result};
use log::{info, warn};
use sea_orm::{
    Database, SqlxSqliteConnector,
    sqlx::{SqlitePool, sqlite::SqliteConnectOptions},
};
use uuid::Uuid;
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, SetFileAttributesW};
#[cfg(windows)]
use windows::core::PWSTR;

use ::fsio::FsIo;
use ::migration::{Migrator, MigratorTrait};

use crate::actions::mixes::initialize_mix_queries;
use crate::actions::recommendation::load_recommendation_vectors;

#[derive(Debug, Clone, PartialEq)]
pub enum StorageMode {
    Portable,
    Redirected(Uuid),
}

pub struct StorageInfo {
    pub state: LibraryState,
    pub rune_dir: PathBuf,
    pub db_dir: PathBuf,
}

impl StorageInfo {
    pub fn get_main_db_path(&self) -> PathBuf {
        self.db_dir.join(".0.db")
    }

    /// Where older versions kept the LMDB recommendation index: a directory
    /// (`.analysis`, before August 2025) or a file (`.analysis.db`). Nothing
    /// reads them any more, they are only deleted.
    pub fn get_legacy_recommendation_db_paths(&self) -> [PathBuf; 2] {
        [
            self.db_dir.join(".analysis"),
            self.db_dir.join(".analysis.db"),
        ]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LibraryState {
    Uninitialized,
    Initialized(StorageMode),
}

impl LibraryState {
    pub fn storage_mode(&self) -> Option<&StorageMode> {
        match self {
            LibraryState::Uninitialized => None,
            LibraryState::Initialized(mode) => Some(mode),
        }
    }
}

/// On Android `lib_path` is a content:// tree URI and FsIo is already rooted
/// at that tree, so the rune dir is addressed relative to the tree root.
/// Everywhere else it is a plain filesystem path joined with `.rune`.
fn rune_dir_path(lib_path: &str) -> PathBuf {
    #[cfg(target_os = "android")]
    {
        let _ = lib_path;
        PathBuf::from(".rune")
    }
    #[cfg(not(target_os = "android"))]
    {
        [lib_path, ".rune"].iter().collect()
    }
}

/// Redirected databases live in the app's local storage (an absolute path),
/// which FsIo on Android cannot address (it is rooted at the SAF tree).
fn db_uses_local_fs(db_dir: &Path) -> bool {
    cfg!(target_os = "android") && db_dir.is_absolute()
}

pub fn check_library_state(fsio: &FsIo, lib_path: &str) -> Result<LibraryState> {
    let rune_dir = rune_dir_path(lib_path);

    // The marker of an initialized library is its main DB or a redirect file —
    // NOT the bare .rune directory, which FsIo on Android creates eagerly for
    // its fs cache (.rune/.android-fs.db) before any initialization happens.
    let main_db = rune_dir.join(".0.db");
    let redirect_file = rune_dir.join(".redirect");
    if !fsio.exists(&main_db)? && !fsio.exists(&redirect_file)? {
        return Ok(LibraryState::Uninitialized);
    }

    let mode = detect_storage_mode(fsio, &rune_dir)?;
    Ok(LibraryState::Initialized(mode))
}

pub fn detect_storage_mode(fsio: &FsIo, rune_dir: &Path) -> Result<StorageMode> {
    let redirect_file = rune_dir.join(".redirect");

    if fsio.exists(&redirect_file)? {
        let content = fsio.read_to_string(&redirect_file)?;
        let uuid = Uuid::parse_str(content.trim()).context("Invalid UUID in .redirect file")?;
        Ok(StorageMode::Redirected(uuid))
    } else {
        Ok(StorageMode::Portable)
    }
}

#[cfg(windows)]
fn set_hidden_attribute(path: &std::path::Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();

    unsafe {
        SetFileAttributesW(PWSTR(wide.as_ptr() as *mut u16), FILE_ATTRIBUTE_HIDDEN)?;
    }
    Ok(())
}

pub fn check_storage_mode(fsio: &FsIo, lib_path: &str) -> Result<StorageMode> {
    let rune_dir = rune_dir_path(lib_path);
    let redirect_file = rune_dir.join(".redirect");

    if !fsio.exists(&rune_dir)? {
        return Ok(StorageMode::Portable);
    }

    if fsio.exists(&redirect_file)? {
        let content = fsio.read_to_string(&redirect_file)?;
        let uuid = Uuid::parse_str(content.trim())?;
        Ok(StorageMode::Redirected(uuid))
    } else {
        Ok(StorageMode::Portable)
    }
}

pub async fn create_redirect(fsio: &FsIo, lib_path: &str) -> Result<()> {
    let rune_dir = rune_dir_path(lib_path);
    if !fsio.exists(&rune_dir)? {
        fsio.ensure_directory(&rune_dir).await?;
        #[cfg(windows)]
        set_hidden_attribute(&rune_dir)?;
    }

    let redirect_file = rune_dir.join(".redirect");
    fsio.write_string(&redirect_file, &Uuid::new_v4().to_string())
        .await?;
    Ok(())
}

pub fn get_storage_info(fsio: &FsIo, lib_path: &str, db_path: Option<&str>) -> Result<StorageInfo> {
    let rune_dir = rune_dir_path(lib_path);
    let state = check_library_state(fsio, lib_path)?;

    let db_dir = match &state {
        LibraryState::Uninitialized => rune_dir.clone(),
        LibraryState::Initialized(mode) => match mode {
            StorageMode::Portable => rune_dir.clone(),
            StorageMode::Redirected(uuid) => {
                let db_path = db_path.context("db_path is required for redirected storage")?;
                PathBuf::from(db_path).join(uuid.to_string())
            }
        },
    };

    Ok(StorageInfo {
        state,
        rune_dir,
        db_dir,
    })
}

pub type MainDbConnection = sea_orm::DatabaseConnection;

pub async fn connect_main_db(
    fsio: &FsIo,
    lib_path: &str,
    db_path: Option<&str>,
    node_id: &str,
) -> Result<MainDbConnection> {
    let storage_info = get_storage_info(fsio, lib_path, db_path)?;
    let db_file = storage_info.get_main_db_path();

    let db_path = if db_uses_local_fs(&storage_info.db_dir) {
        tokio::fs::create_dir_all(&storage_info.db_dir).await?;
        if !tokio::fs::try_exists(&db_file).await? {
            tokio::fs::write(&db_file, b"").await?;
        }
        db_file
    } else {
        if !storage_info.db_dir.exists() {
            fsio.ensure_directory(&storage_info.db_dir).await?;
        }
        let db_node = fsio.ensure_file(&db_file).await?;
        fsio.canonicalize_path(&db_node.path)?
    };

    // The path goes in as it is, not through a URL: a Windows path such as `\\?\UNC\...`
    // would otherwise be cut at its `?`.
    let connection_options = SqliteConnectOptions::new()
        .filename(&db_path)
        .create_if_missing(true);

    let pool = SqlitePool::connect_with(connection_options).await?;

    info!("Initializing main database: {}", db_path.display());

    let db = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);

    initialize_db(&db, node_id).await?;

    Ok(db)
}

pub async fn initialize_db(conn: &sea_orm::DatabaseConnection, node_id: &str) -> Result<()> {
    // Initialize node_id for migrations.
    // We ignore the result because it might have been initialized already, which is fine.
    let _ = migration::initialize_node_id(node_id.to_string());

    Migrator::up(conn, None).await?;
    initialize_mix_queries(conn, node_id).await?;
    Ok(())
}

pub async fn connect_fake_main_db() -> Result<MainDbConnection> {
    info!("Initializing fake main database.");

    let db = Database::connect("sqlite::memory:").await?;

    Ok(db)
}

/// The feature vectors of the analysed tracks with their file ids, sorted by
/// file id.
pub type RecommendationVectors = Arc<Vec<(u32, [f32; 61])>>;

/// The vectors of all analysed tracks, kept in memory for exact
/// nearest-neighbour search.
///
/// The data lives in the `media_analysis` table of the main database and is
/// read from there: when the library is opened (see
/// [`connect_recommendation_db`]) and again after every analysis run (see
/// `sync_recommendation`). Nothing is stored on disk.
///
/// The vectors are replaced as a whole behind an `Arc`, so a query only holds
/// the lock for the moment it takes to clone the pointer and never while it
/// computes (or awaits).
#[derive(Debug, Default)]
pub struct RecommendationDbConnection {
    vectors: RwLock<RecommendationVectors>,
}

impl RecommendationDbConnection {
    /// An empty store, which finds no recommendations until vectors are
    /// loaded into it.
    pub fn new() -> Self {
        Self::default()
    }

    /// The current vectors, sorted by file id, one per file.
    pub fn vectors(&self) -> RecommendationVectors {
        match self.vectors.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Replace all vectors. The input is sorted by file id; on duplicate ids
    /// the last one wins.
    pub fn replace(&self, vectors: impl IntoIterator<Item = (u32, [f32; 61])>) {
        let map: BTreeMap<u32, [f32; 61]> = vectors.into_iter().collect();
        let vectors = Arc::new(map.into_iter().collect::<Vec<_>>());
        match self.vectors.write() {
            Ok(mut guard) => *guard = vectors,
            Err(poisoned) => *poisoned.into_inner() = vectors,
        }
    }
}

/// Prepare the recommendation store of a library.
///
/// Older versions kept the vectors in a LMDB index (the directory `.analysis`
/// or the file `.analysis.db`). It was only a copy of `media_analysis`, so it
/// is deleted here. The store is then filled from `main_db`. Neither a failure
/// to delete the legacy data nor a failure to read the vectors prevents the
/// library from opening: both are logged, and the store stays empty in the
/// second case until the next `sync_recommendation`.
pub async fn connect_recommendation_db(
    fsio: &FsIo,
    lib_path: &str,
    db_path: Option<&str>,
    main_db: &MainDbConnection,
) -> Result<RecommendationDbConnection> {
    let storage_info = get_storage_info(fsio, lib_path, db_path)?;

    for legacy_path in storage_info.get_legacy_recommendation_db_paths() {
        if let Err(e) = remove_legacy_recommendation_db(fsio, &storage_info, &legacy_path).await {
            warn!(
                "Failed to delete the legacy recommendation database {}: {e:#}",
                legacy_path.display()
            );
        }
    }

    info!("Loading recommendation vectors");
    let recommend_db = RecommendationDbConnection::new();
    if let Err(e) = load_recommendation_vectors(main_db, &recommend_db).await {
        warn!("Failed to load the recommendation vectors, recommendations are empty: {e:#}");
    }

    Ok(recommend_db)
}

/// Delete a legacy recommendation file or directory (with its content).
/// Does nothing if it does not exist.
async fn remove_legacy_recommendation_db(
    fsio: &FsIo,
    storage_info: &StorageInfo,
    legacy_path: &Path,
) -> Result<()> {
    if db_uses_local_fs(&storage_info.db_dir) {
        let metadata = match tokio::fs::symlink_metadata(legacy_path).await {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        if metadata.is_dir() {
            tokio::fs::remove_dir_all(legacy_path).await?;
        } else {
            tokio::fs::remove_file(legacy_path).await?;
        }
    } else {
        if !fsio.exists(legacy_path)? {
            return Ok(());
        }
        if fsio.is_dir(legacy_path).await? {
            fsio.remove_dir_all(legacy_path).await?;
        } else {
            fsio.remove_file(legacy_path).await?;
        }
    }
    info!(
        "Deleted legacy recommendation database {}",
        legacy_path.display()
    );
    Ok(())
}

pub fn connect_fake_recommendation_db() -> Result<RecommendationDbConnection> {
    info!("Initializing fake recommendation database");

    Ok(RecommendationDbConnection::new())
}
