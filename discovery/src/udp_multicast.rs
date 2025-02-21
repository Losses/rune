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
use log::{debug, error, info, warn};
use netdev::Interface;
use serde::{Deserialize, Serialize};
use serde_json::json;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::{
    net::UdpSocket,
    sync::{mpsc::Sender, Mutex},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::utils::{DeviceInfo, DeviceType};

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
    /// Broadcast channel for discovery events
    event_tx: Sender<DiscoveredDevice>,
    /// Active listener tasks
    listeners: Mutex<Vec<JoinHandle<()>>>,
    /// Policy for network operation retries
    retry_policy: RetryPolicy,
    /// Track IP addresses per device fingerprint
    device_ips: Arc<DashMap<String, Vec<IpAddr>>>,
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
    /// * `event_tx` - Broadcast channel sender for discovery events
    pub fn new(event_tx: Sender<DiscoveredDevice>) -> Self {
        Self {
            sockets_init: Mutex::new(None),
            event_tx,
            listeners: Mutex::new(Vec::new()),
            retry_policy: RetryPolicy::default(),
            device_ips: Arc::new(DashMap::new()),
        }
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
        let device_ips = self.device_ips.clone();

        let mut handles = Vec::with_capacity(sockets.len());
        for socket in sockets.iter() {
            let socket = Arc::clone(socket);
            let self_fingerprint = self_fingerprint.clone();
            let event_tx = self.event_tx.clone();
            let cancel_token = cancel_token.clone();
            let device_ips = device_ips.clone();

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
                                        &event_tx,
                                        &device_ips
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
    async fn handle_datagram(
        self_fingerprint: &Option<String>,
        data: &[u8],
        addr: SocketAddr,
        event_tx: &Sender<DiscoveredDevice>,
        device_ips: &Arc<DashMap<String, Vec<IpAddr>>>,
    ) -> Result<()> {
        let announcement: serde_json::Value =
            serde_json::from_slice(data).context("Failed to parse announcement")?;

        debug!("Received announcement from {}: {}", addr, announcement);

        // Extract and validate fingerprint
        let fingerprint = match announcement.get("fingerprint") {
            Some(v) => v.as_str().unwrap_or_default().to_string(),
            None => {
                warn!("Received announcement without fingerprint");
                return Ok(());
            }
        };

        // Filter self-announcements
        if Some(fingerprint.clone()) == *self_fingerprint {
            debug!("Ignoring self-announcement");
            return Ok(());
        }

        // Update IP tracking
        let source_ip = addr.ip();
        let current_ips = {
            let mut ips_entry = device_ips.entry(fingerprint.clone()).or_default();

            if ips_entry.is_empty() {
                info!("Found a new fingerprint {}", fingerprint);
            }
            let is_new_ip = !ips_entry.contains(&source_ip);
            if is_new_ip {
                info!(
                    "Received a new IP address {} for the fingerprint {}",
                    source_ip, fingerprint
                );
                ips_entry.push(source_ip);
            }

            ips_entry.clone()
        };

        // Parse device type
        let device_type_value = announcement
            .get("deviceType")
            .ok_or_else(|| anyhow!("Announcement missing deviceType"))?;
        let device_type: DeviceType = serde_json::from_value(device_type_value.clone())
            .context("Failed to parse deviceType")?;

        // Build discovery event
        let discovered = DiscoveredDevice {
            alias: announcement["alias"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string(),
            device_model: announcement["deviceModel"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string(),
            device_type,
            fingerprint,
            ips: current_ips,
            last_seen: Utc::now(),
        };

        // Broadcast discovery event
        event_tx.send(discovered).await?;

        Ok(())
    }

    /// Stops all active listeners and cleans up resources
    pub async fn shutdown(&self) {
        let mut listeners = self.listeners.lock().await;
        for handle in listeners.drain(..) {
            handle.abort(); // Forcefully terminate tasks
        }
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
