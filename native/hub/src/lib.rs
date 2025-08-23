mod apple_bridge;
#[macro_use]
mod macros;
mod handlers;
#[macro_use]
pub mod backends;
pub mod messages;
#[macro_use]
pub mod server;
pub mod utils;

use std::{future::Future, sync::Arc, time::Duration};

use anyhow::Result;
use log::{error, info};
use rustls::crypto::ring::default_provider;
use tokio::sync::Mutex;
use tracing_subscriber::{EnvFilter, fmt};

pub use tokio;

use ::scrobbling::manager::ScrobblingManager;

use utils::{TaskTokens, receive_media_library_path};

use crate::utils::init_logging;

pub struct Session {
    pub fingerprint: String,
    pub host: String,
}

pub trait Signal: Sized {
    type Params;
    type Response;
    fn handle(
        &self,
        params: Self::Params,
        session: Option<Session>,
        dart_signal: &Self,
    ) -> impl Future<Output = Result<Option<Self::Response>>> + Send;
}

rinf::write_interface!();

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(e) = default_provider().install_default() {
        panic!("{e:#?}");
    };

    let args: Vec<String> = std::env::args().collect();
    let enable_log = args.contains(&"--enable-log".to_string());

    let scrobbler = ScrobblingManager::new(10, Duration::new(5, 0));
    let scrobbler = Arc::new(Mutex::new(scrobbler));

    let _guard = if enable_log {
        let file_filter = EnvFilter::new("debug");
        let now = chrono::Local::now();
        let file_name = format!("{}.rune.log", now.format("%Y-%m-%d_%H-%M-%S"));
        let file_appender = tracing_appender::rolling::never(".", file_name);
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

        tracing_subscriber::fmt()
            .with_env_filter(file_filter)
            .with_writer(non_blocking)
            .with_timer(fmt::time::ChronoLocal::rfc_3339())
            .init();

        info!("Logging is enabled");
        Some(guard)
    } else {
        init_logging();
        None
    };

    // Start receiving the media library path
    if let Err(e) = receive_media_library_path(scrobbler).await {
        error!("Failed to receive media library path: {e:?}");
    }

    rinf::dart_shutdown().await;

    if let Some(guard) = _guard {
        drop(guard);
    }
}
