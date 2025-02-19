use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use log::{error, info};
use tokio::sync::{broadcast::Receiver, Mutex, RwLock};
use tokio::task::JoinHandle;

use discovery::{
    discovery_runtime::DiscoveryRuntime, udp_multicast::DiscoveredDevice, utils::DeviceInfo,
};

use super::{Broadcaster, DiscoveredDeviceMessage};

pub struct DeviceScanner {
    pub discovery_runtime: Arc<DiscoveryRuntime>,
    pub broadcast_task: Mutex<Option<JoinHandle<()>>>,
    pub listen_task: Mutex<Option<JoinHandle<()>>>,
    pub devices: Arc<RwLock<HashMap<String, DiscoveredDevice>>>,
    broadcaster: Arc<dyn Broadcaster>,
    is_broadcasting: Arc<AtomicBool>,
}

impl DeviceScanner {
    pub fn new(broadcaster: Arc<dyn Broadcaster>) -> Self {
        let discovery_runtime = Arc::new(DiscoveryRuntime::new_without_store());
        let event_rx = discovery_runtime.subscribe();

        let scanner = Self {
            discovery_runtime,
            broadcast_task: Mutex::new(None),
            listen_task: Mutex::new(None),
            devices: Arc::new(RwLock::new(HashMap::new())),
            broadcaster: Arc::clone(&broadcaster),
            is_broadcasting: Arc::new(AtomicBool::new(false)),
        };

        scanner.start_event_forwarder(event_rx);
        scanner
    }

    fn start_event_forwarder(&self, mut event_rx: Receiver<DiscoveredDevice>) {
        let devices = self.devices.clone();
        let broadcaster = self.broadcaster.clone();

        tokio::spawn(async move {
            while let Ok(device) = event_rx.recv().await {
                // Update local cache
                let mut devices_map = devices.write().await;
                devices_map.insert(device.fingerprint.clone(), device.clone());

                info!("Found device: {}", device.fingerprint);

                // Convert to proto message
                let message = DiscoveredDeviceMessage {
                    alias: device.alias,
                    device_model: device.device_model,
                    device_type: device.device_type.to_string(),
                    fingerprint: device.fingerprint,
                    last_seen_unix_epoch: device.last_seen.timestamp(),
                    ips: device.ips.iter().map(|x| x.to_string()).collect(),
                };

                broadcaster.broadcast(&message);
            }
        });
    }

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
                    Duration::from_secs(3),
                    Some(Duration::from_secs(duration_seconds.into())),
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

    pub async fn start_listening(&self, self_fingerprint: Option<String>) {
        let discovery_runtime = self.discovery_runtime.clone();

        let task = tokio::spawn(async move {
            if let Err(e) = discovery_runtime.start_listening(self_fingerprint).await {
                error!("Listening error: {}", e);
            }
        });

        *self.listen_task.lock().await = Some(task);
    }

    pub async fn stop_listening(&self) {
        if let Some(task) = self.listen_task.lock().await.take() {
            task.abort();
        }
    }

    // Helper method for state checking
    pub fn is_broadcasting(&self) -> bool {
        self.is_broadcasting.load(Ordering::SeqCst)
    }
}
