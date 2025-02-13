use std::fmt;
use std::time::Duration;

use anyhow::Result;
use futures::StreamExt;

use discovery::verifier::fetch_server_certificate;
use log::{error, info};

#[derive(Debug)]
enum VerificationResult {
    Match,
    Mismatch,
    Error(String),
}

impl fmt::Display for VerificationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerificationResult::Match => write!(f, "✅ Match"),
            VerificationResult::Mismatch => write!(f, "❌ Mismatch"),
            VerificationResult::Error(e) => write!(f, "⚠️ Error: {}", e),
        }
    }
}

async fn verify_single_host(host: String, expected_fp: String) -> (String, VerificationResult) {
    let url = format!("https://{}:7863/ping", host);
    info!("Connecting to {}", url);
    let result =
        match tokio::time::timeout(Duration::from_secs(5), fetch_server_certificate(&url)).await {
            Ok(Ok(cert)) => {
                if cert.fingerprint == expected_fp {
                    VerificationResult::Match
                } else {
                    VerificationResult::Mismatch
                }
            }
            Ok(Err(e)) => VerificationResult::Error(e.to_string()),
            Err(_) => VerificationResult::Error("Timeout".into()),
        };

    (host, result)
}

pub async fn verify_servers(expected_fingerprint: &str, hosts: Vec<String>) -> Result<()> {
    let tasks = hosts.into_iter().map(|host| {
        let expected = expected_fingerprint.to_string();
        let host_clone = host.clone();

        tokio::spawn(async move { verify_single_host(host_clone, expected).await })
    });

    let mut success = 0;
    let mut mismatch = 0;
    let mut errors = 0;

    let results = futures::stream::iter(tasks)
        .buffer_unordered(10)
        .collect::<Vec<_>>()
        .await;

    for result in results {
        match result {
            Ok((host, VerificationResult::Match)) => {
                success += 1;
                info!("Host match: {}", host)
            }
            Ok((host, VerificationResult::Mismatch)) => {
                mismatch += 1;
                error!("Host mismatch: {}", host)
            }
            Ok((host, VerificationResult::Error(e))) => {
                errors += 1;
                error!("Unable to verify the host: {} ({})", host, e)
            }
            Err(_) => errors += 1,
        }
    }

    println!("\nVerification Summary:");
    println!("====================");
    println!("Total hosts checked: {}", success + mismatch + errors);
    println!("Matching: {}", success);
    println!("Mismatched: {}", mismatch);
    println!("Errors: {}", errors);

    Ok(())
}
