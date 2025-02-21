//! A multicast-based device discovery service implementation.
//!
//! This module provides functionality for discovering devices on a local network using
//! IPv4 multicast. Devices can announce their presence and listen for announcements from
//! other devices. The service handles network interface management, socket configuration,
//! retry logic, and event broadcasting.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use log::{debug, error, info};
use netdev::Interface;
use serde::{Deserialize, Serialize};
use serde_json::json;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::{net::UdpSocket, sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{
    persistent::PersistentDataManager,
    utils::{DeviceInfo, DeviceType},
};

/// Represents a discovered device in the network
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    /// Human-readable name of the device
    pub alias: String,
    /// Manufacturer's device model identifier
    pub device_model: String,
    /// Type classification of the device
    pub device_type: DeviceType,
    /// Unique identifier for the device (cryptographic hash)
    pub fingerprint: String,
    /// Last time the device was seen (UTC timestamp)
    #[serde(with = "chrono::serde::ts_seconds")]
    pub last_seen: DateTime<Utc>,
    /// List of IP addresses where the device was observed
    pub ips: Vec<IpAddr>,
}

/// Multicast configuration constants
const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);
const MULTICAST_PORT: u16 = 57863;

/// Main discovery service handling network communication and device tracking
pub struct DiscoveryServiceImplementation {
    /// Lazily initialized multicast sockets with retry status
    sockets_init: Mutex<Option<Result<Vec<Arc<UdpSocket>>>>>,
    /// Persistence layer for device discovery
    store: Option<Arc<PersistentDataManager<Vec<DiscoveredDevice>>>>,
    /// Active listener tasks
    listeners: Mutex<Vec<JoinHandle<()>>>,
    /// Policy for network operation retries
    retry_policy: RetryPolicy,
    /// Track the detailed information of all devices
    device_states: Arc<DashMap<String, DiscoveredDevice>>,
}

/// Configuration for network operation retry behavior
#[derive(Clone)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts
    max_retries: usize,
    /// Initial delay between retries (exponential backoff)
    initial_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_secs(1),
        }
    }
}

/// Filters network interfaces to those suitable for multicast
fn get_multicast_interfaces() -> Vec<Interface> {
    netdev::get_interfaces()
        .into_iter()
        .filter(|iface| iface.is_up() && iface.is_multicast())
        .collect()
}

impl DiscoveryServiceImplementation {
    /// Creates a new DiscoveryService with the specified event channel
    ///
    /// # Arguments
    /// * `store` - Optional persistence layer for device discovery
    pub async fn new(store: Option<Arc<PersistentDataManager<Vec<DiscoveredDevice>>>>) -> Self {
        let result = Self {
            sockets_init: Mutex::new(None),
            store,
            listeners: Mutex::new(Vec::new()),
            retry_policy: RetryPolicy::default(),
            device_states: Arc::new(DashMap::new()),
        };

        match result.initialize().await {
            Ok(_) => info!("Discovery service initialized"),
            Err(e) => error!("Failed to initialize discovery service: {}", e),
        };

        result
    }

    pub fn new_without_store() -> Self {
        Self {
            sockets_init: Mutex::new(None),
            store: None,
            listeners: Mutex::new(Vec::new()),
            retry_policy: RetryPolicy::default(),
            device_states: Arc::new(DashMap::new()),
        }
    }

    async fn initialize(&self) -> Result<()> {
        if let Some(store) = &self.store {
            let devices = store.read().await.clone().into_iter();
            for device in devices {
                self.device_states
                    .insert(device.fingerprint.clone(), device);
            }
        }

        Ok(())
    }

    /// Configures the retry policy for network operations
    ///
    /// # Arguments
    /// * `policy` - Retry policy configuration
    ///
    /// # Example
    /// ```
    /// service.with_retry_policy(RetryPolicy {
    ///     max_retries: 5,
    ///     initial_backoff: Duration::from_secs(2)
    /// });
    /// ```
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Initializes multicast sockets with retry logic
    async fn get_sockets_with_retry(&self) -> Result<Arc<Vec<Arc<UdpSocket>>>> {
        let mut lock = self.sockets_init.lock().await;

        if let Some(result) = &*lock {
            return result
                .as_ref()
                .map(|sockets| Arc::new(sockets.clone()))
                .map_err(|e| anyhow!(e.to_string()));
        }

        let mut retries = self.retry_policy.max_retries;
        let mut backoff = self.retry_policy.initial_backoff;
        let mut last_error = None;

        // Retry loop with exponential backoff
        while retries > 0 {
            match Self::try_init_sockets().await {
                Ok(sockets) => {
                    let sockets = Arc::new(sockets);
                    *lock = Some(Ok(sockets.to_vec()));
                    return Ok(sockets);
                }
                Err(e) => {
                    last_error = Some(e);
                    retries -= 1;
                    tokio::time::sleep(backoff).await;
                    backoff *= 2;
                }
            }
        }

        let error = last_error.unwrap_or_else(|| anyhow!("Socket initialization failed"));
        *lock = Some(Err(anyhow!("{}", error)));

        Err(error)
    }

    /// Attempts to initialize multicast sockets on all suitable interfaces
    async fn try_init_sockets() -> Result<Vec<Arc<UdpSocket>>> {
        let mut sockets = Vec::new();

        // Create socket for each multicast-capable interface
        for iface in get_multicast_interfaces() {
            for ipv4_net in iface.ipv4 {
                let interface_ip = ipv4_net.addr();

                // Use blocking task for socket configuration
                let socket = tokio::task::spawn_blocking(move || {
                    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
                    let bind_addr =
                        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), MULTICAST_PORT);

                    // Configure socket options
                    socket.set_reuse_address(true)?;
                    #[cfg(not(target_os = "windows"))]
                    socket.set_reuse_port(true)?;
                    socket.bind(&bind_addr.into())?;

                    // Set multicast parameters
                    socket.set_multicast_ttl_v4(255)?; // Maximum TTL for local network
                    socket.set_multicast_loop_v4(true)?; // Enable local loopback
                    socket.set_multicast_if_v4(&interface_ip)?; // Bind to specific interface
                    socket.join_multicast_v4(&MULTICAST_GROUP, &interface_ip)?;

                    Ok::<_, anyhow::Error>(socket)
                })
                .await??; // Handle both join and socket errors

                // Convert to tokio socket
                let std_socket = socket.into();
                let tokio_socket =
                    UdpSocket::from_std(std_socket).context("Failed to convert to tokio socket")?;

                sockets.push(Arc::new(tokio_socket));
            }
        }

        if sockets.is_empty() {
            return Err(anyhow!("No valid multicast interfaces found"));
        }

        Ok(sockets)
    }

    /// Broadcasts device announcement to the network
    ///
    /// # Arguments
    /// * `device_info` - Device information to announce
    ///
    /// # Errors
    /// Returns error if socket initialization fails or message serialization fails
    pub async fn announce(&self, device_info: DeviceInfo) -> Result<()> {
        let sockets = self.get_sockets_with_retry().await?;
        let announcement = json!({
            "alias": device_info.alias,
            "version": device_info.version,
            "deviceModel": device_info.device_model,
            "deviceType": device_info.device_type,
            "fingerprint": device_info.fingerprint,
            "api_port": device_info.api_port,
            "protocol": device_info.protocol,
            "announce": true  // Flag to distinguish announcements
        });

        let msg = serde_json::to_vec(&announcement)?;

        // Send announcement through all sockets
        for socket in sockets.iter() {
            let target = format!("{}:{}", MULTICAST_GROUP, MULTICAST_PORT);
            match socket.send_to(&msg, &target).await {
                Ok(bytes_sent) => debug!("[{}] Sent {} bytes", socket.local_addr()?, bytes_sent),
                Err(e) => error!("Send error on {}: {}", socket.local_addr()?, e),
            }
        }

        Ok(())
    }

    /// Starts listening for device announcements
    ///
    /// # Arguments
    /// * `self_fingerprint` - Optional fingerprint to filter self-announcements
    /// * `cancel_token` - Optional cancellation token for graceful shutdown
    ///
    /// # Returns
    /// Returns immediately after starting listeners. Use shutdown() to stop.
    pub async fn listen(
        &self,
        self_fingerprint: Option<String>,
        cancel_token: Option<CancellationToken>,
    ) -> Result<()> {
        let sockets = self.get_sockets_with_retry().await?;
        info!("Starting to listen on {} interfaces", sockets.len());

        let cancel_token = cancel_token.unwrap_or_default();
        let devices = self.device_states.clone();
        let store = self.store.clone();

        let mut handles = Vec::with_capacity(sockets.len());
        for socket in sockets.iter() {
            let socket = Arc::clone(socket);
            let self_fingerprint = self_fingerprint.clone();
            let store = store.clone();
            let cancel_token = cancel_token.clone();
            let devices = devices.clone();

            // Spawn listener task per socket
            let handle = tokio::spawn(async move {
                let mut buf = vec![0u8; 65535]; // Maximum UDP packet size
                loop {
                    tokio::select! {
                        _ = cancel_token.cancelled() => {
                            debug!("Listener exiting on {}", socket.local_addr().unwrap());
                            break;
                        }
                        result = socket.recv_from(&mut buf) => {
                            match result {
                                Ok((len, addr)) => {
                                    if let Err(e) = Self::handle_datagram(
                                        &self_fingerprint,
                                        &buf[..len],
                                        addr,
                                        &store,
                                        &devices
                                    ).await {
                                        if e.to_string().contains("channel closed") {
                                            debug!("Channel closed, stopping listener on {}",
                                                socket.local_addr().unwrap());
                                            break;
                                        }

                                        error!("Error handling datagram: {}", e);
                                    }
                                }
                                Err(e) => error!("Receive error: {}", e),
                            }
                        }
                    }
                }
            });
            handles.push(handle);
        }

        self.listeners.lock().await.extend(handles);
        Ok(())
    }

    /// Processes incoming announcement datagrams
    pub async fn handle_datagram(
        self_fingerprint: &Option<String>,
        data: &[u8],
        addr: SocketAddr,
        store: &Option<Arc<PersistentDataManager<Vec<DiscoveredDevice>>>>,
        devices: &Arc<DashMap<String, DiscoveredDevice>>,
    ) -> Result<()> {
        let announcement: serde_json::Value = serde_json::from_slice(data)?;

        let fingerprint = match announcement.get("fingerprint") {
            Some(v) => v.as_str().unwrap_or_default().to_string(),
            None => return Ok(()),
        };

        if Some(fingerprint.clone()) == *self_fingerprint {
            return Ok(());
        }

        let device = DiscoveredDevice {
            fingerprint: fingerprint.clone(),
            alias: announcement["alias"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string(),
            device_model: announcement["deviceModel"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string(),
            device_type: serde_json::from_value(announcement["deviceType"].clone())?,
            ips: vec![addr.ip()],
            last_seen: Utc::now(),
        };

        if let Some(mut existing) = devices.get_mut(&fingerprint) {
            if !existing.ips.contains(&addr.ip()) {
                existing.ips.push(addr.ip());
            }
            existing.last_seen = Utc::now();
        } else {
            devices.insert(fingerprint, device);
        }

        if let Some(store) = store {
            store
                .update(|_| async move {
                    let d = devices
                        .iter()
                        .map(|entry| entry.value().clone())
                        .collect::<Vec<_>>();

                    Ok::<_, anyhow::Error>((d, ()))
                })
                .await?;
        }

        Ok(())
    }

    pub async fn get_all_devices(&self) -> Vec<DiscoveredDevice> {
        self.device_states
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Stops all active listeners and cleans up resources
    pub async fn shutdown(&self) -> Result<()> {
        let mut listeners = self.listeners.lock().await;
        for handle in listeners.drain(..) {
            handle.abort(); // Forcefully terminate tasks
        }

        // Finally ensure store is saved before shutdown
        self.save().await?;

        Ok(())
    }

    pub async fn save(&self) -> Result<()> {
        if let Some(store) = &self.store {
            return store
                .update(|_| async move {
                    let d = self
                        .device_states
                        .iter()
                        .map(|entry| entry.value().clone())
                        .collect::<Vec<_>>();

                    Ok::<_, anyhow::Error>((d, ()))
                })
                .await;
        }

        Ok(())
    }

    /// Checks if the service has successfully initialized sockets
    pub async fn is_operational(&self) -> bool {
        self.sockets_init
            .lock()
            .await
            .as_ref()
            .map(|r| r.is_ok())
            .unwrap_or(false)
    }
}
