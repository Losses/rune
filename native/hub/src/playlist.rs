use log::{debug, error};
use rinf::DartSignal;
use std::sync::Arc;

use database::actions::playlists::get_playlists_groups;
use database::actions::utils::create_count_by_first_letter;
use database::connection::MainDbConnection;
use database::entities::playlists;

use crate::messages::playlist::FetchPlaylistsGroupSummaryRequest;
use crate::messages::playlist::FetchPlaylistsGroupsRequest;
use crate::messages::playlist::Playlist;
use crate::messages::playlist::PlaylistGroupSummaryResponse;
use crate::messages::playlist::PlaylistsGroup;
use crate::messages::playlist::PlaylistsGroupSummary;
use crate::messages::playlist::PlaylistsGroups;

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
