mod analyse;
mod collection;
mod connection;
mod cover_art;
mod directory;
mod library_home;
mod library_manage;
mod media_file;
mod messages;
mod mix;
mod playback;
mod player;
mod playlist;
mod search;
mod stat;
mod system;
mod utils;

use std::sync::Arc;

use log::{debug, error, info};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::{fmt, Layer, Registry};

pub use tokio;

use ::database::connection::connect_main_db;
use ::database::connection::connect_recommendation_db;
use ::database::connection::connect_search_db;
use ::playback::player::Player;

use crate::analyse::*;
use crate::collection::*;
use crate::connection::*;
use crate::cover_art::*;
use crate::directory::*;
use crate::library_home::*;
use crate::library_manage::*;
use crate::media_file::*;
use crate::messages::*;
use crate::mix::*;
use crate::playback::*;
use crate::player::initialize_player;
use crate::playlist::*;
use crate::search::*;
use crate::stat::*;
use crate::system::*;

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

                                if let Err(e) = [<$type:snake>]($($arg),*, dart_signal).await {
                                    error!("{:?}", e)
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

async fn player_loop(path: String) {
    info!("Media Library Received, initialize other receivers");

    tokio::spawn(async move {
        info!("Initializing database");

        let main_db = match connect_main_db(&path).await {
            Ok(db) => Arc::new(db),
            Err(e) => {
                error!("Failed to connect to main DB: {}", e);
                return;
            }
        };

        let recommend_db = match connect_recommendation_db(&path) {
            Ok(db) => Arc::new(db),
            Err(e) => {
                error!("Failed to connect to recommendation DB: {}", e);
                return;
            }
        };

        let search_db = match connect_search_db(&path) {
            Ok(db) => Arc::new(Mutex::new(db)),
            Err(e) => {
                error!("Failed to connect to search DB: {}", e);
                return;
            }
        };

        let lib_path = Arc::new(path);

        let cancel_token = CancellationToken::new();

        info!("Initializing player");
        let player = Player::new(Some(cancel_token.clone()));
        let player = Arc::new(Mutex::new(player));

        let cancel_token = Arc::new(cancel_token);

        info!("Initializing Player events");
        tokio::spawn(initialize_player(main_db.clone(), player.clone()));

        info!("Initializing UI events");
        select_signal!(
            cancel_token,

            CloseLibraryRequest => (lib_path, cancel_token),
            ScanAudioLibraryRequest => (main_db, search_db, cancel_token),
            AnalyseAudioLibraryRequest => (main_db, recommend_db, cancel_token),
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

            IfAnalyseExistsRequest => (main_db),
            GetAnalyseCountRequest => (main_db),

            FetchMediaFilesRequest => (main_db, lib_path),
            FetchMediaFileByIdsRequest => (main_db, lib_path),
            FetchParsedMediaFileRequest => (main_db, lib_path),
            SearchMediaFileSummaryRequest => (main_db),

            FetchCollectionGroupSummaryRequest => (main_db, recommend_db),
            FetchCollectionGroupsRequest => (main_db, recommend_db),
            FetchCollectionByIdsRequest => (main_db, recommend_db),
            SearchCollectionSummaryRequest => (main_db, recommend_db),

            GetRandomCoverArtIdsRequest => (main_db),
            GetCoverArtIdsByMixQueriesRequest => (main_db, recommend_db),

            FetchAllPlaylistsRequest => (main_db),
            CreatePlaylistRequest => (main_db, search_db),
            UpdatePlaylistRequest => (main_db, search_db),
            RemovePlaylistRequest => (main_db, search_db),
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

            FetchLibrarySummaryRequest => (main_db, recommend_db),
            SearchForRequest => (search_db),

            FetchDirectoryTreeRequest => (main_db),

            SystemInfoRequest => (main_db),
        );
    });
}

rinf::write_interface!();

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let enable_log = args.contains(&"--enable-log".to_string());

    let stdout_filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,symphonia_bundle_mp3::demuxer=off,sea_orm_migration::migrator=off,info",
    );

    if enable_log {
        let file_filter = EnvFilter::new("debug");
        let now = chrono::Local::now();
        let file_name = format!("{}.rune.log", now.format("%Y-%m-%d_%H-%M-%S"));
        let file_appender = tracing_appender::rolling::never(".", file_name);
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

        let file_layer = fmt::layer()
            .with_writer(non_blocking)
            .with_timer(fmt::time::ChronoLocal::rfc_3339())
            .with_span_events(fmt::format::FmtSpan::CLOSE)
            .with_filter(file_filter);

        // Combine the layers
        Registry::default().with(file_layer).init();

        info!("Logging is enabled");
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(stdout_filter)
            .with_test_writer()
            .init();
    }

    // Start receiving the media library path
    if let Err(e) = receive_media_library_path(player_loop).await {
        error!("Failed to receive media library path: {}", e);
    }

    rinf::dart_shutdown().await;
}
