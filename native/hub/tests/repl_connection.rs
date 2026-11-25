//! Runs the real `rune-client` REPL against the real `rune-server` binary.
//!
//! The client and server keep their configuration in temporary homes, so the
//! test also exercises the certificate, registration and approval files the
//! CLI and server create for themselves.
#![cfg(unix)]

use std::{
    path::{Path, PathBuf},
    process::{Output, Stdio},
    time::Duration,
};

use anyhow::{Context, Result};
use discovery::client::{CertValidator, fetch_server_certificate, parse_certificate};
use hub::utils::sync_settings::SyncSettings;
use serde_json::Value;
use tempfile::TempDir;
use tokio::{io::AsyncWriteExt, process::Command, time::timeout};
use uuid::Uuid;

const TEST_DEADLINE: Duration = Duration::from_secs(60);

/// A binary Cargo built for this package.
fn binary(name: &str) -> PathBuf {
    std::env::var_os(format!("CARGO_BIN_EXE_{name}"))
        .map(PathBuf::from)
        .or_else(|| {
            let mut path = std::env::current_exe().ok()?;
            path.pop();
            if path.ends_with("deps") {
                path.pop();
            }
            path.push(name);
            Some(path)
        })
        .unwrap_or_else(|| panic!("{name} binary path"))
}

/// The configuration directory `get_config_dir` selects under `home`.
fn config_dir(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library/Application Support/ci.not.rune")
    }
    #[cfg(not(target_os = "macos"))]
    {
        home.join("config/rune")
    }
}

fn config_home(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library/Application Support")
    }
    #[cfg(not(target_os = "macos"))]
    {
        home.join("config")
    }
}

async fn run_client(home: &Path, args: &[&str]) -> Result<Output> {
    let output = timeout(
        TEST_DEADLINE,
        Command::new(binary("rune-client"))
            .args(args)
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", config_home(home))
            .current_dir(home)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("rune-client did not exit")??;
    Ok(output)
}

async fn run_server_command(home: &Path, args: &[&str]) -> Result<Output> {
    let output = timeout(
        TEST_DEADLINE,
        Command::new(binary("rune-server"))
            .args(args)
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", config_home(home))
            .current_dir(home)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("rune-server did not exit")??;
    Ok(output)
}

/// Runs the REPL, asks it for its root listing, and leaves it.
async fn run_repl(home: &Path, port: u16) -> Result<Output> {
    let address = format!("127.0.0.1:{port}");
    let library_id = Uuid::new_v4().to_string();
    let mut child = Command::new(binary("rune-client"))
        .args(["repl", &address, "--library-id", &library_id])
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", config_home(home))
        .current_dir(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("spawn rune-client repl")?;
    let mut stdin = child.stdin.take().context("client stdin")?;
    stdin.write_all(b"ls\nquit\n").await?;
    drop(stdin);
    let output = timeout(TEST_DEADLINE, child.wait_with_output())
        .await
        .context("rune-client repl did not exit")??;
    Ok(output)
}

struct ServerProcess {
    child: tokio::process::Child,
    log: PathBuf,
}

impl ServerProcess {
    async fn stop(mut self) -> Result<()> {
        self.child.kill().await?;
        self.child.wait().await?;
        Ok(())
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

async fn start_server(home: &Path, library: &Path, port: u16) -> Result<ServerProcess> {
    let log_path = home.join("server.log");
    let log = std::fs::File::create(&log_path)?;
    let address = format!("127.0.0.1:{port}");
    let library = library.to_str().context("library path")?;
    let args = ["server", "--addr", address.as_str(), library];
    let mut command = Command::new(binary("rune-server"));
    command
        .args(args)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", config_home(home))
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    let child = command.spawn().context("spawn rune-server")?;
    Ok(ServerProcess {
        child,
        log: log_path,
    })
}

/// Waits until the server answers and returns the certificate it presents.
async fn wait_for_server(port: u16) -> Result<String> {
    let url = format!("https://127.0.0.1:{port}/ping");
    timeout(TEST_DEADLINE, async {
        loop {
            if let Ok(cert) = fetch_server_certificate(&url).await {
                return Ok(cert.fingerprint);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("rune-server did not answer")?
}

/// Waits until the server itself reports the client's status.
async fn wait_for_status(port: u16, fingerprint: &str, expected: &str) -> Result<()> {
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()?;
    let mut url = reqwest::Url::parse(&format!("https://127.0.0.1:{port}/check-fingerprint"))?;
    url.query_pairs_mut()
        .append_pair("fingerprint", fingerprint);
    timeout(TEST_DEADLINE, async {
        loop {
            if let Ok(response) = client.get(url.clone()).send().await
                && let Ok(body) = response.json::<Value>().await
                && body["status"] == expected
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("server did not report the expected client status")?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn repl_connects_with_trust_authorization_and_reports_failures() -> Result<()> {
    let server_home = TempDir::new()?;
    let server_config = config_dir(server_home.path());
    std::fs::create_dir_all(&server_config)?;
    SyncSettings {
        listen: false,
        ..Default::default()
    }
    .save(&server_config)?;
    let library = server_home.path().join("library");
    std::fs::create_dir_all(&library)?;
    let port = portpicker::pick_unused_port().context("unused port")?;
    let server = start_server(server_home.path(), &library, port).await?;
    let server_fingerprint = wait_for_server(port)
        .await
        .with_context(|| format!("server did not start:\n{}", server.log()))?;

    let client_home = TempDir::new()?;
    let client_config = config_dir(client_home.path());
    std::fs::create_dir_all(&client_config)?;
    let self_info = run_client(client_home.path(), &["remote", "self-info"]).await?;
    assert!(
        self_info.status.success(),
        "self-info failed:\n{}",
        String::from_utf8_lossy(&self_info.stderr)
    );
    let certificate = std::fs::read_to_string(client_config.join("certificate.pem"))?;
    let (_, client_fingerprint) = parse_certificate(&certificate)?;
    let validator = CertValidator::new(&client_config).await?;
    validator
        .add_trusted_domains([format!("127.0.0.1:{port}")], &server_fingerprint)
        .await?;

    let register = run_client(
        client_home.path(),
        &["remote", "register", &format!("127.0.0.1:{port}")],
    )
    .await?;
    assert!(
        register.status.success(),
        "register failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&register.stdout),
        String::from_utf8_lossy(&register.stderr)
    );
    let approve = run_server_command(
        server_home.path(),
        &["permission", "modify", "1", "approved"],
    )
    .await?;
    assert!(
        approve.status.success(),
        "approval failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&approve.stdout),
        String::from_utf8_lossy(&approve.stderr)
    );
    wait_for_status(port, &client_fingerprint, "APPROVED").await?;

    let approved = run_repl(client_home.path(), port).await?;
    let stdout = String::from_utf8_lossy(&approved.stdout);
    let stderr = String::from_utf8_lossy(&approved.stderr);
    assert!(
        approved.status.success(),
        "the approved REPL failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("Artists") && stdout.contains("Tracks"),
        "the REPL did not reach its root listing after connecting:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let pending = run_server_command(
        server_home.path(),
        &["permission", "modify", "1", "pending"],
    )
    .await?;
    assert!(
        pending.status.success(),
        "pending change failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&pending.stdout),
        String::from_utf8_lossy(&pending.stderr)
    );
    wait_for_status(port, &client_fingerprint, "PENDING").await?;

    let refused = run_repl(client_home.path(), port).await?;
    let stdout = String::from_utf8_lossy(&refused.stdout);
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success(),
        "an unapproved REPL connection must fail:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("not authorized"),
        "the unapproved client failed for another reason:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    server.stop().await?;
    Ok(())
}
