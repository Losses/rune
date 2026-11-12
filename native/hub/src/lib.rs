#![recursion_limit = "256"]

use fmt::time::ChronoLocal;
pub mod access_policy;
mod apple_bridge;
pub mod flutter_bridge;
#[macro_use]
mod macros;
mod handlers;
#[macro_use]
pub mod backends;
pub mod library_pool;
pub mod messages;
pub mod node;
pub mod request_error;
mod request_owner;
pub mod rpc;
pub mod session;
mod subsonic_binding;
mod subsonic_translate;
#[cfg(test)]
mod test_wait;
#[macro_use]
pub mod server;
pub mod utils;
pub mod visualizer;
mod visualizer_log;

use std::{future::Future, sync::Arc, time::Duration};

use anyhow::Result;
use log::{error, info};
use rustls::crypto::ring::default_provider;
use tokio::sync::Mutex;
use tracing_subscriber::{EnvFilter, fmt};

pub use tokio;
pub use utils::{RemoteEventDeclaration, RemoteEventScope, RinfRustSignal};

use ::scrobbling::manager::ScrobblingManager;

use macros::{receive_node_requests, start_standalone_request_receivers};

use crate::utils::init_logging;

pub struct Session {
    pub public_key: String,
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

pub trait RequestCallId {
    fn call_id(&self) -> Option<u64>;
}

pub trait ResponseCallId {
    fn set_call_id(&mut self, call_id: Option<u64>);
}

impl ResponseCallId for () {
    fn set_call_id(&mut self, _call_id: Option<u64>) {}
}

/// A request whose wire name and response type are declared in the request catalogue.
pub trait TypedRequest {
    type Response;

    fn wire_name() -> &'static str;
    fn response_name() -> &'static str;
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
            .with_timer(ChronoLocal::rfc_3339())
            .init();

        info!("Logging is enabled");
        Some(guard)
    } else {
        init_logging();
        None
    };

    start_standalone_request_receivers();
    tokio::spawn(receive_node_requests(
        node::node_slot(),
        utils::ProductionNodeRequestHandlers::new(Arc::clone(&scrobbler)),
    ));

    // Every receiver runs as a spawned task on this runtime, so main must stay alive until
    // Dart shuts down; returning earlier drops the runtime and every receiver with it.
    rinf::dart_shutdown().await;

    if let Err(error) = node::shutdown_node().await {
        error!("Node shutdown failed: {error:#}");
    }

    if let Some(guard) = _guard {
        drop(guard);
    }
}
