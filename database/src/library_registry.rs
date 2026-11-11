use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use fsio::FsIo;
use log::warn;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, MutexGuard};
use uuid::Uuid;

use crate::connection::StorageMode;
use crate::credentials::{CredentialRef, CredentialStore, CredentialStoreError};

static REGISTRY_WRITE: Mutex<()> = Mutex::const_new(());

/// A stable, node-local identity for one registry entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LibraryRecordId(pub Uuid);

/// A node's record of one library or endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryRecord {
    pub record_id: LibraryRecordId,
    pub alias: String,
    pub library_id: Option<Uuid>,
    pub protocol: ProtocolAdapter,
    pub fs: Vec<FsAdapter>,
    pub catalog: CatalogPlacement,
    pub background_sync: bool,
}

/// How this node talks to the library owner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProtocolAdapter {
    Native,
    RuneForward {
        owner_fingerprint: String,
        mode: RuneForwardMode,
    },
    SubsonicTranslate {
        endpoint: String,
        credential: CredentialRef,
        mode: SubsonicOpenMode,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuneForwardMode {
    Replica {
        access_level: Option<LibraryAccess>,
        connection_access: Option<LibraryConnectionAccess>,
    },
    RemoteControl,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FsAdapter {
    Local {
        path: String,
    },
    RunePeer {
        fingerprint: String,
    },
    WebDav {
        endpoint: String,
        root: String,
        username: String,
        credential: CredentialRef,
    },
    S3 {
        endpoint: String,
        bucket: String,
        region: String,
        root: String,
        access_key_id: String,
        credential: CredentialRef,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "storage_key", rename_all = "snake_case")]
pub enum CatalogPlacement {
    #[default]
    None,
    InMediaRoot,
    Redirected(Uuid),
    InShare(Uuid),
    ConfigReplica,
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
fn replica_root(config_dir: &Path, library_id: Uuid) -> PathBuf {
    config_dir.join("replicas").join(library_id.to_string())
}

fn record_credential(record: &LibraryRecord) -> Option<CredentialRef> {
    match &record.protocol {
        ProtocolAdapter::SubsonicTranslate { credential, .. } => Some(*credential),
        _ => record.fs.iter().find_map(|adapter| match adapter {
            FsAdapter::WebDav { credential, .. } | FsAdapter::S3 { credential, .. } => {
                Some(*credential)
            }
            _ => None,
        }),
    }
}

impl LibraryRecord {
    pub fn catalog_library_id(&self) -> Option<Uuid> {
        self.library_id
    }

    /// A folder path available to UI operations that scan local files.
    pub fn local_folder_path(&self) -> Option<&str> {
        self.fs.iter().find_map(|adapter| match adapter {
            FsAdapter::Local { path } => Some(path.as_str()),
            _ => None,
        })
    }

    /// The library's folder, computed against the current configuration directory.
    pub fn root(&self, config_dir: &Path) -> Option<PathBuf> {
        match (&self.catalog, self.local_folder_path()) {
            (CatalogPlacement::InMediaRoot | CatalogPlacement::Redirected(_), Some(path)) => {
                Some(PathBuf::from(path))
            }
            (CatalogPlacement::ConfigReplica, _) => {
                self.library_id.map(|id| replica_root(config_dir, id))
            }
            (CatalogPlacement::Redirected(_) | CatalogPlacement::InShare(_), None)
                if self.fs.iter().any(|adapter| {
                    matches!(adapter, FsAdapter::WebDav { .. } | FsAdapter::S3 { .. })
                }) =>
            {
                Some(PathBuf::new())
            }
            _ => None,
        }
    }

    /// A display value for logs and task notifications; replicas use their alias.
    pub fn display_name(&self) -> Option<String> {
        match &self.protocol {
            ProtocolAdapter::Native => self.local_folder_path().map(str::to_owned).or_else(|| {
                self.fs
                    .iter()
                    .any(|adapter| {
                        matches!(adapter, FsAdapter::WebDav { .. } | FsAdapter::S3 { .. })
                    })
                    .then(|| self.alias.clone())
            }),
            ProtocolAdapter::RuneForward {
                mode: RuneForwardMode::Replica { .. },
                ..
            } => self
                .local_folder_path()
                .map(str::to_owned)
                .or_else(|| Some(self.alias.clone())),
            _ => None,
        }
    }

    pub fn background_sync(&self) -> bool {
        self.background_sync
    }

    pub fn owner_fingerprint(&self) -> Option<&str> {
        match &self.protocol {
            ProtocolAdapter::RuneForward {
                owner_fingerprint,
                mode: RuneForwardMode::Replica { .. },
            } => Some(owner_fingerprint),
            _ => None,
        }
    }

    pub fn access_level(&self) -> Option<LibraryAccess> {
        match &self.protocol {
            ProtocolAdapter::RuneForward {
                mode: RuneForwardMode::Replica { access_level, .. },
                ..
            } => *access_level,
            _ => None,
        }
    }

    pub fn connection_access(&self) -> Option<LibraryConnectionAccess> {
        match &self.protocol {
            ProtocolAdapter::RuneForward {
                mode:
                    RuneForwardMode::Replica {
                        connection_access, ..
                    },
                ..
            } => *connection_access,
            _ => None,
        }
    }

    pub fn is_owned(&self) -> bool {
        matches!(self.protocol, ProtocolAdapter::Native)
    }

    pub fn db_placement(&self) -> Option<DbPlacement> {
        match self.catalog {
            CatalogPlacement::InMediaRoot | CatalogPlacement::ConfigReplica => {
                Some(DbPlacement::Portable)
            }
            CatalogPlacement::Redirected(storage_key) | CatalogPlacement::InShare(storage_key) => {
                Some(DbPlacement::Redirected { storage_key })
            }
            CatalogPlacement::None => None,
        }
    }

    pub fn validate(&self) -> Result<(), InvalidRecord> {
        match &self.protocol {
            ProtocolAdapter::Native => {
                let open_dal_count = self
                    .fs
                    .iter()
                    .filter(|adapter| {
                        matches!(adapter, FsAdapter::WebDav { .. } | FsAdapter::S3 { .. })
                    })
                    .count();
                if open_dal_count > 0
                    && (open_dal_count != 1
                        || self.fs.len() != 1
                        || !matches!(
                            self.catalog,
                            CatalogPlacement::Redirected(_) | CatalogPlacement::InShare(_)
                        ))
                {
                    return Err(InvalidRecord::OpenDalInvalid);
                }
                if self.library_id.is_none()
                    || self.fs.is_empty()
                    || self
                        .fs
                        .iter()
                        .any(|adapter| matches!(adapter, FsAdapter::RunePeer { .. }))
                    || !matches!(
                        self.catalog,
                        CatalogPlacement::InMediaRoot
                            | CatalogPlacement::Redirected(_)
                            | CatalogPlacement::InShare(_)
                    )
                {
                    return Err(InvalidRecord::NativeInvalid);
                }
                if matches!(self.catalog, CatalogPlacement::InShare(_))
                    && (open_dal_count != 1 || self.fs.len() != 1)
                {
                    return Err(InvalidRecord::OpenDalInvalid);
                }
            }
            ProtocolAdapter::RuneForward {
                owner_fingerprint,
                mode: RuneForwardMode::Replica { .. },
            } => {
                if self.fs.iter().any(|adapter| {
                    matches!(adapter, FsAdapter::WebDav { .. } | FsAdapter::S3 { .. })
                }) {
                    return Err(InvalidRecord::OpenDalInvalid);
                }
                if self.library_id.is_none() {
                    return Err(InvalidRecord::ReplicaInvalid);
                }
                let peer_present = self.fs.iter().any(|adapter| matches!(adapter, FsAdapter::RunePeer { fingerprint } if fingerprint == owner_fingerprint));
                let legacy_local = matches!(
                    self.catalog,
                    CatalogPlacement::InMediaRoot | CatalogPlacement::Redirected(_)
                ) && self.local_folder_path().is_some();
                if !(matches!(self.catalog, CatalogPlacement::ConfigReplica) || legacy_local)
                    || !peer_present
                {
                    return Err(InvalidRecord::ReplicaInvalid);
                }
            }
            ProtocolAdapter::RuneForward {
                mode: RuneForwardMode::RemoteControl,
                ..
            } => {
                if self.library_id.is_some()
                    || !self.fs.is_empty()
                    || self.catalog != CatalogPlacement::None
                    || self.background_sync
                {
                    return Err(InvalidRecord::RemoteControlInvalid);
                }
            }
            ProtocolAdapter::SubsonicTranslate { .. } => {
                if self.library_id.is_some()
                    || !self.fs.is_empty()
                    || self.catalog != CatalogPlacement::None
                {
                    return Err(InvalidRecord::SubsonicInvalid);
                }
            }
        }
        if self.background_sync && self.catalog == CatalogPlacement::None {
            return Err(InvalidRecord::BackgroundSyncWithoutCatalog);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidRecord {
    #[error("Native record fields are invalid")]
    NativeInvalid,
    #[error("Rune replica fields are invalid")]
    ReplicaInvalid,
    #[error("Remote-control record fields are invalid")]
    RemoteControlInvalid,
    #[error("Subsonic record fields are invalid")]
    SubsonicInvalid,
    #[error("Background sync requires a catalog")]
    BackgroundSyncWithoutCatalog,
    #[error("OpenDAL libraries require a redirected catalog and a single filesystem adapter")]
    OpenDalInvalid,
    #[error("Two records share a catalog library id")]
    DuplicateLibraryId,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_open_record: Option<LibraryRecordId>,
}

mod v2 {
    use super::{
        CatalogPlacement, CredentialRef, DbPlacement, FsAdapter, LibraryAccess,
        LibraryConnectionAccess, LibraryRecordId, ProtocolAdapter, RuneForwardMode,
        SubsonicOpenMode, Uuid,
    };
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Registry {
        version: u32,
        libraries: Vec<Record>,
        #[serde(default)]
        last_open_record: Option<LibraryRecordId>,
    }

    #[derive(Deserialize)]
    struct Record {
        record_id: LibraryRecordId,
        alias: String,
        kind: Kind,
    }

    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum Kind {
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
            storage: ReplicaStorage,
        },
        RuneRemoteControl {
            fingerprint: String,
        },
        Subsonic {
            endpoint: String,
            credential: CredentialRef,
            mode: SubsonicOpenMode,
        },
    }

    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum OwnedStorage {
        Local {
            path: String,
            placement: DbPlacement,
        },
    }

    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum ReplicaStorage {
        Local {
            path: String,
            placement: DbPlacement,
        },
        Replica,
    }

    fn background_sync_default() -> bool {
        true
    }

    pub(super) struct ConvertedRegistry {
        pub libraries: Vec<super::LibraryRecord>,
        pub last_open_record: Option<LibraryRecordId>,
    }

    fn catalog(placement: DbPlacement) -> CatalogPlacement {
        match placement {
            DbPlacement::Portable => CatalogPlacement::InMediaRoot,
            DbPlacement::Redirected { storage_key } => CatalogPlacement::Redirected(storage_key),
        }
    }

    pub(super) fn convert(data: &[u8]) -> anyhow::Result<ConvertedRegistry> {
        use super::LibraryRecord;

        let registry: Registry = serde_json::from_slice(data)?;
        anyhow::ensure!(registry.version == 2, "Expected v2 library registry");
        let libraries = registry
            .libraries
            .into_iter()
            .map(|record| {
                let (library_id, protocol, fs, catalog, background_sync) = match record.kind {
                    Kind::Owned {
                        library_id,
                        storage: OwnedStorage::Local { path, placement },
                        background_sync,
                    } => (
                        Some(library_id),
                        ProtocolAdapter::Native,
                        vec![FsAdapter::Local { path }],
                        catalog(placement),
                        background_sync,
                    ),
                    Kind::RuneReplica {
                        library_id,
                        owner_fingerprint,
                        access_level,
                        connection_access,
                        background_sync,
                        storage,
                    } => {
                        let (fs, catalog) = match storage {
                            ReplicaStorage::Replica => (
                                vec![FsAdapter::RunePeer {
                                    fingerprint: owner_fingerprint.clone(),
                                }],
                                CatalogPlacement::ConfigReplica,
                            ),
                            ReplicaStorage::Local { path, placement } => (
                                vec![
                                    FsAdapter::Local { path },
                                    FsAdapter::RunePeer {
                                        fingerprint: owner_fingerprint.clone(),
                                    },
                                ],
                                catalog(placement),
                            ),
                        };
                        (
                            Some(library_id),
                            ProtocolAdapter::RuneForward {
                                owner_fingerprint,
                                mode: RuneForwardMode::Replica {
                                    access_level,
                                    connection_access,
                                },
                            },
                            fs,
                            catalog,
                            background_sync,
                        )
                    }
                    Kind::RuneRemoteControl { fingerprint } => (
                        None,
                        ProtocolAdapter::RuneForward {
                            owner_fingerprint: fingerprint,
                            mode: RuneForwardMode::RemoteControl,
                        },
                        Vec::new(),
                        CatalogPlacement::None,
                        false,
                    ),
                    Kind::Subsonic {
                        endpoint,
                        credential,
                        mode,
                    } => (
                        None,
                        ProtocolAdapter::SubsonicTranslate {
                            endpoint,
                            credential,
                            mode,
                        },
                        Vec::new(),
                        CatalogPlacement::None,
                        false,
                    ),
                };
                LibraryRecord {
                    record_id: record.record_id,
                    alias: record.alias,
                    library_id,
                    protocol,
                    fs,
                    catalog,
                    background_sync,
                }
            })
            .collect();
        Ok(ConvertedRegistry {
            libraries,
            last_open_record: registry.last_open_record,
        })
    }
}

fn write_v2_conversion(path: &Path, original: &[u8], file: &v2::ConvertedRegistry) -> Result<()> {
    let dir = path.parent().context("Registry has no parent directory")?;
    let backup = dir.join("libraries.json.v2.bak");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup)
    {
        Ok(mut backup_file) => {
            if let Err(error) = backup_file.write_all(original) {
                drop(backup_file);
                let _ = fs::remove_file(&backup);
                return Err(error).context("Failed to write v2 registry backup");
            }
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error).context("Failed to create v2 registry backup"),
    }
    let temp = dir.join(format!(".libraries-{}.tmp", Uuid::new_v4()));
    let data = serde_json::to_vec_pretty(&RegistryFile {
        version: 3,
        libraries: file.libraries.clone(),
        last_open_record: file.last_open_record,
    })?;
    let result = (|| -> Result<()> {
        fs::write(&temp, data).context("Failed to write converted registry")?;
        fs::rename(&temp, path).context("Failed to replace converted registry")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
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
    pub invalid: Vec<(LibraryRecordId, InvalidRecord)>,
    pub last_open_record: Option<LibraryRecordId>,
    invalid_records: Vec<LibraryRecord>,
    path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
#[error("Library registry reset required: old libraries.json moved to libraries.json.old")]
pub struct LibraryRegistryResetRequired;

fn validate_registry_version(data: &[u8]) -> Result<()> {
    let version: Version = serde_json::from_slice(data).context("Invalid library file version")?;
    match version.version {
        1 => Err(LibraryRegistryResetRequired.into()),
        2 | 3 => Ok(()),
        other => bail!("Unsupported library registry version {other}"),
    }
}
impl LibraryRegistry {
    /// Load the registry, converting v2 and moving v1 files aside because their format is ambiguous.
    /// Read the registry. A version 2 file is converted in memory only; it is
    /// written back as version 3 by the next write, which holds the writer lock.
    /// This must not take `REGISTRY_WRITE`: it is sync and runs on runtime threads
    /// while async writers hold that lock across awaits.
    pub fn load(config_dir: &Path) -> Result<Self> {
        Self::load_inner(config_dir, false)
    }

    /// Load for a writer that already holds `REGISTRY_WRITE`; persists a v2 conversion.
    fn load_locked(config_dir: &Path) -> Result<Self> {
        Self::load_inner(config_dir, true)
    }

    fn load_inner(config_dir: &Path, persist_conversion: bool) -> Result<Self> {
        let path = config_dir.join("libraries.json");
        let loaded = match fs::read(&path) {
            Ok(data) => match validate_registry_version(&data) {
                Ok(()) => {
                    let version: Version = serde_json::from_slice(&data)?;
                    if version.version == 2 {
                        let file = v2::convert(&data).context("Invalid v2 library registry")?;
                        if persist_conversion {
                            write_v2_conversion(&path, &data, &file)?;
                        }
                        (file.libraries, file.last_open_record)
                    } else {
                        let file: RegistryFile =
                            serde_json::from_slice(&data).context("Invalid library registry")?;
                        (file.libraries, file.last_open_record)
                    }
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
                    (Vec::new(), None)
                }
                Err(error) => return Err(error),
            },
            Err(e) if e.kind() == ErrorKind::NotFound => (Vec::new(), None),
            Err(e) => return Err(e).context("Failed to read library registry"),
        };
        let (records, last_open_record) = loaded;
        let mut counts = HashMap::new();
        for record in &records {
            if let Some(id) = record.library_id {
                *counts.entry(id).or_insert(0usize) += 1;
            }
        }
        let mut libraries = Vec::new();
        let mut invalid = Vec::new();
        let mut invalid_records = Vec::new();
        for record in records {
            let result = if record
                .library_id
                .is_some_and(|id| counts.get(&id).copied().unwrap_or_default() > 1)
            {
                Err(InvalidRecord::DuplicateLibraryId)
            } else {
                record.validate()
            };
            match result {
                Ok(()) => libraries.push(record),
                Err(error) => {
                    invalid.push((record.record_id, error));
                    invalid_records.push(record);
                }
            }
        }
        let invalid_ids: HashSet<_> = invalid.iter().map(|(id, _)| *id).collect();
        libraries.retain(|record| !invalid_ids.contains(&record.record_id));
        Ok(Self {
            libraries,
            invalid,
            invalid_records,
            last_open_record,
            path,
        })
    }
    pub fn record(&self, record_id: LibraryRecordId) -> Result<&LibraryRecord> {
        if let Some((_, invalid)) = self.invalid.iter().find(|(id, _)| *id == record_id) {
            return Err(anyhow::Error::new(*invalid));
        }
        self.libraries
            .iter()
            .find(|record| record.record_id == record_id)
            .context("Library record is not registered on this node")
    }

    /// Iterate over records that were retained because their configuration is invalid.
    pub fn invalid_records(&self) -> impl Iterator<Item = (&LibraryRecord, InvalidRecord)> {
        self.invalid_records.iter().filter_map(|record| {
            self.invalid
                .iter()
                .find(|(record_id, _)| *record_id == record.record_id)
                .map(|(_, reason)| (record, *reason))
        })
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

    pub fn validate(&self) -> Result<(), InvalidRecord> {
        let mut seen = HashSet::new();
        for record in &self.libraries {
            record.validate()?;
            if let Some(id) = record.library_id
                && !seen.insert(id)
            {
                return Err(InvalidRecord::DuplicateLibraryId);
            }
        }
        Ok(())
    }

    fn catalog_mut(&mut self, library_id: Uuid) -> Result<&mut LibraryRecord> {
        self.libraries
            .iter_mut()
            .find(|record| record.catalog_library_id() == Some(library_id))
            .context("Library is not registered on this node")
    }

    /// Resolve a config-local replica by its catalog identity.
    pub fn replica(&self, library_id: Uuid) -> Result<&LibraryRecord> {
        let record = self.catalog(library_id)?;
        ensure!(
            matches!(
                record.protocol,
                ProtocolAdapter::RuneForward {
                    mode: RuneForwardMode::Replica { .. },
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
        let mut registry = Self::load_locked(config_dir)?;
        let before = registry.libraries.clone();
        let before_invalid_records = registry.invalid_records.clone();
        let before_last_open_record = registry.last_open_record;
        let result = change(&mut registry)?;
        if registry.libraries != before
            || registry.invalid_records != before_invalid_records
            || registry.last_open_record != before_last_open_record
        {
            registry.validate().map_err(anyhow::Error::new)?;
            for record in &registry.libraries {
                let changed =
                    before.iter().find(|old| old.record_id == record.record_id) != Some(record);
                if changed
                    && record.library_id.is_some_and(|id| {
                        registry
                            .invalid_records
                            .iter()
                            .any(|invalid| invalid.library_id == Some(id))
                    })
                {
                    return Err(anyhow::Error::new(InvalidRecord::DuplicateLibraryId));
                }
            }
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
            record.background_sync = enabled;
            Ok(())
        })
        .await
    }

    /// Remove one node-local record without changing its catalog or storage files.
    pub async fn remove_record(
        config_dir: &Path,
        record_id: LibraryRecordId,
        credentials: &CredentialStore,
    ) -> Result<bool> {
        let (removed, credential) = Self::update(config_dir, |registry| {
            let Some(index) = registry
                .libraries
                .iter()
                .position(|record| record.record_id == record_id)
            else {
                let Some(index) = registry
                    .invalid_records
                    .iter()
                    .position(|record| record.record_id == record_id)
                else {
                    return Ok((false, None));
                };
                let record = registry.invalid_records.remove(index);
                registry.invalid.retain(|(id, _)| *id != record_id);
                let credential = record_credential(&record);
                return Ok((true, credential));
            };
            let record = registry.libraries.remove(index);
            let credential = record_credential(&record);
            if registry.last_open_record == Some(record_id) {
                registry.last_open_record = None;
            }
            Ok((true, credential))
        })
        .await?;
        if let Some(credential) = credential {
            match credentials.remove(credential).await {
                Ok(()) | Err(CredentialStoreError::CredentialMissing(_)) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(removed)
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
                library.library_id == Some(library_id) && matches!(&library.protocol, ProtocolAdapter::RuneForward { owner_fingerprint, mode: RuneForwardMode::Replica { .. } } if owner_fingerprint == fingerprint)
            }) else {
                return Ok(false);
            };
            let ProtocolAdapter::RuneForward { mode: RuneForwardMode::Replica { access_level, .. }, .. } = &mut library.protocol else { unreachable!() };
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
                library.library_id == Some(library_id) && matches!(&library.protocol, ProtocolAdapter::RuneForward { owner_fingerprint, mode: RuneForwardMode::Replica { .. } } if owner_fingerprint == fingerprint)
            }) else {
                return Ok(false);
            };
            let ProtocolAdapter::RuneForward { mode: RuneForwardMode::Replica { connection_access, .. }, .. } = &mut library.protocol else { unreachable!() };
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
                matches!(&record.protocol, ProtocolAdapter::RuneForward { owner_fingerprint: stored, mode: RuneForwardMode::RemoteControl } if stored == fingerprint)
            }) {
                return Ok(record.clone());
            }
            let record = LibraryRecord {
                record_id: LibraryRecordId(Uuid::new_v4()),
                alias: alias.to_owned(),
                library_id: None,
                protocol: ProtocolAdapter::RuneForward { owner_fingerprint: fingerprint.to_owned(), mode: RuneForwardMode::RemoteControl },
                fs: Vec::new(), catalog: CatalogPlacement::None, background_sync: false,
            };
            record.validate()?;
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
        self.validate().map_err(anyhow::Error::new)?;
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
                    version.version == 3,
                    "Unsupported library registry version {}",
                    version.version
                );
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("Failed to read library registry before saving"),
        }
        let temp = dir.join(format!(".libraries-{}.tmp", Uuid::new_v4()));
        let data = serde_json::to_vec_pretty(&RegistryFile {
            version: 3,
            libraries: self
                .libraries
                .iter()
                .chain(self.invalid_records.iter())
                .cloned()
                .collect(),
            last_open_record: self.last_open_record,
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

pub(crate) async fn registry_write_guard() -> MutexGuard<'static, ()> {
    REGISTRY_WRITE.lock().await
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
                .map(|entry| match &entry.protocol {
                    ProtocolAdapter::Native => LegacyAuthority::Owned,
                    ProtocolAdapter::RuneForward {
                        owner_fingerprint,
                        mode: RuneForwardMode::Replica { .. },
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
        existing.map(|record| &record.catalog),
        Some(CatalogPlacement::ConfigReplica)
    );
    let background_sync = existing.is_none_or(LibraryRecord::background_sync);
    let record_id = existing
        .map(|record| record.record_id)
        .unwrap_or(LibraryRecordId(Uuid::new_v4()));
    let catalog = match placement {
        DbPlacement::Portable => {
            if was_replica {
                CatalogPlacement::ConfigReplica
            } else {
                CatalogPlacement::InMediaRoot
            }
        }
        DbPlacement::Redirected { storage_key } => CatalogPlacement::Redirected(storage_key),
    };
    let (protocol, fs) = match authority {
        LegacyAuthority::Owned => (
            ProtocolAdapter::Native,
            vec![FsAdapter::Local {
                path: root.to_owned(),
            }],
        ),
        LegacyAuthority::Hosted { fingerprint } => {
            let access_level = existing.and_then(LibraryRecord::access_level);
            let connection_access = existing.and_then(LibraryRecord::connection_access);
            let mut adapters = Vec::new();
            if !was_replica {
                adapters.push(FsAdapter::Local {
                    path: root.to_owned(),
                });
            }
            adapters.push(FsAdapter::RunePeer {
                fingerprint: fingerprint.clone(),
            });
            (
                ProtocolAdapter::RuneForward {
                    owner_fingerprint: fingerprint,
                    mode: RuneForwardMode::Replica {
                        access_level,
                        connection_access,
                    },
                },
                adapters,
            )
        }
    };
    let record = LibraryRecord {
        record_id,
        alias,
        library_id: Some(id),
        protocol,
        fs,
        catalog,
        background_sync,
    };
    record.validate().map_err(anyhow::Error::new)?;
    if registry
        .invalid_records
        .iter()
        .any(|invalid| invalid.library_id == Some(id))
    {
        return Err(anyhow::Error::new(InvalidRecord::DuplicateLibraryId));
    }
    registry.libraries.retain(|entry| {
        let collision = entry.catalog_library_id() != Some(id)
            && record.root(Path::new("")) == entry.root(Path::new(""));
        if collision {
            warn!(
                "Removing stale library {} for path {}",
                entry.catalog_library_id().unwrap_or_default(),
                root
            );
        }
        entry.catalog_library_id() != Some(id) && !collision
    });
    registry.libraries.push(record.clone());
    registry.validate().map_err(anyhow::Error::new)?;
    registry.save()?;
    Ok(record)
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
        if matches!(existing.protocol, ProtocolAdapter::Native) {
            bail!("Cannot create a replica of a library this node owns");
        }
        if !matches!(&existing.protocol, ProtocolAdapter::RuneForward { owner_fingerprint, mode: RuneForwardMode::Replica { .. } } if owner_fingerprint == authority_fingerprint)
        {
            bail!("Library already has a different authority");
        }
        return Ok(existing.clone());
    }
    if registry
        .invalid_records
        .iter()
        .any(|record| record.library_id == Some(library_id))
    {
        return Err(anyhow::Error::new(InvalidRecord::DuplicateLibraryId));
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
    let record = LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: file.alias,
        library_id: Some(library_id),
        protocol: ProtocolAdapter::RuneForward {
            owner_fingerprint: authority_fingerprint.to_owned(),
            mode: RuneForwardMode::Replica {
                access_level: None,
                connection_access: None,
            },
        },
        fs: vec![FsAdapter::RunePeer {
            fingerprint: authority_fingerprint.to_owned(),
        }],
        catalog: CatalogPlacement::ConfigReplica,
        background_sync: true,
    };
    record.validate().map_err(anyhow::Error::new)?;
    registry.libraries.push(record.clone());
    registry.validate().map_err(anyhow::Error::new)?;
    if let Err(error) = registry.save() {
        registry.libraries.pop();
        return Err(error);
    }
    Ok(record)
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
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

    fn native_record(id: Uuid, path: &str) -> LibraryRecord {
        LibraryRecord {
            record_id: LibraryRecordId(Uuid::new_v4()),
            alias: "Owned".into(),
            library_id: Some(id),
            protocol: ProtocolAdapter::Native,
            fs: vec![FsAdapter::Local { path: path.into() }],
            catalog: CatalogPlacement::InMediaRoot,
            background_sync: true,
        }
    }

    fn replica_record(id: Uuid, fingerprint: &str) -> LibraryRecord {
        LibraryRecord {
            record_id: LibraryRecordId(Uuid::new_v4()),
            alias: "Replica".into(),
            library_id: Some(id),
            protocol: ProtocolAdapter::RuneForward {
                owner_fingerprint: fingerprint.into(),
                mode: RuneForwardMode::Replica {
                    access_level: None,
                    connection_access: None,
                },
            },
            fs: vec![FsAdapter::RunePeer {
                fingerprint: fingerprint.into(),
            }],
            catalog: CatalogPlacement::ConfigReplica,
            background_sync: true,
        }
    }

    fn remote_control_record(fingerprint: &str) -> LibraryRecord {
        LibraryRecord {
            record_id: LibraryRecordId(Uuid::new_v4()),
            alias: "Control".into(),
            library_id: None,
            protocol: ProtocolAdapter::RuneForward {
                owner_fingerprint: fingerprint.into(),
                mode: RuneForwardMode::RemoteControl,
            },
            fs: Vec::new(),
            catalog: CatalogPlacement::None,
            background_sync: false,
        }
    }

    fn subsonic_record(credential: CredentialRef, mode: SubsonicOpenMode) -> LibraryRecord {
        LibraryRecord {
            record_id: LibraryRecordId(Uuid::new_v4()),
            alias: "Subsonic".into(),
            library_id: None,
            protocol: ProtocolAdapter::SubsonicTranslate {
                endpoint: "https://music.example".into(),
                credential,
                mode,
            },
            fs: Vec::new(),
            catalog: CatalogPlacement::None,
            background_sync: false,
        }
    }

    fn write_v2_registry_fixture(dir: &Path) -> Vec<u8> {
        let data = br#"{
  "version": 2,
  "libraries": [
    {"record_id":"00000000-0000-0000-0000-000000000001","alias":"Owned portable","kind":{"type":"owned","library_id":"10000000-0000-0000-0000-000000000001","storage":{"type":"local","path":"/music/portable","placement":{"type":"portable"}},"background_sync":true}},
    {"record_id":"00000000-0000-0000-0000-000000000002","alias":"Owned redirected","kind":{"type":"owned","library_id":"10000000-0000-0000-0000-000000000002","storage":{"type":"local","path":"/music/redirected","placement":{"type":"redirected","storage_key":"20000000-0000-0000-0000-000000000002"}},"background_sync":false}},
    {"record_id":"00000000-0000-0000-0000-000000000003","alias":"Replica config","kind":{"type":"rune_replica","library_id":"10000000-0000-0000-0000-000000000003","owner_fingerprint":"owner-config","access_level":"ReadWrite","connection_access":"ViewEdit","background_sync":true,"storage":{"type":"replica"}}},
    {"record_id":"00000000-0000-0000-0000-000000000004","alias":"Replica local","kind":{"type":"rune_replica","library_id":"10000000-0000-0000-0000-000000000004","owner_fingerprint":"owner-local","access_level":"Read","connection_access":"View","background_sync":false,"storage":{"type":"local","path":"/music/legacy","placement":{"type":"redirected","storage_key":"20000000-0000-0000-0000-000000000004"}}}},
    {"record_id":"00000000-0000-0000-0000-000000000005","alias":"Remote control","kind":{"type":"rune_remote_control","fingerprint":"remote-owner"}},
    {"record_id":"00000000-0000-0000-0000-000000000006","alias":"Subsonic","kind":{"type":"subsonic","endpoint":"https://music.example","credential":"30000000-0000-0000-0000-000000000006","mode":"play_on_this_device"}}
  ],
  "last_open_record":"00000000-0000-0000-0000-000000000004"
}"#;
        fs::write(dir.join("libraries.json"), data).unwrap();
        data.to_vec()
    }

    #[tokio::test]
    async fn v2_registry_converts_all_record_kinds_and_keeps_backup() {
        let dir = TempDir::new().unwrap();
        let original = write_v2_registry_fixture(dir.path());

        let registry = LibraryRegistry::load(dir.path()).unwrap();
        assert_eq!(registry.libraries.len(), 6);
        assert!(registry.invalid.is_empty());
        assert_eq!(
            registry
                .libraries
                .iter()
                .map(|record| record.record_id)
                .collect::<Vec<_>>(),
            (1..=6)
                .map(|id| LibraryRecordId(Uuid::from_u128(id)))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            registry.last_open_record,
            Some(LibraryRecordId(
                Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap()
            ))
        );
        assert_eq!(registry.libraries[0].protocol, ProtocolAdapter::Native);
        assert_eq!(
            registry.libraries[0].fs,
            vec![FsAdapter::Local {
                path: "/music/portable".into()
            }]
        );
        assert_eq!(registry.libraries[0].catalog, CatalogPlacement::InMediaRoot);
        assert!(registry.libraries[0].background_sync);
        assert_eq!(
            registry.libraries[1].catalog,
            CatalogPlacement::Redirected(
                Uuid::parse_str("20000000-0000-0000-0000-000000000002").unwrap()
            )
        );
        assert!(!registry.libraries[1].background_sync);
        assert_eq!(
            registry.libraries[2].fs,
            vec![FsAdapter::RunePeer {
                fingerprint: "owner-config".into()
            }]
        );
        assert_eq!(
            registry.libraries[2].catalog,
            CatalogPlacement::ConfigReplica
        );
        assert_eq!(
            registry.libraries[2].access_level(),
            Some(LibraryAccess::ReadWrite)
        );
        assert_eq!(
            registry.libraries[2].connection_access(),
            Some(LibraryConnectionAccess::ViewEdit)
        );
        assert_eq!(
            registry.libraries[3].fs,
            vec![
                FsAdapter::Local {
                    path: "/music/legacy".into()
                },
                FsAdapter::RunePeer {
                    fingerprint: "owner-local".into()
                }
            ]
        );
        assert_eq!(
            registry.libraries[3].catalog,
            CatalogPlacement::Redirected(
                Uuid::parse_str("20000000-0000-0000-0000-000000000004").unwrap()
            )
        );
        assert!(matches!(
            registry.libraries[4].protocol,
            ProtocolAdapter::RuneForward {
                mode: RuneForwardMode::RemoteControl,
                ..
            }
        ));
        assert_eq!(registry.libraries[4].catalog, CatalogPlacement::None);
        assert!(!registry.libraries[4].background_sync);
        assert!(
            matches!(&registry.libraries[5].protocol, ProtocolAdapter::SubsonicTranslate { endpoint, mode: SubsonicOpenMode::PlayOnThisDevice, .. } if endpoint == "https://music.example")
        );
        assert!(
            matches!(&registry.libraries[5].protocol, ProtocolAdapter::SubsonicTranslate { credential, .. } if credential == &CredentialRef(Uuid::parse_str("30000000-0000-0000-0000-000000000006").unwrap()))
        );
        // A plain load only converts in memory; the first write persists version 3.
        assert_eq!(
            fs::read(dir.path().join("libraries.json")).unwrap(),
            original
        );
        LibraryRegistry::update(dir.path(), |_| Ok(()))
            .await
            .unwrap();
        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("libraries.json")).unwrap()).unwrap();
        assert_eq!(written["version"], 3);
        assert_eq!(
            fs::read(dir.path().join("libraries.json.v2.bak")).unwrap(),
            original
        );
    }

    #[tokio::test]
    async fn load_does_not_wait_for_the_writer_lock() {
        let dir = TempDir::new().unwrap();
        write_v2_registry_fixture(dir.path());
        // Async writers hold the lock across awaits; a sync load on a runtime
        // thread that waited for it would never let the holder run again.
        let _writer = registry_write_guard().await;
        let path = dir.path().to_path_buf();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(LibraryRegistry::load(&path).map(|r| r.libraries.len()));
        });
        let loaded = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("load waited for the registry writer lock");
        assert_eq!(loaded.unwrap(), 6);
    }

    #[tokio::test]
    async fn registering_after_loading_v2_succeeds() {
        let dir = TempDir::new().unwrap();
        write_v2_registry_fixture(dir.path());
        let _registry = LibraryRegistry::load(dir.path()).unwrap();
        let added = LibraryRegistry::register_remote_control(dir.path(), "new-owner", "New peer")
            .await
            .unwrap();
        assert_eq!(added.alias, "New peer");
        assert_eq!(
            LibraryRegistry::load(dir.path()).unwrap().libraries.len(),
            7
        );
    }

    #[test]
    fn invalid_v2_records_are_retained_in_invalid_list_after_conversion() {
        let dir = TempDir::new().unwrap();
        write_v2_registry_fixture(dir.path());
        let existing_backup = b"keep the earlier backup";
        fs::write(dir.path().join("libraries.json.v2.bak"), existing_backup).unwrap();
        let path = dir.path().join("libraries.json");
        let contents = fs::read_to_string(&path).unwrap();
        let contents = contents.replace(
            "10000000-0000-0000-0000-000000000002",
            "10000000-0000-0000-0000-000000000001",
        );
        fs::write(path, contents).unwrap();

        let registry = LibraryRegistry::load(dir.path()).unwrap();
        assert_eq!(
            registry.invalid,
            vec![
                (
                    LibraryRecordId(Uuid::from_u128(1)),
                    InvalidRecord::DuplicateLibraryId
                ),
                (
                    LibraryRecordId(Uuid::from_u128(2)),
                    InvalidRecord::DuplicateLibraryId
                )
            ]
        );
        assert_eq!(registry.libraries.len(), 4);
        assert_eq!(
            fs::read(dir.path().join("libraries.json.v2.bak")).unwrap(),
            existing_backup
        );
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
            matches!(&first.protocol, ProtocolAdapter::RuneForward { owner_fingerprint, mode: RuneForwardMode::Replica { .. } } if owner_fingerprint == "owner")
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
        let mut expected = first.clone();
        expected.record_id = recovered.record_id;
        assert_eq!(recovered, expected);
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
        assert!(matches!(owned.protocol, ProtocolAdapter::Native));
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
        assert!(matches!(owned.protocol, ProtocolAdapter::Native));
        assert_eq!(owned.catalog_library_id(), Some(id));
        f.registry.libraries[0].protocol = ProtocolAdapter::RuneForward {
            owner_fingerprint: "owner".into(),
            mode: RuneForwardMode::Replica {
                access_level: Some(LibraryAccess::Read),
                connection_access: None,
            },
        };
        f.registry.libraries[0].fs.push(FsAdapter::RunePeer {
            fingerprint: "owner".into(),
        });
        f.registry.save().unwrap();
        let reopened_owned = f.resolve(&root, "", StorageMode::Portable).await;
        assert!(matches!(reopened_owned.protocol, ProtocolAdapter::Native));
        let key = Uuid::new_v4();
        f.resolve(&root, "", StorageMode::Redirected(key)).await;
        let authority = ProtocolAdapter::RuneForward {
            owner_fingerprint: "owner".into(),
            mode: RuneForwardMode::Replica {
                access_level: Some(LibraryAccess::Read),
                connection_access: None,
            },
        };
        f.registry.libraries[0].protocol = authority.clone();
        f.registry.libraries[0].catalog = CatalogPlacement::Redirected(key);
        f.registry.libraries[0].fs.push(FsAdapter::RunePeer {
            fingerprint: "owner".into(),
        });
        f.registry.save().unwrap();
        f.registry = LibraryRegistry::load(f.dir.path()).unwrap();
        let reopened = f.resolve(&root, "", StorageMode::Redirected(key)).await;
        assert_eq!(reopened.protocol, authority);

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
            &hosted.protocol,
            ProtocolAdapter::RuneForward {
                mode: RuneForwardMode::Replica { .. },
                ..
            }
        ));
        assert_eq!(hosted.local_folder_path(), Some(hosted_root.as_str()));
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
            matches!(&third.protocol, ProtocolAdapter::Native)
                && third.local_folder_path() == Some(moved)
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
        let content = r#"{"version":4,"libraries":[]}"#;
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
        assert_eq!(replica.catalog, CatalogPlacement::ConfigReplica);
        assert_eq!(
            replica.root(f.dir.path()).unwrap(),
            f.dir.path().join("replicas").join(id.to_string())
        );
        assert_eq!(replica.display_name().unwrap(), "Remote");
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
        assert_eq!(resolved.catalog, CatalogPlacement::ConfigReplica);
        // Replicas do not collide with each other the way path entries do.
        assert_eq!(f.registry.libraries.len(), 2);
    }

    #[tokio::test]
    async fn every_record_kind_round_trips_through_serde() {
        let records = [
            native_record(Uuid::new_v4(), "/music"),
            replica_record(Uuid::new_v4(), "owner-fingerprint"),
            remote_control_record("remote-fingerprint"),
            subsonic_record(
                CredentialRef(Uuid::new_v4()),
                SubsonicOpenMode::PlayOnThisDevice,
            ),
        ];
        for record in records {
            assert_eq!(record.validate(), Ok(()));
            let json = serde_json::to_vec(&record).unwrap();
            assert_eq!(
                serde_json::from_slice::<LibraryRecord>(&json).unwrap(),
                record
            );
            let dir = TempDir::new().unwrap();
            LibraryRegistry::update(dir.path(), |registry| {
                registry.libraries.push(record.clone());
                Ok(())
            })
            .await
            .unwrap();
            assert_eq!(
                LibraryRegistry::load(dir.path())
                    .unwrap()
                    .record(record.record_id)
                    .unwrap(),
                &record
            );
        }
    }

    #[test]
    fn each_protocol_shape_and_validation_rule_is_typed() {
        let native = native_record(Uuid::new_v4(), "/music");
        let replica = replica_record(Uuid::new_v4(), "owner");
        let remote = remote_control_record("peer");
        let subsonic = subsonic_record(
            CredentialRef(Uuid::new_v4()),
            SubsonicOpenMode::RemoteControl,
        );
        for record in [&native, &replica, &remote, &subsonic] {
            assert_eq!(record.validate(), Ok(()));
        }

        let mut invalid_native = native.clone();
        invalid_native.library_id = None;
        assert_eq!(invalid_native.validate(), Err(InvalidRecord::NativeInvalid));

        let mut invalid_replica = replica.clone();
        invalid_replica.fs.clear();
        assert_eq!(
            invalid_replica.validate(),
            Err(InvalidRecord::ReplicaInvalid)
        );

        let mut invalid_remote = remote.clone();
        invalid_remote.fs.push(FsAdapter::Local {
            path: "/music".into(),
        });
        assert_eq!(
            invalid_remote.validate(),
            Err(InvalidRecord::RemoteControlInvalid)
        );

        let mut invalid_subsonic = subsonic.clone();
        invalid_subsonic.fs.push(FsAdapter::Local {
            path: "/music".into(),
        });
        assert_eq!(
            invalid_subsonic.validate(),
            Err(InvalidRecord::SubsonicInvalid)
        );

        let opendal = LibraryRecord {
            record_id: LibraryRecordId(Uuid::new_v4()),
            alias: "OpenDAL".into(),
            library_id: Some(Uuid::new_v4()),
            protocol: ProtocolAdapter::Native,
            fs: vec![FsAdapter::WebDav {
                endpoint: "http://127.0.0.1:1".into(),
                root: "/music/".into(),
                username: "user".into(),
                credential: CredentialRef(Uuid::new_v4()),
            }],
            catalog: CatalogPlacement::Redirected(Uuid::new_v4()),
            background_sync: true,
        };
        assert_eq!(opendal.validate(), Ok(()));
        assert_eq!(opendal.root(Path::new("/config")), Some(PathBuf::new()));
        let mut in_share = opendal.clone();
        in_share.catalog = CatalogPlacement::InShare(Uuid::new_v4());
        assert_eq!(in_share.validate(), Ok(()));
        assert!(matches!(
            in_share.db_placement(),
            Some(DbPlacement::Redirected { .. })
        ));
        let mut local_in_share = in_share.clone();
        local_in_share.fs = vec![FsAdapter::Local {
            path: "/music".into(),
        }];
        assert_eq!(
            local_in_share.validate(),
            Err(InvalidRecord::OpenDalInvalid)
        );
        let mut opendal_in_media_root = opendal.clone();
        opendal_in_media_root.catalog = CatalogPlacement::InMediaRoot;
        assert_eq!(
            opendal_in_media_root.validate(),
            Err(InvalidRecord::OpenDalInvalid)
        );
        let mut opendal_with_local = opendal.clone();
        opendal_with_local.fs.push(FsAdapter::Local {
            path: "/music".into(),
        });
        assert_eq!(
            opendal_with_local.validate(),
            Err(InvalidRecord::OpenDalInvalid)
        );
        let mut replica_with_opendal = replica.clone();
        replica_with_opendal.fs.push(opendal.fs[0].clone());
        assert_eq!(
            replica_with_opendal.validate(),
            Err(InvalidRecord::OpenDalInvalid)
        );

        let mut invalid_background = subsonic;
        invalid_background.background_sync = true;
        assert_eq!(
            invalid_background.validate(),
            Err(InvalidRecord::BackgroundSyncWithoutCatalog)
        );

        let mut duplicate = LibraryRegistry::load(Path::new("/unused")).unwrap();
        let id = native.library_id.unwrap();
        duplicate.libraries = vec![native.clone(), native_record(id, "/other")];
        assert_eq!(duplicate.validate(), Err(InvalidRecord::DuplicateLibraryId));
    }

    #[tokio::test]
    async fn invalid_registration_is_refused_without_changing_registry_file() {
        let dir = TempDir::new().unwrap();
        let valid = native_record(Uuid::new_v4(), "/music");
        LibraryRegistry::update(dir.path(), |registry| {
            registry.libraries.push(valid);
            Ok(())
        })
        .await
        .unwrap();
        let before = fs::read(dir.path().join("libraries.json")).unwrap();
        let mut invalid = native_record(Uuid::new_v4(), "/bad");
        invalid.fs.clear();
        let error = LibraryRegistry::update(dir.path(), |registry| {
            registry.libraries.push(invalid);
            Ok(())
        })
        .await
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<InvalidRecord>(),
            Some(&InvalidRecord::NativeInvalid)
        );
        assert_eq!(fs::read(dir.path().join("libraries.json")).unwrap(), before);
    }

    #[tokio::test]
    async fn removing_opendal_record_removes_its_credential() {
        use crate::credentials::{Credential, Secret};

        let dir = TempDir::new().unwrap();
        let credentials = CredentialStore::open(dir.path());
        let credential = credentials
            .put(Credential {
                username: "user".into(),
                secret: Secret::new(b"secret".to_vec()),
            })
            .await
            .unwrap();
        let record = LibraryRecord {
            record_id: LibraryRecordId(Uuid::new_v4()),
            alias: "WebDAV".into(),
            library_id: Some(Uuid::new_v4()),
            protocol: ProtocolAdapter::Native,
            fs: vec![FsAdapter::WebDav {
                endpoint: "http://example.test".into(),
                root: "/music/".into(),
                username: "user".into(),
                credential,
            }],
            catalog: CatalogPlacement::Redirected(Uuid::new_v4()),
            background_sync: true,
        };
        LibraryRegistry::update(dir.path(), |registry| {
            registry.libraries.push(record.clone());
            Ok(())
        })
        .await
        .unwrap();
        assert!(credentials.get(credential).await.is_ok());
        assert!(
            LibraryRegistry::remove_record(dir.path(), record.record_id, &credentials)
                .await
                .unwrap()
        );
        assert!(matches!(
            credentials.get(credential).await,
            Err(CredentialStoreError::CredentialMissing(_))
        ));
    }

    #[tokio::test]
    async fn invalid_loaded_record_is_listed_openable_as_error_and_removable() {
        let dir = TempDir::new().unwrap();
        let mut valid = native_record(Uuid::new_v4(), "/music");
        valid.alias = "Valid".into();
        let mut invalid = native_record(Uuid::new_v4(), "/broken");
        invalid.fs.clear();
        invalid.alias = "Invalid".into();
        fs::write(
            dir.path().join("libraries.json"),
            serde_json::to_vec(&RegistryFile {
                version: 3,
                libraries: vec![valid.clone(), invalid.clone()],
                last_open_record: None,
            })
            .unwrap(),
        )
        .unwrap();
        let loaded = LibraryRegistry::load(dir.path()).unwrap();
        assert_eq!(loaded.record(valid.record_id).unwrap(), &valid);
        assert_eq!(
            loaded.invalid,
            vec![(invalid.record_id, InvalidRecord::NativeInvalid)]
        );
        assert_eq!(
            loaded
                .record(invalid.record_id)
                .unwrap_err()
                .downcast_ref::<InvalidRecord>(),
            Some(&InvalidRecord::NativeInvalid)
        );
        let credentials = CredentialStore::open(dir.path());
        assert!(
            LibraryRegistry::remove_record(dir.path(), invalid.record_id, &credentials)
                .await
                .unwrap()
        );
        let loaded = LibraryRegistry::load(dir.path()).unwrap();
        assert!(loaded.invalid.is_empty());
        assert_eq!(loaded.libraries, vec![valid]);
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
                remote_control_record("peer"),
                subsonic_record(
                    CredentialRef(Uuid::new_v4()),
                    SubsonicOpenMode::RemoteControl,
                ),
            ],
            invalid: Vec::new(),
            invalid_records: Vec::new(),
            last_open_record: None,
            path: PathBuf::new(),
        };
        assert_eq!(registry.catalogs().count(), 0);
        for record in &registry.libraries {
            assert_eq!(record.catalog_library_id(), None);
            assert_eq!(record.root(Path::new("/config")), None);
            assert_eq!(record.display_name(), None);
        }
    }
}
