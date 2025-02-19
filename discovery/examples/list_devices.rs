/// Important Note for Local Development
///
/// When testing device discovery functionality on a single machine (localhost),
/// you must enable multicast on the loopback interface first.
///
/// Run the following command:
///
/// ```bash
/// sudo ip link set lo multicast on
/// ```
///
/// This is necessary because:
/// 1. The discovery service uses UDP multicast for device announcements
/// 2. By default, the loopback interface (lo) has multicast disabled
/// 3. Without enabling multicast on loopback, the service won't be able to receive
///    its own announcements during local testing
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use clap::Parser;
use rand::Rng;
use tokio::signal;
use tokio::sync::{broadcast, RwLock};
use tokio::time;
use uuid::Uuid;

use discovery::udp_multicast::{DiscoveredDevice, DiscoveryServiceImplementation};
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
    discovery_service: Arc<DiscoveryServiceImplementation>,
    devices: Arc<RwLock<HashMap<String, DiscoveredDevice>>>,
}

impl DeviceDiscovery {
    async fn new() -> anyhow::Result<Self> {
        let (event_tx, mut event_rx) = broadcast::channel(100);

        let discovery_service = Arc::new(DiscoveryServiceImplementation::new(event_tx));
        let devices = Arc::new(RwLock::new(HashMap::new()));

        let devices_clone = devices.clone();
        tokio::spawn(async move {
            while let Ok(device) = event_rx.recv().await {
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

    async fn start(&self, device_info: DeviceInfo) -> anyhow::Result<()> {
        let devices = self.devices.clone();

        let listen_service = self.discovery_service.clone();
        let self_fingerprint = device_info.fingerprint.clone();
        tokio::spawn(async move {
            if let Err(e) = listen_service.listen(Some(self_fingerprint), None).await {
                eprintln!("Error in listen task: {}", e);
            }
        });

        let cleanup_devices = devices.clone();
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                Self::cleanup_old_devices(&cleanup_devices).await;
            }
        });

        let discovery_service = self.discovery_service.clone();
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(3));
            loop {
                interval.tick().await;
                if let Err(e) = discovery_service.announce(device_info.clone()).await {
                    eprintln!("Failed to announce: {}", e);
                }
            }
        });

        Ok(())
    }

    async fn cleanup_old_devices(devices: &RwLock<HashMap<String, DiscoveredDevice>>) {
        let mut devices = devices.write().await;
        let now = Utc::now();
        devices.retain(|_, device| {
            let duration = now.signed_duration_since(device.last_seen);
            let secs = duration.num_seconds();
            (0..10).contains(&secs)
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
    };

    let discovery = DeviceDiscovery::new().await?;
    discovery.start(device_info).await?;

    signal::ctrl_c().await?;
    println!("\nReceived Ctrl+C, shutting down...");

    Ok(())
}
