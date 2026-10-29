//! The scene folders of a directory, picked at random with a history, the way MilkDrop 2 does it.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rand::Rng;

/// Where a scene is: a folder, or the program (a built-in scene, by name).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Location {
    Folder(PathBuf),
    Builtin(String),
}

impl Location {
    /// What the path of a built-in scene starts with where a folder's path is expected (the
    /// app's messages). A folder's path is absolute, so the two cannot be mixed up.
    const BUILTIN_PREFIX: &'static str = "builtin:";

    /// The location a path of [`Self::to_string`] names.
    pub fn parse(text: &str) -> Self {
        match text.strip_prefix(Self::BUILTIN_PREFIX) {
            Some(name) => Self::Builtin(name.to_string()),
            None => Self::Folder(PathBuf::from(text)),
        }
    }
}

impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Folder(path) => path.display().fmt(f),
            Self::Builtin(name) => write!(f, "{}{name}", Self::BUILTIN_PREFIX),
        }
    }
}

/// A scene and its name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Preset {
    pub location: Location,
    /// The folder's name, dots and all; a built-in scene's `name`.
    pub name: String,
}

impl Preset {
    pub fn from_path(path: &Path) -> Self {
        let full = path.to_string_lossy();
        let file = full.rsplit(['/', '\\']).next().unwrap_or(&full);
        Self {
            name: if file.is_empty() { "scene" } else { file }.to_string(),
            location: Location::Folder(path.to_path_buf()),
        }
    }

    pub fn builtin(name: &str) -> Self {
        Self {
            name: name.to_string(),
            location: Location::Builtin(name.to_string()),
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
    pub location: Location,
    /// Empty for a built-in scene.
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

/// The scenes that take part in rotation (the built-in ones and those of the scene folders),
/// with the history of what was loaded. The rules are
/// those of MilkDrop 2 (Nullsoft, BSD-3): "previous" walks back through the last
/// [`HISTORY_LEN`] scenes and "next" walks forward again before it picks a new random one.
#[derive(Default)]
pub struct PresetList {
    /// The names of the built-in scenes.
    builtins: Vec<String>,
    entries: Vec<Entry>,
    /// The presets loaded through this list, oldest first.
    history: VecDeque<Preset>,
    /// Index into `history` of the preset that is loaded. Meaningless while it is empty.
    cursor: usize,
}

impl PresetList {
    /// A list with these built-in scenes, which are all it holds until folders are scanned.
    pub fn with_builtins(names: Vec<String>) -> Self {
        let mut list = Self {
            builtins: names,
            ..Self::default()
        };
        list.replace(Vec::new());
        list
    }

    /// Scan every folder of `dirs` and its subfolders and replace the list with the scenes of
    /// all of them, once per folder, next to the built-in scenes; a scene folder with the name of a
    /// built-in scene is a scene of its own and does not replace it. Hidden folders and symlinked
    /// folders (possible cycles) are skipped. A folder that cannot be read is skipped; its error is
    /// in the second part of the result. Returns the number of scenes in the list.
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
        for name in &self.builtins {
            entries.push(Entry {
                preset: Preset::builtin(name),
                group: String::new(),
            });
        }
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

    /// The number of scenes that come from folders.
    pub fn folder_len(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e.preset.location, Location::Folder(_)))
            .count()
    }

    /// Every scene, by group and then by name, ignoring case.
    pub fn infos(&self) -> Vec<PresetInfo> {
        let mut infos: Vec<PresetInfo> = self
            .entries
            .iter()
            .map(|entry| PresetInfo {
                name: entry.preset.name.clone(),
                location: entry.preset.location.clone(),
                group: entry.group.clone(),
            })
            .collect();
        infos.sort_by_cached_key(|i| {
            (
                i.group.to_lowercase(),
                i.name.to_lowercase(),
                i.location.clone(),
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

    /// The scene for an automatic switch, while `current` is on screen: forward in the history if
    /// the way back was taken (and that scene is another one), else a new random scene other than
    /// `current`, so that a switch always shows something new unless there is only one scene.
    pub fn next_other(&mut self, rng: &mut impl Rng, current: &Location) -> Result<Preset> {
        if self.cursor + 1 < self.history.len()
            && self.history[self.cursor + 1].location != *current
        {
            self.cursor += 1;
            return Ok(self.history[self.cursor].clone());
        }
        self.random_other(rng, current)
    }

    /// A new random scene other than `current`, or any when `current` is the only one. Forward
    /// history is dropped.
    pub fn random_other(&mut self, rng: &mut impl Rng, current: &Location) -> Result<Preset> {
        self.check_not_empty()?;
        let others: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|entry| entry.preset.location != *current)
            .collect();
        let preset = if others.is_empty() {
            self.entries[rng.gen_range(0..self.entries.len())]
                .preset
                .clone()
        } else {
            others[rng.gen_range(0..others.len())].preset.clone()
        };
        self.remember(preset)
    }

    /// A new random scene. Forward history is dropped.
    pub fn random(&mut self, rng: &mut impl Rng) -> Result<Preset> {
        self.check_not_empty()?;
        let preset = self.entries[rng.gen_range(0..self.entries.len())]
            .preset
            .clone();
        self.remember(preset)
    }

    /// Make `preset` the newest of the history, dropping what lies ahead of the current one.
    fn remember(&mut self, preset: Preset) -> Result<Preset> {
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
    fn builtins_are_listed_and_a_folder_scene_of_the_same_name_is_listed_beside_one() {
        let names = ["Alpha", "Beta"].map(String::from).to_vec();
        let mut list = PresetList::with_builtins(names);
        // Before any scan the built-in scenes are there.
        assert_eq!((list.len(), list.folder_len()), (2, 0));
        let dir = std::env::temp_dir().join(format!("rune-presets-builtin-{}", std::process::id()));
        for name in ["Beta", "Gamma"] {
            scene(&dir, name);
        }
        assert_eq!(list.scan_all(std::slice::from_ref(&dir)).0, 4);
        assert_eq!(list.folder_len(), 2);
        let infos = list.infos();
        // The folder's Beta does not replace the built-in one: both are listed.
        let beta: Vec<_> = infos.iter().filter(|i| i.name == "Beta").collect();
        assert_eq!(beta.len(), 2);
        assert!(
            beta.iter()
                .any(|i| i.location == Location::Folder(dir.join("Beta")))
        );
        assert!(
            beta.iter()
                .any(|i| i.location == Location::Builtin("Beta".into()) && i.group.is_empty())
        );
        assert!(infos.contains(&PresetInfo {
            name: "Alpha".into(),
            location: Location::Builtin("Alpha".into()),
            group: String::new(),
        }));
        // Every scene takes part in random picks and the history.
        let mut rng = rng();
        let mut seen = Vec::new();
        for _ in 0..100 {
            let preset = list.random(&mut rng).unwrap();
            if !seen.contains(&preset.location) {
                seen.push(preset.location);
            }
        }
        assert_eq!(seen.len(), 4);
        assert!(seen.contains(&Location::Builtin("Alpha".into())));
        assert!(seen.contains(&Location::Builtin("Beta".into())));
        assert!(seen.contains(&Location::Folder(dir.join("Beta"))));
        // Without the folder only the built-in scenes are left.
        assert_eq!(list.scan_all(&[]).0, 2);
        assert_eq!(list.folder_len(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn locations_round_trip_through_their_text() {
        for location in [
            Location::Folder(PathBuf::from("/music/Scenes/a b")),
            Location::Builtin("CRT Wall".into()),
        ] {
            assert_eq!(Location::parse(&location.to_string()), location);
        }
        assert_eq!(
            Location::Builtin("Growth".into()).to_string(),
            "builtin:Growth"
        );
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
    fn an_automatic_pick_is_never_the_scene_on_screen_unless_it_is_the_only_one() {
        let mut list = listed(&["a", "b", "c"]);
        let mut rng = rng();
        let mut current = list.random(&mut rng).unwrap();
        for _ in 0..200 {
            let next = list.next_other(&mut rng, &current.location).unwrap();
            assert_ne!(next.location, current.location);
            current = next;
        }
        assert_eq!(list.history.len(), HISTORY_LEN);
        let mut only = listed(&["a"]);
        let alone = Preset::from_path(Path::new("a")).location;
        assert_eq!(only.next_other(&mut rng, &alone).unwrap().location, alone);
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
}
