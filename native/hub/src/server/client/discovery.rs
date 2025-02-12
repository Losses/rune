use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, Result};
use chrono::Utc;
use log::{error, info};
use tokio::{
    sync::{mpsc, Mutex},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use discovery::{
    udp_multicast::{DiscoveredDevice, DiscoveryService},
    utils::DeviceInfo,
};

/// Manages persistent storage and in-memory cache of discovered devices.
/// Automatically handles data expiration and file I/O operations.
#[derive(Clone)]
pub struct DiscoveryStore {
    /// Path to the persistent storage file
    path: PathBuf,
    /// In-memory device list with thread-safe access
    devices: Arc<Mutex<Vec<DiscoveredDevice>>>,
}

impl DiscoveryStore {
    /// Creates a new DiscoveryStore instance with the specified base directory.
    /// The actual storage file will be created at `{base_dir}/.discovered`.
    pub fn new<P: AsRef<Path>>(base_path: P) -> Self {
        Self {
            path: base_path.as_ref().join(".discovered"),
            devices: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Loads devices from persistent storage into memory.
    /// Creates an empty list if the storage file doesn't exist.
    pub async fn load(&self) -> Result<Vec<DiscoveredDevice>> {
        use toml::Value;

        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let content = tokio::fs::read_to_string(&self.path).await?;
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

        *self.devices.lock().await = devices.clone();
        Ok(devices)
    }

    /// Persists the current device list to storage, automatically removing
    /// devices that haven't been seen in the last 30 seconds.
    pub async fn save(&self, devices: &[DiscoveredDevice]) -> Result<()> {
        use toml::Value;

        let filtered: Vec<_> = devices
            .iter()
            .filter(|d| {
                let elapsed = Utc::now().signed_duration_since(d.last_seen);
                elapsed.to_std().unwrap_or(Duration::MAX) < Duration::from_secs(30)
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

        tokio::fs::write(&self.path, content).await?;
        Ok(())
    }

    /// Removes expired devices from both memory and persistent storage
    pub async fn prune_expired(&self) -> Result<()> {
        info!("Pruning expired discovery entry");

        let devices_to_save = {
            let mut devices = self.devices.lock().await;
            let now = Utc::now();
            devices.retain(|d| {
                let elapsed = now.signed_duration_since(d.last_seen);
                elapsed.to_std().unwrap_or(Duration::MAX) < Duration::from_secs(30)
            });
            devices.clone()
        };

        self.save(&devices_to_save).await?;
        info!("Final data prepared");
        Ok(())
    }

    /// Updates or inserts a device into the store and persists changes
    pub async fn update_device(&self, device: DiscoveredDevice) -> Result<()> {
        let devices_to_save = {
            let mut devices = self.devices.lock().await;
            if let Some(existing) = devices
                .iter_mut()
                .find(|d| d.fingerprint == device.fingerprint)
            {
                *existing = device;
            } else {
                devices.push(device);
            }
            devices.clone()
        };

        self.save(&devices_to_save).await
    }

    /// Returns a copy of the current device list
    pub async fn get_devices(&self) -> Vec<DiscoveredDevice> {
        self.devices.lock().await.clone()
    }
}

/// Manages the discovery service lifecycle and coordinates between network operations
/// and device state storage.
pub struct DiscoveryRuntime {
    /// Handle to the discovery service
    service: Arc<DiscoveryService>,
    /// Central device state management
    pub store: DiscoveryStore,
    /// Token for graceful shutdown management
    cancel_token: CancellationToken,
    /// Receiver for discovered device events
    event_rx: Option<mpsc::Receiver<DiscoveredDevice>>,
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
    pub async fn new(config_dir: &Path) -> Result<Self> {
        let (event_tx, event_rx) = mpsc::channel(100);
        let service = DiscoveryService::new(event_tx);
        let store = DiscoveryStore::new(config_dir);

        // Load persisted devices into memory
        store.load().await?;

        Ok(Self {
            service: Arc::new(service),
            store,
            cancel_token: CancellationToken::new(),
            event_rx: Some(event_rx),
            event_listener: Mutex::new(None),
            listener_token: CancellationToken::new(),
        })
    }

    /// Starts the device discovery listener service.
    /// Sets up network listening and event processing loop for discovered devices.
    ///
    /// # Arguments
    /// * `device_info` - Information about the local device to be used in discovery
    ///
    /// # Returns
    /// * `Result<()>` - Success if listener started, error if already running
    pub async fn start_listening(&mut self, device_info: DeviceInfo) -> Result<()> {
        let cancel_token = self.listener_token.child_token();

        // Start network listening
        self.service
            .listen(device_info.clone(), Some(cancel_token.clone()))
            .await?;

        // Start event processing loop
        let mut event_listener = self.event_listener.lock().await;
        if event_listener.is_some() {
            return Err(anyhow::anyhow!("Listener already running"));
        }

        let event_rx = self
            .event_rx
            .take()
            .ok_or_else(|| anyhow!("Event channel consumed"))?;
        let store_clone = self.store.clone();

        *event_listener = Some(tokio::spawn(async move {
            let mut event_rx = event_rx;
            while let Some(device) = event_rx.recv().await {
                if let Err(e) = store_clone.update_device(device).await {
                    error!("Failed to update device: {}", e);
                }
            }
        }));

        Ok(())
    }

    /// Completely stops the listening service by:
    /// - Cancelling the listener token
    /// - Aborting the event processing task
    pub async fn stop_listening(&self) {
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
    ) -> Result<()> {
        let service = self.service.clone();
        let cancel_token = self.cancel_token.child_token();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(e) = service.announce(device_info.clone()).await {
                            error!("Announcement failed: {}", e);
                        }
                    }
                    _ = cancel_token.cancelled() => break,
                }
            }
        });

        Ok(())
    }

    /// Gracefully shuts down the discovery service:
    /// 1. Cancels all ongoing operations
    /// 2. Stops network listeners
    /// 3. Persists final device state
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down discovery runtime");

        // Cancel all operations
        self.cancel_token.cancel();
        self.listener_token.cancel();

        // Stop network services
        self.service.shutdown().await;

        // Persist storage
        let devices = self.store.devices.lock().await.clone();
        self.store.save(&devices).await
    }
}
