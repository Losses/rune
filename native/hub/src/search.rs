use std::sync::Arc;

use anyhow::{Context, Result};
use rinf::DartSignal;

use database::actions::collection::CollectionQueryType;
use database::actions::search::convert_to_collection_types;
use database::actions::search::search_for;
use database::connection::MainDbConnection;

use crate::messages::*;

pub async fn search_for_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<SearchForRequest>,
) -> Result<()> {
    let request = dart_signal.message;
    let query_str = request.query_str;
    if query_str.trim().is_empty() {
        SearchForResponse {
            artists: Vec::new(),
            albums: Vec::new(),
            playlists: Vec::new(),
            tracks: Vec::new(),
        }
        .send_signal_to_dart();
        return Ok(());
    }
    let search_fields = convert_to_collection_types(request.fields);
    let n = request.n as usize;

    let results = search_for(
        &main_db,
        &query_str,
        if search_fields.is_empty() {
            None
        } else {
            Some(search_fields)
        },
        n,
    )
    .await
    .with_context(|| format!("Search request failed: query_str={}, n={}", query_str, n))?;

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

    SearchForResponse {
        artists,
        albums,
        playlists,
        tracks,
    }
    .send_signal_to_dart();

    Ok(())
}
