use std::sync::Arc;

use anyhow::{anyhow, Result};
use clap::Parser;
use log::error;
use regex::Regex;
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

mod cli;
mod commands;
mod editor;
mod fs;
mod hints;

use cli::Command;
use editor::{create_editor, EditorConfig};
use fs::VirtualFS;

/// Program arguments
#[derive(Parser)]
struct Args {
    /// Service URL
    #[arg(help = "The URL of the service, e.g., example.com:1234 or 192.168.1.1:1234")]
    service_url: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup_logging()?;

    // Parse command line arguments
    let args = Args::parse();

    // Validate and format the service URL
    let service_url = match validate_and_format_url(&args.service_url) {
        Ok(x) => x,
        Err(e) => {
            error!("{}", e);
            return Ok(());
        }
    };

    
    let config = EditorConfig::default();
    let fs = Arc::new(RwLock::new(VirtualFS::new()));
    let mut editor = create_editor(config, fs.clone())?;
    
    println!("Welcome to the Rune Speaker Command Line Interface");
    println!("Service URL: {}", service_url);
    println!();
    println!("Type 'help' to see available commands");

    loop {
        let current_dir = {
            let fs = fs.read().await;
            fs.current_dir()
        };
        let prompt = format!("{}> ", current_dir);

        if let Some(helper) = editor.helper_mut() {
            helper.set_colored_prompt(prompt.clone());
        }

        match editor.readline(&prompt) {
            Ok(line) => {
                let command = Command::parse(&line);
                match command {
                    Ok(cmd) => {
                        if !commands::execute(cmd, &fs).await? {
                            break;
                        }
                    }
                    Err(err) => {
                        if !err.use_stderr() {
                            println!("{}", err);
                        } else {
                            eprintln!("Error: {}", err);
                        }
                    }
                }
            }
            Err(err) => match err {
                rustyline::error::ReadlineError::Interrupted => break,
                rustyline::error::ReadlineError::Eof => {
                    println!("Encountered Eof");
                    break;
                }
                _ => {
                    eprintln!("Error: {:?}", err);
                    break;
                }
            },
        }
    }

    Ok(())
}

fn setup_logging() -> Result<(), Box<dyn std::error::Error>> {
    let filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,symphonia_bundle_mp3::demuxer=off,\
         tantivy::directory=off,tantivy::indexer=off,sea_orm_migration::migrator=off,info",
    );

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .init();

    Ok(())
}

fn validate_and_format_url(input: &str) -> Result<String> {
    let re = Regex::new(r"^(?P<host>[^:]+)(:(?P<port>\d+))?$").unwrap();

    if let Some(caps) = re.captures(input) {
        let host = caps.name("host").unwrap().as_str();
        let port = caps.name("port").map_or("7863", |m| m.as_str());

        // Validate host as a domain or IP address
        if !is_valid_host(host) {
            return Err(anyhow!(
                "Invalid host: must be a valid domain or IP address"
            ));
        }

        let formatted_url = format!("{}:{}/ws", host, port);
        Ok(formatted_url)
    } else {
        Err(anyhow!("Invalid URL format"))
    }
}

fn is_valid_host(host: &str) -> bool {
    // Simple validation for domain or IP
    let domain_re = Regex::new(r"^([a-zA-Z0-9-]+\.)+[a-zA-Z]{2,}$").unwrap();
    let ip_re = Regex::new(r"^\d{1,3}(\.\d{1,3}){3}$").unwrap();

    domain_re.is_match(host) || ip_re.is_match(host)
}
