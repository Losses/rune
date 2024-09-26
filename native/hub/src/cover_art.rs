use std::sync::Arc;

use anyhow::{Context, Result};
use futures::future::join_all;
use log::info;
use rinf::DartSignal;

use database::actions::cover_art::get_cover_art_by_file_id;
use database::actions::cover_art::get_cover_art_by_id;
use database::actions::cover_art::get_random_cover_art_ids;
use database::actions::mixes::query_mix_media_files;
use database::connection::MainDbConnection;
use database::connection::RecommendationDbConnection;

use crate::GetCoverArtIdsByMixQueriesRequest;
use crate::GetCoverArtIdsByMixQueriesResponse;
use crate::GetCoverArtIdsByMixQueriesResponseUnit;
use crate::{
    CoverArtByCoverArtIdResponse, CoverArtByFileIdResponse, GetCoverArtByCoverArtIdRequest,
    GetCoverArtByFileIdRequest, GetRandomCoverArtIdsRequest, GetRandomCoverArtIdsResponse,
};

pub async fn get_cover_art_by_file_id_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetCoverArtByFileIdRequest>,
) -> Result<()> {
    let request = dart_signal.message;
    let file_id = request.file_id;

    let cover_art = get_cover_art_by_file_id(&main_db, file_id)
        .await
        .with_context(|| format!("No cover art found: {}", file_id))?;
    match cover_art {
        Some((cover_art_id, cover_art)) => {
            if !cover_art.is_empty() {
                CoverArtByFileIdResponse {
                    file_id,
                    cover_art_id,
                    cover_art: Some(cover_art),
                }
                .send_signal_to_dart();
                // GENERATED
            } else {
                CoverArtByFileIdResponse {
                    file_id,
                    cover_art_id,
                    cover_art: None,
                }
                .send_signal_to_dart();
                // GENERATED
            }
        }
        _none => {
            CoverArtByFileIdResponse {
                file_id,
                cover_art_id: -1,
                cover_art: None,
            }
            .send_signal_to_dart();
        }
    };

    Ok(())
}

pub async fn get_cover_art_by_cover_art_id_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetCoverArtByCoverArtIdRequest>,
) -> Result<()> {
    let cover_art_id = dart_signal.message.cover_art_id;

    let entry = get_cover_art_by_id(&main_db, cover_art_id)
        .await
        .with_context(|| "Failed to get cover art by cover art id")?;

    match entry {
        Some(entry) => CoverArtByCoverArtIdResponse {
            cover_art_id,
            cover_art: Some(entry),
        }
        .send_signal_to_dart(),
        _none => CoverArtByCoverArtIdResponse {
            cover_art_id,
            cover_art: None,
        }
        .send_signal_to_dart(),
    };

    Ok(())
}

pub async fn get_random_cover_art_ids_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetRandomCoverArtIdsRequest>,
) -> Result<()> {
    let count = dart_signal.message.count;

    let items = get_random_cover_art_ids(&main_db, count as usize)
        .await
        .with_context(|| "Unable to get random cover art ids")?;

    GetRandomCoverArtIdsResponse {
        cover_art_ids: items.into_iter().map(|x| x.id).collect(),
    }
    .send_signal_to_dart();

    Ok(())
}

pub async fn get_cover_art_ids_by_mix_queries_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    dart_signal: DartSignal<GetCoverArtIdsByMixQueriesRequest>,
) -> Result<()> {
    let requests = dart_signal.message.requests;

    let files_futures = requests.clone().into_iter().map(|x| {
        let main_db = Arc::clone(&main_db);
        let recommend_db = Arc::clone(&recommend_db);
        async move {
            let query = query_mix_media_files(
                &main_db,
                &recommend_db,
                x.queries
                    .into_iter()
                    .map(|q| (q.operator, q.parameter))
                    .chain([("filter::with_cover_art".to_string(), "true".to_string())])
                    .collect(),
                0,
                36,
            )
            .await;

            match query {
                Ok(files) => {
                    let mut cover_art_ids: Vec<i32> =
                        files.iter().filter_map(|file| file.cover_art_id).collect();

                    cover_art_ids.dedup();

                    GetCoverArtIdsByMixQueriesResponseUnit {
                        id: x.id,
                        cover_art_ids,
                    }
                }
                Err(_) => GetCoverArtIdsByMixQueriesResponseUnit {
                    id: x.id,
                    cover_art_ids: Vec::new(),
                },
            }
        }
    });

    GetCoverArtIdsByMixQueriesResponse {
        result: join_all(files_futures).await,
    }
    .send_signal_to_dart();

    Ok(())
}
