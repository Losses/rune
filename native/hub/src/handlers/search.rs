use anyhow::{Context, Result};
use std::collections::HashMap;

use crate::library_pool::LibraryHandle;
use ::database::actions::collection::CollectionQueryType;
use ::database::actions::search::convert_to_collection_types;
use ::database::actions::search::search_for;

use crate::{
    Session, Signal,
    messages::*,
    utils::{GlobalParams, ParamsExtractor, require_library},
};

impl ParamsExtractor for SearchForRequest {
    type Params = (Option<LibraryHandle>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.library.clone(),)
    }
}

impl Signal for SearchForRequest {
    type Params = (Option<LibraryHandle>,);
    type Response = SearchForResponse;

    async fn handle(
        &self,
        (library,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let library = require_library(library)?;
        let query_str = &request.query_str;
        let search_fields = convert_to_collection_types(request.fields.clone());
        let n = request.n as usize;

        let results = search_for(
            &library.main_db,
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

        Ok(Some(search_response(request.call_id, results)))
    }
}

fn search_response(
    call_id: u64,
    results: HashMap<CollectionQueryType, Vec<i64>>,
) -> SearchForResponse {
    let mut artists = Vec::new();
    let mut albums = Vec::new();
    let mut playlists = Vec::new();
    let mut tracks = Vec::new();
    let mut genres = Vec::new();

    for (collection_type, ids) in results {
        let ids: Vec<i32> = ids.iter().map(|&x| x as i32).collect();
        match collection_type {
            CollectionQueryType::Artist => artists.extend(ids),
            CollectionQueryType::Album => albums.extend(ids),
            CollectionQueryType::Playlist => playlists.extend(ids),
            CollectionQueryType::Track => tracks.extend(ids),
            CollectionQueryType::Genre => genres.extend(ids),
            _ => {}
        }
    }

    SearchForResponse {
        call_id,
        artists,
        albums,
        playlists,
        tracks,
        genres,
    }
}

#[cfg(test)]
mod tests {
    use super::search_response;
    use std::collections::HashMap;

    #[test]
    fn search_handler_response_echoes_call_id() {
        let response = search_response(42, HashMap::new());
        assert_eq!(response.call_id, 42);
    }
}
