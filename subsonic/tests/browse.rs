use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use subsonic::{
    SearchLimits, StreamOptions, SubsonicClient, SubsonicCredentials, SubsonicError,
    SubsonicServerError,
};
use tokio::net::TcpListener;
use url::Url;

type Requests = Arc<Mutex<Vec<(String, HashMap<String, String>)>>>;

struct Fixture {
    url: Url,
    requests: Requests,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn fixture() -> Fixture {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/rest/{*endpoint}", get(handle))
        .with_state(requests.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Fixture {
        url: Url::parse(&format!("http://{address}")).unwrap(),
        requests,
        task,
    }
}

async fn handle(
    State(requests): State<Requests>,
    Path(endpoint): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let endpoint = endpoint.trim_end_matches(".view").to_owned();
    let special_id = query.get("id").cloned().unwrap_or_default();
    requests.lock().unwrap().push((endpoint.clone(), query));
    let payload = match endpoint.as_str() {
        "getArtists" => {
            r#"{"artists":{"index":{"name":"A","artist":{"id":"artist-001","name":"Artist","albumCount":1,"coverArt":"cover-a","unknown":true}}}}"#
        }
        "getArtist" => {
            r#"{"artist":{"id":"artist-001","name":"Artist","albumCount":1,"album":{"id":"album-001","name":"Album","artist":"Artist","artistId":"artist-001","year":2024,"unknown":"ignored"}}}"#
        }
        "getAlbum" => {
            r#"{"album":{"id":"album-001","name":"Album","artist":"Artist","artistId":"artist-001","songCount":1,"song":{"id":"song-001","title":"Track","album":"Album","artist":"Artist","albumId":"album-001","artistId":"artist-001","track":7,"discNumber":2,"year":2024,"genre":"Jazz","duration":182,"size":987654,"suffix":"flac","contentType":"audio/flac","coverArt":"cover-a","bitRate":900,"unknown":true}}}"#
        }
        "getSong" if special_id == "song-old" => {
            r#"{"song":{"id":"song-old","title":"Old Track"}}"#
        }
        "getSong" => {
            r#"{"song":{"id":"opaque-song-009","title":"Track","album":"Album","artist":"Artist","albumId":"opaque-album-008","artistId":"opaque-artist-007","track":7,"discNumber":2,"year":2024,"genre":"Jazz","duration":182,"size":987654,"suffix":"flac","contentType":"audio/flac","coverArt":"opaque-cover-006","bitRate":900,"unknown":true}}"#
        }
        "search3" => {
            r#"{"searchResult3":{"artist":{"id":"artist-001","name":"Artist"},"album":{"id":"album-001","name":"Album"},"song":{"id":"song-001","title":"Track"}}}"#
        }
        "getPlaylists" => {
            r#"{"playlists":{"playlist":{"id":"playlist-001","name":"Mix","songCount":1,"owner":"alice","public":false,"unknown":1}}}"#
        }
        "getPlaylist" => {
            r#"{"playlist":{"id":"playlist-001","name":"Mix","songCount":1,"entry":{"id":"song-001","title":"Track"}}}"#
        }
        _ => r#"{}"#,
    };
    let body = if endpoint == "getSong" && special_id == "secret-id" {
        r#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":70,"message":"not found secret-password"}}}"#.to_owned()
    } else {
        let mut response =
            serde_json::json!({"subsonic-response":{"status":"ok","version":"1.16.1"}});
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
        serde_json::to_string(&response).unwrap()
    };
    (StatusCode::OK, body).into_response()
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
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .expect("request timed out")
}

#[tokio::test]
async fn artists_browse() {
    let server = fixture().await;
    let result = bounded(client(server.url.clone()).artists()).await.unwrap();
    assert_eq!(result[0].id, "artist-001");
    assert_eq!(server.requests.lock().unwrap()[0].0, "getArtists");
}

#[tokio::test]
async fn artist_browse() {
    let server = fixture().await;
    let result = bounded(client(server.url.clone()).artist("artist-001"))
        .await
        .unwrap();
    assert_eq!(result.albums[0].id, "album-001");
    assert_eq!(server.requests.lock().unwrap()[0].1["id"], "artist-001");
}

#[tokio::test]
async fn album_browse() {
    let server = fixture().await;
    let result = bounded(client(server.url.clone()).album("album-001"))
        .await
        .unwrap();
    assert_eq!(result.songs[0].id, "song-001");
    assert_eq!(result.songs[0].disc, Some(2));
}

#[tokio::test]
async fn song_browse() {
    let server = fixture().await;
    let result = bounded(client(server.url.clone()).song("opaque-song-009"))
        .await
        .unwrap();
    assert_eq!(result.album_id.as_deref(), Some("opaque-album-008"));
    assert_eq!(result.size, Some(987654));
}

#[tokio::test]
async fn search_browse() {
    let server = fixture().await;
    let result = bounded(client(server.url.clone()).search(
        "blue note",
        SearchLimits {
            artists: 4,
            albums: 5,
            songs: 6,
        },
    ))
    .await
    .unwrap();
    assert_eq!(result.artists[0].id, "artist-001");
    let request = &server.requests.lock().unwrap()[0].1;
    assert_eq!(request["query"], "blue note");
    assert_eq!(request["artistCount"], "4");
    assert_eq!(request["albumCount"], "5");
    assert_eq!(request["songCount"], "6");
}

#[tokio::test]
async fn playlists_browse() {
    let server = fixture().await;
    let result = bounded(client(server.url.clone()).playlists())
        .await
        .unwrap();
    assert_eq!(result[0].id, "playlist-001");
}

#[tokio::test]
async fn playlist_browse() {
    let server = fixture().await;
    let result = bounded(client(server.url.clone()).playlist("playlist-001"))
        .await
        .unwrap();
    assert_eq!(result.songs[0].id, "song-001");
}

#[tokio::test]
async fn stream_url_auth_and_options() {
    let server = fixture().await;
    bounded(async {
        let url = client(server.url.clone()).stream_url(
            "song-001",
            StreamOptions {
                format: Some("raw".into()),
                max_bit_rate: Some(320),
            },
        );
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        for name in ["t", "s", "u", "v", "c"] {
            assert!(query.contains_key(name), "missing {name}");
        }
        assert!(!query.contains_key("p"));
        assert_eq!(query["id"], "song-001");
        assert_eq!(query["format"], "raw");
        assert_eq!(query["maxBitRate"], "320");
        assert!(url.path().ends_with("/rest/stream.view"));
    })
    .await;
}

#[tokio::test]
async fn cover_art_url_auth_and_size() {
    let server = fixture().await;
    bounded(async {
        let url = client(server.url.clone()).cover_art_url("cover-opaque", Some(512));
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["id"], "cover-opaque");
        assert_eq!(query["size"], "512");
        assert!(query.contains_key("t") && query.contains_key("s"));
        assert!(!query.contains_key("p"));
    })
    .await;
}

#[tokio::test]
async fn missing_optional_fields_are_none_and_error_secrets_are_redacted() {
    let server = fixture().await;
    let song = bounded(client(server.url.clone()).song("song-old"))
        .await
        .unwrap();
    assert_eq!(song.album, None);
    assert_eq!(song.bit_rate, None);

    let error = bounded(client(server.url.clone()).song("secret-id"))
        .await
        .unwrap_err();
    assert_eq!(
        error,
        SubsonicError::Server {
            kind: SubsonicServerError::NotFound,
            message: "not found [REDACTED]".into()
        }
    );
    assert!(!format!("{error:?} {error}").contains("secret-password"));
}
