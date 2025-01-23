use std::net::SocketAddr;

use reqwest::Client;
use serde_json::json;
use tokio::net::UdpSocket;

use crate::utils::DeviceInfo;

pub struct DiscoveryService {
    pub socket: UdpSocket,
    pub device_info: DeviceInfo,
    http_client: Client,
}

impl DiscoveryService {
    pub async fn new(device_info: DeviceInfo) -> anyhow::Result<Self> {
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", device_info.port)).await?;
        socket.join_multicast_v4("224.0.0.167".parse()?, "0.0.0.0".parse()?)?;

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
            "port": self.device_info.port,
            "protocol": self.device_info.protocol,
            "download": self.device_info.download,
            "announce": true
        });

        let msg = serde_json::to_vec(&announcement)?;
        self.socket
            .send_to(&msg, format!("224.0.0.167:{}", self.device_info.port))
            .await?;
        Ok(())
    }

    pub async fn listen(&self) -> anyhow::Result<()> {
        let mut buf = vec![0u8; 65535];
        loop {
            let (len, addr) = self.socket.recv_from(&mut buf).await?;
            if let Ok(announcement) = serde_json::from_slice(&buf[..len]) {
                self.handle_announcement(announcement, addr).await?;
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
            .get("port")
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
            "port": self.device_info.port,
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
                self.socket
                    .send_to(&msg, format!("224.0.0.167:{}", self.device_info.port))
                    .await?;
            }
        }

        Ok(())
    }
}
