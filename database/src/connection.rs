use arroy::distances::Euclidean;
use arroy::Database as ArroyDatabase;
use heed::{Env, EnvOpenOptions};
use log::LevelFilter;
use sea_orm::DbErr;
use sea_orm::{ConnectOptions, Database};
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::result::Result;

use migration::Migrator;
use migration::MigratorTrait;

#[derive(Debug)]
pub enum ConnectMainDbError {
    InvalidPath(OsString),
    IoError(std::io::Error),
    DbError(DbErr),
}

impl fmt::Display for ConnectMainDbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectMainDbError::InvalidPath(path) => {
                write!(f, "Invalid path: {:?}", path)
            }
            ConnectMainDbError::IoError(e) => {
                write!(f, "IO error: {}", e)
            }
            ConnectMainDbError::DbError(e) => {
                write!(f, "Database error: {}", e)
            }
        }
    }
}

impl Error for ConnectMainDbError {}

impl From<std::io::Error> for ConnectMainDbError {
    fn from(error: std::io::Error) -> Self {
        ConnectMainDbError::IoError(error)
    }
}

impl From<DbErr> for ConnectMainDbError {
    fn from(error: DbErr) -> Self {
        ConnectMainDbError::DbError(error)
    }
}

pub async fn connect_main_db(
    lib_path: &str,
) -> Result<sea_orm::DatabaseConnection, ConnectMainDbError> {
    let path: PathBuf = [lib_path, ".rune", ".0.db"].iter().collect();

    let dir_path = path.parent().ok_or_else(|| {
        ConnectMainDbError::InvalidPath("Invalid path: parent directory not found".into())
    })?;

    if !dir_path.exists() {
        std::fs::create_dir_all(dir_path)?;
    }

    let path_str = path.into_os_string().into_string().unwrap();
    let db_url = format!("sqlite:{}?mode=rwc", path_str);
    let mut opt = ConnectOptions::new(db_url);
    opt.sqlx_logging(true)
        .sqlx_logging_level(LevelFilter::Debug);

    let db = Database::connect(opt).await?;

    initialize_db(&db).await?;

    Ok(db)
}

pub async fn initialize_db(conn: &sea_orm::DatabaseConnection) -> Result<(), DbErr> {
    Migrator::up(conn, None).await
}

#[derive(Debug)]
enum ConnectRecommendationDbError {
    InvalidPath(OsString),
    EnvOpenError(Box<dyn Error>),
    WriteTxnError(Box<dyn Error>),
    CreateDbError(Box<dyn Error>),
    CommitError(Box<dyn Error>),
}

impl fmt::Display for ConnectRecommendationDbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectRecommendationDbError::InvalidPath(path) => {
                write!(f, "Invalid path: {:?}", path)
            }
            ConnectRecommendationDbError::EnvOpenError(e) => {
                write!(f, "Failed to open environment: {}", e)
            }
            ConnectRecommendationDbError::WriteTxnError(e) => {
                write!(f, "Failed to create write transaction: {}", e)
            }
            ConnectRecommendationDbError::CreateDbError(e) => {
                write!(f, "Failed to create database: {}", e)
            }
            ConnectRecommendationDbError::CommitError(e) => {
                write!(f, "Failed to commit transaction: {}", e)
            }
        }
    }
}

impl Error for ConnectRecommendationDbError {}

const DB_SIZE: usize = 2 * 1024 * 1024 * 1024;

/// Initialize the recommendation database.
///
/// # Arguments
/// * `db_path` - The path to the database directory.
///
/// # Returns
/// * `Result<(Env, ArroyDatabase<Euclidean>), Box<dyn std::error::Error>>` - The database environment and the Arroy database.
pub fn connect_recommendation_db(
    lib_path: &str,
) -> Result<(Env, ArroyDatabase<Euclidean>), Box<dyn Error>> {
    let path: PathBuf = [lib_path, ".rune", ".analysis"].iter().collect();

    if !path.exists() {
        std::fs::create_dir_all(&path)?;
    }

    let path_str = path
        .into_os_string()
        .into_string()
        .map_err(|path| ConnectRecommendationDbError::InvalidPath(path))?;

    let env = unsafe {
        EnvOpenOptions::new()
            .map_size(DB_SIZE)
            .open(path_str)
            .map_err(|e| ConnectRecommendationDbError::EnvOpenError(Box::new(e)))?
    };

    let mut wtxn = env
        .write_txn()
        .map_err(|e| ConnectRecommendationDbError::WriteTxnError(Box::new(e)))?;

    let db: ArroyDatabase<Euclidean> = env
        .create_database(&mut wtxn, None)
        .map_err(|e| ConnectRecommendationDbError::CreateDbError(Box::new(e)))?;

    wtxn.commit()
        .map_err(|e| ConnectRecommendationDbError::CommitError(Box::new(e)))?;

    Ok((env, db))
}
