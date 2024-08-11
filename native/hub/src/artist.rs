use log::{debug, error};
use rinf::DartSignal;
use std::sync::Arc;

use database::actions::artists::get_artists_groups;
use database::actions::utils::create_count_by_first_letter;
use database::connection::MainDbConnection;
use database::entities::artists;

use crate::messages::artist::Artist;
use crate::messages::artist::ArtistGroupSummaryResponse;
use crate::messages::artist::ArtistsGroup;
use crate::messages::artist::ArtistsGroupSummary;
use crate::messages::artist::ArtistsGroups;
use crate::messages::artist::FetchArtistsGroupSummaryRequest;
use crate::messages::artist::FetchArtistsGroupsRequest;

pub async fn fetch_artists_group_summary_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<FetchArtistsGroupSummaryRequest>,
) {
    debug!("Requesting summary group");

    let count_artists = create_count_by_first_letter::<artists::Entity>();

    match count_artists(&main_db).await {
        Ok(entry) => {
            let artists_groups = entry
                .into_iter()
                .map(|x| ArtistsGroupSummary {
                    group_title: x.0,
                    count: x.1,
                })
                .collect();
            ArtistGroupSummaryResponse { artists_groups }.send_signal_to_dart();
            // GENERATED
        }
        Err(e) => {
            error!("Failed to fetch artists groups summary: {}", e);
        }
    };
}

pub async fn fetch_artists_groups_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchArtistsGroupsRequest>,
) {
    let request = dart_signal.message;

    debug!("Requesting artists groups");

    match get_artists_groups(&main_db, request.group_titles).await {
        Ok(entry) => {
            ArtistsGroups {
                groups: entry
                    .into_iter()
                    .map(|x| ArtistsGroup {
                        group_title: x.0,
                        artists: x
                            .1
                            .into_iter()
                            .map(|x| Artist {
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
            error!("Failed to fetch artists groups: {}", e);
        }
    };
}
