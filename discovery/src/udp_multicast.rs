use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use log::{debug, error, info, warn};
use netdev::Interface;
use serde::{Deserialize, Serialize};
use serde_json::json;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::{
    net::UdpSocket,
    sync::{broadcast::Sender, Mutex},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::utils::{DeviceInfo, DeviceType};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    pub alias: String,
    pub device_model: String,
    pub device_type: DeviceType,
    pub fingerprint: String,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub last_seen: DateTime<Utc>,
    pub ips: Vec<IpAddr>,
}

const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);
const MULTICAST_PORT: u16 = 57863;

pub struct DiscoveryService {
    sockets_init: Mutex<Option<anyhow::Result<Vec<Arc<UdpSocket>>>>>,
    event_tx: Sender<DiscoveredDevice>,
    listeners: Mutex<Vec<JoinHandle<()>>>,
    retry_policy: RetryPolicy,
    device_ips: Arc<Mutex<HashMap<String, Vec<IpAddr>>>>,
}

#[derive(Clone)]
pub struct RetryPolicy {
    max_retries: usize,
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

fn get_multicast_interfaces() -> Vec<Interface> {
    netdev::get_interfaces()
        .into_iter()
        .filter(|iface| iface.is_up() && iface.is_multicast())
        .collect()
}

impl DiscoveryService {
    pub fn new(event_tx: Sender<DiscoveredDevice>) -> Self {
        Self {
            sockets_init: Mutex::new(None),
            event_tx,
            listeners: Mutex::new(Vec::new()),
            retry_policy: RetryPolicy::default(),
            device_ips: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    async fn get_sockets_with_retry(&self) -> anyhow::Result<Arc<Vec<Arc<UdpSocket>>>> {
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

    async fn try_init_sockets() -> anyhow::Result<Vec<Arc<UdpSocket>>> {
        let mut sockets = Vec::new();

        for iface in get_multicast_interfaces() {
            for ipv4_net in iface.ipv4 {
                let interface_ip = ipv4_net.addr();

                let socket = tokio::task::spawn_blocking(move || {
                    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
                    let bind_addr =
                        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), MULTICAST_PORT);

                    socket.set_reuse_address(true)?;
                    #[cfg(not(target_os = "windows"))]
                    socket.set_reuse_port(true)?;
                    socket.bind(&bind_addr.into())?;

                    socket.set_multicast_ttl_v4(255)?;
                    socket.set_multicast_loop_v4(true)?;
                    socket.set_multicast_if_v4(&interface_ip)?;
                    socket.join_multicast_v4(&MULTICAST_GROUP, &interface_ip)?;

                    Ok::<_, anyhow::Error>(socket)
                })
                .await??;

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

    pub async fn announce(&self, device_info: DeviceInfo) -> anyhow::Result<()> {
        let sockets = self.get_sockets_with_retry().await?;
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

        let msg = serde_json::to_vec(&announcement)?;

        for socket in sockets.iter() {
            let target = format!("{}:{}", MULTICAST_GROUP, MULTICAST_PORT);
            match socket.send_to(&msg, &target).await {
                Ok(bytes_sent) => debug!("[{}] Sent {} bytes", socket.local_addr()?, bytes_sent),
                Err(e) => error!("Send error on {}: {}", socket.local_addr()?, e),
            }
        }

        Ok(())
    }

    pub async fn listen(
        &self,
        self_fingerprint: Option<String>,
        cancel_token: Option<CancellationToken>,
    ) -> anyhow::Result<()> {
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

            let handle = tokio::spawn(async move {
                let mut buf = vec![0u8; 65535];
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

    async fn handle_datagram(
        self_fingerprint: &Option<String>,
        data: &[u8],
        addr: SocketAddr,
        event_tx: &Sender<DiscoveredDevice>,
        device_ips: &Arc<Mutex<HashMap<String, Vec<IpAddr>>>>,
    ) -> anyhow::Result<()> {
        let announcement: serde_json::Value =
            serde_json::from_slice(data).context("Failed to parse announcement")?;

        debug!("Received announcement from {}: {}", addr, announcement);

        let fingerprint = match announcement.get("fingerprint") {
            Some(v) => v.as_str().unwrap_or_default().to_string(),
            None => {
                warn!("Received announcement without fingerprint");
                return Ok(());
            }
        };

        if Some(fingerprint.clone()) == *self_fingerprint {
            debug!("Ignoring self-announcement");
            return Ok(());
        }

        let source_ip = addr.ip();
        let mut ips_map = device_ips.lock().await;
        let ips_entry = ips_map.entry(fingerprint.clone()).or_default();

        let is_new_ip = !ips_entry.contains(&source_ip);
        if is_new_ip {
            ips_entry.push(source_ip);
        }
        let current_ips = ips_entry.clone();
        drop(ips_map);

        let device_type_value = announcement
            .get("deviceType")
            .ok_or_else(|| anyhow!("Announcement missing deviceType"))?;
        let device_type: DeviceType = serde_json::from_value(device_type_value.clone())
            .context("Failed to parse deviceType")?;

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

        event_tx.send(discovered)?;

        Ok(())
    }

    pub async fn shutdown(&self) {
        let mut listeners = self.listeners.lock().await;
        for handle in listeners.drain(..) {
            handle.abort();
        }
    }

    pub async fn is_operational(&self) -> bool {
        self.sockets_init
            .lock()
            .await
            .as_ref()
            .map(|r| r.is_ok())
            .unwrap_or(false)
    }
}
