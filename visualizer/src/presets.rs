//! A sorted list of the `.milk` / `.milk2` files under a directory, with a cursor.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rand::Rng;

#[derive(Default)]
pub struct PresetList {
    files: Vec<PathBuf>,
    /// Index of the preset loaded last, if it came from this list.
    current: Option<usize>,
}

impl PresetList {
    /// Scan `dir` and its subdirectories and replace the list. Preset packs are usually sorted
    /// into theme folders. Hidden directories and symlinked directories (possible cycles) are
    /// skipped. Returns the number of presets found.
    pub fn scan(&mut self, dir: &Path) -> Result<usize> {
        let mut files = Vec::new();
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
                let extension = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(str::to_ascii_lowercase);
                if matches!(extension.as_deref(), Some("milk" | "milk2")) {
                    files.push(path);
                }
            }
        }
        files.sort();
        self.files = files;
        self.current = None;
        Ok(self.files.len())
    }

    pub fn next(&mut self) -> Result<PathBuf> {
        let index = match self.current {
            Some(i) => (i + 1) % self.files.len().max(1),
            None => 0,
        };
        self.select(index)
    }

    pub fn previous(&mut self) -> Result<PathBuf> {
        let len = self.files.len().max(1);
        let index = match self.current {
            Some(i) => (i + len - 1) % len,
            None => len - 1,
        };
        self.select(index)
    }

    pub fn random(&mut self) -> Result<PathBuf> {
        let index = rand::thread_rng().gen_range(0..self.files.len().max(1));
        self.select(index)
    }

    fn select(&mut self, index: usize) -> Result<PathBuf> {
        if self.files.is_empty() {
            bail!("No presets in the preset directory");
        }
        self.current = Some(index);
        Ok(self.files[index].clone())
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
        let name = |p: PathBuf| p.file_name().unwrap().to_str().unwrap().to_string();
        assert_eq!(name(list.next().unwrap()), "a.MILK");
        assert_eq!(name(list.next().unwrap()), "b.milk");
        assert_eq!(name(list.previous().unwrap()), "a.MILK");
        assert_eq!(name(list.previous().unwrap()), "c.milk2");
        assert_eq!(name(list.next().unwrap()), "a.MILK");
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
    fn empty_list_is_an_error() {
        assert!(PresetList::default().next().is_err());
    }
}
