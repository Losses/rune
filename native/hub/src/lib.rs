mod album;
mod artist;
mod common;
mod connection;
mod cover_art;
mod library_home;
mod media_file;
mod messages;
mod playback;
mod player;
mod playlist;

use log::{debug, info};
use std::sync::Arc;
use std::sync::Mutex;
use tracing_subscriber::filter::EnvFilter;

pub use tokio;

use ::database::connection::connect_main_db;
use ::database::connection::connect_recommendation_db;
use ::playback::player::Player;

use crate::album::*;
use crate::artist::*;
use crate::connection::*;
use crate::cover_art::*;
use crate::library_home::*;
use crate::media_file::*;
use crate::playback::*;
use crate::player::initialize_player;
use crate::playlist::*;

use messages::album::*;
use messages::artist::*;
use messages::cover_art::*;
use messages::library_home::*;
use messages::media_file::*;
use messages::playback::*;
use messages::playlist::*;
use messages::recommend::*;

macro_rules! select_signal {
    ( $( $type:ty => ($($arg:ident),*) ),* $(,)? ) => {
        paste::paste! {
            $(
                let mut [<receiver_ $type:snake>] = <$type>::get_dart_signal_receiver().unwrap();
            )*

            loop {
                tokio::select! {
                    $(
                        dart_signal = [<receiver_ $type:snake>].recv() => {
                            if let Some(dart_signal) = dart_signal {
                                debug!("Processing signal: {}", stringify!($type));
                                let handler_fn = [<$type:snake>];
                                let _ = handler_fn($($arg.clone()),*, dart_signal).await;
                            }
                        }
                    )*
                    else => continue,
                }
            }
        }
    };
}

rinf::write_interface!();

async fn main() {
    let filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,sea_orm_migration::migrator=off,info",
    );

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .init();

    // Start receiving the media library path
    tokio::spawn(receive_media_library_path());

    // Ensure that the path is set before calling fetch_media_files
    loop {
        if let Some(path) = get_media_library_path().await {
            info!("Media Library Received, initialize other receivers");

            tokio::spawn(async {
                // Move the path into the async block
                info!("Initializing database");
                let main_db = Arc::new(connect_main_db(&path).await.unwrap());
                let recommend_db = Arc::new(connect_recommendation_db(&path).unwrap());
                let lib_path = Arc::new(path);

                info!("Initializing player");
                let player = Player::new();
                let player = Arc::new(Mutex::new(player));

                info!("Initializing Player events");
                tokio::spawn(initialize_player(main_db.clone(), player.clone()));

                info!("Initializing UI events");

                select_signal!(
                    FetchMediaFilesRequest => (main_db, lib_path),
                    CompoundQueryMediaFilesRequest => (main_db, lib_path),
                    PlayFileRequest => (main_db, player),
                    RecommendAndPlayRequest => (main_db, recommend_db, lib_path, player),
                    PlayRequest => (player),
                    PauseRequest => (player),
                    NextRequest => (player),
                    PreviousRequest => (player),
                    SwitchRequest => (player),
                    SeekRequest => (player),
                    RemoveRequest => (player),
                    MovePlaylistItemRequest => (player),
                    GetCoverArtByFileIdRequest => (main_db, lib_path),
                    GetCoverArtByCoverArtIdRequest => (main_db),
                    GetRandomCoverArtIdsRequest => (main_db),
                    FetchArtistsGroupSummaryRequest => (main_db),
                    FetchArtistsGroupsRequest => (main_db),
                    FetchAlbumsGroupSummaryRequest => (main_db),
                    FetchAlbumsGroupsRequest => (main_db),
                    FetchPlaylistsGroupSummaryRequest => (main_db),
                    FetchPlaylistsGroupsRequest => (main_db),
                    FetchLibrarySummaryRequest => (main_db),
                    CreatePlaylistRequest => (main_db),
                    UpdatePlaylistRequest => (main_db),
                    CheckItemsInPlaylistRequest => (main_db),
                    AddItemToPlaylistRequest => (main_db),
                    AddMediaFileToPlaylistRequest => (main_db),
                    ReorderPlaylistItemPositionRequest => (main_db),
                    GetUniquePlaylistGroupsRequest => (main_db),
                    GetPlaylistByIdRequest => (main_db),
                );
            });

            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
