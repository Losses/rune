use std::net::SocketAddr;

use reqwest::Client;
use serde_json::json;
use socket2::{Domain, Socket, Type};
use tokio::net::UdpSocket;

use crate::utils::DeviceInfo;

pub struct DiscoveryService {
    pub socket: UdpSocket,
    pub device_info: DeviceInfo,
    http_client: Client,
}

const MULTICAST_GROUP: &str = "224.0.0.167";
const MULTICAST_PORT: u16 = 57863;

impl DiscoveryService {
    pub async fn new(device_info: DeviceInfo) -> anyhow::Result<Self> {
        let addr: SocketAddr = format!("0.0.0.0:{}", MULTICAST_PORT).parse()?;
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(socket2::Protocol::UDP))?;

        socket.set_reuse_address(true)?;
        #[cfg(not(target_os = "windows"))]
        socket.set_reuse_port(true)?;
        socket.set_multicast_ttl_v4(2)?;
        socket.set_multicast_loop_v4(true)?;

        socket.bind(&addr.into())?;

        let socket: std::net::UdpSocket = socket.into();
        let socket = UdpSocket::from_std(socket)?;

        let interface = "0.0.0.0".parse()?;
        socket.join_multicast_v4(MULTICAST_GROUP.parse()?, interface)?;

        Ok(Self {
            socket,
            device_info,
            http_client: Client::new(),
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
        println!(
            "Sending announcement to {}:{}",
            MULTICAST_GROUP, MULTICAST_PORT
        );
        let bytes_sent = self
            .socket
            .send_to(&msg, format!("{}:{}", MULTICAST_GROUP, MULTICAST_PORT))
            .await?;
        println!("Sent {} bytes", bytes_sent);
        Ok(())
    }

    pub async fn listen(&self) -> anyhow::Result<()> {
        println!("Starting to listen for announcements...");
        let mut buf = vec![0u8; 65535];
        loop {
            match self.socket.recv_from(&mut buf).await {
                Ok((len, addr)) => {
                    println!("Received {} bytes from {}", len, addr);
                    if let Ok(announcement) = serde_json::from_slice(&buf[..len]) {
                        println!("GOT ANNOUNCEMENT: {}", announcement);
                        self.handle_announcement(announcement, addr).await?;
                    }
                }
                Err(e) => {
                    println!("Error receiving: {}", e);
                }
            }
        }
    }

    async fn handle_announcement(
        &self,
        announcement: serde_json::Value,
        addr: SocketAddr,
    ) -> anyhow::Result<()> {
        if let Some(fingerprint) = announcement.get("fingerprint") {
            if *fingerprint == *self.device_info.fingerprint {
                return Ok(());
            }
        }

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
            "alias": self.device_info.alias,
            "version": self.device_info.version,
            "deviceModel": self.device_info.device_model,
            "deviceType": self.device_info.device_type,
            "fingerprint": self.device_info.fingerprint,
            "api_port": self.device_info.api_port,
            "protocol": self.device_info.protocol,
            "download": self.device_info.download,
            "announce": false
        });

        let target_url = format!("{}://{}:{}/api/rune/v2/register", protocol, addr.ip(), port);

        match self
            .http_client
            .post(&target_url)
            .json(&response)
            .send()
            .await
        {
            Ok(_) => return Ok(()),
            Err(_) => {
                let msg = serde_json::to_vec(&response)?;
                self.socket.send_to(&msg, addr).await?;
            }
        }

        Ok(())
    }
}
