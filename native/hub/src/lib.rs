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
mod messages;
mod mix;
mod playback;
mod player;
mod playlist;
mod scrobble;
mod search;
mod sfx;
mod stat;
mod system;
mod utils;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use log::{debug, error, info};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::fmt;
use tracing_subscriber::EnvFilter;

pub use tokio;

use ::playback::player::Player;
use ::playback::sfx_player::SfxPlayer;
use ::scrobbling::manager::ScrobblingManager;

use crate::analyze::*;
use crate::collection::*;
use crate::connection::*;
use crate::cover_art::*;
use crate::directory::*;
use crate::library_home::*;
use crate::library_manage::*;
use crate::license::validate_license_request;
use crate::license::*;
use crate::logging::*;
use crate::lyric::*;
use crate::media_file::*;
use crate::messages::*;
use crate::mix::*;
use crate::playback::*;
use crate::player::initialize_player;
use crate::playlist::*;
use crate::scrobble::*;
use crate::search::*;
use crate::sfx::*;
use crate::stat::*;
use crate::system::*;
use crate::utils::init_logging;

macro_rules! select_signal {
    ($cancel_token:expr, $( $type:ty $(| $response:ty)? => ($($arg:ident),*) ),* $(,)? ) => {
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

                                let result = [<$type:snake>]($($arg),*, dart_signal).await
                                    .with_context(|| format!("Processing signal: {}", stringify!($type)));

                                handle_result!(result, $($response)?);
                            }
                        }
                    )*
                    else => continue,
                }
            }
        }
    };
}

macro_rules! handle_result {
    ($result:expr, $type:ty) => {
        match $result {
            Ok(response) => {
                if let Some(response) = response {
                    response.send_signal_to_dart();
                }
            }
            Err(e) => {
                error!("{:?}", e);
                CrashResponse {
                    detail: format!("{:#?}", e),
                }
                .send_signal_to_dart();
            }
        }
    };
    ($result:expr,) => {
        if let Err(e) = $result {
            error!("{:?}", e);
            CrashResponse {
                detail: format!("{:#?}", e),
            }
            .send_signal_to_dart();
        }
    };
}

#[derive(Default)]
struct TaskTokens {
    scan_token: Option<CancellationToken>,
    analyze_token: Option<CancellationToken>,
}

async fn player_loop(
    path: String,
    db_connections: DatabaseConnections,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
) {
    info!("Media Library Received, initialize other receivers");

    tokio::spawn(async move {
        info!("Initializing database");

        let main_db: Arc<sea_orm::DatabaseConnection> = db_connections.main_db;

        let recommend_db: Arc<database::connection::RecommendationDbConnection> =
            db_connections.recommend_db;

        let lib_path: Arc<String> = Arc::new(path);

        let main_cancel_token = CancellationToken::new();
        let task_tokens = Arc::new(Mutex::new(TaskTokens::default()));

        info!("Initializing player");
        let player = Player::new(Some(main_cancel_token.clone()));
        let player = Arc::new(Mutex::new(player));

        let sfx_player = SfxPlayer::new(Some(main_cancel_token.clone()));
        let sfx_player = Arc::new(Mutex::new(sfx_player));

        let main_cancel_token = Arc::new(main_cancel_token);

        info!("Initializing Player events");
        tokio::spawn(initialize_player(
            lib_path.clone(),
            main_db.clone(),
            player.clone(),
            scrobbler.clone(),
        ));

        info!("Initializing UI events");
        select_signal!(
            main_cancel_token,

            TestLibraryInitializedRequest | TestLibraryInitializedResponse => (main_db),
            CloseLibraryRequest           | CloseLibraryResponse           => (lib_path, main_cancel_token, task_tokens),
            CancelTaskRequest             | CancelTaskResponse             => (task_tokens),
            ScanAudioLibraryRequest                                        => (main_db, task_tokens),
            AnalyzeAudioLibraryRequest                                     => (main_db, recommend_db, task_tokens),

            VolumeRequest                 | VolumeResponse     => (player),
            LoadRequest                                        => (player),
            PlayRequest                                        => (player),
            PauseRequest                                       => (player),
            NextRequest                                        => (main_db, player),
            PreviousRequest                                    => (main_db, player),
            SwitchRequest                                      => (main_db, player),
            SeekRequest                                        => (player),
            RemoveRequest                                      => (player),
            SetPlaybackModeRequest                             => (player),
            MovePlaylistItemRequest                            => (player),
            SetRealtimeFftEnabledRequest                       => (player),
            SetAdaptiveSwitchingEnabledRequest                 => (player),

            SfxPlayRequest                                     => (sfx_player),

            IfAnalyzeExistsRequest             | IfAnalyzeExistsResponse             => (main_db),
            GetAnalyzeCountRequest             | GetAnalyzeCountResponse             => (main_db),

            FetchMediaFilesRequest             | FetchMediaFilesResponse             => (main_db, lib_path),
            FetchMediaFileByIdsRequest         | FetchMediaFileByIdsResponse         => (main_db, lib_path),
            FetchParsedMediaFileRequest        | FetchParsedMediaFileResponse        => (main_db, lib_path),
            SearchMediaFileSummaryRequest      | SearchMediaFileSummaryResponse      => (main_db),

            GetLyricByTrackIdRequest           | GetLyricByTrackIdResponse           => (lib_path, main_db),

            FetchCollectionGroupSummaryRequest | CollectionGroupSummaryResponse      => (main_db),
            FetchCollectionGroupsRequest       | CollectionGroups                    => (main_db, recommend_db),
            FetchCollectionByIdsRequest        | FetchCollectionByIdsResponse        => (main_db, recommend_db),
            SearchCollectionSummaryRequest     | SearchCollectionSummaryResponse     => (main_db),

            GetCoverArtIdsByMixQueriesRequest  | GetCoverArtIdsByMixQueriesResponse  => (main_db, recommend_db),
            GetPrimaryColorByTrackIdRequest    | GetPrimaryColorByTrackIdResponse    => (main_db),

            FetchAllPlaylistsRequest           | FetchAllPlaylistsResponse           => (main_db),
            CreatePlaylistRequest              | CreatePlaylistResponse              => (main_db),
            CreateM3u8PlaylistRequest          | CreateM3u8PlaylistResponse          => (main_db),
            UpdatePlaylistRequest              | UpdatePlaylistResponse              => (main_db),
            RemovePlaylistRequest              | RemovePlaylistResponse              => (main_db),
            AddItemToPlaylistRequest           | AddItemToPlaylistResponse           => (main_db),
            ReorderPlaylistItemPositionRequest | ReorderPlaylistItemPositionResponse => (main_db),
            GetPlaylistByIdRequest             | GetPlaylistByIdResponse             => (main_db),

            FetchAllMixesRequest               | FetchAllMixesResponse               => (main_db),
            CreateMixRequest                   | CreateMixResponse                   => (main_db),
            UpdateMixRequest                   | UpdateMixResponse                   => (main_db),
            RemoveMixRequest                   | RemoveMixResponse                   => (main_db),
            AddItemToMixRequest                | AddItemToMixResponse                => (main_db),
            GetMixByIdRequest                  | GetMixByIdResponse                  => (main_db),
            MixQueryRequest                    | MixQueryResponse                    => (main_db, recommend_db, lib_path),
            FetchMixQueriesRequest             | FetchMixQueriesResponse             => (main_db),
            OperatePlaybackWithMixQueryRequest | OperatePlaybackWithMixQueryResponse => (main_db, recommend_db, lib_path, player),

            SetLikedRequest                    | SetLikedResponse                    => (main_db),
            GetLikedRequest                    | GetLikedResponse                    => (main_db),

            ComplexQueryRequest                | ComplexQueryResponse                => (main_db, recommend_db),
            SearchForRequest                   | SearchForResponse                   => (main_db),

            FetchDirectoryTreeRequest          | FetchDirectoryTreeResponse          => (main_db),

            AuthenticateSingleServiceRequest   | AuthenticateSingleServiceResponse   => (scrobbler),
            AuthenticateMultipleServiceRequest                                       => (scrobbler),
            LogoutSingleServiceRequest                                               => (scrobbler),

            ListLogRequest => (main_db),
            ClearLogRequest => (main_db),
            RemoveLogRequest => (main_db),

            SystemInfoRequest => (main_db),
            RegisterLicenseRequest => (main_db),
            ValidateLicenseRequest => (main_db),
        );
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
