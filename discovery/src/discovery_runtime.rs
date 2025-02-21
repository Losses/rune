use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use log::{error, info};
use tokio_util::sync::CancellationToken;

use crate::{
    persistent::PersistentDataManager,
    udp_multicast::{DiscoveredDevice, DiscoveryServiceImplementation},
    utils::DeviceInfo,
};

/// Manages the discovery service lifecycle and coordinates between network operations
/// and device state storage.
pub struct DiscoveryRuntime {
    /// Handle to the discovery service
    service: Arc<DiscoveryServiceImplementation>,
    /// Token for graceful shutdown management
    announcements_cancel_token: Option<CancellationToken>,
    /// Token for controlling event listener lifecycle
    listening_cancel_token: Option<CancellationToken>,
}

impl DiscoveryRuntime {
    /// Initializes a new DiscoveryRuntime with:
    /// - Configuration directory for persistent storage
    /// - Network event channel setup
    /// - Device state loading from storage
    pub async fn new(config_dir: Option<PathBuf>) -> Result<Self> {
        let store = match config_dir {
            Some(config_dir) => Some(Arc::new(PersistentDataManager::new(config_dir)?)),
            None => None,
        };
        let service = Arc::new(DiscoveryServiceImplementation::new(store).await);

        Ok(Self {
            service,
            announcements_cancel_token: None,
            listening_cancel_token: None,
        })
    }

    pub fn new_without_store() -> Self {
        let service = Arc::new(DiscoveryServiceImplementation::new_without_store());

        Self {
            service,
            announcements_cancel_token: None,
            listening_cancel_token: None,
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
    pub async fn start_listening(&mut self, self_fingerprint: Option<String>) -> Result<()> {
        match self.listening_cancel_token {
            Some(_) => {
                error!("Listener already running");
                Err(anyhow::anyhow!("Listener already running"))
            }
            None => {
                info!("Start listening for new devices");
                let cancel_token = CancellationToken::new();

                self.service
                    .listen(self_fingerprint, Some(cancel_token.clone()))
                    .await?;

                self.listening_cancel_token = Some(cancel_token);

                Ok(())
            }
        }
    }

    /// Completely stops the listening service by:
    /// - Cancelling the listener token
    /// - Aborting the event processing task
    pub fn stop_listening(&self) {
        match &self.listening_cancel_token {
            Some(token) => {
                info!("Stop listening for new devices");
                token.cancel();
            }
            None => {
                info!("Listener not running");
            }
        }
    }

    /// Starts the discovery service with specified network parameters.
    /// Spawns a background task that periodically announces the local device presence.
    ///
    /// # Arguments
    /// * `device_info` - Local device information to advertise
    /// * `interval` - Broadcast interval for service announcements
    pub async fn start_announcements(
        &mut self,
        device_info: DeviceInfo,
        interval: Duration,
        duration_limit: Option<Duration>,
    ) -> Result<()> {
        match &self.announcements_cancel_token {
            Some(_) => {
                error!("Announcements already running");
                return Err(anyhow::anyhow!("Announcements already running"));
            }
            None => {
                info!("Starting device announcements");
                let cancel_token = CancellationToken::new();
                let service = self.service.clone();
                let cancel_token_clone = cancel_token.clone();

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
                            _ = cancel_token_clone.cancelled() => break,
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

                self.announcements_cancel_token = Some(cancel_token);
            }
        }

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
        if let Some(token) = &self.announcements_cancel_token {
            token.cancel();
        }
    }

    /// Gracefully shuts down the discovery service:
    /// 1. Cancels all ongoing operations
    /// 2. Stops network listeners
    /// 3. Persists final device state
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down discovery runtime");
        self.stop_announcements();
        self.stop_listening();

        self.service.shutdown().await?;

        Ok(())
    }

    pub async fn get_all_devices(&self) -> Vec<DiscoveredDevice> {
        self.service.get_all_devices().await
    }

    pub async fn save(&self) -> Result<()> {
        self.service.save().await
    }
}
