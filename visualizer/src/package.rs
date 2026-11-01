//! Scenes packed into one file, `.runescene`: a zip archive of a scene folder, which the app
//! unpacks into a folder of its own and lists with the built-in scenes.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

/// The file name extension of a packed scene.
pub const EXTENSION: &str = "runescene";

/// The most an archive may unpack to, so that a damaged or hostile one cannot fill the disk.
const MAX_UNPACKED: u64 = 512 * 1024 * 1024;

/// Unpack the scene file `file` into the folder `dir` (made if missing), as a folder named after
/// the file, which replaces a scene of that name; returns that folder. The archive holds the
/// scene's files with `scene.toml` at its root, or in one folder at its root (what zipping a
/// scene folder gives). An archive that is not a scene, or has a file outside its root, leaves
/// nothing behind.
pub fn import(file: &Path, dir: &Path) -> Result<PathBuf> {
    let name = file
        .file_stem()
        .map(|stem| stem.to_string_lossy().trim().to_string())
        .filter(|name| !name.is_empty() && !name.starts_with('.'))
        .with_context(|| format!("{} has no name to give the scene", file.display()))?;
    let mut archive = File::open(file)
        .map_err(anyhow::Error::from)
        .and_then(|f| zip::ZipArchive::new(f).map_err(anyhow::Error::from))
        .with_context(|| format!("{} is not a scene file", file.display()))?;

    // Every entry's path inside the archive, refusing any that would land outside it.
    let mut paths = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let path = entry.enclosed_name().ok_or_else(|| {
            let name = entry.name().map(|n| n.into_owned()).unwrap_or_default();
            anyhow!("{} has a file outside of it: {name}", file.display())
        })?;
        paths.push((!ignored(&path)).then_some(path));
    }
    let root = root_of(paths.iter().flatten())
        .with_context(|| format!("{} has no scene.toml", file.display()))?;

    fs::create_dir_all(dir).with_context(|| format!("Failed to make {}", dir.display()))?;
    // Unpacked beside the scenes under a hidden name, which the scan of the folder skips, and
    // moved into place only when complete.
    let staging = dir.join(format!(".import-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    let unpacked = unpack(&mut archive, &paths, &root, &staging);
    if let Err(e) = unpacked.and_then(|()| {
        if crate::scene::is_scene(&staging) {
            Ok(())
        } else {
            Err(anyhow!("{} has no scene.toml", file.display()))
        }
    }) {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }

    let target = dir.join(&name);
    if target.exists() {
        fs::remove_dir_all(&target)
            .with_context(|| format!("Failed to replace {}", target.display()))?;
    }
    fs::rename(&staging, &target)
        .with_context(|| format!("Failed to move the scene into {}", target.display()))?;
    Ok(target)
}

/// Delete the scene folder `path`, which must be a folder directly in `dir` (one [`import`] made).
pub fn remove(path: &Path, dir: &Path) -> Result<()> {
    let parent = path
        .parent()
        .map(fs::canonicalize)
        .transpose()?
        .context("The scene has no folder around it")?;
    if parent != fs::canonicalize(dir)? {
        bail!("{} is not an imported scene", path.display());
    }
    fs::remove_dir_all(path).with_context(|| format!("Failed to delete {}", path.display()))
}

/// What archivers add on their own: macOS's resource forks and Finder's folder settings.
fn ignored(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, Component::Normal(part) if part == "__MACOSX" || part == ".DS_Store"))
}

/// The folder in the archive that holds `scene.toml`: its root, or its one top-level folder.
fn root_of<'a>(paths: impl Iterator<Item = &'a PathBuf> + Clone) -> Option<PathBuf> {
    let manifest = Path::new("scene.toml");
    if paths.clone().any(|p| p == manifest) {
        return Some(PathBuf::new());
    }
    let mut tops = paths.clone().filter_map(|p| p.components().next());
    let top = tops.next()?;
    if tops.any(|c| c != top) {
        return None;
    }
    let folder = PathBuf::from(top.as_os_str());
    paths
        .clone()
        .any(|p| *p == folder.join(manifest))
        .then_some(folder)
}

fn unpack(
    archive: &mut zip::ZipArchive<File>,
    paths: &[Option<PathBuf>],
    root: &Path,
    staging: &Path,
) -> Result<()> {
    let mut total = 0u64;
    for (index, path) in paths.iter().enumerate() {
        let Some(relative) = path.as_ref().and_then(|p| p.strip_prefix(root).ok()) else {
            continue;
        };
        let out = staging.join(relative);
        let entry = archive.by_index(index)?;
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        // The size an entry claims can lie; what is read is counted.
        let mut limited = entry.take(MAX_UNPACKED - total + 1);
        let written = io::copy(&mut limited, &mut File::create(&out)?)?;
        total += written;
        if total > MAX_UNPACKED {
            bail!("The scene unpacks to more than {} MiB", MAX_UNPACKED >> 20);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rune-package-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A zip at `path` with these (name, contents) entries.
    fn zip_with(path: &Path, entries: &[(&str, &str)]) {
        let mut writer = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, contents) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(contents.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn a_scene_at_the_root_or_in_one_folder_is_unpacked_under_the_files_name() {
        let base = temp("roots");
        let scenes = base.join("scenes");
        for (file, entries) in [
            (
                "Flat.runescene",
                vec![("scene.toml", "a"), ("view.wgsl", "b")],
            ),
            (
                "Zipped.runescene",
                vec![
                    ("Folder/scene.toml", "a"),
                    ("Folder/assets/x.png", "c"),
                    ("__MACOSX/Folder/._scene.toml", "junk"),
                ],
            ),
        ] {
            let path = base.join(file);
            zip_with(&path, &entries);
            let folder = import(&path, &scenes).unwrap();
            assert!(crate::scene::is_scene(&folder));
            assert_eq!(folder.parent().unwrap(), scenes);
        }
        assert!(scenes.join("Flat/view.wgsl").is_file());
        assert!(scenes.join("Zipped/assets/x.png").is_file());
        assert!(!scenes.join("__MACOSX").exists());
        // Nothing else is left in the folder.
        let mut names: Vec<_> = fs::read_dir(&scenes)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["Flat", "Zipped"]);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn importing_again_replaces_the_scene() {
        let base = temp("again");
        let scenes = base.join("scenes");
        let path = base.join("Mine.runescene");
        zip_with(&path, &[("scene.toml", "a"), ("old.wgsl", "")]);
        import(&path, &scenes).unwrap();
        zip_with(&path, &[("scene.toml", "b")]);
        let folder = import(&path, &scenes).unwrap();
        assert_eq!(fs::read_to_string(folder.join("scene.toml")).unwrap(), "b");
        assert!(!folder.join("old.wgsl").exists());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn what_is_not_a_scene_leaves_nothing_behind() {
        let base = temp("bad");
        let scenes = base.join("scenes");
        let cases: [(&str, &[(&str, &str)]); 3] = [
            ("NoManifest.runescene", &[("view.wgsl", "")]),
            ("TwoFolders.runescene", &[("a/scene.toml", ""), ("b/x", "")]),
            (
                "Escapes.runescene",
                &[("scene.toml", ""), ("../outside.txt", "")],
            ),
        ];
        for (file, entries) in cases {
            let path = base.join(file);
            zip_with(&path, entries);
            assert!(import(&path, &scenes).is_err(), "{file}");
        }
        fs::write(base.join("Text.runescene"), "not a zip").unwrap();
        assert!(import(&base.join("Text.runescene"), &scenes).is_err());
        assert!(!base.join("outside.txt").exists());
        let left = fs::read_dir(&scenes).map_or(0, |d| d.count());
        assert_eq!(left, 0);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn only_a_scene_folder_directly_in_the_folder_can_be_removed() {
        let base = temp("remove");
        let scenes = base.join("scenes");
        let path = base.join("Gone.runescene");
        zip_with(&path, &[("scene.toml", "")]);
        let folder = import(&path, &scenes).unwrap();
        assert!(remove(&base, &scenes).is_err());
        assert!(remove(&folder.join("scene.toml"), &scenes).is_err());
        remove(&folder, &scenes).unwrap();
        assert!(!folder.exists());
        fs::remove_dir_all(&base).unwrap();
    }
}
