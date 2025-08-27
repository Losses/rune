use std::sync::Arc;

use anyhow::{Context, Result};

use ::database::actions::collection::CollectionQueryType;
use ::database::actions::search::convert_to_collection_types;
use ::database::actions::search::search_for;
use ::database::connection::MainDbConnection;

use crate::{
    Session, Signal,
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
};

impl ParamsExtractor for SearchForRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for SearchForRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = SearchForResponse;

    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let query_str = &request.query_str;
        let search_fields = convert_to_collection_types(request.fields.clone());
        let n = request.n as usize;

        let results = search_for(
            &main_db,
            query_str,
            if search_fields.is_empty() {
                None
            } else {
                Some(search_fields)
            },
            n,
        )
        .await
        .with_context(|| format!("Search request failed: query_str={query_str}, n={n}"))?;

        let mut artists: Vec<i32> = Vec::new();
        let mut albums: Vec<i32> = Vec::new();
        let mut playlists: Vec<i32> = Vec::new();
        let mut tracks: Vec<i32> = Vec::new();

        for (collection_type, ids) in results {
            let ids: Vec<i32> = ids.iter().map(|&x| x as i32).collect();
            match collection_type {
                CollectionQueryType::Artist => artists.extend(ids),
                CollectionQueryType::Album => albums.extend(ids),
                CollectionQueryType::Playlist => playlists.extend(ids),
                CollectionQueryType::Track => tracks.extend(ids),
                _ => {}
            }
        }

        Ok(Some(SearchForResponse {
            artists,
            albums,
            playlists,
            tracks,
        }))
    }
}
