use std::sync::Arc;

use log::{debug, warn};
use rinf::DartSignal;
use tokio::sync::Mutex;

use database::actions::search::search_for;
use database::actions::search::CollectionType;
use database::connection::SearchDbConnection;

use crate::messages::search::{SearchForRequest, SearchForResponse};

pub async fn search_for_request(
    search_db: Arc<Mutex<SearchDbConnection>>,
    dart_signal: DartSignal<SearchForRequest>,
) {
    let request = dart_signal.message;
    let query_str = request.query_str;
    let n = request.n as usize;

    debug!("Received search request: query_str={}, n={}", query_str, n);

    let mut search_db = search_db.lock().await;

    match search_for(&mut search_db, &query_str, n) {
        Ok(results) => {
            let mut artists: Vec<i32> = Vec::new();
            let mut albums: Vec<i32> = Vec::new();
            let mut playlists: Vec<i32> = Vec::new();
            let mut tracks: Vec<i32> = Vec::new();

            for (collection_type, ids) in results {
                let ids: Vec<i32> = ids.iter().map(|&x| x as i32).collect();
                match collection_type {
                    CollectionType::Artist => artists.extend(ids),
                    CollectionType::Album => albums.extend(ids),
                    CollectionType::Playlist => playlists.extend(ids),
                    CollectionType::Track => tracks.extend(ids),
                    _ => {}
                }
            }

            SearchForResponse {
                artists,
                albums,
                playlists,
                tracks,
            }
            .send_signal_to_dart(); // GENERATED
        }
        Err(e) => {
            warn!("Search request failed: {:?}", e);
            SearchForResponse {
                artists: Vec::new(),
                albums: Vec::new(),
                playlists: Vec::new(),
                tracks: Vec::new(),
            }
            .send_signal_to_dart(); // GENERATED
        }
    }
}
