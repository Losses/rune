use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use subsonic::{SubsonicClient, SubsonicCredentials, SubsonicError, SubsonicServerError};
use url::Url;

mod support;

type RequestRecord = (String, HashMap<String, String>);
type RequestLog = Arc<Mutex<Vec<RequestRecord>>>;

#[derive(Clone)]
struct FixtureConfig {
    open: bool,
    jukebox: bool,
    error: Option<i64>,
    malformed: bool,
    http_status: Option<u16>,
}

#[derive(Clone, Default)]
struct FixtureState {
    config: Option<FixtureConfig>,
    requests: RequestLog,
}

struct Fixture {
    url: Url,
    requests: RequestLog,
    _server: support::TestServer,
}

async fn fixture(config: FixtureConfig) -> Fixture {
    let state = FixtureState {
        config: Some(config),
        requests: Arc::new(Mutex::new(Vec::new())),
    };
    let requests = state.requests.clone();
    let app = Router::new()
        .route("/rest/{*endpoint}", get(handle))
        .with_state(state);
    let server = support::serve(app).await;
    Fixture {
        url: server.url.clone(),
        requests,
        _server: server,
    }
}

async fn handle(
    State(state): State<FixtureState>,
    Path(endpoint): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let endpoint = endpoint.trim_end_matches(".view").to_owned();
    state
        .requests
        .lock()
        .unwrap()
        .push((endpoint.clone(), query));
    let config = state.config.unwrap();
    if let Some(status) = config.http_status {
        return (StatusCode::from_u16(status).unwrap(), "server HTTP failure").into_response();
    }
    if config.malformed {
        return (StatusCode::OK, "not json").into_response();
    }
    if let Some(code) = config.error {
        return (StatusCode::OK, format!(r#"{{"subsonic-response":{{"status":"failed","version":"1.16.1","error":{{"code":{code},"message":"server says no"}}}}}}"#)).into_response();
    }
    let response = match endpoint.as_str() {
        "ping" => format!(r#"{{"subsonic-response":{{"status":"ok","version":"1.16.1","type":"Fake","serverVersion":"2.0","openSubsonic":{}}}}}"#, config.open),
        "getOpenSubsonicExtensions" => r#"{"subsonic-response":{"status":"ok","openSubsonicExtensions":[{"name":"apiKeyAuthentication","versions":[1]}]}}"#.into(),
        "tokenInfo" => r#"{"subsonic-response":{"status":"ok","tokenInfo":{"username":"alice"}}}"#.into(),
        "getUser" => format!(r#"{{"subsonic-response":{{"status":"ok","user":{{"username":"alice","streamRole":true,"downloadRole":true,"jukeboxRole":{}}}}}}}"#, config.jukebox),
        _ => r#"{"subsonic-response":{"status":"ok"}}"#.into(),
    };
    (StatusCode::OK, response).into_response()
}

fn config(open: bool, jukebox: bool) -> FixtureConfig {
    FixtureConfig {
        open,
        jukebox,
        error: None,
        malformed: false,
        http_status: None,
    }
}

fn password() -> SubsonicCredentials {
    SubsonicCredentials::Password {
        user: "alice".into(),
        password: "secret-password".into(),
    }
}

#[tokio::test]
async fn password_auth_uses_random_token_parameters_and_probe_orders_calls() {
    let fixture = fixture(config(true, true)).await;
    let client = SubsonicClient::new(fixture.url.clone(), password(), reqwest::Client::new());
    let result = client.probe().await.unwrap();
    assert!(result.roles.jukebox_role);
    let requests = fixture.requests.lock().unwrap().clone();
    assert_eq!(
        requests
            .iter()
            .map(|(endpoint, _)| endpoint.as_str())
            .collect::<Vec<_>>(),
        ["ping", "getOpenSubsonicExtensions", "getUser"]
    );
    for (_, query) in &requests {
        assert_eq!(query["u"], "alice");
        assert_eq!(query["c"], "rune");
        assert_eq!(query["v"], "1.16.1");
        assert_eq!(query["f"], "json");
        let expected = format!(
            "{:x}",
            md5::compute(format!("secret-password{}", query["s"]))
        );
        assert_eq!(query["t"], expected);
    }
}

#[tokio::test]
async fn classic_probe_skips_extensions() {
    let fixture = fixture(config(false, false)).await;
    let result = SubsonicClient::new(fixture.url.clone(), password(), reqwest::Client::new())
        .probe()
        .await
        .unwrap();
    assert!(!result.open_subsonic);
    assert!(result.extensions.is_empty());
    assert_eq!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["ping", "getUser"]
    );
}

#[tokio::test]
async fn api_key_auth_resolves_user_through_token_info() {
    let fixture = fixture(config(true, true)).await;
    let credentials = SubsonicCredentials::ApiKey {
        key: "secret-key".into(),
    };
    let result = SubsonicClient::new(fixture.url.clone(), credentials, reqwest::Client::new())
        .probe()
        .await
        .unwrap();
    assert!(result.roles.jukebox_role);
    let requests = fixture.requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .map(|(endpoint, _)| endpoint.as_str())
            .collect::<Vec<_>>(),
        ["ping", "getOpenSubsonicExtensions", "tokenInfo", "getUser"]
    );
    for (_, query) in requests.iter() {
        assert_eq!(query["apiKey"], "secret-key");
        for forbidden in ["u", "p", "t", "s"] {
            assert!(!query.contains_key(forbidden));
        }
    }
}

#[tokio::test]
async fn every_subsonic_error_code_is_typed_and_malformed_is_reported() {
    let cases = [
        (0, SubsonicServerError::Generic),
        (10, SubsonicServerError::MissingParameter),
        (20, SubsonicServerError::ClientTooOld),
        (30, SubsonicServerError::ServerTooOld),
        (40, SubsonicServerError::WrongCredentials),
        (41, SubsonicServerError::TokenAuthenticationNotSupported),
        (42, SubsonicServerError::AuthenticationMechanismNotSupported),
        (43, SubsonicServerError::ConflictingAuthentication),
        (44, SubsonicServerError::InvalidApiKey),
        (50, SubsonicServerError::NotAuthorized),
        (70, SubsonicServerError::NotFound),
    ];
    for (code, expected) in cases {
        let mut cfg = config(false, false);
        cfg.error = Some(code);
        let fixture = fixture(cfg).await;
        let error = SubsonicClient::new(fixture.url.clone(), password(), reqwest::Client::new())
            .probe()
            .await
            .unwrap_err();
        assert_eq!(
            error,
            SubsonicError::Server {
                kind: expected,
                message: "server says no".into()
            }
        );
    }
    let mut cfg = config(false, false);
    cfg.malformed = true;
    let malformed_fixture = fixture(cfg).await;
    assert!(matches!(
        SubsonicClient::new(
            malformed_fixture.url.clone(),
            password(),
            reqwest::Client::new()
        )
        .probe()
        .await,
        Err(SubsonicError::Malformed(_))
    ));
    let mut cfg = config(false, false);
    cfg.http_status = Some(503);
    let fixture = fixture(cfg).await;
    assert_eq!(
        SubsonicClient::new(fixture.url.clone(), password(), reqwest::Client::new())
            .probe()
            .await
            .unwrap_err(),
        SubsonicError::HttpStatus(StatusCode::SERVICE_UNAVAILABLE)
    );
}

#[test]
fn credentials_debug_redacts_secrets() {
    let password_debug = format!("{:?}", password());
    let key_debug = format!(
        "{:?}",
        SubsonicCredentials::ApiKey {
            key: "secret-key".into()
        }
    );
    assert!(!password_debug.contains("secret-password"));
    assert!(!key_debug.contains("secret-key"));
}
