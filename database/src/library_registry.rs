use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use fsio::FsIo;
use log::warn;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::connection::StorageMode;

/// A node's record of a library and its local storage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryDescriptor {
    pub library_id: Uuid,
    pub alias: String,
    pub authority: LibraryAuthority,
    pub storage: LibraryStorage,
    pub db_placement: DbPlacement,
}
/// The authority responsible for a library.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LibraryAuthority {
    Owned,
}
/// The library's media storage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LibraryStorage {
    Local { path: String },
}
/// The location of the database relative to the library.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DbPlacement {
    Portable,
    Redirected { storage_key: Uuid },
}

#[derive(Serialize, Deserialize)]
struct RegistryFile {
    version: u32,
    libraries: Vec<LibraryDescriptor>,
}
#[derive(Serialize, Deserialize)]
struct PortableFile {
    version: u32,
    library_id: Uuid,
    alias: String,
}
#[derive(Deserialize)]
struct Version {
    version: u32,
}

fn check_version(data: &[u8]) -> Result<()> {
    let version: Version = serde_json::from_slice(data).context("Invalid library file version")?;
    if version.version > 1 {
        bail!("Unsupported library file version {}", version.version);
    }
    Ok(())
}

/// The libraries known to this node, stored under its configuration directory.
pub struct LibraryRegistry {
    pub libraries: Vec<LibraryDescriptor>,
    path: PathBuf,
}
impl LibraryRegistry {
    /// Load the registry, treating a missing file as empty.
    pub fn load(config_dir: &Path) -> Result<Self> {
        let path = config_dir.join("libraries.json");
        let libraries = match fs::read(&path) {
            Ok(data) => {
                check_version(&data)?;
                let file: RegistryFile =
                    serde_json::from_slice(&data).context("Invalid library registry")?;
                file.libraries
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e).context("Failed to read library registry"),
        };
        Ok(Self { libraries, path })
    }
    /// Atomically replace the registry file.
    pub fn save(&self) -> Result<()> {
        let dir = self
            .path
            .parent()
            .context("Registry has no parent directory")?;
        fs::create_dir_all(dir).context("Failed to create registry directory")?;
        match fs::read(&self.path) {
            Ok(data) => check_version(&data)?,
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("Failed to read library registry before saving"),
        }
        let temp = dir.join(format!(".libraries-{}.tmp", Uuid::new_v4()));
        let data = serde_json::to_vec_pretty(&RegistryFile {
            version: 1,
            libraries: self.libraries.clone(),
        })?;
        let result = (|| -> Result<()> {
            fs::write(&temp, data).context("Failed to write temporary registry")?;
            fs::rename(&temp, &self.path).context("Failed to replace registry")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}

/// Resolve a local library's identity and update the node registry.
pub async fn resolve_library(
    fsio: &FsIo,
    registry: &mut LibraryRegistry,
    root: &str,
    alias: &str,
    mode: StorageMode,
) -> Result<LibraryDescriptor> {
    #[cfg(target_os = "android")]
    let rune_dir = PathBuf::from(".rune");
    #[cfg(not(target_os = "android"))]
    let rune_dir = Path::new(root).join(".rune");
    let descriptor_path = rune_dir.join("library.json");
    let mut portable = None;
    let (id, placement) = match mode {
        StorageMode::Portable => {
            let file = if fsio.exists(&descriptor_path)? {
                let data = fsio
                    .read(&descriptor_path)
                    .context("Failed to read library descriptor")?;
                check_version(&data)?;
                serde_json::from_slice::<PortableFile>(&data)
                    .context("Invalid library descriptor")?
            } else {
                PortableFile {
                    version: 1,
                    library_id: Uuid::new_v4(),
                    alias: String::new(),
                }
            };
            let id = file.library_id;
            portable = Some(file);
            (id, DbPlacement::Portable)
        }
        StorageMode::Redirected(key) => {
            let id = registry
                .libraries
                .iter()
                .find(|entry| entry.db_placement == DbPlacement::Redirected { storage_key: key })
                .map(|entry| entry.library_id)
                .unwrap_or_else(Uuid::new_v4);
            (id, DbPlacement::Redirected { storage_key: key })
        }
    };
    let stored_alias = registry
        .libraries
        .iter()
        .find(|entry| entry.library_id == id)
        .map(|entry| entry.alias.as_str())
        .or_else(|| portable.as_ref().map(|file| file.alias.as_str()))
        .unwrap_or_default();
    let alias = if alias.is_empty() {
        stored_alias
    } else {
        alias
    }
    .to_owned();
    if let Some(mut file) = portable {
        file.alias.clone_from(&alias);
        fsio.create_dir_all(&rune_dir)
            .context("Failed to create library directory")?;
        fsio.write(&descriptor_path, &serde_json::to_vec_pretty(&file)?)
            .await
            .context("Failed to write library descriptor")?;
    }
    let descriptor = LibraryDescriptor {
        library_id: id,
        alias,
        authority: LibraryAuthority::Owned,
        storage: LibraryStorage::Local {
            path: root.to_owned(),
        },
        db_placement: placement,
    };
    registry.libraries.retain(|entry| {
        let collision = entry.library_id != id && entry.storage == descriptor.storage;
        if collision {
            warn!(
                "Removing stale library {} for path {}",
                entry.library_id, root
            );
        }
        entry.library_id != id && !collision
    });
    registry.libraries.push(descriptor.clone());
    registry.save()?;
    Ok(descriptor)
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    struct Fixture {
        dir: TempDir,
        fsio: FsIo,
        registry: LibraryRegistry,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let registry = LibraryRegistry::load(dir.path()).unwrap();
            Self {
                dir,
                fsio: FsIo::new(),
                registry,
            }
        }
        fn root(&self, name: &str) -> String {
            let path = self.dir.path().join(name);
            fs::create_dir_all(path.join(".rune")).unwrap();
            path.to_str().unwrap().to_owned()
        }
        async fn resolve(
            &mut self,
            root: &str,
            alias: &str,
            mode: StorageMode,
        ) -> LibraryDescriptor {
            resolve_library(&self.fsio, &mut self.registry, root, alias, mode)
                .await
                .unwrap()
        }
    }
    #[tokio::test]
    async fn portable_reopen_move_and_distinct() {
        let mut f = Fixture::new();
        let root = f.root("one");
        let first = f.resolve(&root, "First", StorageMode::Portable).await;
        assert!(Path::new(&root).join(".rune/library.json").exists());
        let second = f.resolve(&root, "", StorageMode::Portable).await;
        assert_eq!(first.library_id, second.library_id);
        assert_eq!(second.alias, "First");
        let moved = f.dir.path().join("moved");
        fs::rename(&root, &moved).unwrap();
        let moved = moved.to_str().unwrap();
        let third = f.resolve(moved, "Renamed", StorageMode::Portable).await;
        assert_eq!(first.library_id, third.library_id);
        assert_eq!(
            third.storage,
            LibraryStorage::Local {
                path: moved.to_owned()
            }
        );
        let other = f.root("other");
        let fourth = f.resolve(&other, "Other", StorageMode::Portable).await;
        assert_ne!(first.library_id, fourth.library_id);
        let loaded = LibraryRegistry::load(f.dir.path()).unwrap();
        assert_eq!(loaded.libraries.len(), 2);
        assert!(loaded.libraries.contains(&third));
    }
    #[tokio::test]
    async fn redirected_reopen() {
        let mut f = Fixture::new();
        let root = f.root("one");
        let key = Uuid::new_v4();
        let first = f
            .resolve(&root, "First", StorageMode::Redirected(key))
            .await;
        f.registry = LibraryRegistry::load(f.dir.path()).unwrap();
        let second = f.resolve(&root, "", StorageMode::Redirected(key)).await;
        assert_eq!(first.library_id, second.library_id);
        assert!(!Path::new(&root).join(".rune/library.json").exists());
    }
    #[tokio::test]
    async fn newer_versions_are_untouched() {
        let mut f = Fixture::new();
        let registry_path = f.dir.path().join("libraries.json");
        let content = r#"{"version":2,"libraries":[]}"#;
        fs::write(&registry_path, content).unwrap();
        assert!(LibraryRegistry::load(f.dir.path()).is_err());
        assert_eq!(fs::read_to_string(&registry_path).unwrap(), content);
        fs::remove_file(registry_path).unwrap();
        let root = f.root("one");
        let descriptor_path = Path::new(&root).join(".rune/library.json");
        let content = format!(
            r#"{{"version":2,"library_id":"{}","alias":"x"}}"#,
            Uuid::new_v4()
        );
        fs::write(&descriptor_path, &content).unwrap();
        assert!(
            resolve_library(&f.fsio, &mut f.registry, &root, "", StorageMode::Portable)
                .await
                .is_err()
        );
        assert_eq!(fs::read_to_string(descriptor_path).unwrap(), content);
    }
    #[tokio::test]
    async fn path_collision_replaces_stale_entry() {
        let mut f = Fixture::new();
        let root = f.root("one");
        let old = f
            .resolve(&root, "Old", StorageMode::Redirected(Uuid::new_v4()))
            .await;
        let current = f.resolve(&root, "New", StorageMode::Portable).await;
        assert_ne!(old.library_id, current.library_id);
        assert_eq!(f.registry.libraries, vec![current]);
    }
}
