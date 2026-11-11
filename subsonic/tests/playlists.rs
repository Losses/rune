use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use subsonic::{SubsonicClient, SubsonicCredentials};
use url::Url;

mod support;

type RequestLog = Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>;

#[derive(Clone, Default)]
struct StateData {
    calls: RequestLog,
}

fn values<'a>(request: &'a [(String, String)], key: &str) -> Vec<&'a str> {
    request
        .iter()
        .filter(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
        .collect()
}

async fn endpoint(
    State(state): State<StateData>,
    Path(endpoint): Path<String>,
    Query(query): Query<Vec<(String, String)>>,
) -> impl IntoResponse {
    let name = endpoint.trim_end_matches(".view").to_owned();
    state
        .calls
        .lock()
        .unwrap()
        .push((name.clone(), query.clone()));
    let body = match name.as_str() {
        "getPlaylists" => r#"{"playlists":{"playlist":{"id":"p1","name":"First"}}}"#,
        "getPlaylist" => {
            r#"{"playlist":{"id":"p1","name":"First","entry":[{"id":"s1","title":"One"},{"id":"s2","title":"Two"}]}}"#
        }
        _ => "{}",
    };
    let mut response = serde_json::json!({"subsonic-response":{"status":"ok","version":"1.16.1"}});
    if name == "getPlaylists" || name == "getPlaylist" {
        response["subsonic-response"]
            .as_object_mut()
            .unwrap()
            .extend(
                serde_json::from_str::<serde_json::Value>(body)
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
            );
    }
    (StatusCode::OK, response.to_string())
}

#[tokio::test]
async fn playlist_actions_use_authenticated_requests_and_replace_entries() {
    let state = StateData::default();
    let server = support::serve(
        Router::new()
            .route("/rest/{*endpoint}", get(endpoint))
            .with_state(state.clone()),
    )
    .await;
    let client = SubsonicClient::new(
        Url::parse(server.url.as_str()).unwrap(),
        SubsonicCredentials::Password {
            user: "alice".into(),
            password: "secret".into(),
        },
        reqwest::Client::new(),
    );

    assert_eq!(client.playlists().await.unwrap()[0].id, "p1");
    assert_eq!(client.playlist("p1").await.unwrap().songs.len(), 2);
    client.create_playlist("New", &["s1", "s2"]).await.unwrap();
    client
        .replace_playlist_songs("p1", &["s2", "s1"])
        .await
        .unwrap();
    client
        .update_playlist("p1", Some("Renamed"), &["s3"], &[0])
        .await
        .unwrap();
    client.delete_playlist("p1").await.unwrap();

    let calls = state.calls.lock().unwrap();
    assert_eq!(
        calls.iter().map(|call| call.0.as_str()).collect::<Vec<_>>(),
        [
            "getPlaylists",
            "getPlaylist",
            "createPlaylist",
            "createPlaylist",
            "updatePlaylist",
            "deletePlaylist"
        ]
    );
    for (_, query) in calls.iter() {
        for key in ["u", "t", "s", "v", "c", "f"] {
            assert!(query.iter().any(|(name, _)| name == key));
        }
    }
    assert_eq!(values(&calls[2].1, "name"), ["New"]);
    assert_eq!(values(&calls[2].1, "songId"), ["s1", "s2"]);
    assert_eq!(values(&calls[3].1, "playlistId"), ["p1"]);
    assert_eq!(values(&calls[3].1, "songId"), ["s2", "s1"]);
    assert_eq!(values(&calls[4].1, "songIdToAdd"), ["s3"]);
    assert_eq!(values(&calls[4].1, "songIndexToRemove"), ["0"]);
}
