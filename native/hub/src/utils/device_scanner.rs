use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use log::error;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use discovery::{
    discovery_runtime::DiscoveryRuntime, udp_multicast::DiscoveredDevice, utils::DeviceInfo,
};

/// DeviceScanner is responsible for discovering and tracking network devices through UDP multicast.
/// It provides functionality to both broadcast device presence and listen for other devices on the network.
///
/// The scanner maintains a local cache of discovered devices and forwards device discovery events
/// to registered broadcasters for further processing or UI updates.
pub struct DeviceScanner {
    /// Runtime instance managing the device discovery process
    pub discovery_runtime: Arc<DiscoveryRuntime>,
    /// Task handle for the device broadcasting operation
    pub broadcast_task: Mutex<Option<JoinHandle<()>>>,
    /// Task handle for the device listening operation
    pub listen_task: Mutex<Option<JoinHandle<()>>>,
    /// Thread-safe cache of discovered devices, keyed by device fingerprint
    pub devices: Arc<RwLock<HashMap<String, DiscoveredDevice>>>,
    /// Flag indicating whether the scanner is currently broadcasting
    is_broadcasting: Arc<AtomicBool>,
}

impl Default for DeviceScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceScanner {
    /// Creates a new DeviceScanner instance with the specified broadcaster.
    ///
    /// # Returns
    /// A new DeviceScanner instance with initialized components and started event forwarding
    pub fn new() -> Self {
        let discovery_runtime = Arc::new(DiscoveryRuntime::new_without_store());

        Self {
            discovery_runtime,
            broadcast_task: Mutex::new(None),
            listen_task: Mutex::new(None),
            devices: Arc::new(RwLock::new(HashMap::new())),
            is_broadcasting: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Starts broadcasting this device's presence on the network.
    ///
    /// # Arguments
    /// * `device_info` - Information about the current device to broadcast
    /// * `duration_seconds` - How long to broadcast for, in seconds
    ///
    /// # Note
    /// This method will first stop any existing broadcast before starting a new one.
    pub async fn start_broadcast(&self, device_info: &DeviceInfo, duration_seconds: u32) {
        // Terminate existing task first
        self.stop_broadcast().await;

        // Update state
        self.is_broadcasting.store(true, Ordering::SeqCst);

        let discovery_service = self.discovery_runtime.clone();
        let is_broadcasting = self.is_broadcasting.clone();
        let device_info = device_info.clone();

        let task = tokio::spawn(async move {
            let current_device_info = device_info.clone();
            if let Err(e) = discovery_service
                .start_announcements(
                    current_device_info,
                    Duration::from_secs(3), // Broadcast interval
                    Some(Duration::from_secs(duration_seconds.into())), // Total duration
                )
                .await
            {
                error!("Broadcast error: {}", e);
            }

            // Update state when task completes
            is_broadcasting.store(false, Ordering::SeqCst);
        });

        *self.broadcast_task.lock().await = Some(task);
    }

    /// Stops the current device broadcast if one is active.
    /// This includes:
    /// 1. Checking if broadcasting is active
    /// 2. Aborting the broadcast task if it exists
    /// 3. Updating the broadcasting state
    pub async fn stop_broadcast(&self) {
        // Check state to avoid unnecessary operations
        if self.is_broadcasting.load(Ordering::SeqCst) {
            if let Some(task) = self.broadcast_task.lock().await.take() {
                // Graceful shutdown
                if !task.is_finished() {
                    task.abort();
                }
            }
            self.is_broadcasting.store(false, Ordering::SeqCst);
        }
    }

    /// Starts listening for other devices on the network.
    ///
    /// # Arguments
    /// * `self_fingerprint` - Optional fingerprint of this device to filter out self-broadcasts
    pub async fn start_listening(&self, self_fingerprint: Option<String>) {
        let discovery_runtime = self.discovery_runtime.clone();

        let task = tokio::spawn(async move {
            if let Err(e) = discovery_runtime.start_listening(self_fingerprint).await {
                error!("Listening error: {}", e);
            }
        });

        *self.listen_task.lock().await = Some(task);
    }

    /// Stops the device listening process if one is active.
    pub async fn stop_listening(&self) {
        if let Some(task) = self.listen_task.lock().await.take() {
            task.abort();
        }
    }

    /// Returns whether the scanner is currently broadcasting device information.
    ///
    /// # Returns
    /// `true` if broadcasting is active, `false` otherwise
    pub fn is_broadcasting(&self) -> bool {
        self.is_broadcasting.load(Ordering::SeqCst)
    }

    /// Get a list of discovered devices, sorted by last seen time.
    ///
    /// # Returns
    /// A list of discovered devices, sorted by last seen time
    pub async fn get_devices(&self) -> Vec<DiscoveredDevice> {
        let mut devices: Vec<DiscoveredDevice> =
            self.devices.read().await.clone().into_values().collect();
        devices.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
        devices
    }
}
