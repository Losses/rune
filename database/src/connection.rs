use std::fs::create_dir_all;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use arroy::distances::Euclidean;
use arroy::Database as ArroyDatabase;
use heed::{Env, EnvFlags, EnvOpenOptions};
use log::{info, LevelFilter};
use sea_orm::{ConnectOptions, Database};
use tantivy::{schema::*, IndexReader, TantivyError};
use tantivy::{Index, IndexWriter, ReloadPolicy};

use migration::Migrator;
use migration::MigratorTrait;

use crate::actions::mixes::initialize_mix_queries;

pub type MainDbConnection = sea_orm::DatabaseConnection;

pub async fn connect_main_db(lib_path: &str) -> Result<MainDbConnection> {
    let path: PathBuf = [lib_path, ".rune", ".0.db"].iter().collect();

    let dir_path = path
        .parent()
        .with_context(|| "Invalid path: parent directory not found")?;

    if !dir_path.exists() {
        std::fs::create_dir_all(dir_path)?;
    }

    let path_str = path.into_os_string().into_string().unwrap();
    let db_url = format!("sqlite:{}?mode=rwc", path_str);
    let mut opt = ConnectOptions::new(db_url);
    opt.sqlx_logging(true)
        .sqlx_logging_level(LevelFilter::Debug);

    info!("Initializing main database: {}", path_str);

    let db = Database::connect(opt).await?;

    initialize_db(&db).await?;

    Ok(db)
}

pub async fn initialize_db(conn: &sea_orm::DatabaseConnection) -> Result<()> {
    Migrator::up(conn, None).await?;

    initialize_mix_queries(conn).await?;

    Ok(())
}

const DB_SIZE: usize = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct RecommendationDbConnection {
    pub env: Env,
    pub db: ArroyDatabase<Euclidean>,
}

/// Initialize the recommendation database.
///
/// # Arguments
/// * `db_path` - The path to the database directory.
///
/// # Returns
/// * `Result<(Env, ArroyDatabase<Euclidean>), Box<dyn std::error::Error>>` - The database environment and the Arroy database.
pub fn connect_recommendation_db(lib_path: &str) -> Result<RecommendationDbConnection> {
    let path: PathBuf = [lib_path, ".rune", ".analysis"].iter().collect();

    if !path.exists() {
        std::fs::create_dir_all(&path)?;
    }

    let path_str = match path.into_os_string().into_string() {
        Ok(x) => x,
        Err(_) => bail!("Failed to convert database path"),
    };

    info!("Initializing recommendation database: {}", path_str);

    let env = unsafe {
        EnvOpenOptions::new()
            .map_size(DB_SIZE)
            .flags(EnvFlags::NO_LOCK)
            .open(path_str)?
    };

    let mut wtxn = env.write_txn()?;

    let db: ArroyDatabase<Euclidean> = env.create_database(&mut wtxn, None)?;

    wtxn.commit()?;

    Ok(RecommendationDbConnection { env, db })
}

pub struct SearchDbConnection {
    pub w: IndexWriter,
    pub r: IndexReader,
    pub schema: Schema,
    pub index: Index,
}

pub fn connect_search_db(lib_path: &str) -> Result<SearchDbConnection> {
    let path: PathBuf = [lib_path, ".rune", ".search"].iter().collect();
    let exists = path.exists();

    if !exists {
        create_dir_all(path.clone())?;
    }

    let mut schema_builder = Schema::builder();
    schema_builder.add_text_field("name", TEXT | STORED);
    schema_builder.add_text_field("latinization", TEXT | STORED);
    schema_builder.add_text_field("tid", TEXT | STORED);
    schema_builder.add_i64_field("type", INDEXED | FAST);
    schema_builder.add_i64_field("id", INDEXED | FAST | STORED);

    let schema = schema_builder.build();
    let index =
        Index::create_in_dir(path.clone(), schema.clone()).or_else(|error| match error {
            TantivyError::IndexAlreadyExists => Ok(Index::open_in_dir(path.clone())?),
            _ => Err(error),
        })?;

    let writer: IndexWriter = index.writer(15_000_000)?;
    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::OnCommitWithDelay)
        .try_into()?;

    Ok(SearchDbConnection {
        w: writer,
        r: reader,
        schema,
        index,
    })
}
