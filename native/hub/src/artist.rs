use std::sync::Arc;

use anyhow::{Context, Result};
use rinf::DartSignal;

use database::actions::artists::get_artists_by_ids;
use database::actions::artists::get_artists_groups;
use database::actions::artists::list_artists;
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
) -> Result<()> {
    let count_artists = create_count_by_first_letter::<artists::Entity>();

    let entry = count_artists(&main_db)
        .await
        .with_context(|| "Failed to fetch artists groups summary")?;

    let artists_groups = entry
        .into_iter()
        .map(|x| ArtistsGroupSummary {
            group_title: x.0,
            count: x.1,
        })
        .collect();
    ArtistGroupSummaryResponse { artists_groups }.send_signal_to_dart();

    Ok(())
}

pub async fn fetch_artists_groups_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchArtistsGroupsRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let entry = get_artists_groups(&main_db, request.group_titles)
        .await
        .with_context(|| "Failed to fetch artists groups")?;

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

    Ok(())
}

pub async fn fetch_artists_by_ids_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchArtistsByIdsRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let items = get_artists_by_ids(&main_db, &request.ids)
        .await
        .with_context(|| "Failed to fetch artists by id")?;
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

    Ok(())
}

pub async fn search_artist_summary_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<SearchArtistSummaryRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let items = list_artists(&main_db, request.n.try_into().unwrap())
        .await
        .with_context(|| "Failed to search artist summary")?;

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

    Ok(())
}
