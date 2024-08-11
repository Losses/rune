use log::{debug, error};
use rinf::DartSignal;
use std::sync::Arc;

use database::actions::albums::get_albums_groups;
use database::actions::utils::create_count_by_first_letter;
use database::connection::MainDbConnection;
use database::entities::albums;

use crate::messages::album::Album;
use crate::messages::album::AlbumGroupSummaryResponse;
use crate::messages::album::AlbumsGroup;
use crate::messages::album::AlbumsGroupSummary;
use crate::messages::album::AlbumsGroups;
use crate::messages::album::FetchAlbumsGroupSummaryRequest;
use crate::messages::album::FetchAlbumsGroupsRequest;

pub async fn fetch_albums_group_summary_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<FetchAlbumsGroupSummaryRequest>,
) {
    debug!("Requesting summary group");

    let count_albums = create_count_by_first_letter::<albums::Entity>();

    match count_albums(&main_db).await {
        Ok(entry) => {
            let albums_groups = entry
                .into_iter()
                .map(|x| AlbumsGroupSummary {
                    group_title: x.0,
                    count: x.1,
                })
                .collect();
            AlbumGroupSummaryResponse { albums_groups }.send_signal_to_dart();
            // GENERATED
        }
        Err(e) => {
            error!("Failed to fetch albums groups summary: {}", e);
        }
    };
}

pub async fn fetch_albums_groups_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchAlbumsGroupsRequest>,
) {
    let request = dart_signal.message;

    debug!("Requesting albums groups");

    match get_albums_groups(&main_db, request.group_titles).await {
        Ok(entry) => {
            AlbumsGroups {
                groups: entry
                    .into_iter()
                    .map(|x| AlbumsGroup {
                        group_title: x.0,
                        albums: x
                            .1
                            .into_iter()
                            .map(|x| Album {
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
            error!("Failed to fetch albums groups: {}", e);
        }
    };
}
