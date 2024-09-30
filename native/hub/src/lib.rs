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
use tracing_subscriber::filter::EnvFilter;

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
use crate::mix::*;
use crate::playback::*;
use crate::player::initialize_player;
use crate::playlist::*;
use crate::search::*;
use crate::stat::*;
use crate::system::*;

use messages::analyse::*;
use messages::collection::*;
use messages::cover_art::*;
use messages::directory::*;
use messages::library_home::*;
use messages::library_manage::*;
use messages::media_file::*;
use messages::mix::*;
use messages::playback::*;
use messages::playlist::*;
use messages::search::*;
use messages::stat::*;
use messages::system::*;

macro_rules! select_signal {
    ($cancel_token:expr, $( $type:ty => ($($arg:ident),*) ),* $(,)? ) => {
        paste::paste! {
            $(
                let mut [<receiver_ $type:snake>] = match <$type>::get_dart_signal_receiver() {
                    Ok(receiver) => receiver,
                    Err(e) => {
                        error!("Failed to get Dart signal receiver for {}: {}", stringify!($type), e);
                        return;
                    }
                };
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

async fn main() {
    let filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,symphonia_bundle_mp3::demuxer=off,sea_orm_migration::migrator=off,info",
    );

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .init();

    // Start receiving the media library path
    if let Err(e) = receive_media_library_path(player_loop).await {
        error!("Failed to receive media library path: {}", e);
    }
}
