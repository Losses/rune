// The page of an artist of an Apple Music library: the catalog's page of a catalog artist, read
// through its views, and the library's page of an entry of the artist list, placed by what the
// catalog says of its albums.

use std::time::Duration;

use crate::apple_music_classes::{AlbumFacts, ArtistViews, Classes, VIEWS_TTL};
use crate::apple_music_translate::artist_page::classify_in_background;
use crate::messages::{
    ArtistSectionKind, ArtistSectionOpens, FetchArtistPageRequest, FetchArtistPageResponse,
    FetchArtistSectionAlbumsRequest, RemoveMixRequest, SearchField, SearchForRequest,
};

/// The id a catalog artist found by search is numbered by: "Viewed Artist" of the fake.
fn viewed_artist(apple: &OpenApple) -> Result<i32> {
    apple.binding.intern(AppleIdKind::Artist, "catalog:310")
}

/// The request for the page of an artist from a client whose language is unknown, so the
/// sections come untitled.
fn artist_page_request(artist_id: i32) -> FetchArtistPageRequest {
    artist_page_in(artist_id, "")
}

/// The request for the page of an artist from a client in `language`.
fn artist_page_in(artist_id: i32, language: &str) -> FetchArtistPageRequest {
    FetchArtistPageRequest {
        call_id: None,
        artist_id,
        language: language.to_owned(),
    }
}

/// The songs of a page of a mix, by the tracks they play by.
async fn page_keys(
    apple: &OpenApple,
    operator: &str,
    parameter: &str,
    cursor: i32,
    size: i32,
) -> Result<Vec<String>> {
    let mut page = mix(operator, parameter);
    page.cursor = cursor;
    page.page_size = size;
    Ok(apple
        .run(&page)
        .await?
        .files
        .into_iter()
        .map(|file| file.track)
        .collect())
}

/// A page of the albums of a section of the page of `artist` in its groups, each title with
/// the ids the library gives the albums.
async fn section_album_groups(
    apple: &OpenApple,
    artist: i32,
    section: &str,
    cursor: i32,
    page_size: i32,
) -> Result<Vec<(String, Vec<i32>)>> {
    Ok(apple
        .run(&FetchArtistSectionAlbumsRequest {
            call_id: None,
            queries: vec![MixQuery {
                operator: "lib::artist_section".into(),
                parameter: format!("{artist}:{section}"),
            }],
            cursor,
            page_size,
        })
        .await?
        .groups
        .into_iter()
        .map(|group| (group.group_title, group.album_ids))
        .collect())
}

/// A page of the albums of a section of the page of `artist`, by the ids the library gives,
/// whatever groups they are in.
async fn section_albums(
    apple: &OpenApple,
    artist: i32,
    section: &str,
    cursor: i32,
    page_size: i32,
) -> Result<Vec<i32>> {
    Ok(
        section_album_groups(apple, artist, section, cursor, page_size)
            .await?
            .into_iter()
            .flat_map(|(_, ids)| ids)
            .collect(),
    )
}

/// The names the collection translation gives these album ids.
async fn album_names(apple: &OpenApple, ids: Vec<i32>) -> Result<Vec<String>> {
    Ok(apple
        .run(&FetchCollectionByIdsRequest {
            call_id: None,
            collection_type: CollectionType::Album,
            bake_cover_arts: false,
            ids,
        })
        .await?
        .result
        .into_iter()
        .map(|album| album.name)
        .collect())
}

/// The catalog's facts, views and titles as the binding keeps them.
async fn classes(apple: &OpenApple) -> Arc<Classes> {
    apple.binding.classes.get(&apple.binding).await
}

/// Changes what the binding keeps of the catalog's facts, views and titles, as a read does.
async fn change_classes(apple: &OpenApple, change: impl FnOnce(&mut Classes)) {
    let generation = apple.binding.classes.generation();
    assert!(
        apple
            .binding
            .classes
            .change(&apple.binding, generation, change)
            .await
    );
}

/// The time by the library's clock, in seconds since the Unix epoch.
fn now(apple: &OpenApple) -> u64 {
    apple.binding.epoch_seconds()
}

fn kinds() -> AppleOptions {
    AppleOptions {
        kinds: true,
        ..AppleOptions::default()
    }
}

fn lost_tape() -> AppleOptions {
    AppleOptions {
        kinds: true,
        lost_tape: true,
        ..AppleOptions::default()
    }
}

/// Waits until the background read of the catalog's facts of the library's albums has kept
/// those of `ids`.
async fn wait_synced(apple: &OpenApple, ids: &[&str]) -> Result<()> {
    // The album list is kept when something first asks for it.
    apple.binding.albums().await?;
    wait_until("the albums to be classified", || async {
        let classes = classes(apple).await;
        Ok(ids
            .iter()
            .all(|id| classes.albums.contains_key(*id))
            .then_some(()))
    })
    .await
}

/// Opens the page of a library artist in `language` until its sections are titled, which the
/// first page starts reading in the background.
async fn wait_titled(
    apple: &OpenApple,
    artist: i32,
    language: &str,
) -> Result<FetchArtistPageResponse> {
    wait_until("the sections to be titled", || async {
        let page = apple.run(&artist_page_in(artist, language)).await?;
        Ok(
            (!page.sections.is_empty() && page.sections.iter().all(|s| !s.title.is_empty()))
                .then_some(page),
        )
    })
    .await
}

const KINDS_ALBUMS: [&str; 4] = ["4002", "4003", "4004", "4005"];

/// The kind and count of each section of a page.
fn section_counts(page: &FetchArtistPageResponse) -> Vec<(ArtistSectionKind, i32)> {
    page.sections
        .iter()
        .map(|section| (section.kind, section.count))
        .collect()
}

/// Gives "Short EP" the facts of an album: Apple does not call it a single, and its name does
/// not say EP. The catalog still lists it among the singles, as a label that reported an EP
/// under a plain name has it.
async fn short_ep_looks_like_an_album(apple: &OpenApple) {
    change_classes(apple, |classes| {
        let facts = classes
            .albums
            .get_mut("4003")
            .expect("the facts of Short EP");
        facts.name = "Short EP".into();
        facts.single = Some(false);
    })
    .await;
}

/// The section of each release the views of "Artist One" list (see `kinds_artist`).
const ARTIST_ONE_VIEWS: [(&str, &str); 6] = [
    ("4001", "albums"),
    ("4002", "albums"),
    ("4004", "singles"),
    ("4003", "singles"),
    ("4005", "compilations"),
    ("4006", "appears_on"),
];

/// Keeps `sections` as what the views of "Artist One" list, read at `read`, as an earlier page
/// read them. Kept empty and young, they leave every album to its facts, and a page opened
/// meanwhile reads none.
async fn keep_views(apple: &OpenApple, sections: &[(&str, &str)], read: u64) {
    let sections = sections
        .iter()
        .map(|(id, section)| ((*id).to_owned(), (*section).to_owned()))
        .collect();
    change_classes(apple, |classes| {
        classes.storefront = Some("it".into());
        classes.keep_views("it", "300", ArtistViews { sections, read });
    })
    .await;
}

/// When the views of "Artist One" were read, as kept; `None` when they are not.
async fn views_read(apple: &OpenApple) -> Option<u64> {
    classes(apple).await.views_of("300").map(|views| views.read)
}

/// The tracks of an artist's page, sorted.
async fn artist_track_keys(apple: &OpenApple, artist: i32) -> Result<Vec<String>> {
    let mut keys = page_keys(apple, "lib::artist_tracks", &artist.to_string(), 0, 50).await?;
    keys.sort();
    Ok(keys)
}

/// The artists a search for "dawn" finds, as numbered.
async fn searched_artists(apple: &OpenApple) -> Result<Vec<i32>> {
    Ok(apple
        .run(&SearchForRequest {
            call_id: None,
            query_str: "dawn*".into(),
            fields: vec![SearchField::Artist],
            n: 30,
            refresh: false,
        })
        .await?
        .artists)
}

// The catalog's page -------------------------------------------------------------------------

#[test]
fn apple_music_answers_the_artist_page_and_the_albums_of_its_sections() {
    let slots = RequestSlots::apple_music();
    assert_eq!(FetchArtistPageRequest::slot(&slots), Slot::Translate);
    assert_eq!(
        FetchArtistSectionAlbumsRequest::slot(&slots),
        Slot::Translate
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_albums_of_a_catalog_section_come_by_year_by_the_ids_catalog_albums_have() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let artist = viewed_artist(&apple)?;
    apple.run(&artist_page_request(artist)).await?;
    let numbered = |id: &str| apple.binding.intern(AppleIdKind::Album, id);

    let group = |year: &str, ids: &[&str]| -> Result<(String, Vec<i32>)> {
        let ids = ids.iter().map(|id| numbered(id)).collect::<Result<_>>()?;
        Ok((year.to_owned(), ids))
    };

    // The albums view lists Alpha (2019), Beta (2021) and, on its second page, Gamma (2019).
    // The albums are in groups by year, the newest first, so the first page reads the view to
    // its end; Apple's order stays within a year.
    let first = section_album_groups(&apple, artist, "albums", 0, 2).await?;
    assert_eq!(
        first,
        [
            group("2021", &["catalog:2002"])?,
            group("2019", &["catalog:2001"])?,
        ]
    );
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/artists/310/view/full-albums?offset=2"),
        1
    );
    // The next page goes on with 2019, under the same title.
    let total = apple.fake.total();
    let last = section_album_groups(&apple, artist, "albums", 2, 5).await?;
    assert_eq!(last, [group("2019", &["catalog:2003"])?]);
    // Reading again asks Apple nothing, and a page past the end is empty.
    assert_eq!(
        section_albums(&apple, artist, "albums", 1, 2).await?.len(),
        2
    );
    assert!(
        section_albums(&apple, artist, "albums", 3, 5)
            .await?
            .is_empty()
    );
    assert_eq!(apple.fake.total(), total);
    // The collection translation resolves them.
    let ids: Vec<i32> = [first, last]
        .concat()
        .into_iter()
        .flat_map(|(_, ids)| ids)
        .collect();
    assert_eq!(album_names(&apple, ids).await?, ["Beta", "Alpha", "Gamma"]);

    // The essential albums are Apple's picks in Apple's order, the older Alpha first, so they
    // are one group under the section's own title, which an empty title stands for.
    let essential = section_album_groups(&apple, artist, "essential_albums", 0, 10).await?;
    assert_eq!(essential, [group("", &["catalog:2001", "catalog:2002"])?]);
    assert_eq!(
        album_names(&apple, essential[0].1.clone()).await?,
        ["Alpha", "Beta"]
    );
    // A page cuts the group as any other, and the next goes on with it.
    assert_eq!(
        section_album_groups(&apple, artist, "essential_albums", 1, 10).await?,
        [group("", &["catalog:2002"])?]
    );
    // The singles section opens as songs, but its albums are listed the same. Apple gives them
    // no release date, so they are under "#", in Apple's order.
    let singles = section_album_groups(&apple, artist, "singles", 0, 10).await?;
    assert_eq!(
        singles,
        [group(
            "#",
            &["catalog:2010", "catalog:2011", "catalog:2012"]
        )?]
    );
    assert_eq!(
        album_names(&apple, singles[0].1.clone()).await?,
        ["Spark - Single", "Glow - EP", "Odd Name"]
    );

    // The latest release is one album, which is the one the page names and its songs open.
    let latest = section_albums(&apple, artist, "latest_release", 0, 10).await?;
    assert_eq!(latest, [numbered("catalog:2011")?]);
    assert!(
        section_albums(&apple, artist, "latest_release", 1, 10)
            .await?
            .is_empty()
    );
    assert_eq!(
        page_keys(
            &apple,
            "lib::artist_section",
            &format!("{artist}:latest_release"),
            0,
            10
        )
        .await?,
        ["catalog:2106", "catalog:2107"]
    );
    // A request that names no section of a page is refused, and so is a page that would start
    // before the first album or hold fewer than none.
    let refused = apple
        .run(&FetchArtistSectionAlbumsRequest {
            call_id: None,
            queries: vec![MixQuery {
                operator: "lib::artist_tracks".into(),
                parameter: artist.to_string(),
            }],
            cursor: 0,
            page_size: 5,
        })
        .await
        .err()
        .context("not a section")?;
    assert_eq!(
        classify(&refused),
        crate::messages::RequestErrorKind::InvalidInput
    );
    for (cursor, page_size) in [(-1, 5), (0, -5)] {
        let refused = section_album_groups(&apple, artist, "albums", cursor, page_size)
            .await
            .err()
            .context("a negative cursor or page size")?;
        assert_eq!(
            classify(&refused),
            crate::messages::RequestErrorKind::InvalidInput
        );
    }
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_artist_page_lists_its_sections_in_order_with_eps_and_singles_in_one() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let artist = viewed_artist(&apple)?;

    let page = apple.run(&artist_page_in(artist, "en")).await?;

    assert_eq!(page.artist_id, artist);
    // Compilations and live albums hold nothing, and are left out.
    let kinds: Vec<_> = page.sections.iter().map(|section| section.kind).collect();
    assert_eq!(
        kinds,
        [
            ArtistSectionKind::Albums,
            ArtistSectionKind::LatestRelease,
            ArtistSectionKind::Singles,
            ArtistSectionKind::EssentialAlbums,
            ArtistSectionKind::AppearsOn
        ]
    );
    // The names are Apple's own titles of the views, and each section says what it opens.
    let titles: Vec<_> = page.sections.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Albums",
            "Latest Release",
            "Singles & EPs",
            "Essential Albums",
            "Appears On"
        ]
    );
    let opens: Vec<_> = page.sections.iter().map(|section| section.opens).collect();
    assert_eq!(
        opens,
        [
            ArtistSectionOpens::Albums,
            ArtistSectionOpens::Album,
            ArtistSectionOpens::Songs,
            ArtistSectionOpens::Albums,
            ArtistSectionOpens::Albums
        ]
    );
    // Only the latest release names an album: "Glow - EP", numbered as a catalog album is.
    let glow = apple.binding.intern(AppleIdKind::Album, "catalog:2011")?;
    let albums: Vec<_> = page
        .sections
        .iter()
        .map(|section| {
            section
                .album
                .as_ref()
                .map(|album| (album.id, album.name.as_str()))
        })
        .collect();
    assert_eq!(albums, [None, Some((glow, "Glow - EP")), None, None, None]);
    // The count is what the first page held; the singles view's three releases, EPs among
    // them, are the one Singles section, and no section is the EPs. The latest release is one.
    let counts: Vec<_> = page.sections.iter().map(|section| section.count).collect();
    assert_eq!(counts, [2, 1, 3, 2, 1]);
    let queries: Vec<_> = page
        .sections
        .iter()
        .map(|section| {
            section
                .queries
                .iter()
                .map(|query| (query.operator.as_str(), query.parameter.clone()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        queries,
        [
            vec![("lib::artist_section", format!("{artist}:albums"))],
            vec![("lib::artist_section", format!("{artist}:latest_release"))],
            vec![("lib::artist_section", format!("{artist}:singles"))],
            vec![("lib::artist_section", format!("{artist}:essential_albums"))],
            vec![("lib::artist_section", format!("{artist}:appears_on"))],
        ]
    );
    assert_eq!(page.tracks.len(), 1);
    assert_eq!(page.tracks[0].operator, "lib::artist_tracks");
    assert_eq!(page.tracks[0].parameter, artist.to_string());
    // One request read every list, and nothing was read of the releases' songs.
    assert_eq!(apple.fake.count("/v1/catalog/it/artists/310?views="), 1);
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/artists/310?views=latest-release"),
        1
    );
    assert_eq!(apple.fake.count("/v1/catalog/it/artists/310/view"), 0);
    assert_eq!(apple.fake.count("/v1/catalog/it/albums"), 0);
    // The tiles' covers come in as they download, and the page says so by listing them.
    wait_until("the covers of the sections", || async {
        let page = apple.run(&artist_page_request(artist)).await?;
        Ok(page
            .sections
            .iter()
            .all(|section| !section.covers.is_empty())
            .then_some(page))
    })
    .await?;
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_catalog_page_is_titled_by_apple_in_a_language_the_storefront_has_and_untitled_otherwise()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let artist = viewed_artist(&apple)?;

    // The storefront supports "en-GB", which an English client gets.
    let english = apple.run(&artist_page_in(artist, "en")).await?;
    assert_eq!(
        english
            .sections
            .iter()
            .map(|s| s.title.as_str())
            .collect::<Vec<_>>(),
        [
            "Albums",
            "Latest Release",
            "Singles & EPs",
            "Essential Albums",
            "Appears On"
        ]
    );
    assert_eq!(apple.fake.count_containing("/artists/310?views="), 1);
    assert_eq!(apple.fake.count_containing("&l=en-GB"), 1);
    assert_eq!(apple.fake.count("/v1/storefronts/it"), 1);

    // A Chinese client gets no titles, and the page is read as before, with no language.
    // The reads are kept per artist, so this one asks nothing more.
    let chinese = apple.run(&artist_page_in(artist, "zh-Hans")).await?;
    assert_eq!(chinese.sections.len(), 5);
    assert!(
        chinese
            .sections
            .iter()
            .all(|section| section.title.is_empty())
    );
    assert_eq!(apple.fake.count_containing("/artists/310?views="), 1);
    assert_eq!(apple.fake.count("/v1/storefronts/it"), 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unsupported_language_reads_a_catalog_page_without_asking_for_one() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let artist = viewed_artist(&apple)?;

    let page = apple.run(&artist_page_in(artist, "zh-Hans")).await?;

    assert_eq!(page.sections.len(), 5);
    assert!(page.sections.iter().all(|section| section.title.is_empty()));
    assert_eq!(apple.fake.count_containing("/artists/310?views="), 1);
    assert_eq!(apple.fake.count_containing("&l="), 0);
    // A client that later speaks English gets Apple's English titles for the same reads,
    // from one more request.
    let english = apple.run(&artist_page_in(artist, "en")).await?;
    assert_eq!(english.sections[0].title, "Albums");
    assert_eq!(apple.fake.count_containing("&l=en-GB"), 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_read_of_the_titles_leaves_the_catalog_page_untitled() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let artist = viewed_artist(&apple)?;
    // The page is read first with no language, so the titles are read for the next client.
    apple.run(&artist_page_in(artist, "zh-Hans")).await?;
    *apple.fake.fail_once.lock().unwrap() = Some("/v1/catalog/it/artists/310".to_owned());
    let page = apple.run(&artist_page_in(artist, "en")).await?;
    assert_eq!(page.sections.len(), 5);
    assert!(page.sections.iter().all(|section| section.title.is_empty()));
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_section_lists_its_releases_songs_and_reads_later_pages_only_when_paged_to() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let artist = viewed_artist(&apple)?;
    apple.run(&artist_page_request(artist)).await?;
    let albums = format!("{artist}:albums");

    // The first page reaches only the first release.
    assert_eq!(
        page_keys(&apple, "lib::artist_section", &albums, 0, 2).await?,
        ["catalog:2101", "catalog:2102"]
    );
    assert_eq!(apple.fake.count("/v1/catalog/it/artists/310/view"), 0);
    assert_eq!(apple.fake.count("/v1/catalog/it/albums/2001/tracks"), 1);
    assert_eq!(apple.fake.count("/v1/catalog/it/albums/2002"), 0);

    // The next reaches the second release, and then the view's second page for the third.
    assert_eq!(
        page_keys(&apple, "lib::artist_section", &albums, 2, 1).await?,
        ["catalog:2103"]
    );
    assert_eq!(apple.fake.count("/v1/catalog/it/artists/310/view"), 0);
    assert_eq!(
        page_keys(&apple, "lib::artist_section", &albums, 3, 5).await?,
        ["catalog:2104"]
    );
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/artists/310/view/full-albums?offset=2"),
        1
    );
    // Reading the pages again asks Apple nothing more, from any cursor.
    let total = apple.fake.total();
    assert_eq!(
        page_keys(&apple, "lib::artist_section", &albums, 0, 10).await?,
        [
            "catalog:2101",
            "catalog:2102",
            "catalog:2103",
            "catalog:2104"
        ]
    );
    assert_eq!(
        page_keys(&apple, "lib::artist_section", &albums, 3, 10).await?,
        ["catalog:2104"]
    );
    assert_eq!(apple.fake.total(), total);

    // The singles view is one section, EPs and singles in Apple's order, release after
    // release; the EPs section is empty.
    let singles = format!("{artist}:singles");
    assert_eq!(
        page_keys(&apple, "lib::artist_section", &singles, 0, 10).await?,
        [
            "catalog:2105",
            "catalog:2106",
            "catalog:2107",
            "catalog:2109"
        ]
    );
    assert_eq!(
        page_keys(&apple, "lib::artist_section", &singles, 1, 2).await?,
        ["catalog:2106", "catalog:2107"]
    );
    assert!(
        page_keys(
            &apple,
            "lib::artist_section",
            &format!("{artist}:eps"),
            0,
            10
        )
        .await?
        .is_empty()
    );
    assert_eq!(
        page_keys(
            &apple,
            "lib::artist_section",
            &format!("{artist}:appears_on"),
            0,
            10
        )
        .await?,
        ["catalog:2108"]
    );
    // A section the artist has none of lists nothing, and a section is refused when it is
    // combined with another item, which the artist page never does.
    assert!(
        page_keys(
            &apple,
            "lib::artist_section",
            &format!("{artist}:live"),
            0,
            10
        )
        .await?
        .is_empty()
    );
    let mut combined = mix("lib::artist_section", format!("{artist}:singles"));
    combined.queries.push(MixQuery {
        operator: "lib::album".into(),
        parameter: "1".into(),
    });
    assert!(unsupported(
        &apple
            .run(&combined)
            .await
            .err()
            .context("a combined query")?
    ));
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_long_page_of_a_section_reads_its_releases_in_one_request() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let artist = viewed_artist(&apple)?;
    apple.run(&artist_page_request(artist)).await?;
    let albums = format!("{artist}:albums");

    // The three releases of the albums view, the third on the view's second page, come with
    // their songs from one request, not one each.
    assert_eq!(
        page_keys(&apple, "lib::artist_section", &albums, 0, 4096).await?,
        [
            "catalog:2101",
            "catalog:2102",
            "catalog:2103",
            "catalog:2104"
        ]
    );
    assert_eq!(apple.fake.count("/v1/catalog/it/albums?"), 1);
    assert_eq!(apple.fake.count("/v1/catalog/it/albums/"), 0);
    // What was read is kept for the pages after it.
    let total = apple.fake.total();
    assert_eq!(
        page_keys(&apple, "lib::artist_section", &albums, 2, 4096).await?,
        ["catalog:2103", "catalog:2104"]
    );
    assert_eq!(apple.fake.total(), total);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_tracks_of_an_artist_page_come_through_apples_next_only_as_far_as_they_are_asked()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let artist = viewed_artist(&apple)?;
    let id = artist.to_string();

    // The first page came with the artist, and holds the library's song as its own copy.
    assert_eq!(
        page_keys(&apple, "lib::artist_tracks", &id, 0, 2).await?,
        ["i.s1", "catalog:3001"]
    );
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/artists/310/view/top-songs"),
        0
    );

    assert_eq!(
        page_keys(&apple, "lib::artist_tracks", &id, 2, 1).await?,
        ["catalog:3002"]
    );
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/artists/310/view/top-songs?offset=2"),
        1
    );
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/artists/310/view/top-songs?offset=4"),
        0
    );
    // Apple's last page has no next, and so the list ends; the total is logged then, once.
    assert_eq!(
        page_keys(&apple, "lib::artist_tracks", &id, 3, 10).await?,
        ["catalog:3003", "catalog:3004"]
    );
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/artists/310/view/top-songs"),
        2
    );
    assert!(!apple.binding.first_top_songs_total("310"));
    assert!(
        page_keys(&apple, "lib::artist_tracks", &id, 5, 10)
            .await?
            .is_empty()
    );
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/artists/310/view/top-songs"),
        2
    );
    // Playing the tracks resolves them as the page lists them.
    let resolved = apple
        .run(&ResolveMixQueryRequest {
            queries: vec![MixQuery {
                operator: "lib::artist_tracks".into(),
                parameter: id,
            }],
            hint_position: -1,
            initial_playback_item: None,
            instantly_play: false,
            fallback_playing_items: Vec::new(),
            excluded: ExcludedItems::default(),
            call_id: None,
        })
        .await?;
    assert_eq!(
        resolved.playing_items,
        [
            song_ref(&apple, "i.s1"),
            song_ref(&apple, "catalog:3001"),
            song_ref(&apple, "catalog:3002"),
            song_ref(&apple, "catalog:3003"),
            song_ref(&apple, "catalog:3004"),
        ]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_page_of_a_view_or_the_top_songs_leaves_the_list_to_be_read_again() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            fail_once: Some("/v1/catalog/it/artists/310/view/full-albums"),
            ..AppleOptions::default()
        },
    )
    .await?;
    let artist = viewed_artist(&apple)?;
    apple.run(&artist_page_request(artist)).await?;
    let albums = format!("{artist}:albums");

    // The third release is on the view's second page, which Apple fails once.
    assert!(
        page_keys(&apple, "lib::artist_section", &albums, 3, 5)
            .await
            .is_err()
    );
    assert_eq!(
        page_keys(&apple, "lib::artist_section", &albums, 3, 5).await?,
        ["catalog:2104"]
    );
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/artists/310/view/full-albums?offset=2"),
        2
    );

    // The same for the second page of the top songs.
    *apple.fake.fail_once.lock().unwrap() =
        Some("/v1/catalog/it/artists/310/view/top-songs".to_owned());
    let tracks = artist.to_string();
    assert!(
        page_keys(&apple, "lib::artist_tracks", &tracks, 2, 1)
            .await
            .is_err()
    );
    assert_eq!(
        page_keys(&apple, "lib::artist_tracks", &tracks, 2, 1).await?,
        ["catalog:3002"]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_artist_page_needs_the_sign_in_like_any_other_read_and_no_mix_keeps_its_lists()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let artist = viewed_artist(&apple)?;

    apple.fake.reject_everything.store(true, Ordering::SeqCst);
    let error = apple
        .run(&artist_page_request(artist))
        .await
        .err()
        .context("a rejected sign-in")?;
    assert_eq!(
        request_failure_cause(&error),
        Some(RequestFailureCause::ServiceSignInRequired {
            record_id: apple.record.0.to_string(),
        })
    );
    apple.fake.reject_everything.store(false, Ordering::SeqCst);
    assert_eq!(
        apple
            .run(&artist_page_request(artist))
            .await?
            .sections
            .len(),
        5
    );

    // A mix holds no list of an artist's page, as a local library's holds none.
    let refused = apple
        .run(&crate::messages::CreateMixRequest {
            call_id: None,
            name: "Singles".into(),
            group: "Favorite".into(),
            mode: None,
            queries: vec![MixQuery {
                operator: "lib::artist_section".into(),
                parameter: format!("{artist}:singles"),
            }],
        })
        .await
        .err()
        .context("a mix of a section")?;
    assert!(unsupported(&refused));
    assert!(rune_mixes(&apple).await?.is_empty());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_search_finds_a_catalog_artist_the_list_lacks_and_opens_its_catalog_page() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let found = searched_artists(&apple).await?;
    // "Artist One" is on the library's artist list and is answered as that entry; "Dawn Singer"
    // is not, and is answered as the catalog artist.
    assert_eq!(
        found
            .iter()
            .map(|id| apple.binding.resolve(AppleIdKind::Artist, *id))
            .collect::<Result<Vec<_>>>()?,
        ["artist:300", "catalog:301"]
    );

    let page = apple.run(&artist_page_request(found[1])).await?;

    // The catalog's views were read for it.
    assert_eq!(apple.fake.count("/v1/catalog/it/artists/301?views="), 1);
    assert_eq!(page.artist_id, found[1]);
    // A mix of it plays the songs of its albums, read from the catalog.
    assert_eq!(
        page_keys(&apple, "lib::artist", &found[1].to_string(), 0, 10).await?,
        ["catalog:1601", "catalog:1602"]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_search_lists_the_catalog_artists_it_finds_when_the_artist_list_fails() -> Result<()> {
    const ARTISTS: &str = "/v1/me/library/artists";
    let dir = tempfile::tempdir()?;
    // Apple fails every read of the library's artists, so there is no artist list.
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            respond: Some((ARTISTS, 500, r#"{"errors":[{"status":"500"}]}"#)),
            import_unfinished: true,
            ..AppleOptions::default()
        },
    )
    .await?;
    let found = searched_artists(&apple).await?;
    assert!(apple.fake.count(ARTISTS) > 0);
    // Both catalog artists are answered as themselves, and open the catalog's page.
    assert_eq!(
        found
            .iter()
            .map(|id| apple.binding.resolve(AppleIdKind::Artist, *id))
            .collect::<Result<Vec<_>>>()?,
        ["catalog:300", "catalog:301"]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_catalog_artist_that_is_not_on_the_list_still_opens_its_catalog_page() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // The list has no "310", though a song's catalog artist is numbered as a list entry.
    let artist = apple.binding.intern(AppleIdKind::Artist, "artist:310")?;

    let page = apple.run(&artist_page_request(artist)).await?;

    assert_eq!(page.sections.len(), 5);
    assert_eq!(apple.fake.count("/v1/catalog/it/artists/310?views="), 1);
    apple.finish().await
}

// The library's page -------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_artist_without_a_catalog_identity_is_answered_from_the_library() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // The library's "Artist Two" has no catalog artist, and holds one album with one song.
    let listed = all_artists(&apple).await?;
    let artist = artist_named(&listed, "Artist Two")?;

    let page = apple.run(&artist_page_request(artist)).await?;

    assert_eq!(
        page.sections
            .iter()
            .map(|section| section.kind)
            .collect::<Vec<_>>(),
        [ArtistSectionKind::Albums]
    );
    assert_eq!(page.sections[0].count, 1);
    assert_eq!(
        page.sections[0].queries[0].parameter,
        format!("{artist}:albums")
    );
    assert_eq!(page.tracks.len(), 1);
    assert_eq!(
        page_keys(
            &apple,
            "lib::artist_section",
            &format!("{artist}:albums"),
            0,
            10
        )
        .await?,
        ["i.s3"]
    );
    assert_eq!(
        page_keys(&apple, "lib::artist_tracks", &artist.to_string(), 0, 10).await?,
        ["i.s3"]
    );
    // No section but the albums exists for it.
    assert!(
        page_keys(
            &apple,
            "lib::artist_section",
            &format!("{artist}:singles"),
            0,
            10
        )
        .await?
        .is_empty()
    );
    // Nothing of its page asked the catalog.
    assert_eq!(apple.fake.count("/v1/catalog/it/artists"), 0);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_library_entry_with_a_catalog_artist_gets_the_catalog_pages_sections_of_what_it_holds()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), kinds()).await?;
    let listed = all_artists(&apple).await?;
    let artist = artist_named(&listed, "Artist One")?;
    // The entry is numbered by its catalog artist, and still opens the library's page.
    assert_eq!(
        apple.binding.resolve(AppleIdKind::Artist, artist)?,
        "artist:300"
    );
    wait_synced(&apple, &KINDS_ALBUMS).await?;

    let page = wait_titled(&apple, artist, "en").await?;

    // The catalog has two albums, an EP, a single, a compilation and an appears-on album; the
    // library holds the second album, the single, the EP and the compilation. The single and
    // the EP are one Singles section, in the order of the catalog's singles view.
    assert_eq!(
        section_counts(&page),
        [
            (ArtistSectionKind::Albums, 1),
            (ArtistSectionKind::Singles, 2),
            (ArtistSectionKind::Compilations, 1),
        ]
    );
    let songs = |section: &'static str| {
        let apple = &apple;
        async move {
            page_keys(
                apple,
                "lib::artist_section",
                &format!("{artist}:{section}"),
                0,
                20,
            )
            .await
        }
    };
    // The titles are the catalog page's, and the sections open as it does, but the latest
    // release and the essential albums, which the view holds too, are left to the full page.
    assert_eq!(
        page.sections
            .iter()
            .map(|s| s.title.as_str())
            .collect::<Vec<_>>(),
        ["Albums", "Singles & EPs", "Compilations"]
    );
    assert_eq!(
        page.sections.iter().map(|s| s.opens).collect::<Vec<_>>(),
        [
            ArtistSectionOpens::Albums,
            ArtistSectionOpens::Songs,
            ArtistSectionOpens::Albums
        ]
    );
    assert!(page.sections.iter().all(|section| section.album.is_none()));
    // The albums of a section are the library's own, as its albums page numbers them.
    let held = |id: &str| apple.binding.intern(AppleIdKind::Album, id);
    let albums = section_albums(&apple, artist, "albums", 0, 10).await?;
    assert_eq!(albums, [held("l.big")?]);
    let singles = section_albums(&apple, artist, "singles", 0, 10).await?;
    assert_eq!(singles, [held("l.sgl")?, held("l.ep")?]);
    assert_eq!(
        section_albums(&apple, artist, "singles", 1, 10).await?,
        [held("l.ep")?]
    );
    assert!(
        section_albums(&apple, artist, "singles", 2, 10)
            .await?
            .is_empty()
    );
    let compilations = section_albums(&apple, artist, "compilations", 0, 10).await?;
    assert_eq!(compilations, [held("l.va")?]);
    assert_eq!(
        album_names(&apple, [albums, singles, compilations].concat()).await?,
        ["Big Album", "Lone Single", "Short EP", "Hits"]
    );
    // Neither the latest release nor the essential albums are on a library page.
    assert!(
        section_albums(&apple, artist, "latest_release", 0, 10)
            .await?
            .is_empty()
    );
    assert!(
        section_albums(&apple, artist, "essential_albums", 0, 10)
            .await?
            .is_empty()
    );
    // The songs are the library's copies, release after release by number.
    assert_eq!(
        songs("albums").await?,
        ["i.b1", "i.b2", "i.b3", "i.b4", "i.b5", "i.b6", "i.b7"]
    );
    assert_eq!(
        songs("singles").await?,
        ["i.g1x", "i.e1", "i.e2", "i.e3", "i.e4"]
    );
    assert_eq!(
        page_keys(
            &apple,
            "lib::artist_section",
            &format!("{artist}:singles"),
            1,
            2
        )
        .await?,
        ["i.e1", "i.e2"]
    );
    assert!(songs("eps").await?.is_empty());
    assert_eq!(songs("compilations").await?, ["i.v1"]);
    assert!(songs("appears_on").await?.is_empty());
    assert!(songs("live").await?.is_empty());
    // A section pages song after song.
    assert_eq!(
        page_keys(
            &apple,
            "lib::artist_section",
            &format!("{artist}:albums"),
            2,
            2
        )
        .await?,
        ["i.b3", "i.b4"]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_library_page_is_titled_in_the_language_or_not_at_all_and_never_mixed() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let heard = Arc::new(Heard::default());
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            heard: Some(heard.clone()),
            ..kinds()
        },
    )
    .await?;
    let listed = all_artists(&apple).await?;
    let artist = artist_named(&listed, "Artist One")?;
    wait_synced(&apple, &KINDS_ALBUMS).await?;
    // The views were read just now, so the pages read only titles.
    keep_views(&apple, &ARTIST_ONE_VIEWS, now(&apple)).await;
    let titles = |page: &FetchArtistPageResponse| -> Vec<String> {
        page.sections.iter().map(|s| s.title.clone()).collect()
    };
    let untitled = || vec![String::new(); 3];
    // The page's covers download first: they are announced on their own once they are in.
    apple.run(&artist_page_request(artist)).await?;
    let announced = told_settled(&heard).await;

    // Nothing is kept in any language: every title is empty, nothing is asked on the way, and
    // the titles are read in the background, from one artist page, with one announcement.
    let before = apple.fake.count("/v1/catalog/it/artists/300?views=");
    let first = apple.run(&artist_page_in(artist, "en")).await?;
    assert_eq!(titles(&first), untitled());
    wait_until("the titles to be kept", || async {
        Ok(classes(&apple).await.titles_for("en").map(|_| ()))
    })
    .await?;
    told_after(&heard, announced).await?;
    assert_eq!(told_settled(&heard).await, announced + 1);
    assert_eq!(
        apple.fake.count("/v1/catalog/it/artists/300?views="),
        before + 1
    );
    assert!(apple.fake.count_containing("&l=en-GB") >= 1);

    // Fetched again, as it is told, the page has the English titles, and asks nothing.
    let page = apple.run(&artist_page_in(artist, "en")).await?;
    assert_eq!(titles(&page), ["Albums", "Singles & EPs", "Compilations"]);
    // The kept titles serve another English tag of the client, `en-GB`, as well.
    let british = apple.run(&artist_page_in(artist, "en-GB")).await?;
    assert_eq!(
        titles(&british),
        ["Albums", "Singles & EPs", "Compilations"]
    );
    assert_eq!(
        apple.fake.count("/v1/catalog/it/artists/300?views="),
        before + 1
    );

    // A language the storefront lacks gets no titles, now and after its languages are known,
    // and no title is read for it.
    let chinese = apple.run(&artist_page_in(artist, "zh-Hans")).await?;
    assert_eq!(titles(&chinese), untitled());
    tokio::time::sleep(Duration::from_millis(300)).await;
    let chinese = apple.run(&artist_page_in(artist, "zh-Hans")).await?;
    assert_eq!(titles(&chinese), untitled());
    assert_eq!(
        apple.fake.count("/v1/catalog/it/artists/300?views="),
        before + 1
    );
    // An unknown client language is the same.
    let unknown = apple.run(&artist_page_request(artist)).await?;
    assert_eq!(titles(&unknown), untitled());

    // A title missing for one section of the page leaves the whole page untitled.
    change_classes(&apple, |classes| {
        classes
            .section_titles
            .get_mut("it")
            .and_then(|tags| tags.get_mut("en-GB"))
            .expect("the English titles")
            .remove("compilations");
    })
    .await;
    let mixed = apple.run(&artist_page_in(artist, "en")).await?;
    assert_eq!(titles(&mixed), untitled());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_held_album_the_catalog_does_not_know_goes_into_the_albums() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), lost_tape()).await?;
    let listed = all_artists(&apple).await?;
    let artist = artist_named(&listed, "Artist One")?;
    wait_synced(&apple, &["4002", "4999"]).await?;

    let page = apple.run(&artist_page_request(artist)).await?;

    // "Lost Tape" has no facts, as the catalog does not know it, and is not asked for again.
    assert_eq!(
        section_counts(&page),
        [
            (ArtistSectionKind::Albums, 2),
            (ArtistSectionKind::Singles, 2),
            (ArtistSectionKind::Compilations, 1),
        ]
    );
    // The newest album first, then the ones with no facts.
    assert_eq!(
        page_keys(
            &apple,
            "lib::artist_section",
            &format!("{artist}:albums"),
            0,
            20
        )
        .await?,
        [
            "i.b1", "i.b2", "i.b3", "i.b4", "i.b5", "i.b6", "i.b7", "i.l1"
        ]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_albums_of_a_library_section_come_by_year_and_those_without_one_last() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            many_albums: 2,
            ..lost_tape()
        },
    )
    .await?;
    let listed = all_artists(&apple).await?;
    let artist = artist_named(&listed, "Artist One")?;
    wait_synced(&apple, &["4002", "4999", "7000", "7001"]).await?;
    let held = |id: &str| apple.binding.intern(AppleIdKind::Album, id);
    let group = |year: &str, ids: &[&str]| -> Result<(String, Vec<i32>)> {
        let ids = ids.iter().map(|id| held(id)).collect::<Result<_>>()?;
        Ok((year.to_owned(), ids))
    };

    // "Big Album" came out in 2022 and "Lost Tape" in 2019; the two albums Apple does not know
    // have no year, so they come last under "#", in the section's order.
    assert_eq!(
        section_album_groups(&apple, artist, "albums", 0, 10).await?,
        [
            group("2022", &["l.big"])?,
            group("2019", &["l.lost"])?,
            group("#", &["l.many0", "l.many1"])?,
        ]
    );
    // A page that cuts "#" ends with it, and the next goes on with it under the same title.
    assert_eq!(
        section_album_groups(&apple, artist, "albums", 0, 3).await?,
        [
            group("2022", &["l.big"])?,
            group("2019", &["l.lost"])?,
            group("#", &["l.many0"])?,
        ]
    );
    assert_eq!(
        section_album_groups(&apple, artist, "albums", 3, 3).await?,
        [group("#", &["l.many1"])?]
    );
    // Each section is grouped the same.
    assert_eq!(
        section_album_groups(&apple, artist, "compilations", 0, 10).await?,
        [group("2018", &["l.va"])?]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_tracks_of_a_library_page_are_the_liked_songs_then_the_newest_album_without_asking()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), kinds()).await?;
    let listed = all_artists(&apple).await?;
    let artist = artist_named(&listed, "Artist One")?;
    wait_synced(&apple, &KINDS_ALBUMS).await?;
    keep_views(&apple, &ARTIST_ONE_VIEWS, now(&apple)).await;
    apple
        .binding
        .rune_likes
        .set_liked(&apple.binding.like_keys("i.v1"), true)
        .await?;
    let asked = apple.fake.count("/v1/catalog");

    let page = apple.run(&artist_page_request(artist)).await?;

    assert_eq!(page.tracks[0].operator, "lib::artist_tracks");
    // The liked song first, then the others, the newest album first, then disc and number.
    assert_eq!(
        page_keys(&apple, "lib::artist_tracks", &artist.to_string(), 0, 30).await?,
        [
            "i.v1", "i.g1x", "i.b1", "i.b2", "i.b3", "i.b4", "i.b5", "i.b6", "i.b7", "i.e1",
            "i.e2", "i.e3", "i.e4", "i.s1", "i.s2"
        ]
    );
    // Neither the page nor its tracks asked the catalog, and no top songs were read.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(apple.fake.count("/v1/catalog"), asked);
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/artists/300/view/top-songs"),
        0
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_library_page_lists_its_albums_as_albums_while_the_catalog_gives_no_facts() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), kinds()).await?;
    *apple.fake.respond_path.lock().unwrap() =
        Some(("/v1/catalog/it/albums".into(), 500, "{}".into()));
    let listed = all_artists(&apple).await?;
    let artist = artist_named(&listed, "Artist One")?;
    apple.binding.albums().await?;
    fake_asked(&apple, "/v1/catalog/it/albums", 1).await?;
    // Nor do the views list anything.
    keep_views(&apple, &[], now(&apple)).await;

    let page = apple.run(&artist_page_request(artist)).await?;

    // One Albums section of the albums the library holds by the artist or credits it with, the
    // newest year first, and no error page. It has no title of Apple's.
    assert_eq!(section_counts(&page), [(ArtistSectionKind::Albums, 4)]);
    assert!(page.sections[0].title.is_empty());
    assert_eq!(page.sections[0].opens, ArtistSectionOpens::Albums);
    assert_eq!(
        page_keys(
            &apple,
            "lib::artist_section",
            &format!("{artist}:albums"),
            0,
            20
        )
        .await?,
        [
            "i.g1x", "i.b1", "i.b2", "i.b3", "i.b4", "i.b5", "i.b6", "i.b7", "i.e1", "i.e2",
            "i.e3", "i.e4", "i.v1"
        ]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_sync_keeps_the_facts_of_the_held_albums_in_batches_of_a_hundred_and_skips_fresh_ones()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            many_albums: 250,
            ..kinds()
        },
    )
    .await?;
    all_artists(&apple).await?;
    wait_synced(&apple, &["4002", "7000", "7249"]).await?;

    // 254 albums with catalog ids: three requests of at most a hundred ids, none of which
    // names an id twice.
    let batches = apple.fake.count("/v1/catalog/it/albums?");
    assert_eq!(batches, 3);
    let facts = classes(&apple).await;
    assert!(facts.albums.len() >= 254);
    // The facts are the catalog's: the EP is one by name, the compilation one by flag.
    assert_eq!(facts.albums["4003"].name, "Short EP - EP");
    assert_eq!(facts.albums["4005"].compilation, Some(true));
    assert_eq!(facts.albums["4004"].artists, ["300"]);
    // The sync reads no titles: they depend on the language of the page that asks.
    assert_eq!(apple.fake.count("/v1/catalog/it/artists/300?views="), 0);

    // Fresh facts are not read again, however often the albums are asked for or classified.
    let ids: Vec<String> = facts.albums.keys().cloned().collect();
    apple.binding.albums().await?;
    classify_in_background(&apple.binding, ids);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(apple.fake.count("/v1/catalog/it/albums?"), batches);

    // Old ones are: ten of them make one request.
    change_classes(&apple, |classes| {
        for at in 0..10 {
            if let Some(facts) = classes.albums.get_mut(&format!("{}", 7000 + at)) {
                facts.read = 0;
            }
        }
    })
    .await;
    let ids: Vec<String> = (0..10).map(|at| format!("{}", 7000 + at)).collect();
    classify_in_background(&apple.binding, ids);
    fake_asked(&apple, "/v1/catalog/it/albums?", batches + 1).await?;
    wait_until("the facts to be read again", || async {
        Ok((classes(&apple).await.albums["7000"].read > 0).then_some(()))
    })
    .await?;
    assert_eq!(apple.fake.count("/v1/catalog/it/albums?"), batches + 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_library_page_asks_nothing_and_puts_each_album_where_its_facts_say() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), lost_tape()).await?;
    let listed = all_artists(&apple).await?;
    let artist = artist_named(&listed, "Artist One")?;
    wait_synced(&apple, &["4002", "4999"]).await?;
    // The views were read just now and list nothing, so the facts alone place the albums.
    keep_views(&apple, &[], now(&apple)).await;
    wait_titled(&apple, artist, "en").await?;
    // The facts the page derives its sections from: the big album is on another artist's
    // record (Appears On), "Lone Single" is a single, "Short EP" an EP by its name, "Hits" a
    // compilation, and "Lost Tape" has none.
    let read = now(&apple);
    let facts = |name: &str, single, compilation, artists: &[&str]| AlbumFacts {
        name: name.into(),
        single,
        compilation,
        artists: artists.iter().map(|id| (*id).to_owned()).collect(),
        read,
    };
    change_classes(&apple, |classes| {
        classes
            .albums
            .insert("4002".into(), facts("Big Album", None, None, &["302"]));
        classes.albums.insert(
            "4004".into(),
            facts("Lone Single", Some(true), None, &["300"]),
        );
        classes.albums.insert(
            "4003".into(),
            facts("Short EP - EP", Some(false), None, &["300"]),
        );
        classes
            .albums
            .insert("4005".into(), facts("Hits", None, Some(true), &["999"]));
        classes.albums.remove("4999");
        classes
            .section_titles
            .get_mut("it")
            .and_then(|tags| tags.get_mut("en-GB"))
            .expect("the English titles")
            .remove("appears_on");
    })
    .await;
    let asked = apple.fake.count("/v1/catalog");

    let page = apple.run(&artist_page_in(artist, "en")).await?;

    // "Lost Tape" has no facts yet, so it is Albums; asking for it is the page's background
    // business, and it gives nothing, as the catalog does not know it.
    assert_eq!(
        section_counts(&page),
        [
            (ArtistSectionKind::Albums, 1),
            (ArtistSectionKind::Singles, 2),
            (ArtistSectionKind::Compilations, 1),
            (ArtistSectionKind::AppearsOn, 1),
        ]
    );
    // A kept title is missing for one section of the page, so none is used: the page never
    // names some sections in one language and the rest in another.
    assert_eq!(
        page.sections
            .iter()
            .map(|s| s.title.as_str())
            .collect::<Vec<_>>(),
        ["", "", "", ""]
    );
    assert_eq!(
        page_keys(
            &apple,
            "lib::artist_section",
            &format!("{artist}:albums"),
            0,
            20
        )
        .await?,
        ["i.l1"]
    );
    assert_eq!(
        page_keys(
            &apple,
            "lib::artist_section",
            &format!("{artist}:appears_on"),
            0,
            20
        )
        .await?,
        ["i.b1", "i.b2", "i.b3", "i.b4", "i.b5", "i.b6", "i.b7"]
    );
    // Only the album with no facts was asked about.
    wait_until("the unknown album to be asked about", || async {
        Ok((apple.fake.count("/v1/catalog") > asked).then_some(()))
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Nothing else of the catalog was asked for: the page itself made no request.
    assert_eq!(apple.fake.count("/v1/catalog"), asked + 1);
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/albums?ids=4999&include=artists"),
        1
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_album_added_after_the_sync_is_classified_in_the_background_with_one_announcement()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let heard = Arc::new(Heard::default());
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            heard: Some(heard.clone()),
            ..kinds()
        },
    )
    .await?;
    let listed = all_artists(&apple).await?;
    let artist = artist_named(&listed, "Artist One")?;
    wait_synced(&apple, &KINDS_ALBUMS).await?;
    // The views were read just now and list nothing, so the facts alone place the albums.
    keep_views(&apple, &[], now(&apple)).await;
    // "Lone Single" arrived after the last sync: the page has no facts of it yet.
    change_classes(&apple, |classes| {
        classes.albums.remove("4004");
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let asked = apple.fake.count("/v1/catalog/it/albums?");
    let heard_before = told_settled(&heard).await;

    let opened = apple.run(&artist_page_request(artist)).await?;
    // The page answers with the album in Albums, before anything is read.
    assert_eq!(opened.sections[0].count, 2);
    wait_until("the album to be classified", || async {
        Ok(classes(&apple)
            .await
            .albums
            .contains_key("4004")
            .then_some(()))
    })
    .await?;
    told_after(&heard, heard_before).await?;
    assert_eq!(apple.fake.count("/v1/catalog/it/albums?"), asked + 1);

    // Fetched again, as it is told, the page has the album in Singles, and nothing is read or
    // announced again.
    let announced = told_settled(&heard).await;
    let page = apple.run(&artist_page_request(artist)).await?;
    assert_eq!(
        section_counts(&page),
        [
            (ArtistSectionKind::Albums, 1),
            (ArtistSectionKind::Singles, 2),
            (ArtistSectionKind::Compilations, 1),
        ]
    );
    assert_eq!(told_settled(&heard).await, announced);
    assert_eq!(apple.fake.count("/v1/catalog/it/albums?"), asked + 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_library_page_answers_from_the_facts_then_puts_its_albums_where_apples_views_do()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let heard = Arc::new(Heard::default());
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            heard: Some(heard.clone()),
            ..kinds()
        },
    )
    .await?;
    let listed = all_artists(&apple).await?;
    let artist = artist_named(&listed, "Artist One")?;
    wait_synced(&apple, &KINDS_ALBUMS).await?;
    short_ep_looks_like_an_album(&apple).await;
    let release = Arc::new(tokio::sync::Notify::new());
    *apple.fake.hold_once.lock().unwrap() =
        Some(("/v1/catalog/it/artists/300".into(), release.clone()));
    let announced = told_settled(&heard).await;

    // The page answers from the facts, which put "Short EP" with the albums, while the views
    // are still being read behind it.
    let first = apple.run(&artist_page_request(artist)).await?;
    assert_eq!(
        section_counts(&first),
        [
            (ArtistSectionKind::Albums, 2),
            (ArtistSectionKind::Singles, 1),
            (ArtistSectionKind::Compilations, 1),
        ]
    );
    fake_asked(&apple, "/v1/catalog/it/artists/300?views=", 1).await?;
    assert_eq!(views_read(&apple).await, None);

    release.notify_one();
    told_after(&heard, announced).await?;
    // Fetched again, as it is told, the page has "Short EP" among the singles, where Apple's
    // views list it.
    let page = wait_until("the views to place the albums", || async {
        let page = apple.run(&artist_page_request(artist)).await?;
        Ok((section_counts(&page)
            == [
                (ArtistSectionKind::Albums, 1),
                (ArtistSectionKind::Singles, 2),
                (ArtistSectionKind::Compilations, 1),
            ])
        .then_some(page))
    })
    .await?;
    let held = |id: &str| apple.binding.intern(AppleIdKind::Album, id);
    assert_eq!(
        section_albums(&apple, artist, "albums", 0, 10).await?,
        [held("l.big")?]
    );
    assert_eq!(
        section_albums(&apple, artist, "singles", 0, 10).await?,
        [held("l.sgl")?, held("l.ep")?]
    );
    assert_eq!(
        page_keys(
            &apple,
            "lib::artist_section",
            &format!("{artist}:singles"),
            0,
            20
        )
        .await?,
        ["i.g1x", "i.e1", "i.e2", "i.e3", "i.e4"]
    );
    // Each view was read to its end, once.
    assert_eq!(apple.fake.count("/v1/catalog/it/artists/300?views="), 1);
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/artists/300/view/full-albums"),
        1
    );
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/artists/300/view/appears-on-albums"),
        1
    );

    // A later open answers from what was kept, and asks nothing.
    let asked = apple.fake.count("/v1/catalog");
    let told = told_settled(&heard).await;
    let again = apple.run(&artist_page_request(artist)).await?;
    assert_eq!(section_counts(&again), section_counts(&page));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(apple.fake.count("/v1/catalog"), asked);
    assert_eq!(told_settled(&heard).await, told);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_album_no_view_lists_stays_where_its_facts_put_it() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), lost_tape()).await?;
    let listed = all_artists(&apple).await?;
    let artist = artist_named(&listed, "Artist One")?;
    wait_synced(&apple, &["4002", "4999"]).await?;
    short_ep_looks_like_an_album(&apple).await;
    // No view lists "Lost Tape", and its facts make it a single.
    let read = now(&apple);
    change_classes(&apple, |classes| {
        classes.albums.insert(
            "4999".into(),
            AlbumFacts {
                name: "Lost Tape - Single".into(),
                single: Some(true),
                compilation: None,
                artists: vec!["300".into()],
                read,
            },
        );
    })
    .await;

    apple.run(&artist_page_request(artist)).await?;
    wait_until("the views to be kept", || async {
        Ok(views_read(&apple).await.map(|_| ()))
    })
    .await?;
    let page = apple.run(&artist_page_request(artist)).await?;

    // "Short EP" is where the views list it, and "Lost Tape" where its facts put it.
    assert_eq!(
        section_counts(&page),
        [
            (ArtistSectionKind::Albums, 1),
            (ArtistSectionKind::Singles, 3),
            (ArtistSectionKind::Compilations, 1),
        ]
    );
    let held = |id: &str| apple.binding.intern(AppleIdKind::Album, id);
    assert_eq!(
        section_albums(&apple, artist, "singles", 0, 10).await?,
        [held("l.sgl")?, held("l.ep")?, held("l.lost")?]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_a_library_page_reads_no_views_and_uses_those_kept_however_old() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), kinds()).await?;
    let listed = all_artists(&apple).await?;
    let artist = artist_named(&listed, "Artist One")?;
    wait_synced(&apple, &KINDS_ALBUMS).await?;
    short_ep_looks_like_an_album(&apple).await;
    apple.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        Reach::offline(HashSet::new()),
    );
    let asked = apple.fake.count("/v1/catalog");
    let by_facts = [
        (ArtistSectionKind::Albums, 2),
        (ArtistSectionKind::Singles, 1),
        (ArtistSectionKind::Compilations, 1),
    ];
    let by_views = [
        (ArtistSectionKind::Albums, 1),
        (ArtistSectionKind::Singles, 2),
        (ArtistSectionKind::Compilations, 1),
    ];

    // With no views kept, the facts place the albums, and nothing is read.
    let page = apple.run(&artist_page_request(artist)).await?;
    assert_eq!(section_counts(&page), by_facts);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(apple.fake.count("/v1/catalog"), asked);
    assert_eq!(views_read(&apple).await, None);

    // Views kept long ago place them, and are not read again.
    keep_views(&apple, &ARTIST_ONE_VIEWS, 0).await;
    let page = apple.run(&artist_page_request(artist)).await?;
    assert_eq!(section_counts(&page), by_views);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(apple.fake.count("/v1/catalog"), asked);
    assert_eq!(views_read(&apple).await, Some(0));

    // Online again, the page still answers from them, and reads them again behind it.
    apple.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        Reach::default(),
    );
    let page = apple.run(&artist_page_request(artist)).await?;
    assert_eq!(section_counts(&page), by_views);
    wait_until("the views to be read again", || async {
        Ok(views_read(&apple)
            .await
            .filter(|read| *read > 0)
            .map(|_| ()))
    })
    .await?;
    assert_eq!(apple.fake.count("/v1/catalog/it/artists/300?views="), 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn views_older_than_their_time_are_read_again_once() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), kinds()).await?;
    let listed = all_artists(&apple).await?;
    let artist = artist_named(&listed, "Artist One")?;
    wait_synced(&apple, &KINDS_ALBUMS).await?;
    apple.run(&artist_page_request(artist)).await?;
    wait_until("the views to be kept", || async {
        Ok(views_read(&apple).await.map(|_| ()))
    })
    .await?;
    let pages = apple
        .fake
        .count("/v1/catalog/it/artists/300/view/full-albums");
    assert_eq!(pages, 1);

    // Young views are not read again, however often the page opens.
    for _ in 0..3 {
        apple.run(&artist_page_request(artist)).await?;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/artists/300/view/full-albums"),
        pages
    );

    // A day later the views are old, as is what the binding read of the artist, and the pages
    // opened read them again once between them.
    let read = views_read(&apple).await.context("the views read")?;
    apple.clock.advance(VIEWS_TTL);
    for _ in 0..3 {
        apple.run(&artist_page_request(artist)).await?;
    }
    wait_until("the views to be read again", || async {
        Ok(views_read(&apple)
            .await
            .filter(|again| *again > read)
            .map(|_| ()))
    })
    .await?;
    for _ in 0..3 {
        apple.run(&artist_page_request(artist)).await?;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        apple
            .fake
            .count("/v1/catalog/it/artists/300/view/full-albums"),
        pages + 1
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_song_a_rune_playlist_holds_is_on_its_artists_library_page_where_its_album_goes()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), kinds()).await?;
    let artist = artist_named(&all_artists(&apple).await?, "Artist One")?;
    assert_eq!(
        apple.binding.resolve(AppleIdKind::Artist, artist)?,
        "artist:300"
    );
    // "Unheld Hit" of "Artist One" is on "Elsewhere", an album of "Guest".
    let mine = make_rune_playlist(&apple, "Mine", "Favorite").await?;
    add_song(&apple, mine.id, "catalog:8001", None).await?;
    let album = album_listed(&apple, "Elsewhere").await?;
    let mut classified = KINDS_ALBUMS.to_vec();
    classified.push("4007");
    wait_synced(&apple, &classified).await?;

    // The artist keeps the library's page, with the album in the section its catalog facts put
    // it in, as the library's own albums are.
    let page = wait_until("the album to be on the page", || async {
        let page = apple.run(&artist_page_request(artist)).await?;
        Ok(section_counts(&page)
            .contains(&(ArtistSectionKind::AppearsOn, 1))
            .then_some(page))
    })
    .await?;
    assert_eq!(
        section_counts(&page),
        [
            (ArtistSectionKind::Albums, 1),
            (ArtistSectionKind::Singles, 2),
            (ArtistSectionKind::Compilations, 1),
            (ArtistSectionKind::AppearsOn, 1),
        ]
    );
    assert_eq!(
        section_albums(&apple, artist, "appears_on", 0, 10).await?,
        [album]
    );
    assert_eq!(
        page_keys(
            &apple,
            "lib::artist_section",
            &format!("{artist}:appears_on"),
            0,
            10
        )
        .await?,
        ["catalog:8001"]
    );
    // The song is among the artist's tracks.
    let tracks = artist_track_keys(&apple, artist).await?;
    assert!(tracks.contains(&"catalog:8001".to_owned()), "{tracks:?}");
    apple.finish().await
}

// The catalog artists Rune's mixes name ------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_catalog_artist_only_a_rune_mix_names_is_listed_and_opens_its_catalog_page() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let viewed = viewed_artist(&apple)?;
    let made = create_mix(&apple, "Viewed", vec![query("lib::artist", viewed)]).await?;

    // The catalog lists no album of it, so the library holds none of its songs: it joins the
    // Artists page by the id the catalog gives it.
    let listed = wait_until("the artist to be listed", || async {
        let listed = all_artists(&apple).await?;
        Ok(names(&listed).contains(&"Viewed Artist").then_some(listed))
    })
    .await?;
    assert_eq!(
        names(&listed),
        ["Artist One", "Artist Two", "Viewed Artist"]
    );
    assert_eq!(artist_named(&listed, "Viewed Artist")?, viewed);
    assert_eq!(
        names(&all_albums(&apple).await?),
        ["Dawn Album", "Night Album"]
    );
    assert_eq!(all_tracks(&apple).await?.1, 3);

    // Its page is the catalog's, as a search opens it, not an empty page of the library.
    let page = apple.run(&artist_page_in(viewed, "en")).await?;
    assert_eq!(
        page.sections
            .iter()
            .map(|section| section.kind)
            .collect::<Vec<_>>(),
        [
            ArtistSectionKind::Albums,
            ArtistSectionKind::LatestRelease,
            ArtistSectionKind::Singles,
            ArtistSectionKind::EssentialAlbums,
            ArtistSectionKind::AppearsOn
        ]
    );
    assert!(!page.tracks.is_empty());

    // Once no mix names it, it is not listed.
    apple
        .run(&RemoveMixRequest {
            mix_id: made.id,
            call_id: None,
        })
        .await?;
    assert_eq!(
        names(&all_artists(&apple).await?),
        ["Artist One", "Artist Two"]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_catalog_artist_in_a_rune_mix_brings_the_songs_of_its_albums_once() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            twin: true,
            ..AppleOptions::default()
        },
    )
    .await?;
    let artist = artist_named(&all_artists(&apple).await?, "Twin Artist")?;
    assert_eq!(
        apple.binding.resolve(AppleIdKind::Artist, artist)?,
        "artist:330"
    );
    assert_eq!(artist_track_keys(&apple, artist).await?, ["i.t1"]);

    // The mix plays the songs of the artist's two albums, and the library holds them.
    let catalog = apple.binding.intern(AppleIdKind::Artist, "catalog:330")?;
    let made = create_mix(&apple, "Twin", vec![query("lib::artist", catalog)]).await?;
    assert_eq!(
        mix_titles(&apple, stored_queries(&apple, made.id).await?).await?,
        ["Early One", "Early Two", "Twin One", "Late Two"]
    );
    let first = album_listed(&apple, "Twin First").await?;
    assert_eq!(
        names(&all_albums(&apple).await?),
        ["Dawn Album", "Night Album", "Twin First", "Twin Second"]
    );
    assert_eq!(
        album_titles(&apple, first).await?,
        ["Early One", "Early Two"]
    );
    // The album the library holds one song of holds the others too, the library's copy once.
    let second = apple.binding.intern(AppleIdKind::Album, "l.twin")?;
    assert_eq!(
        album_titles(&apple, second).await?,
        ["Twin One", "Late Two"]
    );
    let (titles, count) = all_tracks(&apple).await?;
    assert_eq!(count, 7);
    assert_eq!(
        titles.iter().filter(|title| *title == "Twin One").count(),
        1
    );
    // The artist's albums were listed to the end of the list, and their songs read together,
    // once.
    assert_eq!(apple.fake.count("/v1/catalog/it/artists/330/albums"), 2);
    assert_eq!(
        apple
            .fake
            .count_containing("/v1/catalog/it/artists/330/albums?offset=1"),
        1
    );
    assert_eq!(apple.fake.count_containing("include=artists,tracks"), 1);
    assert_eq!(apple.fake.count("/v1/catalog/it/albums/"), 0);

    // The artist keeps its library page, with both albums in their section and every song once.
    wait_synced(&apple, &["1950", "1951"]).await?;
    wait_until("the albums to be on the page", || async {
        let page = apple.run(&artist_page_request(artist)).await?;
        Ok((section_counts(&page) == [(ArtistSectionKind::Albums, 2)]).then_some(()))
    })
    .await?;
    let mut albums = section_albums(&apple, artist, "albums", 0, 10).await?;
    albums.sort_unstable();
    let mut wanted = vec![first, second];
    wanted.sort_unstable();
    assert_eq!(albums, wanted);
    assert_eq!(
        artist_track_keys(&apple, artist).await?,
        ["catalog:9501", "catalog:9502", "catalog:9511", "i.t1"]
    );

    // Once no mix names it, the library holds only its own song.
    apple
        .run(&RemoveMixRequest {
            mix_id: made.id,
            call_id: None,
        })
        .await?;
    assert_eq!(
        names(&all_albums(&apple).await?),
        ["Dawn Album", "Night Album", "Twin Second"]
    );
    assert_eq!(album_titles(&apple, second).await?, ["Twin One"]);
    assert_eq!(artist_track_keys(&apple, artist).await?, ["i.t1"]);
    assert_eq!(all_tracks(&apple).await?.1, 4);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_artist_a_mix_names_brings_its_new_releases_once_a_day_has_passed() -> Result<()> {
    use crate::apple_music_held::ARTIST_SONGS_AGE;

    let dir = tempfile::tempdir()?;
    let heard = Arc::new(Heard::default());
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            twin: true,
            heard: Some(heard.clone()),
            ..AppleOptions::default()
        },
    )
    .await?;
    let artist = artist_named(&all_artists(&apple).await?, "Twin Artist")?;
    let catalog = apple.binding.intern(AppleIdKind::Artist, "catalog:330")?;
    let made = create_mix(&apple, "Twin", vec![query("lib::artist", catalog)]).await?;
    album_listed(&apple, "Twin First").await?;
    assert_eq!(all_tracks(&apple).await?.1, 7);

    // Apple lists a new release. Within a day, the library keeps what it read.
    apple.fake.twin_new_album.store(true, Ordering::SeqCst);
    let told = told_settled(&heard).await;
    all_albums(&apple).await?;
    assert_eq!(told_settled(&heard).await, told);
    assert!(!names(&all_albums(&apple).await?).contains(&"Twin Third"));

    // A day later, the artist is read again: the release comes in, and the pages are told.
    apple.clock.advance(ARTIST_SONGS_AGE);
    all_albums(&apple).await?;
    let third = album_listed(&apple, "Twin Third").await?;
    told_after(&heard, told).await?;
    assert_eq!(album_titles(&apple, third).await?, ["New One"]);
    assert_eq!(all_tracks(&apple).await?.1, 8);
    assert_eq!(
        artist_track_keys(&apple, artist).await?,
        [
            "catalog:9501",
            "catalog:9502",
            "catalog:9511",
            "catalog:9521",
            "i.t1"
        ]
    );

    // Once no mix names it, the library holds only its own song.
    apple
        .run(&RemoveMixRequest {
            mix_id: made.id,
            call_id: None,
        })
        .await?;
    assert_eq!(
        names(&all_albums(&apple).await?),
        ["Dawn Album", "Night Album", "Twin Second"]
    );
    assert_eq!(artist_track_keys(&apple, artist).await?, ["i.t1"]);
    apple.finish().await
}
