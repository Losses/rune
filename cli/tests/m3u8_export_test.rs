//! Runs the real `rune-cli` on a library in a temporary folder and reads the playlists it writes,
//! and saves playlists of tracks of its own for what a two-track library cannot show.
//!
//! The CLI keeps this device's id and library registry in the user's configuration folder. The
//! tests point it at a folder of their own through the environment, which Windows does not read
//! for that folder, so they run on the other platforms.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::Error;
use anyhow::{Context, Result, ensure};
use database::actions::file::{get_file_by_id, get_file_id_from_path};
use database::connection::connect_main_db;
use database::entities::media_files;
use fsio::FsIo;
use rune::export::save_as_m3u8;
use rust_decimal::Decimal;
use tempfile::TempDir;
use uuid::Uuid;

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

/// The tracks a playlist names, each resolved from the playlist's folder, in the order written.
fn tracks_of(playlist: &Path) -> Result<Vec<PathBuf>> {
    let folder = playlist.parent().context("playlist without a folder")?;
    fs::read_to_string(playlist)
        .with_context(|| format!("no playlist at {}", playlist.display()))?
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            folder
                .join(line)
                .canonicalize()
                .with_context(|| format!("{line} leads to no file"))
        })
        .collect()
}

/// `b.ogg` is the shorter track, so these queries give the two tracks in a known order.
const SHORTEST_FIRST: &str = "lib::all(true);sort::duration(true)";
const LONGEST_FIRST: &str = "lib::all(true);sort::duration(false)";

fn track(directory: &str, file_name: &str) -> media_files::Model {
    media_files::Model {
        id: 0,
        file_name: file_name.to_owned(),
        directory: directory.to_owned(),
        extension: "ogg".to_owned(),
        file_hash: String::new(),
        last_modified: String::new(),
        change_token: None,
        cover_art_uuid: None,
        cover_art_checked_at: None,
        sample_rate: 44100,
        duration: Decimal::ZERO,
        local_available: true,
        content_hash: None,
        content_size: None,
        source_uuid: String::new(),
    }
}

#[test]
fn a_playlist_keeps_the_order_of_its_tracks_and_a_track_given_twice() -> Result<()> {
    let root = TempDir::new()?;
    let library = root.path().canonicalize()?.join("Music");
    let playlist = root.path().join("out/again.m3u8");

    let saved = save_as_m3u8(
        &library,
        &playlist,
        &[
            track("Album", "a.ogg"),
            track("", "b.ogg"),
            track("Album", "a.ogg"),
            track("Other", "c.ogg"),
        ],
    )?;

    assert_eq!(
        fs::read_to_string(saved)?,
        "#EXTM3U\n../Music/Album/a.ogg\n../Music/b.ogg\n../Music/Album/a.ogg\n../Music/Other/c.ogg\n"
    );
    Ok(())
}

#[test]
fn a_playlist_that_replaces_another_keeps_its_permissions() -> Result<()> {
    let root = TempDir::new()?;
    let library = root.path().canonicalize()?.join("Music");
    let playlist = root.path().join("private.m3u8");
    fs::write(&playlist, "#EXTM3U\n")?;
    fs::set_permissions(&playlist, fs::Permissions::from_mode(0o600))?;

    save_as_m3u8(&library, &playlist, &[track("", "b.ogg")])?;

    assert_eq!(fs::read_to_string(&playlist)?, "#EXTM3U\nMusic/b.ogg\n");
    assert_eq!(fs::metadata(&playlist)?.permissions().mode() & 0o777, 0o600);
    Ok(())
}

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
            SHORTEST_FIRST,
            "-f",
            "m3u8",
            "-o",
            &playlist.to_string_lossy(),
        ],
    )?;

    assert_eq!(
        tracks_of(&playlist)?,
        [
            library.root.join("Music/b.ogg"),
            library.root.join("Music/Album/a.ogg")
        ]
    );
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
            LONGEST_FIRST,
            "-f",
            "m3u8",
            "-o",
            "lists/all.m3u8",
        ],
    )?;

    assert_eq!(
        tracks_of(&elsewhere.join("lists/all.m3u8"))?,
        [
            library.root.join("Music/Album/a.ogg"),
            library.root.join("Music/b.ogg")
        ]
    );
    Ok(())
}

#[tokio::test]
async fn recommend_export_preserves_rank_for_a_relative_library() -> Result<()> {
    let library = Library::scanned()?;
    library.succeed(&library.root, &["Music", "analyze", "-c", "cpu"])?;
    let music = library.root.join("Music");
    let db = connect_main_db(
        &FsIo::new(),
        music.to_str().context("non-UTF-8 fixture path")?,
        None,
        &Uuid::new_v4().to_string(),
    )
    .await?;
    // The second track ranks itself first, whichever filename the scanner assigned it.
    let seed = get_file_by_id(&db, 2)
        .await?
        .context("missing second track")?;
    let seed_path = Path::new(&seed.directory).join(seed.file_name);
    for format in ["json", "m3u8"] {
        let output = library.root.join(format!("out/similar.{format}"));
        library.succeed(
            &library.root,
            &[
                "Music",
                "recommend",
                "-p",
                &seed_path.to_string_lossy(),
                "-n",
                "2",
                "-f",
                format,
                "-o",
                &output.to_string_lossy(),
            ],
        )?;
    }

    let json: Vec<(i32, f32)> =
        serde_json::from_reader(fs::File::open(library.root.join("out/similar.json"))?)?;
    let ranked_ids: Vec<_> = json.into_iter().map(|(id, _)| id).collect();
    assert_eq!(
        ranked_ids,
        [2, 1],
        "the fixture must rank both tracks in non-ID order"
    );
    let mut playlist_ids = Vec::new();
    for path in tracks_of(&library.root.join("out/similar.m3u8"))? {
        playlist_ids.push(
            get_file_id_from_path(&db, &music, &path)
                .await
                .map_err(Error::msg)?,
        );
    }
    assert_eq!(
        playlist_ids, ranked_ids,
        "M3U8 must preserve the JSON recommendation order"
    );
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
            SHORTEST_FIRST,
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
            SHORTEST_FIRST,
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
