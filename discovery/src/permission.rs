use log::{error, info};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use thiserror::Error;
use tokio::sync::RwLock;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[error("Path is not a directory")]
    NotADirectory,
    #[error("Path is invalid")]
    InvalidPath,
    #[error("Watch error: {0}")]
    WatchError(String),
}

#[derive(Debug)]
pub struct PermissionManager {
    file_path: PathBuf,
    permissions: Arc<RwLock<PermissionList>>,
    ip_applications: RwLock<HashMap<String, VecDeque<String>>>,
    watcher: Arc<Mutex<RecommendedWatcher>>,
}

impl PermissionManager {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, PermissionError> {
        let path = path.as_ref();

        if !path.exists() {
            fs::create_dir_all(path)?;
        } else if !path.is_dir() {
            return Err(PermissionError::NotADirectory);
        }

        let file_path = path.join(".known-clients");
        info!("Initializing permission manager in: {:?}", file_path);

        let permissions = if file_path.exists() {
            info!("Permission path exists");
            if file_path.is_dir() {
                error!("Permission path is not a file");
                return Err(PermissionError::NotADirectory);
            }
            let content = fs::read_to_string(&file_path)?;
            toml::from_str(&content)?
        } else {
            info!("File does not exist, creating new with empty permissions");
            let permissions = PermissionList {
                users: HashMap::new(),
            };
            let content = toml::to_string(&permissions)?;
            fs::write(&file_path, content)?;
            permissions
        };

        let permissions = Arc::new(RwLock::new(permissions));

        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(tx).map_err(|e| {
            PermissionError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            ))
        })?;

        watcher
            .watch(Path::new(&file_path), RecursiveMode::NonRecursive)
            .map_err(|e| {
                PermissionError::IoError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                ))
            })?;

        let watcher = Arc::new(Mutex::new(watcher));

        let permissions_clone = permissions.clone();
        let file_path_clone = file_path.clone();

        std::thread::spawn(move || {
            for event in rx {
                match event {
                    Ok(event) => {
                        if let EventKind::Modify(_) = event.kind {
                            if event.paths.iter().any(|p| p == Path::new(&file_path_clone)) {
                                match fs::read_to_string(&file_path_clone) {
                                    Ok(content) => {
                                        match toml::from_str::<PermissionList>(&content) {
                                            Ok(new_permissions) => {
                                                let rt = tokio::runtime::Runtime::new().unwrap();
                                                rt.block_on(async {
                                                    let mut perms = permissions_clone.write().await;
                                                    *perms = new_permissions;
                                                });
                                            }
                                            Err(e) => {
                                                eprintln!(
                                                    "Failed to parse permissions file: {}",
                                                    e
                                                );
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to read permissions file: {}", e);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => eprintln!("Error watching file: {:?}", e),
                }
            }
        });

        Ok(Self {
            file_path,
            permissions,
            ip_applications: RwLock::new(HashMap::new()),
            watcher,
        })
    }

    async fn save(&self) -> Result<(), PermissionError> {
        {
            let mut watcher = self.watcher.lock().map_err(|e| {
                PermissionError::IoError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                ))
            })?;
            watcher.unwatch(Path::new(&self.file_path)).map_err(|e| {
                PermissionError::IoError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                ))
            })?;
        }

        let path = Path::new(&self.file_path);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let content = toml::to_string(&*self.permissions.read().await)?;
        tokio::fs::write(&self.file_path, content).await?;

        {
            let mut watcher = self.watcher.lock().map_err(|e| {
                PermissionError::IoError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                ))
            })?;
            watcher
                .watch(Path::new(&self.file_path), RecursiveMode::NonRecursive)
                .map_err(|e| {
                    PermissionError::IoError(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        e.to_string(),
                    ))
                })?;
        }

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
                device_type: user.device_type,
                status: user.status.clone(),
            })
            .collect()
    }

    pub async fn verify_by_fingerprint(&self, fingerprint: &str) -> Option<User> {
        self.permissions
            .read()
            .await
            .users
            .get(fingerprint)
            .cloned()
    }

    pub async fn verify_by_public_key(&self, public_key: &str) -> Option<User> {
        let permissions = self.permissions.read().await;
        permissions
            .users
            .values()
            .find(|user| user.public_key == public_key)
            .cloned()
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
        {
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
                fingerprint: fingerprint.clone(),
                alias,
                device_model,
                device_type,
                status: UserStatus::Pending,
            };

            permissions.users.insert(fingerprint, user);
        }

        self.save().await?;
        Ok(())
    }

    pub async fn change_user_status(
        &mut self,
        fingerprint: &str,
        new_status: UserStatus,
    ) -> Result<(), PermissionError> {
        {
            let mut permissions = self.permissions.write().await;
            let user = permissions
                .users
                .get_mut(fingerprint)
                .ok_or(PermissionError::UserNotFound)?;
            user.status = new_status;
        }

        self.save().await?;
        Ok(())
    }

    pub async fn remove_user(&mut self, fingerprint: &str) -> Result<(), PermissionError> {
        if self
            .permissions
            .write()
            .await
            .users
            .remove(fingerprint)
            .is_none()
        {
            return Err(PermissionError::UserNotFound);
        }

        self.save().await?;
        Ok(())
    }
}
