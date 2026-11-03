use std::{
    collections::BTreeMap,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Mutex, RwLock},
};

use anyhow::{Context, Result, ensure};
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
    /// Browsing a library through a control attachment (queries, covers,
    /// metadata), so a controller can pick songs to queue. Control grants it;
    /// it never allows pulling a replica or downloading audio files.
    Browse {
        library_id: Uuid,
    },
    LibraryRead {
        library_id: Uuid,
    },
    LibraryWrite {
        library_id: Uuid,
    },
    Administer,
}

/// Ordered library access. Control does not raise it; see `Action::Browse`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LibraryLevel {
    #[default]
    NotShared,
    Read,
    ReadWrite,
}

/// Durable per-device grants. A library entry overrides the wildcard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceGrants {
    pub control: bool,
    pub all_libraries: Option<LibraryLevel>,
    pub libraries: BTreeMap<Uuid, LibraryLevel>,
    pub administer: bool,
}
impl DeviceGrants {
    pub fn my_device() -> Self {
        Self {
            control: true,
            all_libraries: Some(LibraryLevel::ReadWrite),
            libraries: BTreeMap::new(),
            administer: false,
        }
    }
    pub fn guest() -> Self {
        Self {
            control: true,
            all_libraries: None,
            libraries: BTreeMap::new(),
            administer: false,
        }
    }
    pub fn level(&self, library_id: Uuid) -> LibraryLevel {
        self.libraries
            .get(&library_id)
            .copied()
            .or(self.all_libraries)
            .unwrap_or_default()
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
        let mut file = match fs::read(&path) {
            Ok(data) => {
                serde_json::from_slice::<GrantsFile>(&data).context("Invalid grants file")?
            }
            Err(error) if error.kind() == ErrorKind::NotFound => GrantsFile {
                version: 1,
                devices: BTreeMap::new(),
            },
            Err(error) => return Err(error).context("Read device grants"),
        };
        ensure!(
            file.version == 1,
            "Unsupported grants version {}",
            file.version
        );
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
                .or_insert_with(DeviceGrants::my_device);
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
            Action::Browse { library_id } => {
                grants.control || grants.level(*library_id) >= LibraryLevel::Read
            }
            Action::LibraryRead { library_id } => grants.level(*library_id) >= LibraryLevel::Read,
            Action::LibraryWrite { library_id } => {
                grants.level(*library_id) >= LibraryLevel::ReadWrite
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
