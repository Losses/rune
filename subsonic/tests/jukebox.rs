use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use subsonic::{SubsonicClient, SubsonicCredentials, SubsonicError, SubsonicServerError};
use url::Url;

mod support;

type RequestLog = Arc<Mutex<Vec<HashMap<String, String>>>>;
const TEST_DEADLINE: Duration = Duration::from_secs(60);
type ResponseOverride = Option<(StatusCode, String)>;

#[derive(Clone)]
struct ServerState {
    requests: RequestLog,
    response: ResponseOverride,
}

struct Fixture {
    url: Url,
    requests: RequestLog,
    _server: support::TestServer,
}

async fn fixture() -> Fixture {
    fixture_with(None).await
}

async fn fixture_with(response: ResponseOverride) -> Fixture {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/rest/{*endpoint}", get(handle))
        .with_state(ServerState {
            requests: requests.clone(),
            response,
        });
    let server = support::serve(app).await;
    Fixture {
        url: server.url.clone(),
        requests,
        _server: server,
    }
}

async fn handle(
    State(state): State<ServerState>,
    Path(endpoint): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    assert_eq!(endpoint, "jukeboxControl.view");
    let action = query.get("action").cloned().unwrap_or_default();
    state.requests.lock().unwrap().push(query.clone());
    if let Some((status, body)) = state.response {
        return (status, body).into_response();
    }
    let payload = match action.as_str() {
        "get" => {
            r#"{"jukeboxPlaylist":{"currentIndex":0,"playing":true,"gain":0.7,"position":12,"entry":{"id":"song-001","title":"Track"}}}"#
        }
        "status" => {
            r#"{"jukeboxStatus":{"currentIndex":0,"playing":true,"gain":0.7,"position":12}}"#
        }
        _ => r#"{"jukeboxStatus":{"currentIndex":0,"playing":false,"gain":0.7,"position":0}}"#,
    };
    let mut response = serde_json::json!({
        "subsonic-response": {"status": "ok", "version": "1.16.1"}
    });
    response["subsonic-response"]
        .as_object_mut()
        .unwrap()
        .extend(
            serde_json::from_str::<serde_json::Value>(payload)
                .unwrap()
                .as_object()
                .unwrap()
                .clone(),
        );
    (StatusCode::OK, response.to_string()).into_response()
}

fn client(url: Url) -> SubsonicClient {
    SubsonicClient::new(
        url,
        SubsonicCredentials::Password {
            user: "alice".into(),
            password: "secret-password".into(),
        },
        reqwest::Client::new(),
    )
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(TEST_DEADLINE, future)
        .await
        .expect("request timed out")
}

fn assert_authenticated_action(fixture: &Fixture, action: &str) {
    let requests = fixture.requests.lock().unwrap();
    let request = requests.last().unwrap();
    assert_eq!(request["action"], action);
    for name in ["u", "t", "s", "v", "c", "f"] {
        assert!(request.contains_key(name), "missing auth parameter {name}");
    }
    assert!(!request.contains_key("p"));
}

#[tokio::test]
async fn jukebox_get_returns_playlist_and_status() {
    let server = fixture().await;
    let jukebox = bounded(client(server.url.clone()).jukebox_get())
        .await
        .unwrap();
    assert_eq!(jukebox.playlist.songs[0].id, "song-001");
    assert_eq!(jukebox.status.current_index, Some(0));
    assert_eq!(jukebox.status.position_secs, Some(12));
    assert_authenticated_action(&server, "get");
}

#[tokio::test]
async fn jukebox_status_returns_typed_status() {
    let server = fixture().await;
    let status = bounded(client(server.url.clone()).jukebox_status())
        .await
        .unwrap();
    assert_eq!(status.current_index, Some(0));
    assert!(status.playing);
    assert_eq!(status.gain, 0.7);
    assert_authenticated_action(&server, "status");
}

#[tokio::test]
async fn jukebox_set_sends_song_ids() {
    let server = fixture().await;
    bounded(client(server.url.clone()).jukebox_set(&["song-a"]))
        .await
        .unwrap();
    assert_authenticated_action(&server, "set");
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests[0]["id"], "song-a");
}

#[tokio::test]
async fn jukebox_add_sends_song_ids() {
    let server = fixture().await;
    bounded(client(server.url.clone()).jukebox_add(&["song-a"]))
        .await
        .unwrap();
    assert_authenticated_action(&server, "add");
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests[0]["id"], "song-a");
}

#[tokio::test]
async fn jukebox_remove_sends_index() {
    let server = fixture().await;
    bounded(client(server.url.clone()).jukebox_remove(3))
        .await
        .unwrap();
    assert_authenticated_action(&server, "remove");
    assert_eq!(server.requests.lock().unwrap()[0]["index"], "3");
}

#[tokio::test]
async fn jukebox_clear_sends_action() {
    let server = fixture().await;
    bounded(client(server.url.clone()).jukebox_clear())
        .await
        .unwrap();
    assert_authenticated_action(&server, "clear");
}

#[tokio::test]
async fn jukebox_skip_sends_index_and_offset() {
    let server = fixture().await;
    bounded(client(server.url.clone()).jukebox_skip(2, 17))
        .await
        .unwrap();
    assert_authenticated_action(&server, "skip");
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests[0]["index"], "2");
    assert_eq!(requests[0]["offset"], "17");
}

#[tokio::test]
async fn jukebox_start_sends_action() {
    let server = fixture().await;
    bounded(client(server.url.clone()).jukebox_start())
        .await
        .unwrap();
    assert_authenticated_action(&server, "start");
}

#[tokio::test]
async fn jukebox_stop_sends_action() {
    let server = fixture().await;
    bounded(client(server.url.clone()).jukebox_stop())
        .await
        .unwrap();
    assert_authenticated_action(&server, "stop");
}

#[tokio::test]
async fn jukebox_shuffle_sends_action() {
    let server = fixture().await;
    bounded(client(server.url.clone()).jukebox_shuffle())
        .await
        .unwrap();
    assert_authenticated_action(&server, "shuffle");
}

#[tokio::test]
async fn jukebox_set_gain_sends_gain() {
    let server = fixture().await;
    bounded(client(server.url.clone()).jukebox_set_gain(0.4))
        .await
        .unwrap();
    assert_authenticated_action(&server, "setGain");
    assert_eq!(server.requests.lock().unwrap()[0]["gain"], "0.4");
}

#[tokio::test]
async fn jukebox_gain_rejects_out_of_range_before_sending() {
    let server = fixture().await;
    let error = bounded(client(server.url.clone()).jukebox_set_gain(1.1))
        .await
        .unwrap_err();
    assert_eq!(error, SubsonicError::InvalidJukeboxGain);
    assert!(server.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn missing_jukebox_role_is_not_authorized() {
    let server = fixture_with(Some((
        StatusCode::OK,
        r#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":50,"message":"jukebox role not granted"}}}"#.into(),
    )))
    .await;
    let error = bounded(client(server.url.clone()).jukebox_start())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        SubsonicError::Server {
            kind: SubsonicServerError::NotAuthorized,
            message: "jukebox role not granted".into(),
        }
    );
    assert_authenticated_action(&server, "start");
}

#[tokio::test]
async fn jukebox_unsupported_server_errors_are_typed() {
    for code in [0, 70] {
        let body = format!(
            r#"{{"subsonic-response":{{"status":"failed","version":"1.16.1","error":{{"code":{code},"message":"jukebox control unsupported"}}}}}}"#
        );
        let server = fixture_with(Some((StatusCode::OK, body))).await;
        assert_eq!(
            bounded(client(server.url.clone()).jukebox_status())
                .await
                .unwrap_err(),
            SubsonicError::JukeboxUnsupported
        );
    }
    for status in [StatusCode::NOT_FOUND, StatusCode::NOT_IMPLEMENTED] {
        let server = fixture_with(Some((status, "".into()))).await;
        assert_eq!(
            bounded(client(server.url.clone()).jukebox_status())
                .await
                .unwrap_err(),
            SubsonicError::JukeboxUnsupported
        );
    }
}
