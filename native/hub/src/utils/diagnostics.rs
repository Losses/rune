#[cfg(target_os = "android")]
use std::time::Instant;
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use fsio::FsIo;
#[cfg(target_os = "android")]
use fsio::self_test::skipped_layer_results;
use fsio::self_test::{FsSelfTestLayerResult, run_fs_self_test};
use log::{error, info};
use rinf::{DartSignal, RustSignal};
use sea_orm::sqlx::{ConnectOptions, Connection, sqlite::SqliteConnectOptions};
use uuid::Uuid;

use crate::macros::BootstrapRequestReceiver;
use crate::messages::*;

const LAYER_SAF_INIT: &str = "L1 SAF initialization (AndroidFsIo)";

fn emit_progress(result: &FsSelfTestLayerResult) {
    FsSelfTestProgress {
        layer: result.layer.clone(),
        ok: result.ok,
        skipped: result.skipped,
        elapsed_ms: result.elapsed_ms as i64,
        detail: result.detail.clone(),
    }
    .send_signal_to_dart();
}

async fn run_self_test(path: &str) -> Vec<FsSelfTestLayerResult> {
    let mut results: Vec<FsSelfTestLayerResult> = Vec::new();

    #[cfg(target_os = "android")]
    {
        let start = Instant::now();
        match FsIo::new(Path::new(".rune/.android-fs.db"), path) {
            Ok(fsio) => {
                let result = FsSelfTestLayerResult {
                    layer: LAYER_SAF_INIT.to_string(),
                    ok: true,
                    skipped: false,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    detail: "SAF tree accessible; ndk-saf initialized and cache refreshed"
                        .to_string(),
                };
                emit_progress(&result);
                results.push(result);

                let layer_results = run_fs_self_test(&fsio, Path::new(""), emit_progress).await;
                results.extend(layer_results);
            }
            Err(e) => {
                let detail = format!("Failed to construct AndroidFsIo: {e:#?}");
                let result = FsSelfTestLayerResult {
                    layer: LAYER_SAF_INIT.to_string(),
                    ok: false,
                    skipped: false,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    detail: detail.clone(),
                };
                emit_progress(&result);
                results.push(result);

                let skip_reason = format!("Skipped: {detail}");
                for result in skipped_layer_results(&skip_reason) {
                    emit_progress(&result);
                    results.push(result);
                }
            }
        }
    }

    #[cfg(not(target_os = "android"))]
    {
        let result = FsSelfTestLayerResult {
            layer: LAYER_SAF_INIT.to_string(),
            ok: false,
            skipped: true,
            elapsed_ms: 0,
            detail: "Not an Android build; using std filesystem backend".to_string(),
        };
        emit_progress(&result);
        results.push(result);

        let fsio = FsIo::new();
        let layer_results = run_fs_self_test(&fsio, Path::new(path), emit_progress).await;
        results.extend(layer_results);
    }

    results
}

async fn run_wal_self_test(path: &str, db_path: &str) -> WalSelfTestResult {
    let result = run_wal_probe(path, db_path).await;
    match result {
        Ok(()) => WalSelfTestResult {
            supported: true,
            detail: None,
        },
        Err(error) => WalSelfTestResult {
            supported: false,
            detail: Some(format!("{error:#}")),
        },
    }
}

async fn run_wal_probe(path: &str, db_path: &str) -> anyhow::Result<()> {
    use anyhow::Context;
    use database::connection::get_storage_info;
    use fsio::FsIo;

    #[cfg(target_os = "android")]
    let fsio = FsIo::new(Path::new(".rune/.android-fs.db"), path)?;
    #[cfg(not(target_os = "android"))]
    let fsio = FsIo::new();

    let storage = get_storage_info(&fsio, path, Some(db_path))?;
    #[cfg(target_os = "android")]
    if storage.db_dir.is_absolute() {
        tokio::fs::create_dir_all(&storage.db_dir).await?;
    } else {
        fsio.ensure_directory(&storage.db_dir).await?;
    }
    #[cfg(not(target_os = "android"))]
    fsio.ensure_directory(&storage.db_dir).await?;

    let temporary_path = storage
        .db_dir
        .join(format!(".wal-probe-{}.db", Uuid::new_v4().simple()));
    #[cfg(target_os = "android")]
    let native_path = if storage.db_dir.is_absolute() {
        tokio::fs::write(&temporary_path, b"").await?;
        temporary_path.clone()
    } else {
        let node = fsio.ensure_file(&temporary_path).await?;
        fsio.canonicalize_path(&node.path)?
    };
    #[cfg(not(target_os = "android"))]
    let native_path = {
        let node = fsio.ensure_file(&temporary_path).await?;
        fsio.canonicalize_path(&node.path)?
    };

    let probe = async {
        let options = SqliteConnectOptions::new()
            .filename(&native_path)
            .create_if_missing(true);
        let mut connection = options.connect().await?;
        let mode: String = sea_orm::sqlx::query_scalar("PRAGMA journal_mode=WAL")
            .fetch_one(&mut connection)
            .await?;
        anyhow::ensure!(
            mode.eq_ignore_ascii_case("wal"),
            "SQLite selected journal mode {mode}"
        );
        sea_orm::sqlx::query("CREATE TABLE wal_probe (value INTEGER NOT NULL)")
            .execute(&mut connection)
            .await?;
        sea_orm::sqlx::query("INSERT INTO wal_probe (value) VALUES (1)")
            .execute(&mut connection)
            .await?;
        let value: i64 = sea_orm::sqlx::query_scalar("SELECT value FROM wal_probe")
            .fetch_one(&mut connection)
            .await?;
        anyhow::ensure!(value == 1, "SQLite WAL probe read back an unexpected value");
        let mode: String = sea_orm::sqlx::query_scalar("PRAGMA journal_mode=DELETE")
            .fetch_one(&mut connection)
            .await?;
        anyhow::ensure!(
            mode.eq_ignore_ascii_case("delete"),
            "SQLite selected journal mode {mode} when switching back to DELETE"
        );
        connection.close().await?;
        Ok::<(), anyhow::Error>(())
    }
    .await
    .context("SQLite WAL probe failed");

    let cleanup_paths = [
        temporary_path.clone(),
        PathBuf::from(format!("{}-wal", temporary_path.display())),
        PathBuf::from(format!("{}-shm", temporary_path.display())),
    ];
    let cleanup = if cfg!(target_os = "android") && storage.db_dir.is_absolute() {
        let native_paths = [
            native_path.clone(),
            PathBuf::from(format!("{}-wal", native_path.display())),
            PathBuf::from(format!("{}-shm", native_path.display())),
        ];
        remove_sqlite_files_if_present(native_paths).await
    } else {
        remove_fsio_files_if_present(&fsio, cleanup_paths).await
    };
    cleanup.context("could not remove SQLite WAL probe files")?;
    probe
}

async fn remove_sqlite_files_if_present(paths: [PathBuf; 3]) -> anyhow::Result<()> {
    let mut first_error = None;
    for path in paths {
        if let Err(error) = tokio::fs::remove_file(path).await
            && error.kind() != ErrorKind::NotFound
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), |error| Err(error.into()))
}

async fn remove_fsio_files_if_present(fsio: &FsIo, paths: [PathBuf; 3]) -> anyhow::Result<()> {
    let mut first_error = None;
    for path in paths {
        let result = match fsio.exists(&path) {
            Ok(true) => fsio.remove_file(&path).await.map_err(anyhow::Error::from),
            Ok(false) => Ok(()),
            Err(error) => Err(anyhow::Error::from(error)),
        };
        if let Err(error) = result
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

pub async fn receive_fs_self_test() {
    let receiver = RunFsSelfTestRequest::get_dart_signal_receiver();

    info!("FS self-test listener started");

    while let Some(dart_signal) = receiver.recv().await {
        let path = dart_signal.message.path;
        let db_path = dart_signal.message.db_path;
        info!("Running filesystem self-test on: {path}");

        let results = std::panic::AssertUnwindSafe(run_self_test(&path));
        let results = match futures::FutureExt::catch_unwind(results).await {
            Ok(results) => results,
            Err(e) => {
                error!("Filesystem self-test panicked: {e:?}");
                let panic_message = e
                    .downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| e.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic");
                vec![FsSelfTestLayerResult {
                    layer: "Self-test runner".to_string(),
                    ok: false,
                    skipped: false,
                    elapsed_ms: 0,
                    detail: format!("panic during self-test: {panic_message}"),
                }]
            }
        };

        let success = results.iter().all(|r| r.skipped || r.ok);
        let wal_probe = run_wal_self_test(&path, &db_path).await;
        let report_json = match serde_json::to_string(&results) {
            Ok(json) => json,
            Err(e) => {
                error!("Failed to serialize self-test report: {e:?}");
                "[]".to_string()
            }
        };

        RunFsSelfTestResponse {
            success,
            report_json,
            wal_probe,
        }
        .send_signal_to_dart();

        info!("Filesystem self-test finished, success: {success}");
    }
}

impl BootstrapRequestReceiver for RunFsSelfTestRequest {
    fn start() {
        tokio::spawn(receive_fs_self_test());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    use tempfile::TempDir;

    #[tokio::test]
    async fn wal_self_test_reports_typed_result_and_removes_probe_file() -> anyhow::Result<()> {
        let directory = TempDir::new()?;
        let library = directory.path().join("library");
        tokio::fs::create_dir_all(&library).await?;
        let result = run_wal_self_test(
            library.to_str().context("library path")?,
            directory.path().to_str().context("database path")?,
        )
        .await;
        assert!(result.supported, "{:?}", result.detail);
        let db_dir = library.join(".rune");
        assert!(!std::fs::read_dir(db_dir)?.any(|entry| {
            entry
                .map(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".wal-probe-")
                })
                .unwrap_or(false)
        }));
        Ok(())
    }
}
