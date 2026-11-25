use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use colored::Colorize;
use log::error;
use tokio::sync::{Mutex, RwLock};

use discovery::{
    client::CertValidator,
    protocol::{DiscoveredDevice, DiscoveryService},
};

use crate::fs::VirtualFS;

pub struct AppState {
    pub fs: Arc<RwLock<VirtualFS>>,
    pub validator: CertValidator,
    pub discovery: Arc<Mutex<Option<DiscoveryService>>>,
    pub config_dir: PathBuf,
}

pub fn print_device_table(devices: &[DiscoveredDevice]) {
    for (i, dev) in devices.iter().enumerate() {
        let index = i + 1;
        let index_str = format!("[{index}]").red().bold();
        let alias = dev.alias.cyan().bold();
        let model_type = format!("{} ({})", dev.device_model, dev.device_type).blue();
        let main_ip = dev
            .ips
            .first()
            .map(|ip| ip.to_string())
            .unwrap_or_default()
            .white();
        let fingerprint_short: String = dev.fingerprint.chars().take(8).collect();
        let fingerprint = fingerprint_short.magenta();
        let last_seen = humantime::format_rfc3339_seconds(dev.last_seen.into())
            .to_string()
            .green();

        println!(
            "{} {} {} {} {} {} {} {} {}",
            index_str,
            alias,
            model_type,
            "•".bright_black(),
            main_ip,
            "•".bright_black(),
            fingerprint,
            "•".bright_black(),
            last_seen
        );
    }
}

pub fn print_device_details(dev: &DiscoveredDevice) {
    println!("{}", dev.alias.to_string().cyan().bold());

    println!("{}", "Device Configuration".yellow().bold());
    println!("    {:<12} {}", "Model:", dev.device_model.blue());
    println!("    {:<12} {}", "Type:", dev.device_type.to_string().blue());

    println!("    {:<12} {}", "Fingerprint:", dev.fingerprint.magenta());

    println!(
        "    {:<12} {}",
        "Last Seen:",
        humantime::format_rfc3339_seconds(dev.last_seen.into())
            .to_string()
            .green()
    );

    println!("{}", "Network Addresses".yellow().bold());
    for ip in &dev.ips {
        println!("    {}", ip.to_string().white());
    }
    println!();
}

pub fn print_certificate_table(validator: &CertValidator) {
    let fp_map = validator.fingerprints();

    match fp_map {
        Ok(fp_map) => {
            let mut fingerprints: Vec<_> = fp_map.keys().collect();
            fingerprints.sort();

            for (i, fp) in fingerprints.iter().enumerate() {
                let index = i + 1;
                let index_str = format!("[{index}]").red().bold();

                let fp_short: String = fp.chars().take(8).collect();
                let fp_display = fp_short.magenta();

                println!("{index_str} {fp_display}");

                if let Some(hosts) = fp_map.get(*fp) {
                    for host in hosts {
                        println!("    {}", host.to_string().white());
                    }
                }

                println!();
            }
        }
        Err(e) => error!("Unable to read the fingerprint map: {e}"),
    }
}

pub fn get_fingerprint_by_index(validator: &CertValidator, index: usize) -> Result<Option<String>> {
    let fp_map = validator.fingerprints()?;
    let mut fingerprints: Vec<_> = fp_map.keys().collect();
    fingerprints.sort();

    // Convert 1-based index to 0-based
    if index == 0 || index > fingerprints.len() {
        return Ok(None);
    }

    Ok(fingerprints.get(index - 1).map(|s| s.to_string()))
}
