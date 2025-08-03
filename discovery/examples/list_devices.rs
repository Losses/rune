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
use std::process::exit;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use clearscreen::clear;
use rand::Rng;
use tokio::signal;
use tokio::time::sleep;
use uuid::Uuid;

use discovery::protocol::DiscoveryService;
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
}

impl DeviceDiscovery {
    async fn new() -> anyhow::Result<Self> {
        let discovery_service = Arc::new(DiscoveryService::without_store());
        Ok(Self { discovery_service })
    }

    async fn start(&self, device_info: DeviceInfo) -> anyhow::Result<()> {
        let listen_service = self.discovery_service.clone();
        let self_fingerprint = device_info.fingerprint.clone();
        tokio::spawn(async move {
            if let Err(e) = listen_service.start_listening(Some(self_fingerprint)).await {
                println!("Error in listen task: {e}");
            } else {
                println!("Listening for devices...");
            }
        });

        let discovery_service = self.discovery_service.clone();
        tokio::spawn(async move {
            if let Err(e) = discovery_service
                .start_announcements(device_info.clone(), Duration::from_secs(1), None)
                .await
            {
                println!("Failed to announce: {e}");
            } else {
                println!("Announcing device with alias: {}", device_info.alias);
            }
        });

        let listen_service = self.discovery_service.clone();
        tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(1)).await;
                let devices = listen_service.get_all_devices();

                clear().unwrap();
                println!("\nDiscovered Devices:");
                println!("{:<20} {:<15} {:<15}", "Alias", "Model", "Type");
                println!("{:-<52}", "");
                for device in devices {
                    println!(
                        "{:<20} {:<15} {:<15}",
                        device.alias, device.device_model, device.device_type
                    );
                }
            }
        });

        Ok(())
    }
}

fn generate_random_alias() -> String {
    let mut rng = rand::thread_rng();
    format!("Device-{:04x}", rng.r#gen::<u16>())
}

fn generate_random_model() -> String {
    let mut rng = rand::thread_rng();
    format!("Model-{:04x}", rng.r#gen::<u16>())
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
    discovery.discovery_service.shutdown().await?;
    exit(0);
}
