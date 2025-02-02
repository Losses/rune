use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use thiserror::Error;

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
    permissions: PermissionList,
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
            permissions,
        })
    }

    fn save(&self) -> Result<(), PermissionError> {
        let content = toml::to_string(&self.permissions)?;
        fs::write(&self.file_path, content)?;
        Ok(())
    }

    pub fn list_users(&self) -> Vec<UserSummary> {
        self.permissions
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

    pub fn verify_by_public_key(&self, public_key: &str) -> Option<&User> {
        self.permissions.users.get(public_key)
    }

    pub fn verify_by_fingerprint(&self, fingerprint: &str) -> Option<&User> {
        self.permissions
            .users
            .values()
            .find(|user| user.fingerprint == fingerprint)
    }

    pub fn add_user(
        &mut self,
        public_key: String,
        fingerprint: String,
        alias: String,
        device_model: String,
    ) -> Result<(), PermissionError> {
        if self.permissions.users.contains_key(&public_key) {
            return Err(PermissionError::UserAlreadyExists);
        }

        let user = User {
            public_key: public_key.clone(),
            fingerprint,
            alias,
            device_model,
            status: UserStatus::Pending,
        };

        self.permissions.users.insert(public_key, user);
        self.save()?;
        Ok(())
    }

    pub fn remove_user(&mut self, public_key: &str) -> Result<(), PermissionError> {
        if self.permissions.users.remove(public_key).is_none() {
            return Err(PermissionError::UserNotFound);
        }
        self.save()?;
        Ok(())
    }

    pub fn change_user_status(
        &mut self,
        public_key: &str,
        new_status: UserStatus,
    ) -> Result<(), PermissionError> {
        let user = self
            .permissions
            .users
            .get_mut(public_key)
            .ok_or(PermissionError::UserNotFound)?;
        user.status = new_status;
        self.save()?;
        Ok(())
    }
}
