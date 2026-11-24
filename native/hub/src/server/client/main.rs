use anyhow::Error;
use rustyline::error::ReadlineError;
pub mod api;
pub mod cli;
pub mod connection;
pub mod editor;
pub mod fs;
pub mod hints;
pub mod repl;
pub mod utils;
pub mod verify;

use std::{path::Path, process::exit, sync::Arc};

use anyhow::{Context, Result, bail};
use clap::Parser;
use log::{error, info};
use rustls::crypto::ring::default_provider;
use tokio::{
    signal::ctrl_c,
    sync::{Mutex, RwLock},
};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use ::discovery::{
    client::CertValidator,
    config::get_config_dir,
    endpoint::{DEFAULT_DEVICE_PORT, DeviceAddress, DeviceScheme},
    protocol::DiscoveryService,
    server::PermissionManager,
};

use cli::{Cli, DiscoveryCmd, RemoteCmd, ReplCommand};
use connection::WSConnection;
use editor::{EditorConfig, create_editor};
use fs::VirtualFS;
use hub::backends::remote::connect_controller;
use hub::messages::TrustedServerCertificate;
use hub::server::api::{add_trusted_server, edit_trusted_hosts, remove_trusted_device};
use utils::{
    AppState, get_fingerprint_by_index, print_certificate_table, print_device_details,
    print_device_table,
};
use verify::{inspect_host, print_device_information, register_current_device, verify_servers};

async fn trust_discovered_server(
    validator: &CertValidator,
    fingerprint: String,
    hosts: Vec<String>,
) -> Result<()> {
    add_trusted_server(
        validator,
        Some(&TrustedServerCertificate { fingerprint, hosts }),
    )
    .await
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_logging()?;

    if let Err(e) = default_provider().install_default() {
        bail!(format!("{e:#?}"));
    };

    let cli = Cli::parse();

    match cli {
        Cli::Repl(args) => {
            let config_dir = get_config_dir()?;
            repl_mode(&args.service_url, args.library_id, config_dir).await
        }
        Cli::Discovery(cmd) => handle_discovery_command(cmd).await,
        Cli::Remote(cmd) => handle_remote_command(cmd).await,
    }
}

async fn repl_mode(service_url: &str, library_id: Uuid, config_dir: &Path) -> Result<()> {
    let address: DeviceAddress = service_url.parse().map_err(Error::msg)?;
    let service_url = address.url(DeviceScheme::Wss, "/ws")?;

    println!("Welcome to the Rune Speaker Command Line Interface");
    println!("Service URL: {service_url}");
    println!("\nType 'help' to see available commands");

    let config = EditorConfig::default();
    let validator = CertValidator::new(config_dir).await?;
    let stream = connect_controller(
        address.host(),
        address.port(),
        config_dir
            .to_str()
            .context("Config directory is not valid UTF-8")?,
        validator.clone(),
    )
    .await?;
    let connection = WSConnection::connect(stream, library_id).await?;
    let connection = Arc::new(connection);
    let fs = Arc::new(RwLock::new(VirtualFS::new(connection)));
    let mut editor = create_editor(config, fs.clone())?;

    let state: Arc<AppState> = Arc::new(AppState {
        fs: fs.clone(),
        validator,
        discovery: Arc::new(Mutex::new(None)),
        config_dir: config_dir.to_path_buf(),
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
                            println!("{err}");
                        } else {
                            eprintln!("Error: {err}");
                        }
                    }
                }
            }
            Err(err) => match err {
                ReadlineError::Interrupted => continue,
                ReadlineError::Eof => {
                    println!("Encountered Eof");
                    break;
                }
                _ => {
                    eprintln!("Error: {err:?}");
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
    let config_dir = get_config_dir()?;

    match cmd {
        DiscoveryCmd::Scan => {
            let rt = DiscoveryService::with_store(config_dir).await?;

            // Start discovery services
            rt.start_listening(None).await?;

            // Executing device scanning
            ctrl_c().await?;

            // Terminate services and persist
            rt.shutdown().await?;

            let final_devices = rt.get_all_devices();
            print_device_table(&final_devices);
        }
        DiscoveryCmd::Ls => {
            let rt = DiscoveryService::with_store(config_dir).await?;
            print_device_table(&rt.get_all_devices());
        }
        DiscoveryCmd::Inspect { index } => {
            let rt = DiscoveryService::with_store(config_dir).await?;
            let devices = rt.get_all_devices();
            let dev = devices.get(index - 1).context("Invalid index")?;
            print_device_details(dev);
        }
        DiscoveryCmd::Trust { index, domains } => {
            let rt = DiscoveryService::with_store(config_dir.clone()).await?;
            let devices = rt.get_all_devices();
            let dev = devices.get(index - 1).context("Invalid index")?;
            let hosts = match domains {
                Some(domains) => domains
                    .split(',')
                    .map(|host| host.trim().to_string())
                    .collect(),
                None => dev
                    .ips
                    .iter()
                    .map(|ip| {
                        DeviceAddress::new(
                            ip.to_string(),
                            dev.api_port.unwrap_or(DEFAULT_DEVICE_PORT),
                        )
                        .map(|address| address.to_string())
                        .map_err(Error::from)
                    })
                    .collect::<Result<Vec<_>>>()?,
            };

            let validator = CertValidator::new(config_dir).await?;
            trust_discovered_server(&validator, dev.fingerprint.clone(), hosts.clone()).await?;

            info!(
                "Trusted fingerprint: {}, with hosts: {}",
                dev.fingerprint,
                hosts.join(", ")
            );
        }
    }

    Ok(())
}

async fn handle_remote_command(cmd: RemoteCmd) -> Result<()> {
    let config_dir = get_config_dir()?;
    let validator = Arc::new(CertValidator::new(config_dir.clone()).await?);

    match cmd {
        RemoteCmd::Ls => {
            print_certificate_table(&validator);
            Ok(())
        }
        RemoteCmd::Untrust { index } => {
            if let Some(actual_fingerprint) = get_fingerprint_by_index(&validator, index)? {
                let permissions = PermissionManager::new(config_dir.clone())?;
                remove_trusted_device(&permissions, &validator, &actual_fingerprint).await?;
                Ok(())
            } else {
                error!("Invalid certificate index");
                exit(1)
            }
        }
        RemoteCmd::Edit { fingerprint, hosts } => {
            let new_hosts: Vec<_> = hosts.split(',').map(|s| s.trim().to_string()).collect();
            edit_trusted_hosts(&validator, &fingerprint, new_hosts).await?;
            Ok(())
        }
        RemoteCmd::Verify { index } => {
            if let Some(fingerprint) = get_fingerprint_by_index(&validator, index)? {
                let hosts = validator.get_hosts_for_fingerprint(&fingerprint).await;

                if hosts.is_empty() {
                    info!("No hosts associated with fingerprint {fingerprint}");
                    return Ok(());
                }

                info!(
                    "Verifying {} hosts for fingerprint {}",
                    hosts.len(),
                    fingerprint
                );

                verify_servers(&fingerprint, hosts).await
            } else {
                error!("Invalid certificate index");
                exit(1)
            }
        }
        RemoteCmd::Inspect { host } => {
            let client_config = validator.into_client_config();
            inspect_host(&host, Arc::new(client_config)).await
        }

        RemoteCmd::Register { host } => {
            let client_config = validator.into_client_config();
            register_current_device(&host, Arc::new(client_config)).await
        }

        RemoteCmd::SelfInfo => print_device_information().await,
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

#[cfg(test)]
mod tests {
    use super::{repl_mode, trust_discovered_server};
    use ::discovery::client::CertValidator;
    use hub::backends::remote::RemoteConnectError;
    use uuid::Uuid;

    #[tokio::test]
    async fn repl_reports_a_connection_failure_to_its_caller() {
        let directory = tempfile::tempdir().unwrap();
        let error = repl_mode("127.0.0.1:1", Uuid::new_v4(), directory.path())
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<RemoteConnectError>(),
            Some(RemoteConnectError::UntrustedEndpoint)
        ));
    }

    #[tokio::test]
    async fn discovered_trust_does_not_store_an_unverified_fingerprint() {
        let directory = tempfile::tempdir().unwrap();
        let validator = CertValidator::new(directory.path()).await.unwrap();
        let result = trust_discovered_server(
            &validator,
            "advertised-fingerprint".to_owned(),
            vec!["127.0.0.1:1".to_owned()],
        )
        .await;
        assert!(result.is_err());
        assert!(validator.list_trusted_fingerprints().await.is_empty());
    }
}
