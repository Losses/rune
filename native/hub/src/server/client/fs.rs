use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct VirtualEntry {
    pub name: String,
    pub id: Option<i32>,
    pub is_directory: bool,
}

pub struct VirtualFS {
    pub current_path: PathBuf,
    pub root_dirs: Vec<String>,
    pub subdirs: HashMap<String, Vec<VirtualEntry>>,
}

impl VirtualFS {
    pub fn new() -> Self {
        let root_dirs = vec![
            "Artists".to_string(),
            "Playlists".to_string(),
            "Tracks".to_string(),
            "Albums".to_string(),
            "Mixes".to_string(),
        ];

        Self {
            current_path: PathBuf::from("/"),
            root_dirs,
            subdirs: HashMap::new(),
        }
    }

    pub fn current_dir(&self) -> String {
        self.current_path.to_string_lossy().to_string()
    }

    pub async fn list_current_dir(&self) -> Vec<VirtualEntry> {
        if self.current_path == Path::new("/") {
            return self
                .root_dirs
                .iter()
                .map(|name| VirtualEntry {
                    name: name.clone(),
                    id: None,
                    is_directory: true,
                })
                .collect();
        }

        let current_dir = self
            .current_path
            .components()
            .last()
            .unwrap()
            .as_os_str()
            .to_string_lossy()
            .to_string();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        self.subdirs
            .get(&current_dir)
            .cloned()
            .unwrap_or_else(std::vec::Vec::new)
    }
}
