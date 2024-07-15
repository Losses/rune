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

pub async fn connect_main_db(lib_path: &str) -> sea_orm::DatabaseConnection {
    let path: PathBuf = [lib_path, ".0.db"].iter().collect();
    let path_str = path.into_os_string().into_string().unwrap();
    let db_url = format!("sqlite:{}?mode=rwc", path_str);
    println!("{}", db_url);
    let mut opt = ConnectOptions::new(db_url);
    opt.sqlx_logging(true)
        .sqlx_logging_level(LevelFilter::Debug);

    let db = Database::connect(opt).await.unwrap();

    initialize_db(&db).await.unwrap();

    db
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
    let path: PathBuf = [lib_path, ".1.db"].iter().collect();
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
