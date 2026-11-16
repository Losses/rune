//! The scenes the visualizer can show, in the user's order, with a history of what was loaded.

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::scene::{SceneInputs, Source};
use anyhow::{Context, Result, bail};
use rand::Rng;

/// Inputs the current source can provide.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VisualizerInputs {
    #[default]
    None,
    SpectrumOnly,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetLoadError {
    InputsUnavailable,
}

impl fmt::Display for PresetLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputsUnavailable => {
                f.write_str("The source does not provide this scene's inputs")
            }
        }
    }
}

impl std::error::Error for PresetLoadError {}

/// Where a scene is: a folder, or the program (a built-in scene, by name). The app's cover wall
/// takes part in the choice like a scene, but the app draws it: the visualizer draws nothing while
/// it is on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Location {
    Folder(PathBuf),
    Builtin(String),
    CoverWall,
}

impl Location {
    /// What the path of a built-in scene starts with where a folder's path is expected (the
    /// app's messages). A folder's path is absolute, so the two cannot be mixed up.
    const BUILTIN_PREFIX: &'static str = "builtin:";
    /// The path of the cover wall in the app's messages.
    const COVER_WALL: &'static str = "app:cover-wall";

    /// The location a path of [`Self::to_string`] names.
    pub fn parse(text: &str) -> Self {
        if text == Self::COVER_WALL {
            return Self::CoverWall;
        }
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
            Self::CoverWall => f.write_str(Self::COVER_WALL),
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

    pub fn cover_wall() -> Self {
        Self {
            name: "Cover Wall".to_string(),
            location: Location::CoverWall,
        }
    }
}

/// How many loaded scenes "previous" can walk back through (MilkDrop 2: `PRESET_HIST_LEN`).
const HISTORY_LEN: usize = 64;

struct Entry {
    preset: Preset,
    inputs: SceneInputs,
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
    pub inputs: SceneInputs,
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
                    inputs: Source::Dir(path.clone())
                        .required_inputs()
                        .unwrap_or_default(),
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

/// The scenes the visualizer can show (the built-in ones, those of the scene folders and the
/// app's cover wall if it has one), with the history of what was loaded. Scenes the user turned
/// off are listed but not picked. A switch takes the next scene in the user's order; "previous"
/// walks back through the last [`HISTORY_LEN`] scenes, as in MilkDrop 2 (Nullsoft, BSD-3), and
/// "random" picks any scene that is on.
#[derive(Default)]
pub struct PresetList {
    /// The names of the built-in scenes.
    builtins: Vec<(String, SceneInputs)>,
    /// Whether the app's cover wall is one of the scenes.
    cover_wall: bool,
    entries: Vec<Entry>,
    /// The scenes picks leave out, unless every scene is off.
    disabled: Vec<Location>,
    /// The user's order of the scenes; scenes not in it come after those that are, in the order
    /// of `entries`.
    order: Vec<Location>,
    /// The presets loaded through this list, oldest first.
    history: VecDeque<Preset>,
    /// Index into `history` of the preset that is loaded. Meaningless while it is empty.
    cursor: usize,
    inputs: VisualizerInputs,
}

impl PresetList {
    /// A list with these built-in scenes, which are all it holds until folders are scanned.
    pub fn with_builtins(names: Vec<(String, SceneInputs)>) -> Self {
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

    /// Make the app's cover wall one of the scenes, or none of them. The history is emptied when
    /// that changes.
    pub fn set_cover_wall(&mut self, cover_wall: bool) {
        if cover_wall == self.cover_wall {
            return;
        }
        self.cover_wall = cover_wall;
        let folders = std::mem::take(&mut self.entries)
            .into_iter()
            .filter(|e| matches!(e.preset.location, Location::Folder(_)))
            .collect();
        self.replace(folders);
    }

    /// Leave the scenes at `locations` out of picks, replacing the earlier ones; those not in the
    /// list (also scenes of folders scanned later) are picked. Turning every scene off counts as
    /// turning none off.
    pub fn set_disabled(&mut self, locations: Vec<Location>) {
        self.disabled = locations;
    }

    /// The order switches go through the scenes in, replacing the earlier one.
    pub fn set_order(&mut self, locations: Vec<Location>) {
        self.order = locations;
    }

    /// Whether picks may choose the scene at `location`.
    pub fn is_enabled(&self, location: &Location) -> bool {
        !self.disabled.contains(location) || self.enabled().next().is_none()
    }

    pub fn set_inputs(&mut self, inputs: VisualizerInputs) -> bool {
        let changed = self.inputs != inputs;
        self.inputs = inputs;
        changed
    }

    pub fn source_inputs(&self) -> VisualizerInputs {
        self.inputs
    }

    pub fn supports_source_inputs(&self, location: &Location) -> bool {
        self.supports_inputs(location, self.inputs)
    }

    /// Inputs the installed visualizer scenes can use; the app's cover wall is not a shader scene.
    pub fn available_inputs(&self) -> VisualizerInputs {
        if self.entries.iter().any(|entry| {
            entry.preset.location != Location::CoverWall
                && self.supports(&entry.preset.location, self.inputs)
        }) {
            self.inputs
        } else {
            VisualizerInputs::None
        }
    }

    /// Whether an entry can draw with the currently available inputs.
    pub fn supports_inputs(&self, location: &Location, inputs: VisualizerInputs) -> bool {
        self.supports(location, inputs)
    }

    /// The scenes random picks choose from: those not turned off.
    fn enabled(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|e| !self.disabled.contains(&e.preset.location))
    }

    /// The scenes picks choose from (every scene when all are off).
    fn choices(&self) -> Vec<&Entry> {
        let enabled: Vec<&Entry> = self.enabled().collect();
        let enabled = if enabled.is_empty() {
            self.entries.iter().collect()
        } else {
            enabled
        };
        enabled
            .into_iter()
            .filter(|entry| match self.inputs {
                VisualizerInputs::None => false,
                VisualizerInputs::SpectrumOnly => {
                    entry.preset.location != Location::CoverWall
                        && entry.inputs == SceneInputs::Spectrum
                }
                VisualizerInputs::Full => true,
            })
            .collect()
    }

    /// Every scene in the user's order.
    fn ordered(&self) -> Vec<&Entry> {
        let mut entries: Vec<&Entry> = self.entries.iter().collect();
        entries.sort_by_key(|entry| {
            self.order
                .iter()
                .position(|l| *l == entry.preset.location)
                .unwrap_or(self.order.len())
        });
        entries
    }

    /// The first scene to pick after `current` in the user's order, going from the last scene to
    /// the first; `current` itself only when it is the only one. From the start of the order when
    /// `current` is not in the list.
    fn after(&self, current: &Location) -> Option<Preset> {
        let ordered = self.ordered();
        let choices = self.choices();
        let start = ordered
            .iter()
            .position(|e| e.preset.location == *current)
            .map_or(0, |index| index + 1);
        (0..ordered.len())
            .map(|step| ordered[(start + step) % ordered.len()])
            .find(|entry| choices.iter().any(|c| std::ptr::eq(*c, *entry)))
            .map(|entry| entry.preset.clone())
    }

    /// The scene a visualizer that starts begins with: the one shown last if it is still listed
    /// and picked, else the one after it in the order, else the first. None when the list is
    /// empty.
    pub fn resume(&mut self) -> Option<Preset> {
        let last = self.history.get(self.cursor).map(|p| p.location.clone());
        if let Some(last) = &last
            && self.supports(last, self.inputs)
            && self.is_enabled(last)
            && self.contains(last)
        {
            return Some(self.history[self.cursor].clone());
        }
        let preset = match last {
            Some(last) => self.after(&last)?,
            None => self
                .ordered()
                .into_iter()
                .find(|entry| {
                    self.is_enabled(&entry.preset.location)
                        && self.supports(&entry.preset.location, self.inputs)
                })?
                .preset
                .clone(),
        };
        self.remember(preset).ok()
    }

    fn replace(&mut self, mut entries: Vec<Entry>) -> usize {
        for (name, inputs) in &self.builtins {
            entries.push(Entry {
                preset: Preset::builtin(name),
                inputs: *inputs,
                group: String::new(),
            });
        }
        if self.cover_wall {
            entries.push(Entry {
                preset: Preset::cover_wall(),
                inputs: SceneInputs::Spectrum,
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

    /// Make the scene at `location`, which was loaded by name, the newest of the history (forward
    /// history is dropped). Nothing happens if it is not in the list.
    pub fn remember_at(&mut self, location: &Location) {
        let preset = self
            .entries
            .iter()
            .find(|e| e.preset.location == *location)
            .map(|e| e.preset.clone());
        if let Some(preset) = preset {
            let _ = self.remember(preset);
        }
    }

    /// The scene shown last before the app started, for [`Self::resume`]: taken only while the
    /// history is empty, so the scenes of this run come first.
    pub fn resume_from(&mut self, location: &Location) {
        if self.history.is_empty() {
            self.remember_at(location);
        }
    }

    /// Whether the scene at `location` is in the list.
    pub fn contains(&self, location: &Location) -> bool {
        self.entries.iter().any(|e| e.preset.location == *location)
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
                inputs: entry.inputs,
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

    /// One step back in the history. `None` (nothing to do) at its start, like MilkDrop 2.
    pub fn previous(&mut self) -> Result<Option<Preset>> {
        self.check_not_empty()?;
        if self.cursor == 0 || self.history.is_empty() {
            return Ok(None);
        }
        let Some(cursor) = (0..self.cursor)
            .rev()
            .find(|&index| self.supports(&self.history[index].location, self.inputs))
        else {
            return Ok(None);
        };
        self.cursor = cursor;
        Ok(Some(self.history[cursor].clone()))
    }

    /// The scene for a switch, while `current` is on screen: the next one in the order that is
    /// not off. A switch shows something new unless there is only one scene to pick. Forward
    /// history is dropped.
    pub fn next_other(&mut self, current: &Location) -> Result<Preset> {
        self.check_not_empty()?;
        let preset = self
            .after(current)
            .context("No available scene to switch to")?;
        self.remember(preset)
    }

    /// A new random scene. Forward history is dropped.
    pub fn random(&mut self, rng: &mut impl Rng) -> Result<Preset> {
        self.check_not_empty()?;
        let choices = self.choices();
        anyhow::ensure!(
            !choices.is_empty(),
            "No scene supports the available inputs"
        );
        let preset = choices[rng.gen_range(0..choices.len())].preset.clone();
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

    fn supports(&self, location: &Location, inputs: VisualizerInputs) -> bool {
        self.entries.iter().any(|entry| {
            entry.preset.location == *location
                && (location == &Location::CoverWall
                    || (inputs != VisualizerInputs::None
                        && (inputs == VisualizerInputs::Full
                            || entry.inputs == SceneInputs::Spectrum)))
        })
    }
}

#[cfg(test)]
mod tests {
    impl PresetList {
        /// The number of scenes that come from folders.
        pub fn folder_len(&self) -> usize {
            self.entries
                .iter()
                .filter(|e| matches!(e.preset.location, Location::Folder(_)))
                .count()
        }
    }

    use rand::{SeedableRng, rngs::StdRng};
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use super::*;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(7)
    }

    /// A list of folder scenes `names`.
    fn listed(names: &[&str]) -> PresetList {
        let entries = names
            .iter()
            .map(|name| Entry {
                preset: Preset::from_path(Path::new(name)),
                inputs: SceneInputs::SpectrumAndChart,
                group: String::new(),
            })
            .collect();
        let mut list = PresetList::default();
        list.set_inputs(VisualizerInputs::Full);
        list.replace(entries);
        list
    }

    #[test]
    fn in_order_a_switch_takes_the_next_scene_that_is_on_and_wraps_around() {
        let mut list = listed(&["a", "b", "c", "d"]);
        let at = |name: &str| Preset::from_path(Path::new(name)).location;
        list.set_order(vec![at("c"), at("a"), at("d"), at("b")]);
        list.set_disabled(vec![at("d")]);
        let mut current = at("c");
        let mut seen = Vec::new();
        for _ in 0..4 {
            current = list.next_other(&current).unwrap().location;
            seen.push(current.clone());
        }
        assert_eq!(seen, [at("a"), at("b"), at("c"), at("a")]);
        // A scene that was turned off while on screen is followed by the one after it.
        assert_eq!(list.next_other(&at("d")).unwrap().location, at("b"));
    }

    #[test]
    fn in_order_a_new_start_resumes_the_scene_shown_last() {
        let mut list = listed(&["a", "b", "c"]);
        let at = |name: &str| Preset::from_path(Path::new(name)).location;
        list.set_order(vec![at("b"), at("c"), at("a")]);
        // Nothing shown yet: the first in the order.
        assert_eq!(list.resume().unwrap().location, at("b"));
        list.next_other(&at("b")).unwrap();
        assert_eq!(list.resume().unwrap().location, at("c"));
        // Turned off since: the one after it.
        list.set_disabled(vec![at("c")]);
        assert_eq!(list.resume().unwrap().location, at("a"));
        // Nothing at all without scenes.
        assert!(PresetList::default().resume().is_none());
    }

    #[test]
    fn a_scene_loaded_by_name_or_from_the_last_run_is_resumed() {
        let mut list = listed(&["a", "b", "c"]);
        let at = |name: &str| Preset::from_path(Path::new(name)).location;
        // The last run's scene counts only while nothing was shown in this run.
        list.resume_from(&at("b"));
        assert_eq!(list.resume().unwrap().location, at("b"));
        list.resume_from(&at("c"));
        assert_eq!(list.resume().unwrap().location, at("b"));
        list.remember_at(&at("c"));
        assert_eq!(list.resume().unwrap().location, at("c"));
        // A scene that is not listed (gone since) is not remembered.
        list.remember_at(&at("gone"));
        assert_eq!(list.resume().unwrap().location, at("c"));
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
        list.set_inputs(VisualizerInputs::Full);
        #[cfg(unix)]
        {
            symlink(&dir, dir.join("loop")).unwrap();
            symlink(dir.join("Fractal"), dir.join("link")).unwrap();
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
        let names = ["Alpha", "Beta"]
            .map(|name| (name.to_string(), SceneInputs::SpectrumAndChart))
            .to_vec();
        let mut list = PresetList::with_builtins(names);
        list.set_inputs(VisualizerInputs::Full);
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
            inputs: SceneInputs::SpectrumAndChart,
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
            Location::CoverWall,
        ] {
            assert_eq!(Location::parse(&location.to_string()), location);
        }
        assert_eq!(
            Location::Builtin("Growth".into()).to_string(),
            "builtin:Growth"
        );
    }

    #[test]
    fn random_picks_leave_out_the_scenes_that_are_off_unless_all_are() {
        let mut list = listed(&["a", "b", "c"]);
        let at = |name: &str| Preset::from_path(Path::new(name)).location;
        list.set_disabled(vec![at("a"), at("c")]);
        let mut rng = rng();
        for _ in 0..50 {
            assert_eq!(list.random(&mut rng).unwrap().location, at("b"));
            // The only scene to pick comes again rather than one that is off.
            assert_eq!(list.next_other(&at("b")).unwrap().location, at("b"));
        }
        // Off is not gone: the scene is still listed and can be loaded by hand.
        assert_eq!(list.infos().len(), 3);
        list.set_disabled(vec![at("a"), at("b"), at("c")]);
        let mut seen: Vec<Location> = (0..50)
            .map(|_| list.random(&mut rng).unwrap().location)
            .collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 3);
    }

    #[test]
    fn the_cover_wall_is_a_scene_only_when_the_app_has_one() {
        let names = [("Alpha".to_string(), SceneInputs::SpectrumAndChart)].to_vec();
        let mut list = PresetList::with_builtins(names);
        list.set_inputs(VisualizerInputs::Full);
        assert_eq!(list.len(), 1);
        list.set_cover_wall(true);
        assert_eq!((list.len(), list.folder_len()), (2, 0));
        assert!(list.infos().contains(&PresetInfo {
            name: "Cover Wall".into(),
            location: Location::CoverWall,
            group: String::new(),
            inputs: SceneInputs::Spectrum,
        }));
        // A scan keeps it.
        assert_eq!(list.scan_all(&[]).0, 2);
        list.set_disabled(vec![Location::Builtin("Alpha".into())]);
        assert_eq!(
            list.random(&mut rng()).unwrap().location,
            Location::CoverWall
        );
        list.set_cover_wall(false);
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn spectrum_only_selection_skips_chart_scenes_for_next_random_and_previous() {
        let at = |name: &str| Preset::from_path(Path::new(name)).location;
        let mut list = listed(&["chart-a", "spectrum", "chart-b"]);
        list.entries
            .iter_mut()
            .find(|entry| entry.preset.location == at("spectrum"))
            .unwrap()
            .inputs = SceneInputs::Spectrum;
        list.set_inputs(VisualizerInputs::SpectrumOnly);
        list.set_order(vec![at("chart-a"), at("spectrum"), at("chart-b")]);
        list.remember_at(&at("spectrum"));
        assert!(list.supports_inputs(&at("spectrum"), VisualizerInputs::SpectrumOnly));
        assert!(!list.supports_inputs(&at("chart-a"), VisualizerInputs::SpectrumOnly));
        assert_eq!(
            list.next_other(&at("chart-a")).unwrap().location,
            at("spectrum")
        );
        assert_eq!(list.previous().unwrap().unwrap().location, at("spectrum"));
        assert_eq!(list.random(&mut rng()).unwrap().location, at("spectrum"));
    }

    #[test]
    fn source_inputs_have_three_typed_states() {
        let mut list = listed(&["chart", "spectrum"]);
        list.set_inputs(VisualizerInputs::None);
        assert_eq!(list.source_inputs(), VisualizerInputs::None);
        list.set_inputs(VisualizerInputs::SpectrumOnly);
        assert_eq!(list.source_inputs(), VisualizerInputs::SpectrumOnly);
        list.set_inputs(VisualizerInputs::Full);
        assert_eq!(list.source_inputs(), VisualizerInputs::Full);
    }

    #[test]
    fn history_walks_back() {
        let mut list = listed(&["a", "b", "c", "d"]);
        let mut rng = rng();
        assert!(list.previous().unwrap().is_none());
        let first = list.random(&mut rng).unwrap();
        assert!(list.previous().unwrap().is_none());
        list.random(&mut rng).unwrap();
        assert_eq!(list.previous().unwrap(), Some(first));
    }

    #[test]
    fn a_switch_is_never_to_the_scene_on_screen_unless_it_is_the_only_one() {
        let mut list = listed(&["a", "b", "c"]);
        let mut current = list.random(&mut rng()).unwrap();
        for _ in 0..200 {
            let next = list.next_other(&current.location).unwrap();
            assert_ne!(next.location, current.location);
            current = next;
        }
        assert_eq!(list.history.len(), HISTORY_LEN);
        let mut only = listed(&["a"]);
        let alone = Preset::from_path(Path::new("a")).location;
        assert_eq!(only.next_other(&alone).unwrap().location, alone);
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
        assert_eq!(list.cursor, 1);
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
        assert!(list.next_other(&Location::CoverWall).is_err());
        assert!(list.previous().is_err());
        assert!(list.random(&mut rng()).is_err());
    }
}
