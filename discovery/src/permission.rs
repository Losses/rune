use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::Path;
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum UserStatus {
    Approved,
    Pending,
    Blocked,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct User {
    public_key: String,
    fingerprint: String,
    alias: String,
    device_model: String,
    status: UserStatus,
}

#[derive(Debug, Clone)]
pub struct UserSummary {
    pub alias: String,
    pub fingerprint: String,
    pub device_model: String,
    pub status: UserStatus,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PermissionList {
    users: HashMap<String, User>,
}

#[derive(Error, Debug)]
pub enum PermissionError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("TOML serialization error: {0}")]
    TomlError(#[from] toml::ser::Error),
    #[error("TOML deserialization error: {0}")]
    FromTomlError(#[from] toml::de::Error),
    #[error("User not found")]
    UserNotFound,
    #[error("User already exists")]
    UserAlreadyExists,
}

pub struct PermissionManager {
    file_path: String,
    permissions: RwLock<PermissionList>,
    ip_applications: RwLock<HashMap<String, VecDeque<String>>>,
}

impl PermissionManager {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, PermissionError> {
        let file_path = path.as_ref().to_str().unwrap().to_string();
        let permissions = if path.as_ref().exists() {
            let content = fs::read_to_string(&path)?;
            toml::from_str(&content)?
        } else {
            PermissionList {
                users: HashMap::new(),
            }
        };

        Ok(Self {
            file_path,
            permissions: RwLock::new(permissions),
            ip_applications: RwLock::new(HashMap::new()),
        })
    }

    async fn save(&self) -> Result<(), PermissionError> {
        let content = toml::to_string(&*self.permissions.read().await)?;
        tokio::fs::write(&self.file_path, content).await?;
        Ok(())
    }

    pub async fn list_users(&self) -> Vec<UserSummary> {
        self.permissions
            .read()
            .await
            .users
            .values()
            .map(|user| UserSummary {
                alias: user.alias.clone(),
                fingerprint: user.fingerprint.clone(),
                device_model: user.device_model.clone(),
                status: user.status.clone(),
            })
            .collect()
    }

    pub async fn verify_by_public_key(&self, public_key: &str) -> Option<User> {
        self.permissions.read().await.users.get(public_key).cloned()
    }

    pub async fn verify_by_fingerprint(&self, fingerprint: &str) -> Option<User> {
        let permissions = self.permissions.read().await;
        permissions
            .users
            .values()
            .find(|user| user.fingerprint == fingerprint)
            .cloned()
    }

    pub async fn add_user(
        &self,
        public_key: String,
        fingerprint: String,
        alias: String,
        device_model: String,
        ip: String,
    ) -> Result<(), PermissionError> {
        let mut permissions = self.permissions.write().await;
        if permissions.users.contains_key(&public_key) {
            return Err(PermissionError::UserAlreadyExists);
        }

        let mut ip_apps = self.ip_applications.write().await;
        let queue = ip_apps.entry(ip).or_default();
        if queue.len() >= 5 {
            if let Some(old_key) = queue.pop_front() {
                permissions.users.remove(&old_key);
            }
        }
        queue.push_back(public_key.clone());

        let user = User {
            public_key: public_key.clone(),
            fingerprint,
            alias,
            device_model,
            status: UserStatus::Pending,
        };

        permissions.users.insert(public_key, user);
        self.save().await?;
        Ok(())
    }

    pub async fn remove_user(&mut self, public_key: &str) -> Result<(), PermissionError> {
        if self
            .permissions
            .write()
            .await
            .users
            .remove(public_key)
            .is_none()
        {
            return Err(PermissionError::UserNotFound);
        }

        self.save().await?;
        Ok(())
    }

    pub async fn change_user_status(
        &mut self,
        public_key: &str,
        new_status: UserStatus,
    ) -> Result<(), PermissionError> {
        let mut permissions = self.permissions.write().await;
        let user = permissions
            .users
            .get_mut(public_key)
            .ok_or(PermissionError::UserNotFound)?;
        user.status = new_status;

        self.save().await?;
        Ok(())
    }
}
