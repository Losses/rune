//! A multicast-based device discovery service implementation.
//!
//! This module provides functionality for discovering devices on a local network using
//! IPv4 multicast. Devices can announce their presence and listen for announcements from
//! other devices. The service handles network interface management, socket configuration,
//! retry logic, and event broadcasting for device discovery and presence tracking.
//!
//! ## Features
//!
//! - **Multicast Discovery:** Utilizes IPv4 multicast for efficient device discovery within a local network.
//! - **Device Announcement:** Allows devices to broadcast their presence and information on the network.
//! - **Device Listening:** Enables services to listen for announcements from other devices on the network.
//! - **Interface Management:** Automatically selects and manages suitable network interfaces for multicast communication.
//! - **Socket Configuration:** Configures UDP sockets with necessary options for multicast, including reuse and loopback.
//! - **Retry Mechanism:** Implements a configurable retry policy with exponential backoff for robust socket initialization.
//! - **Device Tracking:** Maintains a registry of discovered devices, updating their status and last seen time.
//! - **Persistence (Optional):** Supports optional persistence of discovered device information across service restarts.
//! - **Graceful Shutdown:** Provides mechanisms for graceful shutdown of announcement and listening services.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
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

/// Represents a discovered device on the network.
///
/// This struct encapsulates information about a device discovered through multicast announcements,
/// including its alias, model, type, unique fingerprint, last seen timestamp, and IP addresses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    /// Human-readable name of the device, often user-configured.
    pub alias: String,
    /// Manufacturer's device model identifier, indicating the specific model.
    pub device_model: String,
    /// Type classification of the device, categorizing its functionality (e.g., Light, Sensor).
    pub device_type: DeviceType,
    /// Unique identifier for the device, typically a cryptographic hash for device identification.
    pub fingerprint: String,
    /// Last time the device was detected and its announcement was received (UTC timestamp).
    #[serde(with = "chrono::serde::ts_seconds")]
    pub last_seen: DateTime<Utc>,
    /// List of IP addresses from which the device's announcements have been observed.
    pub ips: Vec<IpAddr>,
}

/// Multicast group address for device discovery.
///
/// Devices send announcements to this IPv4 multicast group address.
const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);
/// Port number used for multicast communication in device discovery.
const MULTICAST_PORT: u16 = 57863;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeviceList {
    devices: Vec<DiscoveredDevice>,
}

/// Manages the device discovery service, handling network communication and device tracking.
///
/// The `DiscoveryService` is responsible for initializing multicast sockets, sending device announcements,
/// listening for announcements from other devices, and maintaining a registry of discovered devices.
pub struct DiscoveryService {
    /// Lazily initialized multicast sockets, wrapped in a Mutex for thread-safe access and to handle retry status.
    sockets_init: Mutex<Option<Result<Vec<Arc<UdpSocket>>>>>,
    /// Optional persistence layer for storing and retrieving discovered device information.
    store: Option<Arc<PersistentDataManager<DeviceList>>>,
    /// List of active listener tasks, managed to allow for graceful shutdown.
    listeners: Mutex<Vec<JoinHandle<()>>>,
    /// Policy defining retry behavior for network operations like socket initialization.
    retry_policy: RetryPolicy,
    /// A map storing detailed information about all discovered devices, indexed by fingerprint.
    device_states: Arc<DashMap<String, DiscoveredDevice>>,

    /// Cancellation token for managing the lifecycle of announcement tasks.
    announcements_cancel_token: Mutex<Option<CancellationToken>>,
    /// Cancellation token for controlling the lifecycle of device listening tasks.
    listening_cancel_token: Mutex<Option<CancellationToken>>,
}

/// Configuration for defining the retry behavior of network operations.
///
/// The `RetryPolicy` allows customization of the maximum number of retry attempts and the initial backoff duration
/// for operations like socket initialization, providing resilience to transient network issues.
#[derive(Clone)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts before giving up on an operation.
    max_retries: usize,
    /// Initial delay between retry attempts, used as the base for exponential backoff.
    initial_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            // Default maximum retry attempts is set to 3.
            max_retries: 3,
            // Default initial backoff duration is 1 second.
            initial_backoff: Duration::from_secs(1),
        }
    }
}

/// Retrieves a list of network interfaces that are suitable for multicast communication.
///
/// This function filters all available network interfaces and returns only those that are up and multicast-enabled.
/// These interfaces are considered valid candidates for setting up multicast sockets.
fn get_multicast_interfaces() -> Vec<Interface> {
    netdev::get_interfaces()
        .into_iter()
        .filter(|iface| iface.is_up() && iface.is_multicast())
        .collect()
}

impl DiscoveryService {
    /// Creates a new `DiscoveryService` instance, with persistence enabled.
    ///
    /// This constructor initializes the discovery service, setting up internal state and optionally loading
    /// previously discovered devices from persistent storage.
    ///
    /// # Arguments
    /// * `path` -  Path to the directory where device discovery data will be persisted.
    ///   If persistence is not required, use [`DiscoveryService::new_without_store`].
    ///
    /// # Returns
    /// `Result<Self>` - A `Result` containing the new `DiscoveryService` instance, or an error if initialization fails.
    pub async fn with_store<P: AsRef<Path>>(path: P) -> Result<Self> {
        let storage_path = path.as_ref().join(".discovered");

        let store = Some(Arc::new(PersistentDataManager::new(storage_path)?));
        let result = Self {
            sockets_init: Mutex::new(None),
            store,
            listeners: Mutex::new(Vec::new()),
            retry_policy: RetryPolicy::default(),
            device_states: Arc::new(DashMap::new()),

            announcements_cancel_token: Mutex::new(None),
            listening_cancel_token: Mutex::new(None),
        };

        match result.initialize().await {
            Ok(_) => info!("Discovery service initialized"),
            Err(e) => error!("Failed to initialize discovery service: {e}"),
        };

        Ok(result)
    }

    /// Creates a new `DiscoveryService` instance without persistent storage.
    ///
    /// This constructor creates a `DiscoveryService` that operates in memory, without saving or loading
    /// discovered device information to disk. This is suitable for ephemeral discovery scenarios.
    ///
    /// # Returns
    /// `Self` - A new `DiscoveryService` instance without persistence.
    pub fn without_store() -> Self {
        let result = Self {
            sockets_init: Mutex::new(None),
            store: None,
            listeners: Mutex::new(Vec::new()),
            retry_policy: RetryPolicy::default(),
            device_states: Arc::new(DashMap::new()),

            announcements_cancel_token: Mutex::new(None),
            listening_cancel_token: Mutex::new(None),
        };
        info!("Discovery service initialized without store");
        result
    }

    /// Initializes the discovery service by loading device states from persistent storage, if enabled.
    ///
    /// If a persistent store is configured, this method attempts to read previously discovered devices from storage
    /// and populate the internal device state map. This ensures that known devices are tracked across service restarts.
    async fn initialize(&self) -> Result<()> {
        if let Some(store) = &self.store {
            let devices = store.read().await.clone().devices.into_iter();
            for device in devices {
                self.device_states
                    .insert(device.fingerprint.clone(), device);
            }
        }

        Ok(())
    }

    /// Configures the retry policy for network operations performed by the discovery service.
    ///
    /// This method allows setting a custom retry policy, which dictates the number of retries and the backoff strategy
    /// for operations such as socket initialization. This can be useful to adjust the service's resilience to network issues.
    ///
    /// # Arguments
    /// * `policy` - The `RetryPolicy` configuration to apply to the discovery service.
    ///
    /// # Returns
    /// `Self` - Returns the `DiscoveryService` instance with the updated retry policy, allowing for method chaining.
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Retrieves multicast sockets, initializing them if necessary with retry logic.
    ///
    /// This method ensures that multicast sockets are initialized and available for communication. It uses a retry mechanism
    /// defined by the service's `retry_policy` to handle potential socket initialization failures. The sockets are initialized
    /// only once and then reused for subsequent calls, unless initialization fails persistently.
    ///
    /// # Returns
    /// `Result<Arc<Vec<Arc<UdpSocket>>>>` - A `Result` containing a vector of `Arc<UdpSocket>` representing the initialized multicast sockets,
    ///                                     or an error if socket initialization fails after all retry attempts.
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

    /// Attempts to initialize multicast sockets on all suitable network interfaces.
    ///
    /// This method iterates through available multicast-capable interfaces and creates a UDP socket for each.
    /// It configures each socket to join the multicast group and sets necessary socket options for multicast communication.
    ///
    /// # Returns
    /// `Result<Vec<Arc<UdpSocket>>>` - A `Result` containing a vector of `Arc<UdpSocket>` for successfully initialized sockets,
    ///                                or an error if no valid multicast interfaces are found or socket initialization fails.
    async fn try_init_sockets() -> Result<Vec<Arc<UdpSocket>>> {
        let mut sockets = Vec::new();

        // Create socket for each multicast-capable interface
        for iface in get_multicast_interfaces() {
            for ipv4_net in iface.ipv4 {
                let interface_ip = ipv4_net.addr();

                // Use blocking task for socket configuration as socket2 is blocking
                let socket = tokio::task::spawn_blocking(move || {
                    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
                    socket.set_nonblocking(true)?;

                    let bind_addr =
                        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), MULTICAST_PORT);

                    // Configure socket options for multicast
                    socket.set_reuse_address(true)?; // Allow reuse of the socket address
                    #[cfg(not(target_os = "windows"))]
                    socket.set_reuse_port(true)?; // Allow multiple processes to bind to the same port
                    socket.bind(&bind_addr.into())?; // Bind to the multicast port

                    // Configure multicast specific options
                    socket.set_multicast_ttl_v4(255)?; // Set TTL to maximum for local network scope
                    socket.set_multicast_loop_v4(true)?; // Enable multicast loopback for local announcements
                    socket.set_multicast_if_v4(&interface_ip)?; // Specify the interface for multicast
                    socket.join_multicast_v4(&MULTICAST_GROUP, &interface_ip)?; // Join the multicast group on the interface

                    Ok::<_, anyhow::Error>(socket)
                })
                .await??; // Handle both task join errors and socket errors

                // Convert the standard socket to a Tokio UdpSocket for async operations
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

    /// Broadcasts a device announcement message to the multicast group.
    ///
    /// This function serializes the provided device announcement data into JSON and sends it over all initialized
    /// multicast sockets to the configured multicast group and port.
    ///
    /// # Arguments
    /// * `sockets` - A slice of `Arc<UdpSocket>` representing the multicast sockets to use for broadcasting.
    /// * `announcement` - A `serde_json::Value` representing the announcement message to be sent.
    ///
    /// # Errors
    /// Returns an error if message serialization fails or if sending the message over any socket fails.
    async fn send_announcement(
        sockets: &[Arc<UdpSocket>],
        announcement: &serde_json::Value,
    ) -> Result<()> {
        let msg = serde_json::to_vec(&announcement)?;

        for socket in sockets.iter() {
            let target = format!("{MULTICAST_GROUP}:{MULTICAST_PORT}");
            match socket.send_to(&msg, &target).await {
                Ok(bytes_sent) => debug!("[{}] Sent {} bytes", socket.local_addr()?, bytes_sent),
                Err(e) => error!("Send error on {}: {}", socket.local_addr()?, e),
            }
        }

        Ok(())
    }

    /// Starts the device announcement service, periodically broadcasting device information.
    ///
    /// This method spawns a background task that sends out device announcements at a specified interval.
    /// It uses the provided `DeviceInfo` to construct the announcement messages and broadcasts them to the multicast group.
    ///
    /// # Arguments
    /// * `device_info` - Information about the local device to be announced.
    /// * `interval` - The interval at which device announcements should be broadcasted.
    /// * `duration_limit` - An optional duration limit for how long announcements should be sent. If `Some`, announcements will stop after this duration.
    ///
    /// # Errors
    /// Returns an error if socket initialization fails or if announcements are already running.
    pub async fn start_announcements(
        &self,
        device_info: DeviceInfo,
        interval: Duration,
        duration_limit: Option<Duration>,
    ) -> Result<()> {
        {
            let announcements_cancel_token_guard = self.announcements_cancel_token.lock().await;
            if announcements_cancel_token_guard.is_some() {
                error!("Announcements already running");
                return Err(anyhow::anyhow!("Announcements already running"));
            }
        }

        info!("Starting device announcements");
        info!("The device fingerprint is: {}", device_info.fingerprint);
        let cancel_token = CancellationToken::new();
        let cancel_token_clone = cancel_token.clone();

        {
            let mut announcements_cancel_token_guard = self.announcements_cancel_token.lock().await;
            *announcements_cancel_token_guard = Some(cancel_token);
        }

        // Get sockets before spawning the task to ensure sockets are available
        let sockets = self.get_sockets_with_retry().await?;

        // Clone necessary data for the announcement task
        let sockets = Arc::new(sockets);
        let device_info = Arc::new(device_info);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            let send_announcement_message = || async {
                let announcement = json!({
                    "alias": device_info.alias,
                    "version": device_info.version,
                    "deviceModel": device_info.device_model,
                    "deviceType": device_info.device_type,
                    "fingerprint": device_info.fingerprint,
                    "api_port": device_info.api_port,
                    "protocol": device_info.protocol,
                    "announce": true
                });
                if let Err(e) = Self::send_announcement(&sockets, &announcement).await {
                    error!("Announcement failed: {e}");
                }
            };

            if let Some(limit) = duration_limit {
                let sleep = tokio::time::sleep(limit);
                tokio::pin!(sleep);

                loop {
                    tokio::select! {
                        _ = interval.tick() => send_announcement_message().await,
                        _ = cancel_token_clone.cancelled() => break,
                        _ = &mut sleep => {
                            info!("Announcement duration limit reached");
                            break;
                        }
                    }
                }
            } else {
                loop {
                    tokio::select! {
                        _ = interval.tick() => send_announcement_message().await,
                        _ = cancel_token_clone.cancelled() => break,
                    }
                }
            }
        });

        Ok(())
    }

    /// Stops the device announcement service.
    ///
    /// This method cancels the ongoing device announcement task, causing it to stop broadcasting device information.
    /// It does not immediately terminate any announcements currently in progress but signals the task to stop after the current iteration.
    ///
    /// # Note
    /// This method only stops device announcements. To stop listening for announcements from other devices, use [`stop_listening`].
    pub async fn stop_announcements(&self) {
        let mut announcements_cancel_token_guard = self.announcements_cancel_token.lock().await;
        if let Some(token) = &*announcements_cancel_token_guard {
            info!("Stop announcementing this device");
            token.cancel();
            *announcements_cancel_token_guard = None;
        } else {
            info!("Announcements not running");
        }
    }

    /// Starts listening for device announcements from the network.
    ///
    /// This method initializes listeners on all available multicast sockets to receive device announcements.
    /// It spawns a background task for each socket that listens for incoming UDP datagrams and processes them to discover devices.
    ///
    /// # Arguments
    /// * `self_fingerprint` - An optional fingerprint of the local device. If provided, announcements from this device will be ignored.
    ///
    /// # Returns
    /// `Result<()>` - Returns `Ok(())` if listeners are successfully started, or an error if socket initialization fails or listeners are already running.
    ///
    /// # Remarks
    /// This method returns immediately after starting the listeners in the background. Use [`shutdown`] to stop the listeners and the discovery service gracefully.
    pub async fn start_listening(&self, self_fingerprint: Option<String>) -> Result<()> {
        let sockets = self.get_sockets_with_retry().await?;
        info!("Starting to listen on {} interfaces", sockets.len());

        let mut token_guard = self.listening_cancel_token.lock().await;
        if token_guard.is_some() {
            return Err(anyhow!("Listener already running"));
        }
        let cancel_token = CancellationToken::new();
        *token_guard = Some(cancel_token.clone());
        drop(token_guard);

        {
            // Scope to set the new cancel token after checking and creating it
            let mut listening_cancel_token_guard = self.listening_cancel_token.lock().await;
            *listening_cancel_token_guard = Some(cancel_token.clone()); // Store the new cancel token
        }

        let devices = self.device_states.clone();
        let store = self.store.clone();

        let mut handles = Vec::with_capacity(sockets.len());
        for socket in sockets.iter() {
            let socket = Arc::clone(socket);
            let self_fingerprint = self_fingerprint.clone();
            let store = store.clone();
            let cancel_token = cancel_token.clone();
            let devices = devices.clone();

            // Spawn a listener task for each socket to handle incoming datagrams
            let handle = tokio::spawn(async move {
                let mut buf = vec![0u8; 65535]; // Buffer for incoming UDP packets (max size)
                loop {
                    tokio::select! {
                        _ = cancel_token.cancelled() => {
                            info!("Listener exiting on {}", socket.local_addr().unwrap());
                            break; // Exit loop if listening is cancelled
                        }
                        result = socket.recv_from(&mut buf) => {
                            match result {
                                Ok((len, addr)) => {
                                    // Process the received datagram
                                    if let Err(e) = Self::handle_datagram( // Using Self:: to call associated function
                                        &self_fingerprint,
                                        &buf[..len], // Slice buffer to received length
                                        addr,
                                        &store,
                                        &devices
                                    ).await {
                                        if e.to_string().contains("channel closed") {
                                            debug!("Channel closed, stopping listener on {}",
                                                socket.local_addr().unwrap());
                                            break; // Exit loop if channel is closed (likely during shutdown)
                                        }

                                        error!("Error handling datagram: {e}");
                                    }
                                }
                                Err(e) => error!("Receive error: {e}"), // Log socket receive errors
                            }
                        }
                    }
                }
            });
            handles.push(handle); // Store the handle to manage listener tasks
        }

        self.listeners.lock().await.extend(handles); // Add new handles to the list of listeners
        Ok(())
    }

    /// Handles an incoming device announcement datagram.
    ///
    /// This function is responsible for parsing the received datagram, extracting device information, and updating the device registry.
    /// It deserializes the JSON announcement, checks if it's from the local device (if `self_fingerprint` is provided),
    /// updates the `device_states` map with new or updated device information, and persists the changes if a store is configured.
    ///
    /// # Arguments
    /// * `self_fingerprint` - An optional fingerprint of the local device to ignore self-announcements.
    /// * `data` - A slice of bytes representing the received datagram data.
    /// * `addr` - The `SocketAddr` of the sender.
    /// * `store` - An optional `Arc<PersistentDataManager>` for persisting device information.
    /// * `devices` - An `Arc<DashMap>` storing the current device states.
    ///
    /// # Returns
    /// `Result<()>` - Returns `Ok(())` if the datagram is processed successfully, or an error if parsing or processing fails.
    async fn handle_datagram(
        self_fingerprint: &Option<String>,
        data: &[u8],
        addr: SocketAddr,
        store: &Option<Arc<PersistentDataManager<DeviceList>>>,
        devices: &Arc<DashMap<String, DiscoveredDevice>>,
    ) -> Result<()> {
        let announcement: serde_json::Value = serde_json::from_slice(data)?;

        // Extract fingerprint from the announcement, ignore if missing
        let fingerprint = match announcement.get("fingerprint") {
            Some(v) => v.as_str().unwrap_or_default().to_string(),
            None => return Ok(()), // Ignore announcements without fingerprint
        };

        // Ignore self-announcements if self_fingerprint is provided and matches
        if Some(fingerprint.clone()) == *self_fingerprint {
            return Ok(()); // Skip processing self-announcements
        }

        // Construct DiscoveredDevice from announcement data
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
            device_type: serde_json::from_value(announcement["deviceType"].clone())?, // Deserialize device type
            ips: vec![addr.ip()],  // Record sender IP address
            last_seen: Utc::now(), // Update last seen timestamp to now
        };

        // Update device state or insert new device if not already known
        if let Some(mut existing) = devices.get_mut(&fingerprint) {
            if !existing.ips.contains(&addr.ip()) {
                existing.ips.push(addr.ip()); // Add new IP if not already listed
            }
            existing.last_seen = Utc::now(); // Update last seen timestamp
        } else {
            devices.insert(fingerprint, device); // Insert new device into device states
        }

        // Persist device states if store is configured
        if let Some(store) = store {
            store
                .update(|_| async move {
                    let d = devices
                        .iter()
                        .map(|entry| entry.value().clone())
                        .collect::<Vec<_>>(); // Collect all device states to persist

                    Ok::<_, anyhow::Error>((DeviceList { devices: d }, ())) // Return collected devices for persistence update
                })
                .await?;
        }

        Ok(())
    }

    /// Stops the device listening service.
    ///
    /// This method cancels all active listener tasks, causing them to stop processing incoming device announcements.
    /// It signals the listener tasks to terminate gracefully but does not immediately abort any currently processing datagrams.
    pub async fn stop_listening(&self) {
        let mut listeners = self.listeners.lock().await;
        for handle in listeners.drain(..) {
            handle.abort(); // Forcefully terminate listener tasks to ensure immediate shutdown
        }

        let mut listening_cancel_token_guard = self.listening_cancel_token.lock().await;
        if let Some(token) = &*listening_cancel_token_guard {
            info!("Stop listening for new devices");
            token.cancel();
            *listening_cancel_token_guard = None;
        } else {
            info!("Listener not running");
        }
    }

    /// Retrieves a list of all currently discovered devices.
    ///
    /// This method returns a snapshot of all devices currently tracked by the discovery service. The list is derived from the internal
    /// `device_states` map and provides a read-only view of the discovered devices at the time of calling.
    ///
    /// # Returns
    /// `Vec<DiscoveredDevice>` - A vector containing all currently discovered devices.
    pub fn get_all_devices(&self) -> Vec<DiscoveredDevice> {
        self.device_states
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Shuts down the entire device discovery service gracefully.
    ///
    /// This method stops all active listeners and announcers, cleans up resources, and persists the current device states if persistence is enabled.
    /// It ensures that all ongoing operations are either completed or gracefully terminated before the service is fully stopped.
    ///
    /// # Returns
    /// `Result<()>` - Returns `Ok(())` if shutdown is successful, or an error if saving the device state fails.
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down the whole UDP multicast service");

        self.stop_announcements().await; // Stop device announcements
        self.stop_listening().await; // Stop device listening

        // Ensure device states are saved before shutdown if persistence is enabled
        self.save().await?;

        Ok(())
    }

    /// Saves the current device states to persistent storage if a store is configured.
    ///
    /// This method triggers a save operation on the persistent data manager, writing the current list of discovered devices
    /// to storage. This is typically called during shutdown or at other points where persistence of the device list is desired.
    ///
    /// # Returns
    /// `Result<()>` - Returns `Ok(())` if save operation is successful or if no store is configured, or an error if saving fails.
    pub async fn save(&self) -> Result<()> {
        if let Some(store) = &self.store {
            return store
                .update(|_| async move {
                    let d = self
                        .device_states
                        .iter()
                        .map(|entry| entry.value().clone())
                        .collect::<Vec<_>>(); // Collect current device states for saving

                    Ok::<_, anyhow::Error>((DeviceList { devices: d }, ())) // Return devices for persistence update
                })
                .await;
        }

        Ok(()) // No store configured, save operation is a no-op
    }

    /// Checks if the discovery service is operational, meaning sockets are successfully initialized.
    ///
    /// This method checks the initialization status of the multicast sockets. If socket initialization was successful,
    /// the service is considered operational and ready for announcements and listening.
    ///
    /// # Returns
    /// `bool` - Returns `true` if the service is operational (sockets initialized successfully), `false` otherwise.
    pub async fn is_operational(&self) -> bool {
        self.sockets_init
            .lock()
            .await
            .as_ref()
            .map(|r| r.is_ok())
            .unwrap_or(false)
    }

    /// Retrieves the current status of the discovery service, indicating if it's announcing.
    ///
    /// This method checks the internal state of the `DiscoveryService` to determine whether it is
    /// currently broadcasting device announcements.
    ///
    /// # Returns
    /// `bool` - A boolean flags indicating the announcement status.
    pub async fn is_announcing(&self) -> bool {
        self.announcements_cancel_token.lock().await.is_some()
    }

    /// Retrieves the current status of the discovery service, indicating if it's listening.
    ///
    /// This method checks the internal state of the `DiscoveryService` to determine whether it is
    /// currently listening for devices in local network.
    ///
    /// # Returns
    /// `bool` - A boolean flags indicating the listening status.
    pub async fn is_listening(&self) -> bool {
        self.listening_cancel_token.lock().await.is_some()
    }
}
