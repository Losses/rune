//! Saving what `mix` and `recommend` found as a file.

use std::fmt::Write as _;
use std::fs;
use std::path::{self, Path, PathBuf};

use anyhow::{Context, Result};
use dunce::canonicalize;
use uuid::Uuid;

use database::entities::media_files;

/// The error for a `--format` given without the `--output` to save to.
pub const OUTPUT_REQUIRED: &str = "Output file path is required when format is specified";

/// The error for a `--format` that is not one the command can save.
pub const UNSUPPORTED_FORMAT: &str = "Unsupported format. Supported formats are 'json' and 'm3u8'.";

/// `path` with `expected_extension` in place of any other extension, or added to a path without
/// one.
pub fn check_and_correct_extension(path: &Path, expected_extension: &str) -> PathBuf {
    if path.extension().and_then(|ext| ext.to_str()) != Some(expected_extension) {
        let mut corrected_path = path.to_path_buf();
        corrected_path.set_extension(expected_extension);
        corrected_path
    } else {
        path.to_path_buf()
    }
}

/// Saves `files` as an M3U8 playlist at `output`, one entry for each of them in their order,
/// each track as the path to it from the playlist's folder. `library` is the canonical root that
/// the tracks' directories are relative to; a relative `output` is taken from the working
/// directory.
///
/// The playlist takes the place of `output` only when it is complete, so an export that fails
/// leaves a file that was already there as it was. Returns where the playlist was saved.
pub fn save_as_m3u8<'a>(
    library: &Path,
    output: &Path,
    files: impl IntoIterator<Item = &'a media_files::Model>,
) -> Result<PathBuf> {
    let corrected = check_and_correct_extension(output, "m3u8");
    if corrected != output {
        eprintln!("Warning: Output file extension corrected to .m3u8");
    }
    let name = corrected
        .file_name()
        .with_context(|| format!("{} names no file", corrected.display()))?;
    let folder = path::absolute(&corrected)
        .with_context(|| format!("Failed to resolve {}", corrected.display()))?
        .parent()
        .map(Path::to_path_buf)
        .with_context(|| format!("{} has no folder", corrected.display()))?;
    fs::create_dir_all(&folder)
        .with_context(|| format!("Failed to create {}", folder.display()))?;
    // The library root is canonical, and a path between two folders is only right when both are.
    let folder =
        canonicalize(&folder).with_context(|| format!("Failed to resolve {}", folder.display()))?;

    let mut playlist = String::from("#EXTM3U\n");
    for file in files {
        let track = library.join(&file.directory).join(&file.file_name);
        let from_playlist = pathdiff::diff_paths(&track, &folder).with_context(|| {
            format!(
                "No path leads from {} to {}",
                folder.display(),
                track.display()
            )
        })?;
        writeln!(playlist, "{}", from_playlist.display())?;
    }

    let saved = folder.join(name);
    replace_file(&saved, &playlist)?;
    Ok(saved)
}

/// Puts `contents` at `path` in one step: they are written to a file beside it, which then takes
/// its place, with the permissions of the file it replaces. When that fails, a file already at
/// `path` is as it was.
fn replace_file(path: &Path, contents: &str) -> Result<()> {
    let temp = path.with_file_name(format!(".{}.tmp", Uuid::new_v4()));
    let replaced = write_then_rename(&temp, path, contents);
    if replaced.is_err() {
        let _ = fs::remove_file(&temp);
    }
    replaced
}

fn write_then_rename(temp: &Path, path: &Path, contents: &str) -> Result<()> {
    fs::write(temp, contents).with_context(|| format!("Failed to write {}", temp.display()))?;
    if let Ok(existing) = fs::metadata(path) {
        fs::set_permissions(temp, existing.permissions())
            .with_context(|| format!("Failed to set the permissions of {}", temp.display()))?;
    }
    fs::rename(temp, path).with_context(|| format!("Failed to replace {}", path.display()))
}
