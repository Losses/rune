use std::collections::HashMap;

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use std::sync::{Arc, Mutex};
use subsonic::{SubsonicClient, SubsonicCredentials};

mod support;

type StateData = Arc<Mutex<Vec<(String, HashMap<String, String>)>>>;

async fn client() -> (SubsonicClient, StateData, support::TestServer) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/rest/{*endpoint}", get(handle))
        .with_state(calls.clone());
    let server = support::serve(app).await;
    let client = SubsonicClient::new(
        server.url.clone(),
        SubsonicCredentials::Password {
            user: "alice".into(),
            password: "secret".into(),
        },
        reqwest::Client::new(),
    );
    (client, calls, server)
}

async fn handle(
    State(calls): State<StateData>,
    Path(endpoint): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let endpoint = endpoint.trim_end_matches(".view").to_owned();
    calls
        .lock()
        .unwrap()
        .push((endpoint.clone(), query.clone()));
    let data = match endpoint.as_str() {
        "getStarred2" => {
            r#"{"starred2":{"song":{"id":"song-1","title":"Title","starred":"2026-01-01T00:00:00Z"}}}"#
        }
        "getLyrics" => {
            r#"{"lyrics":{"artist":"Artist","title":"Title","value":"[00:01.00]hello"}}"#
        }
        "getLyricsBySongId" if query.get("id").is_some_and(|id| id == "empty") => {
            r#"{"lyricsList":{}}"#
        }
        "getLyricsBySongId" => {
            r#"{"lyricsList":{"plainLyrics":{"lang":"en","text":"plain extension text"},"structuredLyrics":[{"lang":"en","synced":true,"line":[{"start":1200,"value":"hello"}]}]}}"#
        }
        _ => "{}",
    };
    let mut response = serde_json::json!({"subsonic-response":{"status":"ok","version":"1.16.1"}});
    response["subsonic-response"]
        .as_object_mut()
        .unwrap()
        .extend(
            serde_json::from_str::<serde_json::Value>(data)
                .unwrap()
                .as_object()
                .unwrap()
                .clone(),
        );
    (StatusCode::OK, serde_json::to_string(&response).unwrap())
}

#[tokio::test]
async fn stars_and_lyrics_use_subsonic_parameters_and_models() {
    let (client, calls, _server) = client().await;
    client.star(&["song-1"]).await.unwrap();
    client.unstar(&["song-1"]).await.unwrap();
    let starred = client.starred_songs().await.unwrap();
    assert_eq!(starred[0].id, "song-1");
    let plain = client.lyrics("Artist", "Title").await.unwrap();
    assert_eq!(plain.plain_lyrics.as_deref(), Some("[00:01.00]hello"));
    let synced = client.lyrics_by_song_id("song-1").await.unwrap();
    assert_eq!(synced.synced_lyrics[0].lines[0].start, 1200);
    assert_eq!(synced.plain_lyrics.as_deref(), Some("plain extension text"));
    assert!(
        client
            .lyrics_by_song_id("empty")
            .await
            .unwrap()
            .synced_lyrics
            .is_empty()
    );
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].0, "star");
    assert_eq!(calls[0].1["id"], "song-1");
    assert_eq!(calls[1].0, "unstar");
    assert_eq!(calls[2].0, "getStarred2");
    assert_eq!(calls[3].1["artist"], "Artist");
    assert_eq!(calls[4].1["id"], "song-1");
}
