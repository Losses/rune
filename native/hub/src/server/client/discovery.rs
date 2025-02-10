use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::Result;
use log::error;
use tokio_util::sync::CancellationToken;

use discovery::{
    udp_multicast::{DiscoveredDevice, DiscoveryService},
    utils::DeviceInfo,
};

use crate::utils::update_device_list;

#[derive(Clone)]
pub struct DiscoveryStore {
    path: PathBuf,
}

impl DiscoveryStore {
    pub fn new<P: AsRef<Path>>(base_path: P) -> Self {
        Self {
            path: base_path.as_ref().join("discovered.toml"),
        }
    }

    pub async fn load(&self) -> Result<Vec<DiscoveredDevice>, anyhow::Error> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let content = tokio::fs::read_to_string(&self.path).await?;
        Ok(toml::from_str(&content)?)
    }

    pub async fn save(&self, devices: &[DiscoveredDevice]) -> Result<(), anyhow::Error> {
        let content = toml::to_string(devices)?;
        tokio::fs::write(&self.path, content).await?;
        Ok(())
    }
}

pub struct DiscoveryRuntime {
    pub service: Arc<DiscoveryService>,
    pub store: Arc<DiscoveryStore>,
    pub cancel_token: CancellationToken,
}

impl DiscoveryRuntime {
    pub async fn new(config_dir: &Path) -> Result<Self> {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(32);
        let store = Arc::new(DiscoveryStore::new(config_dir));
        let service = Arc::new(DiscoveryService::new(event_tx));

        let store_clone = store.clone();
        tokio::spawn(async move {
            while let Some(device) = event_rx.recv().await {
                let mut devices = store_clone.load().await.unwrap_or_default();
                update_device_list(&mut devices, device);
                let _ = store_clone.save(&devices).await;
            }
        });

        Ok(Self {
            service,
            store,
            cancel_token: CancellationToken::new(),
        })
    }

    pub async fn start_service(&self, device_info: DeviceInfo, interval: Duration) -> Result<()> {
        self.service
            .listen(device_info.clone(), Some(self.cancel_token.clone()))
            .await?;

        let service = self.service.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval);
            loop {
                interval.tick().await;
                if let Err(e) = service.announce(device_info.clone()).await {
                    error!("Announcement failed: {}", e);
                }
            }
        });

        Ok(())
    }
}
