use std::{
    collections::BTreeMap,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Mutex, RwLock},
};

use anyhow::{Context, Result};
use discovery::{
    access_policy::{AccessPolicy, PeerIdentity},
    server::{UserStatus, UserSummary},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::watch;
use uuid::Uuid;

/// The application actions exposed by authenticated transports.
#[derive(Clone, Debug)]
pub enum Action {
    // Control is node-wide until player sessions exist.
    Control,
    ConnectionView { library_id: Uuid },
    ConnectionEdit { library_id: Uuid },
    SyncPull { library_id: Uuid },
    SyncPush { library_id: Uuid },
    Administer,
}

/// Connection access granted to a device for one library.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ConnectionLevel {
    #[default]
    None,
    View,
    ViewEdit,
}

/// Synchronization access granted to a device for one library.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SyncLevel {
    #[default]
    None,
    Pull,
    PullPush,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryGrants<T> {
    pub all_libraries: Option<T>,
    pub libraries: BTreeMap<Uuid, T>,
}

impl<T: Copy> LibraryGrants<T> {
    pub fn level(&self, library_id: Uuid) -> Option<T> {
        self.libraries
            .get(&library_id)
            .copied()
            .or(self.all_libraries)
    }
}

/// Durable per-device grants. A library entry overrides the wildcard per axis.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceGrants {
    pub control: bool,
    pub connection: LibraryGrants<ConnectionLevel>,
    pub sync: LibraryGrants<SyncLevel>,
    pub administer: bool,
}
impl DeviceGrants {
    pub fn my_device() -> Self {
        Self {
            control: true,
            connection: LibraryGrants {
                all_libraries: Some(ConnectionLevel::ViewEdit),
                libraries: BTreeMap::new(),
            },
            sync: LibraryGrants {
                all_libraries: Some(SyncLevel::PullPush),
                libraries: BTreeMap::new(),
            },
            administer: false,
        }
    }
    pub fn guest() -> Self {
        Self {
            control: true,
            connection: LibraryGrants {
                all_libraries: Some(ConnectionLevel::View),
                libraries: BTreeMap::new(),
            },
            sync: LibraryGrants {
                all_libraries: Some(SyncLevel::None),
                libraries: BTreeMap::new(),
            },
            administer: false,
        }
    }
    pub fn connection_level(&self, library_id: Uuid) -> ConnectionLevel {
        self.connection.level(library_id).unwrap_or_default()
    }
    pub fn sync_level(&self, library_id: Uuid) -> SyncLevel {
        self.sync.level(library_id).unwrap_or_default()
    }
}

#[derive(Debug, Error)]
pub enum Denial {
    #[error("device is not approved")]
    Untrusted,
    #[error("device has no grants")]
    MissingGrants,
    #[error("permission denied for {action:?}")]
    Action { action: Action },
    #[error("request is node-local or unclassified")]
    NodeLocal,
    #[error("permission snapshot unavailable")]
    Unavailable,
}

/// Version 2 stores separate Connection and Sync grants per library.
const GRANTS_VERSION: u32 = 2;

#[derive(Clone, Serialize, Deserialize)]
struct GrantsFile {
    version: u32,
    devices: BTreeMap<String, DeviceGrants>,
}

/// One process-wide policy. Disk writes are serialized; checks only read the snapshot.
pub struct RuneAccessPolicy {
    path: PathBuf,
    snapshot: RwLock<GrantsFile>,
    writer: Mutex<()>,
    changes: watch::Sender<u64>,
}
impl RuneAccessPolicy {
    pub fn load(config: &Path, users: &[UserSummary]) -> Result<Self> {
        let path = config.join("grants.json");
        let fresh = || GrantsFile {
            version: GRANTS_VERSION,
            devices: BTreeMap::new(),
        };
        // A missing file is a node that predates grants: approved devices keep
        // full access. A file that cannot be read as the current version is
        // reset, and its approved devices fall back to the least-privileged
        // preset rather than gaining access they may never have had.
        let (mut file, reset) = match fs::read(&path) {
            Ok(data) => match serde_json::from_slice::<GrantsFile>(&data)
                .ok()
                .filter(|file| file.version == GRANTS_VERSION)
            {
                Some(file) => (file, false),
                None => {
                    log::warn!("Device grants file is not in the current format; resetting it");
                    (fresh(), true)
                }
            },
            Err(error) if error.kind() == ErrorKind::NotFound => (fresh(), false),
            Err(error) => return Err(error).context("Read device grants"),
        };
        file.devices.retain(|fingerprint, _| {
            users
                .iter()
                .any(|user| user.fingerprint == *fingerprint && user.status == UserStatus::Approved)
        });
        for user in users
            .iter()
            .filter(|user| user.status == UserStatus::Approved)
        {
            file.devices
                .entry(user.fingerprint.clone())
                .or_insert_with(|| {
                    if reset {
                        DeviceGrants::guest()
                    } else {
                        DeviceGrants::my_device()
                    }
                });
        }
        let policy = Self {
            path,
            snapshot: RwLock::new(file.clone()),
            writer: Mutex::new(()),
            changes: watch::channel(0).0,
        };
        policy.persist(&file)?;
        Ok(policy)
    }
    fn persist(&self, file: &GrantsFile) -> Result<()> {
        let temp = self.path.with_extension("json.tmp");
        let result = (|| {
            fs::write(&temp, serde_json::to_vec_pretty(file)?).context("Write device grants")?;
            fs::rename(&temp, &self.path).context("Replace device grants")
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
    fn update(&self, change: impl FnOnce(&mut BTreeMap<String, DeviceGrants>)) -> Result<()> {
        let _writer = self
            .writer
            .lock()
            .map_err(|_| anyhow::anyhow!("Grants writer poisoned"))?;
        let mut next = self
            .snapshot
            .read()
            .map_err(|_| anyhow::anyhow!("Grants snapshot poisoned"))?
            .clone();
        change(&mut next.devices);
        self.persist(&next)?;
        *self
            .snapshot
            .write()
            .map_err(|_| anyhow::anyhow!("Grants snapshot poisoned"))? = next;
        self.changes
            .send_modify(|generation| *generation = generation.wrapping_add(1));
        Ok(())
    }
    pub fn grants(&self, fingerprint: &str) -> Result<DeviceGrants> {
        self.snapshot
            .read()
            .map_err(|_| anyhow::anyhow!("Grants snapshot poisoned"))?
            .devices
            .get(fingerprint)
            .cloned()
            .context("Device has no grants")
    }
    pub fn set(&self, fingerprint: &str, grants: DeviceGrants) -> Result<()> {
        self.update(|devices| {
            devices.insert(fingerprint.to_owned(), grants);
        })
    }
    pub fn trust_changed(&self, fingerprint: &str, status: Option<&UserStatus>) -> Result<()> {
        self.update(|devices| {
            if status == Some(&UserStatus::Approved) {
                devices
                    .entry(fingerprint.to_owned())
                    .or_insert_with(DeviceGrants::my_device);
            } else {
                devices.remove(fingerprint);
            }
        })
    }
}
impl AccessPolicy for RuneAccessPolicy {
    type Action = Action;
    type Denial = Denial;
    fn check(&self, peer: &PeerIdentity, action: &Action) -> Result<(), Denial> {
        let snapshot = self.snapshot.read().map_err(|_| Denial::Unavailable)?;
        let grants = snapshot
            .devices
            .get(&peer.fingerprint)
            .ok_or(Denial::MissingGrants)?;
        let allowed = match action {
            Action::Control => grants.control,
            Action::ConnectionView { library_id } => {
                grants.connection_level(*library_id) >= ConnectionLevel::View
            }
            Action::ConnectionEdit { library_id } => {
                grants.connection_level(*library_id) >= ConnectionLevel::ViewEdit
            }
            Action::SyncPull { library_id } => grants.sync_level(*library_id) >= SyncLevel::Pull,
            Action::SyncPush { library_id } => {
                grants.sync_level(*library_id) >= SyncLevel::PullPush
            }
            Action::Administer => grants.administer,
        };
        if allowed {
            Ok(())
        } else {
            Err(Denial::Action {
                action: action.clone(),
            })
        }
    }
    fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
}

#[cfg(test)]
mod load_tests {
    use super::{DeviceGrants, RuneAccessPolicy};
    use discovery::{
        server::{UserStatus, UserSummary},
        utils::DeviceType,
    };
    use std::time::SystemTime;

    fn approved(fingerprint: &str) -> UserSummary {
        UserSummary {
            alias: fingerprint.to_owned(),
            fingerprint: fingerprint.to_owned(),
            device_model: String::new(),
            device_type: DeviceType::Desktop,
            status: UserStatus::Approved,
            add_time: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn a_missing_file_keeps_approved_devices_fully_trusted() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let policy = RuneAccessPolicy::load(dir.path(), &[approved("a")])?;
        assert_eq!(policy.grants("a")?, DeviceGrants::my_device());
        Ok(())
    }

    #[test]
    fn an_unreadable_file_resets_approved_devices_to_guest() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::write(
            dir.path().join("grants.json"),
            br#"{"version":1,"devices":{}}"#,
        )?;
        let policy = RuneAccessPolicy::load(dir.path(), &[approved("a")])?;
        assert_eq!(policy.grants("a")?, DeviceGrants::guest());
        Ok(())
    }
}
