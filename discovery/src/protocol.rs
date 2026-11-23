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
use tokio::time::MissedTickBehavior;

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
use std::future;
use tokio::task;
use tokio::time as tokio_time;
use tokio::time::Instant;

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
    /// Advertised application port, absent in older saved observations.
    #[serde(default)]
    pub api_port: Option<u16>,
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
    /// Lazily initialized multicast sockets, wrapped in a Mutex for thread-safe access.
    /// Only a successful initialization is cached; a failure is returned to the caller so a
    /// later attempt can try the available interfaces again.
    sockets_init: Mutex<Option<Vec<Arc<UdpSocket>>>>,
    /// Optional persistence layer for storing and retrieving discovered device information.
    store: Option<Arc<PersistentDataManager<DeviceList>>>,
    /// List of active listener tasks, managed to allow for graceful shutdown.
    listeners: Mutex<Vec<JoinHandle<()>>>,
    /// Policy defining retry behavior for network operations like socket initialization.
    retry_policy: RetryPolicy,
    /// A map storing detailed information about all discovered devices, indexed by fingerprint.
    device_states: Arc<DashMap<String, DiscoveredDevice>>,

    announcements: Mutex<Option<AnnouncementTask>>,
    /// Cancellation token for controlling the lifecycle of device listening tasks.
    listening_cancel_token: Mutex<Option<CancellationToken>>,
}

struct AnnouncementTask {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
    deadline: Option<tokio_time::Instant>,
}

impl AnnouncementTask {
    fn is_active(&self) -> bool {
        !self.handle.is_finished()
            && self
                .deadline
                .is_none_or(|deadline| deadline > Instant::now())
    }

    async fn stop(self) {
        self.cancel.cancel();
        if let Err(error) = self.handle.await {
            error!("Announcement task failed: {error}");
        }
    }
}

/// An announcement operation is already active.
#[derive(Debug, thiserror::Error)]
#[error("Announcements already running")]
pub struct AnnouncementsAlreadyRunning {
    /// Whether the existing task has no expiration deadline.
    pub indefinite: bool,
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

/// The announcement a device multicasts to describe itself to its neighbours.
pub fn announcement_message(device_info: &DeviceInfo) -> serde_json::Value {
    json!({
        "alias": device_info.alias,
        "version": device_info.version,
        "deviceModel": device_info.device_model,
        "deviceType": device_info.device_type,
        "fingerprint": device_info.fingerprint,
        "api_port": device_info.api_port,
        "announce": true
    })
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

            announcements: Mutex::new(None),
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

            announcements: Mutex::new(None),
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
    /// defined by the service's `retry_policy` to handle potential socket initialization failures. A successful initialization
    /// is reused for subsequent calls. A failed initialization is not cached, so a later call tries the available interfaces
    /// again instead of returning the previous error.
    ///
    /// # Returns
    /// `Result<Arc<Vec<Arc<UdpSocket>>>>` - A `Result` containing a vector of `Arc<UdpSocket>` representing the initialized multicast sockets,
    ///                                     or an error if socket initialization fails after all retry attempts.
    async fn get_sockets_with_retry(&self) -> Result<Arc<Vec<Arc<UdpSocket>>>> {
        let mut lock = self.sockets_init.lock().await;

        if let Some(sockets) = &*lock {
            return Ok(Arc::new(sockets.clone()));
        }

        let mut retries = self.retry_policy.max_retries;
        let mut backoff = self.retry_policy.initial_backoff;
        let mut last_error = None;

        // Retry loop with exponential backoff
        while retries > 0 {
            match Self::try_init_sockets().await {
                Ok(sockets) => {
                    let sockets = Arc::new(sockets);
                    *lock = Some(sockets.to_vec());
                    return Ok(sockets);
                }
                Err(e) => {
                    last_error = Some(e);
                    retries -= 1;
                    tokio_time::sleep(backoff).await;
                    backoff *= 2;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("Socket initialization failed")))
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
                let socket = task::spawn_blocking(move || {
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
        let mut announcements = self.announcements.lock().await;
        if let Some(task) = announcements.as_ref().filter(|task| task.is_active()) {
            return Err(AnnouncementsAlreadyRunning {
                indefinite: task.deadline.is_none(),
            }
            .into());
        }
        if let Some(task) = announcements.take() {
            task.stop().await;
        }

        let sockets = self.get_sockets_with_retry().await?;
        let deadline = duration_limit.map(|limit| Instant::now() + limit);
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();

        info!("Starting device announcements");
        info!("The device fingerprint is: {}", device_info.fingerprint);

        let handle = tokio::spawn(async move {
            let expire = async {
                match deadline {
                    Some(deadline) => tokio_time::sleep_until(deadline).await,
                    None => future::pending().await,
                }
            };
            let announce = async {
                let mut interval = tokio_time::interval(interval);
                interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
                let announcement = announcement_message(&device_info);
                loop {
                    interval.tick().await;
                    if let Err(error) = Self::send_announcement(&sockets, &announcement).await {
                        error!("Announcement failed: {error}");
                    }
                }
            };
            tokio::select! {
                biased;
                _ = task_cancel.cancelled() => {}
                _ = expire => info!("Announcement duration limit reached"),
                _ = announce => {}
            }
        });
        *announcements = Some(AnnouncementTask {
            cancel,
            handle,
            deadline,
        });

        Ok(())
    }

    /// Stops the device announcement service.
    ///
    /// Cancels the task and waits for it to finish before allowing another start.
    pub async fn stop_announcements(&self) {
        let mut announcements = self.announcements.lock().await;
        if let Some(task) = announcements.take() {
            task.stop().await;
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
            api_port: announcement
                .get("api_port")
                .and_then(|value| value.as_u64())
                .and_then(|port| u16::try_from(port).ok())
                .filter(|port| *port != 0),
        };

        // Update device state or insert new device if not already known
        if let Some(mut existing) = devices.get_mut(&fingerprint) {
            if !existing.ips.contains(&addr.ip()) {
                existing.ips.push(addr.ip()); // Add new IP if not already listed
            }
            existing.last_seen = Utc::now(); // Update last seen timestamp
            if device.api_port.is_some() {
                existing.api_port = device.api_port;
            }
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

    /// Process a received discovery announcement from a caller-owned listener.
    /// The announcement is advisory; callers must authenticate the peer separately.
    pub async fn observe_announcement(
        &self,
        data: &[u8],
        source: SocketAddr,
        self_fingerprint: Option<String>,
    ) -> Result<()> {
        Self::handle_datagram(
            &self_fingerprint,
            data,
            source,
            &self.store,
            &self.device_states,
        )
        .await
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
        self.sockets_init.lock().await.is_some()
    }

    /// Retrieves the current status of the discovery service, indicating if it's announcing.
    ///
    /// This method checks the internal state of the `DiscoveryService` to determine whether it is
    /// currently broadcasting device announcements.
    ///
    /// # Returns
    /// `bool` - A boolean flags indicating the announcement status.
    pub async fn is_announcing(&self) -> bool {
        self.announcements
            .lock()
            .await
            .as_ref()
            .is_some_and(AnnouncementTask::is_active)
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

#[cfg(test)]
mod endpoint_tests {
    use super::*;

    #[test]
    fn old_saved_device_without_port_still_loads() {
        let json = r#"{"alias":"old","device_model":"x","device_type":"desktop","fingerprint":"fp","last_seen":0,"ips":["127.0.0.1"]}"#;
        let device: DiscoveredDevice = serde_json::from_str(json).unwrap();
        assert_eq!(device.api_port, None);
    }

    #[tokio::test]
    async fn announcements_record_and_refresh_advertised_port() {
        let devices = Arc::new(DashMap::new());
        let addr: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        for port in [4567, 7654] {
            let data = serde_json::json!({"fingerprint":"fp","alias":"peer","deviceModel":"x","deviceType":"desktop","api_port":port});
            DiscoveryService::handle_datagram(
                &None,
                &serde_json::to_vec(&data).unwrap(),
                addr,
                &None,
                &devices,
            )
            .await
            .unwrap();
            assert_eq!(devices.get("fp").unwrap().api_port, Some(port));
        }
    }

    /// Holds the multicast port so the first socket initialization fails, releases it, and
    /// starts again on the same service. The second attempt must succeed, proving the first
    /// bind error was not cached.
    ///
    /// The test needs an up, multicast, IPv4 interface and a free multicast port. On a host
    /// without either it reports why it is not applicable instead of asserting on an outcome
    /// that cannot distinguish a retry from a cached error.
    #[cfg(unix)]
    #[tokio::test]
    async fn failed_socket_initialization_is_retried_on_the_next_start() {
        if !get_multicast_interfaces()
            .iter()
            .any(|interface| !interface.ipv4.is_empty())
        {
            eprintln!(
                "skipping failed_socket_initialization_is_retried_on_the_next_start: \
                 no up, multicast, IPv4 interface on this host, so a successful retry cannot be proven"
            );
            return;
        }

        // A socket without SO_REUSEPORT keeps the discovery sockets from binding the port.
        // If another process already owns it, the test cannot create the bind failure itself.
        let port_holder = match std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, MULTICAST_PORT)) {
            Ok(socket) => socket,
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                eprintln!(
                    "skipping failed_socket_initialization_is_retried_on_the_next_start: \
                     multicast port {MULTICAST_PORT} is already in use; stop the other Rune instance and rerun"
                );
                return;
            }
            Err(error) => panic!("failed to bind the multicast port for the test: {error}"),
        };

        let device_info = DeviceInfo {
            alias: "retry-test".to_owned(),
            version: "0.0.0".to_owned(),
            device_model: None,
            device_type: None,
            fingerprint: "retry-test".to_owned(),
            api_port: 0,
            protocol_version: None,
            capabilities: None,
        };
        let service = DiscoveryService::without_store().with_retry_policy(RetryPolicy {
            max_retries: 1,
            initial_backoff: Duration::from_millis(1),
        });

        let first = service
            .start_announcements(device_info.clone(), Duration::from_secs(3), None)
            .await
            .expect_err("socket initialization should fail while the multicast port is taken");
        assert!(
            first.to_string().contains("Address already in use"),
            "the first start should fail on the occupied port: {first:#}"
        );
        assert!(!service.is_announcing().await);
        assert!(!service.is_operational().await);
        service.stop_announcements().await;

        drop(port_holder);

        service
            .start_announcements(device_info.clone(), Duration::from_secs(3), None)
            .await
            .expect("the next start should retry socket initialization and succeed");
        assert!(service.is_announcing().await);
        assert!(service.is_operational().await);

        let already_running = service
            .start_announcements(device_info.clone(), Duration::from_secs(3), None)
            .await
            .expect_err("a second start must not replace the running announcements");
        assert!(
            already_running
                .downcast_ref::<AnnouncementsAlreadyRunning>()
                .is_some_and(|error| error.indefinite)
        );

        service.stop_announcements().await;
        service
            .start_announcements(device_info.clone(), Duration::from_secs(3), None)
            .await
            .expect("announcements should start again after stopping");
        service.stop_announcements().await;

        let (left, right) = tokio::join!(
            service.start_announcements(device_info.clone(), Duration::from_secs(3), None),
            service.start_announcements(device_info, Duration::from_secs(3), None),
        );
        assert!(
            left.is_ok() ^ right.is_ok(),
            "exactly one of two concurrent starts should run announcements"
        );
        service.stop_announcements().await;
    }
}
