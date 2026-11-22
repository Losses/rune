use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use reqwest::Client;
use subsonic::{SubsonicClient, SubsonicCredentials, SubsonicError, SubsonicServerError};
use tokio::net::TcpListener;
use url::Url;

mod support;

type CapturedQueries = Arc<Mutex<Vec<HashMap<String, String>>>>;
type ResponseBodyFn = Arc<dyn Fn(&HashMap<String, String>) -> String + Send + Sync>;

async fn serve_response<F>(make_body: F) -> (support::TestServer, CapturedQueries)
where
    F: Fn(&HashMap<String, String>) -> String + Send + Sync + 'static,
{
    let queries: CapturedQueries = Arc::new(Mutex::new(Vec::new()));
    let make_body: ResponseBodyFn = Arc::new(make_body);
    let app = Router::new()
        .route(
            "/rest/{*endpoint}",
            get(
                move |State((queries, make_body)): State<(CapturedQueries, ResponseBodyFn)>,
                      Path(_endpoint): Path<String>,
                      Query(query): Query<HashMap<String, String>>| async move {
                    queries.lock().unwrap().push(query.clone());
                    (StatusCode::OK, make_body(&query)).into_response()
                },
            ),
        )
        .with_state((queries.clone(), make_body));
    let server = support::serve(app).await;
    (server, queries)
}

#[tokio::test]
async fn malformed_envelope_redacts_api_key_in_debug_and_display() {
    let secret = "synthetic-secret-api-key";
    let (server, _) = serve_response(move |_| {
        format!(
            r#"{{"subsonic-response":{{"status":"failed","error":{{"code":"{secret}","message":"rejected"}}}}}}"#
        )
    })
    .await;
    let client = SubsonicClient::new(
        server.url.clone(),
        SubsonicCredentials::ApiKey { key: secret.into() },
        Client::new(),
    );

    let error = client
        .scrobble("synthetic-track", None, false)
        .await
        .unwrap_err();
    assert!(matches!(error, SubsonicError::Malformed(_)));
    let debug = format!("{error:?}");
    let display = error.to_string();
    assert!(!debug.contains(secret), "Debug leaked secret: {debug}");
    assert!(
        !display.contains(secret),
        "Display leaked secret: {display}"
    );
    assert!(display.contains("[REDACTED]"));
}

#[tokio::test]
async fn malformed_envelope_redacts_password_token_and_salt_in_debug_and_display() {
    let password = "synthetic-secret-password";
    let (server, queries) = serve_response(move |query| {
        let token = &query["t"];
        let salt = &query["s"];
        format!(
            r#"{{"subsonic-response":{{"status":"failed","error":{{"code":"{password}|{token}|{salt}","message":"rejected"}}}}}}"#
        )
    })
    .await;
    let client = SubsonicClient::new(
        server.url.clone(),
        SubsonicCredentials::Password {
            user: "alice".into(),
            password: password.into(),
        },
        Client::new(),
    );

    let error = client
        .scrobble("synthetic-track", None, false)
        .await
        .unwrap_err();
    assert!(matches!(error, SubsonicError::Malformed(_)));
    let captured = queries.lock().unwrap();
    let token = &captured[0]["t"];
    let salt = &captured[0]["s"];
    let debug = format!("{error:?}");
    let display = error.to_string();
    for (label, secret) in [
        ("password", password),
        ("token", token.as_str()),
        ("salt", salt.as_str()),
    ] {
        assert!(!debug.contains(secret), "Debug leaked {label}: {debug}");
        assert!(
            !display.contains(secret),
            "Display leaked {label}: {display}"
        );
    }
    assert!(display.contains("[REDACTED]"));
}

#[tokio::test]
async fn malformed_payload_redacts_api_key_and_password_secrets_in_debug_and_display() {
    let api_key = "synthetic-secret-api-key";
    let (api_server, _) = serve_response(move |_| {
        format!(
            r#"{{"subsonic-response":{{"status":"ok","song":{{"id":"song-1","title":"Song","track":"{api_key}"}}}}}}"#
        )
    })
    .await;
    let api_client = SubsonicClient::new(
        api_server.url.clone(),
        SubsonicCredentials::ApiKey {
            key: api_key.into(),
        },
        Client::new(),
    );
    let api_error = api_client.song("song-1").await.unwrap_err();
    assert!(matches!(api_error, SubsonicError::Malformed(_)));
    let api_debug = format!("{api_error:?}");
    let api_display = api_error.to_string();
    assert!(
        !api_debug.contains(api_key),
        "Debug leaked API key: {api_debug}"
    );
    assert!(
        !api_display.contains(api_key),
        "Display leaked API key: {api_display}"
    );
    assert!(api_display.contains("[REDACTED]"));

    let password = "synthetic-secret-password";
    let (pw_server, queries) = serve_response(move |query| {
        let token = &query["t"];
        let salt = &query["s"];
        format!(
            r#"{{"subsonic-response":{{"status":"ok","song":{{"id":"song-1","title":"Song","track":"{password}|{token}|{salt}"}}}}}}"#
        )
    })
    .await;
    let pw_client = SubsonicClient::new(
        pw_server.url.clone(),
        SubsonicCredentials::Password {
            user: "alice".into(),
            password: password.into(),
        },
        Client::new(),
    );
    let pw_error = pw_client.song("song-1").await.unwrap_err();
    assert!(matches!(pw_error, SubsonicError::Malformed(_)));
    let captured = queries.lock().unwrap();
    let token = &captured[0]["t"];
    let salt = &captured[0]["s"];
    let pw_debug = format!("{pw_error:?}");
    let pw_display = pw_error.to_string();
    for (label, secret) in [
        ("password", password),
        ("token", token.as_str()),
        ("salt", salt.as_str()),
    ] {
        assert!(
            !pw_debug.contains(secret),
            "Debug leaked {label}: {pw_debug}"
        );
        assert!(
            !pw_display.contains(secret),
            "Display leaked {label}: {pw_display}"
        );
    }
    assert!(pw_display.contains("[REDACTED]"));
}

#[tokio::test]
async fn well_formed_server_error_redacts_secrets_and_preserves_message() {
    let api_key = "synthetic-secret-api-key";
    let (server, _) = serve_response(move |_| {
        format!(
            r#"{{"subsonic-response":{{"status":"failed","error":{{"code":40,"message":"rejected {api_key}"}}}}}}"#
        )
    })
    .await;
    let client = SubsonicClient::new(
        server.url.clone(),
        SubsonicCredentials::ApiKey {
            key: api_key.into(),
        },
        Client::new(),
    );

    let error = client
        .scrobble("synthetic-track", None, false)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        SubsonicError::Server {
            kind: SubsonicServerError::WrongCredentials,
            message: "rejected [REDACTED]".into(),
        }
    );
    let debug = format!("{error:?}");
    let display = error.to_string();
    assert!(!debug.contains(api_key));
    assert!(!display.contains(api_key));
    assert!(display.contains("rejected [REDACTED]"));
}

#[tokio::test]
async fn transport_error_strips_url_and_secrets() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let base_url = Url::parse(&format!("http://{address}")).unwrap();
    let http = Client::builder().no_proxy().build().unwrap();

    let api_key = "synthetic-secret-api-key";
    let api_client = SubsonicClient::new(
        base_url.clone(),
        SubsonicCredentials::ApiKey {
            key: api_key.into(),
        },
        http.clone(),
    );
    let api_error = api_client
        .scrobble("synthetic-track", None, false)
        .await
        .unwrap_err();
    assert!(
        matches!(api_error, SubsonicError::Transport(_)),
        "expected Transport, got {api_error:?}"
    );
    let api_combined = format!("{api_error:?} {api_error}");
    assert!(!api_combined.contains(api_key));
    assert!(!api_combined.contains("apiKey"));
    assert!(!api_combined.contains(&address.to_string()));

    let password = "synthetic-secret-password";
    let pw_client = SubsonicClient::new(
        base_url,
        SubsonicCredentials::Password {
            user: "alice".into(),
            password: password.into(),
        },
        http,
    );
    let pw_error = pw_client
        .scrobble("synthetic-track", None, false)
        .await
        .unwrap_err();
    assert!(
        matches!(pw_error, SubsonicError::Transport(_)),
        "expected Transport, got {pw_error:?}"
    );
    let pw_combined = format!("{pw_error:?} {pw_error}");
    assert!(!pw_combined.contains(password));
    assert!(!pw_combined.contains("scrobble.view"));
    assert!(!pw_combined.contains(&address.to_string()));
}

#[tokio::test]
async fn empty_secret_does_not_corrupt_error_message() {
    let (server, _) = serve_response(|_| {
        r#"{"subsonic-response":{"status":"failed","error":{"code":"bad","message":"rejected"}}}"#
            .into()
    })
    .await;
    let client = SubsonicClient::new(
        server.url.clone(),
        SubsonicCredentials::ApiKey { key: String::new() },
        Client::new(),
    );
    let error = client
        .scrobble("synthetic-track", None, false)
        .await
        .unwrap_err();
    assert!(matches!(error, SubsonicError::Malformed(_)));
    assert!(!error.to_string().contains("[REDACTED]"));
}
