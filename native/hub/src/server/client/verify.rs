use std::fmt;
use std::time::Duration;

use anyhow::Result;
use colored::Colorize;
use futures::StreamExt;

use discovery::verifier::fetch_server_certificate;
use log::info;

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
            VerificationResult::Mismatch(_) => write!(f, "❌Mismatch"),
            VerificationResult::Error(e) => write!(f, "Error: {}", e),
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
