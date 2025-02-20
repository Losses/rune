use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::Result;
use chrono::Utc;
use log::{error, info};
use tokio::{
    fs::{read_to_string, write},
    sync::{broadcast, Mutex, RwLock},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use toml::Value;

use crate::{
    udp_multicast::{DiscoveredDevice, DiscoveryServiceImplementation},
    utils::DeviceInfo,
};

/// Manages persistent storage and in-memory cache of discovered devices.
/// Automatically handles data expiration and file I/O operations.
#[derive(Clone)]
pub struct DiscoveryStore {
    /// Path to the persistent storage file
    path: Option<PathBuf>,
    /// In-memory device list with thread-safe access
    devices: Arc<RwLock<HashMap<String, DiscoveredDevice>>>,
}

impl DiscoveryStore {
    /// Creates a new DiscoveryStore instance with the specified base directory.
    /// The actual storage file will be created at `{base_dir}/.discovered`.
    pub fn new<P: AsRef<Path>>(base_path: Option<P>) -> Self {
        Self {
            path: base_path.map(|base_path| base_path.as_ref().join(".discovered")),
            devices: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Loads devices from persistent storage into memory.
    /// Creates an empty list if the storage file doesn't exist.
    pub async fn load(&self) -> Result<Vec<DiscoveredDevice>> {
        if let Some(path) = &self.path {
            if !path.exists() {
                return Ok(Vec::new());
            }

            let content = read_to_string(path).await?;
            let parsed: Value = toml::from_str(&content)?;

            let devices = parsed
                .get("devices")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let devices: Vec<DiscoveredDevice> = devices
                .into_iter()
                .filter_map(|v| v.try_into().ok())
                .collect();

            let mut map = HashMap::new();
            for device in devices {
                map.insert(device.fingerprint.clone(), device);
            }

            *self.devices.write().await = map.clone();
            Ok(map.values().cloned().collect())
        } else {
            Ok(Vec::new())
        }
    }

    /// Persists the current device list to storage, automatically removing
    /// devices that haven't been seen in the last 30 seconds.
    pub async fn save(&self, devices: &[DiscoveredDevice]) -> Result<()> {
        if let Some(path) = &self.path {
            let filtered: Vec<_> = devices
                .iter()
                .filter(|d| {
                    let elapsed = Utc::now().signed_duration_since(d.last_seen);
                    elapsed.to_std().unwrap_or(Duration::MAX) < Duration::from_secs(120)
                })
                .cloned()
                .collect();

            let mut root = toml::map::Map::new();
            root.insert(
                "devices".to_string(),
                Value::Array(
                    filtered
                        .iter()
                        .map(|d| toml::Value::try_from(d).unwrap())
                        .collect(),
                ),
            );

            let content = toml::to_string(&Value::Table(root)).inspect_err(|_| {
                error!("TOML serialization failed: {:?}", filtered);
            })?;

            write(path, content).await?;
        }
        Ok(())
    }

    /// Removes expired devices from both memory and persistent storage
    pub async fn prune_expired(&self) -> Result<()> {
        info!("Pruning expired discovery entries");

        let expired_devices = {
            let mut devices = self.devices.write().await;
            let now = Utc::now();

            devices.retain(|_, d| {
                let elapsed = now.signed_duration_since(d.last_seen);
                elapsed.to_std().unwrap_or(Duration::MAX) < Duration::from_secs(30)
            });

            devices.values().cloned().collect::<Vec<_>>()
        };

        self.save(&expired_devices).await
    }

    /// Updates or inserts a device into the store and persists changes
    pub async fn update_device(&self, device: DiscoveredDevice) -> Result<()> {
        let fingerprint = device.fingerprint.clone();

        {
            let mut devices = self.devices.write().await;
            devices.insert(fingerprint, device);
        }

        let devices_to_save = {
            let devices = self.devices.read().await;
            devices.values().cloned().collect::<Vec<_>>()
        };

        self.save(&devices_to_save).await?;

        Ok(())
    }

    /// Returns a copy of the current device list
    pub async fn get_devices(&self) -> Vec<DiscoveredDevice> {
        let devices = self.devices.read().await;
        devices.values().cloned().collect()
    }
}

/// Manages the discovery service lifecycle and coordinates between network operations
/// and device state storage.
pub struct DiscoveryRuntime {
    /// Handle to the discovery service
    service: Arc<DiscoveryServiceImplementation>,
    /// Central device state management
    pub store: DiscoveryStore,
    /// Token for graceful shutdown management
    announcements_cancel_token: CancellationToken,
    /// Sender for discovered device events
    event_tx: broadcast::Sender<DiscoveredDevice>,
    /// Background task handle for event processing
    event_listener: Mutex<Option<JoinHandle<()>>>,
    /// Token for controlling event listener lifecycle
    listener_token: CancellationToken,
}

impl DiscoveryRuntime {
    /// Initializes a new DiscoveryRuntime with:
    /// - Configuration directory for persistent storage
    /// - Network event channel setup
    /// - Device state loading from storage
    pub async fn new(config_dir: Option<PathBuf>) -> Result<Self> {
        let (event_tx, _event_rx) = broadcast::channel(4);
        let service = DiscoveryServiceImplementation::new(event_tx.clone());
        let store = DiscoveryStore::new(config_dir);

        // Load persisted devices into memory
        store.load().await?;

        Ok(Self {
            service: Arc::new(service),
            store,
            announcements_cancel_token: CancellationToken::new(),
            event_tx,
            event_listener: Mutex::new(None),
            listener_token: CancellationToken::new(),
        })
    }

    pub fn new_without_store() -> Self {
        let (event_tx, _event_rx) = broadcast::channel(100);
        let service = DiscoveryServiceImplementation::new(event_tx.clone());
        let store = DiscoveryStore::new::<PathBuf>(None);

        Self {
            service: Arc::new(service),
            store,
            announcements_cancel_token: CancellationToken::new(),
            event_tx,
            event_listener: Mutex::new(None),
            listener_token: CancellationToken::new(),
        }
    }

    /// Starts the device discovery listener service.
    /// Sets up network listening and event processing loop for discovered devices.
    ///
    /// # Arguments
    /// * `self_fingerprint` - the fingerprint of the device itself
    ///
    /// # Returns
    /// * `Result<()>` - Success if listener started, error if already running
    pub async fn start_listening(&self, self_fingerprint: Option<String>) -> Result<()> {
        info!("Start listening for new devices");
        let cancel_token = self.listener_token.child_token();

        self.service
            .listen(self_fingerprint, Some(cancel_token.clone()))
            .await?;

        let mut event_listener = self.event_listener.lock().await;
        if event_listener.is_some() {
            error!("Listener already running");
            return Err(anyhow::anyhow!("Listener already running"));
        }

        let event_rx = self.event_tx.subscribe();
        let store_clone = self.store.clone();

        let handle = tokio::spawn(async move {
            info!("Event processing task started");
            let mut event_rx = event_rx;

            while let Ok(device) = event_rx.recv().await {
                info!("Received event in processor");
                if let Err(e) = store_clone.update_device(device).await {
                    error!("Failed to update device: {}", e);
                }
            }

            info!("Event processing task ended");
        });

        *event_listener = Some(handle);
        Ok(())
    }

    /// Completely stops the listening service by:
    /// - Cancelling the listener token
    /// - Aborting the event processing task
    pub async fn stop_listening(&self) {
        info!("Stop listening for new devices");
        self.listener_token.cancel();
        if let Some(handle) = self.event_listener.lock().await.take() {
            handle.abort();
        }
    }

    /// Starts the discovery service with specified network parameters.
    /// Spawns a background task that periodically announces the local device presence.
    ///
    /// # Arguments
    /// * `device_info` - Local device information to advertise
    /// * `interval` - Broadcast interval for service announcements
    pub async fn start_announcements(
        &self,
        device_info: DeviceInfo,
        interval: Duration,
        duration_limit: Option<Duration>,
    ) -> Result<()> {
        let service = self.service.clone();
        let cancel_token = self.announcements_cancel_token.child_token();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            let sleep_future = duration_limit.map(tokio::time::sleep);
            tokio::pin!(sleep_future);

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(e) = service.announce(device_info.clone()).await {
                            error!("Announcement failed: {}", e);
                        }
                    }
                    _ = cancel_token.cancelled() => break,
                    _ = async {
                        if let Some(future) = sleep_future.as_mut().as_pin_mut() {
                            future.await
                        }
                    } => {
                        info!("Announcement duration limit reached");
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    /// Stops the device announcement service by cancelling the announcement task.
    ///
    /// This method will:
    /// 1. Cancel all ongoing announcement operations using the cancel token
    /// 2. Allow any in-progress announcement to complete gracefully
    ///
    /// # Note
    /// This does not affect the discovery listener - use `stop_listening()` for that.
    ///
    /// # Returns
    /// * `()` - The method always succeeds as it just triggers cancellation
    pub fn stop_announcements(&self) {
        info!("Stopping device announcements");
        self.announcements_cancel_token.cancel();
    }

    /// Gracefully shuts down the discovery service:
    /// 1. Cancels all ongoing operations
    /// 2. Stops network listeners
    /// 3. Persists final device state
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down discovery runtime");
        self.announcements_cancel_token.cancel();
        self.stop_listening().await;
        self.listener_token.cancel();

        let devices = self.store.get_devices().await;

        self.store.save(&devices).await?;

        if let Some(handle) = self.event_listener.lock().await.take() {
            handle.await.ok();
        }

        self.service.shutdown().await;

        Ok(())
    }
}
