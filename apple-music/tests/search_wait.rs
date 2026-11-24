//! How long a search waits for the library and the catalog, what it logs when a part is left
//! out, and the catalog matches it keeps for a repeated search.

use std::mem;
use std::time;

mod common;

use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use apple_music::library::{AppleLibrary, LibraryError, SearchScope, Tokens};
use common::{
    DEVELOPER_TOKEN, FixtureServer, ManualClock, Reply, USER_TOKEN, fast_retry, fixture, keys,
};
use log::{Level, LevelFilter, Log, Metadata, Record};
use serde_json::json;

static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static LOGGER: Once = Once::new();
/// The tests read the one log, so they run one at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Capture;

impl Log for Capture {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Warn
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            WARNINGS.lock().unwrap().push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

/// The warnings about searches logged since the last call.
fn search_warnings() -> Vec<String> {
    LOGGER.call_once(|| {
        log::set_logger(&Capture).unwrap();
        log::set_max_level(LevelFilter::Warn);
    });
    mem::take(&mut *WARNINGS.lock().unwrap())
        .into_iter()
        .filter(|line| line.contains("Apple Music search"))
        .collect()
}

fn serve_search(fixtures: &FixtureServer) {
    fixtures
        .on(
            "GET",
            "/v1/me/library/search",
            fixture("library_search_dawn.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/search",
            fixture("catalog_search_dawn.json"),
        );
}

fn signed_in(fixtures: &FixtureServer, wait: Option<Duration>) -> (AppleLibrary, Arc<ManualClock>) {
    let base_url = fixtures.base_url.clone();
    let clock = ManualClock::new();
    let mut library = AppleLibrary::with_client(|builder| {
        builder
            .base_url(base_url)
            .storefront("us")
            .retry_policy(fast_retry())
    })
    .with_clock(clock.clone());
    if let Some(wait) = wait {
        library = library.with_search_wait(wait);
    }
    library.sign_in(Tokens::new(DEVELOPER_TOKEN, USER_TOKEN).unwrap());
    (library, clock)
}

#[tokio::test]
async fn a_catalog_that_answers_within_a_few_seconds_is_listed() {
    let _serial = SERIAL.lock().await;
    search_warnings();
    let fixtures = FixtureServer::start().await;
    serve_search(&fixtures);
    fixtures.delay("GET", "/v1/catalog/us/search", Duration::from_secs(4));
    let (library, _) = signed_in(&fixtures, None);

    let started = time::Instant::now();
    let found = library
        .search_fresh("dawn", SearchScope::All, false)
        .await
        .unwrap();
    let waited = started.elapsed();
    assert_eq!(keys(&found.songs), ["i.lib1", "catalog:6001"]);
    // It answered once both parts had.
    assert!(
        waited >= Duration::from_secs(4) && waited < Duration::from_secs(6),
        "waited {waited:?}"
    );
    assert!(search_warnings().is_empty());
}

#[tokio::test]
async fn a_part_that_answers_too_late_is_left_out_and_logged() {
    let _serial = SERIAL.lock().await;
    search_warnings();
    let wait = Duration::from_millis(1500);

    // A late catalog leaves the library's matches.
    let fixtures = FixtureServer::start().await;
    serve_search(&fixtures);
    fixtures.delay("GET", "/v1/catalog/us/search", Duration::from_secs(4));
    let (library, _) = signed_in(&fixtures, Some(wait));
    let started = time::Instant::now();
    let found = library
        .search_fresh("dawn", SearchScope::All, false)
        .await
        .unwrap();
    let waited = started.elapsed();
    assert_eq!(keys(&found.songs), ["i.lib1"]);
    assert!(
        waited >= wait && waited < Duration::from_secs(3),
        "waited {waited:?}"
    );
    let warnings = search_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("library's matches only")
            && warnings[0].contains("did not answer within 1.5s"),
        "{}",
        warnings[0]
    );

    // A late library leaves the catalog's matches.
    let fixtures = FixtureServer::start().await;
    serve_search(&fixtures);
    fixtures.delay("GET", "/v1/me/library/search", Duration::from_secs(4));
    let (library, _) = signed_in(&fixtures, Some(wait));
    let found = library
        .search_fresh("dawn", SearchScope::All, false)
        .await
        .unwrap();
    assert_eq!(keys(&found.songs), ["catalog:1001", "catalog:6001"]);
    let warnings = search_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("catalog's matches only"),
        "{}",
        warnings[0]
    );

    // Neither in time fails the search.
    let fixtures = FixtureServer::start().await;
    serve_search(&fixtures);
    fixtures
        .delay("GET", "/v1/me/library/search", Duration::from_secs(4))
        .delay("GET", "/v1/catalog/us/search", Duration::from_secs(4));
    let (library, _) = signed_in(&fixtures, Some(wait));
    let error = library
        .search_fresh("dawn", SearchScope::All, false)
        .await
        .unwrap_err();
    assert!(matches!(error, LibraryError::TimedOut), "{error:?}");
}

#[tokio::test]
async fn a_catalog_match_stays_when_the_library_search_is_left_out_and_the_song_is_kept() {
    let _serial = SERIAL.lock().await;
    search_warnings();
    let fixtures = FixtureServer::start().await;
    serve_search(&fixtures);
    let kept: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string("tests/fixtures/library_search_dawn.json").unwrap(),
    )
    .unwrap();
    let song = kept["results"]["library-songs"]["data"][0].clone();
    fixtures
        .on(
            "GET",
            "/v1/me/library/songs",
            Reply::json(json!({"data": [song]})),
        )
        .on(
            "GET",
            "/v1/me/library/playlists",
            Reply::json(json!({"data": []})),
        );
    let (library, _) = signed_in(&fixtures, Some(Duration::from_millis(1500)));
    assert_eq!(library.songs().await.unwrap().len(), 1);
    fixtures.delay("GET", "/v1/me/library/search", Duration::from_secs(4));

    let found = library
        .search_fresh("dawn", SearchScope::All, false)
        .await
        .unwrap();

    // The library's copy is not among the results, so the catalog's is not left out for it.
    assert_eq!(keys(&found.songs), ["catalog:1001", "catalog:6001"]);
    assert!(found.songs[0].in_library);
}

#[tokio::test]
async fn the_log_of_a_failed_catalog_search_omits_what_apple_says_of_the_term() {
    let _serial = SERIAL.lock().await;
    search_warnings();
    let fixtures = FixtureServer::start().await;
    fixtures.on(
        "GET",
        "/v1/me/library/search",
        fixture("library_search_dawn.json"),
    );
    fixtures.on(
        "GET",
        "/v1/catalog/us/search",
        Reply::json_status(
            500,
            json!({"errors": [{"status": "500", "title": "Bad", "detail": "Invalid term dawn-private"}]}),
        ),
    );
    let (library, _) = signed_in(&fixtures, None);

    library
        .search_fresh("dawn-private", SearchScope::All, false)
        .await
        .unwrap();

    let warnings = search_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("status 500"), "{}", warnings[0]);
    assert!(!warnings[0].contains("private"), "{}", warnings[0]);
}

#[tokio::test]
async fn a_repeated_search_takes_what_the_catalog_found_for_two_minutes_of_a_sign_in() {
    let _serial = SERIAL.lock().await;
    let fixtures = FixtureServer::start().await;
    serve_search(&fixtures);
    let (library, clock) = signed_in(&fixtures, None);
    let catalog_searches = || fixtures.seen_for("GET", "/v1/catalog/us/search").len();

    let first = library
        .search_fresh("Dawn", SearchScope::All, false)
        .await
        .unwrap();
    assert_eq!(keys(&first.songs), ["i.lib1", "catalog:6001"]);
    // The same term in another case or spacing asks the catalog nothing, and the library again.
    let again = library
        .search_fresh("  dawn ", SearchScope::All, false)
        .await
        .unwrap();
    assert_eq!(keys(&again.songs), keys(&first.songs));
    assert_eq!(catalog_searches(), 1);
    assert_eq!(fixtures.seen_for("GET", "/v1/me/library/search").len(), 2);

    // Two minutes on, it is asked again.
    clock.advance(Duration::from_secs(2 * 60));
    library
        .search_fresh("dawn", SearchScope::All, false)
        .await
        .unwrap();
    assert_eq!(catalog_searches(), 2);

    // Another account asks again.
    library.sign_in(Tokens::new(DEVELOPER_TOKEN, "another-user-token").unwrap());
    library
        .search_fresh("dawn", SearchScope::All, false)
        .await
        .unwrap();
    assert_eq!(catalog_searches(), 3);
}
