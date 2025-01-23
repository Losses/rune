use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use clap::Parser;
use rand::Rng;
use tokio::sync::RwLock;
use tokio::time;
use uuid::Uuid;

use discovery::udp_multicast::{DiscoveredDevice, DiscoveryService};
use discovery::utils::{DeviceInfo, DeviceType};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short, long)]
    alias: Option<String>,

    #[clap(short, long)]
    device_model: Option<String>,

    #[clap(short, long)]
    port: Option<u16>,
}

struct DeviceDiscovery {
    discovery_service: Arc<DiscoveryService>,
    devices: Arc<RwLock<HashMap<String, DiscoveredDevice>>>,
}

impl DeviceDiscovery {
    async fn new(device_info: DeviceInfo) -> anyhow::Result<Self> {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(100);

        let discovery_service = Arc::new(DiscoveryService::new(device_info, event_tx).await?);
        let devices = Arc::new(RwLock::new(HashMap::new()));

        let devices_clone = devices.clone();
        tokio::spawn(async move {
            while let Some(device) = event_rx.recv().await {
                let mut devices = devices_clone.write().await;
                devices.insert(device.fingerprint.clone(), device);

                println!("\nDiscovered Devices:");
                println!("{:<20} {:<15} {:<15}", "Alias", "Model", "Type");
                println!("{:-<52}", "");
                for device in devices.values() {
                    println!(
                        "{:<20} {:<15} {:<15}",
                        device.alias, device.device_model, device.device_type
                    );
                }
            }
        });

        Ok(Self {
            discovery_service,
            devices,
        })
    }

    async fn start(&self) -> anyhow::Result<()> {
        let devices = self.devices.clone();

        // Task to clean up old devices
        let cleanup_devices = devices.clone();
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                Self::cleanup_old_devices(&cleanup_devices).await;
            }
        });

        // Broadcast task
        let discovery_service = self.discovery_service.clone();
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(3));
            loop {
                interval.tick().await;
                if let Err(e) = discovery_service.announce().await {
                    eprintln!("Failed to announce: {}", e);
                }
            }
        });

        Ok(())
    }

    async fn cleanup_old_devices(devices: &RwLock<HashMap<String, DiscoveredDevice>>) {
        let mut devices = devices.write().await;
        let now = SystemTime::now();
        devices.retain(|_, device| {
            now.duration_since(device.last_seen)
                .map(|duration| duration.as_secs() < 10)
                .unwrap_or(false)
        });
    }
}

fn generate_random_alias() -> String {
    let mut rng = rand::thread_rng();
    format!("Device-{:04x}", rng.gen::<u16>())
}

fn generate_random_model() -> String {
    let mut rng = rand::thread_rng();
    format!("Model-{:04x}", rng.gen::<u16>())
}

fn generate_random_port() -> u16 {
    rand::thread_rng().gen_range(1024..=65535)
}

fn generate_fingerprint() -> String {
    Uuid::new_v4().to_string()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Starting device discovery...");
    println!("Press Ctrl+C to exit");

    let args = Args::parse();

    let alias = args.alias.unwrap_or_else(generate_random_alias);
    let device_model = args.device_model.unwrap_or_else(generate_random_model);
    let api_port = args.port.unwrap_or_else(generate_random_port);
    let fingerprint = generate_fingerprint();

    let device_info = DeviceInfo {
        alias,
        version: "1.0.0".to_string(),
        device_model: Some(device_model),
        device_type: Some(DeviceType::Headless),
        fingerprint,
        api_port,
        protocol: "http".to_string(),
        download: Some(false),
    };

    let discovery = DeviceDiscovery::new(device_info).await?;
    discovery.start().await?;

    Ok(())
}
