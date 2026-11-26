//! The page of an artist of an Apple Music library: the artist's releases in sections, and the
//! artist's tracks.
//!
//! An artist Apple's catalog names is read through one catalog request that holds the first
//! page of each of the artist's views of releases and of its top songs. Later pages of a view
//! are read only when a page of its songs needs them, and the songs of a release only when a
//! page reaches it. Each view is one section: full albums are Albums, singles and EPs are
//! Singles, live albums Live, compilations Compilations, the albums of others the artist appears
//! on Appears On, and the latest release and the albums Apple picks as essential have sections
//! of their own.
//!
//! An artist opened from the library's artist list, as from the home or the library, is
//! answered from the library's own kept lists alone, with or without a catalog artist Apple
//! names for it: each of its releases goes to the section the catalog artist's views put it in,
//! as kept, or else to the one the kept catalog facts of the album put it in, and its tracks are
//! its songs, the liked ones first and then in album order. The views are read behind the page
//! when it is opened online and they are not kept or old. Only an artist that is no entry of the
//! list, as one a search found, opens the catalog's page.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use anyhow::Result;
use apple_music::models::{Album, ArtistWithViews, LibraryAlbum, Song};
use database::actions::artist_page::{SECTION_COVERS, YearGroup, year_groups};
use database::error::ExpectedError;
use database::mix_operator::ArtistSection as ReleaseSection;

use super::{
    CATALOG_PREFIX, Listed, album_artwork, album_name, album_songs_from_list,
    artist_songs_by_album, is_not_found,
};
use crate::apple_music_artists::{
    CATALOG_ARTIST, NamedArtist, NamedArtists, artist_key, is_named_artist,
};
use crate::apple_music_binding::{
    AppleError, AppleIdKind, AppleMusicBinding, AppleSong, CatalogCollection, catalog_track,
    cover_url, release_year,
};
use crate::apple_music_classes::{AlbumFacts, ArtistViews, Classes, section_of, title_tag};
use crate::apple_music_held::catalog_album_of;
use crate::handlers::artist::{named_section, page_bounds, section_query, tracks_query};
use crate::messages::{
    Album as AlbumRef, ArtistSection, ArtistSectionAlbumGroup, ArtistSectionOpens,
    FetchArtistPageRequest, FetchArtistPageResponse, FetchArtistSectionAlbumsRequest,
    FetchArtistSectionAlbumsResponse,
};
use crate::service::home::denies_the_account;
use crate::service::mixes::ArtistListing;

/// One of the lists of releases Apple keeps for a catalog artist.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ReleaseView {
    /// `latest-release`: the artist's newest release.
    LatestRelease,
    /// `full-albums`.
    FullAlbums,
    /// `singles`, which also holds the artist's EPs.
    Singles,
    /// `compilation-albums`.
    CompilationAlbums,
    /// `live-albums`.
    LiveAlbums,
    /// `appears-on-albums`.
    AppearsOnAlbums,
    /// `featured-albums`: the albums Apple picks as the artist's essential ones.
    FeaturedAlbums,
}

impl ReleaseView {
    /// Every view.
    const ALL: [ReleaseView; 7] = [
        Self::LatestRelease,
        Self::FullAlbums,
        Self::Singles,
        Self::FeaturedAlbums,
        Self::LiveAlbums,
        Self::CompilationAlbums,
        Self::AppearsOnAlbums,
    ];

    /// The name Apple gives the view.
    fn name(self) -> &'static str {
        match self {
            Self::LatestRelease => "latest-release",
            Self::FeaturedAlbums => "featured-albums",
            Self::FullAlbums => "full-albums",
            Self::Singles => "singles",
            Self::CompilationAlbums => "compilation-albums",
            Self::LiveAlbums => "live-albums",
            Self::AppearsOnAlbums => "appears-on-albums",
        }
    }
}

/// The view of a catalog artist that holds its most popular songs in the storefront.
const TOP_SONGS: &str = "top-songs";

/// The sections of a catalog artist's page, in the order the page lists them: the albums
/// first, as on every artist page, and the latest release right after them. EPs and singles
/// are one section, Singles, as Apple Music has it: the `singles` view as Apple orders it.
const SECTIONS: [ReleaseSection; 7] = [
    ReleaseSection::Albums,
    ReleaseSection::LatestRelease,
    ReleaseSection::Singles,
    ReleaseSection::EssentialAlbums,
    ReleaseSection::Live,
    ReleaseSection::Compilations,
    ReleaseSection::AppearsOn,
];

/// The sections of a library artist's page, in the same order: the catalog page's, without
/// the latest release and the essential albums, which the full page from a search has alone.
/// Each is the section of one of the views that sort an artist's releases by kind.
const LIBRARY_SECTIONS: [ReleaseSection; 5] = [
    ReleaseSection::Albums,
    ReleaseSection::Singles,
    ReleaseSection::Live,
    ReleaseSection::Compilations,
    ReleaseSection::AppearsOn,
];

/// How many more pages of one view are read at most when all of its releases are needed: by
/// the background read of an artist's views, whose releases past them are placed by their
/// facts, and by a page of a section's albums, which leaves them out.
const VIEW_PAGES: usize = 25;

/// How many releases a page of a section's songs reads at once at most, in one request, when
/// the page is long enough to reach them.
const RELEASE_BATCH: usize = 100;

/// How many songs a page of a section assumes a release holds, to know how many releases it
/// reaches when it reads them ahead.
const SONGS_PER_RELEASE: usize = 8;

/// How long what a page read of a catalog artist counts as young. A page opened after reads the
/// artist again; the pages of its lists in between page through what was read.
const READS_AGE: Duration = Duration::from_secs(60);

/// How many artists the background read of the section titles asks at most.
const TITLE_ARTISTS: usize = 4;

/// Who an artist of the page is.
enum Identity {
    /// An artist Apple's catalog names, by its catalog id.
    Catalog(String),
    /// An artist of the library's artist list, answered from the library's data.
    Library(Box<NamedArtist>),
}

/// The releases of one view that have been read, and where the next page is.
struct ViewReads {
    releases: Vec<Album>,
    next: Option<String>,
}

/// The top songs that have been read, and where the next page is.
struct TopSongs {
    songs: Vec<Song>,
    next: Option<String>,
}

/// The first page of a catalog artist's views as read, and what later pages added to them.
/// Kept by the binding for a while, so paging through a list asks Apple only for what has not
/// been read.
pub(crate) struct ArtistReads {
    /// When the first page was read, by the binding's clock.
    read: Instant,
    views: tokio::sync::Mutex<HashMap<ReleaseView, ViewReads>>,
    /// Apple's title of each view, by the language tag it was read in. The first read gives
    /// the releases, in whatever language that was; the titles of another language are read
    /// when a page asks for it, and leave the releases as they are.
    titles: Mutex<HashMap<String, HashMap<ReleaseView, String>>>,
    top_songs: tokio::sync::Mutex<TopSongs>,
    /// How many songs each release held when they were read, by catalog id, so a page after
    /// it passes it without reading them again.
    song_counts: Mutex<HashMap<String, usize>>,
    /// The songs of the releases that one request for many of them read, by catalog id, so a
    /// page after it needs no read of them.
    release_songs: Mutex<HashMap<String, Arc<Vec<Song>>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The names of the views a read of a catalog artist's page asks for.
fn view_names() -> Vec<&'static str> {
    ReleaseView::ALL
        .iter()
        .map(|view| view.name())
        .chain([TOP_SONGS])
        .collect()
}

/// The titles Apple gave the views of `artist` that hold releases.
fn view_titles_of(artist: &ArtistWithViews) -> HashMap<ReleaseView, String> {
    ReleaseView::ALL
        .into_iter()
        .filter_map(|view| {
            let page = artist.views.get(view.name())?;
            let title = page.attributes.as_ref()?.title.as_deref()?.trim();
            (!title.is_empty() && !page.data.is_empty()).then(|| (view, title.to_owned()))
        })
        .collect()
}

/// The titles of the views of `artist`, by the syntax of the library page's section each is
/// the view of.
fn view_titles(artist: &ArtistWithViews) -> BTreeMap<String, String> {
    let titles = view_titles_of(artist);
    LIBRARY_SECTIONS
        .into_iter()
        .filter_map(|section| {
            let title = titles.get(&view_of(section)?)?;
            Some((section.syntax().to_owned(), title.clone()))
        })
        .collect()
}

/// The view of Apple's that holds the releases of a section. EPs are in Singles.
fn view_of(section: ReleaseSection) -> Option<ReleaseView> {
    match section {
        ReleaseSection::Albums => Some(ReleaseView::FullAlbums),
        ReleaseSection::Singles => Some(ReleaseView::Singles),
        ReleaseSection::Compilations => Some(ReleaseView::CompilationAlbums),
        ReleaseSection::Live => Some(ReleaseView::LiveAlbums),
        ReleaseSection::AppearsOn => Some(ReleaseView::AppearsOnAlbums),
        ReleaseSection::LatestRelease => Some(ReleaseView::LatestRelease),
        ReleaseSection::EssentialAlbums => Some(ReleaseView::FeaturedAlbums),
        ReleaseSection::Eps => None,
    }
}

/// The name of a catalog album.
fn catalog_album_name(album: &Album) -> &str {
    album
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.name.as_deref())
        .unwrap_or_default()
}

/// The year a catalog album came out.
fn catalog_album_year(album: &Album) -> Option<u32> {
    release_year(album.attributes.as_ref()?.release_date.as_deref())
}

/// The cover of a catalog album.
fn catalog_album_cover(album: &Album) -> Option<String> {
    cover_url(album.attributes.as_ref()?.artwork.as_ref())
}

/// The year a library album came out.
fn album_year(album: &LibraryAlbum) -> Option<u32> {
    release_year(album.attributes.as_ref()?.release_date.as_deref())
}

/// The id the UI numbers the catalog album `album` by, as it numbers every catalog album, with
/// the album's name and cover remembered for its page, unless a read under `generation` was
/// dropped since.
fn number_catalog_album(
    binding: &AppleMusicBinding,
    generation: u64,
    album: &Album,
) -> Result<i32> {
    let id = binding.intern(AppleIdKind::Album, &format!("{CATALOG_PREFIX}{}", album.id))?;
    let artwork = album
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.artwork.as_ref());
    binding.remember_collection(
        generation,
        AppleIdKind::Album,
        id,
        catalog_album_name(album),
        artwork,
    );
    Ok(id)
}

/// The page of the albums of `section` from `cursor`, at most `count`, given in the section's
/// order with their years, in the groups its albums page shows: the essential albums, which
/// Apple picks and orders, as one group under the section's own title, which an empty title
/// stands for, and every other section's in groups by year (see `year_groups`). The other
/// views list releases by kind, and the latest release opens as its one album.
fn section_groups<T>(
    section: ReleaseSection,
    albums: impl IntoIterator<Item = (T, Option<u32>)>,
    cursor: usize,
    count: usize,
) -> Vec<YearGroup<T>> {
    if section != ReleaseSection::EssentialAlbums {
        return year_groups(albums, cursor, count);
    }
    let items: Vec<T> = albums
        .into_iter()
        .map(|(album, _)| album)
        .skip(cursor)
        .take(count)
        .collect();
    if items.is_empty() {
        return Vec::new();
    }
    vec![YearGroup {
        title: String::new(),
        items,
    }]
}

/// What opening a section shows: the songs of Singles & EPs, which are short, the one album of
/// the latest release, and the releases as albums for every other.
fn opens_of(section: ReleaseSection) -> ArtistSectionOpens {
    match section {
        ReleaseSection::Singles | ReleaseSection::Eps => ArtistSectionOpens::Songs,
        ReleaseSection::LatestRelease => ArtistSectionOpens::Album,
        ReleaseSection::Albums
        | ReleaseSection::Compilations
        | ReleaseSection::Live
        | ReleaseSection::AppearsOn
        | ReleaseSection::EssentialAlbums => ArtistSectionOpens::Albums,
    }
}

impl ArtistReads {
    /// The reads of `artist`, which Apple answered in the language `tag` when it is given, read
    /// at `read`.
    fn new(artist: &ArtistWithViews, tag: Option<&str>, read: Instant) -> Self {
        let titles = tag
            .map(|tag| (tag.to_owned(), view_titles_of(artist)))
            .into_iter()
            .collect();
        let views = ReleaseView::ALL
            .into_iter()
            .filter_map(|view| {
                let page = artist.view(view.name());
                let releases = page.albums();
                (!releases.is_empty()).then_some((
                    view,
                    ViewReads {
                        releases,
                        next: page.next,
                    },
                ))
            })
            .collect();
        let top = artist.view(TOP_SONGS);
        Self {
            read,
            views: tokio::sync::Mutex::new(views),
            titles: Mutex::new(titles),
            top_songs: tokio::sync::Mutex::new(TopSongs {
                songs: top.songs(),
                next: top.next,
            }),
            song_counts: Mutex::new(HashMap::new()),
            release_songs: Mutex::new(HashMap::new()),
        }
    }

    /// The release of `section` at `index`, reading the pages of its view that come before it
    /// when they have not been read; `None` when the section holds no more.
    async fn release(
        &self,
        binding: &AppleMusicBinding,
        section: ReleaseSection,
        index: usize,
    ) -> Result<Option<Album>> {
        let Some(view) = view_of(section) else {
            return Ok(None);
        };
        // The latest release is one release, whatever Apple lists in its view.
        if section == ReleaseSection::LatestRelease && index > 0 {
            return Ok(None);
        }
        let mut views = self.views.lock().await;
        loop {
            let Some(reads) = views.get_mut(&view) else {
                return Ok(None);
            };
            if let Some(release) = reads.releases.get(index) {
                return Ok(Some(release.clone()));
            }
            // The cursor stays until the page is read, so a failed read leaves the list to be
            // read again, not ended.
            let Some(next) = reads.next.clone() else {
                return Ok(None);
            };
            let page = binding.read_catalog_view_page(&next).await?;
            reads.releases.extend(page.albums());
            reads.next = page.next;
        }
    }

    /// Every release of `view`, reading the pages of it that have not been read, up to its end
    /// or `VIEW_PAGES` more pages.
    async fn all_releases(
        &self,
        binding: &AppleMusicBinding,
        view: ReleaseView,
    ) -> Result<Vec<Album>> {
        let mut views = self.views.lock().await;
        let Some(reads) = views.get_mut(&view) else {
            return Ok(Vec::new());
        };
        for _ in 0..VIEW_PAGES {
            let Some(next) = reads.next.clone() else {
                break;
            };
            let page = binding.read_catalog_view_page(&next).await?;
            reads.releases.extend(page.albums());
            reads.next = page.next;
        }
        if reads.next.is_some() {
            log::warn!(
                "An Apple Music artist lists more than {} releases in the {} view, and the others are not read",
                reads.releases.len(),
                view.name()
            );
        }
        Ok(reads.releases.clone())
    }

    /// Every release of `section`, reading its view to its end (see `all_releases`). The latest
    /// release is one release, whatever its view lists.
    async fn section_releases(
        &self,
        binding: &AppleMusicBinding,
        section: ReleaseSection,
    ) -> Result<Vec<Album>> {
        let Some(view) = view_of(section) else {
            return Ok(Vec::new());
        };
        let mut releases = self.all_releases(binding, view).await?;
        if section == ReleaseSection::LatestRelease {
            releases.truncate(1);
        }
        Ok(releases)
    }

    /// How many releases `section` holds among those read, and the covers of the first.
    async fn known(&self, section: ReleaseSection) -> (usize, Vec<String>) {
        let views = self.views.lock().await;
        let Some(reads) = view_of(section).and_then(|view| views.get(&view)) else {
            return (0, Vec::new());
        };
        let releases = &reads.releases;
        let limit = if section == ReleaseSection::LatestRelease {
            1
        } else {
            SECTION_COVERS
        };
        let covers = releases
            .iter()
            .filter_map(catalog_album_cover)
            .take(limit)
            .collect();
        let count = if section == ReleaseSection::LatestRelease {
            releases.len().min(1)
        } else {
            releases.len()
        };
        (count, covers)
    }

    /// Apple's title of the view of `section` in the language `tag`; empty when there is no
    /// language or Apple gave none.
    fn title_of(&self, section: ReleaseSection, tag: Option<&str>) -> String {
        let titles = lock(&self.titles);
        tag.and_then(|tag| titles.get(tag))
            .zip(view_of(section))
            .and_then(|(titles, view)| titles.get(&view))
            .cloned()
            .unwrap_or_default()
    }

    /// Reads the titles of the views of the catalog artist `id` in the language `tag` when they
    /// are not read yet.
    async fn learn_titles(&self, binding: &AppleMusicBinding, id: &str, tag: &str) -> Result<()> {
        if lock(&self.titles).contains_key(tag) {
            return Ok(());
        }
        let artist = binding
            .read_catalog_artist_views(id, &view_names(), Some(tag))
            .await?;
        lock(&self.titles).insert(tag.to_owned(), view_titles_of(&artist));
        Ok(())
    }

    fn song_count(&self, release: &str) -> Option<usize> {
        lock(&self.song_counts).get(release).copied()
    }

    fn kept_release_songs(&self, release: &str) -> Option<Arc<Vec<Song>>> {
        lock(&self.release_songs).get(release).cloned()
    }

    fn keep_release_songs(&self, release: String, songs: Arc<Vec<Song>>) {
        lock(&self.release_songs).insert(release, songs);
    }

    fn keep_song_count(&self, release: &str, count: usize) {
        lock(&self.song_counts).insert(release.to_owned(), count);
    }
}

/// Who the artist the UI numbered `id` is. An entry of the library's artist list is the
/// library's, whether or not Apple names a catalog artist for it. A catalog artist that is no
/// entry of the list, as one of a catalog song or a search, is the catalog's.
async fn identity(binding: &AppleMusicBinding, id: i32) -> Result<Identity> {
    let not_found = || ExpectedError::NotFound {
        what: "Apple Music artist",
    };
    let remote = binding.resolve(AppleIdKind::Artist, id)?;
    if is_named_artist(&remote) {
        let named = binding.named_artists().await?;
        if let Some(artist) = named.get(&remote) {
            return Ok(Identity::Library(Box::new(artist.clone())));
        }
        // A catalog artist the list does not hold opens from the catalog.
        return match remote.strip_prefix(CATALOG_ARTIST) {
            Some(catalog) if !catalog.is_empty() => Ok(Identity::Catalog(catalog.to_owned())),
            _ => Err(not_found().into()),
        };
    }
    if let Some(catalog) = remote.strip_prefix(CATALOG_PREFIX) {
        return Ok(Identity::Catalog(catalog.to_owned()));
    }
    // A library artist the UI holds is looked for among the artists the list shows.
    let named = binding.named_artists().await?;
    let artist = named
        .iter()
        .find(|artist| artist.credits.contains(&remote))
        .cloned()
        .ok_or_else(not_found)?;
    Ok(Identity::Library(Box::new(artist)))
}

/// What the page has read of the catalog artist `id`: what is kept, or else read. A page being
/// opened (`fresh`) reads again what is older than `READS_AGE`.
async fn reads_of(
    binding: &AppleMusicBinding,
    id: &str,
    fresh: bool,
    tag: Option<&str>,
) -> Result<Arc<ArtistReads>> {
    if let Some(kept) = binding.kept_artist_reads(id)
        && (!fresh || binding.now().saturating_duration_since(kept.read) < READS_AGE)
    {
        return Ok(kept);
    }
    let generation = binding.generation();
    let artist = binding
        .read_catalog_artist_views(id, &view_names(), tag)
        .await?;
    let reads = Arc::new(ArtistReads::new(&artist, tag, binding.now()));
    binding.keep_artist_reads(generation, id, Arc::clone(&reads));
    let (ends, total) = {
        let top = reads.top_songs.lock().await;
        (top.next.is_none(), top.songs.len())
    };
    if ends {
        note_top_songs_total(binding, id, total);
    }
    Ok(reads)
}

/// Logs once per artist how many top songs Apple gave in all, which it does not document.
fn note_top_songs_total(binding: &AppleMusicBinding, artist: &str, total: usize) {
    if binding.first_top_songs_total(artist) {
        log::info!(
            "Apple Music gave {total} top songs in all for the catalog artist {artist}, and no next page"
        );
    }
}

/// A release of the library the artist's page lists, with the cover it is shown by.
#[derive(Clone)]
struct Kept {
    album: LibraryAlbum,
    cover: Option<String>,
}

/// The albums of the library by the artist, the newest year first, then by name.
async fn library_albums(
    binding: &AppleMusicBinding,
    artist: &NamedArtist,
) -> Result<Vec<LibraryAlbum>> {
    let albums = binding.albums().await?;
    let named = binding.named_artists().await?;
    Ok(albums_of_artist(&albums, artist, &named))
}

/// The albums of `albums` that `artist`, an artist of the list `named`, is the artist of,
/// newest first: those of a library artist that is it, and those credited by its name or
/// another spelling whose library artist is no artist of the list. An album of a library
/// artist that is another artist of the list is that artist's, whatever its name.
fn albums_of_artist(
    albums: &[LibraryAlbum],
    artist: &NamedArtist,
    named: &NamedArtists,
) -> Vec<LibraryAlbum> {
    let names: Vec<String> = std::iter::once(&artist.name)
        .chain(&artist.aliases)
        .map(|name| artist_key(name))
        .collect();
    let listed: HashSet<&str> = named
        .iter()
        .flat_map(|each| each.credits.iter().map(String::as_str))
        .collect();
    let mut found: Vec<LibraryAlbum> = albums
        .iter()
        .filter(|album| match album.artist_id() {
            Some(id) if listed.contains(id) => artist.credits.iter().any(|credit| credit == id),
            _ => album
                .attributes
                .as_ref()
                .and_then(|attributes| attributes.artist_name.as_deref())
                .is_some_and(|name| names.contains(&artist_key(name))),
        })
        .cloned()
        .collect();
    sort_albums(&mut found);
    found
}

/// Newest year first, then by name.
fn sort_albums(albums: &mut [LibraryAlbum]) {
    albums.sort_by_cached_key(|album| {
        (
            album_year(album).map_or(u32::MAX, |year| u32::MAX - 1 - year),
            album_name(album).to_lowercase(),
        )
    });
}

/// The albums the artist is on in the library: those it is the artist of, and those holding a
/// song it is credited with.
async fn held_albums(
    binding: &AppleMusicBinding,
    artist: &NamedArtist,
) -> Result<Vec<LibraryAlbum>> {
    let albums = binding.albums().await?;
    let named = binding.named_artists().await?;
    let mut found = albums_of_artist(&albums, artist, &named);
    let list = binding.songs().await?;
    let credited: HashSet<&str> = list
        .library_entries()
        .filter(|(track, _)| artist.credits_track(track))
        .filter_map(|(_, song)| song.album_id())
        .collect();
    let mut have: HashSet<String> = found.iter().map(|album| album.id.clone()).collect();
    for album in albums.iter() {
        if credited.contains(album.id.as_str()) && have.insert(album.id.clone()) {
            found.push(album.clone());
        }
    }
    Ok(found)
}

/// One section of a library artist's page: the library's releases in it, under Apple's title
/// of the section in the client's language.
struct LibrarySection {
    section: ReleaseSection,
    /// Apple's title for the section; empty when it is not kept in the client's language,
    /// for this section or another of the page.
    title: String,
    releases: Vec<Kept>,
}

/// The sections of a library artist's page, in page order, empty ones left out: the albums
/// the library holds, each in the section the catalog artist's views put it in, as kept, or
/// else the one its kept catalog facts put it in (see `placed`), under the titles Apple gave
/// in the client's `language`, or else in the storefront's default language (see `arranged`).
/// It reads nothing of the catalog, so the page never waits on it. A page being opened
/// (`classify`) reads in the background the facts of the albums that lack them, the titles when
/// they are not kept, and, online, the artist's views when what is kept of them is missing or
/// old; each reloads the page when it changed what shows.
async fn library_sections(
    binding: &AppleMusicBinding,
    artist: &NamedArtist,
    classify: bool,
    language: &str,
) -> Result<Vec<LibrarySection>> {
    let Some(catalog) = artist.catalog_id() else {
        return Ok(arranged(
            library_albums(binding, artist).await?,
            &Classes::default(),
            "",
            "",
        ));
    };
    let held = held_albums(binding, artist).await?;
    let classes = binding.classes.get(binding).await;
    if classify {
        let ids: Vec<String> = held
            .iter()
            .filter_map(|album| catalog_album_of(album).map(str::to_owned))
            .collect();
        let now = binding.epoch_seconds();
        if !classes
            .views_of(catalog)
            .is_some_and(|views| views.is_fresh(now))
        {
            views_in_background(binding, catalog, ids.clone());
        }
        classify_in_background(binding, ids);
    }
    let sections = arranged(held.clone(), &classes, catalog, language);
    if classify && sections.iter().any(|section| section.title.is_empty()) {
        // This artist first, then the others of the albums, for the sections it lacks.
        let mut artists = vec![catalog.to_owned()];
        for id in held.iter().filter_map(catalog_album_of) {
            for other in classes.albums.get(id).into_iter().flat_map(|f| &f.artists) {
                if !artists.contains(other) {
                    artists.push(other.clone());
                }
            }
        }
        titles_in_background(binding, language, artists);
    }
    Ok(sections)
}

/// The section of the page of the catalog artist `artist` that a held album with the catalog
/// id `id` is in: the one the artist's views put it in, as kept, else the one its kept facts
/// put it in (see `section_of`). An album the views list under another id, as another edition
/// of the release, is placed by its facts too. What has no facts, and what would be in a
/// section a library page does not have, is Albums.
fn placed(classes: &Classes, artist: &str, id: Option<&str>) -> ReleaseSection {
    let listed = id
        .zip(classes.views_of(artist))
        .and_then(|(id, views)| views.section(id));
    let section =
        listed.unwrap_or_else(|| section_of(id.and_then(|id| classes.albums.get(id)), artist));
    if LIBRARY_SECTIONS.contains(&section) {
        section
    } else {
        ReleaseSection::Albums
    }
}

/// The held albums in the sections they are in on the page of the catalog artist `artist`
/// (see `placed`), newest first in each. Each section has Apple's title in the client's
/// `language`, or else in the storefront's default language, when it is kept, and an empty one
/// otherwise, which the client names.
fn arranged(
    mut held: Vec<LibraryAlbum>,
    classes: &Classes,
    artist: &str,
    language: &str,
) -> Vec<LibrarySection> {
    sort_albums(&mut held);
    let mut by_section: HashMap<ReleaseSection, Vec<LibraryAlbum>> = HashMap::new();
    for album in held {
        let section = placed(classes, artist, catalog_album_of(&album));
        by_section.entry(section).or_default().push(album);
    }
    let mut sections: Vec<LibrarySection> = LIBRARY_SECTIONS
        .into_iter()
        .filter_map(|section| {
            let albums = by_section.remove(&section)?;
            Some(LibrarySection {
                section,
                title: String::new(),
                releases: albums
                    .into_iter()
                    .map(|album| Kept {
                        cover: cover_url(album_artwork(&album)),
                        album,
                    })
                    .collect(),
            })
        })
        .collect();
    let titles = classes.titles_for(language);
    for each in &mut sections {
        each.title = titles
            .and_then(|titles| titles.get(each.section.syntax()))
            .cloned()
            .unwrap_or_default();
    }
    sections
}

/// Reads the titles of the sections in the client's `language`, or else in the storefront's
/// default language (see `title_tag`), in the background, from the pages of `artists` (catalog
/// ids) until every section has one, and keeps them with the storefront's languages. The lists
/// reload once if that changed what pages show. It asks once per language and first artist
/// under a sign-in, whose sign-out forgets the titles, and a read that failed, wholly or for one
/// of the artists, is only logged, so the next page asks again.
fn titles_in_background(binding: &AppleMusicBinding, language: &str, artists: Vec<String>) {
    let Some(first) = artists.first() else {
        return;
    };
    if !binding.is_signed_in() || tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let Some(binding) = binding.strong() else {
        return;
    };
    let key = format!("titles:{}:{language}:{first}", binding.classes.generation());
    if binding.classes.claim(vec![key.clone()]).is_empty() {
        return;
    }
    let language = language.to_owned();
    tokio::spawn(async move {
        match read_titles(&binding, &language, &artists).await {
            Ok(read) => {
                if read.failed {
                    binding.classes.release(&[key]);
                }
                if read.changed {
                    binding.content_changed();
                }
            }
            Err(error) => {
                log::warn!(
                    "The section titles of Apple Music could not be read, so pages keep what they show: {error:#}"
                );
                binding.classes.release(&[key]);
            }
        }
    });
}

/// What a read of the section titles did.
struct TitlesRead {
    /// Whether what pages show changed.
    changed: bool,
    /// Whether the page of one of the artists could not be read, for anything but Apple not
    /// having the artist.
    failed: bool,
}

/// Reads and keeps the storefront's languages and the section titles in `language`, or else in
/// the storefront's default language.
async fn read_titles(
    binding: &AppleMusicBinding,
    language: &str,
    artists: &[String],
) -> Result<TitlesRead> {
    let generation = binding.classes.generation();
    let mut failed = false;
    let storefront = binding.storefront().await?;
    let languages = binding.storefront_languages().await?;
    let tag = title_tag(language, &languages).map(str::to_owned);
    let mut titles = BTreeMap::new();
    if let Some(tag) = &tag {
        let kept = binding.classes.get(binding).await;
        let have = kept
            .section_titles
            .get(&storefront)
            .and_then(|titles| titles.get(tag))
            .cloned()
            .unwrap_or_default();
        drop(kept);
        for artist in artists.iter().take(TITLE_ARTISTS) {
            let wanted = LIBRARY_SECTIONS.map(ReleaseSection::syntax);
            if wanted
                .iter()
                .all(|section| have.contains_key(*section) || titles.contains_key(*section))
            {
                break;
            }
            match binding
                .read_catalog_artist_views(artist, &view_names(), Some(tag))
                .await
            {
                Ok(page) => {
                    for (section, title) in view_titles(&page) {
                        titles.entry(section).or_insert(title);
                    }
                }
                Err(error) if denies_the_account(&error) => return Err(error),
                Err(error) if is_not_found(&error) => {}
                Err(error) => {
                    log::warn!("The section titles of Apple Music could not be read: {error:#}");
                    failed = true;
                }
            }
        }
    }
    let mut changed = false;
    let kept = binding
        .classes
        .change(binding, generation, |classes| {
            let before = classes.clone();
            classes.storefront = Some(storefront.clone());
            classes
                .languages
                .insert(storefront.clone(), Vec::clone(&languages));
            if let Some(tag) = tag
                && !titles.is_empty()
            {
                classes
                    .section_titles
                    .entry(storefront)
                    .or_default()
                    .entry(tag)
                    .or_default()
                    .extend(titles);
            }
            changed = classes.shown_differs(&before);
        })
        .await;
    Ok(TitlesRead {
        changed: kept && changed,
        failed,
    })
}

/// Reads the release views of the catalog artist `artist` in the background, each to its end,
/// and keeps the section each of them lists its releases in, unless the device is offline. The
/// lists reload once if that moved one of the albums `held` (catalog ids) to another section.
/// A read under way is not started again, and a failed one is only logged, so the next page
/// opened tries again.
fn views_in_background(binding: &AppleMusicBinding, artist: &str, held: Vec<String>) {
    if !binding.is_signed_in()
        || !binding.is_online()
        || tokio::runtime::Handle::try_current().is_err()
    {
        return;
    }
    let Some(binding) = binding.strong() else {
        return;
    };
    let key = format!("views:{artist}");
    if binding.classes.claim(vec![key.clone()]).is_empty() {
        return;
    }
    let artist = artist.to_owned();
    tokio::spawn(async move {
        match read_views(&binding, &artist, &held).await {
            Ok(true) => binding.content_changed(),
            Ok(false) => {}
            Err(error) => log::warn!(
                "The views of an Apple Music artist could not be read, so its page keeps what it shows: {error:#}"
            ),
        }
        binding.classes.release(&[key]);
    });
}

/// Reads the views of the catalog artist `artist` and keeps the section of each release they
/// list: the first section of the page whose view lists it, should several. Whether that moved
/// one of the albums `held` to another section. Views kept young meanwhile are not read again.
async fn read_views(binding: &AppleMusicBinding, artist: &str, held: &[String]) -> Result<bool> {
    let started = Instant::now();
    let generation = binding.classes.generation();
    let now = binding.epoch_seconds();
    if binding
        .classes
        .get(binding)
        .await
        .views_of(artist)
        .is_some_and(|views| views.is_fresh(now))
    {
        return Ok(false);
    }
    let storefront = binding.storefront().await?;
    let reads = reads_of(binding, artist, true, None).await?;
    let mut sections = HashMap::new();
    for section in LIBRARY_SECTIONS {
        let Some(view) = view_of(section) else {
            continue;
        };
        for release in reads.all_releases(binding, view).await? {
            sections
                .entry(release.id)
                .or_insert_with(|| section.syntax().to_owned());
        }
    }
    let count = sections.len();
    let views = ArtistViews {
        sections,
        read: binding.epoch_seconds(),
    };
    let mut moved = false;
    let kept = binding
        .classes
        .change(binding, generation, |classes| {
            let before: Vec<ReleaseSection> = held
                .iter()
                .map(|id| placed(classes, artist, Some(id)))
                .collect();
            classes.storefront = Some(storefront.clone());
            classes.keep_views(&storefront, artist, views);
            moved = held
                .iter()
                .zip(before)
                .any(|(id, was)| placed(classes, artist, Some(id)) != was);
        })
        .await;
    log::debug!(
        "The views of the Apple Music artist {artist} listed {count} releases in {:?}; its page {}",
        started.elapsed(),
        if moved { "reloads" } else { "stays" }
    );
    Ok(kept && moved)
}

/// Reads the catalog facts of the albums `ids` that lack them or have old ones, in the
/// background, one request for every 100 albums. The lists reload once if that changed what
/// pages show. What is under way already is not asked again, and a failed read is only logged,
/// so the next list or page that finds the albums without facts tries again.
pub(crate) fn classify_in_background(binding: &AppleMusicBinding, ids: Vec<String>) {
    if ids.is_empty() || !binding.is_signed_in() || tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let Some(binding) = binding.strong() else {
        return;
    };
    tokio::spawn(async move {
        let kept = binding.classes.get(&binding).await;
        let ids = binding
            .classes
            .claim(kept.needing(&ids, binding.epoch_seconds()));
        drop(kept);
        if ids.is_empty() {
            return;
        }
        match classify(&binding, &ids).await {
            Ok(true) => binding.content_changed(),
            Ok(false) => {}
            Err(error) => log::warn!(
                "Apple Music albums could not be classified, so pages keep what they show: {error:#}"
            ),
        }
        binding.classes.release(&ids);
    });
}

/// Reads the facts of the albums and keeps them. Whether what pages show changed.
async fn classify(binding: &AppleMusicBinding, ids: &[String]) -> Result<bool> {
    let started = Instant::now();
    let generation = binding.classes.generation();
    let albums = binding.read_catalog_albums_with_artists(ids).await?;
    let storefront = binding.storefront().await?;
    let read = binding.epoch_seconds();
    let mut known: HashMap<String, AlbumFacts> = albums
        .into_iter()
        .map(|album| {
            let artists = album
                .relationships
                .and_then(|relationships| relationships.artists)
                .map(|artists| artists.data.into_iter().map(|artist| artist.id).collect())
                .unwrap_or_default();
            let attributes = album.attributes.unwrap_or_default();
            (
                album.id,
                AlbumFacts {
                    name: attributes.name.unwrap_or_default(),
                    single: attributes.is_single,
                    compilation: attributes.is_compilation,
                    artists,
                    read,
                },
            )
        })
        .collect();
    // An album the catalog does not know is kept as it is, so it is not asked for again.
    for id in ids {
        known.entry(id.clone()).or_insert(AlbumFacts {
            name: String::new(),
            single: None,
            compilation: None,
            artists: Vec::new(),
            read,
        });
    }
    let count = known.len();
    let mut changed = false;
    let kept = binding
        .classes
        .change(binding, generation, |classes| {
            let before = classes.clone();
            classes.storefront = Some(storefront.clone());
            for (id, facts) in known {
                classes.albums.insert(id, facts);
            }
            changed = classes.shown_differs(&before);
        })
        .await;
    log::debug!(
        "{count} Apple Music albums were classified in {:?}; pages {}",
        started.elapsed(),
        if changed { "reload" } else { "stay" }
    );
    Ok(kept && changed)
}

/// The library's songs of the releases of one section, release after release, each by disc and
/// number.
async fn library_section_songs(
    binding: &AppleMusicBinding,
    artist: &NamedArtist,
    section: ReleaseSection,
) -> Result<Vec<Listed>> {
    let releases = library_sections(binding, artist, false, "")
        .await?
        .into_iter()
        .find(|each| each.section == section)
        .map(|each| each.releases)
        .unwrap_or_default();
    let list = binding.songs().await?;
    Ok(releases
        .iter()
        .flat_map(|kept| album_songs_from_list(&list, &kept.album.id))
        .collect())
}

/// The songs of the library by the artist, the newest album first, then by disc and number.
async fn library_artist_songs(
    binding: &AppleMusicBinding,
    artist: &NamedArtist,
) -> Result<Vec<Listed>> {
    let list = binding.songs().await?;
    Ok(artist_songs_by_album(&list, artist, true))
}

/// The tracks of the artist's page, from the library alone: the songs the user liked in Rune
/// first, then the others, each group the newest album first, then by disc and number. Rune
/// keeps no play statistics of Apple Music songs, and no top songs are read.
async fn library_tracks(binding: &AppleMusicBinding, artist: &NamedArtist) -> Result<Vec<Listed>> {
    let songs = library_artist_songs(binding, artist).await?;
    let mut liked = Vec::new();
    let mut rest = Vec::new();
    for song in songs {
        // A likes file that cannot be read only leaves the order by album.
        let is_liked = binding
            .rune_likes
            .is_liked(&binding.like_keys(&song.0))
            .await
            .unwrap_or(false);
        if is_liked {
            liked.push(song);
        } else {
            rest.push(song);
        }
    }
    liked.extend(rest);
    Ok(liked)
}

/// The covers of `urls` that are cached, the others starting to download.
async fn cached_covers(binding: &AppleMusicBinding, urls: Vec<String>) -> Vec<String> {
    let mut covers = Vec::with_capacity(urls.len());
    for url in urls {
        covers.extend(binding.cover_path(Some(&url)).await);
    }
    covers
}

/// The tag of the storefront's languages the page is titled in: the one that answers in the
/// client's `language`, or else the storefront's default (see `title_tag`), which asks Apple
/// once per storefront. `None` when Apple could not tell, and the page is then named by Rune.
async fn catalog_tag(binding: &AppleMusicBinding, language: &str) -> Option<String> {
    match binding.storefront_languages().await {
        Ok(languages) => title_tag(language, &languages).map(str::to_owned),
        Err(error) => {
            log::warn!("The languages of the Apple Music storefront could not be read: {error:#}");
            None
        }
    }
}

/// The page of an artist: from Apple's catalog for a catalog artist, its sections the views
/// that hold releases, each titled by Apple in the client's language, or else in the
/// storefront's default language, and its tracks the top songs; from the library's kept lists for an entry of the library's
/// artist list (see `library_sections` and `library_tracks`).
pub(crate) async fn fetch_artist_page_request(
    binding: &AppleMusicBinding,
    request: &FetchArtistPageRequest,
) -> Result<Option<FetchArtistPageResponse>> {
    let mut sections = Vec::new();
    let has_tracks = match identity(binding, request.artist_id).await? {
        Identity::Catalog(id) => {
            let tag = catalog_tag(binding, &request.language).await;
            let reads = reads_of(binding, &id, true, tag.as_deref()).await?;
            if let Some(tag) = &tag
                && let Err(error) = reads.learn_titles(binding, &id, tag).await
            {
                // The page is worth showing without Apple's titles.
                if denies_the_account(&error) {
                    return Err(error);
                }
                log::warn!(
                    "The section titles of an Apple Music artist could not be read: {error:#}"
                );
            }
            let generation = binding.generation();
            for section in SECTIONS {
                let (count, covers) = reads.known(section).await;
                if count == 0 {
                    continue;
                }
                // The latest release opens as the album it is, numbered as every catalog album
                // is.
                let album = match opens_of(section) {
                    ArtistSectionOpens::Album => {
                        let Some(release) = reads.release(binding, section, 0).await? else {
                            continue;
                        };
                        Some(AlbumRef {
                            id: number_catalog_album(binding, generation, &release)?,
                            name: catalog_album_name(&release).to_owned(),
                        })
                    }
                    _ => None,
                };
                sections.push(ArtistSection {
                    kind: section.into(),
                    title: reads.title_of(section, tag.as_deref()),
                    opens: opens_of(section),
                    album,
                    queries: vec![section_query(request.artist_id, section)],
                    covers: cached_covers(binding, covers).await,
                    count: i32::try_from(count).unwrap_or(i32::MAX),
                });
            }
            !reads.top_songs.lock().await.songs.is_empty()
        }
        Identity::Library(artist) => {
            for LibrarySection {
                section,
                title,
                releases,
            } in library_sections(binding, &artist, true, &request.language).await?
            {
                let covers = releases
                    .iter()
                    .filter_map(|kept| kept.cover.clone())
                    .take(SECTION_COVERS)
                    .collect();
                sections.push(ArtistSection {
                    kind: section.into(),
                    title,
                    opens: opens_of(section),
                    album: None,
                    queries: vec![section_query(request.artist_id, section)],
                    covers: cached_covers(binding, covers).await,
                    count: i32::try_from(releases.len()).unwrap_or(i32::MAX),
                });
            }
            // Tracks are the library's songs of the artist in one order or another, so the
            // page has them when the library does, with no catalog request.
            !library_artist_songs(binding, &artist).await?.is_empty()
        }
    };
    Ok(Some(FetchArtistPageResponse {
        call_id: None,
        artist_id: request.artist_id,
        sections,
        tracks: if has_tracks {
            vec![tracks_query(request.artist_id)]
        } else {
            Vec::new()
        },
    }))
}

/// A page of the albums of a section of an artist's page that opens as albums, in the groups its
/// albums page shows (see `section_groups`), by the year of Apple's release date: the catalog's
/// releases of the section's view, which is read to its end for them, numbered as every catalog
/// album is, or the library's kept releases, numbered as the library's own albums are.
pub(crate) async fn fetch_artist_section_albums_request(
    binding: &AppleMusicBinding,
    request: &FetchArtistSectionAlbumsRequest,
) -> Result<Option<FetchArtistSectionAlbumsResponse>> {
    let (artist_id, section) = named_section(&request.queries)?;
    let (cursor, count) = page_bounds(request.cursor, request.page_size)?;
    let generation = binding.generation();
    let mut groups = Vec::new();
    match identity(binding, artist_id).await? {
        Identity::Catalog(id) => {
            let reads = reads_of(binding, &id, false, None).await?;
            let releases = reads.section_releases(binding, section).await?;
            let page = section_groups(
                section,
                releases
                    .iter()
                    .map(|release| (release, catalog_album_year(release))),
                cursor,
                count,
            );
            for group in page {
                let album_ids = group
                    .items
                    .into_iter()
                    .map(|album| number_catalog_album(binding, generation, album))
                    .collect::<Result<_>>()?;
                groups.push(ArtistSectionAlbumGroup {
                    group_title: group.title,
                    album_ids,
                });
            }
        }
        Identity::Library(artist) => {
            let releases = library_sections(binding, &artist, false, "")
                .await?
                .into_iter()
                .find(|each| each.section == section)
                .map(|each| each.releases)
                .unwrap_or_default();
            let page = section_groups(
                section,
                releases
                    .iter()
                    .map(|kept| (&kept.album, album_year(&kept.album))),
                cursor,
                count,
            );
            for group in page {
                let mut album_ids = Vec::with_capacity(group.items.len());
                for album in group.items {
                    let id = binding.intern(AppleIdKind::Album, &album.id)?;
                    binding.remember_collection(
                        generation,
                        AppleIdKind::Album,
                        id,
                        album_name(album),
                        album_artwork(album),
                    );
                    album_ids.push(id);
                }
                groups.push(ArtistSectionAlbumGroup {
                    group_title: group.title,
                    album_ids,
                });
            }
        }
    }
    Ok(Some(FetchArtistSectionAlbumsResponse {
        call_id: None,
        groups,
    }))
}

/// The songs of a catalog release as a section lists them, as tracks of the catalog.
fn release_tracks(songs: &[Song]) -> impl Iterator<Item = Listed> + '_ {
    songs
        .iter()
        .map(|song| (catalog_track(&song.id), AppleSong::Catalog(song.clone())))
}

/// The songs of the section `section` from `cursor`, at most `count`: the songs of its
/// releases one release after another. Pages of the view are read only as far as the page
/// reaches, and the songs of a release only when it does.
async fn section_page(
    binding: &AppleMusicBinding,
    reads: &ArtistReads,
    section: ReleaseSection,
    cursor: usize,
    count: usize,
) -> Result<Vec<Listed>> {
    let end = cursor.saturating_add(count);
    let mut page = Vec::new();
    let mut passed = 0;
    let mut index = 0;
    while passed < end {
        // The releases this page is likely to reach, read ahead in one request.
        let batch = (end - passed)
            .div_ceil(SONGS_PER_RELEASE)
            .clamp(1, RELEASE_BATCH);
        let mut releases = Vec::with_capacity(batch);
        while releases.len() < batch {
            let Some(release) = reads
                .release(binding, section, index + releases.len())
                .await?
            else {
                break;
            };
            releases.push(release);
        }
        if releases.is_empty() {
            break;
        }
        read_ahead(binding, reads, &releases, passed, cursor, end).await?;
        for release in releases {
            if passed >= end {
                break;
            }
            index += 1;
            if let Some(songs) = reads.song_count(&release.id)
                && passed + songs <= cursor
            {
                passed += songs;
                continue;
            }
            let songs = match reads.kept_release_songs(&release.id) {
                Some(songs) => songs,
                None => {
                    let album = binding.intern(
                        AppleIdKind::Album,
                        &format!("{CATALOG_PREFIX}{}", release.id),
                    )?;
                    match binding
                        .catalog_collection_songs(
                            CatalogCollection::Album(album),
                            &release.id,
                            false,
                        )
                        .await
                    {
                        Ok(songs) => songs,
                        // A release Apple no longer has holds no songs.
                        Err(error) if is_not_found(&error) => {
                            log::warn!(
                                "An Apple Music release of an artist's page is gone, so its songs are left out: {error:#}"
                            );
                            reads.keep_song_count(&release.id, 0);
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                }
            };
            reads.keep_song_count(&release.id, songs.len());
            for song in release_tracks(&songs) {
                if passed >= cursor && passed < end {
                    page.push(song);
                }
                passed += 1;
            }
        }
    }
    Ok(page)
}

/// Reads the songs of the `releases` a page from `cursor` to `end` needs and that are not kept
/// yet, in one request for all of them, so a long page asks Apple once for many releases and
/// not once for each. `passed` is how many songs come before the first of them. One release,
/// or none, is left to be read as a collection is, and a release the request does not return
/// is read then too.
async fn read_ahead(
    binding: &AppleMusicBinding,
    reads: &ArtistReads,
    releases: &[Album],
    mut passed: usize,
    cursor: usize,
    end: usize,
) -> Result<()> {
    let mut wanted = Vec::new();
    for release in releases {
        if passed >= end {
            break;
        }
        if let Some(songs) = reads.song_count(&release.id) {
            passed += songs;
            if passed <= cursor {
                continue;
            }
        }
        if reads.kept_release_songs(&release.id).is_some() {
            continue;
        }
        let album = binding.intern(
            AppleIdKind::Album,
            &format!("{CATALOG_PREFIX}{}", release.id),
        )?;
        if binding
            .kept_catalog_songs(CatalogCollection::Album(album))
            .is_some()
        {
            continue;
        }
        wanted.push(release.id.clone());
    }
    if wanted.len() < 2 {
        return Ok(());
    }
    let read = match binding.read_catalog_albums_with_tracks(&wanted).await {
        Ok(read) => read,
        // Each release is read alone then, and the gone ones are left out as they are there.
        Err(AppleError::NotFound) => return Ok(()),
        Err(error) => return Err(binding.request_error(error, "Apple Music album")),
    };
    for (id, songs) in read {
        reads.keep_release_songs(id, Arc::new(songs));
    }
    Ok(())
}

/// The artist's top songs from `cursor`, at most `count`, read from Apple a page at a time as
/// far as the page reaches. A song the library holds is the library's copy.
async fn top_songs_page(
    binding: &AppleMusicBinding,
    id: &str,
    reads: &ArtistReads,
    cursor: usize,
    count: usize,
) -> Result<Vec<Listed>> {
    let end = cursor.saturating_add(count);
    let songs: Vec<Song> = {
        let mut top = reads.top_songs.lock().await;
        // The cursor stays until the page is read, so a failed read leaves the list to be read
        // again, not ended.
        while top.songs.len() < end
            && let Some(next) = top.next.clone()
        {
            let page = binding.read_catalog_view_page(&next).await?;
            top.songs.extend(page.songs());
            top.next = page.next;
            if top.next.is_none() {
                note_top_songs_total(binding, id, top.songs.len());
            }
        }
        top.songs.iter().skip(cursor).take(count).cloned().collect()
    };
    let list = binding.known_songs();
    Ok(songs
        .into_iter()
        .map(|song| {
            let track = catalog_track(&song.id);
            let copy = list.as_ref().and_then(|list| {
                let twin = list.library_twin(&track)?;
                Some((twin.to_owned(), AppleSong::Library(list.get(twin)?.clone())))
            });
            copy.unwrap_or((track, AppleSong::Catalog(song)))
        })
        .collect())
}

/// A page of one list of the page of the artist the UI numbered `artist`: its songs from
/// `cursor`, at most `count`.
pub(super) async fn listing_page(
    binding: &AppleMusicBinding,
    artist: i32,
    listing: ArtistListing,
    cursor: usize,
    count: usize,
) -> Result<Vec<Listed>> {
    let page = |songs: Vec<Listed>| -> Vec<Listed> {
        songs.into_iter().skip(cursor).take(count).collect()
    };
    Ok(match (identity(binding, artist).await?, listing) {
        (Identity::Catalog(id), ArtistListing::Section(section)) => {
            let reads = reads_of(binding, &id, false, None).await?;
            section_page(binding, &reads, section, cursor, count).await?
        }
        (Identity::Catalog(id), ArtistListing::Tracks) => {
            let reads = reads_of(binding, &id, false, None).await?;
            top_songs_page(binding, &id, &reads, cursor, count).await?
        }
        (Identity::Library(artist), ArtistListing::Section(section)) => {
            page(library_section_songs(binding, &artist, section).await?)
        }
        (Identity::Library(artist), ArtistListing::Tracks) => {
            page(library_tracks(binding, &artist).await?)
        }
    })
}

#[cfg(test)]
mod tests {
    use apple_music::models::{LibraryAlbum, LibraryArtist};
    use serde_json::json;

    use super::albums_of_artist;
    use crate::apple_music_artists::tests::catalog;
    use crate::apple_music_artists::{NamedArtists, SongArtists};

    fn library_artist(id: &str, catalog_id: &str) -> LibraryArtist {
        serde_json::from_value(json!({
            "id": id,
            "type": "library-artists",
            "attributes": {"name": "Atlas"},
            "relationships": {"catalog": {"data": [catalog(catalog_id, "Atlas")]}},
        }))
        .unwrap()
    }

    /// An album credited "Atlas", of the library artist `artist` when it names one.
    fn album(id: &str, artist: Option<&str>) -> LibraryAlbum {
        let mut album = json!({
            "id": id,
            "type": "library-albums",
            "attributes": {"name": id, "artistName": "Atlas"},
        });
        if let Some(artist) = artist {
            album["relationships"] =
                json!({"artists": {"data": [{"id": artist, "type": "library-artists"}]}});
        }
        serde_json::from_value(album).unwrap()
    }

    #[test]
    fn an_album_of_another_artist_of_the_same_name_is_not_on_the_page() {
        // Two catalog artists are called "Atlas", each the catalog artist of a library artist.
        let named = NamedArtists::new(
            &[library_artist("r.1", "111"), library_artist("r.2", "222")],
            &[],
            &SongArtists::default(),
        );
        let albums = [
            album("a.first", Some("r.1")),
            album("a.second", Some("r.2")),
            // By no library artist, or by one that is no artist of the list: the name decides.
            album("a.loose", None),
            album("a.orphan", Some("r.gone")),
        ];
        let ids = |remote: &str| -> Vec<String> {
            let artist = named.get(remote).expect("an artist of the list");
            albums_of_artist(&albums, artist, &named)
                .into_iter()
                .map(|album| album.id)
                .collect()
        };
        assert_eq!(ids("artist:111"), ["a.first", "a.loose", "a.orphan"]);
        assert_eq!(ids("artist:222"), ["a.loose", "a.orphan", "a.second"]);
    }
}
