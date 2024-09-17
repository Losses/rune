use std::sync::Arc;

use database::actions::playlists::list_playlists;
use log::{debug, error};
use rinf::DartSignal;
use tokio::sync::Mutex;

use database::actions::cover_art::get_magic_cover_art_id;
use database::actions::library::get_playlist_cover_ids;
use database::actions::playlists::add_item_to_playlist;
use database::actions::playlists::add_media_file_to_playlist;
use database::actions::playlists::check_items_in_playlist;
use database::actions::playlists::create_playlist;
use database::actions::playlists::get_all_playlists;
use database::actions::playlists::get_playlist_by_id;
use database::actions::playlists::get_playlists_by_ids;
use database::actions::playlists::get_playlists_groups;
use database::actions::playlists::get_unique_playlist_groups;
use database::actions::playlists::reorder_playlist_item_position;
use database::actions::playlists::update_playlist;
use database::actions::utils::create_count_by_first_letter;
use database::connection::MainDbConnection;
use database::connection::SearchDbConnection;
use database::entities::playlists;

use crate::PlaylistSummary;
use crate::SearchPlaylistSummaryRequest;
use crate::SearchPlaylistSummaryResponse;
use crate::{
    AddItemToPlaylistRequest, AddItemToPlaylistResponse, AddMediaFileToPlaylistRequest,
    AddMediaFileToPlaylistResponse, CheckItemsInPlaylistRequest, CheckItemsInPlaylistResponse,
    CreatePlaylistRequest, CreatePlaylistResponse, FetchAllPlaylistsRequest,
    FetchAllPlaylistsResponse, FetchPlaylistsByIdsRequest, FetchPlaylistsByIdsResponse,
    FetchPlaylistsGroupSummaryRequest, FetchPlaylistsGroupsRequest, GetPlaylistByIdRequest,
    GetPlaylistByIdResponse, GetUniquePlaylistGroupsRequest, GetUniquePlaylistGroupsResponse,
    Playlist, PlaylistGroupSummaryResponse, PlaylistWithoutCoverIds, PlaylistsGroup,
    PlaylistsGroupSummary, PlaylistsGroups, ReorderPlaylistItemPositionRequest,
    ReorderPlaylistItemPositionResponse, UpdatePlaylistRequest, UpdatePlaylistResponse,
};

pub async fn fetch_playlists_group_summary_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<FetchPlaylistsGroupSummaryRequest>,
) {
    debug!("Requesting summary group");

    let count_playlists = create_count_by_first_letter::<playlists::Entity>();

    match count_playlists(&main_db).await {
        Ok(entry) => {
            let playlists_groups = entry
                .into_iter()
                .map(|x| PlaylistsGroupSummary {
                    group_title: x.0,
                    count: x.1,
                })
                .collect();
            PlaylistGroupSummaryResponse { playlists_groups }.send_signal_to_dart();
            // GENERATED
        }
        Err(e) => {
            error!("Failed to fetch playlists groups summary: {}", e);
        }
    };
}

pub async fn fetch_playlists_groups_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchPlaylistsGroupsRequest>,
) {
    let request = dart_signal.message;

    debug!("Requesting playlists groups");

    match get_playlists_groups(&main_db, request.group_titles).await {
        Ok(entry) => {
            PlaylistsGroups {
                groups: entry
                    .into_iter()
                    .map(|x| PlaylistsGroup {
                        group_title: x.0,
                        playlists: x
                            .1
                            .into_iter()
                            .map(|x| Playlist {
                                id: x.0.id,
                                name: x.0.name,
                                group: x.0.group,
                                cover_ids: x.1.into_iter().collect(),
                            })
                            .collect(),
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to fetch playlists groups: {}", e);
        }
    };
}

pub async fn fetch_playlists_by_ids_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchPlaylistsByIdsRequest>,
) {
    let request = dart_signal.message;

    debug!("Requesting playlists: {:#?}", request.ids);

    match get_playlists_by_ids(&main_db, &request.ids).await {
        Ok(items) => {
            let magic_cover_id = get_magic_cover_art_id(&main_db).await.unwrap_or(-1);
            let covers = get_playlist_cover_ids(&main_db, &items).await.unwrap();

            FetchPlaylistsByIdsResponse {
                result: items
                    .into_iter()
                    .map(|x| Playlist {
                        id: x.id,
                        name: x.name,
                        group: x.group,
                        cover_ids: covers
                            .get(&x.id)
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|x| *x != magic_cover_id)
                            .collect(),
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to fetch albums groups: {}", e);
        }
    };
}

pub async fn fetch_all_playlists_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<FetchAllPlaylistsRequest>,
) {
    debug!("Fetching all playlists");

    match get_all_playlists(&main_db).await {
        Ok(playlists) => {
            FetchAllPlaylistsResponse {
                playlists: playlists
                    .into_iter()
                    .map(|playlist| PlaylistWithoutCoverIds {
                        id: playlist.id,
                        name: playlist.name,
                        group: playlist.group,
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to create playlist: {}", e);
        }
    }
}

pub async fn create_playlist_request(
    main_db: Arc<MainDbConnection>,
    search_db: Arc<Mutex<SearchDbConnection>>,
    dart_signal: DartSignal<CreatePlaylistRequest>,
) {
    let request = dart_signal.message;

    debug!(
        "Creating playlist: name={}, group={}",
        request.name, request.group
    );

    // Lock the mutex to get mutable access to the search_db
    let mut search_db = search_db.lock().await;

    match create_playlist(&main_db, &mut search_db, request.name, request.group).await {
        Ok(playlist) => {
            CreatePlaylistResponse {
                playlist: Some(PlaylistWithoutCoverIds {
                    id: playlist.id,
                    name: playlist.name,
                    group: playlist.group,
                }),
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to create playlist: {}", e);
        }
    }
}

pub async fn update_playlist_request(
    main_db: Arc<MainDbConnection>,
    search_db: Arc<Mutex<SearchDbConnection>>,
    dart_signal: DartSignal<UpdatePlaylistRequest>,
) {
    let request = dart_signal.message;

    debug!(
        "Updating playlist: id={}, name={:?}, group={:?}",
        request.playlist_id, request.name, request.group
    );

    let mut search_db = search_db.lock().await;

    match update_playlist(
        &main_db,
        &mut search_db,
        request.playlist_id,
        Some(request.name),
        Some(request.group),
    )
    .await
    {
        Ok(playlist) => {
            UpdatePlaylistResponse {
                playlist: Some(PlaylistWithoutCoverIds {
                    id: playlist.id,
                    name: playlist.name,
                    group: playlist.group,
                }),
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to update playlist: {}", e);
        }
    }
}

pub async fn check_items_in_playlist_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<CheckItemsInPlaylistRequest>,
) {
    let request = dart_signal.message;

    debug!("Checking items in playlist: id={}", request.playlist_id);

    match check_items_in_playlist(&main_db, request.playlist_id, request.media_file_ids).await {
        Ok(duplicates) => {
            CheckItemsInPlaylistResponse {
                duplicate_media_file_ids: duplicates,
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to check items in playlist: {}", e);
        }
    }
}

pub async fn add_item_to_playlist_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<AddItemToPlaylistRequest>,
) {
    let request = dart_signal.message;

    debug!(
        "Adding item to playlist: playlist_id={}, media_file_id={}, position={:#?}",
        request.playlist_id, request.media_file_id, request.position
    );

    match add_item_to_playlist(
        &main_db,
        request.playlist_id,
        request.media_file_id,
        request.position,
    )
    .await
    {
        Ok(_) => {
            AddItemToPlaylistResponse { success: true }.send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to add item to playlist: {}", e);
            AddItemToPlaylistResponse { success: false }.send_signal_to_dart();
        }
    }
}

pub async fn add_media_file_to_playlist_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<AddMediaFileToPlaylistRequest>,
) {
    let request = dart_signal.message;

    debug!(
        "Adding media file to playlist: playlist_id={}, media_file_id={}",
        request.playlist_id, request.media_file_id
    );

    match add_media_file_to_playlist(&main_db, request.playlist_id, request.media_file_id).await {
        Ok(_) => {
            AddMediaFileToPlaylistResponse { success: true }.send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to add media file to playlist: {}", e);
            AddMediaFileToPlaylistResponse { success: false }.send_signal_to_dart();
        }
    }
}

pub async fn reorder_playlist_item_position_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<ReorderPlaylistItemPositionRequest>,
) {
    let request = dart_signal.message;

    debug!(
        "Reordering playlist item: playlist_id={}, media_file_id={}, new_position={}",
        request.playlist_id, request.media_file_id, request.new_position
    );

    match reorder_playlist_item_position(
        &main_db,
        request.playlist_id,
        request.media_file_id,
        request.new_position,
    )
    .await
    {
        Ok(_) => {
            ReorderPlaylistItemPositionResponse { success: true }.send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to reorder playlist item: {}", e);
            ReorderPlaylistItemPositionResponse { success: false }.send_signal_to_dart();
        }
    }
}

pub async fn get_unique_playlist_groups_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<GetUniquePlaylistGroupsRequest>,
) {
    debug!("Requesting unique playlist groups");

    match get_unique_playlist_groups(&main_db).await {
        Ok(groups) => {
            GetUniquePlaylistGroupsResponse { groups }.send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to get unique playlist groups: {}", e);
        }
    }
}

pub async fn get_playlist_by_id_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetPlaylistByIdRequest>,
) {
    let request = dart_signal.message;

    debug!("Requesting playlist by id: {}", request.playlist_id);

    match get_playlist_by_id(&main_db, request.playlist_id).await {
        Ok(playlist) => match playlist {
            Some(playlist) => {
                GetPlaylistByIdResponse {
                    playlist: Some(PlaylistWithoutCoverIds {
                        id: playlist.id,
                        name: playlist.name,
                        group: playlist.group,
                    }),
                }
                .send_signal_to_dart();
            }
            _none => {
                error!("Playlist not found: {}", request.playlist_id);
            }
        },
        Err(e) => {
            error!("Failed to get playlist by id: {}", e);
        }
    }
}

pub async fn search_playlist_summary_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<SearchPlaylistSummaryRequest>,
) {
    let request = dart_signal.message;

    match list_playlists(&main_db, request.n.try_into().unwrap()).await {
        Ok(items) => {
            SearchPlaylistSummaryResponse {
                result: items
                    .into_iter()
                    .map(|x| PlaylistSummary {
                        id: x.id,
                        name: x.name,
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to search album summary: {}", e);
        }
    };
}
