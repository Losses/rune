//! The user's Apple Music library, signed in with the tokens the platform hands over. The tokens
//! live in memory only, in the binding. The library's lists are kept in the service library's
//! cache on disk, so the binding answers from them before Apple confirms a sign-in, and also
//! before any, as after a restart while the device is offline.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use anyhow::Result;
use apple_music::models::{
    Album, Artwork, CatalogSearchType, LibraryAlbum, LibraryArtist, LibraryEntry, LibraryPlaylist,
    LibrarySearchType, LibrarySong, Playlist, Recommendation, Relationship, Resource, Song,
    SongWithArtists,
};
use apple_music::{
    AppleMusicClient, CATALOG_SEARCH_LIMIT, CATALOG_SONGS_BATCH, Error as AppleApiError,
    LIBRARY_SEARCH_LIMIT, StaticTokens, TokenSource,
};
use async_trait::async_trait;
use database::error::ExpectedError;
use database::library_registry::ServiceKind;
use database::playing_item::PlayingItemMetadataSummary;
use futures::future::{BoxFuture, join, join_all, try_join};
use futures::{FutureExt, StreamExt, TryStreamExt, stream};
use playback::hosted::HostedTrack;
use playback::player::{PlayingItem, ServiceTarget};
use rand::seq::SliceRandom;
use serde::de::{DeserializeOwned, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time as tokio_time;
use uuid::Uuid;

use crate::{
    apple_music_account,
    apple_music_artists::{NamedArtist, NamedArtists, SongArtists, is_named_artist},
    apple_music_held::{self, HeldStore},
    apple_music_import,
    apple_music_translate::search::{CatalogFound, KeptSearches, LibraryFound},
    macros::RequestSlots,
    messages::{AudioAvailability, LibraryContentChanged},
    request_error::ExpectedRequestError,
    service::{
        ServiceBinding, ServiceFuture,
        covers::{CoverSource, MAX_COVER_BYTES, ServiceCoverCache, cover_cache_root},
        device_records::device_records_folder,
        folder::prepare_service_library_folder,
        ids::RemoteIdMap,
        lists::{KeptList, ListSpec, ServiceLists, kept_value, lists_folder},
        lrclib::{LibraryLyrics, Lrclib, Lyrics, SongQuery},
        reach::Reach,
        registry::{Clock, SignInFailure},
        rune_likes::{LIKES_FILE, RuneLikes},
        rune_mixes::{MIXES_FILE, RuneMixes},
        rune_playlists::{PLAYLISTS_FILE, RunePlaylists},
    },
    utils::Broadcaster,
    visualizer::RANDOM_COVERS,
};

/// The name the platform host that plays Apple Music tracks goes by.
pub(crate) const APPLE_MUSIC_HOST: &str = "appleMusic";

/// How long Apple may take to confirm a sign-in.
const SIGN_IN_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// The side, in pixels, of the covers this binding downloads.
const COVER_SIZE: u32 = 600;

/// How long the binding waits after a change of what the library shows before it tells the UI,
/// so that a burst of changes, as the lists read again one after another, is told once, after
/// the last of them.
pub(crate) const CONTENT_CHANGED_DELAY: Duration = Duration::from_millis(300);

/// How long the artist list waits, the first time it is asked for, for the catalog artists of
/// the library's songs to be read before it stands the songs under their whole credits instead.
pub(crate) const FIRST_SONG_ARTISTS_WAIT: Duration = Duration::from_secs(5);

/// How many batches of the library's songs are asked about their catalog artists at once.
const SONG_ARTIST_READS: usize = 4;

/// Library pages and playlists read at once.
const PARALLEL_READS: usize = 4;

/// Relationships read with library songs, for their album and artist ids.
const SONG_INCLUDE: &[&str] = &["albums", "artists"];

/// The songs fetched one by one, kept so a queue can show them without asking Apple again.
const MAX_FETCHED_TRACKS: usize = 2000;

/// How many songs `read_unknown_tracks` asks for in one step.
const READ_TRACKS_CHUNK: usize = 1000;

/// How many songs Apple said it does not have are remembered, so a queue or a page that names
/// them does not ask for them again each time it is shown.
const MAX_MISSING_TRACKS: usize = 2000;

/// How many catalog albums and playlists keep their songs at once, and how many songs they may
/// hold in all.
const MAX_KEPT_COLLECTIONS: usize = 64;
const MAX_KEPT_COLLECTION_SONGS: usize = 20_000;

/// How many recommended mixes are kept across reads of the recommendations.
const MAX_KEPT_MIXES: usize = 1000;

/// How a track names a catalog song: `catalog:<id>`. A library song is named by its library id,
/// which starts with `i.`. The platform host takes a track as it is.
const CATALOG_PREFIX: &str = "catalog:";

/// What Apple's library song ids start with.
const LIBRARY_SONG_PREFIX: &str = "i.";

/// How long the song list and the album list stay current once read. Apple reads them a page
/// at a time, so a large library takes many requests.
pub(crate) const LONG_LIST_AGE: Duration = Duration::from_secs(10 * 60);

/// How long the artists and the catalog artists of the songs stay current once read.
pub(crate) const SHORT_LIST_AGE: Duration = Duration::from_secs(60);

/// The library's songs, then the songs of the user's own playlists the library does not hold.
const SONGS: ListSpec = ListSpec {
    name: "songs",
    max_age: LONG_LIST_AGE,
};

/// The library's albums, the ones added last first.
const ALBUMS: ListSpec = ListSpec {
    name: "albums",
    max_age: LONG_LIST_AGE,
};

/// The library's artists, with the catalog artists Apple maps them to.
const ARTISTS: ListSpec = ListSpec {
    name: "artists",
    max_age: SHORT_LIST_AGE,
};

/// The catalog artists Apple names for the library's songs. A read asks only about the songs not
/// asked about yet.
const SONG_ARTISTS: ListSpec = ListSpec {
    name: "song-artists",
    max_age: SHORT_LIST_AGE,
};

/// What Apple recommends to the user, in groups of albums, playlists and stations.
const RECOMMENDATIONS: ListSpec = ListSpec {
    name: "recommendations",
    max_age: SHORT_LIST_AGE,
};

/// The kinds of ids the binding numbers. A library song and a catalog song share their numbers,
/// as a media file id names a song of either, and so do the kinds of ids one request takes
/// for playlists, or for mixes.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum AppleIdKind {
    LibrarySong,
    CatalogSong,
    Album,
    Artist,
    CoverArt,
    /// A Rune playlist kept on this device, by the id its store keeps it under.
    RunePlaylist,
    /// A playlist of the catalog, by `catalog:` and its catalog id. `PlaylistRef` tells it from
    /// a Rune playlist.
    Playlist,
    /// A Rune mix kept on this device, by the id its store keeps it under.
    RuneMix,
    /// A mix of one album or playlist Apple recommends, by `album:` or `playlist:` and the id of
    /// what it plays. `MixRef` tells it from a Rune mix.
    Mix,
}

impl AppleIdKind {
    /// The kind whose numbers the kind takes.
    fn numbering(self) -> Self {
        match self {
            Self::CatalogSong => Self::LibrarySong,
            Self::Playlist => Self::RunePlaylist,
            Self::Mix => Self::RuneMix,
            other => other,
        }
    }
}

/// What a playlist id the UI holds names. Every request that takes one matches on it first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PlaylistRef {
    /// A Rune playlist kept on this device, by the id its store keeps it under.
    Rune(String),
    /// A playlist of the catalog, by the id the UI holds, which may still be an id the binding
    /// never gave.
    Catalog(i32),
}

/// What a mix id the UI holds names. Every request that takes one matches on it first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MixRef {
    /// A Rune mix kept on this device, by the id its store keeps it under.
    Rune(String),
    /// A mix Apple recommends, by the id the UI holds, which may still be an id the binding
    /// never gave.
    Recommended(i32),
}

/// An album or a playlist of the catalog whose songs a page lists, by the id the UI knows it by.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum CatalogCollection {
    Album(i32),
    Playlist(i32),
}

impl CatalogCollection {
    /// The kind and the id the UI knows the collection by.
    pub(crate) fn kind_and_id(self) -> (AppleIdKind, i32) {
        match self {
            Self::Album(id) => (AppleIdKind::Album, id),
            Self::Playlist(id) => (AppleIdKind::Playlist, id),
        }
    }
}

/// A mix made of one album or playlist Apple recommends. Its id names what it plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecommendedMix {
    pub(crate) name: String,
    /// The title of the recommendation group it was first listed in.
    pub(crate) group: String,
    pub(crate) cover: Option<String>,
}

/// The recommendation groups in Apple's order, each a title with the ids of its mixes.
pub(crate) type MixGroups = Vec<(String, Vec<i32>)>;

/// Whether an Apple id names a song of the user's library.
pub(crate) fn is_library_song_id(id: &str) -> bool {
    id.len() > LIBRARY_SONG_PREFIX.len() && id.starts_with(LIBRARY_SONG_PREFIX)
}

/// Whether an entry of the song list is a song of the library, rather than a song of a playlist
/// the library does not hold.
pub(crate) fn is_library_song(song: &LibrarySong) -> bool {
    is_library_song_id(&song.id)
}

/// The track of the catalog song `id`.
pub(crate) fn catalog_track(id: &str) -> String {
    format!("{CATALOG_PREFIX}{id}")
}

/// The track of an entry of the song list: a library song by its library id, any other entry by
/// the catalog song it is. None for a music video, which has no song to play, and for an entry
/// that names no catalog song.
pub(crate) fn listed_track(song: &LibrarySong) -> Option<String> {
    if song.is_music_video() {
        return None;
    }
    if is_library_song(song) {
        Some(song.id.clone())
    } else {
        song.store_id()
            .filter(|id| !id.is_empty())
            .map(catalog_track)
    }
}

/// The kind of the song a track names and Apple's id of it, or None for a text that names no
/// song.
pub(crate) fn song_of_track(track: &str) -> Option<(AppleIdKind, &str)> {
    match track.strip_prefix(CATALOG_PREFIX) {
        Some(id) if !id.is_empty() && !id.starts_with(CATALOG_PREFIX) => {
            Some((AppleIdKind::CatalogSong, id))
        }
        Some(_) => None,
        None if is_library_song_id(track) => Some((AppleIdKind::LibrarySong, track)),
        None => None,
    }
}

fn parse_track(track: &str) -> Result<(AppleIdKind, &str)> {
    song_of_track(track)
        .ok_or_else(|| ExpectedRequestError::invalid_input("Not an Apple Music track").into())
}

/// The address of the cover `artwork` shows at `size` pixels, never above the largest size
/// Apple offers. None when Apple gives no address.
pub(crate) fn artwork_url(artwork: &Artwork, size: u32) -> Option<String> {
    if artwork.url.is_empty() {
        return None;
    }
    let side = size
        .min(artwork.width.unwrap_or(u32::MAX))
        .max(1)
        .to_string();
    Some(
        artwork
            .url
            .replace("{w}", &side)
            .replace("{h}", &side)
            .replace("{c}", "bb")
            .replace("{f}", "jpg"),
    )
}

/// The address of the cover to show for `artwork`.
pub(crate) fn cover_url(artwork: Option<&Artwork>) -> Option<String> {
    artwork.and_then(|artwork| artwork_url(artwork, COVER_SIZE))
}

/// The year of a release date Apple gives.
pub(crate) fn release_year(date: Option<&str>) -> Option<u32> {
    date?.get(..4)?.parse().ok()
}

/// A related resource cut to its id and type, which is all the binding reads of it.
fn id_only<A, R>(resource: Resource<A, R>) -> Resource<A, R> {
    Resource {
        id: resource.id,
        kind: resource.kind,
        href: None,
        attributes: None,
        relationships: None,
    }
}

fn ids_only<A>(relationship: Option<Relationship<A>>) -> Option<Relationship<A>> {
    relationship.map(|relationship| Relationship {
        href: None,
        next: None,
        data: relationship.data.into_iter().map(id_only).collect(),
    })
}

/// A library song as kept: its relationships name their resources by id only.
fn kept_song(mut song: LibrarySong) -> LibrarySong {
    song.href = None;
    if let Some(relationships) = &mut song.relationships {
        relationships.catalog = ids_only(relationships.catalog.take());
        relationships.albums = ids_only(relationships.albums.take());
        relationships.artists = ids_only(relationships.artists.take());
    }
    song
}

/// A library album as kept: its relationships name their resources by id only.
fn kept_album(mut album: LibraryAlbum) -> LibraryAlbum {
    album.href = None;
    if let Some(Value::Object(relationships)) = &mut album.relationships {
        for relationship in relationships.values_mut() {
            if let Some(Value::Array(data)) = relationship.get_mut("data") {
                for resource in data.iter_mut() {
                    if let Value::Object(resource) = resource {
                        resource.retain(|key, _| key == "id" || key == "type");
                    }
                }
            }
            if let Value::Object(relationship) = relationship {
                relationship.retain(|key, _| key == "data");
            }
        }
    }
    album
}

/// The user's songs: the library's songs in the order Apple lists them, which is the order they
/// were added in, then the songs of the user's own playlists and Favorites that the library does
/// not hold, each once. A catalog song the library holds a copy of is listed as that copy. The
/// songs Rune holds in the library come last (see `apple_music_held`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AppleSongs {
    songs: Vec<LibrarySong>,
    /// The library song that is the library's copy of each catalog song it holds one of, by
    /// catalog id.
    twins: BTreeMap<String, String>,
    /// The track of each song.
    tracks: Vec<String>,
    /// Where each track is in `songs`.
    index: HashMap<String, usize>,
    /// How many of the last songs are those Rune holds in the library. The list kept on disk is
    /// Apple's alone, and holds none.
    held: usize,
}

impl AppleSongs {
    pub(crate) fn songs(&self) -> &[LibrarySong] {
        &self.songs
    }

    /// The songs with their tracks, in order.
    pub(crate) fn entries(&self) -> impl DoubleEndedIterator<Item = (&str, &LibrarySong)> {
        self.tracks.iter().map(String::as_str).zip(&self.songs)
    }

    /// Where the songs Rune holds start.
    fn held_from(&self) -> usize {
        self.songs.len() - self.held
    }

    /// The songs of the library with their tracks, in order: the library's own songs and those
    /// Rune holds in it, not the songs only a playlist holds.
    pub(crate) fn library_entries(&self) -> impl Iterator<Item = (&str, &LibrarySong)> {
        let held_from = self.held_from();
        self.entries()
            .enumerate()
            .filter(move |(at, (_, song))| *at >= held_from || is_library_song(song))
            .map(|(_, entry)| entry)
    }

    /// The songs Rune holds in the library, in order.
    pub(crate) fn held_songs(&self) -> &[LibrarySong] {
        &self.songs[self.held_from()..]
    }

    /// Whether the song of `track` is one Rune holds in the library.
    pub(crate) fn holds(&self, track: &str) -> bool {
        self.index
            .get(track)
            .is_some_and(|&at| at >= self.held_from())
    }

    /// These songs with `held`, the songs Rune holds in the library, after them, each unless it
    /// is listed already, as a song the library has a copy of is, or names no song. A song that
    /// only one of the user's playlists lists so far moves among them, as Rune holds it in the
    /// library.
    pub(crate) fn with_held(&self, held: impl IntoIterator<Item = LibrarySong>) -> Self {
        let held: Vec<LibrarySong> = held.into_iter().collect();
        let held_from = self.held_from();
        let moved: HashSet<String> = held
            .iter()
            .filter_map(listed_track)
            .filter(|track| {
                self.library_twin(track).is_none()
                    && self
                        .index
                        .get(track)
                        .is_some_and(|&at| at < held_from && !is_library_song(&self.songs[at]))
            })
            .collect();
        let mut songs = if moved.is_empty() {
            self.clone()
        } else {
            let mut songs = Self {
                twins: self.twins.clone(),
                ..Self::default()
            };
            for (at, (track, song)) in self.entries().enumerate() {
                if at >= held_from || !moved.contains(track) {
                    songs.push(song.clone());
                }
            }
            songs.held = self.held;
            songs
        };
        let before = songs.len();
        for song in held {
            songs.push(song);
        }
        songs.held += songs.len() - before;
        songs
    }

    pub(crate) fn len(&self) -> usize {
        self.songs.len()
    }

    /// The listed song of `track`.
    pub(crate) fn get(&self, track: &str) -> Option<&LibrarySong> {
        self.index.get(track).map(|&at| &self.songs[at])
    }

    /// The library song that is the library's copy of the catalog song `track` names.
    pub(crate) fn library_twin(&self, track: &str) -> Option<&str> {
        match song_of_track(track)? {
            (AppleIdKind::CatalogSong, id) => self.twins.get(id).map(String::as_str),
            _ => None,
        }
    }

    /// The catalog id of the catalog song the library song `track` is the library's copy of,
    /// as the copies noted name it.
    pub(crate) fn catalog_twin(&self, track: &str) -> Option<&str> {
        self.twins
            .iter()
            .find(|(_, library_id)| *library_id == track)
            .map(|(catalog_id, _)| catalog_id.as_str())
    }

    /// Whether `track`, or the library song a catalog track stands for, is listed.
    fn lists(&self, track: &str) -> bool {
        self.index.contains_key(track) || self.library_twin(track).is_some()
    }

    /// Lists `song` at the end, unless it is listed already or names no song.
    fn push(&mut self, song: LibrarySong) {
        let Some(track) = listed_track(&song) else {
            return;
        };
        if self.lists(&track) {
            return;
        }
        self.index.insert(track.clone(), self.songs.len());
        self.tracks.push(track);
        self.songs.push(kept_song(song));
    }

    /// Notes that the catalog song `catalog_id` is the library song `library_id`. The first copy
    /// noted stays.
    fn add_twin(&mut self, catalog_id: &str, library_id: &str) {
        self.twins
            .entry(catalog_id.to_owned())
            .or_insert_with(|| library_id.to_owned());
    }
}

/// What a song list is kept as: its songs in order and the library copies of catalog songs.
/// The tracks and the index are built again when it is read back.
#[derive(Serialize)]
struct KeptSongsRef<'a> {
    songs: &'a [LibrarySong],
    twins: &'a BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct KeptSongs {
    songs: Vec<LibrarySong>,
    twins: BTreeMap<String, String>,
}

impl Serialize for AppleSongs {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        KeptSongsRef {
            songs: &self.songs,
            twins: &self.twins,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AppleSongs {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let kept = KeptSongs::deserialize(deserializer)?;
        let mut songs = Self {
            twins: kept.twins,
            ..Self::default()
        };
        for song in kept.songs {
            songs.push(song);
        }
        Ok(songs)
    }
}

/// A song as Apple gives it: a song of the library, or a song of the catalog.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AppleSong {
    Library(LibrarySong),
    Catalog(Song),
}

/// What a page shows of a song, read from the song as Apple gives it.
#[derive(Clone, Copy, Debug)]
pub(crate) enum SongRef<'a> {
    Library(&'a LibrarySong),
    Catalog(&'a Song),
}

impl AppleSong {
    pub(crate) fn as_ref(&self) -> SongRef<'_> {
        match self {
            Self::Library(song) => SongRef::Library(song),
            Self::Catalog(song) => SongRef::Catalog(song),
        }
    }
}

macro_rules! song_attribute {
    ($self:ident, $field:ident) => {
        match $self {
            SongRef::Library(song) => song.attributes.as_ref().and_then(|a| a.$field.as_ref()),
            SongRef::Catalog(song) => song.attributes.as_ref().and_then(|a| a.$field.as_ref()),
        }
    };
}

impl<'a> SongRef<'a> {
    pub(crate) fn title(self) -> &'a str {
        song_attribute!(self, name).map_or("", String::as_str)
    }

    pub(crate) fn album(self) -> Option<&'a str> {
        song_attribute!(self, album_name).map(String::as_str)
    }

    pub(crate) fn artist(self) -> Option<&'a str> {
        song_attribute!(self, artist_name).map(String::as_str)
    }

    pub(crate) fn artwork(self) -> Option<&'a Artwork> {
        song_attribute!(self, artwork)
    }

    pub(crate) fn track_number(self) -> Option<u32> {
        song_attribute!(self, track_number).copied()
    }

    pub(crate) fn disc_number(self) -> Option<u32> {
        song_attribute!(self, disc_number).copied()
    }

    pub(crate) fn year(self) -> Option<u32> {
        release_year(song_attribute!(self, release_date).map(String::as_str))
    }

    /// Its length in seconds.
    pub(crate) fn duration(self) -> f64 {
        song_attribute!(self, duration_in_millis)
            .map(|millis| Duration::from_millis(*millis).as_secs_f64())
            .unwrap_or_default()
    }

    /// The id of its album: a library album for a library song, a catalog album for a catalog
    /// one, when Apple named it.
    pub(crate) fn album_id(self) -> Option<&'a str> {
        match self {
            Self::Library(song) => song.album_id(),
            Self::Catalog(song) => song.album_id(),
        }
    }

    /// The id of its first artist, as `album_id` names albums.
    pub(crate) fn artist_id(self) -> Option<&'a str> {
        match self {
            Self::Library(song) => song.artist_id(),
            Self::Catalog(song) => song.artist_id(),
        }
    }
}

/// Why a call to Apple Music failed, without anything Apple sent but its status and codes, so it
/// can be logged and shared by every caller that waited for the call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AppleError {
    /// No tokens were handed over, or Apple rejects them: the user has to sign in again.
    SignedOut,
    /// Apple refused this call though the tokens work.
    Refused,
    /// What was asked for is not in the library or the catalog.
    NotFound,
    /// Apple or the network cannot answer now; trying later may work.
    Unavailable(String),
    /// Any other failure of the call.
    Rejected(String),
}

impl fmt::Display for AppleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SignedOut => f.write_str("Apple Music is not signed in"),
            Self::Refused => f.write_str("Apple Music refused the request"),
            Self::NotFound => f.write_str("Apple Music does not have it"),
            Self::Unavailable(detail) => write!(f, "Apple Music is unavailable: {detail}"),
            Self::Rejected(detail) => write!(f, "Apple Music rejected the request: {detail}"),
        }
    }
}

/// What Apple said about a failed request, without anything that could hold a token or the text
/// of an answer.
fn apple_detail(error: &AppleApiError) -> String {
    match error {
        AppleApiError::Api { status, errors } | AppleApiError::Server { status, errors } => {
            let codes: Vec<&str> = errors
                .iter()
                .filter_map(|error| error.code.as_deref())
                .collect();
            if codes.is_empty() {
                format!("status {status}")
            } else {
                format!("status {status}, code {}", codes.join(", "))
            }
        }
        AppleApiError::ClientNotRegistered => "the client is not registered".to_owned(),
        AppleApiError::Decode(_) => "an answer that could not be read".to_owned(),
        AppleApiError::RateLimited { retry_after } => {
            format!("too many requests, retry after {retry_after:?}")
        }
        AppleApiError::Http(error) if error.is_timeout() => "no answer in time".to_owned(),
        AppleApiError::Http(error) if error.is_connect() => "no connection".to_owned(),
        AppleApiError::Http(_) => "a failed connection".to_owned(),
        _ => "an unexpected answer".to_owned(),
    }
}

/// Whether an error says Apple does not take the tokens.
fn rejects_tokens(error: &AppleApiError) -> bool {
    matches!(
        error,
        AppleApiError::Unauthorized
            | AppleApiError::Forbidden
            | AppleApiError::MissingUserToken
            | AppleApiError::Token(_)
    )
}

/// An error of Apple's client as the binding tells it, without deciding what a 401 or 403
/// means: [`AppleMusicBinding::judged`] does that by reading with the same tokens.
fn classified(error: &AppleApiError) -> AppleError {
    match error {
        AppleApiError::MissingUserToken | AppleApiError::Token(_) => AppleError::SignedOut,
        AppleApiError::Unauthorized | AppleApiError::Forbidden => AppleError::Refused,
        AppleApiError::NotFound(_) => AppleError::NotFound,
        AppleApiError::RateLimited { .. }
        | AppleApiError::Server { .. }
        | AppleApiError::Http(_) => AppleError::Unavailable(apple_detail(error)),
        error => AppleError::Rejected(apple_detail(error)),
    }
}

/// The two tokens of a sign-in, in memory only.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Tokens {
    developer: String,
    user: String,
}

impl Tokens {
    /// The developer token and the Music User Token, or None when one is empty.
    pub(crate) fn new(developer: &str, user: &str) -> Option<Self> {
        (!developer.is_empty() && !user.is_empty()).then(|| Self {
            developer: developer.to_owned(),
            user: user.to_owned(),
        })
    }
}

impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens")
            .field("developer", &"<redacted>")
            .field("user", &"<redacted>")
            .finish()
    }
}

/// The tokens of the binding's sign-in, which its client asks for. Nothing newer can be had on
/// refresh, so a rejected token stays rejected until the platform signs in again.
#[derive(Default)]
pub(crate) struct SessionTokens(Mutex<Option<Tokens>>);

impl SessionTokens {
    /// Keeps `tokens`, or none, and says whether they differ from those held.
    fn replace(&self, tokens: Option<Tokens>) -> bool {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let changed = *held != tokens;
        *held = tokens;
        changed
    }

    fn signed_in(&self) -> bool {
        self.read(|_| ()).is_some()
    }

    fn read<T>(&self, pick: impl FnOnce(&Tokens) -> T) -> Option<T> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(pick)
    }
}

impl fmt::Debug for SessionTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionTokens")
            .field("signed_in", &self.signed_in())
            .finish()
    }
}

#[async_trait]
impl TokenSource for SessionTokens {
    async fn developer_token(&self, _refresh: bool) -> apple_music::Result<String> {
        self.read(|tokens| tokens.developer.clone())
            .ok_or_else(|| AppleApiError::Token("not signed in to Apple Music".into()))
    }

    async fn user_token(&self, _refresh: bool) -> apple_music::Result<Option<String>> {
        Ok(self.read(|tokens| tokens.user.clone()))
    }
}

/// The tokens of a platform sign-in once Apple accepted them, which it shows by answering with
/// the user's storefront. The check runs on `client`, a client of its own made for `tokens`, so a
/// binding keeps its sign-in when new tokens are refused.
pub(crate) async fn verified_tokens(
    client: impl FnOnce(Arc<dyn TokenSource>) -> AppleMusicClient,
    user_token: &str,
    developer_token: Option<&str>,
) -> Result<Tokens, SignInFailure> {
    let developer_token = developer_token
        .filter(|token| !token.is_empty())
        .ok_or(SignInFailure::NoDeveloperToken)?;
    let tokens = Tokens::new(developer_token, user_token).ok_or(SignInFailure::Rejected)?;
    let client = client(Arc::new(StaticTokens::new(
        developer_token,
        Some(user_token.to_owned()),
    )));
    match tokio_time::timeout(SIGN_IN_CHECK_TIMEOUT, client.user_storefront()).await {
        Ok(Ok(_)) => Ok(tokens),
        Ok(Err(error)) if rejects_tokens(&error) => Err(SignInFailure::Rejected),
        Ok(Err(
            AppleApiError::RateLimited { .. }
            | AppleApiError::Server { .. }
            | AppleApiError::Http(_),
        ))
        | Err(_) => Err(SignInFailure::Unreachable),
        Ok(Err(_)) => Err(SignInFailure::Protocol),
    }
}

/// What a collection id was listed with, so a page can name it without another read.
#[derive(Clone)]
pub(crate) struct CollectionInfo {
    pub(crate) name: String,
    pub(crate) cover: Option<String>,
}

/// The songs of a catalog album or playlist, as read, with when they were last used.
struct CollectionSongs {
    songs: Arc<Vec<Song>>,
    /// Where each song is in `songs`, by its catalog id.
    index: HashMap<String, usize>,
    used: u64,
}

/// The recommendation groups with the recommendations they were built from.
struct BuiltMixGroups {
    from: Arc<Vec<Recommendation>>,
    groups: Arc<MixGroups>,
}

/// An artist list with the lists it was built from.
struct BuiltNamedArtists {
    artists: Arc<Vec<LibraryArtist>>,
    songs: Option<Arc<AppleSongs>>,
    song_artists: Arc<SongArtists>,
    named: Arc<NamedArtists>,
}

/// What the binding knows besides its lists, for as long as it lives.
pub(crate) struct AppleCatalog {
    /// Apple's ids, numbered for the binding's lifetime.
    pub(crate) ids: RemoteIdMap<AppleIdKind>,
    infos: HashMap<(AppleIdKind, i32), CollectionInfo>,
    /// The song list as the binding last read it, for what must be answered without waiting.
    songs: Option<Arc<AppleSongs>>,
    /// Songs fetched one by one, by track.
    fetched: HashMap<String, AppleSong>,
    /// The tracks of songs Apple answered it does not have.
    missing: HashSet<String>,
    /// The songs of the catalog albums and playlists read lately.
    collection_songs: HashMap<CatalogCollection, CollectionSongs>,
    /// Counts the uses of the collections' songs, which tells the least recently used.
    uses: u64,
    /// The recommended mixes by id, as the recommendations read under this sign-in listed them.
    mixes: HashMap<i32, RecommendedMix>,
    /// The recommendation groups built last.
    mix_groups: Option<BuiltMixGroups>,
    /// The artist list built last.
    named_artists: Option<BuiltNamedArtists>,
    /// The lists asked for while the binding held no sign-in, which a sign-in reads again.
    owed: HashSet<&'static str>,
    /// What the catalog searches made lately under this sign-in found.
    searches: KeptSearches,
    /// Counts the sign-outs that dropped what was read. A read that began under an earlier
    /// count is not kept.
    generation: u64,
}

impl Default for AppleCatalog {
    fn default() -> Self {
        Self {
            ids: RemoteIdMap::sharing_numbers(AppleIdKind::numbering),
            infos: HashMap::new(),
            songs: None,
            fetched: HashMap::new(),
            missing: HashSet::new(),
            collection_songs: HashMap::new(),
            uses: 0,
            mixes: HashMap::new(),
            mix_groups: None,
            named_artists: None,
            owed: HashSet::new(),
            searches: KeptSearches::default(),
            generation: 0,
        }
    }
}

impl AppleCatalog {
    /// The kept songs of `collection`, which count as used now.
    fn used_collection_songs(&mut self, collection: CatalogCollection) -> Option<Arc<Vec<Song>>> {
        self.uses += 1;
        let used = self.uses;
        let kept = self.collection_songs.get_mut(&collection)?;
        kept.used = used;
        Some(Arc::clone(&kept.songs))
    }

    /// Keeps the songs of `collection`. The collections together hold a bounded number of
    /// songs: the least recently used go until these fit, and songs that are more than the
    /// bound are not kept.
    fn keep_collection_songs(&mut self, collection: CatalogCollection, songs: Arc<Vec<Song>>) {
        self.collection_songs.remove(&collection);
        if songs.len() > MAX_KEPT_COLLECTION_SONGS {
            return;
        }
        let mut held: usize = self
            .collection_songs
            .values()
            .map(|kept| kept.songs.len())
            .sum();
        while self.collection_songs.len() >= MAX_KEPT_COLLECTIONS
            || held + songs.len() > MAX_KEPT_COLLECTION_SONGS
        {
            let Some(oldest) = self
                .collection_songs
                .iter()
                .min_by_key(|(_, kept)| kept.used)
                .map(|(collection, _)| *collection)
            else {
                break;
            };
            if let Some(evicted) = self.collection_songs.remove(&oldest) {
                held -= evicted.songs.len();
            }
        }
        self.uses += 1;
        let index = songs
            .iter()
            .enumerate()
            .map(|(at, song)| (song.id.clone(), at))
            .collect();
        self.collection_songs.insert(
            collection,
            CollectionSongs {
                songs,
                index,
                used: self.uses,
            },
        );
    }

    /// A song of a kept collection, by its catalog id.
    fn collection_song(&self, catalog_id: &str) -> Option<&Song> {
        self.collection_songs
            .values()
            .find_map(|kept| Some(&kept.songs[*kept.index.get(catalog_id)?]))
    }
}

/// The lists an Apple Music library keeps, each read from Apple.
pub(crate) struct AppleLists {
    kept: ServiceLists,
    songs: KeptList<AppleSongs, AppleError>,
    albums: KeptList<Vec<LibraryAlbum>, AppleError>,
    artists: KeptList<Vec<LibraryArtist>, AppleError>,
    song_artists: KeptList<SongArtists, AppleError>,
    recommendations: KeptList<Vec<Recommendation>, AppleError>,
}

/// How a list is read from Apple by the binding.
type ReadList<T> = for<'a> fn(&'a AppleMusicBinding) -> BoxFuture<'a, Result<T, AppleError>>;

/// Declares the list `spec` names on `lists`, which `read` reads with the binding `this`.
fn declare<T>(
    lists: &ServiceLists,
    spec: ListSpec,
    this: &Weak<AppleMusicBinding>,
    read: ReadList<T>,
) -> KeptList<T, AppleError>
where
    T: Serialize + DeserializeOwned + PartialEq + Send + Sync + 'static,
{
    let this = this.clone();
    lists.declare(
        spec,
        move || {
            let this = this.clone();
            async move {
                let Some(binding) = this.upgrade() else {
                    return Err(AppleError::Unavailable("the library is gone".to_owned()));
                };
                read(&binding).await
            }
            .boxed()
        },
        // What a read brings is taken as each request asks for the list.
        |_| {},
    )
}

/// One Apple Music library record. The node keeps its binding for as long as the app runs, so
/// the sign-in survives opening other libraries. A binding is made at the first sign-in, or when
/// the library opens before one, as offline or while Apple does not answer: it then answers from
/// the lists kept on disk, and a read that needs Apple fails typed, until the platform signs it
/// in. The sign-in comes to the same binding, so the ids it gave keep their meaning.
pub(crate) struct AppleMusicBinding {
    /// The binding itself, for what it runs in the background.
    this: Weak<AppleMusicBinding>,
    pub(crate) library_id: Uuid,
    client: AppleMusicClient,
    tokens: Arc<SessionTokens>,
    lists: AppleLists,
    /// Rune's own playlists, kept on this device. Apple's own are never written to.
    pub(crate) rune_playlists: RunePlaylists,
    /// Rune's own mixes, kept on this device beside the mixes Apple recommends.
    pub(crate) rune_mixes: RuneMixes,
    /// The songs liked in Rune, kept on this device. Nothing is written to Apple.
    pub(crate) rune_likes: RuneLikes,
    /// What Rune's playlists, mixes and likes add to the library (see `apple_music_held`).
    pub(crate) held: HeldStore,
    /// Whether Rune's playlists, mixes and likes wait to be told whose they are (see
    /// `apple_music_account::attach`).
    pub(crate) records_held: AtomicBool,
    /// Where the files kept on this device are.
    pub(crate) config_dir: PathBuf,
    /// Whether what comes in from Apple once, its playlists and likes, is being imported into
    /// Rune (see `import_in_background`).
    importing: AtomicBool,
    /// Set when a sign-in came while an import was under way: the import runs once more when it
    /// ends, for that sign-in.
    import_again: AtomicBool,
    /// Counts the changes of the tokens: each sign-in whose tokens differ from those held, and
    /// each sign-out. `AppleCatalog::generation` counts sign-outs only, as what was read stays
    /// for new tokens; what runs for one sign-in, as the import, stops when this moves on.
    token_changes: AtomicU64,
    catalog: Mutex<AppleCatalog>,
    pub(crate) covers: Arc<ServiceCoverCache>,
    cover_source: Arc<AppleCovers>,
    /// Fixes which entries a random home group shows for a while.
    pub(crate) home_seed: u64,
    /// Whether the artist list waited once for the first read of the catalog artists of the
    /// library's songs.
    song_artists_waited: AtomicBool,
    /// Whether the device is online and, offline, which library songs it stores.
    reach: Mutex<Reach>,
    /// When the last change the UI is still to be told of came; none when it was told.
    content_change: Mutex<Option<tokio_time::Instant>>,
    /// The clock the kept lists age by.
    clock: Arc<dyn Clock>,
    /// The lyrics of the library's songs, which Apple gives Rune none of, looked up at LRCLIB as
    /// far as the user allowed it.
    lyrics: LibraryLyrics,
}

/// Downloads covers by their address. Apple serves them from a content network that needs no
/// sign-in, so no token goes with the request.
pub(crate) struct AppleCovers {
    pub(crate) http: reqwest::Client,
}

impl CoverSource for AppleCovers {
    /// A cover above the size cap comes back empty, which the cache does not keep. The body
    /// is never read past the cap.
    fn cover_bytes<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(async move {
            let mut response = self.http.get(url).send().await?.error_for_status()?;
            if response
                .content_length()
                .is_some_and(|length| length > MAX_COVER_BYTES as u64)
            {
                return Ok(Vec::new());
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if bytes.len() + chunk.len() > MAX_COVER_BYTES {
                    return Ok(Vec::new());
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
        })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl AppleMusicBinding {
    /// A binding whose lists answer from what was kept on disk in `config_dir`, before Apple
    /// does and before any sign-in. Its client, which `client` makes for the binding's tokens,
    /// reads Apple, what it keeps ages by `clock`, and its songs' lyrics are looked up at
    /// `lyrics` once the user allows it.
    ///
    /// The lists are kept for no account (`account: None`). This is a choice, made for a service
    /// that cannot tell its accounts apart: the platform hands over tokens at every launch, as
    /// it signs in by itself, and the device's Apple ID can change in the system's settings
    /// without Rune being told, so new tokens cannot be told from a sign-in of the same account.
    /// New tokens therefore keep the lists and read each one again in the background, and that
    /// read replaces it with what the account signed in has (see `sign_in`). A sign-out, or the
    /// removal of the record, clears them, on disk too. Clearing them at every new token instead
    /// would leave the library nothing to open from offline after every launch.
    pub(crate) fn new(
        library_id: Uuid,
        client: impl FnOnce(Arc<dyn TokenSource>) -> AppleMusicClient,
        config_dir: &Path,
        broadcaster: Arc<dyn Broadcaster>,
        clock: Arc<dyn Clock>,
        lyrics: Lrclib,
    ) -> Arc<Self> {
        prepare_service_library_folder(config_dir, library_id);
        let lyrics = LibraryLyrics::new(config_dir, library_id, lyrics, Arc::clone(&clock));
        let held = apple_music_account::prepare(config_dir, library_id);
        let records = device_records_folder(config_dir, library_id);
        let tokens = Arc::new(SessionTokens::default());
        let client = client(Arc::clone(&tokens) as Arc<dyn TokenSource>);
        Arc::new_cyclic(|this: &Weak<Self>| {
            let listener = this.clone();
            // Kept for no account: the doc comment above says why.
            let kept = ServiceLists::new(lists_folder(config_dir, library_id), None, move || {
                if let Some(binding) = listener.upgrade() {
                    binding.content_changed();
                }
            });
            let lists = AppleLists {
                songs: declare(&kept, SONGS, this, |binding| {
                    Box::pin(binding.read_song_list())
                }),
                albums: declare(&kept, ALBUMS, this, |binding| {
                    Box::pin(binding.read_album_list())
                }),
                artists: declare(&kept, ARTISTS, this, |binding| {
                    Box::pin(binding.read_artist_list())
                }),
                song_artists: declare(&kept, SONG_ARTISTS, this, |binding| {
                    Box::pin(binding.read_song_artists())
                }),
                recommendations: declare(&kept, RECOMMENDATIONS, this, |binding| {
                    Box::pin(binding.read_recommendations())
                }),
                kept,
            };
            Self {
                this: this.clone(),
                library_id,
                client,
                tokens,
                lists,
                rune_playlists: if held {
                    RunePlaylists::held(records.join(PLAYLISTS_FILE))
                } else {
                    RunePlaylists::new(records.join(PLAYLISTS_FILE))
                },
                rune_mixes: if held {
                    RuneMixes::held(records.join(MIXES_FILE))
                } else {
                    RuneMixes::new(records.join(MIXES_FILE))
                },
                rune_likes: if held {
                    RuneLikes::held(records.join(LIKES_FILE))
                } else {
                    RuneLikes::new(records.join(LIKES_FILE))
                },
                held: HeldStore::default(),
                records_held: AtomicBool::new(held),
                config_dir: config_dir.to_owned(),
                importing: AtomicBool::new(false),
                import_again: AtomicBool::new(false),
                token_changes: AtomicU64::new(0),
                catalog: Mutex::default(),
                covers: Arc::new(ServiceCoverCache::new(
                    cover_cache_root(config_dir, library_id),
                    broadcaster,
                )),
                cover_source: Arc::new(AppleCovers {
                    http: reqwest::Client::builder()
                        .timeout(Duration::from_secs(30))
                        .build()
                        .unwrap_or_default(),
                }),
                home_seed: rand::random(),
                song_artists_waited: AtomicBool::new(false),
                reach: Mutex::default(),
                content_change: Mutex::default(),
                clock,
                lyrics,
            }
        })
    }

    /// Hands over the tokens of a sign-in Apple accepted. New tokens cannot be told from a
    /// sign-in of the same account (see `new`), so they keep the lists, and read again in the
    /// background every list kept or asked for without a sign-in, which the read under the new
    /// tokens replaces. Any sign-in asks Apple again for the songs, albums and playlists it said
    /// it does not have, as an answer without them may have been a passing one.
    pub(crate) fn sign_in(&self, tokens: Tokens) {
        let changed = self.tokens.replace(Some(tokens));
        let owed = {
            let mut catalog = self.catalog();
            catalog.missing.clear();
            mem::take(&mut catalog.owed)
        };
        apple_music_held::signed_in(self);
        if changed {
            self.token_changes.fetch_add(1, Ordering::SeqCst);
            // The storefront belongs to the account, which may be another one.
            self.client.forget_storefront();
            self.read_again(owed);
        }
    }

    /// Starts importing Apple's playlists and likes into Rune's, as each sign-in does, which does
    /// nothing once each was imported on this device (see `apple_music_import`). A sign-in that
    /// comes while an import is under way does not start a second one: the running import stops
    /// at the change of the tokens, and runs once more when it ends, for the new sign-in.
    pub(crate) fn import_in_background(&self) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        // Set before the flag is taken, so a running import that is about to end sees it.
        self.import_again.store(true, Ordering::SeqCst);
        if self.importing.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(binding) = self.this.upgrade() else {
            self.import_again.store(false, Ordering::SeqCst);
            self.importing.store(false, Ordering::SeqCst);
            return;
        };
        tokio::spawn(async move {
            loop {
                binding.import_again.store(false, Ordering::SeqCst);
                apple_music_import::import(&binding).await;
                // A sign-in that came meanwhile has its import next, with no gap in which none
                // is under way.
                if binding.import_again.load(Ordering::SeqCst) {
                    continue;
                }
                binding.importing.store(false, Ordering::SeqCst);
                // One that came just now: take the flag back unless another task did.
                if !binding.import_again.load(Ordering::SeqCst)
                    || binding.importing.swap(true, Ordering::SeqCst)
                {
                    break;
                }
            }
        });
    }

    /// How many times the tokens changed (see `token_changes`), to take before what runs for
    /// one sign-in and to compare after.
    pub(crate) fn token_changes(&self) -> u64 {
        self.token_changes.load(Ordering::SeqCst)
    }

    /// Forgets the tokens and everything read with them, on disk too. The ids stay, so ids the
    /// UI holds keep their meaning for this binding.
    pub(crate) fn sign_out(&self) {
        if self.tokens.replace(None) {
            self.token_changes.fetch_add(1, Ordering::SeqCst);
            self.client.forget_storefront();
        }
        self.held.clear();
        self.lists.kept.clear(None);
        let mut catalog = self.catalog();
        catalog.infos.clear();
        catalog.songs = None;
        catalog.fetched.clear();
        catalog.missing.clear();
        catalog.collection_songs.clear();
        catalog.mixes.clear();
        catalog.mix_groups = None;
        catalog.named_artists = None;
        catalog.owed.clear();
        catalog.searches = KeptSearches::default();
        catalog.generation += 1;
    }

    /// Stops the covers still downloading, for good, and waits for one being written: the
    /// binding is going away with its record.
    pub(crate) async fn shut_down_covers(&self) {
        self.covers.close_and_wait_downloads().await;
    }

    /// Waits for a change to what the library keeps on this device being written, then refuses
    /// every later one: the binding is going away with its record, whose folder is deleted
    /// after.
    pub(crate) async fn close_device_records(&self) {
        self.rune_playlists.close().await;
        self.rune_mixes.close().await;
        self.rune_likes.close().await;
    }

    /// Whether the binding holds a sign-in.
    pub(crate) fn is_signed_in(&self) -> bool {
        self.tokens.signed_in()
    }

    /// The binding itself, for what it runs in the background.
    pub(crate) fn strong(&self) -> Option<Arc<Self>> {
        self.this.upgrade()
    }

    /// The time by the clock what the binding keeps ages by.
    pub(crate) fn now(&self) -> Instant {
        self.clock.now()
    }

    /// Keeps `value` beside the lists as `name`, soon. A sign-out deletes it with them.
    pub(crate) fn keep_beside_lists<T: Serialize + Send + Sync + 'static>(
        &self,
        name: &str,
        value: T,
    ) {
        self.lists.kept.keep(name, value);
    }

    /// What is kept beside the lists as `name`, if anything is (see `keep_beside_lists`).
    pub(crate) async fn kept_beside_lists<T: DeserializeOwned + Send + 'static>(
        &self,
        name: &'static str,
    ) -> Option<T> {
        let (config_dir, library_id) = (self.config_dir.clone(), self.library_id);
        tokio::task::spawn_blocking(move || kept_value(&config_dir, library_id, None, name))
            .await
            .ok()
            .flatten()
    }

    pub(crate) fn catalog(&self) -> MutexGuard<'_, AppleCatalog> {
        lock(&self.catalog)
    }

    /// The count to take before a read whose result is kept, and to hand back with it.
    pub(crate) fn generation(&self) -> u64 {
        self.catalog().generation
    }

    /// Reads again in the background, as Apple has them now, the lists that are kept and those
    /// `owed` names, which were asked for without a sign-in.
    fn read_again(&self, owed: HashSet<&'static str>) {
        let Some(binding) = self.this.upgrade() else {
            return;
        };
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        tokio::spawn(async move {
            let now = binding.clock.now();
            let lists = &binding.lists;
            let wanted = |name: &'static str, kept: bool| owed.contains(name) || kept;
            let (songs, albums, artists, song_artists, recommendations) = (
                wanted(SONGS.name, lists.songs.kept(now).await.is_some()),
                wanted(ALBUMS.name, lists.albums.kept(now).await.is_some()),
                wanted(ARTISTS.name, lists.artists.kept(now).await.is_some()),
                wanted(
                    SONG_ARTISTS.name,
                    lists.song_artists.kept(now).await.is_some(),
                ),
                wanted(
                    RECOMMENDATIONS.name,
                    lists.recommendations.kept(now).await.is_some(),
                ),
            );
            let read = async |wanted: bool, name: &str, read: BoxFuture<'_, Option<AppleError>>| {
                if wanted && let Some(error) = read.await {
                    log::info!("Could not read the Apple Music {name} after the sign-in: {error}");
                }
            };
            futures::join!(
                read(
                    songs,
                    SONGS.name,
                    lists.songs.current(now).map(Result::err).boxed()
                ),
                read(
                    albums,
                    ALBUMS.name,
                    lists.albums.current(now).map(Result::err).boxed()
                ),
                read(
                    artists,
                    ARTISTS.name,
                    lists.artists.current(now).map(Result::err).boxed()
                ),
                read(
                    song_artists,
                    SONG_ARTISTS.name,
                    lists.song_artists.current(now).map(Result::err).boxed(),
                ),
                read(
                    recommendations,
                    RECOMMENDATIONS.name,
                    lists.recommendations.current(now).map(Result::err).boxed(),
                ),
            );
        });
    }

    /// Notes that the list `name` was asked for without a sign-in, so a sign-in reads it again.
    fn note_asked(&self, name: &'static str) {
        if !self.is_signed_in() {
            self.catalog().owed.insert(name);
        }
    }

    /// The failure of an Apple Music call as a request failure, for something of which `what`
    /// says what it is when Apple does not have it. A binding that is not signed in yet fails a
    /// call that needs Apple as every call fails while the device is offline, and otherwise as
    /// one that waits for the sign-in. The binding keeps its tokens when Apple rejects them, since
    /// the app signs in again by itself.
    pub(crate) fn request_error(&self, error: AppleError, what: &'static str) -> anyhow::Error {
        // The user only learns that Apple Music is unavailable; the log says why.
        let unavailable = |why: &str| -> anyhow::Error {
            // Every request fails while the device is offline, which is no news.
            if self.is_online() {
                log::warn!("Apple Music is unavailable for a request: {why}");
            } else {
                log::debug!("Apple Music is unavailable for a request while offline: {why}");
            }
            ExpectedRequestError::Unavailable {
                what: "Apple Music",
            }
            .into()
        };
        match error {
            AppleError::SignedOut if !self.is_signed_in() && !self.is_online() => {
                unavailable("it is not signed in yet")
            }
            AppleError::SignedOut => ExpectedRequestError::ServiceSignInRequired {
                record_id: self.library_id.to_string(),
            }
            .into(),
            AppleError::NotFound => ExpectedError::NotFound { what }.into(),
            AppleError::Refused => unavailable("Apple refused the tokens"),
            AppleError::Unavailable(detail) => unavailable(&detail),
            AppleError::Rejected(detail) => {
                log::warn!("Apple Music rejected a request: {detail}");
                ExpectedRequestError::UnexpectedResponse {
                    expected: "successful Apple Music request".into(),
                    actual: format!("Apple Music rejected the request ({detail})"),
                }
                .into()
            }
        }
    }

    /// A failure of Apple's client as the binding tells it. A 401 or 403 means an expired
    /// sign-in only when a plain read with the same tokens fails too; otherwise Apple refused
    /// just this request.
    async fn judged(&self, error: AppleApiError) -> AppleError {
        if matches!(
            error,
            AppleApiError::Unauthorized | AppleApiError::Forbidden
        ) && matches!(self.client.user_storefront().await, Err(ref again) if rejects_tokens(again))
        {
            return AppleError::SignedOut;
        }
        classified(&error)
    }

    /// Runs a call that needs Apple: the sign-in is checked before anything is asked.
    async fn call<T>(
        &self,
        call: impl Future<Output = apple_music::Result<T>>,
    ) -> Result<T, AppleError> {
        if !self.is_signed_in() {
            return Err(AppleError::SignedOut);
        }
        match call.await {
            Ok(value) => Ok(value),
            Err(error) => Err(self.judged(error).await),
        }
    }

    /// The library's songs, as kept, or else read, with the songs Rune holds in the library.
    pub(crate) async fn songs(&self) -> Result<Arc<AppleSongs>> {
        self.note_asked(SONGS.name);
        let generation = self.generation();
        let own = self
            .lists
            .songs
            .get(self.clock.now())
            .await
            .map_err(|error| self.request_error(error, "Apple Music song"))?;
        Ok(self.with_held_songs(own, generation).await)
    }

    /// The library's songs as Apple lists them now, joining a read under way or starting one,
    /// with the songs Rune holds in the library.
    pub(crate) async fn current_songs(&self) -> Result<Arc<AppleSongs>> {
        self.note_asked(SONGS.name);
        let generation = self.generation();
        let own = self
            .lists
            .songs
            .current(self.clock.now())
            .await
            .map_err(|error| self.request_error(error, "Apple Music song"))?;
        Ok(self.with_held_songs(own, generation).await)
    }

    /// `own`, the library's songs as Apple lists them, with the songs Rune holds in the library
    /// after them (see `apple_music_held`), each on the library's album of its catalog album as
    /// the albums are kept. This is the song list the binding knows from then on, unless the
    /// account signed out since `generation`, taken before `own` was.
    async fn with_held_songs(&self, own: Arc<AppleSongs>, generation: u64) -> Arc<AppleSongs> {
        let held = apple_music_held::held(self).await;
        // A look at the kept albums reads them again in the background once they are old, so they
        // are looked at only when Rune holds songs.
        let albums = if held.is_empty() {
            None
        } else {
            self.lists.albums.kept(self.clock.now()).await
        };
        let songs = self.held.songs_with(own, &held, albums);
        let mut catalog = self.catalog();
        if catalog.generation == generation {
            catalog.songs = Some(Arc::clone(&songs));
        }
        songs
    }

    /// `own`, the library's albums as Apple lists them, with the albums Rune holds after them
    /// (see `apple_music_held`), of the song list as kept.
    async fn with_held_albums(&self, own: Arc<Vec<LibraryAlbum>>) -> Arc<Vec<LibraryAlbum>> {
        if apple_music_held::held(self).await.is_empty() {
            return own;
        }
        let songs = self.kept_songs().await;
        self.held.albums_with(own, songs)
    }

    /// The song list as the binding last had it, without asking Apple or the disk.
    pub(crate) fn known_songs(&self) -> Option<Arc<AppleSongs>> {
        self.catalog().songs.clone()
    }

    /// The library's albums, as kept, or else read, with the albums Rune holds.
    pub(crate) async fn albums(&self) -> Result<Arc<Vec<LibraryAlbum>>> {
        self.note_asked(ALBUMS.name);
        let own = self
            .lists
            .albums
            .get(self.clock.now())
            .await
            .map_err(|error| self.request_error(error, "Apple Music album"))?;
        Ok(self.with_held_albums(own).await)
    }

    /// The library's artists, as kept, or else read.
    pub(crate) async fn artists(&self) -> Result<Arc<Vec<LibraryArtist>>> {
        self.note_asked(ARTISTS.name);
        self.lists
            .artists
            .get(self.clock.now())
            .await
            .map_err(|error| self.request_error(error, "Apple Music artist"))
    }

    /// The library's songs as kept, here or on disk, without a read; read again behind once old.
    /// The songs Rune holds in the library come with them.
    pub(crate) async fn kept_songs(&self) -> Option<Arc<AppleSongs>> {
        let generation = self.generation();
        let own = self.lists.songs.kept(self.clock.now()).await?;
        Some(self.with_held_songs(own, generation).await)
    }

    /// The library's albums as kept, here or on disk, without a read; read again behind once
    /// old. The albums Rune holds come with them.
    pub(crate) async fn kept_albums(&self) -> Option<Arc<Vec<LibraryAlbum>>> {
        let own = self.lists.albums.kept(self.clock.now()).await?;
        Some(self.with_held_albums(own).await)
    }

    /// What Apple recommends to the user, as kept, or else read.
    pub(crate) async fn recommendations(&self) -> Result<Arc<Vec<Recommendation>>> {
        self.note_asked(RECOMMENDATIONS.name);
        self.lists
            .recommendations
            .get(self.clock.now())
            .await
            .map_err(|error| self.request_error(error, "Apple Music recommendation"))
    }

    /// Reads what Apple recommends to the user.
    async fn read_recommendations(&self) -> Result<Vec<Recommendation>, AppleError> {
        self.call(self.client.recommendations()).await
    }

    /// The recommendation groups built from `from`, if they were built last under this
    /// sign-in.
    pub(crate) fn built_mix_groups(
        &self,
        from: &Arc<Vec<Recommendation>>,
    ) -> Option<Arc<MixGroups>> {
        self.catalog()
            .mix_groups
            .as_ref()
            .filter(|built| Arc::ptr_eq(&built.from, from))
            .map(|built| Arc::clone(&built.groups))
    }

    /// Keeps the recommendation groups built from `from` and their mixes, by a build that began
    /// under `generation`, unless a sign-out dropped what was read since. The mixes of earlier
    /// builds stay until the binding holds too many; then only the ones of this build do.
    pub(crate) fn keep_recommendations(
        &self,
        generation: u64,
        from: &Arc<Vec<Recommendation>>,
        groups: MixGroups,
        mixes: HashMap<i32, RecommendedMix>,
    ) -> Arc<MixGroups> {
        let groups = Arc::new(groups);
        let mut catalog = self.catalog();
        if catalog.generation != generation {
            return groups;
        }
        let added = mixes
            .keys()
            .filter(|id| !catalog.mixes.contains_key(id))
            .count();
        if catalog.mixes.len() + added > MAX_KEPT_MIXES {
            catalog.mixes.clear();
        }
        catalog.mixes.extend(mixes);
        catalog.mix_groups = Some(BuiltMixGroups {
            from: Arc::clone(from),
            groups: Arc::clone(&groups),
        });
        groups
    }

    /// A recommended mix a build of the recommendations listed under this sign-in.
    pub(crate) fn recommended_mix(&self, id: i32) -> Option<RecommendedMix> {
        self.catalog().mixes.get(&id).cloned()
    }

    /// The songs of the catalog album or playlist `collection`, whose catalog id is
    /// `catalog_id`: as kept, unless `afresh` asks for them as Apple has them now or none are
    /// kept, which reads them. An album's come by disc and number. What is read is kept, unless a
    /// sign-out came meanwhile, and the library takes it in when Rune holds the collection (see
    /// `apple_music_held::collection_read`).
    pub(crate) async fn catalog_collection_songs(
        &self,
        collection: CatalogCollection,
        catalog_id: &str,
        afresh: bool,
    ) -> Result<Arc<Vec<Song>>> {
        if !afresh && let Some(songs) = self.catalog().used_collection_songs(collection) {
            return Ok(songs);
        }
        let generation = self.generation();
        let held = self.held.generation();
        let songs = match collection {
            CatalogCollection::Album(_) => self
                .call(self.client.catalog_album_tracks(catalog_id))
                .await
                .map(by_disc_and_number)
                .map_err(|error| self.request_error(error, "Apple Music album")),
            CatalogCollection::Playlist(_) => self
                .call(self.client.catalog_playlist_tracks(catalog_id))
                .await
                .map_err(|error| self.request_error(error, "Apple Music playlist")),
        };
        let songs = Arc::new(songs?);
        {
            let mut catalog = self.catalog();
            if catalog.generation == generation {
                catalog.keep_collection_songs(collection, Arc::clone(&songs));
            }
        }
        apple_music_held::collection_read(self, held, collection, catalog_id, &songs);
        Ok(songs)
    }

    /// The catalog songs `ids`, read from Apple with their catalog albums. A song Apple does not
    /// have is absent from the answer.
    pub(crate) async fn read_catalog_songs_with_albums(
        &self,
        ids: &[String],
    ) -> Result<Vec<Song>, AppleError> {
        self.call(self.client.catalog_songs_with_albums(ids)).await
    }

    /// What Apple's search of the library finds for `term`: songs, albums and artists, as many
    /// of each as Apple lists at once. Apple's 404 is an answer that it found nothing.
    pub(crate) async fn search_library(&self, term: &str) -> Result<LibraryFound, AppleError> {
        let types = [
            LibrarySearchType::Songs,
            LibrarySearchType::Albums,
            LibrarySearchType::Artists,
        ];
        match self
            .call(
                self.client
                    .search_library(term, &types, LIBRARY_SEARCH_LIMIT, 0),
            )
            .await
        {
            Ok(found) => Ok(LibraryFound {
                songs: found.songs.data.into_iter().map(kept_song).collect(),
                albums: found.albums.data,
                artists: found.artists.data,
            }),
            Err(AppleError::NotFound) => Ok(LibraryFound::default()),
            Err(error) => Err(error),
        }
    }

    /// What Apple's search of the catalog finds for `term`: songs, albums and artists, as many
    /// of each as Apple lists at once. Apple's 404 is an answer that it found nothing.
    pub(crate) async fn search_catalog(&self, term: &str) -> Result<CatalogFound, AppleError> {
        let types = [
            CatalogSearchType::Songs,
            CatalogSearchType::Albums,
            CatalogSearchType::Artists,
        ];
        match self
            .call(
                self.client
                    .search_catalog(term, &types, CATALOG_SEARCH_LIMIT, 0),
            )
            .await
        {
            Ok(found) => Ok(CatalogFound {
                songs: found.songs.data,
                albums: found.albums.data,
                artists: found.artists.data,
            }),
            Err(AppleError::NotFound) => Ok(CatalogFound::default()),
            Err(error) => Err(error),
        }
    }

    /// What a catalog search for the term of `key` found lately under this sign-in.
    pub(crate) fn kept_search(&self, key: &str) -> Option<Arc<CatalogFound>> {
        self.catalog().searches.fresh(key, self.now())
    }

    /// Keeps what a catalog search for the term of `key`, begun under `generation`, found.
    pub(crate) fn keep_search(&self, generation: u64, key: String, found: Arc<CatalogFound>) {
        let now = self.now();
        let mut catalog = self.catalog();
        if catalog.generation == generation {
            catalog.searches.keep(key, now, found);
        }
    }

    /// Remembers the songs a search that began under `generation` found, by their tracks, so they
    /// are named without asking Apple again. A song the binding holds already stays as it is: a
    /// search names no album or artist of a song, which an earlier read may have.
    pub(crate) fn remember_found(&self, generation: u64, found: &[(String, Option<AppleSong>)]) {
        let mut catalog = self.catalog();
        if catalog.generation != generation {
            return;
        }
        let holds = |catalog: &AppleCatalog, track: &str| {
            catalog
                .songs
                .as_ref()
                .is_some_and(|list| list.get(track).is_some())
                || catalog.fetched.contains_key(track)
        };
        let new: Vec<(&String, &AppleSong)> = found
            .iter()
            .filter_map(|(track, song)| Some((track, song.as_ref()?)))
            .filter(|(track, _)| !holds(&catalog, track))
            .collect();
        // The cache only spares Apple a repeat; when it is full it starts over.
        if catalog.fetched.len() + new.len() > MAX_FETCHED_TRACKS {
            catalog.fetched.clear();
        }
        for (track, song) in new {
            catalog.fetched.insert(track.clone(), song.clone());
            catalog.missing.remove(track);
        }
    }

    /// `song`, the song of `track`, with the catalog album Apple names for it when it is a
    /// catalog song that names none yet, as one a catalog search found does not. The album is
    /// read once and kept with the song. A song Apple names no album for, or a read that fails,
    /// stays as it is.
    pub(crate) async fn with_catalog_album(&self, track: &str, song: AppleSong) -> AppleSong {
        let AppleSong::Catalog(mut catalog_song) = song else {
            return song;
        };
        if catalog_song.album_id().is_some() {
            return AppleSong::Catalog(catalog_song);
        }
        let generation = self.generation();
        let read = self
            .read_catalog_songs_with_albums(std::slice::from_ref(&catalog_song.id))
            .await;
        let albums = match read {
            Ok(read) => read
                .into_iter()
                .find(|read| read.id == catalog_song.id)
                .and_then(|read| read.relationships?.get("albums").cloned()),
            Err(error) => {
                log::warn!("The album of an Apple Music catalog song could not be read: {error}");
                None
            }
        };
        let Some(albums) = albums else {
            return AppleSong::Catalog(catalog_song);
        };
        // Only the album: the song's credit keeps naming the artist it names.
        match catalog_song
            .relationships
            .as_mut()
            .and_then(Value::as_object_mut)
        {
            Some(relationships) => {
                relationships.insert("albums".to_owned(), albums);
            }
            None => catalog_song.relationships = Some(serde_json::json!({ "albums": albums })),
        }
        let song = AppleSong::Catalog(catalog_song);
        let mut catalog = self.catalog();
        if catalog.generation == generation {
            if catalog.fetched.len() >= MAX_FETCHED_TRACKS {
                catalog.fetched.clear();
            }
            catalog.fetched.insert(track.to_owned(), song.clone());
        }
        song
    }

    /// The catalog album `catalog_id`, read from Apple.
    pub(crate) async fn read_catalog_album(&self, catalog_id: &str) -> Result<Album> {
        self.call(self.client.catalog_album(catalog_id))
            .await
            .map_err(|error| self.request_error(error, "Apple Music album"))
    }

    /// The catalog playlist `catalog_id`, read from Apple.
    pub(crate) async fn read_catalog_playlist(&self, catalog_id: &str) -> Result<Playlist> {
        self.call(self.client.catalog_playlist(catalog_id))
            .await
            .map_err(|error| self.request_error(error, "Apple Music playlist"))
    }

    /// Reads the user's songs: the library's songs, then the songs of the user's own playlists
    /// and Favorites that are not in the library. A catalog song the library holds a copy of is
    /// listed as that copy, also when it comes before the copy.
    async fn read_song_list(&self) -> Result<AppleSongs, AppleError> {
        if !self.is_signed_in() {
            return Err(AppleError::SignedOut);
        }
        let mut list = AppleSongs::default();
        // Library songs whose catalog id only their `catalog` relationship names.
        let mut unresolved = Vec::new();
        let mut catalog_named = Vec::new();
        let library = async {
            let mut pages = self.client.library_song_pages(SONG_INCLUDE, PARALLEL_READS);
            while let Some(page) = pages.try_next().await? {
                for entry in page {
                    match (is_library_song(&entry), entry.store_id()) {
                        (true, Some(catalog_id)) => list.add_twin(catalog_id, &entry.id),
                        (true, None) => unresolved.push(entry.id.clone()),
                        (false, _) => {}
                    }
                    // Entries named by their catalog id wait for the twins of every page, so
                    // none is listed beside the library copy that comes after it.
                    if is_library_song(&entry) {
                        list.push(entry);
                    } else {
                        catalog_named.push(entry);
                    }
                }
            }
            Ok::<_, AppleApiError>(())
        };
        let (library, playlists) = join(
            library,
            self.client
                .library_playlists()
                .try_collect::<Vec<LibraryPlaylist>>(),
        )
        .await;
        if let Err(error) = library {
            return Err(self.judged(error).await);
        }
        let mut unread = None;
        // Playlists Apple cannot list are as a playlist it cannot read: they do not cost a first
        // song list either.
        let playlists = match playlists {
            Ok(playlists) => playlists,
            Err(error) => {
                let error = self.playlist_failure(error).await?;
                log::warn!("Could not list the Apple Music playlists for the song list: {error}");
                unread = Some(error);
                Vec::new()
            }
        };

        // Owned ids: closures taking borrowed items here fail the compiler's Send check.
        let joining: Vec<String> = playlists
            .into_iter()
            .filter(joins_song_list)
            .map(|playlist| playlist.id)
            .collect();
        let read: Vec<_> = stream::iter(joining)
            .map(|id| async move { self.playlist_songs(&id).await })
            .buffered(PARALLEL_READS)
            .collect()
            .await;
        let mut extra = Vec::new();
        for songs in read {
            let error = match songs {
                Ok(songs) => {
                    extra.extend(songs);
                    continue;
                }
                Err(error) => self.playlist_failure(error).await?,
            };
            // One playlist that cannot be read does not cost a first song list.
            log::warn!("Skipped an Apple Music playlist that could not be read: {error}");
            unread = unread.or(Some(error));
        }
        // A list without the songs only that playlist holds must not replace one that has
        // them: a failed read is not a removal. The kept list is served until a read works.
        if let Some(error) = unread
            && self.lists.songs.kept(self.clock.now()).await.is_some()
        {
            return Err(error);
        }

        let catalog_copies = extra
            .iter()
            .chain(&catalog_named)
            .any(|song| !is_library_song(song));
        if catalog_copies && !unresolved.is_empty() {
            // Best effort: without these ids a catalog copy of such a song is listed too.
            if let Ok(songs) = self
                .client
                .library_songs_by_ids(&unresolved, &["catalog"])
                .await
            {
                for song in &songs {
                    if let (true, Some(catalog_id)) = (is_library_song(song), song.store_id()) {
                        list.add_twin(catalog_id, &song.id);
                    }
                }
            }
        }
        // Their catalog ids first, so the catalog copy of such a song is not listed too.
        for song in &extra {
            if let (true, Some(catalog_id)) = (is_library_song(song), song.store_id()) {
                list.add_twin(catalog_id, &song.id);
            }
        }
        // Entries named by their catalog id are listed once the twins are all known.
        for song in catalog_named.into_iter().chain(extra) {
            list.push(song);
        }
        Ok(list)
    }

    /// What a failed read of the playlists for the song list is: an expired sign-in stops the
    /// whole read, as its error; Apple refusing the playlists alone does not, and is answered.
    async fn playlist_failure(&self, error: AppleApiError) -> Result<AppleError, AppleError> {
        if !rejects_tokens(&error) {
            return Ok(classified(&error));
        }
        match self.judged(error).await {
            AppleError::SignedOut => Err(AppleError::SignedOut),
            refused => Ok(refused),
        }
    }

    /// The songs of a library playlist; none for an empty one.
    async fn playlist_songs(&self, id: &str) -> apple_music::Result<Vec<LibrarySong>> {
        match self.client.library_playlist_tracks(id).try_collect().await {
            Ok(tracks) => Ok(tracks),
            // Apple answers 404 for the tracks of an empty playlist.
            Err(AppleApiError::NotFound(_)) => Ok(Vec::new()),
            Err(error) => Err(error),
        }
    }

    /// Reads every album of the library: recently added ones first, in Apple's order, then the
    /// rest.
    async fn read_album_list(&self) -> Result<Vec<LibraryAlbum>, AppleError> {
        if !self.is_signed_in() {
            return Err(AppleError::SignedOut);
        }
        let library = async {
            let mut library = Vec::new();
            // A library album names its catalog album only through the catalog relationship.
            let mut pages = self
                .client
                .library_album_pages(&["artists", "catalog"], PARALLEL_READS);
            while let Some(page) = pages.try_next().await? {
                library.extend(page.into_iter().map(kept_album));
            }
            Ok::<_, AppleApiError>(library)
        };
        let (recent, library) = join(
            self.client.recently_added().try_collect::<Vec<_>>(),
            library,
        )
        .await;
        let library = match library {
            Ok(library) => library,
            Err(error) => return Err(self.judged(error).await),
        };
        // Recently added only orders the list; without it the list is still complete.
        let recent = recent.unwrap_or_default();
        let by_id: HashMap<&str, &LibraryAlbum> = library
            .iter()
            .map(|album| (album.id.as_str(), album))
            .collect();
        let mut seen = HashSet::new();
        let mut albums = Vec::with_capacity(library.len());
        for entry in recent {
            let LibraryEntry::Album(album) = entry else {
                continue;
            };
            if seen.insert(album.id.clone()) {
                albums.push(match by_id.get(album.id.as_str()) {
                    Some(known) => (*known).clone(),
                    None => kept_album(album),
                });
            }
        }
        albums.extend(
            library
                .iter()
                .filter(|album| seen.insert(album.id.clone()))
                .cloned(),
        );
        Ok(albums)
    }

    /// Reads the library's artists, with the catalog artists Apple maps them to.
    async fn read_artist_list(&self) -> Result<Vec<LibraryArtist>, AppleError> {
        self.call(self.client.library_artists().try_collect()).await
    }

    /// The catalog artists of the library's songs: those kept, and those of the songs not asked
    /// about yet, read from Apple as many at once as it takes, `SONG_ARTIST_READS` batches at a
    /// time. A batch that fails is logged; its songs are asked about again by a later read. It
    /// fails only when nothing is kept and Apple cannot be asked: what was read is kept, and the
    /// artist list stands the songs not asked about under their whole credits.
    async fn read_song_artists(&self) -> Result<SongArtists, AppleError> {
        let now = self.clock.now();
        // This read is the one in flight, so asking for the kept list starts no other.
        let mut read = self
            .lists
            .song_artists
            .kept(now)
            .await
            .map(|kept| (*kept).clone())
            .unwrap_or_default();
        if !self.is_signed_in() {
            return Err(AppleError::SignedOut);
        }
        let generation = self.generation();
        let Ok(own) = self.lists.songs.get(now).await else {
            return Ok(read);
        };
        let songs = self.with_held_songs(own, generation).await;
        // Owned batches: closures taking borrowed items here fail the compiler's Send check.
        let batches: Vec<Vec<String>> = read
            .missing(songs.library_entries().map(|(_, song)| song))
            .chunks(CATALOG_SONGS_BATCH)
            .map(<[String]>::to_vec)
            .collect();
        let answers: Vec<_> = stream::iter(batches)
            .map(|batch| async move {
                let answer = self.client.catalog_songs_with_artists(&batch).await;
                (batch, answer)
            })
            .buffered(SONG_ARTIST_READS)
            .collect()
            .await;
        for (batch, answer) in answers {
            match answer {
                Ok(found) => read.add(&batch, found.into_iter().map(song_artists).collect()),
                Err(error) => log::warn!(
                    "Could not read the catalog artists of {} Apple Music songs, naming them by their credits: {}",
                    batch.len(),
                    apple_detail(&error)
                ),
            }
        }
        Ok(read)
    }

    /// The library's artist list, built from the artists and the song list as kept, or else
    /// read. Without the song list it is built from the artists alone.
    pub(crate) async fn named_artists(&self) -> Result<Arc<NamedArtists>> {
        let artists = self.artists().await?;
        let songs = self.songs().await.ok();
        let song_artists = self.song_artists().await;
        Ok(self.built_named_artists(artists, songs, song_artists))
    }

    /// The artist list of these lists: the one built last when it was built from them, or else
    /// built now and kept.
    fn built_named_artists(
        &self,
        artists: Arc<Vec<LibraryArtist>>,
        songs: Option<Arc<AppleSongs>>,
        song_artists: Arc<SongArtists>,
    ) -> Arc<NamedArtists> {
        let generation = {
            let catalog = self.catalog();
            if let Some(built) = &catalog.named_artists
                && Arc::ptr_eq(&built.artists, &artists)
                && Arc::ptr_eq(&built.song_artists, &song_artists)
                && match (&built.songs, &songs) {
                    (Some(built), Some(songs)) => Arc::ptr_eq(built, songs),
                    (None, None) => true,
                    _ => false,
                }
            {
                return Arc::clone(&built.named);
            }
            catalog.generation
        };
        let library = songs
            .as_deref()
            .into_iter()
            .flat_map(AppleSongs::library_entries)
            .map(|(_, song)| song);
        let named = Arc::new(NamedArtists::new(&artists, library, &song_artists));
        let mut catalog = self.catalog();
        if catalog.generation == generation {
            catalog.named_artists = Some(BuiltNamedArtists {
                artists,
                songs,
                song_artists,
                named: Arc::clone(&named),
            });
        }
        named
    }

    /// The catalog artists Apple names for the library's songs, as kept, read again behind
    /// once old, as the other lists are. When none are kept, they are read behind; the first time
    /// the artist list is asked for, it waits for them at most `FIRST_SONG_ARTISTS_WAIT`, and
    /// otherwise not at all, so a large library still answers in time. Until they are kept, there
    /// are none, and the artist list stands the songs under their whole credits; once they are,
    /// the lists that show the library reload.
    async fn song_artists(&self) -> Arc<SongArtists> {
        self.note_asked(SONG_ARTISTS.name);
        let now = self.clock.now();
        if let Some(kept) = self.lists.song_artists.kept(now).await {
            return kept;
        }
        if self.is_signed_in() && !self.song_artists_waited.swap(true, Ordering::SeqCst) {
            match tokio_time::timeout(FIRST_SONG_ARTISTS_WAIT, self.lists.song_artists.get(now))
                .await
            {
                Ok(Ok(read)) => return read,
                Ok(Err(_)) => return Arc::default(),
                Err(_) => {}
            }
        }
        // Read behind, and the lists that show the library reload once it brings them.
        if let Some(binding) = self.this.upgrade() {
            tokio::spawn(async move {
                if binding.lists.song_artists.get(now).await.is_ok() {
                    binding.content_changed();
                }
            });
        }
        Arc::default()
    }

    /// The library's artist list as built from the lists kept, here or on disk, without a read.
    /// None when the artists are not kept.
    pub(crate) async fn kept_named_artists(&self) -> Option<Arc<NamedArtists>> {
        let now = self.clock.now();
        let artists = self.lists.artists.kept(now).await?;
        let songs = self.kept_songs().await;
        let song_artists = self.lists.song_artists.kept(now).await.unwrap_or_default();
        Some(self.built_named_artists(artists, songs, song_artists))
    }

    /// The catalog artists Apple names for the library's songs, as kept, read again behind once
    /// old; none are read when none are kept.
    pub(crate) async fn kept_song_artists(&self) -> Option<Arc<SongArtists>> {
        self.lists.song_artists.kept(self.clock.now()).await
    }

    /// The id of the artist of the library's artist list that the id `id` of `kind` stands
    /// for, as it was numbered; `None` for any other id. Nothing is read.
    pub(crate) fn named_artist_key(&self, kind: AppleIdKind, id: i32) -> Option<String> {
        if kind != AppleIdKind::Artist {
            return None;
        }
        let catalog = self.catalog();
        let remote = catalog.ids.resolve(kind, id).ok()?;
        is_named_artist(remote).then(|| remote.to_owned())
    }

    /// The artist of the library's artist list that the id `remote` names, as the list holds
    /// it now. One the list no longer holds is not found.
    pub(crate) async fn named_artist(&self, remote: &str) -> Result<NamedArtist> {
        let named = self.named_artists().await?;
        Ok(named.get(remote).cloned().ok_or(ExpectedError::NotFound {
            what: "Apple Music artist",
        })?)
    }

    /// The songs of a library album, read from Apple, by disc and number. Each names the album,
    /// and takes its cover when it has none of its own.
    pub(crate) async fn read_album_songs(&self, id: &str) -> Result<Vec<LibrarySong>> {
        let read = self
            .call(try_join(
                self.client.library_album(id, &["artists"]),
                self.client.library_album_tracks(id).try_collect::<Vec<_>>(),
            ))
            .await;
        let (album, songs) =
            read.map_err(|error| self.request_error(error, "Apple Music album"))?;
        let artwork = album
            .attributes
            .as_ref()
            .and_then(|attributes| attributes.artwork.clone());
        let mut songs: Vec<LibrarySong> = songs
            .into_iter()
            .filter(|song| !song.is_music_video())
            .map(|mut song| {
                let relationships = song.relationships.get_or_insert_with(Default::default);
                if relationships.albums.is_none() {
                    relationships.albums = Some(Relationship {
                        href: None,
                        next: None,
                        data: vec![Resource {
                            id: album.id.clone(),
                            kind: "library-albums".to_owned(),
                            href: None,
                            attributes: None,
                            relationships: None,
                        }],
                    });
                }
                if let Some(attributes) = &mut song.attributes
                    && attributes.artwork.is_none()
                {
                    attributes.artwork.clone_from(&artwork);
                }
                kept_song(song)
            })
            .collect();
        songs.sort_by_key(|song| {
            let attributes = song.attributes.as_ref();
            (
                attributes
                    .and_then(|attributes| attributes.disc_number)
                    .unwrap_or_default(),
                attributes
                    .and_then(|attributes| attributes.track_number)
                    .unwrap_or_default(),
            )
        });
        Ok(songs)
    }

    /// The albums of a library artist, read from Apple.
    pub(crate) async fn read_artist_albums(&self, id: &str) -> Result<Vec<LibraryAlbum>> {
        self.call(self.client.library_artist_albums(id).try_collect())
            .await
            .map_err(|error| self.request_error(error, "Apple Music artist"))
    }

    /// Whether a song can play now. Online every song can. Offline only a library song stored
    /// on this device can, and a catalog song never.
    pub(crate) fn audio_availability(&self, track: &str) -> AudioAvailability {
        let stored = match song_of_track(track) {
            Some((AppleIdKind::LibrarySong, id)) => Some(id),
            _ => None,
        };
        lock(&self.reach).availability(self.availability_follows_connectivity(), stored)
    }

    /// Whether the device is online, as the host last reported.
    pub(crate) fn is_online(&self) -> bool {
        lock(&self.reach).is_online()
    }

    /// Tells the UI to reload the lists that show the library's contents, once the changes come
    /// to rest: `CONTENT_CHANGED_DELAY` after the last of them, so a burst is told once. Without
    /// a runtime to wait on, it tells at once.
    pub(crate) fn content_changed(&self) {
        let waiting = lock(&self.content_change)
            .replace(tokio_time::Instant::now())
            .is_some();
        if waiting {
            return;
        }
        let binding = self
            .this
            .upgrade()
            .filter(|_| tokio::runtime::Handle::try_current().is_ok());
        let Some(binding) = binding else {
            *lock(&self.content_change) = None;
            self.tell_content_changed();
            return;
        };
        tokio::spawn(async move {
            loop {
                let due = {
                    let mut last = lock(&binding.content_change);
                    let Some(due) = last.map(|last| last + CONTENT_CHANGED_DELAY) else {
                        return;
                    };
                    // A change that comes once this is cleared waits for a notice of its own.
                    if tokio_time::Instant::now() >= due {
                        *last = None;
                        None
                    } else {
                        Some(due)
                    }
                };
                match due {
                    Some(due) => tokio_time::sleep_until(due).await,
                    None => {
                        binding.tell_content_changed();
                        return;
                    }
                }
            }
        });
    }

    fn tell_content_changed(&self) {
        self.covers.broadcaster.broadcast(&LibraryContentChanged {
            path: String::new(),
        });
    }

    pub(crate) fn intern(&self, kind: AppleIdKind, remote_id: &str) -> Result<i32> {
        self.catalog().ids.intern(kind, remote_id)
    }

    pub(crate) fn resolve(&self, kind: AppleIdKind, id: i32) -> Result<String> {
        Ok(self.catalog().ids.resolve(kind, id)?.to_owned())
    }

    /// The id of the song `track` names, numbered if it was not.
    pub(crate) fn intern_track(&self, track: &str) -> Result<i32> {
        let (kind, id) = parse_track(track)?;
        self.intern(kind, id)
    }

    /// The track a media file id names, of a song of either kind.
    pub(crate) fn resolve_track(&self, id: i32) -> Result<String> {
        let catalog = self.catalog();
        match catalog.ids.resolve(AppleIdKind::LibrarySong, id) {
            Ok(library) => Ok(library.to_owned()),
            Err(error) => match catalog.ids.resolve(AppleIdKind::CatalogSong, id) {
                Ok(catalog_id) => Ok(catalog_track(catalog_id)),
                Err(_) => Err(error),
            },
        }
    }

    /// Remembers what a collection was listed with, by a read that began under `generation`.
    /// A listing without a cover keeps the cover an earlier one gave.
    pub(crate) fn remember_collection(
        &self,
        generation: u64,
        kind: AppleIdKind,
        id: i32,
        name: &str,
        artwork: Option<&Artwork>,
    ) {
        let mut catalog = self.catalog();
        if catalog.generation != generation {
            return;
        }
        let cover = cover_url(artwork).or_else(|| {
            catalog
                .infos
                .get(&(kind, id))
                .and_then(|info| info.cover.clone())
        });
        catalog.infos.insert(
            (kind, id),
            CollectionInfo {
                name: name.to_owned(),
                cover,
            },
        );
    }

    pub(crate) fn collection_info(&self, kind: AppleIdKind, id: i32) -> Option<CollectionInfo> {
        self.catalog().infos.get(&(kind, id)).cloned()
    }

    /// Reads the songs of `tracks` that the binding does not hold yet, all of them, in chunks, so
    /// that the queue can name them. Answers the songs it read, by their tracks: not those the
    /// binding held before, not those Apple does not have, and nothing when a read fails (which
    /// leaves them unnamed) or the sign-in was dropped meanwhile. The songs are also kept for
    /// later, as far as the cache holds them; the answer is complete whatever the cache holds.
    /// Those Apple does not answer with are remembered as missing, and are not asked for again.
    pub(crate) async fn read_unknown_tracks(
        &self,
        tracks: impl IntoIterator<Item = String>,
    ) -> HashMap<String, AppleSong> {
        let mut seen = HashSet::new();
        let wanted: Vec<String> = tracks
            .into_iter()
            .filter(|track| {
                song_of_track(track).is_some()
                    && seen.insert(track.clone())
                    && !self.known_missing(track)
                    && self.known_song(track).is_none()
            })
            .collect();
        let mut read = HashMap::new();
        if wanted.is_empty() {
            return read;
        }
        let generation = self.generation();
        for chunk in wanted.chunks(READ_TRACKS_CHUNK) {
            let found = match self.read_songs(chunk).await {
                Ok(found) => found,
                Err(error) => {
                    log::warn!("Could not read the Apple Music songs the binding lacks: {error}");
                    return HashMap::new();
                }
            };
            let mut catalog = self.catalog();
            if catalog.generation != generation {
                return HashMap::new();
            }
            // A song asked for that Apple did not answer with is one it does not have, which is
            // not asked for again under this sign-in.
            let missing: Vec<String> = chunk
                .iter()
                .filter(|track| !found.contains_key(*track))
                .cloned()
                .collect();
            if catalog.missing.len() + missing.len() > MAX_MISSING_TRACKS {
                catalog.missing.clear();
            }
            catalog.missing.extend(missing);
            // The cache starts over before a chunk that does not fit with what it holds, which
            // only spares Apple a repeat; what was just read is kept.
            if catalog.fetched.len() + found.len() > MAX_FETCHED_TRACKS {
                catalog.fetched.clear();
            }
            for (track, song) in found {
                catalog.fetched.insert(track.clone(), song.clone());
                read.insert(track, song);
            }
        }
        read
    }

    /// The songs of `tracks`, read from Apple, library songs and catalog songs each in batches
    /// as large as Apple takes. A song Apple does not know is absent from the answer.
    async fn read_songs(
        &self,
        tracks: &[String],
    ) -> Result<HashMap<String, AppleSong>, AppleError> {
        let (mut library, mut catalog) = (Vec::new(), Vec::new());
        for track in tracks {
            match song_of_track(track) {
                Some((AppleIdKind::LibrarySong, id)) => library.push(id.to_owned()),
                Some((AppleIdKind::CatalogSong, id)) => catalog.push(id.to_owned()),
                _ => {}
            }
        }
        for ids in [&mut library, &mut catalog] {
            ids.sort_unstable();
            ids.dedup();
        }
        let (library, catalog) = self
            .call(try_join(
                async {
                    if library.is_empty() {
                        return Ok(Vec::new());
                    }
                    match self
                        .client
                        .library_songs_by_ids(&library, SONG_INCLUDE)
                        .await
                    {
                        // A library that holds none of them answers 404.
                        Err(AppleApiError::NotFound(_)) => Ok(Vec::new()),
                        read => read,
                    }
                },
                async {
                    if catalog.is_empty() {
                        return Ok(Vec::new());
                    }
                    self.client.catalog_songs(&catalog).await
                },
            ))
            .await?;
        // An answer may carry songs that were not asked for.
        let asked: HashSet<&str> = tracks.iter().map(String::as_str).collect();
        let mut found = HashMap::new();
        let songs = library
            .into_iter()
            .filter(|song| is_library_song(song) && !song.is_music_video())
            .map(|song| (song.id.clone(), AppleSong::Library(kept_song(song))))
            .chain(
                catalog
                    .into_iter()
                    .map(|song| (catalog_track(&song.id), AppleSong::Catalog(song))),
            );
        for (track, song) in songs {
            if asked.contains(track.as_str()) {
                found.entry(track).or_insert(song);
            }
        }
        Ok(found)
    }

    /// Whether Apple answered, under this sign-in, that it does not have the song of `track`.
    pub(crate) fn known_missing(&self, track: &str) -> bool {
        self.catalog().missing.contains(track)
    }

    /// Remembers that Apple answered a read that began under `generation` that it does not have
    /// the song of `track`. When too many are remembered, they are forgotten, which only costs
    /// reads.
    pub(crate) fn remember_missing(&self, generation: u64, track: &str) {
        let mut catalog = self.catalog();
        if catalog.generation != generation {
            return;
        }
        if catalog.missing.len() >= MAX_MISSING_TRACKS {
            catalog.missing.clear();
        }
        catalog.missing.insert(track.to_owned());
    }

    /// What the playlist id `id` names: a Rune playlist, or else one of the catalog.
    pub(crate) fn playlist_ref(&self, id: i32) -> PlaylistRef {
        match self.catalog().ids.resolve(AppleIdKind::RunePlaylist, id) {
            Ok(stored) => PlaylistRef::Rune(stored.to_owned()),
            Err(_) => PlaylistRef::Catalog(id),
        }
    }

    /// What the mix id `id` names: a Rune mix, or else one Apple recommends.
    pub(crate) fn mix_ref(&self, id: i32) -> MixRef {
        match self.catalog().ids.resolve(AppleIdKind::RuneMix, id) {
            Ok(stored) => MixRef::Rune(stored.to_owned()),
            Err(_) => MixRef::Recommended(id),
        }
    }

    /// The song of `track` was listed again, as the UI just showed it, so it is no longer taken
    /// for missing.
    pub(crate) fn found_again(&self, track: &str) {
        self.catalog().missing.remove(track);
    }

    /// The keys Rune's likes may keep the song of `track` under: the library copy's track first,
    /// which is the one a like is kept under, then the catalog song's. A like then stays found by
    /// the library's own track whenever the song list is not known yet. They are two keys of one
    /// song when the song list names the library copy of a catalog song, or the library song
    /// carries its catalog id; a song with one known key has that key only.
    pub(crate) fn like_keys(&self, track: &str) -> Vec<String> {
        let own = track.to_owned();
        match song_of_track(track) {
            Some((AppleIdKind::LibrarySong, _)) => match self.catalog_twin(track) {
                Some(twin) => vec![own, twin],
                None => vec![own],
            },
            Some((AppleIdKind::CatalogSong, _)) => {
                let twin = self
                    .known_songs()
                    .and_then(|list| list.library_twin(track).map(str::to_owned));
                match twin {
                    Some(twin) => vec![twin, own],
                    None => vec![own],
                }
            }
            _ => vec![own],
        }
    }

    /// The track of the catalog song the library song of `track` is a copy of, as its own
    /// catalog id or the song list's copies name it.
    fn catalog_twin(&self, track: &str) -> Option<String> {
        if let Some(AppleSong::Library(song)) = self.known_song(track)
            && let Some(catalog_id) = song.store_id()
        {
            return Some(catalog_track(catalog_id));
        }
        self.known_songs()?.catalog_twin(track).map(catalog_track)
    }

    /// The track of the library's copy of the song of `track` when the library holds one, else
    /// `track`. The library home names a liked song by it.
    pub(crate) fn library_copy_of(&self, track: &str) -> String {
        self.like_keys(track)
            .into_iter()
            .find(|key| is_library_song_id(key))
            .unwrap_or_else(|| track.to_owned())
    }

    /// The user's library playlists, read from Apple as they are now.
    pub(crate) async fn read_library_playlists(&self) -> Result<Vec<LibraryPlaylist>, AppleError> {
        self.call(self.client.library_playlists().try_collect())
            .await
    }

    /// The songs of the library playlist `id`, read from Apple as they are now.
    pub(crate) async fn read_library_playlist_songs(
        &self,
        id: &str,
    ) -> Result<Vec<LibrarySong>, AppleError> {
        self.call(self.playlist_songs(id)).await
    }

    /// The library songs among `ids` the user rated liked on Apple.
    pub(crate) async fn liked_library_songs(
        &self,
        ids: &[String],
    ) -> Result<HashSet<String>, AppleError> {
        let ratings = self.call(self.client.library_song_ratings(ids)).await?;
        Ok(ratings
            .into_iter()
            .filter(|rating| {
                rating
                    .attributes
                    .as_ref()
                    .and_then(|attributes| attributes.value)
                    == Some(1)
            })
            .map(|rating| rating.id)
            .collect())
    }

    /// Whether the library of the account signed in now holds any of the library songs `ids`, as
    /// Apple answers: the song list kept may still be that of an account signed in before. Fails
    /// when Apple cannot tell.
    pub(crate) async fn holds_any_song(&self, ids: &[String]) -> Result<bool, AppleError> {
        if ids.is_empty() {
            return Ok(false);
        }
        match self.call(self.client.library_songs_by_ids(ids, &[])).await {
            Ok(found) => Ok(!found.is_empty()),
            // A library that holds none of them answers 404.
            Err(AppleError::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// A song this binding already holds, from the song list as last read or an earlier fetch.
    /// It never asks Apple.
    pub(crate) fn known_song(&self, track: &str) -> Option<AppleSong> {
        let catalog = self.catalog();
        if let Some(song) = catalog.songs.as_ref().and_then(|list| list.get(track)) {
            return Some(AppleSong::Library(song.clone()));
        }
        if let Some(song) = catalog.fetched.get(track) {
            return Some(song.clone());
        }
        // The songs of a catalog album or playlist read lately, which are not copied into
        // `fetched`.
        match song_of_track(track)? {
            (AppleIdKind::CatalogSong, id) => catalog
                .collection_song(id)
                .map(|song| AppleSong::Catalog(song.clone())),
            _ => None,
        }
    }

    /// The song of `track`: held already, or else read from Apple.
    pub(crate) async fn song(&self, track: &str) -> Result<AppleSong> {
        if let Some(song) = self.known_song(track) {
            return Ok(song);
        }
        parse_track(track)?;
        let generation = self.generation();
        let mut found = self
            .read_songs(&[track.to_owned()])
            .await
            .map_err(|error| self.request_error(error, "Apple Music song"))?;
        // The list may know a song the lookup missed, or have been read meanwhile.
        let Some(song) = found.remove(track).or_else(|| self.known_song(track)) else {
            return Err(ExpectedError::NotFound {
                what: "Apple Music song",
            }
            .into());
        };
        let mut catalog = self.catalog();
        if catalog.generation == generation {
            // The cache only spares Apple a repeat; when it is full it starts over.
            if catalog.fetched.len() >= MAX_FETCHED_TRACKS {
                catalog.fetched.clear();
            }
            catalog.fetched.insert(track.to_owned(), song.clone());
        }
        Ok(song)
    }

    /// Whether the user allowed looking the songs' lyrics up at LRCLIB for this library.
    pub(crate) fn looks_up_lyrics(&self) -> bool {
        self.lyrics.allowed()
    }

    /// The lyrics LRCLIB has for `song`, the song of `track`, found by its title, credit, album
    /// and length and kept under its catalog id when it has one (see `LibraryLyrics::lyrics_of`).
    pub(crate) async fn lyrics_of(&self, track: &str, song: SongRef<'_>) -> Option<Lyrics> {
        let id = match song {
            SongRef::Library(listed) => listed.store_id(),
            SongRef::Catalog(catalog) => Some(catalog.id.as_str()),
        }
        .unwrap_or(track)
        .to_owned();
        let query = SongQuery {
            title: song.title().to_owned(),
            artist: song.artist().unwrap_or_default().to_owned(),
            album: song.album().map(str::to_owned),
            duration: Some(song.duration())
                .filter(|seconds| *seconds > 0.0)
                .map(Duration::from_secs_f64),
        };
        self.lyrics.lyrics_of(&id, &query).await
    }

    pub(crate) fn song_summary(
        &self,
        track: &str,
        song: SongRef<'_>,
    ) -> PlayingItemMetadataSummary {
        PlayingItemMetadataSummary {
            item: PlayingItem::LibraryTrack {
                library_id: self.library_id,
                track: track.to_owned(),
            },
            artist: song.artist().unwrap_or("Unknown Artist").to_owned(),
            album: song.album().unwrap_or("Unknown Album").to_owned(),
            title: match song.title() {
                "" => "Unknown Title".to_owned(),
                title => title.to_owned(),
            },
            track_number: song
                .track_number()
                .and_then(|number| i32::try_from(number).ok())
                .unwrap_or_default(),
            duration: song.duration(),
        }
    }

    /// The cached cover at `url`, starting its download when it is not cached yet.
    pub(crate) async fn cover_path(&self, url: Option<&str>) -> Option<String> {
        let url = url?;
        if let Some(path) = self.covers.cached_path(url).await.ok().flatten() {
            return Some(path);
        }
        self.covers
            .schedule(Arc::clone(&self.cover_source), url.to_owned());
        None
    }

    /// Waits until the covers at `urls` are cached, downloading those that are not, and answers
    /// the addresses of those that now are. A cover that cannot be had is left out.
    pub(crate) async fn cache_covers(&self, urls: HashSet<String>) -> HashSet<String> {
        self.covers.cache_covers(&self.cover_source, urls).await
    }

    /// The covers of `(object id, cover address)` pairs that are cached already; the others
    /// start downloading.
    pub(crate) async fn cover_map(
        &self,
        covers: Vec<(i32, String)>,
    ) -> Result<HashMap<i32, String>> {
        let mut map = HashMap::new();
        for (object_id, url) in covers {
            if let Some(path) = self.covers.cached_path(&url).await? {
                map.insert(object_id, path);
            } else {
                self.covers.schedule(Arc::clone(&self.cover_source), url);
            }
        }
        Ok(map)
    }
}

/// The songs of a catalog album, by disc and number.
fn by_disc_and_number(mut songs: Vec<Song>) -> Vec<Song> {
    songs.sort_by_key(|song| {
        let song = SongRef::Catalog(song);
        (
            song.disc_number().unwrap_or_default(),
            song.track_number().unwrap_or_default(),
        )
    });
    songs
}

/// The catalog artists Apple names for a catalog song, with the song's id.
fn song_artists(song: SongWithArtists) -> (String, Vec<apple_music::models::Artist>) {
    let artists = song
        .relationships
        .and_then(|relationships| relationships.artists)
        .map(|artists| artists.data)
        .unwrap_or_default();
    (song.id, artists)
}

/// Whether a playlist's songs belong to the user's own music: playlists the user made (editable,
/// or without a catalog counterpart, as Favorites) do; subscribed catalog playlists do not.
fn joins_song_list(playlist: &LibraryPlaylist) -> bool {
    let attributes = playlist.attributes.as_ref();
    playlist.can_append() || attributes.and_then(|attributes| attributes.has_catalog) != Some(true)
}

/// How an Apple Music track plays on this device: the platform host plays it by the track it
/// carries, so telling it costs no request. MusicKit plays with the authorization the user gave
/// it on this device, not with the tokens the hub holds, so this holds whether or not the hub is
/// signed in, as after a restart before the app signs it in again. A user whom MusicKit does not
/// authorize learns it from the host when the track plays, as access revoked.
pub(crate) fn hosted_target(track: &str) -> Result<ServiceTarget> {
    // Only checks that the text names a song; the host takes the text as it is.
    parse_track(track)?;
    Ok(ServiceTarget::Hosted(HostedTrack {
        host: APPLE_MUSIC_HOST.to_owned(),
        key: track.to_owned(),
    }))
}

impl ServiceBinding for AppleMusicBinding {
    fn library_id(&self) -> Uuid {
        self.library_id
    }

    fn slots(&self) -> RequestSlots {
        RequestSlots::apple_music()
    }

    fn service(&self) -> ServiceKind {
        ServiceKind::AppleMusic
    }

    fn signed_in(&self) -> bool {
        self.is_signed_in()
    }

    /// Apple gives Rune no lyrics, which the library looks up at LRCLIB as far as the user
    /// allowed it.
    fn online_lyrics(&self) -> bool {
        self.looks_up_lyrics()
    }

    fn set_online_lyrics(&self, allowed: bool) {
        self.lyrics.set_allowed(allowed);
    }

    /// Apple Music plays from Apple, so offline only the songs downloaded on this device play.
    fn availability_follows_connectivity(&self) -> bool {
        true
    }

    fn reach_changed(&self, reach: &Reach) {
        {
            let mut known = lock(&self.reach);
            if *known == *reach {
                return;
            }
            known.clone_from(reach);
        }
        self.content_changed();
    }

    fn item_metadata<'a>(
        &'a self,
        track: &'a str,
        with_cover: bool,
    ) -> ServiceFuture<'a, (PlayingItemMetadataSummary, Option<String>)> {
        Box::pin(async move {
            let song = self.song(track).await?;
            let summary = self.song_summary(track, song.as_ref());
            let cover = if with_cover {
                self.cover_path(cover_url(song.as_ref().artwork()).as_deref())
                    .await
            } else {
                None
            };
            Ok((summary, cover))
        })
    }

    fn cached_summary(&self, track: &str) -> Option<PlayingItemMetadataSummary> {
        let song = self.known_song(track)?;
        Some(self.song_summary(track, song.as_ref()))
    }

    fn cover_when_cached<'a>(&'a self, track: &'a str) -> ServiceFuture<'a, Option<String>> {
        Box::pin(async move {
            let song = self.song(track).await?;
            match cover_url(song.as_ref().artwork()) {
                Some(url) => {
                    self.covers
                        .ensure_cached(Arc::clone(&self.cover_source), &url)
                        .await
                }
                None => Ok(None),
            }
        })
    }

    fn visualizer_covers<'a>(
        &'a self,
        track: &'a str,
    ) -> ServiceFuture<'a, (Option<String>, Vec<String>)> {
        Box::pin(async move {
            let song = self.song(track).await?;
            let playing_url = cover_url(song.as_ref().artwork());
            let playing = match playing_url.as_deref() {
                Some(url) => {
                    self.covers
                        .ensure_cached(Arc::clone(&self.cover_source), url)
                        .await?
                }
                None => None,
            };
            // The wall draws on the song list when Apple has been read for it; it never starts
            // that read.
            let wall_urls = match self.known_songs() {
                Some(list) => {
                    let mut urls: Vec<String> = list
                        .songs()
                        .iter()
                        .filter_map(|song| cover_url(song.attributes.as_ref()?.artwork.as_ref()))
                        .filter(|url| Some(url) != playing_url.as_ref())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    urls.shuffle(&mut rand::thread_rng());
                    urls.truncate(RANDOM_COVERS);
                    urls
                }
                None => Vec::new(),
            };
            // A cover of the wall that cannot be had is left out; it never fails the request.
            // The download queue limits how many run at once.
            let wall = join_all(wall_urls.iter().map(|url| async move {
                self.covers
                    .ensure_cached(Arc::clone(&self.cover_source), url)
                    .await
                    .ok()
                    .flatten()
            }))
            .await
            .into_iter()
            .flatten()
            .collect();
            Ok((playing, wall))
        })
    }

    fn playback_source<'a>(&'a self, track: &'a str) -> ServiceFuture<'a, ServiceTarget> {
        Box::pin(async move { hosted_target(track) })
    }

    fn read_tracks(&self, tracks: Vec<String>) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.read_unknown_tracks(tracks).await;
        })
    }

    fn opened(&self) {
        // Reading a big library takes a while, so it starts now. A request that comes meanwhile
        // joins that read.
        let Some(binding) = self.this.upgrade() else {
            return;
        };
        tokio::spawn(async move {
            let _ = binding.songs().await;
        });
    }
}

#[cfg(test)]
#[path = "apple_music_binding_tests.rs"]
mod tests;
