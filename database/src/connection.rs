use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{Context, Result};
use arroy::{
    Database as ArroyDatabase,
    distances::Euclidean,
    internals::{KeyCodec, NodeCodec},
};
use heed::{Env, EnvFlags, EnvOpenOptions};
use log::info;
use sea_orm::{
    Database, SqlxSqliteConnector,
    sqlx::{SqlitePool, sqlite::SqliteConnectOptions},
};
use tempfile::tempdir;
use uuid::Uuid;
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, SetFileAttributesW};
#[cfg(windows)]
use windows::core::PWSTR;

use ::fsio::FsIo;
use ::migration::{Migrator, MigratorTrait};

use crate::actions::mixes::initialize_mix_queries;

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

    pub fn get_recommendation_db_path(&self) -> PathBuf {
        self.db_dir.join(".analysis.db")
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

    let db_url = if db_uses_local_fs(&storage_info.db_dir) {
        tokio::fs::create_dir_all(&storage_info.db_dir).await?;
        if !tokio::fs::try_exists(&db_file).await? {
            tokio::fs::write(&db_file, b"").await?;
        }
        format!("sqlite:{}?mode=rwc", db_file.to_string_lossy())
    } else {
        if !storage_info.db_dir.exists() {
            fsio.ensure_directory(&storage_info.db_dir).await?;
        }
        let db_node = fsio.ensure_file(&db_file).await?;
        format!(
            "sqlite:{}?mode=rwc",
            fsio.canonicalize_path(&db_node.path)?.to_string_lossy()
        )
    };

    let connection_options = SqliteConnectOptions::from_str(&db_url)?;

    let pool = SqlitePool::connect_with(connection_options).await?;

    info!("Initializing main database: {}", { db_url });

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

const DB_SIZE: usize = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct RecommendationDbConnection {
    pub env: Env,
    pub db: ArroyDatabase<Euclidean>,
}

pub async fn connect_recommendation_db(
    fsio: &FsIo,
    lib_path: &str,
    db_path: Option<&str>,
) -> Result<RecommendationDbConnection> {
    let storage_info = get_storage_info(fsio, lib_path, db_path)?;
    let analysis_path = storage_info.get_recommendation_db_path();

    let path_string = if db_uses_local_fs(&storage_info.db_dir) {
        tokio::fs::create_dir_all(&storage_info.db_dir).await?;
        if !tokio::fs::try_exists(&analysis_path).await? {
            tokio::fs::write(&analysis_path, b"").await?;
        }
        analysis_path.to_string_lossy().into_owned()
    } else {
        if !storage_info.db_dir.exists() {
            fsio.ensure_directory(&storage_info.db_dir).await?;
        }
        let db_node = fsio.ensure_file(&analysis_path).await?;
        db_node.path.to_string_lossy().into_owned()
    };
    let path_str = path_string.as_str();

    info!("Initializing recommendation database: {path_str}");

    let env = unsafe {
        EnvOpenOptions::new()
            .map_size(DB_SIZE)
            .flags(EnvFlags::NO_LOCK)
            .flags(EnvFlags::NO_SUB_DIR)
            .open(path_str)
    }
    .with_context(|| "Failed to open the recommendation database")?;

    let mut wtxn = env.write_txn()?;
    let db: ArroyDatabase<Euclidean> = env
        .database_options()
        .types::<KeyCodec, NodeCodec<Euclidean>>()
        .create(&mut wtxn)?;
    wtxn.commit()?;

    Ok(RecommendationDbConnection { env, db })
}

pub fn connect_fake_recommendation_db() -> Result<RecommendationDbConnection> {
    info!("Initializing fake recommendation database");

    let dir = tempdir()?;
    let env = unsafe {
        EnvOpenOptions::new()
            .map_size(DB_SIZE)
            .flags(EnvFlags::NO_LOCK)
            .open(dir.path())?
    };

    let mut wtxn = env.write_txn()?;
    let db: ArroyDatabase<Euclidean> = env
        .database_options()
        .types::<KeyCodec, NodeCodec<Euclidean>>()
        .create(&mut wtxn)?;
    wtxn.commit()?;

    Ok(RecommendationDbConnection { env, db })
}
