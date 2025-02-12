pub mod api;
pub mod cli;
pub mod connection;
pub mod discovery;
pub mod editor;
pub mod fs;
pub mod hints;
pub mod repl;
pub mod utils;

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use log::error;
use regex::Regex;
use tokio::{
    signal::ctrl_c,
    sync::{Mutex, RwLock},
};
use tracing_subscriber::EnvFilter;

use hub::server::utils::path::init_system_paths;

use ::discovery::verifier::CertValidator;

use cli::{Cli, DiscoveryCmd, RemoteCmd, ReplCommand};
use connection::WSConnection;
use discovery::DiscoveryRuntime;
use editor::{create_editor, EditorConfig};
use fs::VirtualFS;
use utils::{print_device_details, print_device_table, AppState};

#[tokio::main]
async fn main() -> Result<()> {
    setup_logging()?;

    let cli = Cli::parse();

    match cli {
        Cli::Repl(args) => repl_mode(&args.service_url).await,
        Cli::Discovery(cmd) => handle_discovery_command(cmd).await,
        Cli::Remote(cmd) => handle_remote_command(cmd).await,
    }
}

async fn repl_mode(service_url: &str) -> Result<()> {
    let service_url = match validate_and_format_url(service_url) {
        Ok(x) => x,
        Err(e) => {
            error!("{}", e);
            return Ok(());
        }
    };

    println!("Welcome to the Rune Speaker Command Line Interface");
    println!("Service URL: {}", service_url);
    println!("\nType 'help' to see available commands");

    let config = EditorConfig::default();
    let connection = match WSConnection::connect(service_url.clone()).await {
        Ok(x) => x,
        Err(e) => {
            error!("{}", e);
            return Ok(());
        }
    };
    let connection = Arc::new(connection);
    let fs = Arc::new(RwLock::new(VirtualFS::new(connection)));
    let mut editor = create_editor(config, fs.clone())?;

    let config_dir = init_system_paths()?;
    let state: Arc<AppState> = Arc::new(AppState {
        fs: fs.clone(),
        validator: CertValidator::new(config_dir.join("certs"))?,
        discovery: Arc::new(Mutex::new(None)),
        config_dir: config_dir.clone(),
    });

    loop {
        let state = state.clone();
        let current_dir = {
            let fs_read_guard = fs.read().await;
            fs_read_guard.current_dir().to_owned()
        };

        let prompt = format!("{}> ", current_dir.to_string_lossy());

        if let Some(helper) = editor.helper_mut() {
            helper.set_colored_prompt(prompt.clone());
        }

        match editor.readline(&prompt) {
            Ok(line) => {
                let command = ReplCommand::parse(&line);
                match command {
                    Ok(cmd) => {
                        if !handle_repl_command(cmd, state).await? {
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
                rustyline::error::ReadlineError::Interrupted => continue,
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

async fn handle_repl_command(command: ReplCommand, state: Arc<AppState>) -> Result<bool> {
    use ReplCommand::*;

    match command {
        Ls { long } => repl::handle_ls(state, long).await,
        Pwd => repl::handle_pwd(state).await,
        Cd { path, id } => repl::handle_cd(state, path, id).await,
        Opq {
            path,
            playback_mode,
            instant_play,
            operate_mode,
            id,
        } => repl::handle_opq(state, path, playback_mode, instant_play, operate_mode, id).await,
        Play => repl::handle_play(state).await,
        Pause => repl::handle_pause(state).await,
        Next => repl::handle_next(state).await,
        Previous => repl::handle_previous(state).await,
        SetMode { mode } => repl::handle_setmode(state, mode).await,
        Quit => Ok(false),
        Exit => Ok(false),
        // Handle aliases (should never reach here due to parse conversion)
        Cdi { .. } | Opqi { .. } => unreachable!(),
    }
}

async fn handle_discovery_command(cmd: DiscoveryCmd) -> Result<()> {
    let config_dir = init_system_paths()?;

    match cmd {
        DiscoveryCmd::Scan => {
            let mut rt = DiscoveryRuntime::new(&config_dir).await?;

            // Start discovery services
            rt.start_listening().await?;

            // Executing device scanning
            ctrl_c().await?;

            // Terminate services and persist
            rt.shutdown().await?;
            let final_devices = rt.store.get_devices().await;
            rt.store.save(&final_devices).await?;
            print_device_table(&final_devices);
        }
        DiscoveryCmd::List => {
            let rt = DiscoveryRuntime::new(&config_dir).await?;
            print_device_table(&rt.store.get_devices().await);
        }
    }

    Ok(())
}

async fn handle_remote_command(cmd: RemoteCmd) -> Result<()> {
    let config_dir = init_system_paths()?;
    let validator = CertValidator::new(config_dir.clone())?;

    match cmd {
        RemoteCmd::Inspect { index } => {
            let rt = DiscoveryRuntime::new(&config_dir).await?;
            let devices = rt.store.load().await?;
            let dev = devices.get(index - 1).context("Invalid index")?;
            print_device_details(dev);
            Ok(())
        }
        RemoteCmd::Trust { index, domains } => {
            let rt = DiscoveryRuntime::new(&config_dir).await?;
            let devices = rt.store.load().await?;
            let dev = devices.get(index - 1).context("Invalid index")?;
            let hosts = domains
                .map(|d| {
                    d.split(',')
                        .map(|s| s.trim().to_string())
                        .collect::<Vec<String>>()
                })
                .unwrap_or_else(|| dev.ips.iter().map(|ip| ip.to_string()).collect());
            validator.add_trusted_domains(hosts, &dev.fingerprint)?;
            Ok(())
        }
        RemoteCmd::Untrust { fingerprint } => {
            validator.remove_fingerprint(&fingerprint)?;
            Ok(())
        }
        RemoteCmd::Edit { fingerprint, hosts } => {
            let new_hosts: Vec<_> = hosts.split(',').map(|s| s.trim().to_string()).collect();
            validator.replace_hosts_for_fingerprint(&fingerprint, new_hosts)?;
            Ok(())
        }
    }
}

fn setup_logging() -> Result<()> {
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

        if !is_valid_host(host) {
            return Err(anyhow!(
                "Invalid host: must be a valid domain or IP address"
            ));
        }

        Ok(format!("ws://{}:{}/ws", host, port))
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
