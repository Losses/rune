use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
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

/// A stable, node-local identity for one registry entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LibraryRecordId(pub Uuid);

/// A node's record of one library or endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryRecord {
    pub record_id: LibraryRecordId,
    pub alias: String,
    pub kind: LibraryKind,
}

/// The endpoint represented by a registry record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LibraryKind {
    Owned {
        library_id: Uuid,
        storage: OwnedStorage,
        #[serde(default = "background_sync_default")]
        background_sync: bool,
    },
    RuneReplica {
        library_id: Uuid,
        owner_fingerprint: String,
        #[serde(default)]
        access_level: Option<LibraryAccess>,
        #[serde(default)]
        connection_access: Option<LibraryConnectionAccess>,
        #[serde(default = "background_sync_default")]
        background_sync: bool,
        // Hosted local libraries existed in v1 as well as config-local replicas.
        storage: ReplicaStorage,
    },
    RuneRemoteControl {
        fingerprint: String,
    },
    Subsonic {
        endpoint: String,
        account: String,
        mode: SubsonicOpenMode,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OwnedStorage {
    Local {
        path: String,
        placement: DbPlacement,
    },
}

/// Storage retained for a Rune replica. `Local` preserves the legacy hosted-plus-local case.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReplicaStorage {
    Local {
        path: String,
        placement: DbPlacement,
    },
    Replica,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubsonicOpenMode {
    RemoteControl,
    PlayOnThisDevice,
}

/// Access granted by the authority of a hosted library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibraryAccess {
    NotShared,
    Read,
    ReadWrite,
}
/// Connection access advertised by the authority of a hosted library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibraryConnectionAccess {
    None,
    View,
    ViewEdit,
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

impl LibraryRecord {
    pub fn catalog_library_id(&self) -> Option<Uuid> {
        match self.kind {
            LibraryKind::Owned { library_id, .. } | LibraryKind::RuneReplica { library_id, .. } => {
                Some(library_id)
            }
            LibraryKind::RuneRemoteControl { .. } | LibraryKind::Subsonic { .. } => None,
        }
    }

    /// The library's folder, computed against the current configuration directory.
    pub fn root(&self, config_dir: &Path) -> Option<PathBuf> {
        match &self.kind {
            LibraryKind::Owned {
                storage: OwnedStorage::Local { path, .. },
                ..
            }
            | LibraryKind::RuneReplica {
                storage: ReplicaStorage::Local { path, .. },
                ..
            } => Some(PathBuf::from(path)),
            LibraryKind::RuneReplica {
                library_id,
                storage: ReplicaStorage::Replica,
                ..
            } => Some(replica_root(config_dir, *library_id)),
            LibraryKind::RuneRemoteControl { .. } | LibraryKind::Subsonic { .. } => None,
        }
    }

    /// The name the app uses to refer to this library.
    pub fn app_name(&self) -> Option<String> {
        match &self.kind {
            LibraryKind::Owned {
                storage: OwnedStorage::Local { path, .. },
                ..
            }
            | LibraryKind::RuneReplica {
                storage: ReplicaStorage::Local { path, .. },
                ..
            } => Some(path.clone()),
            LibraryKind::RuneReplica {
                library_id,
                storage: ReplicaStorage::Replica,
                ..
            } => Some(format!("{REPLICA_SCHEME}{library_id}")),
            LibraryKind::RuneRemoteControl { .. } | LibraryKind::Subsonic { .. } => None,
        }
    }

    pub fn background_sync(&self) -> bool {
        match &self.kind {
            LibraryKind::Owned {
                background_sync, ..
            }
            | LibraryKind::RuneReplica {
                background_sync, ..
            } => *background_sync,
            _ => false,
        }
    }

    pub fn owner_fingerprint(&self) -> Option<&str> {
        match &self.kind {
            LibraryKind::RuneReplica {
                owner_fingerprint, ..
            } => Some(owner_fingerprint),
            _ => None,
        }
    }

    pub fn access_level(&self) -> Option<LibraryAccess> {
        match &self.kind {
            LibraryKind::RuneReplica { access_level, .. } => *access_level,
            _ => None,
        }
    }

    pub fn connection_access(&self) -> Option<LibraryConnectionAccess> {
        match &self.kind {
            LibraryKind::RuneReplica {
                connection_access, ..
            } => *connection_access,
            _ => None,
        }
    }

    pub fn is_owned(&self) -> bool {
        matches!(self.kind, LibraryKind::Owned { .. })
    }

    pub fn db_placement(&self) -> Option<DbPlacement> {
        match &self.kind {
            LibraryKind::Owned {
                storage: OwnedStorage::Local { placement, .. },
                ..
            }
            | LibraryKind::RuneReplica {
                storage: ReplicaStorage::Local { placement, .. },
                ..
            } => Some(placement.clone()),
            LibraryKind::RuneReplica {
                storage: ReplicaStorage::Replica,
                ..
            } => Some(DbPlacement::Portable),
            _ => None,
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
    libraries: Vec<LibraryRecord>,
}
#[derive(Serialize, Deserialize)]
struct PortableFile {
    version: u32,
    library_id: Uuid,
    alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    authority: Option<LegacyAuthority>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LegacyAuthority {
    Owned,
    Hosted { fingerprint: String },
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
    pub libraries: Vec<LibraryRecord>,
    path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
#[error("Library registry reset required: old libraries.json moved to libraries.json.old")]
pub struct LibraryRegistryResetRequired;

fn validate_registry_version(data: &[u8]) -> Result<()> {
    let version: Version = serde_json::from_slice(data).context("Invalid library file version")?;
    match version.version {
        1 => Err(LibraryRegistryResetRequired.into()),
        2 => Ok(()),
        other => bail!("Unsupported library registry version {other}"),
    }
}
impl LibraryRegistry {
    /// Load the registry, moving v1 files aside because their format is ambiguous.
    pub fn load(config_dir: &Path) -> Result<Self> {
        let path = config_dir.join("libraries.json");
        let libraries = match fs::read(&path) {
            Ok(data) => match validate_registry_version(&data) {
                Ok(()) => {
                    let file: RegistryFile =
                        serde_json::from_slice(&data).context("Invalid library registry")?;
                    file.libraries
                }
                Err(error)
                    if error
                        .downcast_ref::<LibraryRegistryResetRequired>()
                        .is_some() =>
                {
                    let primary_backup = config_dir.join("libraries.json.old");
                    let old_path = if primary_backup.exists() {
                        config_dir.join(format!("libraries.json.old.{}", Uuid::new_v4()))
                    } else {
                        primary_backup
                    };
                    fs::rename(&path, &old_path)
                        .context("Failed to move old library registry aside")?;
                    warn!(
                        "Old library registry moved to {}; the registry was reset",
                        old_path.display()
                    );
                    Vec::new()
                }
                Err(error) => return Err(error),
            },
            Err(e) if e.kind() == ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e).context("Failed to read library registry"),
        };
        Ok(Self { libraries, path })
    }
    pub fn record(&self, record_id: LibraryRecordId) -> Result<&LibraryRecord> {
        self.libraries
            .iter()
            .find(|record| record.record_id == record_id)
            .context("Library record is not registered on this node")
    }

    pub fn catalog(&self, library_id: Uuid) -> Result<&LibraryRecord> {
        self.libraries
            .iter()
            .find(|record| record.catalog_library_id() == Some(library_id))
            .context("Catalog library is not registered on this node")
    }

    pub fn catalogs(&self) -> impl Iterator<Item = &LibraryRecord> {
        self.libraries
            .iter()
            .filter(|record| record.catalog_library_id().is_some())
    }

    fn catalog_mut(&mut self, library_id: Uuid) -> Result<&mut LibraryRecord> {
        self.libraries
            .iter_mut()
            .find(|record| record.catalog_library_id() == Some(library_id))
            .context("Library is not registered on this node")
    }

    /// Resolve the app's replica:// name.
    pub fn replica(&self, library_id: Uuid) -> Result<&LibraryRecord> {
        let record = self.catalog(library_id)?;
        ensure!(
            matches!(
                record.kind,
                LibraryKind::RuneReplica {
                    storage: ReplicaStorage::Replica,
                    ..
                }
            ),
            "Library is not a config-local replica"
        );
        Ok(record)
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
            let record = registry.catalog_mut(library_id)?;
            match &mut record.kind {
                LibraryKind::Owned {
                    background_sync, ..
                }
                | LibraryKind::RuneReplica {
                    background_sync, ..
                } => *background_sync = enabled,
                _ => bail!("Non-catalog record cannot have a background sync preference"),
            }
            Ok(())
        })
        .await
    }

    /// Remove one node-local record without changing its catalog or storage files.
    pub async fn remove_record(config_dir: &Path, record_id: LibraryRecordId) -> Result<bool> {
        Self::update(config_dir, |registry| {
            let Some(index) = registry
                .libraries
                .iter()
                .position(|record| record.record_id == record_id)
            else {
                return Ok(false);
            };
            registry.libraries.remove(index);
            Ok(true)
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
                matches!(&library.kind, LibraryKind::RuneReplica { library_id: id, owner_fingerprint, .. } if *id == library_id && owner_fingerprint == fingerprint)
            }) else {
                return Ok(false);
            };
            let LibraryKind::RuneReplica { access_level, .. } = &mut library.kind else { unreachable!() };
            let changed = *access_level != Some(level);
            *access_level = Some(level);
            Ok(changed)
        })
        .await
    }

    /// Record an authority's current connection grant on its replica.
    pub async fn set_connection_access(
        config_dir: &Path,
        library_id: Uuid,
        fingerprint: &str,
        access: LibraryConnectionAccess,
    ) -> Result<bool> {
        Self::update(config_dir, |registry| {
            let Some(library) = registry.libraries.iter_mut().find(|library| {
                matches!(&library.kind, LibraryKind::RuneReplica { library_id: id, owner_fingerprint, .. } if *id == library_id && owner_fingerprint == fingerprint)
            }) else {
                return Ok(false);
            };
            let LibraryKind::RuneReplica { connection_access, .. } = &mut library.kind else { unreachable!() };
            let changed = *connection_access != Some(access);
            *connection_access = Some(access);
            Ok(changed)
        })
        .await
    }

    /// Register a trusted Rune peer as a remote-control endpoint.
    pub async fn register_remote_control(
        config_dir: &Path,
        fingerprint: &str,
        alias: &str,
    ) -> Result<LibraryRecord> {
        Self::update(config_dir, |registry| {
            if let Some(record) = registry.libraries.iter().find(|record| {
                matches!(&record.kind, LibraryKind::RuneRemoteControl { fingerprint: stored } if stored == fingerprint)
            }) {
                return Ok(record.clone());
            }
            let record = LibraryRecord {
                record_id: LibraryRecordId(Uuid::new_v4()),
                alias: alias.to_owned(),
                kind: LibraryKind::RuneRemoteControl { fingerprint: fingerprint.to_owned() },
            };
            registry.libraries.push(record.clone());
            Ok(record)
        }).await
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
            Ok(data) => {
                let version: Version =
                    serde_json::from_slice(&data).context("Invalid library file version")?;
                if version.version == 1 {
                    bail!(LibraryRegistryResetRequired);
                }
                ensure!(
                    version.version == 2,
                    "Unsupported library registry version {}",
                    version.version
                );
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("Failed to read library registry before saving"),
        }
        let temp = dir.join(format!(".libraries-{}.tmp", Uuid::new_v4()));
        let data = serde_json::to_vec_pretty(&RegistryFile {
            version: 2,
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
) -> Result<LibraryRecord> {
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
                .find(|entry| {
                    entry.db_placement() == Some(DbPlacement::Redirected { storage_key: key })
                })
                .and_then(LibraryRecord::catalog_library_id)
                .unwrap_or_else(Uuid::new_v4);
            (id, DbPlacement::Redirected { storage_key: key })
        }
    };
    let authority = portable
        .as_ref()
        .map(|file| file.authority.clone().unwrap_or(LegacyAuthority::Owned))
        .or_else(|| {
            registry
                .libraries
                .iter()
                .find(|entry| entry.catalog_library_id() == Some(id))
                .map(|entry| match &entry.kind {
                    LibraryKind::Owned { .. } => LegacyAuthority::Owned,
                    LibraryKind::RuneReplica {
                        owner_fingerprint, ..
                    } => LegacyAuthority::Hosted {
                        fingerprint: owner_fingerprint.clone(),
                    },
                    _ => LegacyAuthority::Owned,
                })
        })
        .unwrap_or(LegacyAuthority::Owned);
    let stored_alias = registry
        .libraries
        .iter()
        .find(|entry| entry.catalog_library_id() == Some(id))
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
    let existing = registry
        .libraries
        .iter()
        .find(|entry| entry.catalog_library_id() == Some(id));
    // Opening a replica's folder by path must not turn it into a path entry.
    // Opening a replica folder by path must not turn it into a local path record.
    let was_replica = matches!(
        existing.map(|record| &record.kind),
        Some(LibraryKind::RuneReplica {
            storage: ReplicaStorage::Replica,
            ..
        })
    );
    let background_sync = existing.is_none_or(LibraryRecord::background_sync);
    let record_id = existing
        .map(|record| record.record_id)
        .unwrap_or(LibraryRecordId(Uuid::new_v4()));
    let kind = match authority {
        LegacyAuthority::Owned => LibraryKind::Owned {
            library_id: id,
            storage: OwnedStorage::Local {
                path: root.to_owned(),
                placement: placement.clone(),
            },
            background_sync,
        },
        LegacyAuthority::Hosted { fingerprint } => LibraryKind::RuneReplica {
            library_id: id,
            owner_fingerprint: fingerprint,
            access_level: match existing.map(|record| &record.kind) {
                Some(LibraryKind::RuneReplica { access_level, .. }) => *access_level,
                _ => None,
            },
            connection_access: match existing.map(|record| &record.kind) {
                Some(LibraryKind::RuneReplica {
                    connection_access, ..
                }) => *connection_access,
                _ => None,
            },
            background_sync,
            storage: if was_replica {
                ReplicaStorage::Replica
            } else {
                ReplicaStorage::Local {
                    path: root.to_owned(),
                    placement: placement.clone(),
                }
            },
        },
    };
    let descriptor = LibraryRecord {
        record_id,
        alias,
        kind,
    };
    registry.libraries.retain(|entry| {
        let collision = entry.catalog_library_id() != Some(id)
            && descriptor.root(Path::new("")) == entry.root(Path::new(""));
        if collision {
            warn!(
                "Removing stale library {} for path {}",
                entry.catalog_library_id().unwrap_or_default(),
                root
            );
        }
        entry.catalog_library_id() != Some(id) && !collision
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
) -> Result<LibraryRecord> {
    let _guard = REGISTRY_WRITE.lock().await;
    registry.reload()?;
    if let Some(existing) = registry
        .libraries
        .iter()
        .find(|entry| entry.catalog_library_id() == Some(library_id))
    {
        if matches!(existing.kind, LibraryKind::Owned { .. }) {
            bail!("Cannot create a replica of a library this node owns");
        }
        if !matches!(&existing.kind, LibraryKind::RuneReplica { owner_fingerprint, .. } if owner_fingerprint == authority_fingerprint)
        {
            bail!("Library already has a different authority");
        }
        return Ok(existing.clone());
    }
    let root = replica_root(config_dir, library_id);
    let rune_dir = root.join(".rune");
    let path = rune_dir.join("library.json");
    let authority = LegacyAuthority::Hosted {
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
    let descriptor = LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: file.alias,
        kind: LibraryKind::RuneReplica {
            library_id,
            owner_fingerprint: authority_fingerprint.to_owned(),
            storage: ReplicaStorage::Replica,
            background_sync: true,
            access_level: None,
            connection_access: None,
        },
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
        async fn resolve(&mut self, root: &str, alias: &str, mode: StorageMode) -> LibraryRecord {
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
        data["libraries"][0]["kind"]
            .as_object_mut()
            .unwrap()
            .remove("background_sync");
        data["libraries"][0]["kind"]
            .as_object_mut()
            .unwrap()
            .remove("access_level");
        fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
        let registry = LibraryRegistry::load(f.dir.path()).unwrap();
        let descriptor = serde_json::to_value(&registry.libraries[0]).unwrap();
        assert_eq!(descriptor["kind"]["background_sync"], true);
        assert_eq!(descriptor["kind"]["access_level"], serde_json::Value::Null);
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
                let path = library.root(&config).unwrap().to_str().unwrap().to_owned();
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
                    .any(|library| library.catalog_library_id() == Some(id)
                        && !library.background_sync())
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
        assert!(
            matches!(&first.kind, LibraryKind::RuneReplica { owner_fingerprint, storage: ReplicaStorage::Replica, .. } if owner_fingerprint == "owner")
        );
        assert_eq!(first.db_placement(), Some(DbPlacement::Portable));
        let root = first.root(f.dir.path()).unwrap();
        assert_eq!(root, f.dir.path().join("replicas").join(id.to_string()));
        let path = root.to_str().unwrap();
        let data = fs::read(root.join(".rune/library.json")).unwrap();
        let file: PortableFile = serde_json::from_slice(&data).unwrap();
        assert_eq!(file.version, 1);
        assert_eq!(
            file.authority,
            Some(LegacyAuthority::Hosted {
                fingerprint: "owner".into()
            })
        );
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
        assert_eq!(recovered.kind, first.kind);
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
        assert!(matches!(owned.kind, LibraryKind::Owned { .. }));
        assert!(
            create_replica(
                f.dir.path(),
                &mut f.registry,
                owned.catalog_library_id().unwrap(),
                "",
                "peer"
            )
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
        assert!(matches!(owned.kind, LibraryKind::Owned { .. }));
        assert_eq!(owned.catalog_library_id(), Some(id));
        f.registry.libraries[0].kind = LibraryKind::RuneReplica {
            library_id: id,
            owner_fingerprint: "owner".into(),
            storage: ReplicaStorage::Local {
                path: root.clone(),
                placement: DbPlacement::Portable,
            },
            background_sync: true,
            access_level: Some(LibraryAccess::Read),
            connection_access: None,
        };
        f.registry.save().unwrap();
        let reopened_owned = f.resolve(&root, "", StorageMode::Portable).await;
        assert!(matches!(reopened_owned.kind, LibraryKind::Owned { .. }));
        let key = Uuid::new_v4();
        f.resolve(&root, "", StorageMode::Redirected(key)).await;
        let authority = LibraryKind::RuneReplica {
            library_id: id,
            owner_fingerprint: "owner".into(),
            storage: ReplicaStorage::Local {
                path: root.clone(),
                placement: DbPlacement::Redirected { storage_key: key },
            },
            background_sync: true,
            access_level: Some(LibraryAccess::Read),
            connection_access: None,
        };
        f.registry.libraries[0].kind = authority.clone();
        f.registry.save().unwrap();
        f.registry = LibraryRegistry::load(f.dir.path()).unwrap();
        let reopened = f.resolve(&root, "", StorageMode::Redirected(key)).await;
        assert_eq!(reopened.kind, authority);

        let hosted_root = f.root("hosted_local");
        let hosted_id = Uuid::new_v4();
        let hosted_authority = LegacyAuthority::Hosted {
            fingerprint: "remote-owner".into(),
        };
        fs::write(
            Path::new(&hosted_root).join(".rune/library.json"),
            serde_json::to_vec(&PortableFile {
                version: 1,
                library_id: hosted_id,
                alias: "Hosted local".into(),
                authority: Some(hosted_authority),
            })
            .unwrap(),
        )
        .unwrap();
        let hosted = f.resolve(&hosted_root, "", StorageMode::Portable).await;
        assert!(matches!(
            &hosted.kind,
            LibraryKind::RuneReplica {
                storage: ReplicaStorage::Local { path, placement: DbPlacement::Portable },
                ..
            } if path == &hosted_root
        ));
        assert_eq!(hosted.root(f.dir.path()), Some(PathBuf::from(hosted_root)));
    }

    #[tokio::test]
    async fn portable_reopen_move_and_distinct() {
        let mut f = Fixture::new();
        let root = f.root("one");
        let first = f.resolve(&root, "First", StorageMode::Portable).await;
        assert!(Path::new(&root).join(".rune/library.json").exists());
        let second = f.resolve(&root, "", StorageMode::Portable).await;
        assert_eq!(first.catalog_library_id(), second.catalog_library_id());
        assert_eq!(second.alias, "First");
        let moved = f.dir.path().join("moved");
        fs::rename(&root, &moved).unwrap();
        let moved = moved.to_str().unwrap();
        let third = f.resolve(moved, "Renamed", StorageMode::Portable).await;
        assert_eq!(first.catalog_library_id(), third.catalog_library_id());
        assert!(
            matches!(&third.kind, LibraryKind::Owned { storage: OwnedStorage::Local { path, .. }, .. } if path == moved)
        );
        let other = f.root("other");
        let fourth = f.resolve(&other, "Other", StorageMode::Portable).await;
        assert_ne!(first.catalog_library_id(), fourth.catalog_library_id());
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
        assert_eq!(first.catalog_library_id(), second.catalog_library_id());
        assert!(!Path::new(&root).join(".rune/library.json").exists());
    }
    #[tokio::test]
    async fn newer_versions_are_untouched() {
        let mut f = Fixture::new();
        let registry_path = f.dir.path().join("libraries.json");
        let content = r#"{"version":3,"libraries":[]}"#;
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
        assert_ne!(old.catalog_library_id(), current.catalog_library_id());
        assert_eq!(f.registry.libraries, vec![current]);
    }

    #[tokio::test]
    async fn replica_follows_a_moved_config_dir() {
        let mut f = Fixture::new();
        let id = Uuid::new_v4();
        let replica = create_replica(f.dir.path(), &mut f.registry, id, "Remote", "owner")
            .await
            .unwrap();
        assert!(matches!(
            replica.kind,
            LibraryKind::RuneReplica {
                storage: ReplicaStorage::Replica,
                ..
            }
        ));
        assert_eq!(
            replica.root(f.dir.path()).unwrap(),
            f.dir.path().join("replicas").join(id.to_string())
        );
        assert_eq!(replica.app_name().unwrap(), format!("replica://{id}"));
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
        assert_eq!(loaded.root(moved.path()), Some(to));
        assert!(
            loaded
                .root(moved.path())
                .unwrap()
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
        let root = replica.root(f.dir.path()).unwrap();
        let resolved = f
            .resolve(root.to_str().unwrap(), "", StorageMode::Portable)
            .await;
        assert_eq!(resolved.catalog_library_id(), Some(id));
        assert!(matches!(
            resolved.kind,
            LibraryKind::RuneReplica {
                storage: ReplicaStorage::Replica,
                ..
            }
        ));
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

    #[test]
    fn every_record_kind_round_trips_through_serde() {
        let library_id = Uuid::new_v4();
        let records = [
            LibraryRecord {
                record_id: LibraryRecordId(Uuid::new_v4()),
                alias: "Owned".into(),
                kind: LibraryKind::Owned {
                    library_id,
                    storage: OwnedStorage::Local {
                        path: "/music".into(),
                        placement: DbPlacement::Redirected {
                            storage_key: Uuid::new_v4(),
                        },
                    },
                    background_sync: true,
                },
            },
            LibraryRecord {
                record_id: LibraryRecordId(Uuid::new_v4()),
                alias: "Replica".into(),
                kind: LibraryKind::RuneReplica {
                    library_id,
                    owner_fingerprint: "owner-fingerprint".into(),
                    access_level: Some(LibraryAccess::ReadWrite),
                    connection_access: Some(LibraryConnectionAccess::ViewEdit),
                    background_sync: false,
                    storage: ReplicaStorage::Replica,
                },
            },
            LibraryRecord {
                record_id: LibraryRecordId(Uuid::new_v4()),
                alias: "Control".into(),
                kind: LibraryKind::RuneRemoteControl {
                    fingerprint: "remote-fingerprint".into(),
                },
            },
            LibraryRecord {
                record_id: LibraryRecordId(Uuid::new_v4()),
                alias: "Subsonic".into(),
                kind: LibraryKind::Subsonic {
                    endpoint: "https://music.example/base".into(),
                    account: "alice".into(),
                    mode: SubsonicOpenMode::PlayOnThisDevice,
                },
            },
        ];
        let expected_types = ["owned", "rune_replica", "rune_remote_control", "subsonic"];
        for (record, expected_type) in records.into_iter().zip(expected_types) {
            let json = serde_json::to_vec(&record).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
            assert_eq!(value["kind"]["type"], expected_type);
            assert_eq!(
                serde_json::from_slice::<LibraryRecord>(&json).unwrap(),
                record
            );
        }
    }

    #[test]
    fn legacy_registry_is_typed_reset_and_moved_aside() {
        let old = br#"{"version":1,"libraries":[]}"#;
        let error = validate_registry_version(old).unwrap_err();
        assert!(
            error
                .downcast_ref::<LibraryRegistryResetRequired>()
                .is_some()
        );

        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("libraries.json"), old).unwrap();
        let registry = LibraryRegistry::load(dir.path()).unwrap();
        assert!(registry.libraries.is_empty());
        assert!(!dir.path().join("libraries.json").exists());
        assert_eq!(
            fs::read(dir.path().join("libraries.json.old")).unwrap(),
            old
        );
    }

    #[test]
    fn non_catalog_records_are_not_catalogs_and_have_no_catalog_paths() {
        let registry = LibraryRegistry {
            libraries: vec![
                LibraryRecord {
                    record_id: LibraryRecordId(Uuid::new_v4()),
                    alias: "Remote control".into(),
                    kind: LibraryKind::RuneRemoteControl {
                        fingerprint: "peer".into(),
                    },
                },
                LibraryRecord {
                    record_id: LibraryRecordId(Uuid::new_v4()),
                    alias: "Subsonic".into(),
                    kind: LibraryKind::Subsonic {
                        endpoint: "https://music.example".into(),
                        account: "alice".into(),
                        mode: SubsonicOpenMode::RemoteControl,
                    },
                },
            ],
            path: PathBuf::new(),
        };
        assert_eq!(registry.catalogs().count(), 0);
        for record in &registry.libraries {
            assert_eq!(record.catalog_library_id(), None);
            assert_eq!(record.root(Path::new("/config")), None);
            assert_eq!(record.app_name(), None);
        }
    }
}
