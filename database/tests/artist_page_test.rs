use anyhow::{Context, Result};
use database::actions::artist_page::{
    YearGroup, artist_page_sections, artist_section_album_groups, artist_section_files,
};
use database::actions::collection::{CollectionQuery, CollectionQueryListMode};
use database::actions::downloads::validate_queries;
use database::actions::mixes::query_mix_media_files;
use database::connection::RecommendationDbConnection;
use database::entities::{
    media_file_likes, media_file_stats, media_files, portable_albums, portable_artists,
};
use database::error::ExpectedError;
use database::mix_operator::ArtistSection;
use database::sync::LibrarySyncStore;
use database::test_fixtures::fixtures;
use fsio::FsIo;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use tempfile::TempDir;
use uuid::Uuid;

/// A library the tests write tagged audio files into and scan.
struct Library {
    db: DatabaseConnection,
    sync: LibrarySyncStore,
    actor: String,
    root: TempDir,
}

impl Library {
    async fn new() -> Result<Self> {
        let actor = Uuid::new_v4();
        let db = fixtures::memory_database(actor).await?;
        let sync = fixtures::open_library(db.clone(), actor, None).await?;
        Ok(Self {
            db,
            sync,
            actor: actor.to_string(),
            root: tempfile::tempdir()?,
        })
    }

    /// Writes the song at `path`, as `<directory>/<file>`, `seconds` long with the tags given
    /// as `(key, value)`.
    fn song(&self, path: &str, seconds: u32, tags: &[(&str, &str)]) -> Result<String> {
        fixtures::write_tagged_flac(&self.root.path().join(path), seconds, tags)?;
        Ok(path.to_owned())
    }

    /// Writes a release of `count` songs of `seconds` each, all by `artist`, in a directory of
    /// its own, with the tags on every song. Its songs' paths come in track order.
    fn release(
        &self,
        artist: &str,
        name: &str,
        count: u32,
        seconds: u32,
        tags: &[(&str, &str)],
    ) -> Result<Vec<String>> {
        (1..=count)
            .map(|number| {
                let number = number.to_string();
                let title = format!("{name} {number}");
                let mut all = vec![
                    ("ARTIST", artist),
                    ("ALBUM", name),
                    ("TITLE", title.as_str()),
                    ("TRACKNUMBER", number.as_str()),
                ];
                all.extend_from_slice(tags);
                self.song(&format!("{name}/{number}.flac"), seconds, &all)
            })
            .collect()
    }

    async fn scan(&self) -> Result<()> {
        fixtures::scan_library(
            &FsIo::new(),
            &self.db,
            &self.actor,
            self.root.path(),
            &self.sync.record_store(),
        )
        .await?;
        Ok(())
    }

    /// The file a song was written to.
    async fn file(&self, path: &str) -> Result<media_files::Model> {
        let (directory, name) = path.rsplit_once('/').context("a path in a directory")?;
        media_files::Entity::find()
            .filter(media_files::Column::Directory.eq(directory))
            .filter(media_files::Column::FileName.eq(name))
            .one(&self.db)
            .await?
            .with_context(|| format!("the scanned file {path}"))
    }

    async fn ids(&self, paths: &[String]) -> Result<Vec<i32>> {
        let mut ids = Vec::new();
        for path in paths {
            ids.push(self.file(path).await?.id);
        }
        Ok(ids)
    }

    /// The id the library lists the artist of this name by.
    async fn artist(&self, name: &str) -> Result<i32> {
        Ok(portable_artists::Entity::find()
            .filter(portable_artists::Column::Name.eq(name))
            .one(&self.db)
            .await?
            .with_context(|| format!("the artist {name}"))?
            .id)
    }

    /// The id the album collection lists the album of this name by.
    async fn album(&self, name: &str) -> Result<i32> {
        Ok(
            portable_albums::Model::list(&self.db, 1000, CollectionQueryListMode::Name)
                .await?
                .into_iter()
                .find(|album| album.name == name)
                .with_context(|| format!("the album {name}"))?
                .id,
        )
    }

    /// Gives every song the cover of its directory, as a cover scan of one cover image in
    /// each directory would.
    async fn covers(&self) -> Result<()> {
        for file in media_files::Entity::find().all(&self.db).await? {
            let cover = format!("cover-{}", file.directory);
            let mut file: media_files::ActiveModel = file.into();
            file.cover_art_uuid = Set(Some(cover));
            file.update(&self.db).await?;
        }
        Ok(())
    }

    async fn stats(&self, id: i32, device: &str, played: i32, skipped: i32) -> Result<()> {
        let track = media_files::Entity::find_by_id(id)
            .one(&self.db)
            .await?
            .context("a song")?;
        media_file_stats::ActiveModel {
            track_uuid: Set(track.source_uuid),
            device_id: Set(device.into()),
            source_uuid: Set(Uuid::new_v4().to_string()),
            skipped: Set(skipped),
            played_through: Set(played),
            updated_at: Set("2026".into()),
            ..Default::default()
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    async fn like(&self, id: i32) -> Result<()> {
        let track = media_files::Entity::find_by_id(id)
            .one(&self.db)
            .await?
            .context("a song")?;
        media_file_likes::ActiveModel {
            track_uuid: Set(track.source_uuid),
            source_uuid: Set(Uuid::new_v4().to_string()),
            liked: Set(true),
            updated_at: Set("2026".into()),
            ..Default::default()
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    async fn query(
        &self,
        operator: &str,
        parameter: String,
        cursor: usize,
        page_size: usize,
    ) -> Result<Vec<i32>> {
        Ok(query_mix_media_files(
            &self.db,
            &RecommendationDbConnection::new(),
            vec![(operator.into(), parameter)],
            cursor,
            page_size,
            &[],
        )
        .await?
        .into_iter()
        .map(|file| file.id)
        .collect())
    }

    /// Every album of the artist's section, page after page of `size`, each under the title of
    /// the group it is in.
    async fn section_albums(
        &self,
        artist: i32,
        section: ArtistSection,
        size: usize,
    ) -> Result<Vec<(String, i32)>> {
        let mut all = Vec::new();
        loop {
            let page =
                artist_section_album_groups(&self.db, artist, section, all.len(), size).await?;
            let before = all.len();
            for group in page {
                all.extend(group.items.into_iter().map(|id| (group.title.clone(), id)));
            }
            if all.len() - before < size {
                return Ok(all);
            }
        }
    }

    async fn every_page(&self, operator: &str, parameter: &str, size: usize) -> Result<Vec<i32>> {
        let mut all = Vec::new();
        loop {
            let page = self
                .query(operator, parameter.into(), all.len(), size)
                .await?;
            if page.is_empty() {
                return Ok(all);
            }
            all.extend(page);
        }
    }

    /// The sections of the artist's page with how many releases each holds.
    async fn sections(&self, artist: i32) -> Result<Vec<(ArtistSection, usize)>> {
        Ok(artist_page_sections(&self.db, artist, 4)
            .await?
            .context("the artist's page")?
            .sections
            .into_iter()
            .map(|section| (section.section, section.releases))
            .collect())
    }

    /// The names of the albums of one section of the artist's page, sorted.
    async fn section_names(&self, artist: i32, section: ArtistSection) -> Result<Vec<String>> {
        let ids: Vec<i32> = self
            .section_albums(artist, section, 100)
            .await?
            .into_iter()
            .map(|(_, id)| id)
            .collect();
        let mut names: Vec<String> = portable_albums::Model::get_by_ids(&self.db, &ids)
            .await?
            .into_iter()
            .map(|album| album.name)
            .collect();
        names.sort_unstable();
        Ok(names)
    }
}

fn is_invalid_input(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<ExpectedError>(),
        Some(ExpectedError::InvalidInput { .. })
    )
}

#[tokio::test]
async fn releases_fall_into_sections_by_their_tags_album_artists_and_length() -> Result<()> {
    let lib = Library::new().await?;
    // Releases whose tags give no type go by their length.
    let single = lib.release("Ann", "Single", 2, 200, &[("DATE", "2020-05-01")])?;
    let ep = lib.release("Ann", "Ep", 5, 200, &[("DATE", "2019")])?;
    // Short, but long enough for an album.
    let long_pair = lib.release("Ann", "Long pair", 2, 1000, &[("DATE", "2015")])?;
    let old_album = lib.release("Ann", "Old album", 10, 240, &[("DATE", "2018")])?;
    // The newer album has two discs and its songs are written out of order.
    let mut new_album = Vec::new();
    for (disc, number) in [("2", "1"), ("1", "2"), ("2", "2"), ("1", "1")] {
        let title = format!("New album {disc}-{number}");
        new_album.push(lib.song(
            &format!("New album/{disc}-{number}.flac"),
            600,
            &[
                ("ARTIST", "Ann"),
                ("ALBUMARTIST", "Ann"),
                ("ALBUM", "New album"),
                ("TITLE", &title),
                ("DATE", "2022"),
                ("DISCNUMBER", disc),
                ("TRACKNUMBER", number),
            ],
        )?);
    }
    let flagged = lib.release("Ann", "Flagged", 8, 200, &[("COMPILATION", "1")])?;
    // A release type tag can name a compilation too, as Picard writes it.
    let various = lib.release(
        "Ann",
        "Various",
        8,
        200,
        &[
            ("ALBUMARTIST", "Various Artists"),
            ("RELEASETYPE", "album; compilation"),
        ],
    )?;
    let live = lib.release("Ann", "Live", 9, 300, &[("RELEASETYPE", "album; live")])?;
    // Ann is on one song of a release by Bob.
    let appears = lib.song(
        "Bob's record/1.flac",
        200,
        &[
            ("ARTIST", "Ann & Bob"),
            ("ALBUMARTIST", "Bob"),
            ("ALBUM", "Bob's record"),
            ("TITLE", "With Ann"),
            ("TRACKNUMBER", "1"),
        ],
    )?;
    let bob_only = lib.song(
        "Bob's record/2.flac",
        200,
        &[
            ("ARTIST", "Bob"),
            ("ALBUMARTIST", "Bob"),
            ("ALBUM", "Bob's record"),
            ("TITLE", "Without Ann"),
            ("TRACKNUMBER", "2"),
        ],
    )?;
    lib.scan().await?;
    lib.covers().await?;
    let ann = lib.artist("Ann").await?;

    let page = artist_page_sections(&lib.db, ann, 2).await?.unwrap();
    assert!(page.has_songs);
    let sections = page.sections;
    let summary: Vec<_> = sections
        .iter()
        .map(|section| (section.section, section.releases))
        .collect();
    assert_eq!(
        summary,
        vec![
            (ArtistSection::Albums, 3),
            // The EP and the single are one section.
            (ArtistSection::Singles, 2),
            (ArtistSection::Compilations, 2),
            (ArtistSection::Live, 1),
            (ArtistSection::AppearsOn, 1),
        ]
    );
    // A tile gets a song of each of the newest releases, up to the limit, one per cover.
    assert_eq!(sections[0].cover_files.len(), 2);
    assert_eq!(
        sections[0].cover_files[0].cover_art_uuid.as_deref(),
        Some("cover-New album")
    );
    assert_eq!(sections[4].cover_files.len(), 1);

    let section = |name: &'static str| format!("{ann}:{name}");
    // Newest release first, the old album (2018) before the long pair (2015), each release in
    // disc and track order. The new album's songs are written as disc 2/1, 1/2, 2/2 and 1/1.
    let new_album = lib.ids(&new_album).await?;
    let expected: Vec<i32> = [new_album[3], new_album[1], new_album[0], new_album[2]]
        .into_iter()
        .chain(lib.ids(&old_album).await?)
        .chain(lib.ids(&long_pair).await?)
        .collect();
    assert_eq!(
        lib.every_page("lib::artist_section", &section("albums"), 100)
            .await?,
        expected
    );
    // Paging gives the same list however it is cut.
    for size in [1, 3, 4, 5] {
        assert_eq!(
            lib.every_page("lib::artist_section", &section("albums"), size)
                .await?,
            expected,
            "page size {size}"
        );
    }
    // The albums of a section are listed in the same order, each under its year, however the
    // pages are cut, by the ids the album collection gives them.
    let expected_albums = [
        ("2022".to_owned(), lib.album("New album").await?),
        ("2018".to_owned(), lib.album("Old album").await?),
        ("2015".to_owned(), lib.album("Long pair").await?),
    ];
    for size in [1, 2, 3, 10] {
        assert_eq!(
            lib.section_albums(ann, ArtistSection::Albums, size).await?,
            expected_albums,
            "page size {size}"
        );
    }
    // The live album has no date, so it is under "#".
    assert_eq!(
        artist_section_album_groups(&lib.db, ann, ArtistSection::Live, 0, 5).await?,
        vec![YearGroup {
            title: "#".to_owned(),
            items: vec![lib.album("Live").await?],
        }]
    );
    // The page after the last is empty, and a section without releases lists none.
    assert!(
        artist_section_album_groups(&lib.db, ann, ArtistSection::Albums, 3, 5)
            .await?
            .is_empty()
    );
    for none in [
        ArtistSection::Eps,
        ArtistSection::LatestRelease,
        ArtistSection::EssentialAlbums,
    ] {
        assert!(
            artist_section_album_groups(&lib.db, ann, none, 0, 5)
                .await?
                .is_empty()
        );
    }
    assert!(
        lib.every_page("lib::artist_section", &section("eps"), 2)
            .await?
            .is_empty()
    );
    let mut singles = lib
        .every_page("lib::artist_section", &section("singles"), 1)
        .await?;
    singles.sort_unstable();
    let mut both: Vec<i32> = lib
        .ids(&ep)
        .await?
        .into_iter()
        .chain(lib.ids(&single).await?)
        .collect();
    both.sort_unstable();
    assert_eq!(singles, both);
    // Both compilations have no date, so they are listed by name.
    assert_eq!(
        lib.every_page("lib::artist_section", &section("compilations"), 5)
            .await?,
        lib.ids(&flagged)
            .await?
            .into_iter()
            .chain(lib.ids(&various).await?)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        lib.every_page("lib::artist_section", &section("live"), 4)
            .await?,
        lib.ids(&live).await?
    );
    // Appears On lists the whole release, not only the artist's song.
    assert_eq!(
        lib.every_page("lib::artist_section", &section("appears_on"), 4)
            .await?,
        lib.ids(&[appears, bob_only]).await?
    );
    Ok(())
}

#[tokio::test]
async fn a_section_plays_one_version_of_each_track_as_the_release_does() -> Result<()> {
    let lib = Library::new().await?;
    // Two versions of each of three tracks in one release, told apart by a comment.
    for number in ["1", "2", "3"] {
        for version in ["a", "b"] {
            let title = format!("Song {number}");
            lib.song(
                &format!("Twice/{number}{version}.flac"),
                300,
                &[
                    ("ARTIST", "Ann"),
                    ("ALBUM", "Twice"),
                    ("TITLE", &title),
                    ("TRACKNUMBER", number),
                    ("DATE", "2020"),
                    ("COMMENT", version),
                ],
            )?;
        }
    }
    lib.scan().await?;
    lib.covers().await?;
    let ann = lib.artist("Ann").await?;
    let album = lib.album("Twice").await?;

    // The release has three tracks and its album plays one version of each.
    assert_eq!(lib.sections(ann).await?, [(ArtistSection::Singles, 1)]);
    let mut release = lib.every_page("lib::album", &album.to_string(), 10).await?;
    release.sort_unstable();
    assert_eq!(release.len(), 3);
    for cover_only in [false, true] {
        let mut section: Vec<i32> =
            artist_section_files(&lib.db, ann, ArtistSection::Singles, cover_only, 0, 10)
                .await?
                .into_iter()
                .map(|file| file.id)
                .collect();
        section.sort_unstable();
        assert_eq!(section, release, "cover only: {cover_only}");
    }
    Ok(())
}

#[tokio::test]
async fn a_typed_release_goes_to_the_section_of_its_type_whatever_its_length() -> Result<()> {
    let lib = Library::new().await?;
    // By their length these would be albums.
    lib.release("Ann", "Long EP", 5, 600, &[("RELEASETYPE", "EP")])?;
    lib.release("Ann", "Long single", 2, 1200, &[("RELEASETYPE", "single")])?;
    lib.release("Ann", "Flagged single", 3, 900, &[("ISSINGLE", "1")])?;
    // By its length this would be a single.
    lib.release("Ann", "Short album", 2, 100, &[("RELEASETYPE", "album")])?;
    lib.scan().await?;
    let ann = lib.artist("Ann").await?;

    assert_eq!(
        lib.sections(ann).await?,
        [(ArtistSection::Albums, 1), (ArtistSection::Singles, 3)]
    );
    assert_eq!(
        lib.section_names(ann, ArtistSection::Albums).await?,
        ["Short album"]
    );
    assert_eq!(
        lib.section_names(ann, ArtistSection::Singles).await?,
        ["Flagged single", "Long EP", "Long single"]
    );
    Ok(())
}

#[tokio::test]
async fn a_compilation_flag_puts_a_release_in_the_compilations() -> Result<()> {
    let lib = Library::new().await?;
    // Albums by their length and by Ann herself, flagged in either way.
    lib.release(
        "Ann",
        "Flagged",
        10,
        300,
        &[("ALBUMARTIST", "Ann"), ("COMPILATION", "1")],
    )?;
    lib.release(
        "Ann",
        "Typed",
        10,
        300,
        &[("RELEASETYPE", "album; compilation")],
    )?;
    // A dedicated flag that says no outweighs the release type, as the catalog reads them.
    lib.release(
        "Ann",
        "Unflagged",
        10,
        300,
        &[("RELEASETYPE", "album; compilation"), ("COMPILATION", "0")],
    )?;
    lib.scan().await?;
    let ann = lib.artist("Ann").await?;

    assert_eq!(
        lib.sections(ann).await?,
        [(ArtistSection::Albums, 1), (ArtistSection::Compilations, 2)]
    );
    assert_eq!(
        lib.section_names(ann, ArtistSection::Albums).await?,
        ["Unflagged"]
    );
    assert_eq!(
        lib.section_names(ann, ArtistSection::Compilations).await?,
        ["Flagged", "Typed"]
    );
    Ok(())
}

#[tokio::test]
async fn an_untagged_release_goes_by_its_length() -> Result<()> {
    let lib = Library::new().await?;
    lib.release("Ann", "Three short", 3, 200, &[])?;
    // Six songs a little under half an hour together.
    lib.release("Ann", "Six short", 6, 290, &[])?;
    lib.release("Ann", "Seven short", 7, 100, &[])?;
    lib.release("Ann", "Long pair", 2, 1000, &[])?;
    lib.scan().await?;
    let ann = lib.artist("Ann").await?;

    assert_eq!(
        lib.sections(ann).await?,
        [(ArtistSection::Albums, 2), (ArtistSection::Singles, 2)]
    );
    assert_eq!(
        lib.section_names(ann, ArtistSection::Albums).await?,
        ["Long pair", "Seven short"]
    );
    assert_eq!(
        lib.section_names(ann, ArtistSection::Singles).await?,
        ["Six short", "Three short"]
    );
    Ok(())
}

#[tokio::test]
async fn the_albums_of_a_section_are_grouped_by_year_newest_first_and_undated_last() -> Result<()> {
    let lib = Library::new().await?;
    for (name, date) in [
        ("Cee", Some("2021")),
        ("B side", Some("2020-06-01")),
        ("A side", Some("2020-01-01")),
        // A date of the year alone is older than the dates of its year that name a day.
        ("Year only", Some("2020")),
        ("Late", Some("2022-12")),
        ("Undated one", None),
        ("Undated two", None),
    ] {
        let tags: Vec<(&str, &str)> = date.map(|date| ("DATE", date)).into_iter().collect();
        lib.release("Ann", name, 8, 300, &tags)?;
    }
    lib.scan().await?;
    let ann = lib.artist("Ann").await?;
    let mut expected = Vec::new();
    for (year, name) in [
        ("2022", "Late"),
        ("2021", "Cee"),
        // One year keeps the section's order: the later date first.
        ("2020", "B side"),
        ("2020", "A side"),
        ("2020", "Year only"),
        ("#", "Undated one"),
        ("#", "Undated two"),
    ] {
        expected.push((year.to_owned(), lib.album(name).await?));
    }
    // The newest release's songs come first.
    let newest = artist_section_files(&lib.db, ann, ArtistSection::Albums, false, 0, 1).await?;
    assert_eq!(newest[0].directory, "Late");
    for size in [1, 2, 3, 4, 10] {
        assert_eq!(
            lib.section_albums(ann, ArtistSection::Albums, size).await?,
            expected,
            "page size {size}"
        );
    }
    // A page that cuts 2020 ends with it, and the next starts with it.
    let titles = |groups: Vec<YearGroup<i32>>| -> Vec<(String, usize)> {
        groups
            .into_iter()
            .map(|group| (group.title, group.items.len()))
            .collect()
    };
    assert_eq!(
        titles(artist_section_album_groups(&lib.db, ann, ArtistSection::Albums, 0, 3).await?),
        [
            ("2022".to_owned(), 1),
            ("2021".to_owned(), 1),
            ("2020".to_owned(), 1)
        ]
    );
    assert_eq!(
        titles(artist_section_album_groups(&lib.db, ann, ArtistSection::Albums, 3, 3).await?),
        [("2020".to_owned(), 2), ("#".to_owned(), 1)]
    );
    Ok(())
}

#[tokio::test]
async fn tracks_are_listed_most_listened_first() -> Result<()> {
    let lib = Library::new().await?;
    let song = |album: &str, number: &str, date: &str| {
        let title = format!("{album} {number}");
        lib.song(
            &format!("{album}/{number}.flac"),
            100,
            &[
                ("ARTIST", "Ann"),
                ("ALBUM", album),
                ("TITLE", &title),
                ("TRACKNUMBER", number),
                ("DATE", date),
            ],
        )
    };
    let paths = [
        song("Old", "1", "2010")?,
        song("New", "2", "2024")?,
        song("New", "1", "2024")?,
        song("Old", "2", "2010")?,
        song("Old", "3", "2010")?,
        song("Old", "4", "2010")?,
        song("Old", "5", "2010")?,
        song("Old", "6", "2010")?,
    ];
    // Someone else's song is not the artist's.
    lib.song(
        "Old/7.flac",
        100,
        &[
            ("ARTIST", "Bob"),
            ("ALBUM", "Old"),
            ("TITLE", "Old 7"),
            ("TRACKNUMBER", "7"),
            ("DATE", "2010"),
        ],
    )?;
    lib.scan().await?;
    let ann = lib.artist("Ann").await?;
    let [
        never_old,
        never_new_2,
        never_new_1,
        never_liked,
        tied_skipped,
        tied_plain,
        tied_liked,
        most,
    ] = lib.ids(&paths).await?[..]
    else {
        unreachable!("eight songs were written");
    };

    lib.like(never_liked).await?;
    lib.like(tied_liked).await?;
    lib.stats(most, "a", 5, 9).await?;
    // The counts of every device are added up.
    lib.stats(tied_plain, "a", 1, 0).await?;
    lib.stats(tied_plain, "b", 1, 0).await?;
    lib.stats(tied_liked, "a", 2, 4).await?;
    lib.stats(tied_skipped, "a", 2, 3).await?;

    let expected = vec![
        most,
        // Played through twice: liked first, then fewer skips.
        tied_liked,
        tied_plain,
        tied_skipped,
        // Never played: liked first, then the newest release, then track order.
        never_liked,
        never_new_1,
        never_new_2,
        never_old,
    ];
    let parameter = ann.to_string();
    assert_eq!(
        lib.every_page("lib::artist_tracks", &parameter, 100)
            .await?,
        expected
    );
    for size in [1, 3] {
        assert_eq!(
            lib.every_page("lib::artist_tracks", &parameter, size)
                .await?,
            expected,
            "page size {size}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn artist_page_queries_stand_alone_and_are_not_offline_rules() -> Result<()> {
    let lib = Library::new().await?;
    // An artist the library knows of without songs, as a synced artist can be.
    let ann = portable_artists::ActiveModel {
        name: Set("Ann".into()),
        group: Set("A".into()),
        source_uuid: Set(Uuid::new_v4().to_string()),
        ..Default::default()
    }
    .insert(&lib.db)
    .await?
    .id;
    // An artist the library does not have has no songs.
    assert!(
        lib.query("lib::artist_tracks", "999".into(), 0, 10)
            .await?
            .is_empty()
    );
    assert!(
        artist_section_files(&lib.db, 999, ArtistSection::Albums, false, 0, 10)
            .await?
            .is_empty()
    );
    assert!(
        artist_section_album_groups(&lib.db, 999, ArtistSection::Albums, 0, 10)
            .await?
            .is_empty()
    );
    assert!(artist_page_sections(&lib.db, 999, 4).await?.is_none());
    let empty = artist_page_sections(&lib.db, ann, 4).await?.unwrap();
    assert!(empty.sections.is_empty());
    assert!(!empty.has_songs);
    let alone = query_mix_media_files(
        &lib.db,
        &RecommendationDbConnection::new(),
        vec![
            ("lib::artist_tracks".into(), ann.to_string()),
            ("lib::all".into(), "true".into()),
        ],
        0,
        10,
        &[],
    )
    .await;
    assert!(is_invalid_input(&alone.unwrap_err()));
    for query in [
        ("lib::artist_tracks", ann.to_string()),
        ("lib::artist_section", format!("{ann}:albums")),
    ] {
        assert!(validate_queries(&[(query.0.into(), query.1)]).is_err());
    }
    Ok(())
}

#[tokio::test]
async fn an_artist_page_query_may_come_with_the_cover_filter_and_nothing_else() -> Result<()> {
    let lib = Library::new().await?;
    let songs = lib.release("Ann", "Album", 8, 300, &[("DATE", "2020")])?;
    lib.scan().await?;
    lib.covers().await?;
    let ann = lib.artist("Ann").await?;
    let songs = lib.ids(&songs).await?;
    // The second and third songs have no cover.
    for song in &songs[1..3] {
        media_files::Entity::update_many()
            .col_expr(
                media_files::Column::CoverArtUuid,
                sea_orm::sea_query::Expr::value(Option::<String>::None),
            )
            .filter(media_files::Column::Id.eq(*song))
            .exec(&lib.db)
            .await?;
    }
    let mut with_covers: Vec<i32> = songs
        .iter()
        .copied()
        .filter(|id| *id != songs[1] && *id != songs[2])
        .collect();
    with_covers.sort_unstable();
    let cover = ("filter::with_cover_art".to_owned(), "true".to_owned());
    for page in [
        ("lib::artist_section".to_owned(), format!("{ann}:albums")),
        ("lib::artist_tracks".to_owned(), ann.to_string()),
    ] {
        let ask = |extra: Vec<(String, String)>, cursor: usize, size: usize| {
            let mut queries = vec![page.clone()];
            queries.extend(extra);
            let db = lib.db.clone();
            async move {
                query_mix_media_files(
                    &db,
                    &RecommendationDbConnection::new(),
                    queries,
                    cursor,
                    size,
                    &[],
                )
                .await
            }
        };
        // The filter drops the songs before the page is cut, so a page is full.
        let mut all = Vec::new();
        for _ in 0..3 {
            let next = ask(vec![cover.clone()], all.len(), 4)
                .await?
                .into_iter()
                .map(|file| file.id);
            all.extend(next);
        }
        all.sort_unstable();
        assert_eq!(all, with_covers, "{}", page.0);
        assert_eq!(ask(vec![cover.clone()], 0, 4).await?.len(), 4);
        // Without the filter every song is listed.
        assert_eq!(ask(Vec::new(), 0, 20).await?.len(), 8);
        // Other clauses, and the filter for songs without a cover, are still refused.
        for extra in [
            vec![("lib::all".to_owned(), "true".to_owned())],
            vec![("sort::duration".to_owned(), "true".to_owned())],
            vec![("filter::with_cover_art".to_owned(), "false".to_owned())],
            vec![
                cover.clone(),
                ("filter::liked".to_owned(), "true".to_owned()),
            ],
        ] {
            let error = ask(extra, 0, 20).await.unwrap_err();
            assert!(is_invalid_input(&error), "{}: {error:#}", page.0);
        }
    }
    Ok(())
}

/// The median of five timings of `call`, in milliseconds.
async fn median_ms<F, Fut, T>(mut call: F) -> Result<f64>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut times = Vec::new();
    for _ in 0..5 {
        let start = std::time::Instant::now();
        call().await?;
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    Ok(times[2])
}

/// Timing of the artist page on a large catalog. Run with
/// `cargo test -p database --release --test artist_page_test -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "timing run, slow to set up"]
async fn artist_page_timing() -> Result<()> {
    let lib = Library::new().await?;
    // 300 releases of 10 songs, with tags, stats and likes.
    let mut ann_songs = Vec::new();
    for r in 0..300 {
        let date = format!("{}", 1970 + r % 50);
        let mut tags = vec![("DATE", date.as_str()), ("DISCNUMBER", "1")];
        if r % 5 == 0 {
            tags.push(("RELEASETYPE", "album; live"));
        }
        if r % 7 == 0 {
            tags.push(("ALBUMARTIST", "Ann"));
        }
        ann_songs.push(lib.release("Ann", &format!("Release {r}"), 10, 240, &tags)?);
    }
    // 20,000 songs of others in the same library.
    for r in 0..2000 {
        lib.release(
            "Other",
            &format!("Other {r}"),
            10,
            240,
            &[("DATE", "2000"), ("DISCNUMBER", "1")],
        )?;
    }
    lib.scan().await?;
    let ann = lib.artist("Ann").await?;
    for (r, songs) in ann_songs.iter().enumerate() {
        for (i, song) in lib.ids(songs).await?.into_iter().enumerate() {
            if i % 2 == 0 {
                lib.stats(song, "a", (i + r % 4) as i32, 1).await?;
                lib.stats(song, "b", 1, 0).await?;
            }
            if i % 5 == 0 {
                lib.like(song).await?;
            }
        }
    }

    let parameter = ann.to_string();
    let sections = artist_page_sections(&lib.db, ann, 4)
        .await?
        .unwrap()
        .sections
        .len();
    let t = median_ms(|| artist_page_sections(&lib.db, ann, 4)).await?;
    println!("sections ({sections} sections): {t:.1} ms");
    let section_parameter = format!("{ann}:albums");
    let total = lib
        .every_page("lib::artist_section", &section_parameter, 100)
        .await?
        .len();
    println!("albums section holds {total} songs");
    for (operator, parameter, total) in [
        ("lib::artist_section", section_parameter.as_str(), total),
        ("lib::artist_tracks", parameter.as_str(), 3000),
    ] {
        let last = (total - 1) / 100 * 100;
        for (name, cursor) in [("first", 0), ("middle", total / 200 * 100), ("last", last)] {
            let t = median_ms(|| lib.query(operator, parameter.into(), cursor, 100)).await?;
            println!("{operator} {name} page (cursor {cursor}): {t:.1} ms");
        }
    }
    Ok(())
}
