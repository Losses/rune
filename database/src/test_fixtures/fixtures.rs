use crate::connection::initialize_db;
use crate::{
    actions::metadata::{MissingHostFilePolicy, ScanOptions, scan_audio_library},
    sync::{LibrarySyncStore, SeaOrmSyncStore, model::RuneModel},
};
use anyhow::Result;
use fsio::FsIo;
use http_request::sync::MetadataPeer;
use migration::{Migrator, MigratorTrait, initialize_node_id};
use reqwest::Client;
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::fs;
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

pub fn unreachable_peer() -> MetadataPeer<RuneModel> {
    MetadataPeer::new("http://127.0.0.1:1", Client::new())
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

pub async fn scan_audio_fixtures(
    fs: &FsIo,
    db: &DatabaseConnection,
    actor: &str,
    root: &Path,
    store: &SeaOrmSyncStore,
    names: &[&str],
) -> Result<()> {
    let source = audio_fixture();
    for name in names {
        fs::copy(&source, root.join(name))?;
    }
    scan_library(fs, db, actor, root, store).await?;
    Ok(())
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

pub async fn initialized_memory(actor: Uuid) -> Result<DatabaseConnection> {
    let db = Database::connect("sqlite::memory:").await?;
    initialize_db(&db, &actor.to_string()).await?;
    Ok(db)
}

/// Writes a FLAC file `seconds` long with the Vorbis comments `tags`, given as `(key, value)`.
/// It holds no audio frames: its length is the one its stream information gives, which the
/// scanner reads as it reads any FLAC file's.
pub fn write_tagged_flac(path: &Path, seconds: u32, tags: &[(&str, &str)]) -> Result<()> {
    const SAMPLE_RATE: u64 = 44_100;
    fn block(output: &mut Vec<u8>, kind: u8, last: bool, body: &[u8]) -> Result<()> {
        let length = u32::try_from(body.len())?;
        anyhow::ensure!(length < 1 << 24, "a FLAC metadata block is too long");
        output.push(kind | if last { 0x80 } else { 0 });
        output.extend(&length.to_be_bytes()[1..]);
        output.extend(body);
        Ok(())
    }

    // Stream information: block sizes, unknown frame sizes, then the sample rate, the
    // channels less one (mono), the bits per sample less one (16 bits) and the total samples
    // in 64 bits, and an empty checksum.
    let mut info = Vec::new();
    info.extend(4096u16.to_be_bytes());
    info.extend(4096u16.to_be_bytes());
    info.extend([0; 6]);
    let samples = SAMPLE_RATE * u64::from(seconds);
    info.extend(((SAMPLE_RATE << 44) | (15 << 36) | samples).to_be_bytes());
    info.extend([0; 16]);

    // Vorbis comments keep their lengths in little-endian order.
    let mut comments = Vec::new();
    let vendor = b"rune test fixture";
    comments.extend(u32::try_from(vendor.len())?.to_le_bytes());
    comments.extend(vendor);
    comments.extend(u32::try_from(tags.len())?.to_le_bytes());
    for (key, value) in tags {
        let comment = format!("{key}={value}");
        comments.extend(u32::try_from(comment.len())?.to_le_bytes());
        comments.extend(comment.as_bytes());
    }

    let mut file = b"fLaC".to_vec();
    block(&mut file, 0, false, &info)?;
    block(&mut file, 4, true, &comments)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, file)?;
    Ok(())
}
