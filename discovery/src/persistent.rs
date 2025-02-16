use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use anyhow::{Context, Result};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::{broadcast, RwLock, RwLockReadGuard};

/// A thread-safe manager for persistent data that automatically handles file I/O and change notifications.
///
/// This struct provides functionality to:
/// - Read and write data to/from a TOML file
/// - Watch for file changes and automatically reload data
/// - Broadcast notifications when data changes
/// - Provide concurrent access through RwLock
///
/// Type parameter T must implement necessary traits for serialization and thread safety.
#[derive(Debug)]
pub struct PersistentDataManager<T> {
    /// The managed data wrapped in a thread-safe RwLock
    data: Arc<RwLock<T>>,
    /// Path to the persistent storage file
    file_path: PathBuf,
    /// File system watcher to detect external changes
    watcher: Arc<RwLock<RecommendedWatcher>>,
    /// Broadcast channel to notify subscribers of data changes
    change_tx: broadcast::Sender<()>,
}

impl<T> PersistentDataManager<T>
where
    T: Serialize + DeserializeOwned + Default + Send + Sync + 'static,
{
    /// Creates a new PersistentDataManager instance.
    ///
    /// # Arguments
    /// * `path` - Path to the storage file
    ///
    /// # Returns
    /// * `Result<Self>` - A new instance or an error if initialization fails
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let file_path = path.to_path_buf();

        // Ensure parent directory exists
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).context("Failed to create parent directories")?;
        }

        // Initialize data from file or create new
        let initial_data = if file_path.exists() {
            let content =
                std::fs::read_to_string(&file_path).context("Failed to read data file")?;
            toml::from_str(&content).context("Failed to parse data file")?
        } else {
            let default_data = T::default();
            let content =
                toml::to_string(&default_data).context("Failed to serialize default data")?;
            std::fs::write(&file_path, &content).context("Failed to write initial data file")?;
            default_data
        };

        // Set up file monitoring
        let (fs_tx, fs_rx) = mpsc::channel();
        let mut watcher =
            notify::recommended_watcher(fs_tx).context("Failed to create file watcher")?;
        watcher
            .watch(&file_path, RecursiveMode::NonRecursive)
            .context("Failed to start watching data file")?;

        // Create broadcast channel for change notifications
        let (change_tx, _) = broadcast::channel(16);

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
                            // Debounce: wait 100ms before reading
                            std::thread::sleep(Duration::from_millis(100));

                            if let Ok(content) = std::fs::read_to_string(&path_clone) {
                                if let Ok(new_data) = toml::from_str::<T>(&content) {
                                    let rt = tokio::runtime::Runtime::new().unwrap();
                                    rt.block_on(async {
                                        let mut data = data_clone.write().await;
                                        *data = new_data;
                                        let _ = tx_clone.send(());
                                    });
                                }
                            }
                        }
                    }
                    Err(e) => eprintln!("Watcher error: {:?}", e),
                    _ => {}
                }
            }
        });
    }

    /// Saves the current state to the persistent storage file.
    ///
    /// Temporarily disables file watching during the save operation to prevent recursive updates.
    pub async fn save(&self) -> Result<()> {
        let content =
            toml::to_string(&*self.data.read().await).context("Failed to serialize data")?;

        // Pause watching
        self.watcher
            .write()
            .await
            .unwatch(&self.file_path)
            .context("Failed to unwatch file")?;

        // Write to file
        std::fs::write(&self.file_path, &content).context("Failed to write data file")?;

        // Resume watching
        self.watcher
            .write()
            .await
            .watch(&self.file_path, RecursiveMode::NonRecursive)
            .context("Failed to rewatch file")?;

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
    pub async fn update<F, R, E>(&self, updater: F) -> Result<R>
    where
        F: FnOnce(&mut T) -> Result<R, E>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let mut data = self.data.write().await;
        let result = updater(&mut data).map_err(|e| anyhow::anyhow!(e))?;
        self.save().await?;
        Ok(result)
    }

    /// Returns a receiver for change notifications.
    ///
    /// Subscribers will receive notifications whenever the managed data changes,
    /// either through direct updates or file changes.
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.change_tx.subscribe()
    }
}
