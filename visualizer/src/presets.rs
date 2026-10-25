//! The scene folders of a directory, picked at random with a history and on a timer, the way
//! MilkDrop 2 does it.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rand::Rng;

/// A scene's folder and its name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Preset {
    pub path: PathBuf,
    /// The folder's name, dots and all.
    pub name: String,
}

impl Preset {
    pub fn from_path(path: &Path) -> Self {
        let full = path.to_string_lossy();
        let file = full.rsplit(['/', '\\']).next().unwrap_or(&full);
        Self {
            name: if file.is_empty() { "scene" } else { file }.to_string(),
            path: path.to_path_buf(),
        }
    }
}

/// How many loaded scenes "previous" can walk back through (MilkDrop 2: `PRESET_HIST_LEN`).
const HISTORY_LEN: usize = 64;

struct Entry {
    preset: Preset,
    /// The folder the scene is in, relative to the folder it was found under.
    group: String,
}

/// A scene of the library as the preset menu shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetInfo {
    pub name: String,
    pub path: PathBuf,
    pub group: String,
}

/// The folder of `path` relative to `root`, with `/` between the folders; the name of `root`
/// itself for what lies directly in it.
fn group_of(root: &Path, path: &Path) -> String {
    let relative = path
        .parent()
        .and_then(|parent| parent.strip_prefix(root).ok())
        .unwrap_or(Path::new(""));
    let parts: Vec<_> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    if parts.is_empty() {
        root.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    } else {
        parts.join("/")
    }
}

/// Add the scenes of `dir` and its subfolders to `entries`; nothing of `dir` on an error. A scene
/// is a folder with a `scene.toml`; what is inside it belongs to it and is not scanned.
fn collect(dir: &Path, entries: &mut Vec<Entry>) -> Result<()> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let read = std::fs::read_dir(&current)
            .with_context(|| format!("Failed to read {}", current.display()))?;
        for entry in read {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_dir() || entry.file_name().to_string_lossy().starts_with('.')
            {
                continue;
            }
            if crate::scene::is_scene(&path) {
                found.push(Entry {
                    group: group_of(dir, &path),
                    preset: Preset::from_path(&path),
                });
            } else {
                pending.push(path);
            }
        }
    }
    entries.extend(found);
    Ok(())
}

/// The scenes that take part in rotation, with the history of what was loaded. The rules are
/// those of MilkDrop 2 (Nullsoft, BSD-3): "previous" walks back through the last
/// [`HISTORY_LEN`] scenes and "next" walks forward again before it picks a new random one.
#[derive(Default)]
pub struct PresetList {
    entries: Vec<Entry>,
    /// The presets loaded through this list, oldest first.
    history: VecDeque<Preset>,
    /// Index into `history` of the preset that is loaded. Meaningless while it is empty.
    cursor: usize,
}

impl PresetList {
    /// Scan every folder of `dirs` and its subfolders and replace the list with the scenes of
    /// all of them, once per folder. Hidden folders and symlinked folders (possible cycles) are
    /// skipped. A folder that cannot be read is skipped; its error is in the
    /// second part of the result. Returns the number of scenes found.
    pub fn scan_all(&mut self, dirs: &[PathBuf]) -> (usize, Vec<anyhow::Error>) {
        let mut entries = Vec::new();
        let mut errors = Vec::new();
        for dir in dirs {
            if let Err(e) = collect(dir, &mut entries) {
                errors.push(e);
            }
        }
        (self.replace(entries), errors)
    }

    fn replace(&mut self, mut entries: Vec<Entry>) -> usize {
        entries.sort_by(|a, b| a.preset.cmp(&b.preset));
        entries.dedup_by(|a, b| a.preset == b.preset);
        self.entries = entries;
        self.history.clear();
        self.cursor = 0;
        self.entries.len()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Every scene, by group and then by name, ignoring case.
    pub fn infos(&self) -> Vec<PresetInfo> {
        let mut infos: Vec<PresetInfo> = self
            .entries
            .iter()
            .map(|entry| PresetInfo {
                name: entry.preset.name.clone(),
                path: entry.preset.path.clone(),
                group: entry.group.clone(),
            })
            .collect();
        infos.sort_by_cached_key(|i| {
            (
                i.group.to_lowercase(),
                i.name.to_lowercase(),
                i.path.clone(),
            )
        });
        infos
    }

    /// MilkDrop 2's `NextPreset`: forward in the history if the user went back, else a new
    /// random scene.
    pub fn next(&mut self, rng: &mut impl Rng) -> Result<Preset> {
        if !self.history.is_empty() && self.cursor + 1 < self.history.len() {
            self.cursor += 1;
            return Ok(self.history[self.cursor].clone());
        }
        self.random(rng)
    }

    /// One step back in the history. `None` (nothing to do) at its start, like MilkDrop 2.
    pub fn previous(&mut self) -> Result<Option<Preset>> {
        self.check_not_empty()?;
        if self.cursor == 0 || self.history.is_empty() {
            return Ok(None);
        }
        self.cursor -= 1;
        Ok(Some(self.history[self.cursor].clone()))
    }

    /// A new random scene. Forward history is dropped.
    pub fn random(&mut self, rng: &mut impl Rng) -> Result<Preset> {
        self.check_not_empty()?;
        let preset = self.entries[rng.gen_range(0..self.entries.len())]
            .preset
            .clone();
        if !self.history.is_empty() {
            self.history.truncate(self.cursor + 1);
        }
        self.history.push_back(preset.clone());
        if self.history.len() > HISTORY_LEN {
            self.history.pop_front();
        }
        self.cursor = self.history.len() - 1;
        Ok(preset)
    }

    fn check_not_empty(&self) -> Result<()> {
        if self.entries.is_empty() {
            bail!("No scenes in the scene directory");
        }
        Ok(())
    }
}

/// When the next automatic switch is due, on the render loop's clock (seconds). MilkDrop 2:
/// a scene is shown for the blend time, 16 s and up to 10 s more at random.
#[derive(Default)]
pub struct AutoSwitch {
    due: Option<f64>,
}

impl AutoSwitch {
    /// MilkDrop 2's blend time, kept in the wait (scenes cut).
    pub const BLEND_SECONDS: f64 = 2.7;
    const BETWEEN_SECONDS: f64 = 16.0;
    const RANDOM_SECONDS: f64 = 10.0;

    /// A scene was loaded: count again from the next frame.
    pub fn restart(&mut self) {
        self.due = None;
    }

    /// Call once per frame. Returns whether a switch is due now; the caller then loads a scene
    /// and calls [`Self::restart`]. While `paused` the timer keeps starting over.
    pub fn tick(&mut self, now: f64, paused: bool, rng: &mut impl Rng) -> bool {
        if paused {
            self.due = None;
            return false;
        }
        let due = *self.due.get_or_insert_with(|| {
            now + Self::BLEND_SECONDS
                + Self::BETWEEN_SECONDS
                + rng.gen_range(0.0..1.0) * Self::RANDOM_SECONDS
        });
        now > due
    }
}

#[cfg(test)]
mod tests {
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    use super::*;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(7)
    }

    fn listed(names: &[&str]) -> PresetList {
        let entries = names
            .iter()
            .map(|name| Entry {
                preset: Preset::from_path(Path::new(name)),
                group: String::new(),
            })
            .collect();
        let mut list = PresetList::default();
        list.replace(entries);
        list
    }

    /// A scene folder `name` under `root`: an empty `scene.toml` is enough to be found.
    fn scene(root: &Path, name: &str) {
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(root.join(name).join("scene.toml"), "").unwrap();
    }

    #[test]
    fn scan_is_recursive_but_skips_hidden_and_symlinked_folders() {
        let dir = std::env::temp_dir().join(format!("rune-presets-{}", std::process::id()));
        for name in [
            "b",
            "a",
            "Fractal/deep",
            ".git/hidden",
            "Fractal/deep/inner",
        ] {
            scene(&dir, name);
        }
        std::fs::write(dir.join("ignored.milk"), "").unwrap();
        std::fs::write(dir.join("Fractal/also.milk"), "").unwrap();
        let mut list = PresetList::default();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
            std::os::unix::fs::symlink(dir.join("Fractal"), dir.join("link")).unwrap();
        }
        // `inner` is inside a scene: it is not scanned.
        assert_eq!(list.scan_all(std::slice::from_ref(&dir)).0, 3);
        let mut rng = rng();
        let mut names: Vec<String> = (0..50)
            .map(|_| list.random(&mut rng).unwrap().name)
            .collect();
        names.sort();
        names.dedup();
        assert_eq!(names, ["a", "b", "deep"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scan_all_combines_folders_once_per_scene_and_skips_unreadable_ones() {
        let root = std::env::temp_dir().join(format!("rune-presets-many-{}", std::process::id()));
        let (one, two) = (root.join("one"), root.join("one/two"));
        let other = root.join("other");
        for name in ["one/a", "one/two/b", "other/c"] {
            scene(&root, name);
        }
        let mut list = PresetList::default();
        // `two` lies inside `one`: its scene is not counted twice.
        let dirs = [one.clone(), two, other.clone(), root.join("missing")];
        let (count, errors) = list.scan_all(&dirs);
        assert_eq!((count, errors.len()), (3, 1));
        assert_eq!(list.scan_all(&[]).0, 0);
        assert_eq!(list.len(), 0);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn groups_are_relative_to_the_scene_folder() {
        let root = Path::new("/music/Scenes");
        assert_eq!(group_of(root, &root.join("a")), "Scenes");
        assert_eq!(group_of(root, &root.join("Night/Calm/x")), "Night/Calm");
    }

    #[test]
    fn infos_are_sorted_by_group_then_name_ignoring_case() {
        let dir = std::env::temp_dir().join(format!("rune-presets-infos-{}", std::process::id()));
        for name in ["b/z", "b/Y", "A/q", "root"] {
            scene(&dir, name);
        }
        let mut list = PresetList::default();
        list.scan_all(std::slice::from_ref(&dir));
        let root = dir.file_name().unwrap().to_string_lossy().into_owned();
        let got: Vec<(String, String)> = list
            .infos()
            .into_iter()
            .map(|i| (i.group, i.name))
            .collect();
        let expected = |g: &str, n: &str| (g.to_string(), n.to_string());
        let mut want = vec![
            expected("A", "q"),
            expected("b", "Y"),
            expected("b", "z"),
            expected(&root, "root"),
        ];
        want.sort_by_key(|(g, n)| (g.to_lowercase(), n.to_lowercase()));
        assert_eq!(got, want);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn history_walks_back_and_forward() {
        let mut list = listed(&["a", "b", "c", "d"]);
        let mut rng = rng();
        assert!(list.previous().unwrap().is_none());
        let first = list.random(&mut rng).unwrap();
        assert!(list.previous().unwrap().is_none());
        let second = list.random(&mut rng).unwrap();
        assert_eq!(list.previous().unwrap(), Some(first));
        assert_eq!(list.next(&mut rng).unwrap(), second);
    }

    #[test]
    fn random_after_previous_drops_forward_history() {
        let mut list = listed(&["a", "b", "c"]);
        let mut rng = rng();
        list.random(&mut rng).unwrap();
        list.random(&mut rng).unwrap();
        list.random(&mut rng).unwrap();
        list.previous().unwrap();
        list.previous().unwrap();
        list.random(&mut rng).unwrap();
        assert_eq!(list.history.len(), 2);
        // Nothing to go forward to: next picks a new one and extends the history.
        list.next(&mut rng).unwrap();
        assert_eq!(list.history.len(), 3);
        assert_eq!(list.cursor, 2);
    }

    #[test]
    fn history_keeps_the_last_64() {
        let mut list = listed(&["a", "b"]);
        let mut rng = rng();
        for _ in 0..100 {
            list.random(&mut rng).unwrap();
        }
        assert_eq!(list.history.len(), HISTORY_LEN);
        for _ in 0..HISTORY_LEN - 1 {
            assert!(list.previous().unwrap().is_some());
        }
        assert!(list.previous().unwrap().is_none());
    }

    #[test]
    fn empty_list_is_an_error() {
        let mut list = PresetList::default();
        list.scan_all(&[]);
        assert!(list.next(&mut rng()).is_err());
        assert!(list.previous().is_err());
        assert!(list.random(&mut rng()).is_err());
    }

    #[test]
    fn auto_switch_schedule() {
        let mut rng = rng();
        let mut auto = AutoSwitch::default();
        // The first tick fixes the due time: 2.7 + 16 + 0..10 s after it.
        assert!(!auto.tick(100.0, false, &mut rng));
        let due = auto.due.unwrap();
        assert!((118.7..128.7).contains(&due), "{due}");
        assert!(!auto.tick(due - 0.01, false, &mut rng));
        assert!(auto.tick(due + 0.01, false, &mut rng));
        // A load starts the count again.
        auto.restart();
        assert!(!auto.tick(due + 0.02, false, &mut rng));
        assert!(auto.due.unwrap() >= due + 0.02 + 18.7);
    }

    #[test]
    fn auto_switch_pause_keeps_pushing_the_start() {
        let mut rng = rng();
        let mut auto = AutoSwitch::default();
        assert!(!auto.tick(0.0, false, &mut rng));
        for t in [10.0, 100.0, 1000.0] {
            assert!(!auto.tick(t, true, &mut rng));
        }
        // Unpausing counts from zero, not from the old due time.
        assert!(!auto.tick(1000.5, false, &mut rng));
        let due = auto.due.unwrap();
        assert!((1019.2..1029.2).contains(&due), "{due}");
    }
}
