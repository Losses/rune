//! Exercise scan exit statuses through the real CLI, with isolated user configuration.
#![cfg(unix)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result};
use database::entities::media_files;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, EntityTrait, Statement};
use tempfile::TempDir;

struct Library {
    home: TempDir,
}

impl Library {
    fn new() -> Result<Self> {
        let home = TempDir::new()?;
        fs::create_dir(home.path().join("Music"))?;
        let library = Self { home };
        let output = library.scan()?;
        assert!(output.status.success(), "initial scan failed: {output:?}");
        Ok(library)
    }

    fn scan(&self) -> Result<Output> {
        Ok(Command::new(env!("CARGO_BIN_EXE_rune-cli"))
            .arg(self.home.path().join("Music"))
            .arg("scan")
            .env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("config"))
            .output()?)
    }

    fn add_audio(&self) -> Result<()> {
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/startup_0.ogg"),
            self.home.path().join("Music/track.ogg"),
        )?;
        Ok(())
    }

    async fn database(&self) -> Result<DatabaseConnection> {
        Ok(Database::connect(format!(
            "sqlite://{}",
            self.home.path().join("Music/.rune/.0.db").display()
        ))
        .await?)
    }

    async fn execute(&self, sql: &str) -> Result<()> {
        let db = self.database().await?;
        db.execute_raw(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
            .await?;
        db.close().await?;
        Ok(())
    }

    fn assert_failure(&self, diagnostic: &str) -> Result<()> {
        let output = self.scan()?;
        let logs = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // Confirm setup completed and the intended returned-error branch was reached.
        assert!(logs.contains(diagnostic), "{logs}");
        assert_eq!(output.status.code(), Some(1), "{logs}");
        Ok(())
    }
}

#[tokio::test]
async fn successful_audio_and_artwork_scan_exits_zero() -> Result<()> {
    let library = Library::new()?;
    library.add_audio()?;
    let output = library.scan()?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let db = library.database().await?;
    let track = media_files::Entity::find()
        .one(&db)
        .await?
        .context("audio scan did not persist the track")?;
    assert_eq!(track.file_name, "track.ogg");
    assert!(
        track.cover_art_checked_at.is_some(),
        "artwork was not inspected"
    );
    db.close().await?;
    Ok(())
}

#[test]
fn replica_open_error_exits_nonzero() -> Result<()> {
    let library = Library::new()?;
    #[cfg(target_os = "macos")]
    let config_dir = library
        .home
        .path()
        .join("Library/Application Support/ci.not.rune");
    #[cfg(not(target_os = "macos"))]
    let config_dir = library.home.path().join("config/rune");
    fs::write(
        config_dir.join("nid"),
        "00000000-0000-0000-0000-000000000000",
    )?;
    library.assert_failure("Failed to open metadata replica")
}

#[tokio::test]
async fn audio_scan_error_exits_nonzero() -> Result<()> {
    let library = Library::new()?;
    // Unlike a constraint violation (which rejects one file), a query failure aborts the scan.
    library.execute(
        "CREATE TRIGGER fail_metadata BEFORE INSERT ON music_metadata_source_tags BEGIN SELECT * FROM missing_scan_table; END;",
    ).await?;
    library.add_audio()?;
    library.assert_failure("Library scan failed")
}

#[tokio::test]
async fn artwork_scan_error_exits_nonzero() -> Result<()> {
    let library = Library::new()?;
    library.add_audio()?;
    let output = library.scan()?;
    assert!(output.status.success(), "fixture scan failed: {output:?}");
    library
        .execute("UPDATE media_files SET cover_art_checked_at = NULL")
        .await?;
    library.execute(
        "CREATE TRIGGER fail_artwork BEFORE UPDATE OF cover_art_checked_at ON media_files BEGIN SELECT * FROM missing_scan_table; END;",
    ).await?;
    library.assert_failure("Artwork scan failed")
}
