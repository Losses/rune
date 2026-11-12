use anyhow::{Context, Result};
use std::collections::HashMap;

use crate::library_pool::LibraryHandle;
use ::database::actions::collection::CollectionQueryType;
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
        let search_fields: Vec<CollectionQueryType> =
            request.fields.iter().copied().map(Into::into).collect();
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
    call_id: Option<u64>,
    results: HashMap<CollectionQueryType, Vec<i64>>,
) -> SearchForResponse {
    let mut artists = Vec::new();
    let mut albums = Vec::new();
    let mut playlists = Vec::new();
    let mut tracks = Vec::new();
    let mut genres = Vec::new();
    let mut directories = Vec::new();
    let mut mixes = Vec::new();

    for (collection_type, ids) in results {
        let ids: Vec<i32> = ids.iter().map(|&x| x as i32).collect();
        match collection_type {
            CollectionQueryType::Artist => artists.extend(ids),
            CollectionQueryType::Album => albums.extend(ids),
            CollectionQueryType::Playlist => playlists.extend(ids),
            CollectionQueryType::Track => tracks.extend(ids),
            CollectionQueryType::Genre => genres.extend(ids),
            CollectionQueryType::Directory => directories.extend(ids),
            CollectionQueryType::Mix => mixes.extend(ids),
        }
    }

    SearchForResponse {
        call_id,
        artists,
        albums,
        playlists,
        tracks,
        genres,
        directories,
        mixes,
    }
}

#[cfg(test)]
mod tests {
    use super::search_response;
    use crate::messages::SearchField;
    use database::actions::{
        collection::CollectionQueryType,
        search::{add_term, search_for},
    };
    use migration::{Migrator, MigratorTrait};
    use sea_orm::Database;
    use std::collections::HashMap;
    use uuid::Uuid;

    #[test]
    fn search_handler_response_echoes_call_id() {
        let response = search_response(Some(42), HashMap::new());
        assert_eq!(response.call_id, Some(42));
    }

    #[tokio::test]
    async fn every_search_field_returns_its_group_from_the_library() -> anyhow::Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        let _ = migration::initialize_node_id(Uuid::new_v4().to_string());
        Migrator::up(&db, None).await?;
        let fields = [
            (SearchField::Track, CollectionQueryType::Track),
            (SearchField::Artist, CollectionQueryType::Artist),
            (SearchField::Directory, CollectionQueryType::Directory),
            (SearchField::Album, CollectionQueryType::Album),
            (SearchField::Mix, CollectionQueryType::Mix),
            (SearchField::Playlist, CollectionQueryType::Playlist),
            (SearchField::Genre, CollectionQueryType::Genre),
        ];

        for (index, (_, collection_type)) in fields.iter().enumerate() {
            add_term(
                &db,
                collection_type.clone(),
                index as i32 + 1,
                &format!("field-fixture-{index}"),
            )
            .await?;
        }

        for (index, (field, collection_type)) in fields.into_iter().enumerate() {
            let results = search_for(
                &db,
                &format!("field-fixture-{index}"),
                Some(vec![field.into()]),
                10,
            )
            .await?;
            let response = search_response(None, results);
            let expected_id = index as i32 + 1;
            let actual = match field {
                SearchField::Track => response.tracks,
                SearchField::Artist => response.artists,
                SearchField::Directory => response.directories,
                SearchField::Album => response.albums,
                SearchField::Mix => response.mixes,
                SearchField::Playlist => response.playlists,
                SearchField::Genre => response.genres,
            };
            if collection_type == CollectionQueryType::Directory {
                assert!(actual.is_empty(), "directories are not indexed");
            } else {
                assert!(
                    actual.contains(&expected_id),
                    "{collection_type:?}: {actual:?}"
                );
            }
        }

        Ok(())
    }
}
