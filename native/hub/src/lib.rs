mod request;

mod analyze;
mod apple_bridge;
mod collection;
mod connection;
mod cover_art;
mod directory;
mod library_home;
mod library_manage;
mod license;
mod logging;
mod lyric;
mod media_file;
pub mod messages;
mod mix;
mod playback;
pub mod player;
mod playlist;
mod scrobble;
mod search;
mod sfx;
mod stat;
mod system;
pub mod utils;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use log::{error, info};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::fmt;
use tracing_subscriber::EnvFilter;

pub use tokio;

use ::database::connection::{MainDbConnection, RecommendationDbConnection};
use ::playback::player::Player;
use ::playback::sfx_player::SfxPlayer;
use ::scrobbling::manager::ScrobblingManager;

use utils::receive_media_library_path;
use utils::Broadcaster;
use utils::DatabaseConnections;
use utils::TaskTokens;

use crate::messages::*;
use crate::player::initialize_player;
use crate::utils::init_logging;
use crate::utils::GlobalParams;
use crate::utils::ParamsExtractor;

pub trait Signal: Sized {
    type Params;
    type Response;
    fn handle(
        &self,
        params: Self::Params,
        dart_signal: &Self,
    ) -> impl Future<Output = Result<Option<Self::Response>>> + Send;
}

async fn player_loop(
    path: String,
    db_connections: DatabaseConnections,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
    broadcaster: Arc<dyn Broadcaster>,
) {
    info!("Media Library Received, initialize other receivers");

    tokio::spawn(async move {
        info!("Initializing database");

        let main_db: Arc<MainDbConnection> = db_connections.main_db;

        let recommend_db: Arc<RecommendationDbConnection> = db_connections.recommend_db;

        let lib_path: Arc<String> = Arc::new(path);

        let main_cancel_token = CancellationToken::new();
        let task_tokens: Arc<Mutex<TaskTokens>> = Arc::new(Mutex::new(TaskTokens::default()));

        info!("Initializing player");
        let player = Player::new(Some(main_cancel_token.clone()));
        let player: Arc<Mutex<Player>> = Arc::new(Mutex::new(player));

        let sfx_player = SfxPlayer::new(Some(main_cancel_token.clone()));
        let sfx_player: Arc<Mutex<SfxPlayer>> = Arc::new(Mutex::new(sfx_player));

        let main_cancel_token = Arc::new(main_cancel_token);

        info!("Initializing Player events");
        tokio::spawn(initialize_player(
            lib_path.clone(),
            main_db.clone(),
            player.clone(),
            scrobbler.clone(),
            broadcaster.clone(),
        ));

        info!("Initializing UI events");

        let global_params = Arc::new(GlobalParams {
            lib_path,
            main_db,
            recommend_db,
            main_token: main_cancel_token,
            task_tokens,
            player,
            sfx_player,
            scrobbler,
            broadcaster,
        });

        for_all_requests!(listen_local_gui_event, global_params);
    });
}

rinf::write_interface!();

#[tokio::main(flavor = "current_thread")]
async fn main() {
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
    if let Err(e) = receive_media_library_path(player_loop, scrobbler).await {
        error!("Failed to receive media library path: {}", e);
    }

    rinf::dart_shutdown().await;

    if let Some(guard) = _guard {
        drop(guard);
    }
}
