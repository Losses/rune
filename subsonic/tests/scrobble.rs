use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use subsonic::{SubsonicClient, SubsonicCredentials};
mod support;

type Requests = Arc<Mutex<Vec<HashMap<String, String>>>>;

#[tokio::test]
async fn scrobble_sends_track_time_submission_and_authentication() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/rest/{*endpoint}", get(handle))
        .with_state(requests.clone());
    let server = support::serve(app).await;
    let client = SubsonicClient::new(
        server.url.clone(),
        SubsonicCredentials::Password {
            user: "alice".into(),
            password: "secret-password".into(),
        },
        reqwest::Client::new(),
    );

    tokio::time::timeout(
        Duration::from_secs(3),
        client.scrobble("song-42", None, false),
    )
    .await
    .expect("request timed out")
    .unwrap();
    client
        .scrobble("song-42", Some(1_700_000_123_456), true)
        .await
        .unwrap();

    let requests = requests.lock().unwrap();
    assert_eq!(requests[0]["id"], "song-42");
    assert_eq!(requests[0]["submission"], "false");
    assert!(!requests[0].contains_key("time"));
    assert_eq!(requests[1]["id"], "song-42");
    assert_eq!(requests[1]["time"], "1700000123456");
    assert_eq!(requests[1]["submission"], "true");
    for name in ["u", "t", "s", "v", "c", "f"] {
        assert!(
            requests[1].contains_key(name),
            "missing auth parameter {name}"
        );
    }
    assert!(!requests[1].contains_key("p"));
}

async fn handle(
    State(requests): State<Requests>,
    Path(endpoint): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    assert_eq!(endpoint, "scrobble.view");
    requests.lock().unwrap().push(query);
    (
        StatusCode::OK,
        r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#,
    )
}
