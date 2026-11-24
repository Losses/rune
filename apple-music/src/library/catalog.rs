//! Apple's catalog beside the library: searched and browsed, and the user's recommendations.

use std::collections::{HashMap, HashSet};
use std::result;
use std::sync;
use std::sync::Arc;
use std::time::Duration;

use futures_util::TryStreamExt;
use futures_util::future::{join, try_join};
use tokio::time;
use tokio::time::error::Elapsed;

use super::cache::ArtistNames;
use super::types::{
    AlbumItem, ArtistItem, CatalogAlbumDetail, CatalogArtistDetail, CatalogArtistPage,
    CatalogPlaylistDetail, ItemId, RecommendationGroup, ReleasePage, ReleaseView, SearchResults,
    SearchScope, SongList, SongPage, Track,
};
use super::{AppleLibrary, Failure, LibraryError, Missing, Outcome, RECOMMENDATIONS, map};
use crate::error::Error;
use crate::models::{
    self, CatalogSearchResults, CatalogSearchType, LibraryAlbum, LibraryArtist, LibraryPlaylist,
    LibrarySearchResults, LibrarySearchType, LibrarySong, Song,
};
use crate::{CATALOG_SEARCH_LIMIT, LIBRARY_SEARCH_LIMIT};

/// Recommendation groups [`AppleLibrary::recommendations`] is usually asked for.
pub const DEFAULT_RECOMMENDATIONS: usize = 10;

/// The most results of each kind a search lists from the library, and from the catalog.
const SEARCH_LIMIT: usize = 25;
const _: () = assert!(CATALOG_SEARCH_LIMIT == 25 && LIBRARY_SEARCH_LIMIT == 25);

/// How long a search of the library and the catalog waits for both, from its start. Each part
/// may take a few seconds on a slow network, and a part answered within this time is listed;
/// one that fails or answers later is left out, its retries stop, and the search answers with
/// the other. It stays well within the time a caller waits for a request.
pub const SEARCH_WAIT: Duration = Duration::from_secs(16);

/// The name of the view of an artist's most popular songs.
const TOP_SONGS: &str = "top-songs";

/// A page of a release view as the library's types, not yet marked.
fn release_page(view: ReleaseView, page: &models::View) -> ReleasePage {
    ReleasePage {
        title: page
            .attributes
            .as_ref()
            .and_then(|attributes| attributes.title.clone())
            .filter(|title| !title.trim().is_empty()),
        releases: page
            .albums()
            .iter()
            .map(|album| map::artist_release(album, view == ReleaseView::Singles))
            .collect(),
        next: page.next.clone(),
    }
}

/// A page of songs as the library's types, not yet marked.
fn song_page(page: &models::View) -> SongPage {
    SongPage {
        songs: page.songs().iter().filter_map(map::catalog_track).collect(),
        next: page.next.clone(),
    }
}

fn mark_releases(marks: &Marks, page: &mut ReleasePage) {
    for release in &mut page.releases {
        release.album.in_library = marks.album(&release.album);
    }
}

fn mark_songs(marks: &Marks, page: &mut SongPage) {
    for song in &mut page.songs {
        song.in_library = catalog_song_id(song).is_some_and(|id| marks.song(id));
    }
}

/// What the library knows to be its own, to tell catalog items apart from it. Albums match
/// by catalog id when the library names one, and by name and artist only when it names
/// none; artists match by name, as library artists name no catalog id.
///
/// Two readings are made: the library's own copies (what a catalog item that duplicates
/// is left out for), and what marks a catalog item as in the library. They differ for artists
/// only: a library artist is a copy of the catalog artist of its whole name only, but every
/// artist its name credits is in the library.
#[derive(Default)]
struct Marks {
    songs: Option<Arc<SongList>>,
    found_songs: HashSet<String>,
    /// Catalog ids of the library's albums.
    album_ids: HashSet<String>,
    /// Names of library albums that name no catalog id.
    album_names: HashSet<(String, String)>,
    /// Whole names of the library artists the search found.
    found_artists: HashSet<String>,
    /// Each artist the names of the library artists the search found credit.
    found_credited: HashSet<String>,
    artists: Option<Arc<ArtistNames>>,
}

impl Marks {
    fn add_found(&mut self, found: &LibraryFound, split: fn(&str) -> Vec<String>) {
        self.found_songs.extend(
            found
                .songs
                .iter()
                .filter_map(|song| song.store_id())
                .map(str::to_owned),
        );
        for album in &found.albums {
            let attributes = album.attributes.as_ref();
            self.add_library_album(
                album.catalog_id(),
                attributes
                    .and_then(|attributes| attributes.name.as_deref())
                    .unwrap_or_default(),
                attributes.and_then(|attributes| attributes.artist_name.as_deref()),
            );
        }
        for artist in &found.artists {
            let name = artist
                .attributes
                .as_ref()
                .and_then(|attributes| attributes.name.as_deref())
                .unwrap_or_default();
            self.found_artists.extend(name_of(name));
            self.found_credited.extend(credited(name, split));
        }
    }

    fn add_library_album(&mut self, catalog_id: Option<&str>, name: &str, artist: Option<&str>) {
        match catalog_id {
            Some(id) => {
                self.album_ids.insert(id.to_owned());
            }
            None => self.album_names.extend(album_key(name, artist)),
        }
    }

    fn song(&self, catalog_id: &str) -> bool {
        self.found_songs.contains(catalog_id)
            || self
                .songs
                .as_ref()
                .is_some_and(|list| list.has_library_copy(catalog_id))
    }

    fn album(&self, item: &AlbumItem) -> bool {
        item.id
            .catalog_id()
            .is_some_and(|id| self.album_ids.contains(id))
            || self.album_named(item)
    }

    fn album_named(&self, item: &AlbumItem) -> bool {
        album_key(&item.name, item.artist.as_deref())
            .is_some_and(|key| self.album_names.contains(&key))
    }

    fn artist_copy(&self, name: &str) -> bool {
        name_of(name).is_some_and(|key| {
            self.found_artists.contains(&key)
                || self
                    .artists
                    .as_ref()
                    .is_some_and(|names| names.whole.contains(&key))
        })
    }

    fn artist(&self, name: &str) -> bool {
        self.artist_copy(name)
            || name_of(name).is_some_and(|key| {
                self.found_credited.contains(&key)
                    || self
                        .artists
                        .as_ref()
                        .is_some_and(|names| names.credited.contains(&key))
            })
    }
}

/// A name as it is compared, `None` when it is empty and so names nothing.
fn name_of(name: &str) -> Option<String> {
    let key = map::name_key(name);
    (!key.is_empty()).then_some(key)
}

/// The name keys of the artists `name` credits, split by `split`.
fn credited(name: &str, split: fn(&str) -> Vec<String>) -> impl Iterator<Item = String> {
    split(name).into_iter().filter_map(|name| name_of(&name))
}

fn album_key(name: &str, artist: Option<&str>) -> Option<(String, String)> {
    Some((name_of(name)?, map::name_key(artist.unwrap_or_default())))
}

/// What a library search found, before mapping.
#[derive(Default)]
struct LibraryFound {
    songs: Vec<LibrarySong>,
    albums: Vec<LibraryAlbum>,
    artists: Vec<LibraryArtist>,
    playlists: Vec<LibraryPlaylist>,
}

/// What a catalog search found, before mapping.
#[derive(Default)]
pub(super) struct CatalogFound {
    songs: Vec<Song>,
    albums: Vec<models::Album>,
    artists: Vec<models::Artist>,
}

/// The term Apple is asked to search for. Apple's search has no wildcards, so a trailing
/// `*`, as a search box may append, is left out, and so are the quotes of an empty search.
fn search_term(query: &str) -> &str {
    let term = query.trim().trim_end_matches('*').trim_end();
    if term == "\"\"" { "" } else { term }
}

/// The key a catalog search for `term` is kept under: what Apple matches the same way, as
/// letter case and runs of spaces are not told apart.
fn search_key(term: &str) -> String {
    term.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Whether `failure` is about the sign-in, which the caller has to learn of, rather than about
/// this one request.
fn about_the_sign_in(failure: &Failure) -> bool {
    match failure.root() {
        Failure::Apple(error) => super::rejects_tokens(error),
        Failure::Library(error) => matches!(error, LibraryError::SignedOut),
        Failure::Shared(_) => false,
    }
}

/// What a part of a search gave: what it found, or why it found nothing.
enum Part<T> {
    Found(T),
    Failed(Failure),
    Late,
}

impl<T> Part<T> {
    fn of(answer: Result<Outcome<T>, Elapsed>) -> Self {
        match answer {
            Ok(Ok(found)) => Self::Found(found),
            Ok(Err(failure)) => Self::Failed(failure),
            Err(_) => Self::Late,
        }
    }

    fn failure(&self) -> Option<&Failure> {
        match self {
            Self::Failed(failure) => Some(failure),
            _ => None,
        }
    }
}

/// The catalog id of `item`; a library item names nothing in the catalog.
fn catalog_of(item: &ItemId, what: Missing) -> result::Result<&str, LibraryError> {
    item.catalog_id().ok_or(LibraryError::NotFound(what))
}

impl AppleLibrary {
    /// What the library knows to be its own, read from what is kept, and from `found`.
    /// Nothing is read from Apple for it.
    fn library_marks(&self, found: Option<&LibraryFound>) -> Marks {
        let mut marks = Marks {
            songs: self.caches.songs.peek(),
            artists: self.caches.artists.peek(),
            ..Marks::default()
        };
        if let Some(list) = self.caches.albums.peek() {
            for album in list.iter() {
                marks.add_library_album(
                    album.catalog_id.as_deref(),
                    &album.name,
                    album.artist.as_deref(),
                );
            }
        }
        if let Some(found) = found {
            marks.add_found(found, self.split_artists);
        }
        marks
    }

    /// The names of the library's artists, whole and split into the artists they credit,
    /// kept for ten minutes. Best effort: `None` when they cannot be read, so artists matched
    /// against them alone read as not in the library.
    async fn artist_names(&self) -> Option<Arc<ArtistNames>> {
        self.caches
            .artists
            .get(self.clock.now(), super::cache::TTL, || async {
                let artists: Vec<LibraryArtist> =
                    self.client.library_artists().try_collect().await?;
                let mut names = ArtistNames::default();
                for artist in &artists {
                    let name = map::artist(artist).name;
                    names.whole.extend(name_of(&name));
                    names.credited.extend(credited(&name, self.split_artists));
                }
                Ok::<_, Failure>(names)
            })
            .await
            .ok()
    }

    // Search ---------------------------------------------------------------

    /// Searches for songs, albums, artists and playlists. `Library` lists the library's
    /// matches; `Catalog` lists the catalog's, albums and artists as well as songs, in Apple's
    /// order, and marks which of them the library holds from what is known of the library,
    /// without asking Apple's library search; `All` lists the library's matches first, then
    /// the catalog's that duplicate none of the library's own copies. What the catalog found
    /// is kept for two minutes per term, so a search repeated meanwhile asks the catalog
    /// nothing.
    ///
    /// The library's part and the catalog's part are each capped at 25 of every kind before
    /// they are merged, so many library matches never hide the catalog's. There is no paging
    /// beyond the cap in this version. Catalog playlists are not asked for, though Apple's
    /// search offers them, so the playlists are the library's. With `All`, the search
    /// answers once both parts have, or [`SEARCH_WAIT`] after it started: a part that fails
    /// or has not answered by then is left out and logged, and the other part answers. A
    /// failure about the sign-in still fails the search, and so does a search neither part
    /// answered. With `Catalog` a failing catalog search fails the search. An empty query
    /// finds nothing: [`songs`](Self::songs) lists everything. The catalog is read with the
    /// developer token only.
    ///
    /// With `refresh` the catalog is searched again even when a search for the same term
    /// was kept, and what it finds replaces the kept one.
    pub async fn search_fresh(
        &self,
        query: &str,
        scope: SearchScope,
        refresh: bool,
    ) -> super::Result<SearchResults> {
        // A search asked again after a sign-in changed under it keeps its time.
        let deadline = time::Instant::now() + self.search_wait;
        // Apple's 404 answers a search with nothing and never reaches the caller, so the
        // kind named here only stands in.
        self.signed_in_read(Missing::Song, || {
            self.search_once(query, scope, refresh, deadline)
        })
        .await
    }

    async fn search_once(
        &self,
        query: &str,
        scope: SearchScope,
        refresh: bool,
        deadline: time::Instant,
    ) -> Outcome<SearchResults> {
        let term = search_term(query);
        if term.is_empty() {
            return Ok(SearchResults::default());
        }
        let (library, catalog) = match scope {
            SearchScope::Library => (self.library_search(term).await?, Arc::default()),
            SearchScope::Catalog => (
                LibraryFound::default(),
                self.kept_catalog_search(term, refresh).await?,
            ),
            SearchScope::All => self.search_both(term, refresh, deadline).await?,
        };
        // A catalog search marks against what is known of the library, and waits for no
        // search of the library's own.
        let marks = self.library_marks((scope != SearchScope::Catalog).then_some(&library));

        let mut results = SearchResults::default();
        if scope != SearchScope::Catalog {
            results.songs = library
                .songs
                .iter()
                .filter_map(map::library_track)
                .take(SEARCH_LIMIT)
                .collect();
            results.albums = library
                .albums
                .iter()
                .map(map::library_album_item)
                .take(SEARCH_LIMIT)
                .collect();
            results.artists = library
                .artists
                .iter()
                .map(map::library_artist_item)
                .take(SEARCH_LIMIT)
                .collect();
            results.playlists = library
                .playlists
                .iter()
                .map(map::library_playlist_item)
                .take(SEARCH_LIMIT)
                .collect();
        }
        let dropping = scope == SearchScope::All;
        results.songs.extend(
            catalog
                .songs
                .iter()
                .take(SEARCH_LIMIT)
                .filter(|song| !dropping || !marks.song(&song.id))
                .filter_map(map::catalog_track)
                .map(|mut song| {
                    song.in_library = catalog_song_id(&song).is_some_and(|id| marks.song(id));
                    song
                }),
        );
        results.albums.extend(
            catalog
                .albums
                .iter()
                .take(SEARCH_LIMIT)
                .map(map::catalog_album_item)
                .filter(|album| !dropping || !marks.album(album))
                .map(|mut album| {
                    album.in_library = marks.album(&album);
                    album
                }),
        );
        results.artists.extend(
            catalog
                .artists
                .iter()
                .take(SEARCH_LIMIT)
                .map(map::catalog_artist_item)
                .filter(|artist| !dropping || !marks.artist_copy(&artist.name))
                .map(|mut artist| {
                    artist.in_library = marks.artist(&artist.name);
                    artist
                }),
        );
        Ok(results)
    }

    /// The library's and the catalog's matches, searched at once and waited for until both
    /// have answered or `deadline` has passed. A part that fails or has not answered by then
    /// finds nothing, and is logged with the time the search took. A failure of the library
    /// that is about the sign-in fails the search, and so does a search neither part answered.
    async fn search_both(
        &self,
        term: &str,
        refresh: bool,
        deadline: time::Instant,
    ) -> Outcome<(LibraryFound, Arc<CatalogFound>)> {
        let started = time::Instant::now();
        let (library, catalog) = join(
            time::timeout_at(deadline, self.library_search(term)),
            time::timeout_at(deadline, self.kept_catalog_search(term, refresh)),
        )
        .await;
        let elapsed = started.elapsed();
        let why = |part: Option<&Failure>| match part {
            Some(failure) => format!("it failed: {failure}"),
            None => format!("it did not answer within {:?}", self.search_wait),
        };
        match (Part::of(library), Part::of(catalog)) {
            (Part::Found(library), Part::Found(catalog)) => Ok((library, catalog)),
            (Part::Found(library), catalog) => {
                log::warn!(
                    "An Apple Music search lists the library's matches only, after {elapsed:?}: the catalog search was left out, as {}",
                    why(catalog.failure())
                );
                Ok((library, Arc::default()))
            }
            (Part::Failed(failure), _) if about_the_sign_in(&failure) => Err(failure),
            (library, Part::Found(catalog)) => {
                log::warn!(
                    "An Apple Music search lists the catalog's matches only, after {elapsed:?}: the library search was left out, as {}",
                    why(library.failure())
                );
                Ok((LibraryFound::default(), catalog))
            }
            (Part::Failed(failure), _) => Err(failure),
            (Part::Late, _) => Err(LibraryError::TimedOut.into()),
        }
    }

    /// What the catalog finds for `term`, as kept from a search for it within the last two
    /// minutes of this sign-in, or else searched for and kept. With `refresh` the kept search
    /// is left aside, and what is found replaces it.
    async fn kept_catalog_search(&self, term: &str, refresh: bool) -> Outcome<Arc<CatalogFound>> {
        let key = search_key(term);
        let session = self.caches.session();
        let kept = if refresh {
            None
        } else {
            self.caches.searches.fresh_for(
                &key,
                session,
                self.clock.now(),
                super::cache::SEARCH_TTL,
            )
        };
        if let Some(found) = kept {
            return Ok(found);
        }
        let found = Arc::new(self.catalog_search(term).await?);
        self.caches
            .keep_search(session, &key, self.clock.now(), found.clone());
        Ok(found)
    }

    async fn library_search(&self, term: &str) -> Outcome<LibraryFound> {
        let types = [
            LibrarySearchType::Songs,
            LibrarySearchType::Albums,
            LibrarySearchType::Artists,
            LibrarySearchType::Playlists,
        ];
        let found = match self
            .client
            .search_library(term, &types, LIBRARY_SEARCH_LIMIT, 0)
            .await
        {
            Ok(found) => found,
            Err(Error::NotFound(_)) => LibrarySearchResults::default(),
            Err(error) => return Err(error.into()),
        };
        Ok(LibraryFound {
            songs: found.songs.data,
            albums: found.albums.data,
            artists: found.artists.data,
            playlists: found.playlists.data,
        })
    }

    async fn catalog_search(&self, term: &str) -> Outcome<CatalogFound> {
        let types = [
            CatalogSearchType::Songs,
            CatalogSearchType::Albums,
            CatalogSearchType::Artists,
        ];
        let found = match self
            .client
            .search_catalog(term, &types, CATALOG_SEARCH_LIMIT, 0)
            .await
        {
            Ok(found) => found,
            Err(Error::NotFound(_)) => CatalogSearchResults::default(),
            Err(error) => return Err(error.into()),
        };
        Ok(CatalogFound {
            songs: found.songs.data,
            albums: found.albums.data,
            artists: found.artists.data,
        })
    }

    // Browsing -------------------------------------------------------------

    /// A catalog album with its songs, kept for 30 seconds. Its album and each of its songs
    /// are marked as in the library by their own ids, as a search marks them. A library
    /// item names nothing in the catalog and is not found. Read with the developer token
    /// only.
    pub async fn catalog_album(&self, id: &ItemId) -> super::Result<CatalogAlbumDetail> {
        self.signed_in_read(Missing::Album, || async {
            let id = catalog_of(id, Missing::Album)?;
            let session = self.caches.session();
            let now = self.clock.now();
            let detail = match self.caches.album_details.fresh(id, session, now) {
                Some(detail) => detail,
                None => {
                    let detail = Arc::new(self.read_catalog_album(id).await?);
                    self.caches
                        .keep_detail(session, |c| &c.album_details, id, now, detail.clone());
                    detail
                }
            };
            let marks = self.library_marks(None);
            let mut detail = CatalogAlbumDetail::clone(&detail);
            detail.album.in_library = marks.album(&detail.album);
            for track in &mut detail.tracks {
                track.in_library = catalog_song_id(track).is_some_and(|id| marks.song(id));
            }
            Ok(detail)
        })
        .await
    }

    async fn read_catalog_album(&self, id: &str) -> Outcome<CatalogAlbumDetail> {
        let (album, tracks) = try_join(
            async { Ok(self.client.catalog_album(id).await?) },
            self.catalog_tracks(id, false),
        )
        .await?;
        Ok(album_detail(id, &album, &tracks))
    }

    /// Catalog albums with their songs, as [`catalog_album`](Self::catalog_album) gives each,
    /// read many in one request, in the order of `ids`. A library item, or an album the catalog
    /// does not know, is left out. Nothing is kept: the caller keeps what it needs. Read with
    /// the developer token only.
    pub async fn catalog_albums(&self, ids: &[ItemId]) -> super::Result<Vec<CatalogAlbumDetail>> {
        self.signed_in_read(Missing::Album, || async {
            let wanted: Vec<String> = ids
                .iter()
                .filter_map(|id| id.catalog_id().map(str::to_owned))
                .collect();
            if wanted.is_empty() {
                return Ok(Vec::new());
            }
            let read = self.client.catalog_albums_with_tracks(&wanted).await?;
            let marks = self.library_marks(None);
            let mut found: HashMap<String, CatalogAlbumDetail> = read
                .into_iter()
                .map(|entry| {
                    let relationships = entry.relationships.unwrap_or_default();
                    let album = models::Album {
                        id: entry.id.clone(),
                        kind: entry.kind,
                        href: entry.href,
                        attributes: entry.attributes,
                        relationships: Some(models::AlbumRelationships {
                            artists: relationships.artists,
                        }),
                    };
                    let tracks = relationships.tracks.map(|tracks| tracks.data);
                    let mut detail = album_detail(&entry.id, &album, &tracks.unwrap_or_default());
                    detail.album.in_library = marks.album(&detail.album);
                    for track in &mut detail.tracks {
                        track.in_library = catalog_song_id(track).is_some_and(|id| marks.song(id));
                    }
                    (entry.id, detail)
                })
                .collect();
            Ok(wanted.iter().filter_map(|id| found.remove(id)).collect())
        })
        .await
    }

    /// A catalog playlist with its songs, kept for 30 seconds. A personal mix (`pl.pm-...`)
    /// is read with the user token, any other playlist with the developer token only.
    pub async fn catalog_playlist(&self, id: &ItemId) -> super::Result<CatalogPlaylistDetail> {
        self.catalog_playlist_read(id, false).await
    }

    /// Like [`catalog_playlist`](Self::catalog_playlist), read from Apple whatever is kept, and
    /// kept in place of it: for what follows a playlist's changes, as Apple makes them.
    pub async fn catalog_playlist_afresh(
        &self,
        id: &ItemId,
    ) -> super::Result<CatalogPlaylistDetail> {
        self.catalog_playlist_read(id, true).await
    }

    async fn catalog_playlist_read(
        &self,
        id: &ItemId,
        afresh: bool,
    ) -> super::Result<CatalogPlaylistDetail> {
        self.signed_in_read(Missing::Playlist, || async {
            let id = catalog_of(id, Missing::Playlist)?;
            let session = self.caches.session();
            let now = self.clock.now();
            let kept = (!afresh)
                .then(|| self.caches.playlist_details.fresh(id, session, now))
                .flatten();
            let detail = match kept {
                Some(detail) => detail,
                None => {
                    let (playlist, tracks) = try_join(
                        async { Ok(self.client.catalog_playlist(id).await?) },
                        self.catalog_tracks(id, true),
                    )
                    .await?;
                    let detail = Arc::new(CatalogPlaylistDetail {
                        playlist: map::catalog_playlist_item(&playlist),
                        tracks: tracks.iter().filter_map(map::catalog_track).collect(),
                    });
                    self.caches.keep_detail(
                        session,
                        |c| &c.playlist_details,
                        id,
                        now,
                        detail.clone(),
                    );
                    detail
                }
            };
            let marks = self.library_marks(None);
            let mut detail = CatalogPlaylistDetail::clone(&detail);
            for track in &mut detail.tracks {
                track.in_library = catalog_song_id(track).is_some_and(|id| marks.song(id));
            }
            Ok(detail)
        })
        .await
    }

    /// A catalog artist with their catalog albums, kept for 30 seconds. Whether the artist
    /// is in the library is told by name, as a search tells it.
    pub async fn catalog_artist(&self, id: &ItemId) -> super::Result<CatalogArtistDetail> {
        self.catalog_artist_read(id, false).await
    }

    /// Like [`catalog_artist`](Self::catalog_artist), read from Apple whatever is kept, and kept
    /// in place of it: for what follows an artist's releases, as they come out.
    pub async fn catalog_artist_afresh(&self, id: &ItemId) -> super::Result<CatalogArtistDetail> {
        self.catalog_artist_read(id, true).await
    }

    async fn catalog_artist_read(
        &self,
        id: &ItemId,
        afresh: bool,
    ) -> super::Result<CatalogArtistDetail> {
        self.signed_in_read(Missing::Artist, || async {
            let id = catalog_of(id, Missing::Artist)?;
            let session = self.caches.session();
            let now = self.clock.now();
            let kept = (!afresh)
                .then(|| self.caches.artist_details.fresh(id, session, now))
                .flatten();
            let detail = match kept {
                Some(detail) => detail,
                None => {
                    let (artist, albums) = try_join(
                        self.client.catalog_artist(id),
                        self.client.catalog_artist_albums(id),
                    )
                    .await?;
                    let artist = map::catalog_artist_item(&artist);
                    let albums = albums
                        .iter()
                        .map(|album| {
                            let mut album = map::catalog_album_item(album);
                            album.artist_id.get_or_insert_with(|| artist.id.clone());
                            album.artist.get_or_insert_with(|| artist.name.clone());
                            album
                        })
                        .collect();
                    let detail = Arc::new(CatalogArtistDetail { artist, albums });
                    self.caches.keep_detail(
                        session,
                        |c| &c.artist_details,
                        id,
                        now,
                        detail.clone(),
                    );
                    detail
                }
            };
            self.artist_names().await;
            let marks = self.library_marks(None);
            let mut detail = CatalogArtistDetail::clone(&detail);
            detail.artist.in_library = marks.artist(&detail.artist.name);
            for album in &mut detail.albums {
                album.in_library = marks.album(album);
            }
            Ok(detail)
        })
        .await
    }

    /// A catalog artist's page in one request: the first page of each release view and of the
    /// top songs, kept for 30 seconds. The releases and songs are marked as in the library by
    /// their own ids, as [`catalog_album`](Self::catalog_album) marks them. A view Apple has no
    /// releases in is left out. Later pages of a view are read with
    /// [`catalog_releases_after`](Self::catalog_releases_after) and
    /// [`catalog_top_songs_after`](Self::catalog_top_songs_after). Read with the developer
    /// token only.
    ///
    /// Apple's texts, such as the views' titles, come in `language` (`l=<tag>`) when given. The
    /// tag must be one of [`storefront_languages`](Self::storefront_languages). Kept per artist
    /// and language.
    pub async fn catalog_artist_page_in(
        &self,
        id: &ItemId,
        language: Option<&str>,
    ) -> super::Result<CatalogArtistPage> {
        self.signed_in_read(Missing::Artist, || async {
            let id = catalog_of(id, Missing::Artist)?;
            let key = match language {
                Some(language) => format!("{id}?l={language}"),
                None => id.to_owned(),
            };
            let session = self.caches.session();
            let now = self.clock.now();
            let page = match self.caches.artist_pages.fresh(&key, session, now) {
                Some(page) => page,
                None => {
                    let names: Vec<&str> = ReleaseView::ALL
                        .iter()
                        .map(|view| view.name())
                        .chain([TOP_SONGS])
                        .collect();
                    let artist = self
                        .client
                        .catalog_artist_with_views_in(id, &names, language)
                        .await?;
                    let views = ReleaseView::ALL
                        .iter()
                        .filter_map(|&view| {
                            let page = release_page(view, &artist.view(view.name()));
                            (!page.releases.is_empty()).then_some((view, page))
                        })
                        .collect();
                    let page = Arc::new(CatalogArtistPage {
                        artist: map::catalog_artist_item(&models::Artist {
                            id: artist.id.clone(),
                            kind: "artists".to_owned(),
                            href: None,
                            attributes: artist.attributes.clone(),
                            relationships: None,
                        }),
                        views,
                        top_songs: song_page(&artist.view(TOP_SONGS)),
                    });
                    self.caches
                        .keep_detail(session, |c| &c.artist_pages, &key, now, page.clone());
                    page
                }
            };
            let marks = self.library_marks(None);
            let mut page = CatalogArtistPage::clone(&page);
            for (_, releases) in &mut page.views {
                mark_releases(&marks, releases);
            }
            mark_songs(&marks, &mut page.top_songs);
            Ok(page)
        })
        .await
    }

    /// What the catalog says of albums, by their catalog ids, one request for every
    /// [`CATALOG_ALBUMS_BATCH`](crate::CATALOG_ALBUMS_BATCH) of them. Albums the catalog does
    /// not know are left out. Read with the developer token only.
    pub async fn catalog_album_facts(
        &self,
        ids: &[String],
    ) -> super::Result<Vec<super::CatalogAlbumFacts>> {
        self.signed_in_read(Missing::Album, || async {
            let albums = self.client.catalog_albums_with_artists(ids).await?;
            Ok(albums
                .into_iter()
                .map(|album| {
                    let attributes = album.attributes.unwrap_or_default();
                    super::CatalogAlbumFacts {
                        id: album.id,
                        name: attributes.name.unwrap_or_default(),
                        is_single: attributes.is_single,
                        is_compilation: attributes.is_compilation,
                        artists: album
                            .relationships
                            .and_then(|relationships| relationships.artists)
                            .map(|artists| {
                                artists.data.into_iter().map(|artist| artist.id).collect()
                            })
                            .unwrap_or_default(),
                    }
                })
                .collect())
        })
        .await
    }

    /// The storefront the catalog is read in: the configured one, else the user's, which is
    /// looked up once and kept.
    pub async fn storefront(&self) -> super::Result<String> {
        self.signed_in_read(Missing::Artist, || async {
            Ok(self.client.storefront().await?)
        })
        .await
    }

    /// The language tags the catalog of the configured or the user's storefront has its texts
    /// in, and the one it answers in by default. Read once per storefront and kept. Read with
    /// the developer token only.
    pub async fn storefront_languages(&self) -> super::Result<Arc<super::StorefrontLanguages>> {
        self.signed_in_read(Missing::Artist, || async {
            let id = self.client.storefront().await?;
            let kept = self
                .caches
                .storefront_languages
                .lock()
                .unwrap_or_else(sync::PoisonError::into_inner)
                .get(&id)
                .cloned();
            if let Some(kept) = kept {
                return Ok(kept);
            }
            let attributes = self
                .client
                .catalog_storefront(&id)
                .await?
                .attributes
                .unwrap_or_default();
            let languages = Arc::new(super::StorefrontLanguages {
                default: attributes.default_language_tag,
                supported: attributes.supported_language_tags,
            });
            self.caches
                .storefront_languages
                .lock()
                .unwrap_or_else(sync::PoisonError::into_inner)
                .insert(id, Arc::clone(&languages));
            Ok(languages)
        })
        .await
    }

    /// The page of a release view that `next` names, which is the `next` of an earlier page of
    /// it, handed back as Apple gave it. Marked as the first page is. Not kept.
    pub async fn catalog_releases_after(
        &self,
        view: ReleaseView,
        next: &str,
    ) -> super::Result<ReleasePage> {
        self.signed_in_read(Missing::Artist, || async {
            let page = self.client.catalog_view_page(next).await?;
            let mut page = release_page(view, &page);
            mark_releases(&self.library_marks(None), &mut page);
            Ok(page)
        })
        .await
    }

    /// The page of an artist's top songs that `next` names, which is the `next` of an earlier
    /// page of them, handed back as Apple gave it. Marked as the first page is. Not kept.
    pub async fn catalog_top_songs_after(&self, next: &str) -> super::Result<SongPage> {
        self.signed_in_read(Missing::Artist, || async {
            let page = self.client.catalog_view_page(next).await?;
            let mut page = song_page(&page);
            mark_songs(&self.library_marks(None), &mut page);
            Ok(page)
        })
        .await
    }

    /// The catalog artists of catalog songs, by the songs' catalog ids, each song with its
    /// artists in Apple's order. Songs the catalog does not know are left out. Read with the
    /// developer token only, as many songs at once as Apple takes.
    pub async fn catalog_song_artists(
        &self,
        ids: &[String],
    ) -> super::Result<Vec<(String, Vec<ArtistItem>)>> {
        self.signed_in_read(Missing::Song, || async {
            let songs = self.client.catalog_songs_with_artists(ids).await?;
            Ok(songs
                .into_iter()
                .map(|song| {
                    let artists = song
                        .relationships
                        .and_then(|relationships| relationships.artists)
                        .map(|artists| artists.data)
                        .unwrap_or_default();
                    (
                        song.id,
                        artists.iter().map(map::catalog_artist_item).collect(),
                    )
                })
                .collect())
        })
        .await
    }

    /// The songs of a catalog album or playlist; none when Apple says it has no tracks.
    async fn catalog_tracks(&self, id: &str, playlist: bool) -> Outcome<Vec<Song>> {
        let tracks = if playlist {
            self.client.catalog_playlist_tracks(id).await
        } else {
            self.client.catalog_album_tracks(id).await
        };
        match tracks {
            Ok(tracks) => Ok(tracks),
            Err(Error::NotFound(_)) => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    // Recommendations ------------------------------------------------------

    /// Up to `limit` groups of the user's recommendations (see
    /// [`DEFAULT_RECOMMENDATIONS`]), in Apple's order, with their catalog playlists and
    /// albums. Stations and groups left with nothing else are dropped. The groups are kept
    /// for ten minutes, and served while they are refreshed in the background once old; the
    /// items are marked against the library on every read. Read with the user token.
    pub async fn recommendations(&self, limit: usize) -> super::Result<Vec<RecommendationGroup>> {
        // Apple's 404 is absorbed, so the kind named here only stands in.
        let groups = self.kept_list(&RECOMMENDATIONS, Missing::Playlist).await?;
        let marks = self.library_marks(None);
        Ok(groups
            .iter()
            .take(limit)
            .cloned()
            .map(|mut group| {
                for album in &mut group.albums {
                    album.in_library = album.in_library || marks.album(album);
                }
                group
            })
            .collect())
    }

    pub(super) async fn build_recommendations(&self) -> Outcome<Vec<RecommendationGroup>> {
        let recommendations = match self.client.recommendations().await {
            Ok(recommendations) => recommendations,
            Err(Error::NotFound(_)) => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        Ok(recommendations
            .iter()
            .filter_map(map::recommendation_group)
            .collect())
    }
}

/// A catalog album `id` with its songs `tracks`: each song is on the album, and has the album's
/// artist and cover when it names the same artist and has no cover of its own.
fn album_detail(id: &str, album: &models::Album, tracks: &[Song]) -> CatalogAlbumDetail {
    let album = map::catalog_album_item(album);
    let tracks = tracks
        .iter()
        .filter_map(map::catalog_track)
        .map(|mut song| {
            song.album_id = Some(id.to_owned());
            if song.artist == album.artist {
                song.artist_id = album.artist_id.as_ref().map(|id| id.as_str().to_owned());
            }
            if song.artwork.is_none() {
                song.artwork.clone_from(&album.artwork);
            }
            song
        })
        .collect();
    CatalogAlbumDetail { album, tracks }
}

/// The catalog id of a catalog song.
fn catalog_song_id(track: &Track) -> Option<&str> {
    track.key.catalog_id()
}
