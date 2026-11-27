//! Searching an open Apple Music library: the library's own matches and those of Apple's catalog,
//! searched together through Apple's search, with Rune's own playlists matched by name.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use apple_music::models::{Album, Artist, Artwork, LibraryAlbum, LibraryArtist, LibrarySong, Song};
use database::actions::collection::CollectionQueryType;
use futures::future::join;
use tokio::time::{self as tokio_time, error::Elapsed};

use super::{CATALOG_PREFIX, album_artwork, album_name, to_count};
use crate::apple_music_artists::{NamedArtist, NamedArtists, named_artist_remote};
use crate::apple_music_binding::{
    AppleError, AppleIdKind, AppleMusicBinding, AppleSong, catalog_track, is_library_song,
};
use crate::messages::{SearchForRequest, SearchForResponse};

/// How long a search waits for the library's part and the catalog's, from its start. Each may
/// take a few seconds on a slow network. A part that fails or has not answered by then is left
/// out and the other answers, within the ten seconds the search page waits for an answer.
pub(crate) const SEARCH_WAIT: Duration = Duration::from_secs(8);

/// How long what the catalog found for a term answers a search for the same term again.
const KEPT_SEARCH_AGE: Duration = Duration::from_secs(2 * 60);

/// How many catalog searches are kept at most; the oldest goes first.
const MAX_KEPT_SEARCHES: usize = 32;

/// What the library's search found.
#[derive(Default)]
pub(crate) struct LibraryFound {
    pub(crate) songs: Vec<LibrarySong>,
    pub(crate) albums: Vec<LibraryAlbum>,
    pub(crate) artists: Vec<LibraryArtist>,
}

/// What the catalog's search found.
#[derive(Default)]
pub(crate) struct CatalogFound {
    pub(crate) songs: Vec<Song>,
    pub(crate) albums: Vec<Album>,
    pub(crate) artists: Vec<Artist>,
}

/// The catalog searches made lately under one sign-in, by the key of their term.
#[derive(Default)]
pub(crate) struct KeptSearches(HashMap<String, (Instant, Arc<CatalogFound>)>);

impl KeptSearches {
    /// What the catalog found for `key` within `KEPT_SEARCH_AGE` before `now`.
    pub(crate) fn fresh(&self, key: &str, now: Instant) -> Option<Arc<CatalogFound>> {
        let (at, found) = self.0.get(key)?;
        (now.saturating_duration_since(*at) < KEPT_SEARCH_AGE).then(|| Arc::clone(found))
    }

    /// Keeps what the catalog found for `key` at `now`, in place of what was kept for it.
    pub(crate) fn keep(&mut self, key: String, now: Instant, found: Arc<CatalogFound>) {
        self.0.remove(&key);
        while self.0.len() >= MAX_KEPT_SEARCHES {
            let Some(oldest) = self
                .0
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.0.remove(&oldest);
        }
        self.0.insert(key, (now, found));
    }
}

/// The term Apple is asked to search for. Apple's search has no wildcards, so a trailing `*`,
/// as the search box appends, is left out, and so are the quotes of an empty search.
fn search_term(query: &str) -> &str {
    let term = query.trim().trim_end_matches('*').trim_end();
    if term == "\"\"" { "" } else { term }
}

/// The key a catalog search for `term` is kept under: what Apple matches the same way, as letter
/// case and runs of spaces are not told apart.
fn search_key(term: &str) -> String {
    term.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// What a part of a search gave: what it found, or why it found nothing.
enum Part<T> {
    Found(T),
    Failed(AppleError),
    Late,
}

impl<T> Part<T> {
    fn of(answer: Result<Result<T, AppleError>, Elapsed>) -> Self {
        match answer {
            Ok(Ok(found)) => Self::Found(found),
            Ok(Err(error)) => Self::Failed(error),
            Err(_) => Self::Late,
        }
    }

    /// Why the part was left out.
    fn left_out(&self) -> String {
        match self {
            Self::Found(_) => String::new(),
            Self::Failed(error) => format!("it failed: {error}"),
            Self::Late => format!("it did not answer within {SEARCH_WAIT:?}"),
        }
    }
}

/// What the catalog finds for `term`: as kept from a search for it within `KEPT_SEARCH_AGE` of
/// this sign-in, or else searched for and kept. With `refresh` the kept search is left aside,
/// and what is found replaces it.
async fn kept_catalog_search(
    binding: &AppleMusicBinding,
    term: &str,
    refresh: bool,
) -> Result<Arc<CatalogFound>, AppleError> {
    let key = search_key(term);
    if !refresh && let Some(found) = binding.kept_search(&key) {
        return Ok(found);
    }
    let generation = binding.generation();
    let found = Arc::new(binding.search_catalog(term).await?);
    binding.keep_search(generation, key, Arc::clone(&found));
    Ok(found)
}

/// The library's and the catalog's matches for `term`, searched at once and waited for until
/// both answered or `SEARCH_WAIT` passed. A part that fails or has not answered by then finds
/// nothing, and is logged with the time the search took. A failure about the sign-in fails the
/// search, as does a search neither part answered.
async fn search_both(
    binding: &AppleMusicBinding,
    term: &str,
    refresh: bool,
) -> Result<(LibraryFound, Arc<CatalogFound>), AppleError> {
    let started = tokio_time::Instant::now();
    let deadline = started + SEARCH_WAIT;
    let (library, catalog) = join(
        tokio_time::timeout_at(deadline, binding.search_library(term)),
        tokio_time::timeout_at(deadline, kept_catalog_search(binding, term, refresh)),
    )
    .await;
    let elapsed = started.elapsed();
    match (Part::of(library), Part::of(catalog)) {
        (Part::Found(library), Part::Found(catalog)) => Ok((library, catalog)),
        (Part::Failed(AppleError::SignedOut), _) | (_, Part::Failed(AppleError::SignedOut)) => {
            Err(AppleError::SignedOut)
        }
        (Part::Found(library), catalog) => {
            log::warn!(
                "An Apple Music search lists the library's matches only, after {elapsed:?}: the catalog search was left out, as {}",
                catalog.left_out()
            );
            Ok((library, Arc::default()))
        }
        (library, Part::Found(catalog)) => {
            log::warn!(
                "An Apple Music search lists the catalog's matches only, after {elapsed:?}: the library search was left out, as {}",
                library.left_out()
            );
            Ok((LibraryFound::default(), catalog))
        }
        (Part::Failed(error), _) | (Part::Late, Part::Failed(error)) => Err(error),
        (Part::Late, Part::Late) => Err(AppleError::Unavailable(format!(
            "neither search answered within {SEARCH_WAIT:?}"
        ))),
    }
}

/// What Apple relates of the library's songs and albums to the catalog's, as the library's
/// search found them and the lists kept of the library know them. Nothing is read from Apple
/// for it.
#[derive(Default)]
struct Copies {
    /// The library song of each catalog song the library's search found a copy of.
    songs: HashMap<String, String>,
    /// The catalog album of each library album Apple names one for, by the library album's id.
    catalog_albums: HashMap<String, String>,
}

impl Copies {
    fn new(found: &LibraryFound, kept_albums: &[LibraryAlbum]) -> Self {
        let mut copies = Self::default();
        for song in &found.songs {
            if let Some(catalog_id) = song.store_id() {
                copies
                    .songs
                    .entry(catalog_id.to_owned())
                    .or_insert_with(|| song.id.clone());
            }
        }
        // The library's search may answer an album without its catalog album, which the kept
        // album list names. Only Apple relates the two: an album of the same name and artist
        // may be another edition.
        for album in found.albums.iter().chain(kept_albums) {
            if let Some(catalog_id) = album.catalog_id() {
                copies
                    .catalog_albums
                    .entry(album.id.clone())
                    .or_insert_with(|| catalog_id.to_owned());
            }
        }
        copies
    }

    /// The library song that is a copy of the catalog song `catalog_id`, as the library's search
    /// or the kept song list knows it.
    fn song(&self, binding: &AppleMusicBinding, catalog_id: &str) -> Option<String> {
        self.songs.get(catalog_id).cloned().or_else(|| {
            binding
                .known_songs()?
                .library_twin(&catalog_track(catalog_id))
                .map(str::to_owned)
        })
    }

    /// The catalog album Apple names for the library album `library_id`.
    fn catalog_album(&self, library_id: &str) -> Option<&str> {
        self.catalog_albums.get(library_id).map(String::as_str)
    }
}

/// The songs found, as tracks: the library's matches, then the catalog's, `n` of each. A catalog
/// song the library holds a copy of is that copy, listed once. Each comes with the song the
/// binding is to remember for it: none for a copy the song list holds already.
fn found_tracks(
    binding: &AppleMusicBinding,
    library: &LibraryFound,
    catalog: &CatalogFound,
    copies: &Copies,
    n: usize,
) -> Vec<(String, Option<AppleSong>)> {
    let mut listed = Vec::new();
    let mut seen = HashSet::new();
    let own = library
        .songs
        .iter()
        .filter(|song| is_library_song(song) && !song.is_music_video())
        .take(n);
    for song in own {
        if seen.insert(song.id.clone()) {
            listed.push((song.id.clone(), Some(AppleSong::Library(song.clone()))));
        }
    }
    for song in catalog.songs.iter().take(n) {
        let entry = match copies.song(binding, &song.id) {
            Some(copy) => {
                let found = library.songs.iter().find(|listed| listed.id == copy);
                let remembered = found.map(|found| AppleSong::Library(found.clone()));
                (copy, remembered)
            }
            None => (
                catalog_track(&song.id),
                Some(AppleSong::Catalog(song.clone())),
            ),
        };
        if seen.insert(entry.0.clone()) {
            listed.push(entry);
        }
    }
    listed
}

/// The albums found, as numbered: the library's matches, then the catalog's, `n` of each, each
/// listed once. An album Apple names a catalog album for is that catalog album, whether or not
/// the library holds a copy of it: a search finds the whole of an album, which the catalog's
/// album lists and the library's copy lists only as far as the library holds its songs. An
/// album of the library Apple names none for is the library's. Each is remembered with its
/// name and cover, so it opens without another read.
fn found_albums(
    binding: &AppleMusicBinding,
    generation: u64,
    library: &LibraryFound,
    catalog: &CatalogFound,
    copies: &Copies,
    n: usize,
) -> Result<Vec<i32>> {
    let mut found: Vec<(String, &str, Option<&Artwork>)> = Vec::new();
    for album in library.albums.iter().take(n) {
        let remote = match copies.catalog_album(&album.id) {
            Some(catalog_id) => format!("{CATALOG_PREFIX}{catalog_id}"),
            None => album.id.clone(),
        };
        found.push((remote, album_name(album), album_artwork(album)));
    }
    for album in catalog.albums.iter().take(n) {
        let attributes = album.attributes.as_ref();
        let name = attributes
            .and_then(|attributes| attributes.name.as_deref())
            .unwrap_or_default();
        let artwork = attributes.and_then(|attributes| attributes.artwork.as_ref());
        found.push((format!("{CATALOG_PREFIX}{}", album.id), name, artwork));
    }
    let mut listed = Vec::new();
    let mut seen = HashSet::new();
    for (remote, name, artwork) in found {
        let id = binding.intern(AppleIdKind::Album, &remote)?;
        if seen.insert(id) {
            binding.remember_collection(generation, AppleIdKind::Album, id, name, artwork);
            listed.push(id);
        }
    }
    Ok(listed)
}

/// The artists found, as numbered: those the library artists found are, then the catalog
/// artists found, `n` of each, each listed once. An artist Apple names a catalog artist for is
/// that catalog artist, whether or not the library's artist list has it: a search finds the
/// whole of an artist, which the catalog's page of it shows and the library's page does not. It
/// is remembered with its name and picture. An artist of the list Apple names no catalog artist
/// for is that entry, which opens the library's page of it.
fn found_artists(
    binding: &AppleMusicBinding,
    generation: u64,
    named: &NamedArtists,
    library: &LibraryFound,
    catalog: &CatalogFound,
    n: usize,
) -> Result<Vec<i32>> {
    // Each artist with the name and picture a catalog artist is remembered by.
    type Found<'a> = (String, Option<(&'a str, Option<&'a Artwork>)>);
    fn of_list(listed: &NamedArtist) -> Found<'_> {
        match listed.catalog_id() {
            Some(id) => (
                format!("{CATALOG_PREFIX}{id}"),
                Some((listed.name.as_str(), listed.artwork.as_ref())),
            ),
            None => (listed.remote(), None),
        }
    }
    let mut found: Vec<Found> = Vec::new();
    for artist in library.artists.iter().take(n) {
        let credited = named
            .iter()
            .filter(|listed| listed.credits.contains(&artist.id));
        let before = found.len();
        found.extend(credited.map(of_list));
        if found.len() > before {
            continue;
        }
        let by_name = artist
            .attributes
            .as_ref()
            .and_then(|attributes| attributes.name.as_deref())
            .and_then(named_artist_remote)
            .and_then(|remote| named.get(&remote));
        found.extend(by_name.map(of_list));
    }
    for artist in catalog.artists.iter().take(n) {
        let attributes = artist.attributes.as_ref();
        let name = attributes
            .and_then(|attributes| attributes.name.as_deref())
            .unwrap_or_default();
        let artwork = attributes.and_then(|attributes| attributes.artwork.as_ref());
        found.push((
            format!("{CATALOG_PREFIX}{}", artist.id),
            Some((name, artwork)),
        ));
    }
    let mut listed = Vec::new();
    let mut seen = HashSet::new();
    for (remote, remembered) in found {
        let id = binding.intern(AppleIdKind::Artist, &remote)?;
        if !seen.insert(id) {
            continue;
        }
        if let Some((name, artwork)) = remembered {
            binding.remember_collection(generation, AppleIdKind::Artist, id, name, artwork);
        }
        listed.push(id);
    }
    Ok(listed)
}

/// Rune's playlists whose names hold `term`, ignoring case: the first `n` in the order they were
/// made. The playlist list holds Rune's playlists only.
async fn rune_playlist_matches(
    binding: &AppleMusicBinding,
    term: &str,
    n: usize,
) -> Result<Vec<i32>> {
    let term = term.to_lowercase();
    if n == 0 || term.is_empty() {
        return Ok(Vec::new());
    }
    binding
        .rune_playlists
        .listed()
        .await?
        .iter()
        .filter(|playlist| playlist.name.to_lowercase().contains(&term))
        .take(n)
        .map(|playlist| binding.intern(AppleIdKind::RunePlaylist, &playlist.id))
        .collect()
}

/// Searches the library and Apple's catalog together, `n` of each kind from each, in Apple's
/// order: the library's matches come first, then the catalog's. A catalog song the library holds
/// a copy of is answered as that copy, and an album or artist Apple names a catalog one for as
/// the catalog's, which holds the whole of it. When one part cannot be searched in time, the
/// other still answers (see `search_both`). What the catalog found is kept for a while, so a
/// search for the same text again asks it nothing; with `refresh` it is asked again. The artists
/// found are the catalog artists Apple names for those the library holds, then those the catalog
/// found, and the artists of the library's artist list Apple names none for (see
/// `found_artists`), or the catalog artists alone when the list cannot be read in time, and the
/// playlists Rune's own whose names hold the text. Catalog songs are named `catalog:<id>`, and
/// catalog albums and artists open from the catalog. A search for playlists alone asks Apple
/// nothing, so it finds Rune's signed out and offline.
pub(crate) async fn search_for_request(
    binding: &AppleMusicBinding,
    request: &SearchForRequest,
) -> Result<Option<SearchForResponse>> {
    let fields: Vec<CollectionQueryType> = request.fields.iter().copied().map(Into::into).collect();
    let wants = |field: CollectionQueryType| {
        if fields.is_empty() || fields.contains(&field) {
            to_count(request.n)
        } else {
            0
        }
    };
    let mut response = SearchForResponse {
        call_id: request.call_id,
        artists: Vec::new(),
        albums: Vec::new(),
        playlists: Vec::new(),
        tracks: Vec::new(),
        genres: Vec::new(),
        directories: Vec::new(),
        mixes: Vec::new(),
    };
    let term = search_term(&request.query_str);
    if term.is_empty() {
        return Ok(Some(response));
    }
    let playlists = wants(CollectionQueryType::Playlist);
    let from_apple = [
        CollectionQueryType::Track,
        CollectionQueryType::Album,
        CollectionQueryType::Artist,
    ];
    if from_apple.into_iter().all(|field| wants(field) == 0) {
        response.playlists = rune_playlist_matches(binding, term, playlists).await?;
        return Ok(Some(response));
    }
    let generation = binding.generation();
    let artists = wants(CollectionQueryType::Artist);
    // The artist list is read only for a search that asks for artists.
    let (found, named) = join(search_both(binding, term, request.refresh), async {
        if artists == 0 {
            return None;
        }
        Some(tokio_time::timeout(SEARCH_WAIT, binding.named_artists()).await)
    })
    .await;
    let (library, catalog) =
        found.map_err(|error| binding.request_error(error, "Apple Music search"))?;
    let copies = Copies::new(&library, &binding.kept_albums().await.unwrap_or_default());

    let tracks = found_tracks(
        binding,
        &library,
        &catalog,
        &copies,
        wants(CollectionQueryType::Track),
    );
    binding.remember_found(generation, &tracks);
    response.tracks = tracks
        .iter()
        .map(|(track, _)| binding.intern_track(track))
        .collect::<Result<_>>()?;
    response.albums = found_albums(
        binding,
        generation,
        &library,
        &catalog,
        &copies,
        wants(CollectionQueryType::Album),
    )?;
    // Without the artist list, the catalog artists found are still listed, as themselves: they
    // open the catalog's page, which needs no list.
    let named = match named {
        None => None,
        Some(Ok(Ok(named))) => Some(named),
        Some(Ok(Err(error))) => {
            log::warn!(
                "A search lists only the catalog's Apple Music artists, as the artist list failed: {error:#}"
            );
            Some(Arc::default())
        }
        Some(Err(_)) => {
            log::warn!(
                "A search lists only the catalog's Apple Music artists, as the artist list took too long"
            );
            Some(Arc::default())
        }
    };
    response.artists = match named {
        Some(named) => found_artists(binding, generation, &named, &library, &catalog, artists)?,
        None => Vec::new(),
    };
    response.playlists = rune_playlist_matches(binding, term, playlists).await?;
    Ok(Some(response))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use apple_music::models::LibraryAlbum;
    use serde_json::json;

    use super::{
        CatalogFound, Copies, KEPT_SEARCH_AGE, KeptSearches, LibraryFound, MAX_KEPT_SEARCHES,
        search_key, search_term,
    };

    fn library_album(id: &str, catalog_id: Option<&str>) -> LibraryAlbum {
        let mut album = json!({"id": id, "type": "library-albums",
            "attributes": {"name": "Night", "artistName": "Artist One"}});
        if let Some(catalog_id) = catalog_id {
            album["relationships"] =
                json!({"catalog": {"data": [{"id": catalog_id, "type": "albums"}]}});
        }
        serde_json::from_value(album).unwrap()
    }

    #[test]
    fn a_library_album_is_a_catalog_album_only_when_apple_relates_the_two() {
        // The library's search answers the library album without its catalog album, which the
        // kept album list names.
        let found = LibraryFound {
            albums: vec![library_album("l.night", None)],
            ..LibraryFound::default()
        };
        let copies = Copies::new(&found, &[library_album("l.night", Some("100"))]);
        assert_eq!(copies.catalog_album("l.night"), Some("100"));
        // A library album whose catalog album nothing names stands for none, whatever the
        // catalog holds of the same name and artist.
        let copies = Copies::new(&found, &[]);
        assert_eq!(copies.catalog_album("l.night"), None);
    }

    #[test]
    fn a_term_is_searched_without_the_wildcard_and_kept_by_what_apple_matches() {
        assert_eq!(search_term("  dawn* "), "dawn");
        assert_eq!(search_term("\"\""), "");
        assert_eq!(search_term("*"), "");
        assert_eq!(search_key("Dawn   Light"), search_key("dawn light"));
    }

    #[test]
    fn a_kept_search_answers_for_a_while_and_the_oldest_goes_first() {
        let start = Instant::now();
        let mut kept = KeptSearches::default();
        kept.keep("dawn".into(), start, Default::default());
        assert!(kept.fresh("dawn", start + KEPT_SEARCH_AGE / 2).is_some());
        assert!(kept.fresh("dawn", start + KEPT_SEARCH_AGE).is_none());
        for at in 1..=MAX_KEPT_SEARCHES {
            kept.keep(
                format!("term {at}"),
                start + Duration::from_secs(at as u64),
                std::sync::Arc::new(CatalogFound::default()),
            );
        }
        assert!(kept.fresh("dawn", start).is_none());
        assert!(kept.fresh("term 1", start).is_some());
    }
}
