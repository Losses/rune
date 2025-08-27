use std::fmt::Debug;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;
use tokio::sync::{RwLock, RwLockReadGuard, broadcast};

/// Represents errors that can occur during persistent data management.
///
/// This enum categorizes different types of errors encountered when working with
/// persistent data, such as file I/O issues, serialization problems, or filesystem
/// watcher errors.
#[derive(Error, Debug)]
pub enum PersistenceError {
    /// Represents file input/output errors.
    ///
    /// This error variant is used when operations like reading from or writing to a file fail.
    /// It includes the underlying `std::io::Error` and a context string to provide more
    /// information about where the error occurred.
    #[error("File I/O error while {context}: {source}")]
    Io {
        source: std::io::Error,
        context: String,
    },
    /// Represents errors during TOML serialization.
    ///
    /// This error variant occurs when converting data into a TOML format string fails.
    #[error("Serialization error")]
    Serialization(#[from] toml::ser::Error),
    /// Represents errors during TOML deserialization.
    ///
    /// This error variant occurs when parsing a TOML format string back into data fails.
    #[error("Deserialization error")]
    Deserialization(#[from] toml::de::Error),
    /// Represents errors from the filesystem watcher.
    ///
    /// This error variant is used when the filesystem watcher encounters an issue, such as
    /// failing to set up watching or during event processing. It includes the underlying
    /// `notify::Error` and an action string to describe what the watcher was doing when the error occurred.
    #[error("Filesystem watcher error during {action}: {source}")]
    Watcher {
        source: notify::Error,
        action: String,
    },
}

/// Manages persistent data, providing file-based storage with change monitoring.
///
/// `PersistentDataManager` handles loading data from a TOML file, saving data back to the file,
/// and monitoring the file for external changes. It uses a `RwLock` to allow concurrent reads
/// and exclusive writes to the managed data, and a filesystem watcher to detect and react to
/// modifications made by other processes or users. It also provides a broadcast channel for
/// notifying subscribers of data changes.
#[derive(Debug)]
pub struct PersistentDataManager<T> {
    /// The managed data, protected by a `RwLock` for concurrent access.
    data: Arc<RwLock<T>>,
    /// The path to the file used for persistent storage of the data.
    file_path: PathBuf,
    /// Filesystem watcher for monitoring changes to the persistent data file.
    watcher: Arc<RwLock<RecommendedWatcher>>,
    /// Broadcast channel sender to notify subscribers of data changes.
    change_tx: broadcast::Sender<T>,
}

impl<T> PersistentDataManager<T>
where
    T: Serialize + DeserializeOwned + Default + Send + Sync + Clone + Debug + 'static,
{
    /// Creates a new `PersistentDataManager` instance, initializing data from a file or default value.
    ///
    /// This constructor attempts to load data from the specified file path. If the file exists,
    /// it deserializes the TOML content into the managed data `T`. If the file does not exist,
    /// it creates a new file and initializes it with the default value of `T`. It also sets up
    /// a filesystem watcher to monitor the file for external changes and a broadcast channel
    /// for change notifications.
    ///
    /// # Arguments
    /// * `path` - The file path for persistent storage. The parent directory will be created if it does not exist.
    ///
    /// # Returns
    /// `Result<Self, PersistenceError>` - A `Result` containing the new `PersistentDataManager` instance,
    ///                                     or a `PersistenceError` if initialization fails.
    ///
    /// # Errors
    /// Returns `PersistenceError` if:
    /// - Parent directory creation fails.
    /// - File reading or writing fails.
    /// - TOML serialization or deserialization fails.
    /// - Filesystem watcher initialization or file watching setup fails.
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

    /// Starts a background thread to monitor the persistent file for changes.
    ///
    /// This private method spawns a new thread that listens for filesystem events from the `notify` watcher.
    /// When a modify event is detected for the watched file, it reads the file content, deserializes it,
    /// and updates the in-memory data. It also sends a notification through the broadcast channel to inform
    /// subscribers about the data change. A debouncing mechanism is implemented with a short sleep to avoid
    /// reacting to intermediate file states during rapid writes.
    ///
    /// # Arguments
    /// * `rx` - The `mpsc::Receiver` for filesystem events from the `notify` watcher.
    fn start_file_watcher(&self, rx: mpsc::Receiver<notify::Result<notify::Event>>) {
        let data_clone = self.data.clone();
        let path_clone = self.file_path.clone();
        let tx_clone = self.change_tx.clone();

        std::thread::spawn(move || {
            for event in rx {
                match event {
                    Ok(event) if matches!(event.kind, EventKind::Modify(_)) => {
                        if event.paths.iter().any(|p| p == &path_clone) {
                            std::thread::sleep(Duration::from_millis(100)); // Debounce to avoid reacting to partial saves
                            match std::fs::read_to_string(&path_clone) {
                                Ok(content) => {
                                    if let Ok(new_data) = toml::from_str::<T>(&content) {
                                        let rt = tokio::runtime::Runtime::new().unwrap(); // Create a temporary runtime
                                        rt.block_on(async {
                                            let mut data = data_clone.write().await;
                                            *data = new_data.clone(); // Update in-memory data
                                            let _ = tx_clone.send(new_data); // Notify subscribers of change
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
                    _ => {} // Ignore other event types
                }
            }
        });
    }

    /// Saves the current in-memory data to the persistent storage file.
    ///
    /// This method serializes the current data into TOML format and writes it to the file specified
    /// during initialization. It temporarily stops file watching during the save operation to prevent
    /// recursive update triggers if the save operation itself causes a file modification event.
    ///
    /// # Returns
    /// `Result<(), PersistenceError>` - A `Result` indicating success or failure due to a `PersistenceError`.
    ///
    /// # Errors
    /// Returns `PersistenceError` if:
    /// - TOML serialization fails.
    /// - File writing fails.
    /// - Filesystem watcher unwatching or re-watching fails.
    pub async fn save(&self) -> Result<(), PersistenceError> {
        let data = self.data.read().await; // Acquire read lock to get current data
        self.save_internal(&*data).await // Call internal save with dereferenced data
    }

    /// Internal save function that handles the actual saving process with watcher pausing.
    ///
    /// This asynchronous function performs the core logic of saving data to file, including
    /// serialization, file writing, and pausing/resuming the filesystem watcher to avoid
    /// self-induced update loops.
    ///
    /// # Arguments
    /// * `data` - A reference to the data to be saved.
    ///
    /// # Returns
    /// `Result<(), PersistenceError>` - A `Result` indicating success or failure due to a `PersistenceError`.
    async fn save_internal(&self, data: &T) -> Result<(), PersistenceError> {
        let content = toml::to_string(data)?; // Serialize data to TOML string

        // Pause watching to prevent reacting to self-modification
        self.watcher
            .write()
            .await
            .unwatch(&self.file_path)
            .map_err(|e| PersistenceError::Watcher {
                source: e,
                action: "unwatching file".to_string(),
            })?;

        // Write serialized content to file
        std::fs::write(&self.file_path, &content).map_err(|e| PersistenceError::Io {
            source: e,
            context: format!("writing to file {}", self.file_path.display()),
        })?;

        // Resume watching after save is complete
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

    /// Obtains a read lock on the managed data, allowing concurrent read access.
    ///
    /// This method asynchronously acquires a read lock on the internally managed data and returns
    /// a `RwLockReadGuard`. This guard allows read-only access to the data and will release the lock
    /// when it goes out of scope. Multiple read locks can be held simultaneously.
    ///
    /// # Returns
    /// `RwLockReadGuard<'_, T>` - A read guard for accessing the managed data.
    pub async fn read(&self) -> RwLockReadGuard<'_, T> {
        self.data.read().await
    }

    // People may need this later?
    // /// Obtains a write lock on the managed data.
    // pub async fn write(&self) -> RwLockWriteGuard<'_, T> {
    //     self.data.write().await
    // }

    /// Updates the managed data using a provided function and automatically saves the changes.
    ///
    /// This method takes a closure `updater` that operates on a clone of the current data. The closure
    /// should modify the data and return both the modified data and a result value. After the updater
    /// function completes successfully, this method updates the managed data with the modified version
    /// and saves it to the persistent file.
    ///
    /// # Arguments
    /// * `updater` - A closure that takes a clone of the current data, modifies it, and returns
    ///   a `Result` containing a tuple of the updated data and a result value of type `R`.
    ///
    /// # Returns
    /// * `Result<R, E>` - The result of the update operation as returned by the `updater` closure.
    ///   The error type `E` must be able to be created from `PersistenceError`.
    ///
    /// # Type Parameters
    /// * `F` - The type of the updater closure.
    /// * `Fut` - The type of the future returned by the updater closure.
    /// * `R` - The type of the result value returned by the updater closure.
    /// * `E` - The error type that can be returned by the updater closure and must be able to handle `PersistenceError`.
    pub async fn update<F, Fut, R, E>(&self, updater: F) -> Result<R, E>
    where
        F: FnOnce(T) -> Fut,
        Fut: Future<Output = Result<(T, R), E>>,
        E: From<PersistenceError>,
    {
        let mut data = self.data.write().await; // Acquire exclusive write lock
        let current_data = (*data).clone(); // Clone current data for the updater function
        let (new_data, result) = updater(current_data).await?; // Call the updater function
        *data = new_data.clone(); // Update managed data with the new data
        let _ = self.change_tx.send(new_data);
        self.save_internal(&*data).await.map_err(E::from)?; // Save the updated data to file
        Ok(result) // Return the result from the updater function
    }

    /// Returns a broadcast receiver for change notifications.
    ///
    /// This method provides a `broadcast::Receiver` that can be used to subscribe to notifications
    /// whenever the managed data is changed. Changes can originate from direct updates through the
    /// `update` method or from external modifications detected by the filesystem watcher.
    ///
    /// # Returns
    /// `broadcast::Receiver<T>` - A receiver for data change notifications. Each subscriber will
    ///                             receive a clone of the data whenever it is updated.
    pub fn subscribe(&self) -> broadcast::Receiver<T> {
        self.change_tx.subscribe()
    }
}
