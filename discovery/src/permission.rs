use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;

use log::error;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{broadcast, RwLock};

use crate::persistent::{PersistenceError, PersistentDataManager};
use crate::utils::DeviceType;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum UserStatus {
    Approved,
    Pending,
    Blocked,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct User {
    pub public_key: String,
    fingerprint: String,
    pub alias: String,
    device_model: String,
    device_type: DeviceType,
    pub status: UserStatus,
}

#[derive(Debug, Clone)]
pub struct UserSummary {
    pub alias: String,
    pub fingerprint: String,
    pub device_model: String,
    pub device_type: DeviceType,
    pub status: UserStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionList {
    pub users: HashMap<String, User>, // Fingerprint -> User
}

#[derive(Error, Debug)]
pub enum PermissionError {
    #[error("Persistence error: {0}")]
    Persistence(#[from] PersistenceError),
    #[error("User not found")]
    UserNotFound,
    #[error("User already exists")]
    UserAlreadyExists,
    #[error("Path is not a directory")]
    NotADirectory,
    #[error("Path is invalid")]
    InvalidPath,
}

#[derive(Debug)]
pub struct PermissionManager {
    storage: PersistentDataManager<PermissionList>,
    ip_applications: Arc<RwLock<HashMap<String, VecDeque<String>>>>,
}

impl PermissionManager {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, PermissionError> {
        let storage_path = path.as_ref().join(".known-clients");
        let storage =
            PersistentDataManager::new(storage_path).map_err(PermissionError::Persistence)?;

        Ok(Self {
            storage,
            ip_applications: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn list_users(&self) -> Vec<UserSummary> {
        self.storage
            .read()
            .await
            .users
            .values()
            .map(|user| UserSummary {
                alias: user.alias.clone(),
                fingerprint: user.fingerprint.clone(),
                device_model: user.device_model.clone(),
                device_type: user.device_type,
                status: user.status.clone(),
            })
            .collect()
    }

    pub async fn add_user(
        &self,
        public_key: String,
        fingerprint: String,
        alias: String,
        device_model: String,
        device_type: DeviceType,
        ip: String,
    ) -> Result<(), PermissionError> {
        self.storage
            .update::<_, (), PermissionError>(|permissions| {
                if permissions.users.contains_key(&fingerprint) {
                    return Err(PermissionError::UserAlreadyExists);
                }

                let mut ip_apps = self.ip_applications.blocking_write();
                let queue = ip_apps.entry(ip).or_default();

                if queue.len() >= 5 {
                    if let Some(old_key) = queue.pop_front() {
                        permissions.users.remove(&old_key);
                    }
                }
                queue.push_back(fingerprint.clone());

                permissions.users.insert(
                    fingerprint.clone(),
                    User {
                        public_key,
                        fingerprint,
                        alias,
                        device_model,
                        device_type,
                        status: UserStatus::Pending,
                    },
                );
                Ok::<(), PermissionError>(())
            })
            .await
    }

    pub async fn verify_by_fingerprint(&self, fingerprint: &str) -> Option<User> {
        self.storage.read().await.users.get(fingerprint).cloned()
    }

    pub async fn verify_by_public_key(&self, public_key: &str) -> Option<User> {
        let permissions = self.storage.read().await;
        permissions
            .users
            .values()
            .find(|user| user.public_key == public_key)
            .cloned()
    }

    pub async fn change_user_status(
        &self,
        fingerprint: &str,
        new_status: UserStatus,
    ) -> Result<(), PermissionError> {
        self.storage
            .update(|permissions| {
                let user = permissions
                    .users
                    .get_mut(fingerprint)
                    .ok_or(PermissionError::UserNotFound)?;
                user.status = new_status.clone();
                Ok::<(), PermissionError>(())
            })
            .await
    }

    pub async fn remove_user(&self, fingerprint: &str) -> Result<(), PermissionError> {
        self.storage
            .update(|permissions| {
                if permissions.users.remove(fingerprint).is_none() {
                    Err(PermissionError::UserNotFound)
                } else {
                    Ok(())
                }
            })
            .await
    }

    pub async fn find_by_device_type(&self, device_type: DeviceType) -> Vec<UserSummary> {
        self.storage
            .read()
            .await
            .users
            .values()
            .filter(|u| u.device_type == device_type)
            .map(|user| UserSummary {
                alias: user.alias.clone(),
                fingerprint: user.fingerprint.clone(),
                device_model: user.device_model.clone(),
                device_type: user.device_type,
                status: user.status.clone(),
            })
            .collect()
    }

    pub async fn get_pending_count(&self) -> usize {
        self.storage
            .read()
            .await
            .users
            .values()
            .filter(|u| u.status == UserStatus::Pending)
            .count()
    }

    pub fn subscribe_changes(&self) -> broadcast::Receiver<PermissionList> {
        self.storage.subscribe()
    }
}
