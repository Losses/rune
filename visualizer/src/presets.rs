//! A sorted list of the `.milk` / `.milk2` presets of a directory (or of the built-in pack),
//! with a cursor.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rand::Rng;

use crate::pack::Zip;

/// Where a preset's bytes are.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Location {
    File(PathBuf),
    /// A path inside the built-in pack.
    Pack(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Preset {
    pub location: Location,
    /// The file name without its extension.
    pub name: String,
}

impl Preset {
    fn new(location: Location) -> Self {
        let full = match &location {
            Location::File(path) => path.to_string_lossy().into_owned(),
            Location::Pack(path) => path.clone(),
        };
        let file = full.rsplit(['/', '\\']).next().unwrap_or(&full);
        let name = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
        Self {
            name: if name.is_empty() { "preset" } else { name }.to_string(),
            location,
        }
    }

    pub fn from_path(path: &Path) -> Self {
        Self::new(Location::File(path.to_path_buf()))
    }
}

fn is_preset_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".milk") || lower.ends_with(".milk2")
}

#[derive(Default)]
pub struct PresetList {
    presets: Vec<Preset>,
    /// Index of the preset loaded last, if it came from this list.
    current: Option<usize>,
}

impl PresetList {
    /// Scan `dir` and its subdirectories and replace the list. Preset packs are usually sorted
    /// into theme folders. Hidden directories and symlinked directories (possible cycles) are
    /// skipped. Returns the number of presets found.
    pub fn scan(&mut self, dir: &Path) -> Result<usize> {
        let mut presets = Vec::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(current) = pending.pop() {
            let entries = std::fs::read_dir(&current)
                .with_context(|| format!("Failed to read {}", current.display()))?;
            for entry in entries {
                let entry = entry?;
                let path = entry.path();
                if entry.file_type()?.is_dir() {
                    if !entry.file_name().to_string_lossy().starts_with('.') {
                        pending.push(path);
                    }
                    continue;
                }
                if path.to_str().is_some_and(is_preset_name) {
                    presets.push(Preset::new(Location::File(path)));
                }
            }
        }
        Ok(self.replace(presets))
    }

    /// Replace the list with the presets of `zip` (`presets/**.milk`, `.milk2`). Returns their count.
    pub fn scan_pack(&mut self, zip: &Zip) -> usize {
        let presets = zip
            .file_names()
            .into_iter()
            .filter(|name| name.starts_with("presets/") && is_preset_name(name))
            .map(|name| Preset::new(Location::Pack(name)))
            .collect();
        self.replace(presets)
    }

    fn replace(&mut self, mut presets: Vec<Preset>) -> usize {
        presets.sort();
        self.presets = presets;
        self.current = None;
        self.presets.len()
    }

    pub fn len(&self) -> usize {
        self.presets.len()
    }

    pub fn next(&mut self) -> Result<Preset> {
        let index = match self.current {
            Some(i) => (i + 1) % self.presets.len().max(1),
            None => 0,
        };
        self.select(index)
    }

    pub fn previous(&mut self) -> Result<Preset> {
        let len = self.presets.len().max(1);
        let index = match self.current {
            Some(i) => (i + len - 1) % len,
            None => len - 1,
        };
        self.select(index)
    }

    pub fn random(&mut self) -> Result<Preset> {
        let index = rand::thread_rng().gen_range(0..self.presets.len().max(1));
        self.select(index)
    }

    fn select(&mut self, index: usize) -> Result<Preset> {
        if self.presets.is_empty() {
            bail!("No presets in the preset directory");
        }
        self.current = Some(index);
        Ok(self.presets[index].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_sorted_and_cycles() {
        let dir = std::env::temp_dir().join(format!("rune-presets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["b.milk", "a.MILK", "c.milk2", "ignored.txt"] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        let mut list = PresetList::default();
        assert_eq!(list.scan(&dir).unwrap(), 3);
        let name = |p: Preset| p.name;
        assert_eq!(name(list.next().unwrap()), "a");
        assert_eq!(name(list.next().unwrap()), "b");
        assert_eq!(name(list.previous().unwrap()), "a");
        assert_eq!(name(list.previous().unwrap()), "c");
        assert_eq!(name(list.next().unwrap()), "a");
        assert!(list.random().is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scans_theme_folders_but_not_hidden_ones() {
        let dir = std::env::temp_dir().join(format!("rune-presets-nested-{}", std::process::id()));
        for sub in ["Fractal/Deep", "Waveform", ".git"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        for file in [
            "top.milk",
            "Fractal/Deep/a.milk",
            "Waveform/b.milk2",
            ".git/c.milk",
        ] {
            std::fs::write(dir.join(file), "").unwrap();
        }
        let mut list = PresetList::default();
        assert_eq!(list.scan(&dir).unwrap(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pack_presets_are_sorted_and_cycle() {
        let bytes = crate::pack::test_zip(&[
            ("presets/b.milk", b""),
            ("presets/a.MILK", b""),
            ("presets/sub/c.milk2", b""),
            ("textures/t.jpg", b""),
            ("presets/readme.txt", b""),
            ("SOURCES.txt", b""),
        ]);
        let zip = Zip::new(bytes).unwrap();
        let mut list = PresetList::default();
        assert_eq!(list.scan_pack(&zip), 3);
        assert_eq!(list.len(), 3);
        let first = list.next().unwrap();
        assert_eq!(first.name, "a");
        assert_eq!(first.location, Location::Pack("presets/a.MILK".into()));
        assert_eq!(list.next().unwrap().name, "b");
        assert_eq!(list.next().unwrap().name, "c");
        assert_eq!(list.next().unwrap().name, "a");
        assert_eq!(list.previous().unwrap().name, "c");
        for _ in 0..20 {
            assert!(matches!(list.random().unwrap().location, Location::Pack(_)));
        }
    }

    #[test]
    fn empty_list_is_an_error() {
        assert!(PresetList::default().next().is_err());
    }
}
