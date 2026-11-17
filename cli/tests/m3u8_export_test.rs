//! Runs the real `rune-cli` on a library in a temporary folder and reads the playlists it writes.
//!
//! The CLI keeps this device's id and library registry in the user's configuration folder. The
//! tests point it at a folder of their own through the environment, which Windows does not read
//! for that folder, so they run on the other platforms.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, ensure};
use tempfile::TempDir;

struct Library {
    home: TempDir,
    /// The canonical folder that holds `Music`, the library.
    root: PathBuf,
}

impl Library {
    /// A scanned library of two tracks, `Music/b.ogg` and `Music/Album/a.ogg`.
    fn scanned() -> Result<Self> {
        let home = TempDir::new()?;
        let root = home.path().canonicalize()?.join("work");
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets");
        fs::create_dir_all(root.join("Music/Album"))?;
        fs::create_dir_all(root.join("elsewhere"))?;
        fs::copy(assets.join("startup_0.ogg"), root.join("Music/Album/a.ogg"))?;
        fs::copy(assets.join("startup_1.ogg"), root.join("Music/b.ogg"))?;
        let library = Self { home, root };
        library.succeed(&library.root, &["Music", "scan"])?;
        Ok(library)
    }

    fn tracks(&self) -> Vec<PathBuf> {
        vec![
            self.root.join("Music/Album/a.ogg"),
            self.root.join("Music/b.ogg"),
        ]
    }

    fn run(&self, cwd: &Path, arguments: &[&str]) -> Result<Output> {
        Command::new(env!("CARGO_BIN_EXE_rune-cli"))
            .args(arguments)
            .current_dir(cwd)
            .env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("config"))
            .output()
            .context("Failed to run rune-cli")
    }

    fn succeed(&self, cwd: &Path, arguments: &[&str]) -> Result<()> {
        let output = self.run(cwd, arguments)?;
        ensure!(
            output.status.success(),
            "rune-cli {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }
}

/// The tracks a playlist names, each resolved from the playlist's folder, in order of path.
fn tracks_of(playlist: &Path) -> Result<Vec<PathBuf>> {
    let folder = playlist.parent().context("playlist without a folder")?;
    let mut tracks = fs::read_to_string(playlist)
        .with_context(|| format!("no playlist at {}", playlist.display()))?
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            folder
                .join(line)
                .canonicalize()
                .with_context(|| format!("{line} leads to no file"))
        })
        .collect::<Result<Vec<_>>>()?;
    tracks.sort();
    Ok(tracks)
}

const ALL_TRACKS: &str = "lib::all(true)";

#[test]
fn mix_export_to_an_absolute_path_names_the_tracks() -> Result<()> {
    let library = Library::scanned()?;
    let playlist = library.root.join("out/all.m3u8");

    library.succeed(
        &library.root.join("Music"),
        &[
            ".",
            "mix",
            "-m",
            ALL_TRACKS,
            "-f",
            "m3u8",
            "-o",
            &playlist.to_string_lossy(),
        ],
    )?;

    assert_eq!(tracks_of(&playlist)?, library.tracks());
    Ok(())
}

#[test]
fn mix_export_to_a_relative_path_names_the_tracks_from_any_folder() -> Result<()> {
    let library = Library::scanned()?;
    let elsewhere = library.root.join("elsewhere");

    library.succeed(
        &elsewhere,
        &[
            "../Music",
            "mix",
            "-m",
            ALL_TRACKS,
            "-f",
            "m3u8",
            "-o",
            "lists/all.m3u8",
        ],
    )?;

    assert_eq!(
        tracks_of(&elsewhere.join("lists/all.m3u8"))?,
        library.tracks()
    );
    Ok(())
}

#[test]
fn recommend_export_names_the_tracks_for_a_relative_library() -> Result<()> {
    let library = Library::scanned()?;
    library.succeed(&library.root, &["Music", "analyze", "-c", "cpu"])?;
    let playlist = library.root.join("out/similar.m3u8");

    library.succeed(
        &library.root,
        &[
            "Music",
            "recommend",
            "-p",
            "b.ogg",
            "-f",
            "m3u8",
            "-o",
            &playlist.to_string_lossy(),
        ],
    )?;

    assert_eq!(tracks_of(&playlist)?, library.tracks());
    Ok(())
}

#[test]
fn a_failed_export_reports_it_and_keeps_the_file_already_there() -> Result<()> {
    let library = Library::scanned()?;
    let folder = library.root.join("out");
    let playlist = folder.join("kept.m3u8");
    fs::create_dir_all(&folder)?;
    fs::write(&playlist, "#EXTM3U\nkept.ogg\n")?;
    // Nothing can be added to the folder, so the new playlist cannot be written.
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o555))?;

    let output = library.run(
        &library.root,
        &[
            "Music",
            "mix",
            "-m",
            ALL_TRACKS,
            "-f",
            "m3u8",
            "-o",
            &playlist.to_string_lossy(),
        ],
    );
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o755))?;

    assert!(!output?.status.success());
    assert_eq!(fs::read_to_string(&playlist)?, "#EXTM3U\nkept.ogg\n");
    Ok(())
}

#[test]
fn an_export_that_cannot_make_its_folder_reports_it() -> Result<()> {
    let library = Library::scanned()?;
    let blocker = library.root.join("blocker");
    fs::write(&blocker, "a file, not a folder")?;

    let output = library.run(
        &library.root,
        &[
            "Music",
            "mix",
            "-m",
            ALL_TRACKS,
            "-f",
            "m3u8",
            "-o",
            &blocker.join("all.m3u8").to_string_lossy(),
        ],
    )?;

    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(&blocker)?, "a file, not a folder");
    Ok(())
}
