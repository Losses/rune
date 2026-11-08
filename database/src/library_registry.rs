use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use fsio::FsIo;
use log::warn;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::connection::StorageMode;

static REGISTRY_WRITE: Mutex<()> = Mutex::const_new(());

fn background_sync_default() -> bool {
    true
}

/// A node's record of a library and its local storage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryDescriptor {
    pub library_id: Uuid,
    pub alias: String,
    pub authority: LibraryAuthority,
    pub storage: LibraryStorage,
    pub db_placement: DbPlacement,
    #[serde(default = "background_sync_default")]
    pub background_sync: bool,
    #[serde(default)]
    pub access_level: Option<LibraryAccess>,
}

/// Access granted by the authority of a hosted library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibraryAccess {
    NotShared,
    Read,
    ReadWrite,
}
/// The authority responsible for a library.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LibraryAuthority {
    Owned,
    Hosted { fingerprint: String },
}
/// The library's media storage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LibraryStorage {
    Local {
        path: String,
    },
    /// A replica kept in this node's data directory and named by its library id. The data
    /// directory can move (an iOS reinstall), so its absolute path is never stored.
    Replica,
}

/// Scheme of the name the app uses for a replica instead of a folder path.
pub const REPLICA_SCHEME: &str = "replica://";

/// What a library name given by the app refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryLocation {
    /// A folder the user chose, by its path.
    Path(String),
    /// A replica in this node's data directory, by its library id.
    Replica(Uuid),
}
impl LibraryLocation {
    /// Parse an app-facing library name. A malformed replica name is an error rather than a path.
    pub fn parse(name: &str) -> Result<Self> {
        match name.strip_prefix(REPLICA_SCHEME) {
            None => Ok(Self::Path(name.to_owned())),
            Some(id) => Ok(Self::Replica(
                Uuid::parse_str(id).context("Invalid replica library name")?,
            )),
        }
    }
}

fn replica_root(config_dir: &Path, library_id: Uuid) -> PathBuf {
    config_dir.join("replicas").join(library_id.to_string())
}

impl LibraryDescriptor {
    /// The library's folder, computed against the current configuration directory.
    pub fn root(&self, config_dir: &Path) -> PathBuf {
        match &self.storage {
            LibraryStorage::Local { path } => PathBuf::from(path),
            LibraryStorage::Replica => replica_root(config_dir, self.library_id),
        }
    }

    /// The name the app uses to refer to this library.
    pub fn app_name(&self) -> String {
        match &self.storage {
            LibraryStorage::Local { path } => path.clone(),
            LibraryStorage::Replica => format!("{REPLICA_SCHEME}{}", self.library_id),
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    authority: Option<LibraryAuthority>,
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
    /// The registered replica with this id, for a library name given by the app.
    pub fn replica(&self, library_id: Uuid) -> Result<&LibraryDescriptor> {
        self.libraries
            .iter()
            .find(|library| {
                library.library_id == library_id && library.storage == LibraryStorage::Replica
            })
            .context("Replica library is not registered on this node")
    }
    /// Load, modify and persist a registry under the process-wide writer lock.
    pub async fn update<T>(
        config_dir: &Path,
        change: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let _guard = REGISTRY_WRITE.lock().await;
        let mut registry = Self::load(config_dir)?;
        let before = registry.libraries.clone();
        let result = change(&mut registry)?;
        if registry.libraries != before {
            registry.save()?;
        }
        Ok(result)
    }

    /// Persist a library's device-local background sync preference.
    pub async fn set_background_sync(
        config_dir: &Path,
        library_id: Uuid,
        enabled: bool,
    ) -> Result<()> {
        Self::update(config_dir, |registry| {
            let library = registry
                .libraries
                .iter_mut()
                .find(|library| library.library_id == library_id)
                .context("Library is not registered on this node")?;
            library.background_sync = enabled;
            Ok(())
        })
        .await
    }

    /// Record an authority's current grant on its replica.
    pub async fn set_access_level(
        config_dir: &Path,
        library_id: Uuid,
        fingerprint: &str,
        level: LibraryAccess,
    ) -> Result<bool> {
        Self::update(config_dir, |registry| {
            let Some(library) = registry.libraries.iter_mut().find(|library| {
                library.library_id == library_id
                    && library.authority
                        == (LibraryAuthority::Hosted {
                            fingerprint: fingerprint.to_owned(),
                        })
            }) else {
                return Ok(false);
            };
            let changed = library.access_level != Some(level);
            library.access_level = Some(level);
            Ok(changed)
        })
        .await
    }

    fn reload(&mut self) -> Result<()> {
        *self = Self::load(
            self.path
                .parent()
                .context("Registry has no parent directory")?,
        )?;
        Ok(())
    }

    // Callers hold REGISTRY_WRITE across load, modification and atomic replacement.
    fn save(&self) -> Result<()> {
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
    let _guard = REGISTRY_WRITE.lock().await;
    registry.reload()?;
    let rune_dir = if cfg!(target_os = "android") && root.starts_with("content://") {
        PathBuf::from(".rune")
    } else {
        Path::new(root).join(".rune")
    };
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
                    authority: None,
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
    let authority = portable
        .as_ref()
        .map(|file| file.authority.clone().unwrap_or(LibraryAuthority::Owned))
        .or_else(|| {
            registry
                .libraries
                .iter()
                .find(|entry| entry.library_id == id)
                .map(|entry| entry.authority.clone())
        })
        .unwrap_or(LibraryAuthority::Owned);
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
    let access_level = if authority == LibraryAuthority::Owned {
        None
    } else {
        registry
            .libraries
            .iter()
            .find(|entry| entry.library_id == id)
            .and_then(|entry| entry.access_level)
    };
    // Opening a replica's folder by path must not turn it into a path entry.
    let storage = match registry
        .libraries
        .iter()
        .find(|entry| entry.library_id == id)
    {
        Some(entry) if entry.storage == LibraryStorage::Replica => LibraryStorage::Replica,
        _ => LibraryStorage::Local {
            path: root.to_owned(),
        },
    };
    let descriptor = LibraryDescriptor {
        library_id: id,
        alias,
        authority,
        storage,
        db_placement: placement,
        access_level,
        background_sync: registry
            .libraries
            .iter()
            .find(|entry| entry.library_id == id)
            .map(|entry| entry.background_sync)
            .unwrap_or(true),
    };
    registry.libraries.retain(|entry| {
        let collision = entry.library_id != id
            && matches!(descriptor.storage, LibraryStorage::Local { .. })
            && entry.storage == descriptor.storage;
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

/// Register a remote library in a single Portable folder in this node's config directory.
pub async fn create_replica(
    config_dir: &Path,
    registry: &mut LibraryRegistry,
    library_id: Uuid,
    alias: &str,
    authority_fingerprint: &str,
) -> Result<LibraryDescriptor> {
    let _guard = REGISTRY_WRITE.lock().await;
    registry.reload()?;
    if let Some(existing) = registry
        .libraries
        .iter()
        .find(|entry| entry.library_id == library_id)
    {
        if existing.authority == LibraryAuthority::Owned {
            bail!("Cannot create a replica of a library this node owns");
        }
        if existing.authority
            != (LibraryAuthority::Hosted {
                fingerprint: authority_fingerprint.to_owned(),
            })
        {
            bail!("Library already has a different authority");
        }
        return Ok(existing.clone());
    }
    let root = replica_root(config_dir, library_id);
    let rune_dir = root.join(".rune");
    let path = rune_dir.join("library.json");
    let authority = LibraryAuthority::Hosted {
        fingerprint: authority_fingerprint.to_owned(),
    };
    let file = match fs::read(&path) {
        Ok(data) => {
            check_version(&data)?;
            let file: PortableFile =
                serde_json::from_slice(&data).context("Invalid replica descriptor")?;
            if file.library_id != library_id || file.authority.as_ref() != Some(&authority) {
                bail!("Replica folder has a different identity or authority");
            }
            file
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir_all(&rune_dir).context("Failed to create replica directory")?;
            let file = PortableFile {
                version: 1,
                library_id,
                alias: alias.to_owned(),
                authority: Some(authority.clone()),
            };
            fs::write(&path, serde_json::to_vec_pretty(&file)?)
                .context("Failed to write replica descriptor")?;
            file
        }
        Err(error) => return Err(error).context("Failed to read replica descriptor"),
    };
    let descriptor = LibraryDescriptor {
        library_id,
        alias: file.alias,
        authority,
        storage: LibraryStorage::Replica,
        db_placement: DbPlacement::Portable,
        background_sync: true,
        access_level: None,
    };
    registry.libraries.push(descriptor.clone());
    if let Err(error) = registry.save() {
        registry.libraries.pop();
        return Err(error);
    }
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
    async fn background_sync_defaults_on_for_legacy_registry() {
        let mut f = Fixture::new();
        let root = f.root("legacy");
        f.resolve(&root, "Legacy", StorageMode::Portable).await;
        let path = f.dir.path().join("libraries.json");
        let mut data: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        data["libraries"][0]
            .as_object_mut()
            .unwrap()
            .remove("background_sync");
        data["libraries"][0]
            .as_object_mut()
            .unwrap()
            .remove("access_level");
        fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
        let registry = LibraryRegistry::load(f.dir.path()).unwrap();
        let descriptor = serde_json::to_value(&registry.libraries[0]).unwrap();
        assert_eq!(descriptor["background_sync"], true);
        assert_eq!(registry.libraries[0].access_level, None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_registry_writers_keep_all_updates() {
        let dir = TempDir::new().unwrap();
        let mut writers = Vec::new();
        // All writers start with the same stale snapshot, as independent requests can.
        for _ in 0..32 {
            writers.push((LibraryRegistry::load(dir.path()).unwrap(), Uuid::new_v4()));
        }
        let mut tasks = Vec::new();
        for (mut registry, id) in writers {
            let config = dir.path().to_owned();
            tasks.push(tokio::spawn(async move {
                let library = create_replica(&config, &mut registry, id, "Replica", "owner")
                    .await
                    .unwrap();
                LibraryRegistry::set_background_sync(&config, id, false)
                    .await
                    .unwrap();
                let path = library.root(&config).to_str().unwrap().to_owned();
                resolve_library(
                    &FsIo::new(),
                    &mut registry,
                    &path,
                    "Replica",
                    StorageMode::Portable,
                )
                .await
                .unwrap();
                id
            }));
        }
        let mut ids = Vec::new();
        for task in tasks {
            ids.push(task.await.unwrap());
        }
        let registry = LibraryRegistry::load(dir.path()).unwrap();
        assert_eq!(registry.libraries.len(), ids.len());
        for id in ids {
            assert!(
                registry
                    .libraries
                    .iter()
                    .any(|library| library.library_id == id && !library.background_sync)
            );
        }
    }
    #[tokio::test]
    async fn replica_authority_reopen_and_idempotency() {
        let mut f = Fixture::new();
        let id = Uuid::new_v4();
        let first = create_replica(f.dir.path(), &mut f.registry, id, "Remote", "owner")
            .await
            .unwrap();
        assert_eq!(
            first.authority,
            LibraryAuthority::Hosted {
                fingerprint: "owner".into()
            }
        );
        assert_eq!(first.db_placement, DbPlacement::Portable);
        assert_eq!(first.storage, LibraryStorage::Replica);
        let root = first.root(f.dir.path());
        assert_eq!(root, f.dir.path().join("replicas").join(id.to_string()));
        let path = root.to_str().unwrap();
        let data = fs::read(root.join(".rune/library.json")).unwrap();
        let file: PortableFile = serde_json::from_slice(&data).unwrap();
        assert_eq!(file.version, 1);
        assert_eq!(file.authority, Some(first.authority.clone()));
        f.registry = LibraryRegistry::load(f.dir.path()).unwrap();
        let second = create_replica(f.dir.path(), &mut f.registry, id, "Changed", "owner")
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(f.registry.libraries.len(), 1);
        assert_eq!(
            fs::read_dir(f.dir.path().join("replicas")).unwrap().count(),
            1
        );
        let reopened = f.resolve(path, "", StorageMode::Portable).await;
        assert_eq!(reopened, first);
        // A descriptor survives a registry loss or a crash before registration.
        f.registry.libraries.clear();
        f.registry.save().unwrap();
        let recovered = create_replica(f.dir.path(), &mut f.registry, id, "Changed", "owner")
            .await
            .unwrap();
        assert_eq!(recovered, first);
        assert!(
            create_replica(f.dir.path(), &mut f.registry, id, "", "other")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn replica_refuses_owned_library() {
        let mut f = Fixture::new();
        let root = f.root("owned");
        let owned = f.resolve(&root, "Owned", StorageMode::Portable).await;
        assert_eq!(owned.authority, LibraryAuthority::Owned);
        assert!(
            create_replica(f.dir.path(), &mut f.registry, owned.library_id, "", "peer")
                .await
                .is_err()
        );
        assert_eq!(f.registry.libraries, vec![owned]);
        assert!(!f.dir.path().join("replicas").exists());
    }

    #[tokio::test]
    async fn authority_defaults_owned_and_redirected_preserves_registry() {
        let mut f = Fixture::new();
        let root = f.root("old");
        let id = Uuid::new_v4();
        fs::write(
            Path::new(&root).join(".rune/library.json"),
            format!(r#"{{"version":1,"library_id":"{id}","alias":"Old"}}"#),
        )
        .unwrap();
        let owned = f.resolve(&root, "", StorageMode::Portable).await;
        assert_eq!(owned.authority, LibraryAuthority::Owned);
        assert_eq!(owned.library_id, id);
        f.registry.libraries[0].authority = LibraryAuthority::Hosted {
            fingerprint: "owner".into(),
        };
        f.registry.libraries[0].access_level = Some(LibraryAccess::Read);
        f.registry.save().unwrap();
        let reopened_owned = f.resolve(&root, "", StorageMode::Portable).await;
        assert_eq!(reopened_owned.access_level, None);
        let key = Uuid::new_v4();
        f.resolve(&root, "", StorageMode::Redirected(key)).await;
        let authority = LibraryAuthority::Hosted {
            fingerprint: "owner".into(),
        };
        f.registry.libraries[0].authority = authority.clone();
        f.registry.save().unwrap();
        f.registry = LibraryRegistry::load(f.dir.path()).unwrap();
        let reopened = f.resolve(&root, "", StorageMode::Redirected(key)).await;
        assert_eq!(reopened.authority, authority);
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

    #[tokio::test]
    async fn replica_follows_a_moved_config_dir() {
        let mut f = Fixture::new();
        let id = Uuid::new_v4();
        let replica = create_replica(f.dir.path(), &mut f.registry, id, "Remote", "owner")
            .await
            .unwrap();
        assert_eq!(replica.storage, LibraryStorage::Replica);
        assert_eq!(
            replica.root(f.dir.path()),
            f.dir.path().join("replicas").join(id.to_string())
        );
        assert_eq!(replica.app_name(), format!("replica://{id}"));
        let json = fs::read_to_string(f.dir.path().join("libraries.json")).unwrap();
        assert!(!json.contains(f.dir.path().to_str().unwrap()));
        // An iOS reinstall gives the app a new data directory holding the same files.
        let moved = TempDir::new().unwrap();
        fs::copy(
            f.dir.path().join("libraries.json"),
            moved.path().join("libraries.json"),
        )
        .unwrap();
        let from = f.dir.path().join("replicas").join(id.to_string());
        let to = moved.path().join("replicas").join(id.to_string());
        fs::create_dir_all(to.join(".rune")).unwrap();
        fs::copy(
            from.join(".rune/library.json"),
            to.join(".rune/library.json"),
        )
        .unwrap();
        let registry = LibraryRegistry::load(moved.path()).unwrap();
        let loaded = registry.replica(id).unwrap();
        assert_eq!(loaded.root(moved.path()), to);
        assert!(
            loaded
                .root(moved.path())
                .join(".rune/library.json")
                .exists()
        );
    }

    #[tokio::test]
    async fn resolving_a_replica_folder_keeps_replica_storage() {
        let mut f = Fixture::new();
        let id = Uuid::new_v4();
        let other = Uuid::new_v4();
        let replica = create_replica(f.dir.path(), &mut f.registry, id, "Remote", "owner")
            .await
            .unwrap();
        create_replica(f.dir.path(), &mut f.registry, other, "Other", "owner")
            .await
            .unwrap();
        let root = replica.root(f.dir.path());
        let resolved = f
            .resolve(root.to_str().unwrap(), "", StorageMode::Portable)
            .await;
        assert_eq!(resolved.library_id, id);
        assert_eq!(resolved.storage, LibraryStorage::Replica);
        // Replicas do not collide with each other the way path entries do.
        assert_eq!(f.registry.libraries.len(), 2);
    }

    #[test]
    fn library_location_parsing() {
        let id = Uuid::new_v4();
        assert_eq!(
            LibraryLocation::parse(&format!("replica://{id}")).unwrap(),
            LibraryLocation::Replica(id)
        );
        assert_eq!(
            LibraryLocation::parse("/music").unwrap(),
            LibraryLocation::Path("/music".into())
        );
        for bad in ["replica://", "replica://not-a-uuid", "replica:///tmp/x"] {
            assert!(LibraryLocation::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(
            LibraryLocation::parse("replica:/x").unwrap(),
            LibraryLocation::Path("replica:/x".into())
        );
    }
}
