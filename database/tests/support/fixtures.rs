#![allow(dead_code)]

use anyhow::Result;
use database::{
    actions::metadata::{MissingHostFilePolicy, ScanOptions, scan_audio_library},
    sync::{LibrarySyncStore, SeaOrmSyncStore},
};
use fsio::FsIo;
use migration::{Migrator, MigratorTrait, initialize_node_id};
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::{
    fs::File,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub fn asset_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets")
}

pub fn audio_fixture() -> PathBuf {
    audio_fixture_at(0)
}

pub fn audio_fixture_at(index: usize) -> PathBuf {
    asset_directory().join(format!("startup_{index}.ogg"))
}

pub async fn migrate(db: &DatabaseConnection, actor: Uuid) -> Result<()> {
    let _ = initialize_node_id(actor.to_string());
    Migrator::up(db, None).await?;
    Ok(())
}

pub async fn disk_database(path: &Path, actor: Uuid) -> Result<DatabaseConnection> {
    File::create(path)?;
    let db = Database::connect(format!("sqlite://{}", path.display())).await?;
    migrate(&db, actor).await?;
    Ok(db)
}

pub async fn open_library(
    db: DatabaseConnection,
    actor: Uuid,
    namespace: Option<Uuid>,
) -> Result<LibrarySyncStore> {
    LibrarySyncStore::open(db, actor, namespace).await
}

pub async fn scan_library(
    fs: &FsIo,
    db: &DatabaseConnection,
    actor: &str,
    root: &Path,
    store: &SeaOrmSyncStore,
) -> Result<usize> {
    scan_audio_library(
        fs,
        db,
        actor,
        root,
        ScanOptions {
            cleanup: false,
            force: false,
            missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
            cancel_token: None,
        },
        |_| {},
        store,
    )
    .await
}

pub async fn memory_database(actor: Uuid) -> Result<DatabaseConnection> {
    let _ = initialize_node_id(actor.to_string());
    let db = Database::connect("sqlite::memory:").await?;
    Migrator::up(&db, None).await?;
    Ok(db)
}

pub async fn cleanup_scan(
    fs: &FsIo,
    db: &DatabaseConnection,
    actor: &str,
    root: &Path,
    store: &SeaOrmSyncStore,
    policy: MissingHostFilePolicy,
) -> Result<usize> {
    scan_audio_library(
        fs,
        db,
        actor,
        root,
        ScanOptions {
            cleanup: true,
            force: false,
            missing_host_file: policy,
            cancel_token: None,
        },
        |_| {},
        store,
    )
    .await
}

pub fn single_connection_options() -> ConnectOptions {
    let mut options = ConnectOptions::new("sqlite::memory:");
    options
        .max_connections(1)
        .min_connections(1)
        .idle_timeout(None)
        .max_lifetime(None);
    options
}
