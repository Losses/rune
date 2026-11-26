// Searching an Apple Music library: the library's matches and the catalog's, searched together
// through the fake's search.

use crate::messages::{SearchField, SearchForRequest, SearchForResponse};

const LIBRARY_SEARCH: &str = "/v1/me/library/search";
const CATALOG_SEARCH: &str = "/v1/catalog/it/search";

fn search_request(n: i32, fields: Vec<SearchField>, refresh: bool) -> SearchForRequest {
    SearchForRequest {
        call_id: None,
        query_str: "dawn*".into(),
        fields,
        n,
        refresh,
    }
}

async fn search_for(apple: &OpenApple) -> Result<SearchForResponse> {
    apple.run(&search_request(30, Vec::new(), false)).await
}

/// The tracks the media files of `ids` name, in order.
async fn track_keys(apple: &OpenApple, ids: &[i32]) -> Result<Vec<String>> {
    Ok(apple
        .run(&FetchMediaFileByIdsRequest {
            call_id: None,
            ids: ids.to_vec(),
            bake_cover_arts: false,
        })
        .await?
        .media_files
        .into_iter()
        .map(|file| file.track)
        .collect())
}

/// Apple's ids of the albums or artists `ids` name, as `kind`.
fn remotes(apple: &OpenApple, kind: AppleIdKind, ids: &[i32]) -> Result<Vec<String>> {
    ids.iter()
        .map(|id| apple.binding.resolve(kind, *id))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_search_lists_the_librarys_matches_then_the_catalogs_and_each_song_once() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let search = search_for(&apple).await?;

    // The library's song comes first. The catalog's copy of it is that song, and the catalog's
    // others follow, as Apple ordered them.
    assert_eq!(
        track_keys(&apple, &search.tracks).await?,
        ["i.s1", "catalog:900", "catalog:1603"]
    );
    // The library's "Dawn Album" names no catalog album, so the catalog's album of the same
    // name and artist may be another edition: both are listed, the library's first.
    assert_eq!(
        remotes(&apple, AppleIdKind::Album, &search.albums)?,
        ["l.alb1", "catalog:1500", "catalog:1600"]
    );
    assert_eq!(
        collection_names(&apple, CollectionType::Album, &search.albums).await?,
        ["Dawn Album", "Dawn Album", "Dawn Remixes"]
    );
    // The artists are those of the library's artist list, then the catalog's others: the
    // library's "Artist One" and the catalog artist it is are one. "Dawn Singer" is no artist of
    // the library, and is the catalog artist, which opens its catalog page.
    assert_eq!(
        remotes(&apple, AppleIdKind::Artist, &search.artists)?,
        ["artist:300", "catalog:301"]
    );
    assert_eq!(
        collection_names(&apple, CollectionType::Artist, &search.artists).await?,
        ["Artist One", "Dawn Singer"]
    );
    // The playlists are Rune's own, and none is called "dawn".
    assert!(search.playlists.is_empty());
    // Both were searched once, and what the search found is named without asking again.
    assert_eq!(apple.fake.count(LIBRARY_SEARCH), 1);
    assert_eq!(apple.fake.count(CATALOG_SEARCH), 1);
    let read_for_it = |(target, _): &(String, bool)| {
        target.starts_with("/v1/catalog/it/albums")
            || (target.starts_with("/v1/catalog/it/songs")
                && ["900", "1603"].iter().any(|id| target.contains(id)))
    };
    assert!(!apple.fake.seen.lock().unwrap().iter().any(read_for_it));
    // The term Apple was asked for has no wildcard.
    assert_eq!(
        apple.fake.count(&format!(
            "{CATALOG_SEARCH}?term=dawn&types=songs,albums,artists"
        )),
        1
    );

    // A field limits the answer to its kind.
    let tracks = apple
        .run(&search_request(30, vec![SearchField::Track], false))
        .await?;
    assert_eq!(tracks.tracks, search.tracks);
    assert!(tracks.albums.is_empty() && tracks.artists.is_empty() && tracks.playlists.is_empty());
    // The count asked for is taken from each part: the catalog's first song is
    // the library's own, and its first album is no copy.
    let one = apple.run(&search_request(1, Vec::new(), false)).await?;
    assert_eq!(one.tracks, search.tracks[..1]);
    assert_eq!(one.albums, search.albums[..2]);
    // The same search again asks the library, and takes what the catalog found before.
    assert_eq!(apple.fake.count(LIBRARY_SEARCH), 3);
    assert_eq!(apple.fake.count(CATALOG_SEARCH), 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refreshed_search_asks_apple_again_and_the_next_one_uses_that_answer() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let first = apple.run(&search_request(10, Vec::new(), false)).await?;
    apple.run(&search_request(10, Vec::new(), false)).await?;
    assert_eq!(apple.fake.count(CATALOG_SEARCH), 1);
    let refreshed = apple.run(&search_request(10, Vec::new(), true)).await?;
    assert_eq!(apple.fake.count(CATALOG_SEARCH), 2);
    assert_eq!(refreshed.tracks, first.tracks);
    apple.run(&search_request(10, Vec::new(), false)).await?;
    assert_eq!(apple.fake.count(CATALOG_SEARCH), 2);

    // What was kept is a while's: after it, the catalog is asked again.
    apple.clock.advance(std::time::Duration::from_secs(121));
    apple.run(&search_request(10, Vec::new(), false)).await?;
    assert_eq!(apple.fake.count(CATALOG_SEARCH), 3);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_catalog_search_leaves_the_librarys_matches() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    *apple.fake.respond_path.lock().unwrap() = Some((
        CATALOG_SEARCH.into(),
        400,
        r#"{"errors":[{"status":"400","code":"40000"}]}"#.into(),
    ));
    let search = search_for(&apple).await?;
    assert_eq!(track_keys(&apple, &search.tracks).await?, ["i.s1"]);
    assert_eq!(
        remotes(&apple, AppleIdKind::Album, &search.albums)?,
        ["l.alb1"]
    );
    // A failure is not kept: the next search asks the catalog again.
    *apple.fake.respond_path.lock().unwrap() = None;
    let again = search_for(&apple).await?;
    assert_eq!(again.tracks.len(), 3);
    assert_eq!(apple.fake.count(CATALOG_SEARCH), 2);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_library_search_leaves_the_catalogs_matches_as_the_copies_the_library_holds()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    *apple.fake.respond_path.lock().unwrap() = Some((
        LIBRARY_SEARCH.into(),
        500,
        r#"{"errors":[{"status":"500","code":"50000"}]}"#.into(),
    ));
    // The albums page was shown, so the album list is kept.
    apple
        .run(&FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Album,
        })
        .await?;
    let search = search_for(&apple).await?;
    // The song list still knows the library's copy of the catalog's song. The album list holds
    // no album that names a catalog album, so the catalog's albums are the catalog's.
    assert_eq!(
        track_keys(&apple, &search.tracks).await?,
        ["i.s1", "catalog:900", "catalog:1603"]
    );
    assert_eq!(
        remotes(&apple, AppleIdKind::Album, &search.albums)?,
        ["catalog:1500", "catalog:1600"]
    );
    assert_eq!(
        remotes(&apple, AppleIdKind::Artist, &search.artists)?,
        ["artist:300", "catalog:301"]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_search_whose_sign_in_apple_rejects_fails_as_a_sign_in_to_renew() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    apple.fake.reject_everything.store(true, Ordering::SeqCst);
    let refused = search_for(&apple)
        .await
        .err()
        .context("a rejected search")?;
    assert!(
        matches!(
            refused.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::ServiceSignInRequired { .. })
        ),
        "{refused:#}"
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_catalog_search_is_waited_for_and_a_repeated_one_is_answered_at_once() -> Result<()>
{
    use crate::apple_music_translate::search::SEARCH_WAIT;
    use std::time::Duration;

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // Apple's catalog takes long to answer, as on a slow network.
    *apple.fake.delay_path.lock().unwrap() = Some((CATALOG_SEARCH.into(), Duration::from_secs(4)));
    let started = Instant::now();
    let search = search_for(&apple).await?;
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_secs(4) && waited < SEARCH_WAIT,
        "waited {waited:?}"
    );
    assert_eq!(
        track_keys(&apple, &search.tracks).await?,
        ["i.s1", "catalog:900", "catalog:1603"]
    );

    // The same search again takes what the catalog found, and asks it nothing.
    let started = Instant::now();
    let again = search_for(&apple).await?;
    assert!(started.elapsed() < Duration::from_secs(4));
    assert_eq!(
        (&again.tracks, &again.albums, &again.artists),
        (&search.tracks, &search.albums, &search.artists)
    );
    assert_eq!(apple.fake.count(CATALOG_SEARCH), 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn search_finds_rune_playlists_by_name_in_the_order_they_were_made() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            playlists: true,
            ..AppleOptions::default()
        },
    )
    .await?;
    let road_songs = make_rune_playlist(&apple, "Road Songs", "Mood").await?;
    make_rune_playlist(&apple, "Dawn Walk", "Mood").await?;
    let road_trip = playlist_named(&apple, "Road Trip").await?;
    let search = |n| SearchForRequest {
        call_id: None,
        query_str: "ROAD".into(),
        fields: Vec::new(),
        n,
        refresh: false,
    };
    // Rune's are the only playlists found, "Road Trip" first as the sign-in imported it.
    assert_eq!(
        apple.run(&search(10)).await?.playlists,
        [road_trip, road_songs.id]
    );
    assert_eq!(apple.run(&search(1)).await?.playlists, [road_trip]);
    let opened = apple
        .run(&FetchCollectionByIdsRequest {
            call_id: None,
            collection_type: CollectionType::Playlist,
            bake_cover_arts: false,
            ids: vec![road_songs.id],
            queue: CallerQueue::default(),
        })
        .await?;
    assert_eq!(
        (opened.result[0].name.as_str(), opened.result[0].readonly),
        ("Road Songs", false)
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_search_for_tracks_alone_reads_no_artist_list() -> Result<()> {
    const ARTISTS: &str = "/v1/me/library/artists";
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let artists = apple.fake.count(ARTISTS);
    let tracks = apple
        .run(&search_request(30, vec![SearchField::Track], false))
        .await?;
    assert!(!tracks.tracks.is_empty());
    assert!(tracks.artists.is_empty());
    assert_eq!(apple.fake.count(ARTISTS), artists);
    // A search for artists reads it.
    let found = apple
        .run(&search_request(30, vec![SearchField::Artist], false))
        .await?;
    assert!(!found.artists.is_empty());
    assert_eq!(apple.fake.count(ARTISTS), artists + 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_search_for_playlists_alone_finds_runes_signed_out_without_asking_apple() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let road_songs = make_rune_playlist(&apple, "Road Songs", "Mood").await?;
    apple.binding.sign_out();
    let playlists = SearchForRequest {
        call_id: None,
        query_str: "road*".into(),
        fields: vec![SearchField::Playlist],
        n: 10,
        refresh: false,
    };
    assert_eq!(apple.run(&playlists).await?.playlists, [road_songs.id]);
    assert_eq!(apple.fake.count(LIBRARY_SEARCH), 0);
    assert_eq!(apple.fake.count(CATALOG_SEARCH), 0);
    // A search for more than playlists still needs Apple.
    assert!(
        apple
            .run(&search_request(10, Vec::new(), false))
            .await
            .is_err()
    );
    apple.finish().await
}

/// The parsed media file of the song `id`, as the menu of its row reads it.
async fn parsed(
    apple: &OpenApple,
    id: i32,
) -> Result<crate::messages::FetchParsedMediaFileResponse> {
    apple
        .run(&FetchParsedMediaFileRequest { call_id: None, id })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_catalog_song_found_by_search_opens_its_album_and_asks_for_it_once() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let search = apple
        .run(&search_request(30, vec![SearchField::Track], false))
        .await?;
    let id = *search.tracks.last().context("the catalog song found")?;
    assert_eq!(apple.binding.resolve_track(id)?, "catalog:1603");
    let first = parsed(&apple, id).await?;
    assert_eq!(first.file.title, "Dawn Bonus");
    assert_eq!(
        apple.binding.resolve(AppleIdKind::Album, first.album.id)?,
        "catalog:1600"
    );
    // The song's credit still names its artist as the library's artist list does.
    assert_eq!(first.artists.len(), 1);
    assert_eq!(first.artists[0].name, "Dawn Singer");
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/songs?ids=1603&include=albums"),
        1
    );
    // The album's page lists the album's songs.
    let page = apple.run(&mix("lib::album", first.album.id)).await?;
    assert!(
        page.files.iter().any(|file| file.title == "Remix One"),
        "{:?}",
        page.files
    );
    // The song keeps its album: asking again reads nothing.
    let asked = apple.fake.total();
    assert_eq!(parsed(&apple, id).await?.album.id, first.album.id);
    assert_eq!(apple.fake.total(), asked);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_album_read_leaves_the_searched_song_without_an_album() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let search = apple
        .run(&search_request(30, vec![SearchField::Track], false))
        .await?;
    let id = *search.tracks.last().context("the catalog song found")?;
    *apple.fake.respond_path.lock().unwrap() =
        Some(("/v1/catalog/it/songs".into(), 500, "{}".into()));
    let parsed = parsed(&apple, id).await?;
    assert_eq!(parsed.album.id, 0);
    assert_eq!(parsed.file.track, "catalog:1603");
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_a_search_kept_goes_with_the_sign_out() -> Result<()> {
    use crate::messages::{
        ServiceSignIn, SignInServiceLibraryRequest, SignOutServiceLibraryRequest,
    };
    use crate::utils::{sign_in_service_library_on_node, sign_out_service_library_on_node};

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    search_for(&apple).await?;
    assert_eq!(apple.fake.count(CATALOG_SEARCH), 1);
    let record_id = apple.record.0.to_string();
    sign_out_service_library_on_node(
        &SignOutServiceLibraryRequest {
            record_id: record_id.clone(),
            call_id: None,
        },
        &apple.node,
    )
    .await?;
    sign_in_service_library_on_node(
        &SignInServiceLibraryRequest {
            record_id,
            sign_in: ServiceSignIn::Platform {
                user_token: USER_TOKEN.into(),
                developer_token: Some(DEVELOPER_TOKEN.into()),
            },
            call_id: None,
        },
        &apple.node,
    )
    .await?;
    search_for(&apple).await?;
    assert_eq!(apple.fake.count(CATALOG_SEARCH), 2);
    apple.finish().await
}
