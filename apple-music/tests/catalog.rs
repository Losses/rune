//! The catalog layer of the library against the fixture server.
//!
//! The `catalog_*` and `library_search_*` fixtures are hand-written in the shape of Apple's
//! documented responses (Search for Catalog Resources, Get a Catalog Album, Artist or
//! Playlist), not recorded from Apple.

use std::time;

mod common;

use std::sync::Arc;
use std::time::Duration;

use apple_music::library::{
    AlbumItem, AppleLibrary, DEFAULT_RECOMMENDATIONS, ItemId, LibraryError, RecommendationKind,
    ReleaseView, SearchScope, Tokens, Track,
};
use common::{
    DETAIL_TTL, DEVELOPER_TOKEN, FixtureServer, ManualClock, Reply, TTL, USER_TOKEN,
    assert_not_found, fast_retry, fixture, keys, until,
};
use serde_json::json;

const LIBRARY_PAGE_1: &str = "/v1/me/library/songs?limit=100&include=albums,artists";
const LIBRARY_PAGE_2: &str = "/v1/me/library/songs?offset=2&limit=100&include=albums,artists";

fn signed_in(fixtures: &FixtureServer) -> (AppleLibrary, Arc<ManualClock>) {
    let base_url = fixtures.base_url.clone();
    let clock = ManualClock::new();
    let library = AppleLibrary::with_client(|builder| {
        builder
            .base_url(base_url)
            .storefront("us")
            .retry_policy(fast_retry())
    })
    .with_clock(clock.clone());
    library.sign_in(Tokens::new(DEVELOPER_TOKEN, USER_TOKEN).unwrap());
    (library, clock)
}

fn catalog(id: &str) -> ItemId {
    ItemId::catalog(id).unwrap()
}

fn library_item(id: &str) -> ItemId {
    ItemId::library(id).unwrap()
}

fn marks(tracks: &[Track]) -> Vec<(String, bool)> {
    tracks
        .iter()
        .map(|track| (track.key.to_string(), track.in_library))
        .collect()
}

fn album_marks(albums: &[AlbumItem]) -> Vec<(ItemId, bool)> {
    albums
        .iter()
        .map(|album| (album.id.clone(), album.in_library))
        .collect()
}

/// Every catalog request carried the developer token only, and every request for the
/// user's own data the Music User Token as well.
fn assert_tokens(fixtures: &FixtureServer) {
    let seen = fixtures.seen();
    assert!(!seen.is_empty());
    for request in seen {
        let user = request.path.starts_with("/v1/me/");
        let catalog = request.path.starts_with("/v1/catalog/");
        assert!(user || catalog, "{request:?}");
        assert_eq!(
            request.user_token.is_some(),
            user,
            "{} {}",
            request.method,
            request.target
        );
        assert_eq!(
            request.authorization.as_deref(),
            Some(format!("Bearer {DEVELOPER_TOKEN}").as_str())
        );
    }
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

fn serve_catalog_items(fixtures: &FixtureServer) {
    fixtures
        .on(
            "GET",
            "/v1/catalog/us/albums/7002",
            fixture("catalog_album_sunrise.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/albums/7002/tracks",
            fixture("catalog_album_sunrise_tracks.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/artists/8002",
            fixture("catalog_artist_nine.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/artists/8002/albums",
            fixture("catalog_artist_nine_albums_page1.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/artists/8002/albums",
            fixture("catalog_artist_nine_albums_page2.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/playlists/pl.morning",
            fixture("catalog_playlist_morning.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/playlists/pl.morning/tracks",
            fixture("catalog_collection_tracks_page2.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/albums/404",
            Reply::fixture_status(404, "error_not_found.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/albums/404/tracks",
            Reply::fixture_status(404, "error_not_found.json"),
        )
        .on(
            "GET",
            "/v1/me/library/artists",
            Reply::json(json!({"data": [
                {"id": "r.art1", "type": "library-artists", "attributes": {"name": "Artist One"}},
                {"id": "r.art2", "type": "library-artists", "attributes": {"name": "Artist Two"}}
            ]})),
        );
}

/// The library's songs and albums, as the kept lists the catalog marks read.
fn serve_library(fixtures: &FixtureServer) {
    fixtures
        .on_target(
            "GET",
            LIBRARY_PAGE_1,
            fixture("union_library_songs_page1.json"),
        )
        .on_target(
            "GET",
            LIBRARY_PAGE_2,
            fixture("union_library_songs_page2.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists",
            fixture("union_playlists.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists/p.mine1/tracks",
            fixture("union_playlist_mine_tracks.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists/p.favorites/tracks",
            fixture("union_playlist_favorites_tracks.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists/p.editorial/tracks",
            fixture("union_playlist_editorial_tracks.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists/p.empty/tracks",
            Reply::fixture_status(404, "error_not_found.json"),
        )
        .on(
            "GET",
            "/v1/me/library/playlists/p.public/tracks",
            fixture("union_playlist_public_tracks.json"),
        )
        .on(
            "GET",
            "/v1/me/library/recently-added",
            fixture("browse_recently_added.json"),
        )
        .on(
            "GET",
            "/v1/me/library/albums",
            fixture("browse_albums.json"),
        );
}

// Search ------------------------------------------------------------------

#[tokio::test]
async fn searching_everything_lists_the_library_first_then_new_catalog_items() {
    let fixtures = FixtureServer::start().await;
    serve_search(&fixtures);
    let (library, _) = signed_in(&fixtures);

    // A search box may append `*`; Apple's search has no wildcards.
    let found = library
        .search_fresh("dawn*", SearchScope::All, false)
        .await
        .unwrap();

    // 1001 is i.lib1's catalog id; the album and the artist match by name.
    assert_eq!(
        marks(&found.songs),
        [
            ("i.lib1".to_owned(), true),
            ("catalog:6001".to_owned(), false)
        ]
    );
    assert_eq!(
        album_marks(&found.albums),
        [(library_item("l.alb1"), true), (catalog("7002"), false)]
    );
    let artists: Vec<_> = found
        .artists
        .iter()
        .map(|artist| (artist.id.clone(), artist.in_library))
        .collect();
    assert_eq!(
        artists,
        [(library_item("r.art1"), true), (catalog("8002"), false)]
    );
    assert_eq!(found.playlists.len(), 1);
    assert_eq!(found.playlists[0].id, library_item("p.dawn"));
    assert!(found.playlists[0].in_library && found.playlists[0].can_append);

    let new_song = &found.songs[1];
    assert_eq!(new_song.title, "Dawn Chorus");
    // Taken from the song's relationships, as Apple sent them.
    assert_eq!(new_song.album_id.as_deref(), Some("7002"));
    assert_eq!(new_song.artist_id.as_deref(), Some("8002"));
    assert_eq!(found.albums[1].artist.as_deref(), Some("Artist Nine"));
    assert!(found.artists[1].artwork.is_some());

    let searches = fixtures.seen_for("GET", "/v1/catalog/us/search");
    assert_eq!(searches.len(), 1);
    assert_eq!(
        searches[0].target,
        "/v1/catalog/us/search?term=dawn&types=songs,albums,artists&limit=25&offset=0"
    );
    let library_searches = fixtures.seen_for("GET", "/v1/me/library/search");
    assert_eq!(library_searches.len(), 1);
    assert!(library_searches[0].target.starts_with(
        "/v1/me/library/search?term=dawn&types=library-songs,library-albums,library-artists,library-playlists&limit=25"
    ));
    assert_tokens(&fixtures);
}

#[tokio::test]
async fn searching_the_catalog_marks_what_the_library_holds_without_a_library_search() {
    let fixtures = FixtureServer::start().await;
    serve_search(&fixtures);
    serve_library(&fixtures);
    serve_catalog_items(&fixtures);
    fixtures.on(
        "GET",
        "/v1/me/library/albums",
        fixture("browse_albums.json"),
    );
    let (library, _) = signed_in(&fixtures);
    // What is known of the library is what the lists kept say.
    library.songs().await.unwrap();
    library.albums().await.unwrap();
    // Opening a catalog artist reads the library's artist names and keeps them.
    library.catalog_artist(&catalog("8002")).await.unwrap();

    let found = library
        .search_fresh("dawn", SearchScope::Catalog, false)
        .await
        .unwrap();
    assert_eq!(
        marks(&found.songs),
        [
            ("catalog:1001".to_owned(), true),
            ("catalog:6001".to_owned(), false)
        ]
    );
    assert_eq!(
        album_marks(&found.albums),
        [(catalog("7001"), true), (catalog("7002"), false)]
    );
    let artists: Vec<_> = found
        .artists
        .iter()
        .map(|artist| (artist.id.clone(), artist.in_library))
        .collect();
    assert_eq!(artists, [(catalog("8001"), true), (catalog("8002"), false)]);
    assert!(found.playlists.is_empty());

    // Apple's library search was never asked.
    assert!(fixtures.seen_for("GET", "/v1/me/library/search").is_empty());
    assert_eq!(fixtures.seen_for("GET", "/v1/catalog/us/search").len(), 1);
}

#[tokio::test]
async fn a_refreshed_catalog_search_replaces_the_kept_one() {
    let fixtures = FixtureServer::start().await;
    serve_search(&fixtures);
    let (library, _) = signed_in(&fixtures);

    let first = library
        .search_fresh("dawn", SearchScope::Catalog, false)
        .await
        .unwrap();
    // Asked again within the two minutes, it is answered from what was kept.
    library
        .search_fresh("dawn", SearchScope::Catalog, false)
        .await
        .unwrap();
    assert_eq!(fixtures.seen_for("GET", "/v1/catalog/us/search").len(), 1);

    // A refresh asks Apple again, and what it finds is kept in turn.
    let refreshed = library
        .search_fresh("dawn", SearchScope::Catalog, true)
        .await
        .unwrap();
    assert_eq!(refreshed, first);
    assert_eq!(fixtures.seen_for("GET", "/v1/catalog/us/search").len(), 2);
    library
        .search_fresh("dawn", SearchScope::Catalog, false)
        .await
        .unwrap();
    assert_eq!(fixtures.seen_for("GET", "/v1/catalog/us/search").len(), 2);
}

#[tokio::test]
async fn an_empty_search_finds_nothing_and_asks_nobody() {
    let fixtures = FixtureServer::start().await;
    serve_search(&fixtures);
    let (library, _) = signed_in(&fixtures);

    for query in ["", "  ", "\"\"", "*"] {
        for scope in [SearchScope::Library, SearchScope::Catalog, SearchScope::All] {
            let found = library.search_fresh(query, scope, false).await.unwrap();
            assert!(
                found.songs.is_empty() && found.albums.is_empty(),
                "{query:?}"
            );
        }
    }
    assert!(fixtures.seen().is_empty());
}

#[tokio::test]
async fn a_failing_catalog_leaves_the_library_matches() {
    for (status, body) in [(500, "error_server.json"), (401, "error_unauthorized.json")] {
        let fixtures = FixtureServer::start().await;
        fixtures
            .on(
                "GET",
                "/v1/me/library/search",
                fixture("library_search_dawn.json"),
            )
            .on(
                "GET",
                "/v1/catalog/us/search",
                Reply::fixture_status(status, body),
            )
            .on(
                "GET",
                "/v1/me/storefront",
                Reply::fixture("storefront.json"),
            );
        let (library, _) = signed_in(&fixtures);

        let found = library
            .search_fresh("dawn", SearchScope::All, false)
            .await
            .unwrap();
        assert_eq!(keys(&found.songs), ["i.lib1"], "{status}");
        assert_eq!(found.albums.len(), 1, "{status}");
        assert_eq!(found.artists.len(), 1, "{status}");
        assert_eq!(found.playlists.len(), 1, "{status}");

        // The catalog alone fails as Apple did.
        let error = library
            .search_fresh("dawn", SearchScope::Catalog, false)
            .await
            .unwrap_err();
        if status == 500 {
            assert!(matches!(error, LibraryError::Unavailable(_)), "{error:?}");
        } else {
            // The tokens work, so Apple refused just this request.
            assert!(matches!(error, LibraryError::Refused), "{error:?}");
        }
    }
}

#[tokio::test]
async fn a_catalog_still_retrying_when_the_search_wait_ends_leaves_the_library_matches() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/library/search",
            fixture("library_search_dawn.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/search",
            Reply::fixture_status(503, "error_server.json"),
        );
    // The production policy, which retries a 503 for several seconds.
    let base_url = fixtures.base_url.clone();
    let library = AppleLibrary::with_client(|builder| builder.base_url(base_url).storefront("us"))
        .with_search_wait(Duration::from_secs(2));
    library.sign_in(Tokens::new(DEVELOPER_TOKEN, USER_TOKEN).unwrap());

    let started = time::Instant::now();
    let found = library
        .search_fresh("dawn", SearchScope::All, false)
        .await
        .unwrap();
    let waited = started.elapsed();
    assert_eq!(keys(&found.songs), ["i.lib1"]);
    assert_eq!(found.albums.len(), 1);
    assert!(
        waited >= Duration::from_secs(2) && waited < Duration::from_secs(4),
        "waited {waited:?}"
    );
}

#[tokio::test]
async fn a_failing_library_search_leaves_the_catalog_matches_unless_the_sign_in_is_lost() {
    // The library's search fails after its retries; the catalog's matches are still listed.
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/library/search",
            Reply::fixture_status(500, "error_server.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/search",
            fixture("catalog_search_dawn.json"),
        );
    let (library, _) = signed_in(&fixtures);
    let found = library
        .search_fresh("dawn", SearchScope::All, false)
        .await
        .unwrap();
    assert_eq!(keys(&found.songs), ["catalog:1001", "catalog:6001"]);
    assert!(found.playlists.is_empty());

    // Tokens Apple rejects fail the search, so the user learns to sign in again.
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/library/search",
            Reply::fixture_status(401, "error_unauthorized.json"),
        )
        .on(
            "GET",
            "/v1/me/storefront",
            Reply::fixture_status(401, "error_unauthorized.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/search",
            fixture("catalog_search_dawn.json"),
        );
    let (library, _) = signed_in(&fixtures);
    let error = library
        .search_fresh("dawn", SearchScope::All, false)
        .await
        .unwrap_err();
    assert!(matches!(error, LibraryError::SignedOut), "{error:?}");

    // Both parts failing fail the search as the library did.
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/library/search",
            Reply::fixture_status(500, "error_server.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/search",
            Reply::fixture_status(500, "error_server.json"),
        );
    let (library, _) = signed_in(&fixtures);
    let error = library
        .search_fresh("dawn", SearchScope::All, false)
        .await
        .unwrap_err();
    assert!(matches!(error, LibraryError::Unavailable(_)), "{error:?}");

    // Parts that each take a few seconds are both listed.
    let fixtures = FixtureServer::start().await;
    serve_search(&fixtures);
    fixtures
        .delay("GET", "/v1/catalog/us/search", Duration::from_secs(4))
        .delay("GET", "/v1/me/library/search", Duration::from_secs(5));
    let (library, _) = signed_in(&fixtures);
    let found = library
        .search_fresh("dawn", SearchScope::All, false)
        .await
        .unwrap();
    assert_eq!(keys(&found.songs), ["i.lib1", "catalog:6001"]);
}

#[tokio::test]
async fn a_catalog_that_finds_nothing_is_empty() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/library/search",
            fixture("library_search_dawn.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/search",
            Reply::fixture_status(404, "error_not_found.json"),
        );
    let (library, _) = signed_in(&fixtures);

    let catalog = library
        .search_fresh("dawn", SearchScope::Catalog, false)
        .await
        .unwrap();
    assert!(catalog.songs.is_empty() && catalog.albums.is_empty() && catalog.artists.is_empty());
    let all = library
        .search_fresh("dawn", SearchScope::All, false)
        .await
        .unwrap();
    assert_eq!(keys(&all.songs), ["i.lib1"]);
}

// Browsing ----------------------------------------------------------------

#[tokio::test]
async fn catalog_albums_open_by_their_ids_and_are_kept_for_thirty_seconds() {
    let fixtures = FixtureServer::start().await;
    serve_catalog_items(&fixtures);
    let (library, clock) = signed_in(&fixtures);

    let detail = library.catalog_album(&catalog("7002")).await.unwrap();
    let album = &detail.album;
    assert_eq!(album.id, catalog("7002"));
    assert_eq!(album.name, "Sunrise EP");
    assert_eq!(album.artist_id, Some(catalog("8002")));
    assert_eq!(album.year, Some(2021));
    assert!(!album.in_library);
    // The music video is left out.
    assert_eq!(keys(&detail.tracks), ["catalog:6001", "catalog:6002"]);
    assert!(detail.tracks.iter().all(|song| {
        song.album_id.as_deref() == Some("7002")
            && !song.in_library
            && song.artwork == album.artwork
    }));
    assert_eq!(detail.tracks[0].artist_id.as_deref(), Some("8002"));
    assert_eq!(detail.tracks[1].artist_id, None);
    assert_eq!(
        fixtures.seen_for("GET", "/v1/catalog/us/albums/7002")[0].target,
        "/v1/catalog/us/albums/7002?include=artists"
    );

    let reads = || {
        (
            fixtures.seen_for("GET", "/v1/catalog/us/albums/7002").len(),
            fixtures
                .seen_for("GET", "/v1/catalog/us/albums/7002/tracks")
                .len(),
        )
    };
    assert_eq!(reads(), (1, 1));
    assert_eq!(
        library.catalog_album(&catalog("7002")).await.unwrap(),
        detail
    );
    clock.advance(DETAIL_TTL - Duration::from_secs(1));
    library.catalog_album(&catalog("7002")).await.unwrap();
    assert_eq!(reads(), (1, 1));
    clock.advance(Duration::from_secs(1));
    library.catalog_album(&catalog("7002")).await.unwrap();
    assert_eq!(reads(), (2, 2));

    assert_not_found(library.catalog_album(&catalog("404")).await);
    // A library item names nothing in the catalog.
    assert_not_found(library.catalog_album(&library_item("l.alb1")).await);
    assert_not_found(library.catalog_artist(&library_item("r.art1")).await);
    assert_tokens(&fixtures);
}

#[tokio::test]
async fn catalog_albums_are_read_many_at_once_with_every_song_of_each() {
    let fixtures = FixtureServer::start().await;
    let song = |id: &str, kind: &str| {
        json!({"id": id, "type": kind, "attributes": {"name": format!("Song {id}"),
            "albumName": "Long", "artistName": "Artist",
            "playParams": {"id": id, "kind": "song"}}})
    };
    let artwork = json!({"width": 3000, "height": 3000,
        "url": "http://artwork.invalid/long/{w}x{h}bb.jpg"});
    fixtures
        .on(
            "GET",
            "/v1/catalog/us/albums",
            Reply::json(json!({"data": [
                {"id": "7002", "type": "albums",
                    "attributes": {"name": "Sunrise EP", "artistName": "Artist Nine"},
                    "relationships": {"tracks": {"data": [song("6001", "songs")]}}},
                // Apple gives the first songs of a long album with it, and names the rest.
                {"id": "7090", "type": "albums",
                    "attributes": {"name": "Long", "artistName": "Artist", "artwork": artwork},
                    "relationships": {
                        "artists": {"data": [{"id": "8090", "type": "artists"}]},
                        "tracks": {
                            "next": "/v1/catalog/us/albums/7090/tracks?offset=2",
                            "data": [song("6091", "songs"), song("6092", "music-videos")],
                        },
                    }},
            ]})),
        )
        .on(
            "GET",
            "/v1/catalog/us/albums/7090/tracks",
            Reply::json(json!({"data": [song("6093", "songs")]})),
        );
    let (library, _) = signed_in(&fixtures);

    let ids = [
        catalog("7090"),
        library_item("l.alb1"),
        catalog("404"),
        catalog("7002"),
    ];
    let albums = library.catalog_albums(&ids).await.unwrap();
    // In the order asked, without the library's or the one the catalog does not know.
    let names: Vec<&str> = albums
        .iter()
        .map(|album| album.album.name.as_str())
        .collect();
    assert_eq!(names, ["Long", "Sunrise EP"]);
    // Every song of each, music videos left out, on the album and with its artist and cover.
    assert_eq!(keys(&albums[0].tracks), ["catalog:6091", "catalog:6093"]);
    assert!(albums[0].tracks.iter().all(|song| {
        song.album_id.as_deref() == Some("7090")
            && song.artist_id.as_deref() == Some("8090")
            && song.artwork == albums[0].album.artwork
    }));
    assert_eq!(keys(&albums[1].tracks), ["catalog:6001"]);
    // One request for the albums, and one for the rest of the long one.
    let read: Vec<String> = fixtures
        .seen_for("GET", "/v1/catalog/us/albums")
        .into_iter()
        .map(|request| request.target)
        .collect();
    assert_eq!(
        read,
        ["/v1/catalog/us/albums?ids=7090,404,7002&include=artists,tracks"]
    );
    assert_eq!(
        fixtures
            .seen_for("GET", "/v1/catalog/us/albums/7090/tracks")
            .len(),
        1
    );
    assert!(library.catalog_albums(&[]).await.unwrap().is_empty());
    assert_tokens(&fixtures);
}

#[tokio::test]
async fn album_pages_mark_the_songs_the_library_holds() {
    let fixtures = FixtureServer::start().await;
    serve_library(&fixtures);
    let song = |id: &str, name: &str| {
        json!({"id": id, "type": "songs", "attributes": {"name": name, "albumName": "Mixed",
            "artistName": "Artist", "playParams": {"id": id, "kind": "song"}}})
    };
    fixtures
        .on(
            "GET",
            "/v1/catalog/us/albums/7050",
            Reply::json(json!({"data": [{"id": "7050", "type": "albums",
                "attributes": {"name": "Mixed", "artistName": "Artist"}}]})),
        )
        .on(
            "GET",
            "/v1/catalog/us/albums/7050/tracks",
            // 1001 is the catalog id of the library song i.lib1.
            Reply::json(json!({"data": [song("1001", "Held"), song("6002", "Not Held")]})),
        );
    let (library, _) = signed_in(&fixtures);
    library.songs().await.unwrap();

    let detail = library.catalog_album(&catalog("7050")).await.unwrap();
    assert_eq!(
        marks(&detail.tracks),
        [
            ("catalog:1001".to_owned(), true),
            ("catalog:6002".to_owned(), false)
        ]
    );
}

#[tokio::test]
async fn catalog_artists_open_with_all_their_albums() {
    let fixtures = FixtureServer::start().await;
    serve_catalog_items(&fixtures);
    let (library, clock) = signed_in(&fixtures);

    let detail = library.catalog_artist(&catalog("8002")).await.unwrap();
    assert_eq!(detail.artist.name, "Artist Nine");
    assert!(!detail.artist.in_library);
    assert!(detail.artist.artwork.is_some());
    let ids: Vec<_> = detail.albums.iter().map(|album| album.id.clone()).collect();
    assert_eq!(ids, [catalog("7002"), catalog("7003")]);
    assert!(
        detail
            .albums
            .iter()
            .all(|album| album.artist_id == Some(catalog("8002")))
    );
    // The albums are listed a page of a hundred at a time, to the end of the list.
    let pages: Vec<String> = fixtures
        .seen_for("GET", "/v1/catalog/us/artists/8002/albums")
        .into_iter()
        .map(|request| request.target)
        .collect();
    assert_eq!(
        pages,
        [
            "/v1/catalog/us/artists/8002/albums",
            "/v1/catalog/us/artists/8002/albums?offset=1"
        ]
    );

    library.catalog_artist(&catalog("8002")).await.unwrap();
    assert_eq!(
        fixtures
            .seen_for("GET", "/v1/catalog/us/artists/8002")
            .len(),
        1
    );
    clock.advance(DETAIL_TTL);
    library.catalog_artist(&catalog("8002")).await.unwrap();
    assert_eq!(
        fixtures
            .seen_for("GET", "/v1/catalog/us/artists/8002")
            .len(),
        2
    );
    // Read afresh, it is read again however young what is kept, and kept in its place.
    let afresh = library
        .catalog_artist_afresh(&catalog("8002"))
        .await
        .unwrap();
    assert_eq!(afresh.artist.name, "Artist Nine");
    assert_eq!(
        fixtures
            .seen_for("GET", "/v1/catalog/us/artists/8002")
            .len(),
        3
    );
    library.catalog_artist(&catalog("8002")).await.unwrap();
    assert_eq!(
        fixtures
            .seen_for("GET", "/v1/catalog/us/artists/8002")
            .len(),
        3
    );
    assert_tokens(&fixtures);
}

#[tokio::test]
async fn catalog_playlists_open_with_their_songs_and_mixes_with_the_user_token() {
    let fixtures = FixtureServer::start().await;
    serve_catalog_items(&fixtures);
    fixtures
        .on(
            "GET",
            "/v1/catalog/us/playlists/pl.pm-mix1",
            Reply::json(json!({"data": [{"id": "pl.pm-mix1", "type": "playlists",
                "attributes": {"name": "Mix One", "playlistType": "personal-mix"}}]})),
        )
        .on(
            "GET",
            "/v1/catalog/us/playlists/pl.pm-mix1/tracks",
            fixture("catalog_collection_tracks_page2.json"),
        );
    let (library, _) = signed_in(&fixtures);

    let playlist = library
        .catalog_playlist(&catalog("pl.morning"))
        .await
        .unwrap();
    assert_eq!(playlist.playlist.name, "Morning Mix");
    assert_eq!(playlist.playlist.curator.as_deref(), Some("Apple Music"));
    assert!(!playlist.playlist.can_append && !playlist.playlist.in_library);
    assert_eq!(keys(&playlist.tracks), ["catalog:t2"]);
    // Any other playlist needs only the developer token.
    for path in [
        "/v1/catalog/us/playlists/pl.morning",
        "/v1/catalog/us/playlists/pl.morning/tracks",
    ] {
        assert!(
            fixtures
                .seen_for("GET", path)
                .iter()
                .all(|request| request.user_token.is_none())
        );
    }

    let before = fixtures.seen().len();
    let mix = library
        .catalog_playlist(&catalog("pl.pm-mix1"))
        .await
        .unwrap();
    assert_eq!(mix.playlist.name, "Mix One");
    assert_eq!(keys(&mix.tracks), ["catalog:t2"]);
    for request in &fixtures.seen()[before..] {
        assert_eq!(
            request.user_token.as_deref(),
            Some(USER_TOKEN),
            "{request:?}"
        );
    }
    assert_not_found(library.catalog_playlist(&library_item("p.mine1")).await);

    // Kept, it is not read again; read afresh, it is, and kept in its place.
    let reads = || {
        fixtures
            .seen_for("GET", "/v1/catalog/us/playlists/pl.morning/tracks")
            .len()
    };
    let before = reads();
    library
        .catalog_playlist(&catalog("pl.morning"))
        .await
        .unwrap();
    assert_eq!(reads(), before);
    library
        .catalog_playlist_afresh(&catalog("pl.morning"))
        .await
        .unwrap();
    assert_eq!(reads(), before + 1);
    library
        .catalog_playlist(&catalog("pl.morning"))
        .await
        .unwrap();
    assert_eq!(reads(), before + 1);
}

#[tokio::test]
async fn catalog_reads_use_the_developer_token_and_the_users_data_the_user_token() {
    let fixtures = FixtureServer::start().await;
    serve_library(&fixtures);
    serve_search(&fixtures);
    serve_catalog_items(&fixtures);
    fixtures.on(
        "GET",
        "/v1/me/recommendations",
        fixture("recommendations_page2.json"),
    );
    let (library, _) = signed_in(&fixtures);
    library.songs().await.unwrap();
    library
        .search_fresh("dawn", SearchScope::All, false)
        .await
        .unwrap();
    library.catalog_album(&catalog("7002")).await.unwrap();
    library.catalog_artist(&catalog("8002")).await.unwrap();
    library
        .catalog_playlist(&catalog("pl.morning"))
        .await
        .unwrap();
    library
        .recommendations(DEFAULT_RECOMMENDATIONS)
        .await
        .unwrap();

    assert_tokens(&fixtures);
    // The check is not vacuous: every kind of request was made.
    for path in [
        "/v1/catalog/us/search",
        "/v1/catalog/us/albums/7002",
        "/v1/catalog/us/albums/7002/tracks",
        "/v1/catalog/us/artists/8002",
        "/v1/catalog/us/artists/8002/albums",
        "/v1/catalog/us/playlists/pl.morning",
        "/v1/catalog/us/playlists/pl.morning/tracks",
        "/v1/me/library/search",
        "/v1/me/recommendations",
    ] {
        assert!(
            fixtures.seen().iter().any(|request| request.path == path),
            "{path}"
        );
    }
}

// Recommendations ---------------------------------------------------------

#[tokio::test]
async fn recommendations_group_albums_and_playlists_and_are_kept() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/recommendations",
            fixture("recommendations_page1.json"),
        )
        .on(
            "GET",
            "/v1/me/recommendations",
            fixture("recommendations_page2.json"),
        )
        .on("POST", "/v1/me/library", Reply::status(202));
    let (library, _) = signed_in(&fixtures);

    let groups = library
        .recommendations(DEFAULT_RECOMMENDATIONS)
        .await
        .unwrap();
    // The group of stations only is left out, and so are the stations in the others.
    let titles: Vec<&str> = groups.iter().map(|group| group.title.as_str()).collect();
    assert_eq!(titles, ["Made for Someone", "Recently Played"]);
    let made = &groups[0];
    assert_eq!(made.id, "rec-1");
    assert_eq!(made.kind, RecommendationKind::Personal);
    assert!(made.albums.is_empty());
    assert_eq!(made.playlists.len(), 1);
    let mix = &made.playlists[0];
    assert_eq!(mix.id, catalog("pl.pm-mix1"));
    assert_eq!(mix.name, "Mix One");
    assert_eq!(mix.curator.as_deref(), Some("Curator"));
    assert!(!mix.can_append && !mix.in_library);
    assert!(mix.artwork.is_some());
    let recent = &groups[1];
    assert_eq!(recent.kind, RecommendationKind::Recent);
    assert_eq!(recent.albums[0].id, catalog("alb1"));
    assert_eq!(recent.albums[0].artist.as_deref(), Some("Artist"));
    assert!(!recent.albums[0].in_library);

    // The count limits the groups, and defaults to ten.
    assert_eq!(library.recommendations(1).await.unwrap().len(), 1);
    assert_eq!(library.recommendations(0).await.unwrap().len(), 0);
    assert_eq!(fixtures.seen_for("GET", "/v1/me/recommendations").len(), 2);
}

#[tokio::test]
async fn recommendations_are_marked_on_every_read_and_read_again_when_old() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/recommendations",
            fixture("recommendations_page1.json"),
        )
        .on(
            "GET",
            "/v1/me/recommendations",
            fixture("recommendations_page2.json"),
        )
        .on(
            "GET",
            "/v1/me/library/recently-added",
            Reply::json(json!({"data": []})),
        )
        .on(
            "GET",
            "/v1/me/library/albums",
            Reply::json(json!({"data": [
                {"id": "l.alb1", "type": "library-albums", "attributes": {
                    "name": "Album One", "artistName": "Artist",
                    "playParams": {"id": "l.alb1", "kind": "album", "catalogId": "alb1"}}}
            ]})),
        );
    let (library, clock) = signed_in(&fixtures);

    let first = library.recommendations(10).await.unwrap();
    assert_eq!(first.len(), 2);
    assert!(!first[1].albums[0].in_library);
    // The album list read meanwhile marks the album at once.
    library.albums().await.unwrap();
    let again = library.recommendations(10).await.unwrap();
    assert!(again[1].albums[0].in_library);
    // Read from Apple once: the first page and the one it names.
    assert_eq!(fixtures.seen_for("GET", "/v1/me/recommendations").len(), 2);

    // Old: served at once, refreshed in the background.
    clock.advance(TTL);
    assert_eq!(
        library
            .recommendations(DEFAULT_RECOMMENDATIONS)
            .await
            .unwrap()
            .len(),
        2
    );
    until(|| fixtures.seen_for("GET", "/v1/me/recommendations").len() == 3).await;
}

// Sign-in -----------------------------------------------------------------

#[tokio::test]
async fn new_tokens_forget_lists_pages_and_recommendations() {
    let fixtures = FixtureServer::start().await;
    serve_library(&fixtures);
    serve_search(&fixtures);
    serve_catalog_items(&fixtures);
    fixtures.on(
        "GET",
        "/v1/me/recommendations",
        fixture("recommendations_page2.json"),
    );
    let (library, _) = signed_in(&fixtures);
    library.songs().await.unwrap();
    library
        .recommendations(DEFAULT_RECOMMENDATIONS)
        .await
        .unwrap();
    library.catalog_album(&catalog("7002")).await.unwrap();
    let marked = || async {
        library
            .search_fresh("dawn", SearchScope::Catalog, false)
            .await
            .unwrap()
            .songs[0]
            .in_library
    };
    assert!(marked().await);

    library.sign_in(Tokens::new(DEVELOPER_TOKEN, "another-user-token").unwrap());
    assert!(!marked().await);
    library
        .recommendations(DEFAULT_RECOMMENDATIONS)
        .await
        .unwrap();
    library.catalog_album(&catalog("7002")).await.unwrap();
    assert_eq!(fixtures.seen_for("GET", "/v1/me/recommendations").len(), 2);
    assert_eq!(
        fixtures.seen_for("GET", "/v1/catalog/us/albums/7002").len(),
        2
    );
}

#[tokio::test]
async fn catalog_calls_need_a_sign_in() {
    let fixtures = FixtureServer::start().await;
    serve_search(&fixtures);
    let library = AppleLibrary::with_client(|builder| {
        builder
            .base_url(fixtures.base_url.clone())
            .storefront("us")
            .retry_policy(fast_retry())
    });
    let signed_out = |result: Result<(), LibraryError>| {
        assert!(matches!(result, Err(LibraryError::SignedOut)), "{result:?}");
    };
    signed_out(
        library
            .search_fresh("dawn", SearchScope::All, false)
            .await
            .map(drop),
    );
    signed_out(library.catalog_album(&catalog("7002")).await.map(drop));
    signed_out(library.catalog_artist(&catalog("8002")).await.map(drop));
    signed_out(
        library
            .catalog_playlist(&catalog("pl.morning"))
            .await
            .map(drop),
    );
    signed_out(
        library
            .recommendations(DEFAULT_RECOMMENDATIONS)
            .await
            .map(drop),
    );
    assert!(fixtures.seen().is_empty());
}

#[test]
fn item_ids_are_checked_and_round_trip_through_text() {
    for text in ["l.alb1", "7002", "catalog:7002", "catalog:l.alb1"] {
        let id: ItemId = text.parse().unwrap();
        assert_eq!(id.to_string(), text);
        assert_eq!(id.is_catalog(), text.starts_with("catalog:"));
    }
    assert_eq!(catalog("7002").catalog_id(), Some("7002"));
    assert_eq!(catalog("7002").library_id(), None);
    assert_eq!(library_item("l.1").library_id(), Some("l.1"));
    for text in ["", "catalog:", "catalog:catalog:1"] {
        assert!(text.parse::<ItemId>().is_err(), "{text:?}");
    }
    for id in ["", "catalog:1"] {
        assert!(ItemId::catalog(id).is_err(), "{id:?}");
        assert!(ItemId::library(id).is_err(), "{id:?}");
    }
}

// Editions ----------------------------------------------------------------

#[tokio::test]
async fn an_edition_with_another_catalog_id_is_not_the_librarys_album() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/library/search",
            fixture("library_search_editions.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/search",
            fixture("catalog_search_editions.json"),
        );
    let (library, _) = signed_in(&fixtures);

    // The library's album names catalog id 7010: the edition 7011 shares its name and
    // artist, and is another album.
    let all = library
        .search_fresh("dawn", SearchScope::All, false)
        .await
        .unwrap();
    assert_eq!(
        album_marks(&all.albums),
        [(library_item("l.alb9"), true), (catalog("7011"), false)]
    );
}

#[tokio::test]
async fn kept_albums_match_by_catalog_id_when_they_name_one() {
    let fixtures = FixtureServer::start().await;
    let album = |id: &str, name: &str, artist: &str| json!({"id": id, "type": "albums", "attributes": {"name": name, "artistName": artist}});
    fixtures
        .on(
            "GET",
            "/v1/me/library/recently-added",
            Reply::json(json!({"data": []})),
        )
        .on(
            "GET",
            "/v1/me/library/albums",
            Reply::json(json!({"data": [
                {"id": "l.alb9", "type": "library-albums", "attributes": {
                    "name": "Dawn Album", "artistName": "Artist One",
                    "playParams": {"id": "l.alb9", "kind": "album", "catalogId": "7010"}}},
                {"id": "l.nameless", "type": "library-albums", "attributes": {}}
            ]})),
        )
        .on(
            "GET",
            "/v1/me/library/artists",
            Reply::json(json!({"data": []})),
        )
        .on(
            "GET",
            "/v1/catalog/us/artists/9001",
            Reply::json(json!({"data": [
                {"id": "9001", "type": "artists", "attributes": {"name": "Artist One"}}
            ]})),
        )
        .on(
            "GET",
            "/v1/catalog/us/artists/9001/albums",
            Reply::json(json!({"data": [
                album("7010", "Dawn Album", "Artist One"),
                album("7011", "Dawn Album", "Artist One"),
                // Nothing is named by an empty name.
                album("7012", "", ""),
            ]})),
        );
    let (library, _) = signed_in(&fixtures);
    library.albums().await.unwrap();

    let detail = library.catalog_artist(&catalog("9001")).await.unwrap();
    let marked: Vec<_> = detail
        .albums
        .iter()
        .map(|album| (album.id.clone(), album.in_library))
        .collect();
    assert_eq!(
        marked,
        [
            (catalog("7010"), true),
            (catalog("7011"), false),
            (catalog("7012"), false)
        ]
    );
}

#[tokio::test]
async fn the_library_artists_are_kept_and_read_again_when_old() {
    let fixtures = FixtureServer::start().await;
    serve_catalog_items(&fixtures);
    let (library, clock) = signed_in(&fixtures);

    library.catalog_artist(&catalog("8002")).await.unwrap();
    // Further reads share the kept names.
    clock.advance(DETAIL_TTL);
    library.catalog_artist(&catalog("8002")).await.unwrap();
    assert_eq!(fixtures.seen_for("GET", "/v1/me/library/artists").len(), 1);
    clock.advance(TTL);
    library.catalog_artist(&catalog("8002")).await.unwrap();
    assert_eq!(fixtures.seen_for("GET", "/v1/me/library/artists").len(), 2);
}

#[tokio::test]
async fn a_catalog_artist_opens_when_the_library_artists_cannot_be_read() {
    let fixtures = FixtureServer::start().await;
    // The first read of the library's artists fails.
    fixtures.on(
        "GET",
        "/v1/me/library/artists",
        Reply::fixture_status(404, "error_not_found.json"),
    );
    serve_catalog_items(&fixtures);
    let (library, _) = signed_in(&fixtures);

    // The artist reads as not in the library.
    let detail = library.catalog_artist(&catalog("8002")).await.unwrap();
    assert_eq!(detail.artist.name, "Artist Nine");
    assert!(!detail.artist.in_library);
}

#[tokio::test]
async fn the_library_search_does_not_hide_the_catalog_behind_its_cap() {
    let fixtures = FixtureServer::start().await;
    let songs: Vec<_> = (0..30)
        .map(|n| {
            json!({"id": format!("i.many{n}"), "type": "library-songs",
                "attributes": {"name": format!("Many {n}"),
                    "playParams": {"id": format!("i.many{n}"), "kind": "song", "isLibrary": true}}})
        })
        .collect();
    fixtures
        .on(
            "GET",
            "/v1/me/library/search",
            Reply::json(json!({"results": {"library-songs": {"data": songs}}})),
        )
        .on(
            "GET",
            "/v1/catalog/us/search",
            fixture("catalog_search_dawn.json"),
        );
    let (library, _) = signed_in(&fixtures);

    let all = library
        .search_fresh("many", SearchScope::All, false)
        .await
        .unwrap();
    let listed = keys(&all.songs);
    assert_eq!(listed.len(), 27);
    assert_eq!(listed[24], "i.many24");
    assert_eq!(&listed[25..], ["catalog:1001", "catalog:6001"]);
    let own = library
        .search_fresh("many", SearchScope::Library, false)
        .await
        .unwrap();
    assert_eq!(own.songs.len(), 25);
}

// Sign-in during a call ---------------------------------------------------

fn another_account() -> Tokens {
    Tokens::new(DEVELOPER_TOKEN, "another-user-token").unwrap()
}

#[tokio::test]
async fn an_album_read_in_flight_when_the_tokens_change_is_read_again() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/catalog/us/albums/7002",
            fixture("catalog_album_sunrise.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/albums/7002",
            Reply::json(json!({"data": [{"id": "7002", "type": "albums",
                "attributes": {"name": "Sunrise Reissue"}}]})),
        )
        .on(
            "GET",
            "/v1/catalog/us/albums/7002/tracks",
            fixture("catalog_album_sunrise_tracks.json"),
        );
    fixtures.delay(
        "GET",
        "/v1/catalog/us/albums/7002",
        Duration::from_millis(500),
    );
    let (library, _) = signed_in(&fixtures);

    let reading = tokio::spawn({
        let library = library.clone();
        async move { library.catalog_album(&catalog("7002")).await }
    });
    until(|| {
        !fixtures
            .seen_for("GET", "/v1/catalog/us/albums/7002")
            .is_empty()
    })
    .await;
    library.sign_in(another_account());
    let detail = reading.await.unwrap().unwrap();

    // The answer is the second read's, and that is what was kept.
    assert_eq!(detail.album.name, "Sunrise Reissue");
    assert_eq!(
        fixtures.seen_for("GET", "/v1/catalog/us/albums/7002").len(),
        2
    );
    library.catalog_album(&catalog("7002")).await.unwrap();
    assert_eq!(
        fixtures.seen_for("GET", "/v1/catalog/us/albums/7002").len(),
        2
    );
}

#[tokio::test]
async fn recommendations_in_flight_when_the_tokens_change_are_read_again() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/recommendations",
            fixture("recommendations_page2.json"),
        )
        .on(
            "GET",
            "/v1/me/recommendations",
            Reply::json(
                json!({"data": [{"id": "rec-9", "type": "personal-recommendation",
                "attributes": {"title": {"stringForDisplay": "Fresh"}, "kind": "recently-played"},
                "relationships": {"contents": {"data": [
                    {"id": "alb9", "type": "albums", "attributes": {"name": "Nine"}}]}}}]}),
            ),
        );
    fixtures.delay("GET", "/v1/me/recommendations", Duration::from_millis(500));
    let (library, _) = signed_in(&fixtures);

    let reading = tokio::spawn({
        let library = library.clone();
        async move { library.recommendations(DEFAULT_RECOMMENDATIONS).await }
    });
    until(|| {
        !fixtures
            .seen_for("GET", "/v1/me/recommendations")
            .is_empty()
    })
    .await;
    library.sign_in(another_account());
    let groups = reading.await.unwrap().unwrap();

    let titles: Vec<&str> = groups.iter().map(|group| group.title.as_str()).collect();
    assert_eq!(titles, ["Fresh"]);
    assert_eq!(fixtures.seen_for("GET", "/v1/me/recommendations").len(), 2);
    assert_eq!(
        fixtures.seen_for("GET", "/v1/me/recommendations")[1]
            .user_token
            .as_deref(),
        Some("another-user-token")
    );
}

#[tokio::test]
async fn catalog_songs_name_their_catalog_artists_in_apples_order() {
    let fixtures = FixtureServer::start().await;
    let artist =
        |id: &str, name: &str| json!({"id": id, "type": "artists", "attributes": {"name": name}});
    fixtures.on(
        "GET",
        "/v1/catalog/us/songs",
        Reply::json(json!({"data": [
            {"id": "1", "type": "songs", "attributes": {"name": "Duet"},
                "relationships": {"artists": {"data": [artist("20", "Bob"), artist("10", "Ann")]}}},
            {"id": "2", "type": "songs", "attributes": {"name": "Alone"}},
        ]})),
    );
    let (library, _) = signed_in(&fixtures);

    let found = library
        .catalog_song_artists(&["1".to_owned(), "2".to_owned(), "3".to_owned()])
        .await
        .unwrap();

    let named: Vec<(&str, Vec<(ItemId, &str)>)> = found
        .iter()
        .map(|(song, artists)| {
            (
                song.as_str(),
                artists
                    .iter()
                    .map(|artist| (artist.id.clone(), artist.name.as_str()))
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        named,
        [
            ("1", vec![(catalog("20"), "Bob"), (catalog("10"), "Ann")]),
            ("2", Vec::new()),
        ]
    );
    let asked = fixtures.seen_for("GET", "/v1/catalog/us/songs");
    assert_eq!(asked.len(), 1);
    assert!(asked[0].target.contains("include=artists"));
    // The catalog is asked with the developer token only.
    assert!(asked[0].user_token.is_none());
}

fn view_release(id: &str, name: &str, is_single: Option<bool>) -> serde_json::Value {
    json!({"id": id, "type": "albums", "attributes": {
        "name": name, "artistName": "Artist Nine", "isSingle": is_single,
        "releaseDate": "2020-05-01", "trackCount": 3
    }})
}

fn view_song(id: &str, name: &str) -> serde_json::Value {
    json!({"id": id, "type": "songs", "attributes": {"name": name, "artistName": "Artist Nine"}})
}

#[tokio::test]
async fn an_artist_page_reads_every_view_in_one_request_and_pages_on_by_next() {
    let fixtures = FixtureServer::start().await;
    serve_search(&fixtures);
    fixtures
        .on(
            "GET",
            "/v1/catalog/us/artists/8002",
            Reply::json(json!({"data": [{
                "id": "8002", "type": "artists",
                "attributes": {"name": "Artist Nine"},
                "views": {
                    "full-albums": {"next": "/v1/catalog/us/artists/8002/view/full-albums?offset=1",
                        "data": [view_release("1", "First", Some(false))]},
                    "singles": {"data": [
                        view_release("2", "Midnight - EP", Some(true)),
                        view_release("3", "Hello - Single", Some(true)),
                        view_release("4", "Plain Name", Some(false)),
                    ]},
                    "live-albums": {"data": []},
                    "top-songs": {"next": "/v1/catalog/us/artists/8002/view/top-songs?offset=2",
                        "data": [view_song("9001", "Hit"), {"id": "v1", "type": "music-videos"}]}
                }
            }]})),
        )
        .on(
            "GET",
            "/v1/catalog/us/artists/8002/view/full-albums",
            Reply::json(json!({"data": [view_release("5", "Second", Some(false))]})),
        )
        .on(
            "GET",
            "/v1/catalog/us/artists/8002/view/top-songs",
            Reply::json(json!({"data": [view_song("9002", "Second Hit")]})),
        );
    let (library, clock) = signed_in(&fixtures);

    let page = library
        .catalog_artist_page_in(&catalog("8002"), None)
        .await
        .unwrap();

    assert_eq!(page.artist.name, "Artist Nine");
    // Only the views that hold releases are listed, in the order of `ReleaseView::ALL`.
    let views: Vec<_> = page.views.iter().map(|(view, _)| *view).collect();
    assert_eq!(views, [ReleaseView::FullAlbums, ReleaseView::Singles]);
    assert_eq!(
        page.views[0].1.next.as_deref(),
        Some("/v1/catalog/us/artists/8002/view/full-albums?offset=1")
    );
    // The singles view holds EPs too: the name decides, and `isSingle` where it says neither.
    let eps: Vec<_> = page.views[1]
        .1
        .releases
        .iter()
        .map(|release| (release.album.id.to_string(), release.ep))
        .collect();
    assert_eq!(
        eps,
        [
            ("catalog:2".to_owned(), true),
            ("catalog:3".to_owned(), false),
            ("catalog:4".to_owned(), true)
        ]
    );
    assert!(page.views[0].1.releases.iter().all(|release| !release.ep));
    assert_eq!(
        page.top_songs
            .songs
            .iter()
            .map(|song| song.key.to_string())
            .collect::<Vec<_>>(),
        ["catalog:9001"]
    );
    let asked = fixtures.seen_for("GET", "/v1/catalog/us/artists/8002");
    assert_eq!(asked.len(), 1);
    assert_eq!(
        asked[0].target,
        "/v1/catalog/us/artists/8002?views=latest-release,full-albums,singles,featured-albums,live-albums,compilation-albums,appears-on-albums,top-songs"
    );

    let more = library
        .catalog_releases_after(
            ReleaseView::FullAlbums,
            page.views[0].1.next.as_deref().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(more.releases[0].album.id, catalog("5"));
    assert_eq!(more.next, None);
    let more = library
        .catalog_top_songs_after(page.top_songs.next.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!(more.songs[0].key.to_string(), "catalog:9002");
    assert_eq!(more.next, None);

    // The first page is kept for a while, and read again once old.
    library
        .catalog_artist_page_in(&catalog("8002"), None)
        .await
        .unwrap();
    assert_eq!(
        fixtures
            .seen_for("GET", "/v1/catalog/us/artists/8002")
            .len(),
        1
    );
    clock.advance(DETAIL_TTL);
    library
        .catalog_artist_page_in(&catalog("8002"), None)
        .await
        .unwrap();
    assert_eq!(
        fixtures
            .seen_for("GET", "/v1/catalog/us/artists/8002")
            .len(),
        2
    );
    assert_tokens(&fixtures);
}

#[tokio::test]
async fn an_artist_page_that_apple_does_not_know_is_not_found() {
    let fixtures = FixtureServer::start().await;
    let (library, _) = signed_in(&fixtures);
    assert_not_found(
        library
            .catalog_artist_page_in(&library_item("r.art1"), None)
            .await,
    );
}

#[tokio::test]
async fn an_artist_page_asks_for_a_language_and_the_storefront_languages_are_read_once() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/storefronts/us",
            Reply::fixture("storefront.json"),
        )
        .on(
            "GET",
            "/v1/catalog/us/artists/8002",
            Reply::json(json!({"data": [{
                "id": "8002", "type": "artists",
                "attributes": {"name": "Artist Nine"},
                "views": {"full-albums": {"data": [view_release("1", "First", Some(false))]}}
            }]})),
        );
    let (library, _clock) = signed_in(&fixtures);

    let languages = library.storefront_languages().await.unwrap();
    library.storefront_languages().await.unwrap();
    assert_eq!(languages.default.as_deref(), Some("en-US"));
    assert_eq!(languages.supported, ["en-US", "es-MX"]);
    assert_eq!(fixtures.seen_for("GET", "/v1/storefronts/us").len(), 1);

    library
        .catalog_artist_page_in(&catalog("8002"), None)
        .await
        .unwrap();
    library
        .catalog_artist_page_in(&catalog("8002"), Some("en-GB"))
        .await
        .unwrap();
    library
        .catalog_artist_page_in(&catalog("8002"), Some("en-GB"))
        .await
        .unwrap();
    let asked = fixtures.seen_for("GET", "/v1/catalog/us/artists/8002");
    // One read without a language, one with, and the second with is kept.
    assert_eq!(asked.len(), 2);
    assert!(!asked[0].target.contains("l="));
    assert!(asked[1].target.ends_with("&l=en-GB"));
}

#[tokio::test]
async fn a_repeated_album_id_is_answered_each_time() {
    let fixtures = FixtureServer::start().await;
    fixtures.on(
        "GET",
        "/v1/catalog/us/albums",
        Reply::json(json!({"data": [{
            "id": "7002", "type": "albums",
            "attributes": {"name": "Sunrise EP", "artistName": "Artist Nine"}
        }]})),
    );
    let (library, _) = signed_in(&fixtures);

    let albums = library
        .catalog_albums(&[catalog("7002"), catalog("404"), catalog("7002")])
        .await
        .unwrap();

    assert_eq!(albums.len(), 2);
}

#[tokio::test]
async fn an_artist_page_marks_an_artist_the_library_has() {
    let fixtures = FixtureServer::start().await;
    fixtures
        .on(
            "GET",
            "/v1/me/library/artists",
            Reply::json(json!({"data": [
                {"id": "r.art9", "type": "library-artists", "attributes": {"name": "Artist Nine"}}
            ]})),
        )
        .on(
            "GET",
            "/v1/catalog/us/artists/8002",
            Reply::json(json!({"data": [{
                "id": "8002", "type": "artists",
                "attributes": {"name": "Artist Nine"},
                "views": {}
            }]})),
        )
        .on(
            "GET",
            "/v1/catalog/us/artists/8002/albums",
            Reply::json(json!({"data": []})),
        );
    let (library, _) = signed_in(&fixtures);
    // The artist's detail reads the library's artists, which the page then marks by.
    assert!(
        library
            .catalog_artist(&catalog("8002"))
            .await
            .unwrap()
            .artist
            .in_library
    );

    let page = library
        .catalog_artist_page_in(&catalog("8002"), None)
        .await
        .unwrap();

    assert!(page.artist.in_library);
}

#[tokio::test]
async fn a_library_artist_found_by_search_keeps_its_picture() {
    let fixtures = FixtureServer::start().await;
    fixtures.on(
        "GET",
        "/v1/me/library/search",
        Reply::json(json!({"results": {"library-artists": {"data": [{
            "id": "r.art1", "type": "library-artists",
            "attributes": {"name": "Artist One"},
            "relationships": {"catalog": {"data": [{
                "id": "8001", "type": "artists",
                "attributes": {"name": "Artist One", "artwork": {
                    "width": 600, "height": 600, "url": "http://artwork.invalid/{w}x{h}.jpg"
                }}
            }]}}
        }]}}})),
    );
    let (library, _) = signed_in(&fixtures);

    let found = library
        .search_fresh("one", SearchScope::Library, false)
        .await
        .unwrap();

    assert!(found.artists[0].artwork.is_some());
}
