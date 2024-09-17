use std::sync::Arc;

use log::{debug, error};
use rinf::DartSignal;

use database::actions::artists::list_artists;
use database::actions::artists::get_artists_by_ids;
use database::actions::artists::get_artists_groups;
use database::actions::cover_art::get_magic_cover_art_id;
use database::actions::library::get_artist_cover_ids;
use database::actions::utils::create_count_by_first_letter;
use database::connection::MainDbConnection;
use database::entities::artists;

use crate::ArtistSummary;
use crate::SearchArtistSummaryRequest;
use crate::SearchArtistSummaryResponse;
use crate::{
    Artist, ArtistGroupSummaryResponse, ArtistsGroup, ArtistsGroupSummary, ArtistsGroups,
    FetchArtistsByIdsRequest, FetchArtistsByIdsResponse, FetchArtistsGroupSummaryRequest,
    FetchArtistsGroupsRequest,
};

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

pub async fn fetch_artists_by_ids_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchArtistsByIdsRequest>,
) {
    let request = dart_signal.message;

    debug!("Requesting artists: {:#?}", request.ids);

    match get_artists_by_ids(&main_db, &request.ids).await {
        Ok(items) => {
            let magic_cover_id = get_magic_cover_art_id(&main_db).await.unwrap_or(-1);
            let covers = get_artist_cover_ids(&main_db, &items).await.unwrap();

            FetchArtistsByIdsResponse {
                result: items
                    .into_iter()
                    .map(|x| Artist {
                        id: x.id,
                        name: x.name,
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

pub async fn search_artist_summary_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<SearchArtistSummaryRequest>,
) {
    let request = dart_signal.message;

    match list_artists(&main_db, request.n.try_into().unwrap()).await {
        Ok(items) => {
            SearchArtistSummaryResponse {
                result: items
                    .into_iter()
                    .map(|x| ArtistSummary {
                        id: x.id,
                        name: x.name,
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to search artist summary: {}", e);
        }
    };
}
