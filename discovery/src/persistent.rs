use std::fmt::Debug;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;
use tokio::sync::{broadcast, RwLock, RwLockReadGuard};

#[derive(Error, Debug)]
pub enum PersistenceError {
    #[error("File I/O error while {context}: {source}")]
    Io {
        source: std::io::Error,
        context: String,
    },
    #[error("Serialization error")]
    Serialization(#[from] toml::ser::Error),
    #[error("Deserialization error")]
    Deserialization(#[from] toml::de::Error),
    #[error("Filesystem watcher error during {action}: {source}")]
    Watcher {
        source: notify::Error,
        action: String,
    },
}

#[derive(Debug)]
pub struct PersistentDataManager<T> {
    /// The managed data wrapped in a thread-safe RwLock
    data: Arc<RwLock<T>>,
    /// Path to the persistent storage file
    file_path: PathBuf,
    /// File system watcher to detect external changes
    watcher: Arc<RwLock<RecommendedWatcher>>,
    /// Broadcast channel to notify subscribers of data changes
    change_tx: broadcast::Sender<T>,
}

impl<T> PersistentDataManager<T>
where
    T: Serialize + DeserializeOwned + Default + Send + Sync + Clone + Debug + 'static,
{
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, PersistenceError> {
        let path = path.as_ref();
        let file_path = path.to_path_buf();

        // Ensure parent directory exists
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PersistenceError::Io {
                source: e,
                context: format!("creating parent directory {}", parent.display()),
            })?;
        }

        // Initialize data from file or create new
        let initial_data = if file_path.exists() {
            let content =
                std::fs::read_to_string(&file_path).map_err(|e| PersistenceError::Io {
                    source: e,
                    context: format!("reading file {}", file_path.display()),
                })?;
            toml::from_str(&content)?
        } else {
            let default_data = T::default();
            let content = toml::to_string(&default_data)?;
            std::fs::write(&file_path, &content).map_err(|e| PersistenceError::Io {
                source: e,
                context: format!("writing to file {}", file_path.display()),
            })?;
            default_data
        };

        // Set up file monitoring
        let (fs_tx, fs_rx) = mpsc::channel();
        let mut watcher =
            notify::recommended_watcher(fs_tx).map_err(|e| PersistenceError::Watcher {
                source: e,
                action: "creating watcher".to_string(),
            })?;
        watcher
            .watch(&file_path, RecursiveMode::NonRecursive)
            .map_err(|e| PersistenceError::Watcher {
                source: e,
                action: format!("watching file {}", file_path.display()),
            })?;

        // Create broadcast channel for change notifications
        let (change_tx, _) = broadcast::channel::<T>(128);

        let manager = Self {
            data: Arc::new(RwLock::new(initial_data)),
            file_path,
            watcher: Arc::new(RwLock::new(watcher)),
            change_tx,
        };

        // Start the file monitoring thread
        manager.start_file_watcher(fs_rx);
        Ok(manager)
    }

    /// Starts a background thread that monitors file changes and updates the in-memory data.
    ///
    /// Implements debouncing to prevent rapid successive updates and handles file reading errors gracefully.
    fn start_file_watcher(&self, rx: mpsc::Receiver<notify::Result<notify::Event>>) {
        let data_clone = self.data.clone();
        let path_clone = self.file_path.clone();
        let tx_clone = self.change_tx.clone();

        std::thread::spawn(move || {
            for event in rx {
                match event {
                    Ok(event) if matches!(event.kind, EventKind::Modify(_)) => {
                        if event.paths.iter().any(|p| p == &path_clone) {
                            std::thread::sleep(Duration::from_millis(100));
                            match std::fs::read_to_string(&path_clone) {
                                Ok(content) => {
                                    if let Ok(new_data) = toml::from_str::<T>(&content) {
                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                        rt.block_on(async {
                                            let mut data = data_clone.write().await;
                                            *data = new_data.clone();
                                            let _ = tx_clone.send(new_data);
                                        });
                                    }
                                }
                                Err(e) => eprintln!(
                                    "Error reading file during watch: {}",
                                    PersistenceError::Io {
                                        source: e,
                                        context: format!(
                                            "reading file during watch {}",
                                            path_clone.display()
                                        )
                                    }
                                ),
                            }
                        }
                    }
                    Err(e) => eprintln!(
                        "Watcher error: {}",
                        PersistenceError::Watcher {
                            source: e,
                            action: "monitoring file changes".to_string()
                        }
                    ),
                    _ => {}
                }
            }
        });
    }

    /// Saves the current state to the persistent storage file.
    ///
    /// Temporarily disables file watching during the save operation to prevent recursive updates.
    pub async fn save(&self) -> Result<(), PersistenceError> {
        let data = self.data.read().await;
        self.save_internal(&*data).await
    }

    async fn save_internal(&self, data: &T) -> Result<(), PersistenceError> {
        println!("SAVING!: {:#?}", data);
        let content = toml::to_string(data)?;

        // Pause watching
        self.watcher
            .write()
            .await
            .unwatch(&self.file_path)
            .map_err(|e| PersistenceError::Watcher {
                source: e,
                action: "unwatching file".to_string(),
            })?;

        // Write to file
        std::fs::write(&self.file_path, &content).map_err(|e| PersistenceError::Io {
            source: e,
            context: format!("writing to file {}", self.file_path.display()),
        })?;

        // Resume watching
        self.watcher
            .write()
            .await
            .watch(&self.file_path, RecursiveMode::NonRecursive)
            .map_err(|e| PersistenceError::Watcher {
                source: e,
                action: "rewatching file".to_string(),
            })?;

        Ok(())
    }

    /// Obtains a read lock on the managed data.
    pub async fn read(&self) -> RwLockReadGuard<'_, T> {
        self.data.read().await
    }

    // People may need this later?
    // /// Obtains a write lock on the managed data.
    // pub async fn write(&self) -> RwLockWriteGuard<'_, T> {
    //     self.data.write().await
    // }

    /// Updates the managed data using the provided update function and saves the changes.
    ///
    /// # Arguments
    /// * `updater` - A function that modifies the data and returns a Result
    ///
    /// # Returns
    /// * `Result<R>` - The result of the update operation
    pub async fn update<F, Fut, R, E>(&self, updater: F) -> Result<R, E>
    where
        F: FnOnce(T) -> Fut,
        Fut: Future<Output = Result<(T, R), E>>,
        E: From<PersistenceError>,
    {
        let mut data = self.data.write().await;
        let current_data = (*data).clone();
        let (new_data, result) = updater(current_data).await?;
        *data = new_data;
        self.save_internal(&*data).await.map_err(E::from)?;
        Ok(result)
    }

    /// Returns a receiver for change notifications.
    ///
    /// Subscribers will receive notifications whenever the managed data changes,
    /// either through direct updates or file changes.
    pub fn subscribe(&self) -> broadcast::Receiver<T> {
        self.change_tx.subscribe()
    }
}
