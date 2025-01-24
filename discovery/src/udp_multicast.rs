use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::SystemTime,
};

use futures::future::join_all;
use netdev::Interface;
use reqwest::Client;
use serde_json::json;
use socket2::{Domain, Socket, Type};
use tokio::{net::UdpSocket, sync::mpsc::Sender};

use crate::utils::DeviceInfo;

#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub alias: String,
    pub device_model: String,
    pub device_type: String,
    pub fingerprint: String,
    pub last_seen: SystemTime,
}

pub struct DiscoveryService {
    pub sockets: Vec<Arc<UdpSocket>>,
    pub device_info: DeviceInfo,
    http_client: Client,
    event_tx: Sender<DiscoveredDevice>,
}

const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);
const MULTICAST_PORT: u16 = 57863;

fn get_multicast_interfaces() -> Vec<Interface> {
    netdev::get_interfaces()
        .into_iter()
        .filter(|iface| iface.is_up() && iface.is_multicast())
        .collect()
}

impl DiscoveryService {
    pub async fn new(
        device_info: DeviceInfo,
        event_tx: Sender<DiscoveredDevice>,
    ) -> anyhow::Result<Self> {
        let mut sockets = Vec::new();

        for iface in get_multicast_interfaces() {
            for ipv4_net in iface.ipv4 {
                let interface_ip = ipv4_net.addr();

                let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(socket2::Protocol::UDP))?;

                let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), MULTICAST_PORT);

                socket.set_reuse_address(true)?;
                #[cfg(not(target_os = "windows"))]
                socket.set_reuse_port(true)?;
                socket.bind(&bind_addr.into())?;

                socket.set_multicast_ttl_v4(255)?;
                socket.set_multicast_loop_v4(true)?;
                socket.set_multicast_if_v4(&interface_ip)?;
                socket.join_multicast_v4(&MULTICAST_GROUP, &interface_ip)?;

                let std_socket: std::net::UdpSocket = socket.into();
                let tokio_socket = UdpSocket::from_std(std_socket)?;

                sockets.push(Arc::new(tokio_socket));
            }
        }

        Ok(Self {
            sockets,
            device_info,
            http_client: Client::new(),
            event_tx,
        })
    }

    pub async fn announce(&self) -> anyhow::Result<()> {
        let announcement = json!({
            "alias": self.device_info.alias,
            "version": self.device_info.version,
            "deviceModel": self.device_info.device_model,
            "deviceType": self.device_info.device_type,
            "fingerprint": self.device_info.fingerprint,
            "api_port": self.device_info.api_port,
            "protocol": self.device_info.protocol,
            "download": self.device_info.download,
            "announce": true
        });

        let msg = serde_json::to_vec(&announcement)?;

        for socket in &self.sockets {
            let target = format!("{}:{}", MULTICAST_GROUP, MULTICAST_PORT);
            match socket.send_to(&msg, &target).await {
                Ok(bytes_sent) => println!("[{}] Sent {} bytes", socket.local_addr()?, bytes_sent),
                Err(e) => eprintln!("Send error on {}: {}", socket.local_addr()?, e),
            }
        }

        Ok(())
    }

    pub async fn listen(&self) -> anyhow::Result<()> {
        println!("Starting to listen for announcements...");
        let mut join_handles = Vec::new();

        let sockets = self.sockets.clone();
        let device_info = self.device_info.clone();
        let http_client = self.http_client.clone();
        let event_tx = self.event_tx.clone();

        for socket in sockets {
            println!("Listening to {}...", socket.local_addr()?);
            let device_info = device_info.clone();
            let http_client = http_client.clone();
            let event_tx = event_tx.clone();

            join_handles.push(tokio::spawn(async move {
                let mut buf = vec![0u8; 65535];
                loop {
                    match socket.recv_from(&mut buf).await {
                        Ok((len, addr)) => match serde_json::from_slice(&buf[..len]) {
                            Ok(announcement) => {
                                println!("[{:#?}] Received from {}", socket.local_addr(), addr);
                                if let Err(e) = DiscoveryService::handle_announcement(
                                    &device_info,
                                    &http_client,
                                    &socket,
                                    announcement,
                                    addr,
                                    &event_tx,
                                )
                                .await
                                {
                                    eprintln!("Error handling announcement: {}", e);
                                }
                            }
                            Err(e) => {
                                eprintln!("Failed to parse announcement: {}", e);
                                eprintln!("Raw data: {:?}", &buf[..len]);
                                continue;
                            }
                        },
                        Err(e) => eprintln!("Receive error: {}", e),
                    }
                }
            }));
        }

        join_all(join_handles).await;
        Ok(())
    }

    async fn handle_announcement(
        device_info: &DeviceInfo,
        http_client: &Client,
        socket: &UdpSocket,
        announcement: serde_json::Value,
        addr: SocketAddr,
        event_tx: &Sender<DiscoveredDevice>,
    ) -> anyhow::Result<()> {
        println!("Received announcement from {}: {}", addr, announcement);

        if let Some(fingerprint) = announcement.get("fingerprint") {
            if *fingerprint == *device_info.fingerprint {
                println!("[DEBUG] Received self-announcement, skipping");
                return Ok(());
            }
        } else {
            eprintln!("Announcement missing fingerprint, skipping");
            return Ok(());
        }

        let discovered = DiscoveredDevice {
            alias: announcement["alias"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string(),
            device_model: announcement["deviceModel"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string(),
            device_type: announcement["deviceType"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string(),
            fingerprint: announcement["fingerprint"].as_str().unwrap().to_string(),
            last_seen: SystemTime::now(),
        };

        let _ = event_tx.send(discovered).await;

        if !announcement
            .get("announce")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Ok(());
        }

        let port = announcement
            .get("api_port")
            .and_then(|v| v.as_u64())
            .unwrap_or(53317) as u16;
        let protocol = announcement
            .get("protocol")
            .and_then(|v| v.as_str())
            .unwrap_or("http");

        let response = json!({
            "alias": device_info.alias,
            "version": device_info.version,
            "deviceModel": device_info.device_model,
            "deviceType": device_info.device_type,
            "fingerprint": device_info.fingerprint,
            "api_port": device_info.api_port,
            "protocol": device_info.protocol,
            "download": device_info.download,
            "announce": false
        });

        let target_url = format!("{}://{}:{}/api/rune/v2/register", protocol, addr.ip(), port);

        match http_client.post(&target_url).json(&response).send().await {
            Ok(_) => Ok(()),
            Err(e) => {
                let msg = serde_json::to_vec(&response)?;
                socket.send_to(&msg, addr).await?;
                eprintln!("Failed to send discovered device: {}", e);
                Ok(())
            }
        }
    }
}
