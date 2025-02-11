use std::{path::PathBuf, sync::Arc};

use tokio::sync::{Mutex, RwLock};

use discovery::{udp_multicast::DiscoveredDevice, verifier::CertValidator};

use crate::{discovery::DiscoveryRuntime, fs::VirtualFS};

pub struct AppState {
    pub fs: Arc<RwLock<VirtualFS>>,
    pub validator: CertValidator,
    pub discovery: Arc<Mutex<Option<DiscoveryRuntime>>>,
    pub config_dir: PathBuf,
}

pub fn print_device_table(devices: &[DiscoveredDevice]) {
    let table = devices
        .iter()
        .enumerate()
        .map(|(i, dev)| {
            let main_ip = dev.ips.first().map(|ip| ip.to_string()).unwrap_or_default();
            let last_seen = humantime::format_rfc3339_seconds(dev.last_seen);
            vec![
                (i + 1).to_string(),
                dev.alias.clone(),
                dev.device_model.clone(),
                dev.device_type.to_string(),
                dev.fingerprint[..8].to_string(),
                main_ip,
                last_seen.to_string(),
            ]
        })
        .collect::<Vec<_>>();

    let headers = vec![
        "#".to_string(),
        "Alias".to_string(),
        "Model".to_string(),
        "Type".to_string(),
        "Fingerprint".to_string(),
        "IP".to_string(),
        "Last Seen".to_string(),
    ];
    print_table(headers, table);
}

pub fn print_device_details(dev: &DiscoveredDevice) {
    println!("┌{:─<30}┐", " Device Details ");
    println!("│{:<15}: {}│", "Alias", dev.alias);
    println!("│{:<15}: {}│", "Device Model", dev.device_model);
    println!("│{:<15}: {}│", "Device Type", dev.device_type);
    println!("│{:<15}: {}│", "Fingerprint", dev.fingerprint);
    println!(
        "│{:<15}: {}│",
        "Last Seen",
        humantime::format_rfc3339_seconds(dev.last_seen)
    );
    println!("├{:─<30}┤", " Network Addresses ");
    for ip in &dev.ips {
        println!("│ - {}│", ip);
    }
    println!("└{:─<30}┘", "");
}

pub fn print_table(headers: Vec<String>, rows: Vec<Vec<String>>) {
    if headers.is_empty() {
        return;
    }

    let column_widths: Vec<usize> = headers
        .iter()
        .enumerate()
        .map(|(i, header)| {
            let max_row_width = rows.iter().map(|row| row[i].len()).max().unwrap_or(0);
            std::cmp::max(header.len(), max_row_width)
        })
        .collect();

    let top_border = format!(
        "┌{}┐",
        column_widths
            .iter()
            .map(|&w| "─".repeat(w))
            .collect::<Vec<_>>()
            .join("┬")
    );

    let header_line = format!(
        "│{}│",
        headers
            .iter()
            .enumerate()
            .map(|(i, h)| format!("{:<width$}", h, width = column_widths[i]))
            .collect::<Vec<_>>()
            .join("│")
    );

    let mid_border = format!(
        "├{}┤",
        column_widths
            .iter()
            .map(|&w| "─".repeat(w))
            .collect::<Vec<_>>()
            .join("┼")
    );

    let data_lines: Vec<String> = rows
        .iter()
        .map(|row| {
            format!(
                "│{}│",
                row.iter()
                    .enumerate()
                    .map(|(i, cell)| format!("{:<width$}", cell, width = column_widths[i]))
                    .collect::<Vec<_>>()
                    .join("│")
            )
        })
        .collect();

    let bottom_border = format!(
        "└{}┘",
        column_widths
            .iter()
            .map(|&w| "─".repeat(w))
            .collect::<Vec<_>>()
            .join("┴")
    );

    println!("{}", top_border);
    println!("{}", header_line);
    if !rows.is_empty() {
        println!("{}", mid_border);
        for line in data_lines {
            println!("{}", line);
        }
    }
    println!("{}", bottom_border);
}
