// The binding against the fixture server of the `apple-music` crate: the library's lists, how
// they are read, kept and read again, and how the sign-in and Apple's failures are told.

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use apple_music::test_fixtures::{FixtureServer, Reply, Seen};
use apple_music::{AppleMusicClient, RetryPolicy, TokenSource};
use axum::Router;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use futures::future;
use playback::player::ServiceTarget;
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

use super::*;
use crate::request_error::{classify, request_failure_cause};
use crate::service::lists::lists_folder;
use crate::service::registry::{Clock, SystemClock};
use crate::service::store::ListStore;
use crate::test_wait::wait_until;
use crate::utils::{Broadcaster, RinfRustSignal};

const DEVELOPER_TOKEN: &str = "developer-token";
const USER_TOKEN: &str = "music-user-token";

const LIBRARY_PAGE_1: &str = "/v1/me/library/songs?limit=100&include=albums,artists";
const LIBRARY_PAGE_2: &str = "/v1/me/library/songs?offset=2&limit=100&include=albums,artists";
const ALBUMS_PAGE: &str = "/v1/me/library/albums?limit=100&include=artists,catalog";

/// Counts what it is told by name.
#[derive(Default)]
struct Counted(Mutex<Vec<String>>);

impl Broadcaster for Counted {
    fn broadcast(&self, signal: &dyn RinfRustSignal) {
        self.0.lock().unwrap().push(signal.name());
    }
}

impl Counted {
    /// How many times the UI was told the library's contents changed.
    fn told(&self) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|name| *name == "LibraryContentChanged")
            .count()
    }
}

/// A clock that moves only when told to.
struct ManualClock {
    start: Instant,
    passed: Mutex<Duration>,
}

impl ManualClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            start: Instant::now(),
            passed: Mutex::default(),
        })
    }

    fn advance(&self, by: Duration) {
        *self.passed.lock().unwrap() += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.start + *self.passed.lock().unwrap()
    }
}

/// Retries quickly, so the tests stay fast.
fn fast_retry() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 5,
        base_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(40),
        max_retry_after: Duration::from_secs(5),
    }
}

fn client_for(fixtures: &FixtureServer, tokens: Arc<dyn TokenSource>) -> AppleMusicClient {
    AppleMusicClient::builder(tokens)
        .base_url(fixtures.base_url.clone())
        .storefront("us")
        .retry_policy(fast_retry())
        .build()
}

fn tokens() -> Tokens {
    Tokens::new(DEVELOPER_TOKEN, USER_TOKEN).unwrap()
}

fn tokens_of(user: &str) -> Tokens {
    Tokens::new(DEVELOPER_TOKEN, user).unwrap()
}

/// A binding of a library kept in a folder, reading the fixture server, with what it told the UI
/// and the clock its lists age by.
struct Bound {
    binding: Arc<AppleMusicBinding>,
    told: Arc<Counted>,
    clock: Arc<ManualClock>,
}

/// A binding of the library `library_id` kept in `dir`, not signed in.
fn bound_in(fixtures: &FixtureServer, dir: &Path, library_id: Uuid) -> Bound {
    let told = Arc::new(Counted::default());
    let clock = ManualClock::new();
    let binding = AppleMusicBinding::new(
        library_id,
        |tokens| client_for(fixtures, tokens),
        dir,
        told.clone(),
        clock.clone(),
    );
    Bound {
        binding,
        told,
        clock,
    }
}

/// A signed-in binding of a library of its own, and the folder it keeps its lists in.
fn signed_in(fixtures: &FixtureServer) -> (TempDir, Bound) {
    let dir = tempfile::tempdir().unwrap();
    let bound = bound_in(fixtures, dir.path(), Uuid::new_v4());
    bound.binding.sign_in(tokens());
    (dir, bound)
}

/// A binding that reads no server.
fn unserved() -> (TempDir, Arc<AppleMusicBinding>, Arc<Counted>) {
    let dir = tempfile::tempdir().unwrap();
    let counted = Arc::new(Counted::default());
    let binding = AppleMusicBinding::new(
        Uuid::new_v4(),
        AppleMusicClient::new,
        dir.path(),
        counted.clone(),
        Arc::new(SystemClock),
    );
    (dir, binding, counted)
}

fn library_song(id: &str, name: &str) -> Value {
    json!({
        "id": id,
        "type": "library-songs",
        "href": format!("/v1/me/library/songs/{id}"),
        "attributes": {
            "name": name,
            "albumName": "Long Album",
            "artistName": "Artist Seven",
            "durationInMillis": 120000,
            "genreNames": ["Jazz"],
            "playParams": {"id": id, "kind": "song", "isLibrary": true}
        }
    })
}

fn id_count(request: &Seen) -> usize {
    request
        .target
        .split("ids=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .split(',')
        .count()
}

/// Reads of library song pages, not lookups by id.
fn library_page_reads(fixtures: &FixtureServer) -> usize {
    fixtures
        .seen_for("GET", "/v1/me/library/songs")
        .iter()
        .filter(|request| !request.target.contains("ids="))
        .count()
}

/// Waits until `condition` holds.
async fn until(condition: impl Fn() -> bool) {
    wait_until("the condition to hold", || async {
        Ok(condition().then_some(()))
    })
    .await
    .unwrap();
}

fn waits_for_sign_in<T: fmt::Debug>(result: Result<T>, binding: &AppleMusicBinding) {
    let error = result.expect_err("a read without a sign-in");
    assert_eq!(
        request_failure_cause(&error),
        Some(
            crate::messages::RequestFailureCause::ServiceSignInRequired {
                record_id: binding.library_id.to_string(),
            }
        ),
        "{error:#}"
    );
}

fn unavailable<T: fmt::Debug>(result: Result<T>) {
    let error = result.expect_err("a read Apple refused");
    assert_eq!(
        classify(&error),
        crate::messages::RequestErrorKind::Unavailable,
        "{error:#}"
    );
}

fn not_found<T: fmt::Debug>(result: Result<T>) {
    let error = result.expect_err("a read of something Apple does not have");
    assert!(
        matches!(
            error.downcast_ref::<ExpectedError>(),
            Some(ExpectedError::NotFound { .. })
        ),
        "{error:#}"
    );
}

/// The library, three joining playlists (one empty), Favorites and a subscribed one.
fn serve_library(fixtures: &FixtureServer) {
    fixtures
        .on_target(
            "GET",
            LIBRARY_PAGE_1,
            Reply::fixture("union_library_songs_page1.json"),
        )
        .on_target(
            "GET",
            LIBRARY_PAGE_2,
            Reply::fixture("union_library_songs_page2.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists",
            Reply::fixture("union_playlists.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists/p.mine1/tracks",
            Reply::fixture("union_playlist_mine_tracks.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists/p.favorites/tracks",
            Reply::fixture("union_playlist_favorites_tracks.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists/p.editorial/tracks",
            Reply::fixture("union_playlist_editorial_tracks.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists/p.empty/tracks",
            Reply::fixture_status(404, "error_not_found.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists/p.public/tracks",
            Reply::fixture("union_playlist_public_tracks.json"),
        );
}

fn serve_albums(fixtures: &FixtureServer) {
    fixtures
        .on(
            "GET",
            "/v1/me/library/recently-added",
            Reply::json(json!({"data": []})),
        )
        .on(
            "GET",
            "/v1/me/library/albums",
            Reply::fixture("browse_albums.json"),
        );
}

const UNION: [&str; 6] = [
    "i.lib1",
    "i.lib2",
    "i.lib3",
    "i.pl1",
    "catalog:3001",
    "catalog:4001",
];

fn tracks(list: &AppleSongs) -> Vec<String> {
    list.entries().map(|(track, _)| track.to_owned()).collect()
}

async fn song_tracks(binding: &AppleMusicBinding) -> Vec<String> {
    tracks(&binding.songs().await.unwrap())
}

fn album_ids(albums: &[LibraryAlbum]) -> Vec<&str> {
    albums.iter().map(|album| album.id.as_str()).collect()
}

// Sign-in ----------------------------------------------------------------

#[tokio::test]
async fn every_read_that_needs_apple_waits_for_the_sign_in_and_asks_nothing() {
    let fixtures = FixtureServer::start().await;
    serve_library(&fixtures);
    serve_albums(&fixtures);
    let dir = tempfile::tempdir().unwrap();
    let Bound { binding, .. } = bound_in(&fixtures, dir.path(), Uuid::new_v4());

    waits_for_sign_in(binding.songs().await, &binding);
    waits_for_sign_in(binding.song("i.lib1").await, &binding);
    waits_for_sign_in(binding.song("catalog:1").await, &binding);
    waits_for_sign_in(binding.albums().await, &binding);
    waits_for_sign_in(binding.read_album_songs("l.alb1").await, &binding);
    waits_for_sign_in(binding.artists().await, &binding);
    waits_for_sign_in(binding.read_artist_albums("r.art1").await, &binding);
    assert!(fixtures.seen().is_empty());
    // A sign-in that cannot be used asks nothing either.
    assert!(Tokens::new(DEVELOPER_TOKEN, "").is_none());
    assert!(Tokens::new("", USER_TOKEN).is_none());
}

#[tokio::test]
async fn a_sign_out_forgets_the_lists_and_a_sign_in_reads_them_again() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on_target(
            "GET",
            LIBRARY_PAGE_1,
            Reply::fixture("union_library_songs_page2.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists",
            Reply::json(json!({"data": []})),
        );
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);
    assert_eq!(binding.songs().await.unwrap().len(), 1);

    binding.sign_out();
    assert!(binding.known_songs().is_none());
    waits_for_sign_in(binding.songs().await, &binding);
    binding.sign_in(tokens());
    assert_eq!(binding.songs().await.unwrap().len(), 1);
    assert_eq!(library_page_reads(&fixtures), 2);
}

#[tokio::test]
async fn a_sign_in_hands_both_tokens_to_its_reads_and_keeps_them_out_of_debug() {
    let fixtures = FixtureServer::start().await;
    fixtures.on(
        "GET",
        "/v1/me/library/artists",
        Reply::fixture("browse_artists_page2.json"),
    );
    let dir = tempfile::tempdir().unwrap();
    let Bound { binding, .. } = bound_in(&fixtures, dir.path(), Uuid::new_v4());
    waits_for_sign_in(binding.artists().await, &binding);

    binding.sign_in(tokens());
    binding.sign_in(tokens());
    binding.artists().await.unwrap();

    let seen = fixtures.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].authorization.as_deref(),
        Some("Bearer developer-token")
    );
    assert_eq!(seen[0].user_token.as_deref(), Some(USER_TOKEN));

    for debug in [format!("{:?}", tokens()), format!("{:?}", binding.tokens)] {
        assert!(!debug.contains(USER_TOKEN), "{debug}");
        assert!(!debug.contains(DEVELOPER_TOKEN), "{debug}");
    }
}

#[tokio::test]
async fn a_sign_in_is_verified_by_reading_the_storefront_with_its_tokens() {
    let fixtures = FixtureServer::start().await;
    fixtures.on(
        "GET",
        "/v1/me/storefront",
        Reply::fixture("storefront.json"),
    );
    let verified = verified_tokens(
        |tokens| client_for(&fixtures, tokens),
        USER_TOKEN,
        Some(DEVELOPER_TOKEN),
    )
    .await;
    assert_eq!(verified.map(|verified| verified == tokens()), Ok(true));
    let seen = fixtures.seen_for("GET", "/v1/me/storefront");
    assert_eq!(seen[0].user_token.as_deref(), Some(USER_TOKEN));

    let rejected = FixtureServer::start().await;
    rejected.on(
        "GET",
        "/v1/me/storefront",
        Reply::fixture_status(401, "error_unauthorized.json"),
    );
    let verify = |developer| {
        verified_tokens(
            |tokens| client_for(&rejected, tokens),
            USER_TOKEN,
            developer,
        )
    };
    assert_eq!(
        verify(Some(DEVELOPER_TOKEN)).await,
        Err(SignInFailure::Rejected)
    );
    assert_eq!(verify(None).await, Err(SignInFailure::NoDeveloperToken));
    assert_eq!(verify(Some("")).await, Err(SignInFailure::NoDeveloperToken));
}

#[tokio::test]
async fn rejected_tokens_read_as_an_expired_sign_in() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/library/artists",
            Reply::fixture_status(401, "error_unauthorized.json"),
        )
        .on(
            "GET",
            "/v1/me/library/albums",
            Reply::fixture_status(403, "error_forbidden.json"),
        )
        .on(
            "GET",
            "/v1/me/library/recently-added",
            Reply::json(json!({"data": []})),
        )
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::fixture_status(401, "error_unauthorized.json"),
        );
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);

    waits_for_sign_in(binding.artists().await, &binding);
    // The tokens cannot be refreshed: the same ones are sent once more, then it ends.
    assert_eq!(fixtures.seen_for("GET", "/v1/me/library/artists").len(), 2);
    assert!(!fixtures.seen_for("GET", "/v1/me/storefront").is_empty());
    waits_for_sign_in(binding.albums().await, &binding);
    // The binding keeps its tokens: the app signs in again by itself.
    assert!(binding.is_signed_in());
}

#[tokio::test]
async fn refused_requests_with_working_tokens_are_not_an_expired_sign_in() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/library/artists",
            Reply::fixture_status(401, "error_unauthorized.json"),
        )
        .on(
            "GET",
            "/v1/me/library/albums",
            Reply::fixture_status(403, "error_forbidden.json"),
        )
        .on(
            "GET",
            "/v1/me/library/recently-added",
            Reply::json(json!({"data": []})),
        )
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::fixture("storefront.json"),
        );
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);

    unavailable(binding.artists().await);
    unavailable(binding.albums().await);
    assert_eq!(fixtures.seen_for("GET", "/v1/me/storefront").len(), 2);
}

#[tokio::test]
async fn new_tokens_keep_the_lists_and_read_them_again_under_the_new_sign_in() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on_target(
            "GET",
            LIBRARY_PAGE_1,
            Reply::fixture("union_library_songs_page2.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists",
            Reply::json(json!({"data": []})),
        );
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);
    assert_eq!(binding.songs().await.unwrap().len(), 1);

    // The same tokens keep everything, and read nothing.
    binding.sign_in(tokens());
    assert_eq!(binding.songs().await.unwrap().len(), 1);
    assert_eq!(library_page_reads(&fixtures), 1);

    // Other tokens may be another account's, which Apple does not say: the list kept is still
    // given, and read again under the new sign-in, which replaces it.
    binding.sign_in(tokens_of("another-user-token"));
    assert_eq!(binding.songs().await.unwrap().len(), 1);
    until(|| library_page_reads(&fixtures) == 2).await;
    let last = fixtures
        .seen_for("GET", "/v1/me/library/songs")
        .pop()
        .unwrap();
    assert_eq!(last.user_token.as_deref(), Some("another-user-token"));
}

// Songs ------------------------------------------------------------------

#[tokio::test]
async fn songs_join_the_library_and_the_users_own_playlists_once() {
    let fixtures = FixtureServer::start().await;
    serve_library(&fixtures);
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);

    let first = binding.songs().await.unwrap();
    assert_eq!(tracks(&first), UNION);
    // Read once; the second page read by its offset.
    assert_eq!(song_tracks(&binding).await, UNION);
    assert_eq!(library_page_reads(&fixtures), 2);
    let pages: Vec<String> = fixtures
        .seen_for("GET", "/v1/me/library/songs")
        .into_iter()
        .map(|request| request.target)
        .filter(|target| !target.contains("ids="))
        .collect();
    assert_eq!(pages, [LIBRARY_PAGE_1, LIBRARY_PAGE_2]);

    // Editorial playlists are not the user's own music: their songs are not read.
    assert!(
        fixtures
            .seen_for("GET", "/v1/me/library/playlists/p.editorial/tracks")
            .is_empty()
    );
    assert!(first.get("catalog:9001").is_none());
    // Favorites and the user's playlists are.
    assert_eq!(
        fixtures
            .seen_for("GET", "/v1/me/library/playlists/p.favorites/tracks")
            .len(),
        1
    );
    assert!(first.get("i.pl1").is_some());

    // A playlist track outside the library is a catalog song, not in the library.
    let loved = first.get("catalog:3001").unwrap();
    assert_eq!(SongRef::Library(loved).title(), "Loved Elsewhere");
    assert!(!is_library_song(loved));
    assert_eq!(loved.store_id(), Some("3001"));
    // A library song keeps its relationships when a playlist lists it too.
    let second = first.get("i.lib2").unwrap();
    assert_eq!(second.album_id(), Some("l.alb1"));
    assert!(is_library_song(second));
    // A catalog song the library has a copy of is that copy, and not listed again.
    assert_eq!(first.library_twin("catalog:2001"), Some("i.pl1"));
    assert!(first.get("catalog:2001").is_none());
    assert_eq!(first.library_twin("catalog:3001"), None);
}

#[tokio::test]
async fn catalog_copies_of_library_songs_are_dropped_by_catalog_id_relationship_or_purchase() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on_target(
            "GET",
            LIBRARY_PAGE_1,
            Reply::fixture("dedup_library_songs.json"),
        )
        .on_target(
            "GET",
            "/v1/me/library/songs?ids=i.up1&include=catalog",
            Reply::fixture("dedup_library_catalog.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists",
            Reply::fixture("library_playlist_favorites.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists/p.favorites/tracks",
            Reply::fixture("dedup_favorites_tracks.json"),
        );
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);

    // i.up1 names its catalog id only in its relationship, i.buy1 only as purchased.
    assert_eq!(
        song_tracks(&binding).await,
        ["i.up1", "i.buy1", "catalog:7006"]
    );
    let lookups: Vec<String> = fixtures
        .seen_for("GET", "/v1/me/library/songs")
        .into_iter()
        .map(|request| request.target)
        .filter(|target| target.contains("ids="))
        .collect();
    assert_eq!(lookups, ["/v1/me/library/songs?ids=i.up1&include=catalog"]);
}

#[tokio::test]
async fn a_large_library_is_read_by_offset_from_its_total() {
    let fixtures = FixtureServer::start().await;
    let page = |from: usize, to: usize, next: Option<usize>| {
        let songs: Vec<Value> = (from..to)
            .map(|n| library_song(&format!("i.s{n:03}"), &format!("Song {n}")))
            .collect();
        let mut body = json!({"data": songs, "meta": {"total": 250}});
        if let Some(next) = next {
            body["next"] = json!(format!("/v1/me/library/songs?offset={next}"));
        }
        Reply::json(body)
    };
    fixtures
        .on_target("GET", LIBRARY_PAGE_1, page(0, 100, Some(100)))
        .on_target(
            "GET",
            "/v1/me/library/songs?offset=100&limit=100&include=albums,artists",
            // Only the total leads to the last page: this one names no next.
            page(100, 200, None),
        )
        .on_target(
            "GET",
            "/v1/me/library/songs?offset=200&limit=100&include=albums,artists",
            page(200, 250, None),
        )
        .on(
            "GET",
            "/v1/me/library/playlists",
            Reply::json(json!({"data": []})),
        );
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);

    let songs = binding.songs().await.unwrap();

    assert_eq!(songs.len(), 250);
    let listed = tracks(&songs);
    assert_eq!(listed[0], "i.s000");
    assert_eq!(listed[100], "i.s100");
    assert_eq!(listed[249], "i.s249");
    let offsets: Vec<String> = fixtures
        .seen_for("GET", "/v1/me/library/songs")
        .into_iter()
        .map(|request| request.target)
        .collect();
    assert_eq!(offsets.len(), 3);
    assert!(offsets.iter().any(|target| target.contains("offset=100")));
    assert!(offsets.iter().any(|target| target.contains("offset=200")));
}

#[tokio::test]
async fn a_playlist_that_cannot_be_read_does_not_fail_the_first_song_list() {
    let fixtures = FixtureServer::start().await;
    serve_library(&fixtures);
    fixtures.on_target(
        "GET",
        "/v1/me/library/playlists/p.mine1/tracks?limit=100",
        Reply::fixture_status(500, "error_server.json"),
    );
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);

    let songs = binding.songs().await.unwrap();

    assert!(
        !fixtures
            .seen_for("GET", "/v1/me/library/playlists/p.mine1/tracks")
            .is_empty()
    );
    // The songs only that playlist held are missing, and the others are listed.
    assert!(
        !songs.songs().is_empty() && songs.len() < 6,
        "{}",
        songs.len()
    );
}

#[tokio::test]
async fn a_read_again_that_cannot_read_a_playlist_keeps_the_complete_song_list() {
    let fixtures = FixtureServer::start().await;
    serve_library(&fixtures);
    let (_dir, Bound { binding, clock, .. }) = signed_in(&fixtures);
    assert_eq!(binding.songs().await.unwrap().len(), 6);

    clock.advance(LONG_LIST_AGE);
    fixtures.on_target(
        "GET",
        "/v1/me/library/playlists/p.mine1/tracks?limit=100",
        Reply::fixture_status(500, "error_server.json"),
    );
    let reads = || {
        fixtures
            .seen_for("GET", "/v1/me/library/playlists/p.mine1/tracks")
            .len()
    };
    let before = reads();
    // The old list answers, and the read in the background fails on the playlist.
    assert_eq!(binding.songs().await.unwrap().len(), 6);
    until(|| reads() > before).await;
    tokio_time::sleep(Duration::from_millis(300)).await;

    // The failed read did not replace the list with one that lacks the playlist's songs.
    assert_eq!(binding.songs().await.unwrap().len(), 6);
}

#[tokio::test]
async fn an_entry_named_by_catalog_id_is_not_listed_beside_a_later_library_copy() {
    let song = |id: &str| {
        json!({
            "id": id,
            "type": "library-songs",
            "attributes": {
                "name": "Twin",
                "playParams": {"id": id, "kind": "song", "catalogId": "1001"}
            }
        })
    };
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/library/songs",
            Reply::json(json!({"data": [song("1001"), song("i.twin")]})),
        )
        .on(
            "GET",
            "/v1/me/library/playlists",
            Reply::json(json!({"data": []})),
        );
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);

    assert_eq!(song_tracks(&binding).await, ["i.twin"]);
}

#[tokio::test]
async fn an_entry_named_by_catalog_id_is_not_listed_beside_a_library_copy_found_by_lookup() {
    let song = |id: &str, catalog: Option<&str>| {
        let mut params = json!({"id": id, "kind": "song"});
        if let Some(catalog) = catalog {
            params["catalogId"] = json!(catalog);
        }
        json!({
            "id": id,
            "type": "library-songs",
            "attributes": {"name": "Twin", "playParams": params}
        })
    };
    let mut copy = song("i.twin", None);
    copy["relationships"] = json!({"catalog": {"data": [{"id": "1001", "type": "songs"}]}});
    let fixtures = FixtureServer::start().await;
    fixtures
        .on_target(
            "GET",
            LIBRARY_PAGE_1,
            Reply::json(json!({"data": [song("1001", Some("1001")), song("i.twin", None)]})),
        )
        .on_target(
            "GET",
            "/v1/me/library/songs?ids=i.twin&include=catalog",
            Reply::json(json!({"data": [copy]})),
        )
        .on(
            "GET",
            "/v1/me/library/playlists",
            Reply::fixture("library_playlist_favorites.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists/p.favorites/tracks",
            Reply::fixture("dedup_favorites_tracks.json"),
        );
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);

    let songs = song_tracks(&binding).await;

    assert!(songs.contains(&"i.twin".to_owned()), "{songs:?}");
    assert!(!songs.contains(&"catalog:1001".to_owned()), "{songs:?}");
}

#[tokio::test]
async fn the_song_list_is_kept_for_ten_minutes_then_given_while_it_is_read_again() {
    let fixtures = FixtureServer::start().await;
    serve_library(&fixtures);
    let (_dir, Bound { binding, clock, .. }) = signed_in(&fixtures);

    assert_eq!(binding.songs().await.unwrap().len(), 6);
    clock.advance(LONG_LIST_AGE - Duration::from_secs(1));
    binding.songs().await.unwrap();
    assert_eq!(library_page_reads(&fixtures), 2);

    // Old, and the library has changed: the old list is given at once, the new one is read in
    // the background.
    clock.advance(Duration::from_secs(1));
    fixtures
        .on_target(
            "GET",
            LIBRARY_PAGE_1,
            Reply::fixture("union_library_songs_page2.json"),
        )
        .delay("GET", "/v1/me/library/songs", Duration::from_millis(200));
    assert_eq!(binding.songs().await.unwrap().len(), 6);
    let (summary, _) = binding.item_metadata("i.lib2", false).await.unwrap();
    assert_eq!(summary.title, "Second Wind");
    assert!(
        fixtures
            .seen_for("GET", "/v1/me/library/songs")
            .iter()
            .all(|request| !request.target.contains("ids=i.lib2"))
    );
    // Asking again meanwhile does not start another read.
    assert_eq!(binding.songs().await.unwrap().len(), 6);
    until(|| library_page_reads(&fixtures) >= 3).await;
    let mut first_song = String::new();
    for _ in 0..200 {
        first_song = song_tracks(&binding).await[0].clone();
        if first_song != "i.lib1" {
            break;
        }
        tokio_time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(first_song, "i.lib3");
    assert_eq!(library_page_reads(&fixtures), 3);

    // The list read again counts its age from its own read.
    clock.advance(LONG_LIST_AGE / 2);
    binding.songs().await.unwrap();
    assert_eq!(library_page_reads(&fixtures), 3);
}

#[tokio::test]
async fn a_song_is_read_from_apple_or_taken_from_the_list() {
    let fixtures = FixtureServer::start().await;
    serve_library(&fixtures);
    fixtures
        .on(
            "GET",
            "/v1/catalog/us/songs",
            Reply::fixture("catalog_songs.json"),
        )
        .on_target(
            "GET",
            "/v1/me/library/songs?ids=i.lib1&include=albums,artists",
            Reply::fixture("library_song_single.json"),
        )
        .on_target(
            "GET",
            "/v1/me/library/songs?ids=i.gone&include=albums,artists",
            Reply::fixture_status(404, "error_not_found.json"),
        );
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);

    let song = binding.song("i.lib1").await.unwrap();
    let read = song.as_ref();
    assert!(matches!(&song, AppleSong::Library(song) if song.id == "i.lib1"));
    assert_eq!(read.title(), "First Light");
    assert_eq!(read.album(), Some("Dawn Album"));
    assert_eq!(read.artist(), Some("Artist One"));
    assert_eq!(read.album_id(), Some("l.alb1"));
    assert_eq!(read.artist_id(), Some("r.art1"));
    assert_eq!(read.track_number(), Some(1));
    assert_eq!(read.disc_number(), Some(1));
    assert_eq!(read.year(), Some(2020));
    assert!((read.duration() - 245.6).abs() < 1e-9);
    assert_eq!(
        read.artwork().and_then(|artwork| artwork_url(artwork, 300)),
        Some("http://artwork.invalid/dawn/300x300bb.jpg".to_owned())
    );
    assert_eq!(
        fixtures.seen_for("GET", "/v1/me/library/songs")[0].target,
        "/v1/me/library/songs?ids=i.lib1&include=albums,artists"
    );

    let catalog = binding.song("catalog:617154366").await.unwrap();
    let read = catalog.as_ref();
    assert_eq!(read.title(), "Get Lucky");
    assert_eq!(read.track_number(), Some(8));
    assert_eq!(read.year(), Some(2013));
    assert!((read.duration() - 369.629).abs() < 1e-9);
    let lookup = &fixtures.seen_for("GET", "/v1/catalog/us/songs")[0];
    assert_eq!(lookup.target, "/v1/catalog/us/songs?ids=617154366");
    // The catalog read needs only the developer token.
    assert_eq!(lookup.user_token, None);

    // Songs of the list need no read.
    binding.songs().await.unwrap();
    let before = fixtures.seen().len();
    assert_eq!(
        binding.song("catalog:3001").await.unwrap().as_ref().title(),
        "Loved Elsewhere"
    );
    assert_eq!(fixtures.seen().len(), before);
    not_found(binding.song("i.gone").await);
    assert!(binding.song("not a track").await.is_err());
}

#[tokio::test]
async fn songs_are_read_by_track_in_batches_of_300_per_kind() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/library/songs",
            // One song too many: what was not asked for stays out.
            Reply::json(json!({"data": [
                library_song("i.t0", "Zero"),
                library_song("i.t300", "Three Hundred"),
                library_song("i.other", "Other"),
            ]})),
        )
        .on(
            "GET",
            "/v1/catalog/us/songs",
            Reply::fixture("catalog_songs.json"),
        );
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);

    let mut asked: Vec<String> = (0..301).map(|i| format!("i.t{i}")).collect();
    asked.extend(["catalog:617154366", "catalog:9999", "i.t0"].map(str::to_owned));
    let found = binding.read_unknown_tracks(asked.clone()).await;
    let mut titles: Vec<(String, String)> = found
        .iter()
        .map(|(track, song)| (track.clone(), song.as_ref().title().to_owned()))
        .collect();
    titles.sort();
    assert_eq!(
        titles,
        [
            ("catalog:617154366".to_owned(), "Get Lucky".to_owned()),
            ("i.t0".to_owned(), "Zero".to_owned()),
            ("i.t300".to_owned(), "Three Hundred".to_owned()),
        ]
    );
    // 301 library ids, each asked for once, in two requests; the catalog ids in one.
    let reads = fixtures.seen_for("GET", "/v1/me/library/songs");
    assert_eq!(reads.iter().map(id_count).collect::<Vec<_>>(), [300, 1]);
    let catalog = fixtures.seen_for("GET", "/v1/catalog/us/songs");
    assert_eq!(catalog.len(), 1);
    assert_eq!(id_count(&catalog[0]), 2);
    // Those Apple does not have are not asked for again, and those it has are held.
    assert!(binding.known_missing("catalog:9999"));
    assert!(binding.read_unknown_tracks(asked).await.is_empty());
    assert_eq!(fixtures.seen_for("GET", "/v1/me/library/songs").len(), 2);
    assert!(binding.cached_summary("i.t300").is_some());
}

// Browsing ---------------------------------------------------------------

#[tokio::test]
async fn the_artists_an_artists_albums_and_an_albums_songs_are_read_from_the_library() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/library/artists",
            Reply::fixture("browse_artists_page1.json"),
        )
        .on(
            "GET",
            "/v1/me/library/artists",
            Reply::fixture("browse_artists_page2.json"),
        )
        .on(
            "GET",
            "/v1/me/library/artists/r.art1/albums",
            Reply::fixture("browse_artist_albums.json"),
        )
        .on(
            "GET",
            "/v1/me/library/albums/l.alb1",
            Reply::fixture("browse_album.json"),
        )
        .on(
            "GET",
            "/v1/me/library/albums/l.alb1/tracks",
            Reply::fixture("browse_album_tracks.json"),
        )
        .on(
            "GET",
            "/v1/me/library/albums/l.missing",
            Reply::fixture_status(404, "error_not_found.json"),
        )
        .on(
            "GET",
            "/v1/me/library/albums/l.missing/tracks",
            Reply::fixture_status(404, "error_not_found.json"),
        );
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);

    let artists = binding.artists().await.unwrap();
    let ids: Vec<&str> = artists.iter().map(|artist| artist.id.as_str()).collect();
    assert_eq!(ids, ["r.art1", "r.art2"]);
    assert_eq!(
        fixtures.seen_for("GET", "/v1/me/library/artists")[1].target,
        "/v1/me/library/artists?offset=1&limit=100&include=catalog"
    );

    let albums = binding.read_artist_albums("r.art1").await.unwrap();
    assert_eq!(album_ids(&albums), ["l.alb1", "l.alb3"]);

    let songs = binding.read_album_songs("l.alb1").await.unwrap();
    let listed: Vec<&str> = songs.iter().map(|song| song.id.as_str()).collect();
    assert_eq!(listed, ["i.lib1", "i.lib2", "i.lib4"]);
    // Each names its album, and without a cover of its own takes the album's.
    assert!(songs.iter().all(|song| song.album_id() == Some("l.alb1")));
    let cover = |song: &LibrarySong| cover_url(SongRef::Library(song).artwork());
    assert!(cover(&songs[2]).is_some());
    assert_eq!(
        fixtures.seen_for("GET", "/v1/me/library/albums/l.alb1")[0].target,
        "/v1/me/library/albums/l.alb1?include=artists"
    );

    not_found(binding.read_album_songs("l.missing").await);
}

#[tokio::test]
async fn albums_list_the_recently_added_first_and_stay_complete_without_them() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/library/recently-added",
            Reply::fixture("browse_recently_added.json"),
        )
        .on(
            "GET",
            "/v1/me/library/albums",
            Reply::fixture("browse_albums.json"),
        );
    let (_dir, Bound { binding, clock, .. }) = signed_in(&fixtures);

    let albums = binding.albums().await.unwrap();
    assert_eq!(album_ids(&albums), ["l.alb2", "l.alb1", "l.alb3"]);
    assert_eq!(albums[0].artist_id(), Some("r.art2"));
    // The catalog album comes only through the catalog relationship, which is kept.
    assert_eq!(albums[0].catalog_id(), Some("1440002"));
    assert_eq!(albums[1].catalog_id(), None);
    assert_eq!(
        fixtures.seen_for("GET", "/v1/me/library/albums")[0].target,
        ALBUMS_PAGE
    );
    assert_eq!(
        fixtures.seen_for("GET", "/v1/me/library/recently-added")[0].target,
        "/v1/me/library/recently-added?limit=25"
    );
    binding.albums().await.unwrap();
    assert_eq!(fixtures.seen_for("GET", "/v1/me/library/albums").len(), 1);
    // Old: given at once, read again in the background.
    clock.advance(LONG_LIST_AGE);
    assert_eq!(binding.albums().await.unwrap().len(), 3);
    until(|| fixtures.seen_for("GET", "/v1/me/library/albums").len() == 2).await;

    let without = FixtureServer::start().await;
    without
        .on(
            "GET",
            "/v1/me/library/recently-added",
            Reply::fixture_status(503, "error_server.json"),
        )
        .on(
            "GET",
            "/v1/me/library/albums",
            Reply::fixture("browse_albums.json"),
        );
    let (_dir, Bound { binding, .. }) = signed_in(&without);
    let albums = binding.albums().await.unwrap();
    assert_eq!(album_ids(&albums), ["l.alb1", "l.alb2", "l.alb3"]);
}

#[tokio::test]
async fn the_artist_list_shows_every_catalog_artist_apple_names_with_its_picture() {
    let fixtures = FixtureServer::start().await;
    let catalog = |id: &str, name: &str| {
        json!({
            "id": id,
            "type": "artists",
            "attributes": {
                "name": name,
                "artwork": {"url": format!("https://example.com/{id}/{{w}}x{{h}}.jpg"), "width": 1200},
            },
        })
    };
    fixtures
        .on(
            "GET",
            "/v1/me/library/artists",
            Reply::json(json!({"data": [
                {
                    "id": "r.duo",
                    "type": "library-artists",
                    "attributes": {"name": "Artist One & Guest"},
                    "relationships": {"catalog": {"data": [
                        catalog("300", "Artist One"),
                        // One Apple sent without an id is left out, and the others are kept.
                        {"type": "artists", "attributes": {"name": "Nobody"}},
                        catalog("302", "Guest"),
                    ]}},
                },
                {
                    "id": "r.empty",
                    "type": "library-artists",
                    "attributes": {"name": "Empty"},
                    "relationships": {"catalog": {"data": []}},
                },
                {"id": "r.plain", "type": "library-artists", "attributes": {"name": "Plain"}},
            ]})),
        )
        .on_target("GET", LIBRARY_PAGE_1, Reply::json(json!({"data": []})))
        .on(
            "GET",
            "/v1/me/library/playlists",
            Reply::json(json!({"data": []})),
        );
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);

    let named = binding.named_artists().await.unwrap();
    let listed: Vec<(String, String, Option<String>)> = named
        .iter()
        .map(|artist| {
            (
                artist.name.clone(),
                artist.remote(),
                cover_url(artist.artwork.as_ref()),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            (
                "Artist One".to_owned(),
                "artist:300".to_owned(),
                Some("https://example.com/300/600x600.jpg".to_owned())
            ),
            (
                "Guest".to_owned(),
                "artist:302".to_owned(),
                Some("https://example.com/302/600x600.jpg".to_owned())
            ),
            ("Empty".to_owned(), "name:empty".to_owned(), None),
            ("Plain".to_owned(), "name:plain".to_owned(), None),
        ]
    );
    let seen = fixtures.seen_for("GET", "/v1/me/library/artists");
    assert!(
        seen[0].target.contains("include=catalog"),
        "{}",
        seen[0].target
    );
}

#[tokio::test]
async fn the_catalog_artists_of_the_songs_are_read_in_apples_order_with_the_developer_token() {
    let fixtures = FixtureServer::start().await;
    let artist =
        |id: &str, name: &str| json!({"id": id, "type": "artists", "attributes": {"name": name}});
    let song = |id: &str, catalog_id: &str| {
        json!({
            "id": id,
            "type": "library-songs",
            "attributes": {"name": id, "playParams": {"id": id, "kind": "song", "catalogId": catalog_id}},
        })
    };
    fixtures
        .on(
            "GET",
            "/v1/catalog/us/songs",
            Reply::json(json!({"data": [
                {"id": "1", "type": "songs", "attributes": {"name": "Duet"},
                    "relationships": {"artists": {"data": [artist("20", "Bob"), artist("10", "Ann")]}}},
                {"id": "2", "type": "songs", "attributes": {"name": "Alone"}},
            ]})),
        )
        .on_target(
            "GET",
            LIBRARY_PAGE_1,
            Reply::json(json!({"data": [song("i.1", "1"), song("i.2", "2"), song("i.3", "3")]})),
        )
        .on(
            "GET",
            "/v1/me/library/playlists",
            Reply::json(json!({"data": []})),
        );
    let (_dir, Bound { binding, clock, .. }) = signed_in(&fixtures);

    let read = binding.read_song_artists().await.unwrap();
    let named: Vec<(&str, Vec<&str>)> = read
        .songs
        .iter()
        .map(|(song, artists)| {
            (
                song.as_str(),
                artists
                    .iter()
                    .map(|id| read.artists[id].name.as_str())
                    .collect(),
            )
        })
        .collect();
    // A song the catalog does not know is asked about, and names nobody.
    assert_eq!(
        named,
        [
            ("1", vec!["Bob", "Ann"]),
            ("2", Vec::new()),
            ("3", Vec::new())
        ]
    );
    let asked = fixtures.seen_for("GET", "/v1/catalog/us/songs");
    assert_eq!(asked.len(), 1);
    assert!(asked[0].target.contains("include=artists"));
    // The catalog is asked with the developer token only.
    assert!(asked[0].user_token.is_none());

    // Kept, a song is not asked about again, also once the list is old.
    let song_artists = binding.song_artists().await;
    assert_eq!(song_artists.songs.len(), 3);
    clock.advance(SHORT_LIST_AGE);
    binding.song_artists().await;
    tokio_time::sleep(Duration::from_millis(200)).await;
    assert_eq!(fixtures.seen_for("GET", "/v1/catalog/us/songs").len(), 2);
}

#[tokio::test]
async fn the_library_artists_are_kept_and_read_again_once_old() {
    let fixtures = FixtureServer::start().await;
    fixtures.on(
        "GET",
        "/v1/me/library/artists",
        Reply::fixture("browse_artists_page2.json"),
    );
    let (_dir, Bound { binding, clock, .. }) = signed_in(&fixtures);
    let reads = || fixtures.seen_for("GET", "/v1/me/library/artists").len();

    binding.artists().await.unwrap();
    clock.advance(SHORT_LIST_AGE - Duration::from_secs(1));
    binding.artists().await.unwrap();
    assert_eq!(reads(), 1);
    clock.advance(Duration::from_secs(1));
    binding.artists().await.unwrap();
    until(|| reads() == 2).await;
}

// Kept lists -------------------------------------------------------------

#[tokio::test]
async fn an_old_list_is_given_at_once_and_only_one_that_changed_is_told() {
    let fixtures = FixtureServer::start().await;
    serve_albums(&fixtures);
    let (
        _dir,
        Bound {
            binding,
            told,
            clock,
        },
    ) = signed_in(&fixtures);
    let reads = || fixtures.seen_for("GET", "/v1/me/library/albums").len();

    assert_eq!(binding.albums().await.unwrap().len(), 3);
    // Old, and Apple answers the same: one read runs, however often the list is asked for
    // meanwhile, and nobody is told.
    clock.advance(LONG_LIST_AGE);
    fixtures.delay("GET", "/v1/me/library/albums", Duration::from_millis(200));
    for _ in 0..3 {
        assert_eq!(binding.albums().await.unwrap().len(), 3);
    }
    until(|| reads() == 2).await;
    tokio_time::sleep(Duration::from_millis(300) + CONTENT_CHANGED_DELAY).await;
    assert_eq!(reads(), 2);
    assert_eq!(told.told(), 0);
    // The list read again counts its age from its read.
    binding.albums().await.unwrap();
    assert_eq!(reads(), 2);

    // Old again, and Apple lists another album: the old list answers at once, and the UI hears
    // of the new one, which answers from then on.
    clock.advance(LONG_LIST_AGE);
    fixtures.on_target("GET", ALBUMS_PAGE, Reply::fixture("library_albums.json"));
    assert_eq!(binding.albums().await.unwrap().len(), 3);
    until(|| told.told() == 1).await;
    assert_eq!(album_ids(&binding.albums().await.unwrap()), ["l.NWqWi1m"]);
}

#[tokio::test]
async fn a_failed_read_again_keeps_the_old_list_and_is_not_tried_again_at_once() {
    let fixtures = FixtureServer::start().await;
    serve_albums(&fixtures);
    let (
        _dir,
        Bound {
            binding,
            told,
            clock,
        },
    ) = signed_in(&fixtures);
    let reads = || fixtures.seen_for("GET", "/v1/me/library/albums").len();
    assert_eq!(binding.albums().await.unwrap().len(), 3);

    clock.advance(LONG_LIST_AGE);
    fixtures.on_target(
        "GET",
        ALBUMS_PAGE,
        Reply::fixture_status(503, "error_server.json"),
    );
    assert_eq!(binding.albums().await.unwrap().len(), 3);
    until(|| reads() > 1).await;
    // Once the failed read has ended, asking again starts no other for a minute.
    tokio_time::sleep(Duration::from_millis(300)).await;
    let failed = reads();
    for _ in 0..5 {
        assert_eq!(binding.albums().await.unwrap().len(), 3);
    }
    tokio_time::sleep(Duration::from_millis(300)).await;
    assert_eq!(reads(), failed);

    // After that minute the next call reads again.
    clock.advance(Duration::from_secs(61));
    assert_eq!(binding.albums().await.unwrap().len(), 3);
    until(|| reads() > failed).await;
    assert_eq!(told.told(), 0);
}

#[tokio::test]
async fn the_callers_of_a_failing_first_read_share_one_attempt() {
    let fixtures = FixtureServer::start().await;
    serve_albums(&fixtures);
    fixtures.on_target(
        "GET",
        ALBUMS_PAGE,
        Reply::fixture_status(503, "error_server.json"),
    );
    fixtures.delay("GET", "/v1/me/library/albums", Duration::from_millis(100));
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);

    let results = future::join_all((0..5).map(|_| binding.albums())).await;

    for result in results {
        unavailable(result);
    }
    // One read: the five attempts of the client's retry policy, not five times that.
    assert_eq!(fixtures.seen_for("GET", "/v1/me/library/albums").len(), 5);
}

#[tokio::test]
async fn a_list_is_still_given_once_apple_rejects_the_sign_in() {
    let fixtures = FixtureServer::start().await;
    serve_albums(&fixtures);
    let (_dir, Bound { binding, clock, .. }) = signed_in(&fixtures);
    let reads = || fixtures.seen_for("GET", "/v1/me/library/albums").len();
    assert_eq!(binding.albums().await.unwrap().len(), 3);

    clock.advance(LONG_LIST_AGE);
    fixtures
        .on_target(
            "GET",
            ALBUMS_PAGE,
            Reply::fixture_status(401, "error_unauthorized.json"),
        )
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::fixture_status(401, "error_unauthorized.json"),
        );
    // A lost sign-in clears nothing: the list is still given while the app signs in again.
    assert_eq!(binding.albums().await.unwrap().len(), 3);
    until(|| reads() > 1).await;
    tokio_time::sleep(Duration::from_millis(200)).await;
    assert_eq!(binding.albums().await.unwrap().len(), 3);
}

#[tokio::test]
async fn kept_lists_answer_a_binding_made_again_before_its_sign_in_which_reads_them_again() {
    let fixtures = FixtureServer::start().await;
    serve_library(&fixtures);
    serve_albums(&fixtures);
    let dir = tempfile::tempdir().unwrap();
    let library_id = Uuid::new_v4();
    let lists = lists_folder(dir.path(), library_id);
    let (songs, albums) = {
        let Bound { binding, .. } = bound_in(&fixtures, dir.path(), library_id);
        binding.sign_in(tokens());
        (
            binding.songs().await.unwrap(),
            binding.albums().await.unwrap(),
        )
    };
    until(|| lists.join("songs.json").exists() && lists.join("albums.json").exists()).await;
    let asked = fixtures.seen().len();

    // As after a restart: the lists answer at once, before any sign-in, without Apple.
    let Bound { binding, told, .. } = bound_in(&fixtures, dir.path(), library_id);
    assert_eq!(binding.songs().await.unwrap(), songs);
    assert_eq!(binding.albums().await.unwrap(), albums);
    assert_eq!(binding.known_songs(), Some(songs.clone()));
    // Nothing else is answered signed out.
    waits_for_sign_in(binding.artists().await, &binding);
    waits_for_sign_in(binding.song("i.unlisted").await, &binding);
    assert_eq!(fixtures.seen().len(), asked);

    // The first sign-in keeps them, and reads again those asked for: the albums changed.
    fixtures.on_target("GET", ALBUMS_PAGE, Reply::fixture("library_albums.json"));
    binding.sign_in(tokens());
    until(|| told.told() == 1).await;
    for path in ["/v1/me/library/songs", "/v1/me/library/albums"] {
        until(|| fixtures.seen_for("GET", path).len() > 1).await;
    }
    assert_eq!(album_ids(&binding.albums().await.unwrap()), ["l.NWqWi1m"]);
}

#[tokio::test]
async fn a_sign_out_deletes_the_kept_lists_and_other_tokens_replace_them() {
    let fixtures = FixtureServer::start().await;
    serve_library(&fixtures);
    serve_albums(&fixtures);
    let dir = tempfile::tempdir().unwrap();
    let library_id = Uuid::new_v4();
    let lists = lists_folder(dir.path(), library_id);
    let Bound { binding, .. } = bound_in(&fixtures, dir.path(), library_id);
    binding.sign_in(tokens());
    binding.songs().await.unwrap();
    until(|| lists.join("songs.json").exists()).await;

    binding.sign_out();
    assert!(!lists.join("songs.json").exists());
    let Bound {
        binding: restarted, ..
    } = bound_in(&fixtures, dir.path(), library_id);
    waits_for_sign_in(restarted.songs().await, &restarted);

    // Other tokens keep the albums until the read under them replaces them.
    binding.sign_in(tokens());
    binding.albums().await.unwrap();
    until(|| lists.join("albums.json").exists()).await;
    fixtures.on_target("GET", ALBUMS_PAGE, Reply::fixture("library_albums.json"));
    binding.sign_in(tokens_of("another-user-token"));
    until(|| {
        fs::read_to_string(lists.join("albums.json")).is_ok_and(|kept| kept.contains("l.NWqWi1m"))
    })
    .await;
    assert_eq!(album_ids(&binding.albums().await.unwrap()), ["l.NWqWi1m"]);
}

#[tokio::test]
async fn a_kept_list_that_cannot_be_read_is_ignored_and_written_again() {
    let fixtures = FixtureServer::start().await;
    serve_library(&fixtures);
    serve_albums(&fixtures);
    let dir = tempfile::tempdir().unwrap();
    let library_id = Uuid::new_v4();
    let lists = lists_folder(dir.path(), library_id);
    fs::create_dir_all(&lists).unwrap();
    fs::write(lists.join("songs.json"), "{\"version\":1,\"value\":").unwrap();
    fs::write(lists.join("albums.json"), r#"{"version":999,"value":[]}"#).unwrap();
    let Bound { binding, .. } = bound_in(&fixtures, dir.path(), library_id);
    waits_for_sign_in(binding.songs().await, &binding);
    waits_for_sign_in(binding.albums().await, &binding);

    binding.sign_in(tokens());
    assert_eq!(binding.songs().await.unwrap().len(), 6);
    assert_eq!(binding.albums().await.unwrap().len(), 3);
    let store = ListStore::new(lists, None);
    until(|| {
        store.load::<AppleSongs>("songs").map(|list| list.len()) == Some(6)
            && store
                .load::<Vec<LibraryAlbum>>("albums")
                .map(|list| list.len())
                == Some(3)
    })
    .await;
}

// What the binding tells -------------------------------------------------

#[test]
fn what_apple_answered_never_reaches_what_the_binding_tells_of_a_failure() {
    let unreadable =
        AppleApiError::Decode(serde_json::from_str::<Vec<u32>>(r#"["dawn-private"]"#).unwrap_err());
    let told = classified(&unreadable);
    assert_eq!(
        told,
        AppleError::Rejected("an answer that could not be read".to_owned())
    );
    assert!(!told.to_string().contains("private"), "{told}");
    let rejected = AppleApiError::Api {
        status: 400,
        errors: vec![apple_music::ApiError {
            code: Some("PARAMETER_ERROR".to_owned()),
            detail: Some("dawn-private".to_owned()),
            ..Default::default()
        }],
    };
    let told = classified(&rejected).to_string();
    assert!(
        told.contains("status 400") && told.contains("PARAMETER_ERROR"),
        "{told}"
    );
    assert!(!told.contains("private"), "{told}");
}

#[test]
fn a_track_names_a_library_song_or_a_catalog_song() {
    assert_eq!(
        song_of_track("i.abc"),
        Some((AppleIdKind::LibrarySong, "i.abc"))
    );
    assert_eq!(
        song_of_track("catalog:123"),
        Some((AppleIdKind::CatalogSong, "123"))
    );
    for text in ["", "i.", "catalog:", "catalog:catalog:1", "l.album", "123"] {
        assert_eq!(song_of_track(text), None, "{text}");
    }
    let (_dir, binding, _) = unserved();
    // A library song and a catalog song share the numbers a media file id is, so one names
    // either.
    let library = binding.intern_track("i.abc").unwrap();
    let catalog = binding.intern_track("catalog:123").unwrap();
    assert_ne!(library, catalog);
    assert_eq!(binding.resolve_track(library).unwrap(), "i.abc");
    assert_eq!(binding.resolve_track(catalog).unwrap(), "catalog:123");
    assert!(binding.resolve_track(catalog + 1).is_err());
    assert!(binding.intern_track("not a track").is_err());
}

#[tokio::test]
async fn a_cover_above_the_cap_is_dropped_and_one_at_it_is_kept() {
    let app = Router::new()
        .route(
            "/over",
            get(|| async { (StatusCode::OK, vec![1u8; MAX_COVER_BYTES + 1]).into_response() }),
        )
        .route(
            "/at",
            get(|| async { (StatusCode::OK, vec![1u8; MAX_COVER_BYTES]).into_response() }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let covers = AppleCovers {
        http: reqwest::Client::new(),
    };
    assert!(
        covers
            .cover_bytes(&format!("{base}/over"))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        covers
            .cover_bytes(&format!("{base}/at"))
            .await
            .unwrap()
            .len(),
        MAX_COVER_BYTES
    );
    server.abort();
}

#[tokio::test]
async fn tracks_go_to_the_host_by_their_key_without_a_request_signed_in_or_not() {
    let fixtures = FixtureServer::start().await;
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);
    let hosted = async |track: &str| match binding.playback_source(track).await {
        Ok(ServiceTarget::Hosted(hosted)) => (hosted.host, hosted.key),
        _ => panic!("{track} must go to the host"),
    };

    for track in ["i.vMX1AgVtgLE0KDY", "catalog:1440833098"] {
        assert_eq!(
            hosted(track).await,
            (APPLE_MUSIC_HOST.to_owned(), track.to_owned())
        );
    }
    assert!(
        binding.playback_source("not a track").await.is_err(),
        "a text that names no song has no target"
    );
    // MusicKit authorizes the user on its own, so the hub's sign-in does not matter.
    binding.sign_out();
    assert_eq!(
        hosted("i.vMX1AgVtgLE0KDY").await,
        (APPLE_MUSIC_HOST.to_owned(), "i.vMX1AgVtgLE0KDY".to_owned())
    );
    assert!(fixtures.seen().is_empty());
}

#[tokio::test]
async fn every_unknown_song_is_read_and_answered_past_the_cache_bound() {
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&asked);
    let app = Router::new().fallback(
        move |axum::extract::Query(query): axum::extract::Query<HashMap<String, String>>| {
            counted.fetch_add(1, Ordering::SeqCst);
            async move {
                let data: Vec<_> = query
                    .get("ids")
                    .map(|ids| ids.split(',').map(str::to_owned).collect::<Vec<String>>())
                    .unwrap_or_default()
                    .into_iter()
                    .map(
                        |id| json!({"id": id, "type": "library-songs", "attributes": {"name": id}}),
                    )
                    .collect();
                axum::Json(json!({ "data": data }))
            }
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let binding = AppleMusicBinding::new(
        Uuid::new_v4(),
        |tokens| AppleMusicClient::builder(tokens).base_url(base).build(),
        dir.path(),
        Arc::new(Counted::default()),
        Arc::new(SystemClock),
    );
    binding.sign_in(tokens());
    let wanted = MAX_FETCHED_TRACKS + 500;
    let tracks: Vec<String> = (0..wanted).map(|index| format!("i.u{index}")).collect();
    let read = binding.read_unknown_tracks(tracks.clone()).await;
    assert_eq!(read.len(), wanted);
    for track in &tracks {
        assert!(read.contains_key(track), "{track}");
    }
    assert!(binding.catalog().fetched.len() <= MAX_FETCHED_TRACKS);
    assert!(asked.load(Ordering::SeqCst) > 0);
    server.abort();
}

#[tokio::test]
async fn songs_read_for_the_queue_are_kept_together_when_the_cache_is_almost_full() {
    let fixtures = FixtureServer::start().await;
    fixtures.on(
        "GET",
        "/v1/me/library/songs",
        Reply::json(json!({"data": [library_song("i.a", "A"), library_song("i.b", "B")]})),
    );
    let (_dir, Bound { binding, .. }) = signed_in(&fixtures);
    {
        let mut catalog = binding.catalog();
        for index in 1..MAX_FETCHED_TRACKS {
            let held: LibrarySong =
                serde_json::from_value(library_song(&format!("i.h{index}"), "Held")).unwrap();
            catalog
                .fetched
                .insert(held.id.clone(), AppleSong::Library(held));
        }
    }

    binding
        .read_unknown_tracks(["i.a".to_owned(), "i.b".to_owned()])
        .await;
    for track in ["i.a", "i.b"] {
        assert!(binding.known_song(track).is_some(), "{track}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_burst_of_changes_tells_the_ui_once_after_the_last() {
    let (_dir, binding, counted) = unserved();
    let step = CONTENT_CHANGED_DELAY / 3;
    // Changes closer together than the delay wait for one another, however long they go on.
    for _ in 0..20 {
        binding.content_changed();
        tokio_time::sleep(step).await;
    }
    assert_eq!(counted.told(), 0);
    tokio_time::sleep(CONTENT_CHANGED_DELAY).await;
    assert_eq!(counted.told(), 1);
    // Nothing more comes of them.
    tokio_time::sleep(CONTENT_CHANGED_DELAY * 4).await;
    assert_eq!(counted.told(), 1);
    // A change after that is told again, once it has rested.
    binding.content_changed();
    tokio_time::sleep(CONTENT_CHANGED_DELAY - Duration::from_millis(1)).await;
    assert_eq!(counted.told(), 1);
    tokio_time::sleep(Duration::from_millis(2)).await;
    assert_eq!(counted.told(), 2);
}

#[test]
fn without_a_runtime_a_change_is_told_at_once() {
    let (_dir, binding, counted) = unserved();
    binding.content_changed();
    binding.content_changed();
    assert_eq!(counted.told(), 2);
}

#[test]
fn a_listing_without_a_cover_keeps_the_cover_an_earlier_one_gave() {
    let (_dir, binding, _) = unserved();
    let artwork: Artwork = serde_json::from_value(json!({
        "url": "https://covers.invalid/{w}x{h}{c}.{f}",
    }))
    .unwrap();
    let id = binding.intern(AppleIdKind::Artist, "r.1").unwrap();
    let generation = binding.generation();
    binding.remember_collection(generation, AppleIdKind::Artist, id, "Name", Some(&artwork));
    binding.remember_collection(generation, AppleIdKind::Artist, id, "Name", None);
    let info = binding.collection_info(AppleIdKind::Artist, id).unwrap();
    assert_eq!(
        info.cover.as_deref(),
        Some("https://covers.invalid/600x600bb.jpg")
    );
    // A listing from before a sign-out is not remembered at all.
    binding.sign_out();
    binding.remember_collection(generation, AppleIdKind::Artist, id, "Old", Some(&artwork));
    assert!(binding.collection_info(AppleIdKind::Artist, id).is_none());
}

#[test]
fn kept_mixes_add_up_across_builds_until_too_many_and_only_under_their_sign_in() {
    let (_dir, binding, _) = unserved();
    let from = Arc::new(Vec::new());
    let keep = |generation: u64, ids: std::ops::Range<i32>| {
        let mixes = ids
            .clone()
            .map(|id| {
                let mix = RecommendedMix {
                    name: format!("Mix {id}"),
                    group: "Made for You".into(),
                    cover: None,
                };
                (id, mix)
            })
            .collect();
        binding.keep_recommendations(
            generation,
            &from,
            vec![("Made for You".into(), ids.collect())],
            mixes,
        );
    };
    let held = |id: i32| binding.recommended_mix(id).is_some();
    let generation = binding.generation();
    let half = MAX_KEPT_MIXES as i32 / 2;
    keep(generation, 0..half);
    // A later build adds to them, so a page opened from an earlier one finds its mix.
    keep(generation, half..half + 10);
    assert!(held(0) && held(half + 9));
    assert_eq!(
        binding.built_mix_groups(&from).unwrap()[0].1,
        (half..half + 10).collect::<Vec<_>>()
    );
    // Recommendations read again are other ones, which the groups are built from anew.
    assert!(binding.built_mix_groups(&Arc::new(Vec::new())).is_none());
    // Mixes held already do not count again: listing them anew keeps the others.
    keep(generation, 0..half);
    assert!(held(0) && held(half + 9));
    // Past the bound, only the last build's are held.
    keep(generation, half + 10..2 * half + 10);
    assert!(!held(0) && !held(half + 9) && held(half + 10));
    // A sign-out drops them, and a build begun before it is not kept after it.
    binding.sign_out();
    assert!(!held(half + 10) && binding.built_mix_groups(&from).is_none());
    keep(generation, 0..1);
    assert!(!held(0) && binding.built_mix_groups(&from).is_none());
}

#[test]
fn the_songs_of_catalog_collections_are_kept_within_their_bounds_least_used_first() {
    let (_dir, binding, _) = unserved();
    let songs = |count: usize, prefix: &str| -> Arc<Vec<Song>> {
        Arc::new(
            (0..count)
                .map(|n| {
                    serde_json::from_value(json!({"id": format!("{prefix}{n}"), "type": "songs"}))
                        .unwrap()
                })
                .collect(),
        )
    };
    let mut catalog = binding.catalog();
    let (first, second, third) = (
        CatalogCollection::Playlist(1),
        CatalogCollection::Playlist(2),
        CatalogCollection::Album(3),
    );
    catalog.keep_collection_songs(first, songs(12_000, "a"));
    catalog.keep_collection_songs(second, songs(5_000, "b"));
    // The first is used again, so the second goes when the third does not fit beside both.
    assert!(catalog.used_collection_songs(first).is_some());
    catalog.keep_collection_songs(third, songs(5_000, "c"));
    assert!(catalog.used_collection_songs(second).is_none());
    assert!(catalog.used_collection_songs(first).is_some());
    assert!(catalog.used_collection_songs(third).is_some());
    assert_eq!(
        catalog.collection_song("c7").map(|song| song.id.as_str()),
        Some("c7")
    );
    assert!(catalog.collection_song("b7").is_none());
    // Songs that are more than the bound are not kept at all.
    catalog.keep_collection_songs(second, songs(MAX_KEPT_COLLECTION_SONGS + 1, "b"));
    assert!(catalog.used_collection_songs(second).is_none());
    // Only so many collections are kept, the least recently used going first.
    for id in 10..10 + MAX_KEPT_COLLECTIONS as i32 {
        catalog.keep_collection_songs(CatalogCollection::Album(id), songs(1, &format!("x{id}.")));
    }
    assert_eq!(catalog.collection_songs.len(), MAX_KEPT_COLLECTIONS);
    assert!(catalog.used_collection_songs(first).is_none());
}
