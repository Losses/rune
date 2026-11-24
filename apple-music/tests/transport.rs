//! Authentication, retries and error mapping against the fixture server.

use tokio::net;
use tokio::time;

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use apple_music::{AppleMusicClient, Error, RetryPolicy, StaticTokens};
use common::{FixtureServer, Reply, TestTokens, client};

#[tokio::test]
async fn user_endpoint_sends_both_tokens() {
    let server = FixtureServer::start().await;
    server.on(
        "GET",
        "/v1/me/storefront",
        Reply::fixture("storefront.json"),
    );
    let c = client(&server, TestTokens::new());

    let storefront = c.user_storefront().await.unwrap();
    assert_eq!(storefront.id, "us");

    let seen = server.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer dev-0"));
    assert_eq!(seen[0].user_token.as_deref(), Some("user-0"));
}

#[tokio::test]
async fn catalog_endpoint_sends_only_the_developer_token() {
    let server = FixtureServer::start().await;
    server.on(
        "GET",
        "/v1/catalog/us/songs",
        Reply::fixture("catalog_songs.json"),
    );
    let c = client(&server, TestTokens::without_user());

    c.catalog_songs(&["617154366".to_owned()]).await.unwrap();

    let seen = server.seen();
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer dev-0"));
    assert_eq!(seen[0].user_token, None);
}

#[tokio::test]
async fn user_endpoint_without_user_token_fails_before_sending() {
    let server = FixtureServer::start().await;
    let c = client(&server, TestTokens::without_user());

    let err = c.user_storefront().await.unwrap_err();
    assert!(matches!(err, Error::MissingUserToken), "{err:?}");
    assert!(server.seen().is_empty());
}

#[tokio::test]
async fn unauthorized_refreshes_user_token_then_succeeds() {
    let server = FixtureServer::start().await;
    server
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::fixture_status(401, "error_unauthorized.json"),
        )
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::fixture("storefront.json"),
        );
    let tokens = TestTokens::new();
    let c = client(&server, tokens.clone());

    c.user_storefront().await.unwrap();

    let seen = server.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].user_token.as_deref(), Some("user-1"));
    assert_eq!(seen[1].authorization.as_deref(), Some("Bearer dev-0"));
    assert_eq!(tokens.user_refreshes(), 1);
    assert_eq!(tokens.dev_refreshes(), 0);
}

#[tokio::test]
async fn unauthorized_twice_refreshes_developer_token_then_succeeds() {
    let server = FixtureServer::start().await;
    server
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::fixture_status(401, "error_unauthorized.json"),
        )
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::fixture_status(401, "error_unauthorized.json"),
        )
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::fixture("storefront.json"),
        );
    let tokens = TestTokens::new();
    let c = client(&server, tokens.clone());

    c.user_storefront().await.unwrap();

    let seen = server.seen();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[2].authorization.as_deref(), Some("Bearer dev-1"));
    assert_eq!(tokens.user_refreshes(), 1);
    assert_eq!(tokens.dev_refreshes(), 1);
}

#[tokio::test]
async fn unauthorized_forever_gives_up_after_one_refresh_of_each() {
    let server = FixtureServer::start().await;
    server.on(
        "GET",
        "/v1/me/storefront",
        Reply::fixture_status(401, "error_unauthorized.json"),
    );
    let tokens = TestTokens::new();
    let c = client(&server, tokens.clone());

    let err = c.user_storefront().await.unwrap_err();
    assert!(matches!(err, Error::Unauthorized), "{err:?}");
    assert_eq!(server.seen().len(), 3);
    assert_eq!(tokens.user_refreshes(), 1);
    assert_eq!(tokens.dev_refreshes(), 1);
}

#[tokio::test]
async fn catalog_unauthorized_refreshes_only_the_developer_token() {
    let server = FixtureServer::start().await;
    server.on(
        "GET",
        "/v1/catalog/us/songs",
        Reply::fixture_status(401, "error_unauthorized.json"),
    );
    let tokens = TestTokens::new();
    let c = client(&server, tokens.clone());

    let err = c.catalog_songs(&["1".to_owned()]).await.unwrap_err();
    assert!(matches!(err, Error::Unauthorized), "{err:?}");
    assert_eq!(server.seen().len(), 2);
    assert_eq!(tokens.user_refreshes(), 0);
    assert_eq!(tokens.dev_refreshes(), 1);
}

#[tokio::test]
async fn forbidden_maps_to_forbidden() {
    let server = FixtureServer::start().await;
    server.on(
        "GET",
        "/v1/me/storefront",
        Reply::fixture_status(403, "error_forbidden.json"),
    );
    let tokens = TestTokens::new();
    let c = client(&server, tokens.clone());

    let err = c.user_storefront().await.unwrap_err();
    assert!(matches!(err, Error::Forbidden), "{err:?}");
    // One user token refresh is tried: Apple answers 403 for an expired user token.
    assert_eq!(server.seen().len(), 2);
}

#[tokio::test]
async fn redirects_are_not_followed_with_the_tokens() {
    let server = FixtureServer::start().await;
    server
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::status(302).header("location", "/elsewhere"),
        )
        .on("GET", "/elsewhere", Reply::fixture("storefront.json"));
    let c = client(&server, TestTokens::new());

    let err = c.user_storefront().await.unwrap_err();
    assert!(matches!(err, Error::Api { status: 302, .. }), "{err:?}");
    assert!(server.seen_for("GET", "/elsewhere").is_empty());
}

#[tokio::test]
async fn not_found_with_client_not_found_body_is_client_not_registered() {
    let server = FixtureServer::start().await;
    server.on(
        "GET",
        "/v1/me/storefront",
        Reply::fixture_status(404, "error_client_not_found.json"),
    );
    let c = client(&server, TestTokens::new());

    let err = c.user_storefront().await.unwrap_err();
    assert!(matches!(err, Error::ClientNotRegistered), "{err:?}");
    assert_eq!(server.seen().len(), 1, "404 must not be retried");
}

#[tokio::test]
async fn other_not_found_is_not_found() {
    let server = FixtureServer::start().await;
    server.on(
        "GET",
        "/v1/me/library/playlists/p.missing/tracks",
        Reply::fixture_status(404, "error_not_found.json"),
    );
    let c = client(&server, TestTokens::new());

    use futures_util::TryStreamExt;
    let err = c
        .library_playlist_tracks("p.missing")
        .try_collect::<Vec<_>>()
        .await
        .unwrap_err();
    match err {
        Error::NotFound(message) => assert!(message.contains("p.missing"), "{message}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn rate_limit_honours_retry_after_then_succeeds() {
    let server = FixtureServer::start().await;
    server
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::status(429).header("Retry-After", "1"),
        )
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::fixture("storefront.json"),
        );
    let c = client(&server, TestTokens::new());

    let started = Instant::now();
    c.user_storefront().await.unwrap();

    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(server.seen().len(), 2);
}

#[tokio::test]
async fn rate_limit_without_retry_after_backs_off_and_gives_up() {
    let server = FixtureServer::start().await;
    server.on("GET", "/v1/me/storefront", Reply::status(429));
    let c = client(&server, TestTokens::new());

    let err = c.user_storefront().await.unwrap_err();
    assert!(
        matches!(err, Error::RateLimited { retry_after: None }),
        "{err:?}"
    );
    assert_eq!(server.seen().len(), 5);
}

#[tokio::test]
async fn server_errors_are_retried_five_times_then_reported() {
    let server = FixtureServer::start().await;
    server.on(
        "GET",
        "/v1/me/storefront",
        Reply::fixture_status(503, "error_server.json"),
    );
    let c = client(&server, TestTokens::new());

    let err = c.user_storefront().await.unwrap_err();
    match err {
        Error::Server { status, errors } => {
            assert_eq!(status, 503);
            assert_eq!(errors[0].title.as_deref(), Some("Internal Server Error"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(server.seen().len(), 5);
}

#[tokio::test]
async fn server_error_then_success_recovers() {
    let server = FixtureServer::start().await;
    server
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::fixture_status(500, "error_server.json"),
        )
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::fixture("storefront.json"),
        );
    let c = client(&server, TestTokens::new());

    assert_eq!(c.user_storefront().await.unwrap().id, "us");
}

#[tokio::test]
async fn backoff_delays_grow_and_stay_capped() {
    let server = FixtureServer::start().await;
    server.on("GET", "/v1/me/storefront", Reply::status(500));
    let c = AppleMusicClient::builder(TestTokens::new())
        .base_url(server.base_url.clone())
        .retry_policy(RetryPolicy {
            max_attempts: 4,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(200),
            max_retry_after: Duration::from_secs(1),
        })
        .build();

    let started = Instant::now();
    let _ = c.user_storefront().await;
    // Three waits: 100, 200 and 200 ms, each with jitter between 50% and 100%.
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(250), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(1500), "{elapsed:?}");
}

#[tokio::test]
async fn concurrency_is_capped() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::routing::get;

    let current = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let app = {
        let (current, peak) = (current.clone(), peak.clone());
        axum::Router::new().route(
            "/v1/catalog/us/songs",
            get(move || {
                let (current, peak) = (current.clone(), peak.clone());
                async move {
                    let now = current.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    time::sleep(Duration::from_millis(50)).await;
                    current.fetch_sub(1, Ordering::SeqCst);
                    r#"{"data":[]}"#
                }
            }),
        )
    };
    let listener = net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let c = AppleMusicClient::builder(Arc::new(StaticTokens::new("dev", None)))
        .base_url(base)
        .storefront("us")
        .max_concurrent_requests(2)
        .build();

    let tasks: Vec<_> = (0..8)
        .map(|i| {
            let c = c.clone();
            tokio::spawn(async move { c.catalog_songs(&[i.to_string()]).await.unwrap() })
        })
        .collect();
    for t in tasks {
        t.await.unwrap();
    }
    assert_eq!(peak.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn tokens_that_cannot_refresh_are_sent_once_more_after_a_rejection() {
    let server = FixtureServer::start().await;
    server.on(
        "GET",
        "/v1/me/storefront",
        Reply::fixture_status(401, "error_unauthorized.json"),
    );
    server.on(
        "GET",
        "/v1/catalog/us/songs",
        Reply::fixture_status(403, "error_forbidden.json"),
    );
    let tokens = Arc::new(StaticTokens::new("dev", Some("user".to_owned())));
    let c = AppleMusicClient::builder(tokens)
        .base_url(server.base_url.clone())
        .storefront("us")
        .retry_policy(common::fast_retry())
        .build();

    let err = c.user_storefront().await.unwrap_err();
    assert!(matches!(err, Error::Unauthorized), "{err:?}");
    assert_eq!(server.seen_for("GET", "/v1/me/storefront").len(), 2);

    let err = c.catalog_songs(&["1".to_owned()]).await.unwrap_err();
    assert!(
        matches!(err, Error::Unauthorized | Error::Forbidden),
        "{err:?}"
    );
    // A 403 on a catalog read has no token of the user's to refresh, so it ends at once.
    assert_eq!(server.seen_for("GET", "/v1/catalog/us/songs").len(), 1);
}

#[tokio::test]
async fn a_retry_after_beyond_the_cap_is_reported_at_once() {
    let server = FixtureServer::start().await;
    server.on(
        "GET",
        "/v1/me/storefront",
        Reply::status(429).header("Retry-After", "3600"),
    );
    let c = client(&server, TestTokens::new());

    let started = Instant::now();
    let err = c.user_storefront().await.unwrap_err();

    match err {
        Error::RateLimited { retry_after } => {
            assert_eq!(retry_after, Some(Duration::from_secs(3600)));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(server.seen().len(), 1);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn a_retry_after_of_zero_still_waits_the_base_delay() {
    let server = FixtureServer::start().await;
    server.on(
        "GET",
        "/v1/me/storefront",
        Reply::status(429).header("Retry-After", "0"),
    );
    let c = AppleMusicClient::builder(TestTokens::new())
        .base_url(server.base_url.clone())
        .retry_policy(RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(100),
            max_retry_after: Duration::from_secs(1),
        })
        .build();

    let started = Instant::now();
    let _ = c.user_storefront().await;

    // Three attempts, two waits of at least the base delay.
    assert_eq!(server.seen().len(), 3);
    assert!(started.elapsed() >= Duration::from_millis(200));
}

#[tokio::test]
async fn a_transport_error_does_not_carry_the_url() {
    let listener = net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let c = AppleMusicClient::builder(TestTokens::new())
        .base_url(base)
        .storefront("us")
        .retry_policy(RetryPolicy {
            max_attempts: 1,
            ..common::fast_retry()
        })
        .build();

    let err = c
        .search_catalog(
            "secret term",
            &[apple_music::models::CatalogSearchType::Songs],
            5,
            0,
        )
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Http(_)), "{err:?}");
    let text = format!("{err} {err:?}");
    assert!(!text.contains("secret"), "{text}");
    assert!(!text.contains("search"), "{text}");
}

#[tokio::test]
async fn a_next_link_that_leaves_the_api_is_not_followed() {
    let server = FixtureServer::start().await;
    server.on(
        "GET",
        "/v1/me/library/playlists",
        Reply::json(serde_json::json!({
            "data": [{"id": "p.1", "type": "library-playlists"}],
            "next": "@elsewhere.invalid/v1/me/library/playlists?offset=1"
        })),
    );
    let c = client(&server, TestTokens::new());

    let result: Result<Vec<_>, _> =
        futures_util::TryStreamExt::try_collect(c.library_playlists()).await;

    assert!(result.is_err(), "{result:?}");
    assert_eq!(server.seen().len(), 1);
}

#[tokio::test]
async fn a_one_off_rejection_with_tokens_that_cannot_refresh_is_absorbed() {
    let server = FixtureServer::start().await;
    server
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::fixture_status(401, "error_unauthorized.json"),
        )
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::fixture("storefront.json"),
        );
    let tokens = Arc::new(StaticTokens::new("dev", Some("user".to_owned())));
    let c = AppleMusicClient::builder(tokens)
        .base_url(server.base_url.clone())
        .retry_policy(common::fast_retry())
        .build();

    assert_eq!(c.user_storefront().await.unwrap().id, "us");
    assert_eq!(server.seen().len(), 2);
}

#[tokio::test]
async fn tokens_are_not_sent_to_a_plain_http_host_beyond_this_machine() {
    let server = FixtureServer::start().await;
    server.on(
        "GET",
        "/v1/me/storefront",
        Reply::fixture("storefront.json"),
    );
    // The fixture server answers on a loopback address, which is allowed.
    let remote = server.base_url.replace("127.0.0.1", "example.invalid");
    for base in [
        remote,
        "http://192.0.2.1".to_owned(),
        "ftp://localhost".to_owned(),
    ] {
        let c = AppleMusicClient::builder(TestTokens::new())
            .base_url(base)
            .storefront("us")
            .build();
        let error = c.user_storefront().await.unwrap_err();
        assert!(matches!(error, Error::InsecureBaseUrl), "{error:?}");
    }
    assert!(server.seen().is_empty());
}
