use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
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
    #[arg(help = "The URL of the service")]
    service_url: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup_logging()?;

    // Parse command line arguments
    let args = Args::parse();

    // Use the service URL from arguments
    let service_url = args.service_url;

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
