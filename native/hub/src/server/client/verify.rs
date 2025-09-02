use std::{fmt, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use colored::Colorize;
use futures::StreamExt;
use log::info;
use rustls::ClientConfig;

use ::discovery::{
    client::{fetch_server_certificate, parse_certificate},
    config::get_config_dir,
};

use hub::server::{
    api::{fetch_device_info, register_device},
    generate_or_load_certificates, get_or_generate_alias,
};

#[derive(Debug)]
enum VerificationResult {
    Match,
    Mismatch(String),
    Error(String),
}

impl fmt::Display for VerificationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerificationResult::Match => write!(f, "Match"),
            VerificationResult::Mismatch(_) => write!(f, "Mismatch"),
            VerificationResult::Error(e) => write!(f, "Error: {e}"),
        }
    }
}

async fn verify_single_host(host: String, expected_fp: String) -> (String, VerificationResult) {
    let url = format!("https://{host}:7863/ping");
    info!("Connecting to {url}");

    let result =
        match tokio::time::timeout(Duration::from_secs(5), fetch_server_certificate(&url)).await {
            Ok(Ok(cert)) => {
                if cert.fingerprint == expected_fp {
                    VerificationResult::Match
                } else {
                    VerificationResult::Mismatch(cert.fingerprint)
                }
            }
            Ok(Err(e)) => VerificationResult::Error(e.to_string()),
            Err(_) => VerificationResult::Error("Timeout after 5s".into()),
        };

    (host, result)
}

pub async fn verify_servers(expected_fingerprint: &str, hosts: Vec<String>) -> Result<()> {
    let tasks = hosts.into_iter().map(|host| {
        let expected = expected_fingerprint.to_string();
        tokio::spawn(async move { verify_single_host(host.clone(), expected).await })
    });

    let mut success = 0;
    let mut mismatch = 0;
    let mut errors = 0;

    let results = futures::stream::iter(tasks)
        .buffer_unordered(10)
        .collect::<Vec<_>>()
        .await;

    println!("{}", "Verification Report".bold().yellow());
    for result in results {
        match result {
            Ok((host, VerificationResult::Match)) => {
                success += 1;
                println!("  {}", host.bold());
                println!("    Status:      {}", "MATCH".green().bold());
                println!(
                    "    Fingerprint: {}\n",
                    expected_fingerprint.to_string().magenta()
                );
            }
            Ok((host, VerificationResult::Mismatch(actual))) => {
                mismatch += 1;
                println!("   {}", host.bold());
                println!("    Status:      {}", "MISMATCH".red().bold());
                println!("    Expected:    {}", expected_fingerprint.green());
                println!("    Actual:      {}\n", actual.red());
            }
            Ok((host, VerificationResult::Error(e))) => {
                errors += 1;
                println!("   {}", host.bold());
                println!("    Status:      {}", "ERROR".yellow().bold());
                println!("    Reason:      {}\n", e.red());
            }
            Err(e) => {
                errors += 1;
                println!("   {}", "Task Failed".bold());
                println!("    Reason:      {}\n", e.to_string().red());
            }
        }
    }

    println!("{}", "Summary".bold().yellow());
    println!(
        "  {} {}",
        "Total Hosts:".cyan().bold(),
        (success + mismatch + errors).to_string().cyan()
    );
    println!(
        "  {} {}",
        "Matching:   ".green().bold(),
        success.to_string().green()
    );
    println!(
        "  {} {}",
        "Mismatched: ".red().bold(),
        mismatch.to_string().red()
    );
    println!(
        "  {} {}",
        "Errors:     ".red().bold(),
        errors.to_string().red()
    );

    Ok(())
}

pub async fn inspect_host(host: &str, config: Arc<ClientConfig>) -> Result<()> {
    let device_info = fetch_device_info(host, config).await?;

    println!("{}", format!("Device Info for {host}").bold().yellow());
    println!("  {:15}: {}", "Alias", device_info.alias);
    println!("  {:15}: {}", "Version", device_info.version);
    println!("  {:15}: {}", "Device Type", device_info.device_type);
    if let Some(model) = &device_info.device_model {
        println!("  {:15}: {}", "Device Model", model);
    }

    Ok(())
}

pub async fn register_current_device(host: &str, config: Arc<ClientConfig>) -> Result<()> {
    let config_dir = get_config_dir()?;
    let certificate_id = get_or_generate_alias(config_dir).await?;
    let (_, certificate, _) = generate_or_load_certificates(&config_dir, &certificate_id)
        .await
        .context("Failed to load client certificates")?;

    let (public_key, fingerprint) =
        parse_certificate(&certificate).context("Failed to parse client certificate")?;

    register_device(
        host,
        config,
        public_key,
        fingerprint,
        certificate_id,
        "RuneAudio".to_owned(),
        "Headless".to_owned(),
    )
    .await?;

    Ok(())
}

pub async fn print_device_information() -> Result<()> {
    let config_dir = get_config_dir()?;
    let certificate_id = get_or_generate_alias(config_dir).await?;
    let (_, certificate, _) = generate_or_load_certificates(&config_dir, &certificate_id)
        .await
        .context("Failed to load client certificates")?;

    let (public_key, fingerprint) =
        parse_certificate(&certificate).context("Failed to parse client certificate")?;

    println!("{}", "Client Information".bold().cyan());
    println!("  {:15}: {}", "Fingerprint", fingerprint.cyan());
    println!("  {:15}: {}", "Alias", certificate_id.cyan());
    println!("  {:15}: {}", "Device Type", "Headless".cyan());
    println!("  {:15}: {}", "Device Model", "RuneAudio".cyan());
    println!("{public_key}");

    Ok(())
}
