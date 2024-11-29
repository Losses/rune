mod analyze;
mod collection;
mod connection;
mod cover_art;
mod directory;
mod library_home;
mod library_manage;
mod license;
mod logging;
mod media_file;
mod messages;
mod mix;
mod playback;
mod player;
mod playlist;
mod search;
mod sfx;
mod stat;
mod system;
mod utils;

use std::sync::Arc;

use anyhow::Context;
use license::check_store_license;
use log::{debug, error, info};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::fmt;
use tracing_subscriber::EnvFilter;

pub use tokio;

use ::playback::player::Player;
use ::playback::sfx_player::SfxPlayer;

use crate::analyze::*;
use crate::collection::*;
use crate::connection::*;
use crate::cover_art::*;
use crate::directory::*;
use crate::library_home::*;
use crate::library_manage::*;
use crate::logging::*;
use crate::media_file::*;
use crate::messages::*;
use crate::mix::*;
use crate::playback::*;
use crate::player::initialize_player;
use crate::playlist::*;
use crate::search::*;
use crate::sfx::*;
use crate::stat::*;
use crate::system::*;
use crate::utils::init_logging;

macro_rules! select_signal {
    ($cancel_token:expr, $( $type:ty => ($($arg:ident),*) ),* $(,)? ) => {
        paste::paste! {
            $(
                let [<receiver_ $type:snake>] = <$type>::get_dart_signal_receiver();
            )*

            loop {
                if $cancel_token.is_cancelled() {
                    info!("Cancellation requested. Exiting main loop.");
                    break;
                }

                tokio::select! {
                    $(
                        dart_signal = [<receiver_ $type:snake>].recv() => {
                            $(let $arg = Arc::clone(&$arg);)*
                            if let Some(dart_signal) = dart_signal {
                                debug!("Processing signal: {}", stringify!($type));

                                if let Err(e) = [<$type:snake>]($($arg),*, dart_signal).await.with_context(|| format!("Processing signal: {}", stringify!($type))) {
                                    error!("{:?}", e);
                                    CrashResponse {
                                        detail: format!("{:#?}", e),
                                    }
                                    .send_signal_to_dart();
                                };
                            }
                        }
                    )*
                    else => continue,
                }
            }
        }
    };
}

#[derive(Default)]
struct TaskTokens {
    scan_token: Option<CancellationToken>,
    analyze_token: Option<CancellationToken>,
}

async fn player_loop(path: String, db_connections: DatabaseConnections) {
    info!("Media Library Received, initialize other receivers");

    tokio::spawn(async move {
        info!("Initializing database");

        let main_db: Arc<sea_orm::DatabaseConnection> = db_connections.main_db;

        let recommend_db: Arc<database::connection::RecommendationDbConnection> =
            db_connections.recommend_db;

        let lib_path = Arc::new(path);

        let main_cancel_token = CancellationToken::new();
        let task_tokens = Arc::new(Mutex::new(TaskTokens::default()));

        info!("Initializing player");
        let player = Player::new(Some(main_cancel_token.clone()));
        let player = Arc::new(Mutex::new(player));

        let sfx_player = SfxPlayer::new(Some(main_cancel_token.clone()));
        let sfx_player = Arc::new(Mutex::new(sfx_player));

        let main_cancel_token = Arc::new(main_cancel_token);

        info!("Initializing Player events");
        tokio::spawn(initialize_player(main_db.clone(), player.clone()));

        info!("Initializing UI events");
        select_signal!(
            main_cancel_token,

            TestLibraryInitializedRequest => (main_db),
            CloseLibraryRequest => (lib_path, main_cancel_token, task_tokens),
            ScanAudioLibraryRequest => (main_db, task_tokens),
            AnalyzeAudioLibraryRequest => (main_db, recommend_db, task_tokens),
            CancelTaskRequest => (task_tokens),

            LoadRequest => (player),
            PlayRequest => (player),
            PauseRequest => (player),
            NextRequest => (main_db, player),
            PreviousRequest => (main_db, player),
            SwitchRequest => (main_db, player),
            SeekRequest => (player),
            RemoveRequest => (player),
            VolumeRequest => (player),
            SetPlaybackModeRequest => (player),
            MovePlaylistItemRequest => (player),
            SetRealtimeFftEnabledRequest => (player),

            SfxPlayRequest => (sfx_player),

            IfAnalyzeExistsRequest => (main_db),
            GetAnalyzeCountRequest => (main_db),

            FetchMediaFilesRequest => (main_db, lib_path),
            FetchMediaFileByIdsRequest => (main_db, lib_path),
            FetchParsedMediaFileRequest => (main_db, lib_path),
            SearchMediaFileSummaryRequest => (main_db),

            FetchCollectionGroupSummaryRequest => (main_db, recommend_db),
            FetchCollectionGroupsRequest => (main_db, recommend_db),
            FetchCollectionByIdsRequest => (main_db, recommend_db),
            SearchCollectionSummaryRequest => (main_db, recommend_db),

            GetCoverArtIdsByMixQueriesRequest => (main_db, recommend_db),
            GetPrimaryColorByTrackIdRequest => (main_db),

            FetchAllPlaylistsRequest => (main_db),
            CreatePlaylistRequest => (main_db),
            UpdatePlaylistRequest => (main_db),
            RemovePlaylistRequest => (main_db),
            AddItemToPlaylistRequest => (main_db),
            ReorderPlaylistItemPositionRequest => (main_db),
            GetPlaylistByIdRequest => (main_db),

            FetchAllMixesRequest => (main_db),
            CreateMixRequest => (main_db),
            UpdateMixRequest => (main_db),
            RemoveMixRequest => (main_db),
            AddItemToMixRequest => (main_db),
            GetMixByIdRequest => (main_db),
            MixQueryRequest => (main_db, recommend_db, lib_path),
            FetchMixQueriesRequest => (main_db),
            OperatePlaybackWithMixQueryRequest => (main_db, recommend_db, lib_path, player),

            SetLikedRequest => (main_db),
            GetLikedRequest => (main_db),

            ComplexQueryRequest => (main_db, recommend_db),
            SearchForRequest => (main_db),

            FetchDirectoryTreeRequest => (main_db),

            ListLogRequest => (main_db),
            ClearLogRequest => (main_db),
            RemoveLogRequest => (main_db),

            SystemInfoRequest => (main_db),
        );
    });
}

rinf::write_interface!();

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let enable_log = args.contains(&"--enable-log".to_string());

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

    let license = check_store_license().await;

    match license {
        Ok(license) => match license {
            Some((_sku, is_active, is_trial)) => StoreLicense {
                is_store_mode: true,
                is_active,
                is_trial,
            }
            .send_signal_to_dart(),
            _ => StoreLicense {
                is_store_mode: false,
                is_active: false,
                is_trial: false,
            }
            .send_signal_to_dart(),
        },
        Err(e) => {
            CrashResponse {
                detail: format!("{:#?}", e),
            }
            .send_signal_to_dart();
        }
    }

    // Start receiving the media library path
    if let Err(e) = receive_media_library_path(player_loop).await {
        error!("Failed to receive media library path: {}", e);
    }

    rinf::dart_shutdown().await;

    if let Some(guard) = _guard {
        drop(guard);
    }
}
