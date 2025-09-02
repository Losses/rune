use std::{
    fs,
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

pub fn check_library_state(lib_path: &str) -> Result<LibraryState> {
    let rune_dir: PathBuf = [lib_path, ".rune"].iter().collect();

    if !rune_dir.exists() {
        return Ok(LibraryState::Uninitialized);
    }

    let mode = detect_storage_mode(&rune_dir)?;
    Ok(LibraryState::Initialized(mode))
}

pub fn detect_storage_mode(rune_dir: &Path) -> Result<StorageMode> {
    let redirect_file = rune_dir.join(".redirect");

    if redirect_file.exists() {
        let content = fs::read_to_string(redirect_file)?;
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

pub fn check_storage_mode(lib_path: &str) -> Result<StorageMode> {
    let rune_dir: PathBuf = [lib_path, ".rune"].iter().collect();
    let redirect_file = rune_dir.join(".redirect");

    if !rune_dir.exists() {
        return Ok(StorageMode::Portable);
    }

    if redirect_file.exists() {
        let content = fs::read_to_string(redirect_file)?;
        let uuid = Uuid::parse_str(content.trim())?;
        Ok(StorageMode::Redirected(uuid))
    } else {
        Ok(StorageMode::Portable)
    }
}

pub fn create_redirect(lib_path: &str) -> Result<()> {
    let rune_dir: PathBuf = [lib_path, ".rune"].iter().collect();
    if !rune_dir.exists() {
        fs::create_dir_all(&rune_dir)?;
        #[cfg(windows)]
        set_hidden_attribute(&rune_dir)?;
    }

    let redirect_file = rune_dir.join(".redirect");
    fs::write(redirect_file, Uuid::new_v4().to_string())?;
    Ok(())
}

pub fn get_storage_info(lib_path: &str, db_path: Option<&str>) -> Result<StorageInfo> {
    let rune_dir: PathBuf = [lib_path, ".rune"].iter().collect();
    let state = check_library_state(lib_path)?;

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
    let storage_info = get_storage_info(lib_path, db_path)?;
    let db_path = storage_info.get_main_db_path();

    if !storage_info.db_dir.exists() {
        fsio.ensure_directory(&storage_info.db_dir).await?;
    }

    let db_path = fsio.ensure_file(&db_path).await?;
    let db_url = format!(
        "sqlite:{}?mode=rwc",
        fsio.canonicalize_path(&db_path.path)?.to_string_lossy()
    );

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
    let storage_info = get_storage_info(lib_path, db_path)?;
    let analysis_path = storage_info.get_recommendation_db_path();

    if !storage_info.db_dir.exists() {
        fsio.ensure_directory(&storage_info.db_dir).await?;
    }

    let db_path = fsio.ensure_file(&analysis_path).await?;
    let path_str = db_path.path.to_string_lossy();
    let path_str = path_str.as_ref();

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
