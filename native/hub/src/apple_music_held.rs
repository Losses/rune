//! What Rune's own mixes, playlists and likes add to an Apple Music library. The library holds
//! what they hold, as a standard Rune server does, for as long as one of them holds it, and
//! nothing is written to the Apple account:
//! - a catalog album a Rune mix names, with its songs;
//! - a catalog artist a Rune mix names, with the songs a mix of it plays: those of every album
//!   of its own (see `AppleMusicBinding::catalog_collection_songs`), read again once a day so
//!   its new releases come in (see `ARTIST_SONGS_AGE`);
//! - a catalog playlist a Rune mix names, with the songs it holds now: whenever a mix or a page
//!   of it reads it again, the library takes in what that read brought (see `collection_read`),
//!   so the changes Apple makes come in;
//! - a catalog song a Rune mix names, that a Rune playlist holds, imported playlists among them,
//!   or that is liked in Rune. An entry of a playlist whose library song is gone counts as the
//!   catalog song it was a copy of.
//!
//! What they name of the library itself, its songs, albums and artists and Rune's playlists, adds
//! nothing. Which items count is worked out from the mixes, playlists and likes whenever they
//! change, and is never kept apart from them.
//!
//! The songs join the song list after the library's, each once, unless the library has a copy of
//! it (see `AppleSongs::with_held`). Each is on the library's album of its catalog album when the
//! library holds one, as a song the Apple library adds to an album it holds some of is; otherwise
//! on the album Rune holds of it (see `RUNE_ALBUM`), which joins the album list after the
//! library's albums. That album holds the songs Rune holds of the catalog album, all of them when
//! a mix names the album. Their artists are the library's artists as every song's are; a catalog
//! artist a mix names that holds no song yet is listed by itself (see
//! `apple_music_translate::entries`).
//!
//! What the catalog said of the items is kept for the binding's sign-in, on disk too beside the
//! binding's lists (see `HELD`), so the pages list it at once after a restart. What is missing is
//! read in the background, and the pages that show the library reload once it is read, so no page
//! waits on the catalog for it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use anyhow::Result;
use apple_music::models::{
    LibraryAlbum, LibraryAlbumAttributes, LibrarySong, LibrarySongAttributes,
    LibrarySongRelationships, PlayParams, Relationship, Resource, Song,
};
use database::mix_operator::MixOperator;
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::{Map, json};

use crate::apple_music_binding::{
    AppleIdKind, AppleMusicBinding, AppleSongs, CatalogCollection, catalog_track, release_year,
    song_of_track,
};
use crate::apple_music_translate::{CATALOG_PREFIX, catalog_copy, gone, is_not_found};
use crate::service::rune_likes::StoredLike;
use crate::service::rune_mixes::StoredMix;
use crate::service::rune_playlists::StoredPlaylist;

/// What the id of an album Rune holds starts with, before the id of its catalog album. Apple's
/// library ids start with a letter and a dot and its catalog ones are kept as `catalog:<id>`, so
/// no id Apple gives starts with it, and none is ever sent to Apple.
const RUNE_ALBUM: &str = "rune-album:";

/// The id of the album Rune holds of the catalog album `catalog_id`.
fn rune_album_remote(catalog_id: &str) -> String {
    format!("{RUNE_ALBUM}{catalog_id}")
}

/// Whether `remote` is the id of an album Rune holds.
pub(crate) fn is_rune_album(remote: &str) -> bool {
    remote.starts_with(RUNE_ALBUM)
}

/// The catalog album an album of the album list is of: the one Apple names for an album of the
/// library, or the one an album Rune holds is of.
pub(crate) fn catalog_album_of(album: &LibraryAlbum) -> Option<&str> {
    album
        .catalog_id()
        .or_else(|| album.id.strip_prefix(RUNE_ALBUM))
        .filter(|id| !id.is_empty())
}

/// How many catalog albums are read at once for what Rune's mixes name.
const ALBUM_READS: usize = 4;

/// How long after a read of the items not read yet failed they are asked for again.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// How long what was read of the songs of a catalog artist a mix names counts as young. Older,
/// it is read again in the background, so the artist's new releases come in, and the pages that
/// show the library reload when that changed them.
pub(crate) const ARTIST_SONGS_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// The name what the catalog said of the items is kept as beside the binding's lists. It is a
/// copy of what Apple said of the items Rune's mixes, playlists and likes name when it is kept,
/// and goes with the lists when the account signs out:
///
/// ```json
/// {"albums": {"<catalog album id>": [<song>, ...]}, "tracks": {"catalog:<id>": <song>},
///  "artists": {"<catalog artist id>": "<name>"},
///  "artist_songs": {"<catalog artist id>": [<song>, ...]},
///  "artist_read": {"<catalog artist id>": <seconds since the Unix epoch>},
///  "playlists": {"<catalog playlist id>": [<song>, ...]}}
/// ```
///
/// Each song is a catalog song as Apple gives it, with only the id of its catalog album among
/// its relationships. `artist_read` says when the songs of each artist were read, by the
/// binding's clock; one it does not name is read again.
const HELD: &str = "held";

/// What `HELD` holds.
#[derive(Default, Serialize, Deserialize)]
struct StoredRead {
    #[serde(default)]
    albums: BTreeMap<String, Vec<Song>>,
    #[serde(default)]
    tracks: BTreeMap<String, Song>,
    #[serde(default)]
    artists: BTreeMap<String, String>,
    #[serde(default)]
    artist_songs: BTreeMap<String, Vec<Song>>,
    #[serde(default)]
    artist_read: BTreeMap<String, u64>,
    #[serde(default)]
    playlists: BTreeMap<String, Vec<Song>>,
}

/// `song` as Rune keeps it, on the catalog album `album` when it is on one: all of it but its
/// address and its relationships, of which only the id of its album is kept.
fn kept_on(song: &Song, album: Option<&str>) -> Song {
    Resource {
        id: song.id.clone(),
        kind: song.kind.clone(),
        href: None,
        attributes: song.attributes.clone(),
        relationships: album.map(|id| json!({"albums": {"data": [{"id": id, "type": "albums"}]}})),
    }
}

/// `song` as Rune keeps it, on the catalog album Apple named for it.
fn kept(song: &Song) -> Song {
    kept_on(song, song.album_id())
}

/// The catalog items Rune's mixes, playlists and likes hold, each once, in the order they are
/// first named.
#[derive(Debug, Default)]
struct Named {
    /// The catalog albums, by catalog id.
    albums: Vec<String>,
    /// The catalog artists, by catalog id.
    artists: Vec<String>,
    /// The catalog playlists, by catalog id.
    playlists: Vec<String>,
    /// The catalog songs named on their own, by track.
    tracks: Vec<String>,
}

/// Items in the order they are first added, each once.
struct Distinct {
    items: Vec<String>,
    seen: HashSet<String>,
}

impl Distinct {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            seen: HashSet::new(),
        }
    }

    fn add(&mut self, item: Option<String>) {
        if let Some(item) = item
            && self.seen.insert(item.clone())
        {
            self.items.push(item);
        }
    }
}

/// The catalog id of the catalog album, artist or playlist a mix keeps as `parameter`; None for
/// one of the library, an album Rune holds, an artist of the library's artist list and a Rune
/// playlist.
fn catalog_item(parameter: &str) -> Option<String> {
    parameter
        .strip_prefix(CATALOG_PREFIX)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/// `track` when it names a catalog song.
fn catalog_song(track: &str) -> Option<String> {
    matches!(song_of_track(track), Some((AppleIdKind::CatalogSong, _))).then(|| track.to_owned())
}

/// What the binding knows of a song that decides which catalog song Rune holds for it.
trait SongFacts {
    /// The keys a like of the song of `track` is kept under (see
    /// `AppleMusicBinding::like_keys`).
    fn like_keys(&self, track: &str) -> Vec<String>;
    /// Whether Apple said it does not have the song of `track`, and nothing listed it since.
    fn gone(&self, track: &str) -> bool;
}

impl SongFacts for AppleMusicBinding {
    fn like_keys(&self, track: &str) -> Vec<String> {
        AppleMusicBinding::like_keys(self, track)
    }

    fn gone(&self, track: &str) -> bool {
        gone(self, track)
    }
}

impl Named {
    /// The catalog items of `mixes`, `playlists` and `likes`: the albums, artists, playlists and
    /// songs a mix names by `lib::album`, `lib::artist`, `lib::playlist` and `lib::track`, the
    /// songs of the playlists' entries and the songs liked now. An entry whose library song is gone plays the catalog
    /// song it was a copy of, which counts instead. A like counts when the entry made last for
    /// the song, under any of its keys, likes it, as `RuneLikes::is_liked` decides.
    fn of(
        mixes: &[StoredMix],
        playlists: &[StoredPlaylist],
        likes: &[StoredLike],
        facts: &impl SongFacts,
    ) -> Self {
        let (mut albums, mut artists, mut catalog_playlists, mut tracks) = (
            Distinct::new(),
            Distinct::new(),
            Distinct::new(),
            Distinct::new(),
        );
        for query in mixes.iter().flat_map(|mix| &mix.queries) {
            match MixOperator::from_syntax(&query.operator) {
                Ok(MixOperator::LibAlbum) => albums.add(catalog_item(&query.parameter)),
                Ok(MixOperator::LibArtist) => artists.add(catalog_item(&query.parameter)),
                Ok(MixOperator::LibPlaylist) => {
                    catalog_playlists.add(catalog_item(&query.parameter));
                }
                Ok(MixOperator::LibTrack) => tracks.add(catalog_song(&query.parameter)),
                _ => {}
            }
        }
        for entry in playlists.iter().flat_map(|playlist| &playlist.entries) {
            match catalog_song(&entry.track) {
                Some(track) => tracks.add(Some(track)),
                None if facts.gone(&entry.track) => tracks.add(catalog_copy(entry)),
                None => {}
            }
        }
        let last: HashMap<&str, usize> = likes
            .iter()
            .enumerate()
            .map(|(at, like)| (like.id.as_str(), at))
            .collect();
        for like in likes.iter().filter(|like| like.liked) {
            let Some(track) = catalog_song(&like.id) else {
                continue;
            };
            // The song may have been unliked under its library copy's key since.
            let decided = facts
                .like_keys(&track)
                .iter()
                .filter_map(|key| last.get(key.as_str()))
                .max()
                .copied();
            if decided.is_some_and(|at| likes[at].liked) {
                tracks.add(Some(track));
            }
        }
        Self {
            albums: albums.items,
            artists: artists.items,
            playlists: catalog_playlists.items,
            tracks: tracks.items,
        }
    }
}

/// A catalog artist a Rune mix names, as the catalog names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HeldArtist {
    /// Its catalog id.
    pub(crate) id: String,
    pub(crate) name: String,
}

/// What Rune's mixes, playlists and likes add to the library, as far as the catalog was read for
/// it.
#[derive(Debug, PartialEq)]
pub(crate) struct Held {
    /// The catalog songs, each on its catalog album as `kept_on` keeps it: those of the albums,
    /// album by album in the albums' order, then those of the artists, then those of the
    /// playlists, then those named on their own, each once.
    songs: Vec<Song>,
    /// The catalog artists the mixes name, whose names were read.
    artists: Vec<HeldArtist>,
}

impl Held {
    fn built(named: &Named, read: &Read) -> Self {
        let mut seen = HashSet::new();
        let albums = named
            .albums
            .iter()
            .filter_map(|id| read.albums.get(id))
            .flatten();
        let artists = named
            .artists
            .iter()
            .filter_map(|id| read.artist_songs.get(id))
            .flatten();
        let playlists = named
            .playlists
            .iter()
            .filter_map(|id| read.playlists.get(id))
            .flatten();
        let tracks = named
            .tracks
            .iter()
            .filter_map(|track| read.tracks.get(track));
        Self {
            songs: albums
                .chain(artists)
                .chain(playlists)
                .chain(tracks)
                .filter(|song| seen.insert(song.id.clone()))
                .cloned()
                .collect(),
            artists: named
                .artists
                .iter()
                .filter_map(|id| {
                    Some(HeldArtist {
                        id: id.clone(),
                        name: read.artists.get(id)?.clone(),
                    })
                })
                .collect(),
        }
    }

    /// Whether it adds no song to the library.
    pub(crate) fn is_empty(&self) -> bool {
        self.songs.is_empty()
    }

    /// The catalog artists the mixes name.
    pub(crate) fn artists(&self) -> &[HeldArtist] {
        &self.artists
    }
}

/// The entry the song list lists the catalog song `song` as once Rune holds it in the library: an
/// entry of a catalog song, named by its catalog id as the songs only a playlist holds are, and on
/// `album` when it is on one, the library's album of its catalog album or the album Rune holds of
/// it.
fn held_entry(song: &Song, album: Option<&str>) -> LibrarySong {
    let attributes = song.attributes.as_ref().map(|attributes| {
        let play_params = match &attributes.play_params {
            Some(params) => PlayParams {
                catalog_id: Some(song.id.clone()),
                ..params.clone()
            },
            None => PlayParams {
                id: song.id.clone(),
                kind: "song".to_owned(),
                is_library: None,
                catalog_id: Some(song.id.clone()),
                purchased_id: None,
                reporting: None,
                reporting_id: None,
                extra: Map::new(),
            },
        };
        LibrarySongAttributes {
            album_name: attributes.album_name.clone(),
            artist_name: attributes.artist_name.clone(),
            artwork: attributes.artwork.clone(),
            content_rating: attributes.content_rating.clone(),
            disc_number: attributes.disc_number,
            duration_in_millis: attributes.duration_in_millis,
            genre_names: attributes.genre_names.clone(),
            has_lyrics: attributes.has_lyrics,
            in_favorites: None,
            name: attributes.name.clone(),
            play_params: Some(play_params),
            release_date: attributes.release_date.clone(),
            track_number: attributes.track_number,
        }
    });
    let relationships = album.map(|id| LibrarySongRelationships {
        albums: Some(Relationship {
            href: None,
            next: None,
            data: vec![Resource {
                id: id.to_owned(),
                kind: "library-albums".to_owned(),
                href: None,
                attributes: None,
                relationships: None,
            }],
        }),
        ..LibrarySongRelationships::default()
    });
    Resource {
        id: song.id.clone(),
        kind: song.kind.clone(),
        href: None,
        attributes,
        relationships,
    }
}

/// `own`, the library's songs, with the songs of `held` after them (see `AppleSongs::with_held`),
/// each on the album of `albums`, the library's, of its catalog album, or else on the album Rune
/// holds of it.
fn joined_songs(own: &AppleSongs, held: &Held, albums: Option<&[LibraryAlbum]>) -> AppleSongs {
    let library: HashMap<&str, &str> = albums
        .into_iter()
        .flatten()
        .filter_map(|album| Some((album.catalog_id()?, album.id.as_str())))
        .collect();
    own.with_held(held.songs.iter().map(|song| {
        let album = song.album_id().map(|catalog| match library.get(catalog) {
            Some(album) => (*album).to_owned(),
            None => rune_album_remote(catalog),
        });
        held_entry(song, album.as_deref())
    }))
}

/// The albums Rune holds of the songs Rune holds in `songs`, in the order of their first songs.
/// Each is named, and names its artist and cover, as its first song does, takes the release of
/// its earliest song and counts the songs Rune holds of it.
fn rune_albums(songs: &AppleSongs) -> Vec<LibraryAlbum> {
    let mut albums: Vec<LibraryAlbum> = Vec::new();
    let mut at: HashMap<&str, usize> = HashMap::new();
    for song in songs.held_songs() {
        let (Some(id), Some(attributes)) = (song.album_id(), &song.attributes) else {
            continue;
        };
        if !is_rune_album(id) {
            continue;
        }
        match at.get(id) {
            Some(&index) => {
                if let Some(album) = albums[index].attributes.as_mut() {
                    add_to_album(album, attributes);
                }
            }
            None => {
                at.insert(id, albums.len());
                albums.push(Resource {
                    id: id.to_owned(),
                    kind: "library-albums".to_owned(),
                    href: None,
                    attributes: Some(LibraryAlbumAttributes {
                        artist_name: attributes.artist_name.clone(),
                        artwork: attributes.artwork.clone(),
                        name: attributes.album_name.clone(),
                        release_date: attributes.release_date.clone(),
                        track_count: Some(1),
                        ..LibraryAlbumAttributes::default()
                    }),
                    relationships: None,
                });
            }
        }
    }
    albums
}

/// Counts one more song on an album Rune holds, `song` and `album` being their attributes. The
/// album takes the song's release when it is earlier, and its cover while it has none.
fn add_to_album(album: &mut LibraryAlbumAttributes, song: &LibrarySongAttributes) {
    album.track_count = Some(album.track_count.unwrap_or_default() + 1);
    let earlier = match (
        release_year(album.release_date.as_deref()),
        release_year(song.release_date.as_deref()),
    ) {
        (Some(year), Some(other)) => other < year,
        (year, other) => year.is_none() && other.is_some(),
    };
    if earlier {
        album.release_date.clone_from(&song.release_date);
    }
    if album.artwork.is_none() {
        album.artwork.clone_from(&song.artwork);
    }
}

/// What the catalog said of the items Rune's mixes, playlists and likes name, under the binding's
/// sign-in. Only the items named are kept.
#[derive(Default)]
struct Read {
    /// The songs of each album, by catalog id, in album order, each on that album.
    albums: HashMap<String, Vec<Song>>,
    /// The songs named on their own, by track.
    tracks: HashMap<String, Song>,
    /// The name of each artist, by catalog id.
    artists: HashMap<String, String>,
    /// The songs a mix of each artist plays, by catalog id, each on its album.
    artist_songs: HashMap<String, Vec<Song>>,
    /// When the songs of each artist were read, in seconds since the Unix epoch by the binding's
    /// clock.
    artist_read: HashMap<String, u64>,
    /// The songs of each playlist, by catalog id, as they were read last.
    playlists: HashMap<String, Vec<Song>>,
    /// The albums, artists and playlists Apple said it does not have, by catalog id, which are
    /// not asked for again under this sign-in, and the artists whose songs it said it does not
    /// have all of. The songs it does not have the binding remembers itself.
    missing_albums: HashSet<String>,
    missing_artists: HashSet<String>,
    missing_artist_songs: HashSet<String>,
    missing_playlists: HashSet<String>,
}

impl Read {
    /// Forgets what is not named any more. Answers whether that was anything but what Apple
    /// does not have.
    fn keep_only(&mut self, named: &Named) -> bool {
        let albums: HashSet<&String> = named.albums.iter().collect();
        let artists: HashSet<&String> = named.artists.iter().collect();
        let playlists: HashSet<&String> = named.playlists.iter().collect();
        let tracks: HashSet<&String> = named.tracks.iter().collect();
        let sizes = |read: &Self| {
            [
                read.albums.len(),
                read.tracks.len(),
                read.artists.len(),
                read.artist_songs.len(),
                read.playlists.len(),
            ]
        };
        let before = sizes(self);
        self.albums.retain(|id, _| albums.contains(id));
        self.missing_albums.retain(|id| albums.contains(id));
        self.artists.retain(|id, _| artists.contains(id));
        self.missing_artists.retain(|id| artists.contains(id));
        self.artist_songs.retain(|id, _| artists.contains(id));
        self.artist_read.retain(|id, _| artists.contains(id));
        self.missing_artist_songs.retain(|id| artists.contains(id));
        self.playlists.retain(|id, _| playlists.contains(id));
        self.missing_playlists.retain(|id| playlists.contains(id));
        self.tracks.retain(|track, _| tracks.contains(track));
        before != sizes(self)
    }

    /// What is kept of it as `HELD`.
    fn stored(&self) -> StoredRead {
        StoredRead {
            albums: self.albums.clone().into_iter().collect(),
            tracks: self.tracks.clone().into_iter().collect(),
            artists: self.artists.clone().into_iter().collect(),
            artist_songs: self.artist_songs.clone().into_iter().collect(),
            artist_read: self.artist_read.clone().into_iter().collect(),
            playlists: self.playlists.clone().into_iter().collect(),
        }
    }

    /// Takes in what was kept as `HELD`, under what was read since.
    fn restore(&mut self, stored: StoredRead) {
        for (id, songs) in stored.albums {
            self.albums.entry(id).or_insert(songs);
        }
        for (track, song) in stored.tracks {
            self.tracks.entry(track).or_insert(song);
        }
        for (id, name) in stored.artists {
            self.artists.entry(id).or_insert(name);
        }
        for (id, songs) in stored.artist_songs {
            self.artist_songs.entry(id).or_insert(songs);
        }
        for (id, read) in stored.artist_read {
            self.artist_read.entry(id).or_insert(read);
        }
        for (id, songs) in stored.playlists {
            self.playlists.entry(id).or_insert(songs);
        }
    }

    /// Takes in what one read found. The songs of an artist read since, as a page's read taken in
    /// while this one ran, stay with the time they were read. Answers whether that changed what
    /// it holds of an item.
    fn extend(&mut self, other: Self) -> bool {
        fn take<V: PartialEq>(into: &mut HashMap<String, V>, from: HashMap<String, V>) -> bool {
            let mut changed = false;
            for (key, value) in from {
                changed |= into.get(&key) != Some(&value);
                into.insert(key, value);
            }
            changed
        }
        let mut changed = take(&mut self.albums, other.albums);
        changed |= take(&mut self.tracks, other.tracks);
        changed |= take(&mut self.artists, other.artists);
        let mut artist_songs = other.artist_songs;
        artist_songs.retain(|id, _| {
            let read = other.artist_read.get(id);
            self.artist_read
                .get(id)
                .is_none_or(|held| read.is_some_and(|read| read > held))
        });
        changed |= take(&mut self.artist_songs, artist_songs);
        changed |= take(&mut self.playlists, other.playlists);
        for (id, read) in other.artist_read {
            let held = self.artist_read.entry(id).or_insert(read);
            *held = (*held).max(read);
        }
        self.missing_albums.extend(other.missing_albums);
        self.missing_artists.extend(other.missing_artists);
        self.missing_artist_songs.extend(other.missing_artist_songs);
        self.missing_playlists.extend(other.missing_playlists);
        changed
    }

    /// The artists whose songs were read `ARTIST_SONGS_AGE` or longer before `now`, in seconds
    /// since the Unix epoch, or at a time not known.
    fn stale_artists(&self, now: u64) -> Vec<String> {
        self.artist_songs
            .keys()
            .filter(|id| {
                self.artist_read
                    .get(*id)
                    .is_none_or(|read| now.saturating_sub(*read) >= ARTIST_SONGS_AGE.as_secs())
            })
            .cloned()
            .collect()
    }

    /// The named items not read yet that may still be read.
    fn unread(&self, named: &Named, binding: &AppleMusicBinding) -> Unread {
        Unread {
            albums: named
                .albums
                .iter()
                .filter(|id| !self.albums.contains_key(*id) && !self.missing_albums.contains(*id))
                .cloned()
                .collect(),
            // An artist whose name or songs are not read, unless Apple does not have it.
            artists: named
                .artists
                .iter()
                .filter(|id| {
                    !self.missing_artists.contains(*id)
                        && (!self.artists.contains_key(*id)
                            || (!self.artist_songs.contains_key(*id)
                                && !self.missing_artist_songs.contains(*id)))
                })
                .cloned()
                .collect(),
            playlists: named
                .playlists
                .iter()
                .filter(|id| {
                    !self.playlists.contains_key(*id) && !self.missing_playlists.contains(*id)
                })
                .cloned()
                .collect(),
            tracks: named
                .tracks
                .iter()
                .filter(|track| !self.tracks.contains_key(*track) && !binding.known_missing(track))
                .cloned()
                .collect(),
            stale_artists: Vec::new(),
        }
    }
}

/// The named items that were not read, and the artists whose songs are to be read again.
#[derive(Clone, Debug, Default)]
struct Unread {
    albums: Vec<String>,
    artists: Vec<String>,
    playlists: Vec<String>,
    tracks: Vec<String>,
    /// The artists whose songs are old (see `ARTIST_SONGS_AGE`).
    stale_artists: Vec<String>,
}

impl Unread {
    fn is_empty(&self) -> bool {
        self.albums.is_empty()
            && self.artists.is_empty()
            && self.playlists.is_empty()
            && self.tracks.is_empty()
            && self.stale_artists.is_empty()
    }

    /// The items of these that `other` does not name.
    fn not_in(&self, other: &Self) -> Self {
        let not_in = |items: &[String], other: &[String]| {
            items
                .iter()
                .filter(|item| !other.contains(item))
                .cloned()
                .collect()
        };
        Self {
            albums: not_in(&self.albums, &other.albums),
            artists: not_in(&self.artists, &other.artists),
            playlists: not_in(&self.playlists, &other.playlists),
            tracks: not_in(&self.tracks, &other.tracks),
            stale_artists: not_in(&self.stale_artists, &other.stale_artists),
        }
    }
}

/// What one read of the unread items found.
#[derive(Default)]
struct Found {
    read: Read,
    /// Whether a read failed for anything but Apple not having the item.
    failed: bool,
}

/// What `held` built, and from what.
struct Built {
    mixes: Arc<Vec<StoredMix>>,
    playlists: Arc<Vec<StoredPlaylist>>,
    likes: Arc<Vec<StoredLike>>,
    version: u64,
    named: Named,
    held: Arc<Held>,
    /// The items it lacks, which a read in the background is for.
    unread: Unread,
}

/// The song list with the songs Rune holds, and what it was made of.
struct SongsWith {
    own: Arc<AppleSongs>,
    held: Arc<Held>,
    albums: Option<Arc<Vec<LibraryAlbum>>>,
    joined: Arc<AppleSongs>,
}

/// The album list with the albums Rune holds, and what it was made of.
struct AlbumsWith {
    own: Arc<Vec<LibraryAlbum>>,
    songs: Arc<AppleSongs>,
    joined: Arc<Vec<LibraryAlbum>>,
}

#[derive(Default)]
struct State {
    /// Counts the sign-outs that dropped what was read. A read that began under an earlier count
    /// is not kept.
    generation: u64,
    read: Read,
    /// Counts the changes to `read`, so what was built before one is built again.
    version: u64,
    built: Option<Built>,
    /// When the items not read yet were last asked for, by the binding's clock.
    asked: Option<Instant>,
    /// Whether a read of the items not read yet waits for a sign-in.
    owed: bool,
    songs: Option<SongsWith>,
    albums: Option<AlbumsWith>,
}

/// The binding's part of what Rune's mixes, playlists and likes add to its library: what the
/// catalog said of their items, and the lists made with it.
#[derive(Default)]
pub(crate) struct HeldStore {
    state: Mutex<State>,
    /// Whether the unread items are being read.
    reading: AtomicBool,
    /// Set once `HELD` was read.
    restored: tokio::sync::OnceCell<()>,
}

/// Whether two lists that may be missing are the same.
fn same<T>(left: &Option<Arc<T>>, right: &Option<Arc<T>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        (None, None) => true,
        _ => false,
    }
}

impl HeldStore {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The count to take before a read of the catalog whose result is taken in, and to hand
    /// back with it.
    pub(crate) fn generation(&self) -> u64 {
        self.state().generation
    }

    /// Forgets what was read, as the account signs out: what a read begun before brings is not
    /// taken in. It comes before the binding's lists are cleared, so nothing is kept beside them
    /// after.
    pub(crate) fn clear(&self) {
        let mut state = self.state();
        let generation = state.generation + 1;
        let version = state.version + 1;
        *state = State {
            generation,
            version,
            ..State::default()
        };
    }

    /// `own`, the library's songs, with the songs of `held` (see `joined_songs`), each on its
    /// album of `albums`, the library's. Made once for them.
    pub(crate) fn songs_with(
        &self,
        own: Arc<AppleSongs>,
        held: &Arc<Held>,
        albums: Option<Arc<Vec<LibraryAlbum>>>,
    ) -> Arc<AppleSongs> {
        if held.is_empty() {
            return own;
        }
        if let Some(with) = &self.state().songs
            && Arc::ptr_eq(&with.own, &own)
            && Arc::ptr_eq(&with.held, held)
            && same(&with.albums, &albums)
        {
            return Arc::clone(&with.joined);
        }
        let joined = Arc::new(joined_songs(
            &own,
            held,
            albums.as_deref().map(Vec::as_slice),
        ));
        self.state().songs = Some(SongsWith {
            own,
            held: Arc::clone(held),
            albums,
            joined: Arc::clone(&joined),
        });
        joined
    }

    /// `own`, the library's albums, with the albums Rune holds of the songs Rune holds in `songs`
    /// after them (see `rune_albums`). Made once for them.
    pub(crate) fn albums_with(
        &self,
        own: Arc<Vec<LibraryAlbum>>,
        songs: Option<Arc<AppleSongs>>,
    ) -> Arc<Vec<LibraryAlbum>> {
        let Some(songs) = songs.filter(|songs| !songs.held_songs().is_empty()) else {
            return own;
        };
        if let Some(with) = &self.state().albums
            && Arc::ptr_eq(&with.own, &own)
            && Arc::ptr_eq(&with.songs, &songs)
        {
            return Arc::clone(&with.joined);
        }
        let added = rune_albums(&songs);
        let joined = if added.is_empty() {
            Arc::clone(&own)
        } else {
            let mut all = Vec::clone(&own);
            all.extend(added);
            Arc::new(all)
        };
        self.state().albums = Some(AlbumsWith {
            own,
            songs,
            joined: Arc::clone(&joined),
        });
        joined
    }

    /// What was built last, and the items it lacks and the artists whose songs grew old when
    /// they are to be asked for at `now`, which is `seconds` since the Unix epoch, when it was
    /// built from `mixes`, `playlists` and `likes` and what was read since; None otherwise.
    fn current(
        &self,
        mixes: &Arc<Vec<StoredMix>>,
        playlists: &Arc<Vec<StoredPlaylist>>,
        likes: &Arc<Vec<StoredLike>>,
        (now, seconds): (Instant, u64),
    ) -> Option<(Arc<Held>, Option<Unread>)> {
        let state = self.state();
        let built = state.built.as_ref()?;
        let current = Arc::ptr_eq(&built.mixes, mixes)
            && Arc::ptr_eq(&built.playlists, playlists)
            && Arc::ptr_eq(&built.likes, likes)
            && built.version == state.version;
        if !current {
            return None;
        }
        let wanted = Unread {
            stale_artists: state.read.stale_artists(seconds),
            ..built.unread.clone()
        };
        let ask_again = !wanted.is_empty()
            && state
                .asked
                .is_none_or(|asked| now.saturating_duration_since(asked) >= RETRY_AFTER);
        Some((Arc::clone(&built.held), ask_again.then_some(wanted)))
    }

    /// Builds what `named`, the items of `mixes`, `playlists` and `likes`, add to the library
    /// from what was read, forgetting what they no longer name when `whole` says that every one
    /// of their files was read, and answers it with the items not read yet.
    fn build(
        &self,
        binding: &AppleMusicBinding,
        named: Named,
        whole: bool,
        (mixes, playlists, likes): Sources,
    ) -> (Arc<Held>, Unread) {
        let mut state = self.state();
        let state = &mut *state;
        if whole && state.read.keep_only(&named) {
            binding.keep_beside_lists(HELD, state.read.stored());
        }
        let built = Held::built(&named, &state.read);
        // An unchanged build keeps its lists.
        let held = match &state.built {
            Some(before) if *before.held == built => Arc::clone(&before.held),
            _ => Arc::new(built),
        };
        let mut unread = state.read.unread(&named, binding);
        unread.stale_artists = state.read.stale_artists(binding.epoch_seconds());
        state.built = Some(Built {
            mixes,
            playlists,
            likes,
            version: state.version,
            named,
            held: Arc::clone(&held),
            unread: unread.clone(),
        });
        (held, unread)
    }

    /// Takes in what a read begun under `generation` found, unless the account signed out since,
    /// and keeps it beside the binding's lists. Answers whether that changed what the library
    /// holds. The albums and playlists it read were not read when it began: one taken in since
    /// is kept as it is, as it came from this read itself or from a later one of its own (see
    /// `collection_read`).
    fn keep(&self, binding: &AppleMusicBinding, generation: u64, found: Found) -> bool {
        let mut state = self.state();
        let state = &mut *state;
        if state.generation != generation {
            return false;
        }
        let Found { mut read, failed } = found;
        read.albums
            .retain(|id, _| !state.read.albums.contains_key(id));
        read.playlists
            .retain(|id, _| !state.read.playlists.contains_key(id));
        let read_again = !read.artist_read.is_empty();
        let missing = !read.missing_albums.is_empty()
            || !read.missing_artists.is_empty()
            || !read.missing_artist_songs.is_empty()
            || !read.missing_playlists.is_empty();
        let changed = state.read.extend(read);
        if changed || read_again || missing {
            state.version += 1;
        } else if let Some(built) = &mut state.built {
            // A read that found nothing new may still have found songs Apple does not have: the
            // build's list of what is unread must say so, or they are handed to every read.
            built.unread = state.read.unread(&built.named, binding);
        }
        if changed || read_again {
            binding.keep_beside_lists(HELD, state.read.stored());
        }
        if failed {
            state.asked = Some(binding.now());
        }
        changed
    }

    /// Reads what was kept as `HELD` once, unless the account signed out meanwhile.
    async fn restore(&self, binding: &AppleMusicBinding) {
        self.restored
            .get_or_init(|| async {
                let generation = self.generation();
                let Some(stored) = binding.kept_beside_lists::<StoredRead>(HELD).await else {
                    return;
                };
                let mut state = self.state();
                if state.generation == generation {
                    state.read.restore(stored);
                    state.version += 1;
                }
            })
            .await;
    }
}

/// The mixes, playlists and likes something was built from.
type Sources = (
    Arc<Vec<StoredMix>>,
    Arc<Vec<StoredPlaylist>>,
    Arc<Vec<StoredLike>>,
);

/// What Rune's mixes, playlists and likes add to the library, built from what was read of their
/// items. What is not read yet is read in the background (see `read_in_background`), and asked
/// for again at most every `RETRY_AFTER` when a read of it failed.
pub(crate) async fn held(binding: &AppleMusicBinding) -> Arc<Held> {
    binding.held.restore(binding).await;
    // A file that cannot be read names nothing, as it lists nothing; but what was read for the
    // items it names is not forgotten, as they come back once it can be read again.
    let (mixes, playlists, likes) = (
        binding.rune_mixes.list().await,
        binding.rune_playlists.list().await,
        binding.rune_likes.list().await,
    );
    let whole = mixes.is_ok() && playlists.is_ok() && likes.is_ok();
    let (mixes, playlists, likes) = (
        mixes.unwrap_or_default(),
        playlists.unwrap_or_default(),
        likes.unwrap_or_default(),
    );
    if let Some((held, again)) = binding.held.current(
        &mixes,
        &playlists,
        &likes,
        (binding.now(), binding.epoch_seconds()),
    ) {
        if let Some(unread) = again {
            read_in_background(binding, unread);
        }
        return held;
    }
    let named = Named::of(&mixes, &playlists, &likes, binding);
    let (held, unread) = binding
        .held
        .build(binding, named, whole, (mixes, playlists, likes));
    if !unread.is_empty() {
        read_in_background(binding, unread);
    }
    held
}

/// Takes in the songs a read of the catalog album, playlist or artist `catalog_id` that began
/// under `generation` brought, when Rune holds it: a mix or a page of a playlist reads it again
/// as Apple has it now, and the library follows. A read of an artist's songs counts as the one
/// made once a day for its new releases. The pages that show the library reload when that changed
/// what it holds.
pub(crate) fn collection_read(
    binding: &AppleMusicBinding,
    generation: u64,
    collection: CatalogCollection,
    catalog_id: &str,
    songs: &[Song],
) {
    {
        let mut state = binding.held.state();
        let state = &mut *state;
        let Some(built) = &state.built else {
            return;
        };
        let names = |ids: &[String]| ids.iter().any(|id| id == catalog_id);
        let (held, songs) = match collection {
            CatalogCollection::Album(_) if names(&built.named.albums) => (
                &mut state.read.albums,
                songs
                    .iter()
                    .map(|song| kept_on(song, Some(catalog_id)))
                    .collect(),
            ),
            CatalogCollection::Playlist(_) if names(&built.named.playlists) => {
                (&mut state.read.playlists, songs.iter().map(kept).collect())
            }
            CatalogCollection::Artist(_) if names(&built.named.artists) => {
                if state.generation == generation {
                    state
                        .read
                        .artist_read
                        .insert(catalog_id.to_owned(), binding.epoch_seconds());
                }
                (
                    &mut state.read.artist_songs,
                    songs.iter().map(kept).collect(),
                )
            }
            _ => return,
        };
        if state.generation != generation || held.get(catalog_id) == Some(&songs) {
            return;
        }
        held.insert(catalog_id.to_owned(), songs);
        state.version += 1;
        binding.keep_beside_lists(HELD, state.read.stored());
    }
    binding.content_changed();
}

/// Holds the flag that says the unread items are being read, and clears it when dropped, however
/// the read ends.
struct Reading(Arc<AppleMusicBinding>);

impl Drop for Reading {
    fn drop(&mut self) {
        self.0.held.reading.store(false, Ordering::SeqCst);
    }
}

/// Reads the items `unread` names in the background, unless a read is under way, which reads
/// next what was named meanwhile. Signed out, the next sign-in starts it. When it finds an item,
/// the pages that show the library reload.
fn read_in_background(binding: &AppleMusicBinding, unread: Unread) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    if !binding.is_signed_in() {
        binding.held.state().owed = true;
        return;
    }
    if binding.held.reading.swap(true, Ordering::SeqCst) {
        return;
    }
    binding.held.state().asked = Some(binding.now());
    let Some(binding) = binding.strong() else {
        binding.held.reading.store(false, Ordering::SeqCst);
        return;
    };
    tokio::spawn(async move {
        // A read that panics must not leave the flag set, or nothing is read again.
        let reading = Reading(Arc::clone(&binding));
        let generation = binding.held.generation();
        let found = read_unread(&binding, unread.clone()).await;
        let added = binding.held.keep(&binding, generation, found);
        drop(reading);
        if added {
            binding.content_changed();
        }
        // What was named while this read ran and the last build lacks. After a sign-out, what
        // this read brought was not taken in, so everything the last build lacks is read.
        let next = {
            let state = binding.held.state();
            let signed_out = state.generation != generation;
            state
                .built
                .as_ref()
                .map(|built| {
                    if signed_out {
                        built.unread.clone()
                    } else {
                        built.unread.not_in(&unread)
                    }
                })
                .unwrap_or_default()
        };
        if !next.is_empty() {
            read_in_background(&binding, next);
        }
    });
}

/// A sign-in was handed over: the albums, artists and playlists Apple said it does not have are
/// asked for again, as the songs are, and a read owed to a sign-in starts.
pub(crate) fn signed_in(binding: &AppleMusicBinding) {
    let owed = {
        let mut state = binding.held.state();
        state.read.missing_albums.clear();
        state.read.missing_artists.clear();
        state.read.missing_artist_songs.clear();
        state.read.missing_playlists.clear();
        state.version += 1;
        mem::take(&mut state.owed)
    };
    if !owed || tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    if let Some(binding) = binding.strong() {
        tokio::spawn(async move {
            held(&binding).await;
        });
    }
}

/// Reads the songs of the albums, many at once, and of the artists and the playlists, one after
/// another, as a mix of one does, which the binding keeps, those of the artists whose songs are
/// old afresh; and the songs named on their own together, with their albums. What Apple does not
/// have is noted, and a failed read is logged.
async fn read_unread(binding: &AppleMusicBinding, unread: Unread) -> Found {
    let mut found = Found::default();
    let albums: Vec<(String, Result<Arc<Vec<Song>>>)> = stream::iter(unread.albums)
        .map(|id| async move {
            let read = async {
                let album = binding.intern(AppleIdKind::Album, &format!("{CATALOG_PREFIX}{id}"))?;
                binding
                    .catalog_collection_songs(CatalogCollection::Album(album), &id, false)
                    .await
            }
            .await;
            (id, read)
        })
        .buffered(ALBUM_READS)
        .collect()
        .await;
    for (id, read) in albums {
        match read {
            Ok(songs) => {
                let songs = songs.iter().map(|song| kept_on(song, Some(&id))).collect();
                found.read.albums.insert(id, songs);
            }
            Err(error) if is_not_found(&error) => {
                found.read.missing_albums.insert(id);
            }
            Err(error) => {
                log::warn!("Could not read the Apple Music album {id} a Rune mix names: {error:#}");
                found.failed = true;
            }
        }
    }
    for id in unread.artists {
        read_artist(binding, &id, &mut found).await;
    }
    for id in unread.stale_artists {
        read_artist_again(binding, &id, &mut found).await;
    }
    for id in unread.playlists {
        let read = async {
            let playlist =
                binding.intern(AppleIdKind::Playlist, &format!("{CATALOG_PREFIX}{id}"))?;
            binding
                .catalog_collection_songs(CatalogCollection::Playlist(playlist), &id, false)
                .await
        }
        .await;
        match read {
            Ok(songs) => {
                found
                    .read
                    .playlists
                    .insert(id, songs.iter().map(kept).collect());
            }
            Err(error) if is_not_found(&error) => {
                found.read.missing_playlists.insert(id);
            }
            Err(error) => {
                log::warn!(
                    "Could not read the Apple Music playlist {id} a Rune mix names: {error:#}"
                );
                found.failed = true;
            }
        }
    }
    if !unread.tracks.is_empty() {
        read_tracks(binding, &unread.tracks, &mut found).await;
    }
    found
}

/// The songs a mix of the catalog artist `id` plays, as the binding keeps them, or else read
/// (`afresh`, whatever it keeps).
async fn artist_songs(
    binding: &AppleMusicBinding,
    id: &str,
    afresh: bool,
) -> Result<Arc<Vec<Song>>> {
    let artist = binding.intern(AppleIdKind::Artist, &format!("{CATALOG_PREFIX}{id}"))?;
    binding
        .catalog_collection_songs(CatalogCollection::Artist(artist), id, afresh)
        .await
}

/// Reads the catalog artist `id` into `found`: its name, which it is listed by, then the songs a
/// mix of it plays.
async fn read_artist(binding: &AppleMusicBinding, id: &str, found: &mut Found) {
    let generation = binding.generation();
    match binding.read_catalog_artist(id).await {
        Ok(artist) => {
            let attributes = artist.attributes.unwrap_or_default();
            let name = attributes.name.unwrap_or_default();
            if let Ok(numbered) =
                binding.intern(AppleIdKind::Artist, &format!("{CATALOG_PREFIX}{id}"))
            {
                binding.remember_collection(
                    generation,
                    AppleIdKind::Artist,
                    numbered,
                    &name,
                    attributes.artwork.as_ref(),
                );
            }
            found.read.artists.insert(id.to_owned(), name);
        }
        Err(error) if is_not_found(&error) => {
            found.read.missing_artists.insert(id.to_owned());
            return;
        }
        Err(error) => {
            log::warn!("Could not read the Apple Music artist {id} a Rune mix names: {error:#}");
            found.failed = true;
            return;
        }
    }
    match artist_songs(binding, id, false).await {
        Ok(songs) => {
            let songs = songs.iter().map(kept).collect();
            found.read.artist_songs.insert(id.to_owned(), songs);
            found
                .read
                .artist_read
                .insert(id.to_owned(), binding.epoch_seconds());
        }
        Err(error) if is_not_found(&error) => {
            found.read.missing_artist_songs.insert(id.to_owned());
        }
        Err(error) => {
            log::warn!(
                "Could not read the songs of the Apple Music artist {id} a Rune mix names: {error:#}"
            );
            found.failed = true;
        }
    }
}

/// Reads the songs of the catalog artist `id` again into `found`, as Apple has them now, so its
/// new releases come in. An artist gone from the catalog keeps what was read, and is not asked
/// for again until its songs grow old once more.
async fn read_artist_again(binding: &AppleMusicBinding, id: &str, found: &mut Found) {
    match artist_songs(binding, id, true).await {
        Ok(songs) => {
            let songs = songs.iter().map(kept).collect();
            found.read.artist_songs.insert(id.to_owned(), songs);
            found
                .read
                .artist_read
                .insert(id.to_owned(), binding.epoch_seconds());
        }
        Err(error) if is_not_found(&error) => {
            found
                .read
                .artist_read
                .insert(id.to_owned(), binding.epoch_seconds());
        }
        Err(error) => {
            log::warn!(
                "Could not read the songs of the Apple Music artist {id} a Rune mix names again: {error:#}"
            );
            found.failed = true;
        }
    }
}

/// Reads the catalog songs of `tracks` together, with their albums, into `found`. A song Apple
/// does not answer with is one it does not have, which the binding remembers.
async fn read_tracks(binding: &AppleMusicBinding, tracks: &[String], found: &mut Found) {
    let generation = binding.generation();
    let ids: Vec<String> = tracks
        .iter()
        .filter_map(|track| song_of_track(track).map(|(_, id)| id.to_owned()))
        .collect();
    let read = match binding.read_catalog_songs_with_albums(&ids).await {
        Ok(read) => read,
        Err(error) => {
            log::warn!("Could not read the Apple Music songs Rune's records name: {error}");
            found.failed = true;
            return;
        }
    };
    let mut read: HashMap<String, Song> = read
        .iter()
        .map(|song| (catalog_track(&song.id), kept(song)))
        .collect();
    for track in tracks {
        match read.remove(track) {
            Some(song) => {
                found.read.tracks.insert(track.clone(), song);
            }
            None => binding.remember_missing(generation, track),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::service::rune_mixes::StoredQuery;
    use crate::service::rune_playlists::{StoredEntry, TrackAbout};

    fn mix(queries: &[(&str, &str)]) -> StoredMix {
        StoredMix {
            id: "m".into(),
            name: "Mix".into(),
            group: "Favorite".into(),
            mode: None,
            queries: queries
                .iter()
                .map(|(operator, parameter)| StoredQuery {
                    operator: (*operator).to_owned(),
                    parameter: (*parameter).to_owned(),
                })
                .collect(),
        }
    }

    fn playlist(tracks: &[&str]) -> StoredPlaylist {
        StoredPlaylist {
            id: "p".into(),
            name: "Playlist".into(),
            group: "Favorite".into(),
            entries: tracks
                .iter()
                .map(|track| StoredEntry {
                    occurrence: (*track).to_owned(),
                    track: (*track).to_owned(),
                    about: TrackAbout::default(),
                })
                .collect(),
            imported_from: None,
        }
    }

    fn like(id: &str, liked: bool) -> StoredLike {
        StoredLike {
            id: id.to_owned(),
            liked,
        }
    }

    /// A catalog song on the catalog album `album`, `(id, name)`, released in `year`.
    fn song(id: &str, album: Option<(&str, &str)>, year: u32) -> Song {
        let mut song = json!({
            "id": id,
            "type": "songs",
            "attributes": {
                "name": id,
                "artistName": "Singer",
                "releaseDate": format!("{year}-01-01"),
                "playParams": {"id": id, "kind": "song"},
            },
        });
        if let Some((album, name)) = album {
            song["attributes"]["albumName"] = Value::from(name);
            song["relationships"] = json!({"albums": {"data": [{"id": album, "type": "albums"}]}});
        }
        serde_json::from_value(song).unwrap()
    }

    /// Songs that are twins, and library songs Apple does not have.
    #[derive(Default)]
    struct Facts {
        twins: Vec<(&'static str, &'static str)>,
        gone: Vec<&'static str>,
    }

    impl SongFacts for Facts {
        fn like_keys(&self, track: &str) -> Vec<String> {
            match self
                .twins
                .iter()
                .find(|(library, catalog)| track == *library || track == *catalog)
            {
                Some((library, catalog)) => vec![(*library).to_owned(), (*catalog).to_owned()],
                None => vec![track.to_owned()],
            }
        }

        fn gone(&self, track: &str) -> bool {
            self.gone.contains(&track)
        }
    }

    #[test]
    fn a_read_of_an_artists_songs_is_taken_in_only_when_it_is_newer_than_the_one_held() {
        let read_at = |id: &str, at: u64| {
            let mut read = Read::default();
            read.artist_songs
                .insert("330".into(), vec![song(id, Some(("1950", "Twin")), 2020)]);
            read.artist_read.insert("330".into(), at);
            read
        };
        let mut held = read_at("9521", 200);
        // A read that began earlier, and ends after a page took in a newer one, changes nothing.
        assert!(!held.extend(read_at("9501", 100)));
        assert_eq!(held.artist_songs["330"][0].id, "9521");
        assert_eq!(held.artist_read["330"], 200);
        // A newer read is taken in, with its time.
        assert!(held.extend(read_at("9502", 300)));
        assert_eq!(held.artist_songs["330"][0].id, "9502");
        assert_eq!(held.artist_read["330"], 300);
    }

    #[test]
    fn the_catalog_items_of_mixes_and_playlists_are_named_once_and_nothing_else_is() {
        let named = Named::of(
            &[
                mix(&[
                    ("lib::album", "catalog:1600"),
                    ("lib::album", "l.alb1"),
                    ("lib::album", "rune-album:1700"),
                    ("lib::track", "catalog:900"),
                    ("lib::track", "i.s1"),
                    ("lib::artist", "catalog:301"),
                    ("lib::artist", "name:ann"),
                    ("lib::playlist", "catalog:pl.cat1"),
                    ("lib::playlist", "rune-playlist:p1"),
                ]),
                mix(&[
                    ("lib::album", "catalog:1600"),
                    ("lib::track", "catalog:777"),
                ]),
            ],
            &[playlist(&[
                "i.s2",
                "catalog:777",
                "catalog:778",
                "not a key",
            ])],
            &[
                like("catalog:779", true),
                like("catalog:780", false),
                like("i.s3", true),
                like("catalog:777", true),
            ],
            &Facts::default(),
        );
        assert_eq!(named.albums, ["1600"]);
        // A catalog artist counts, an artist of the library's artist list does not.
        assert_eq!(named.artists, ["301"]);
        assert_eq!(named.playlists, ["pl.cat1"]);
        assert_eq!(
            named.tracks,
            ["catalog:900", "catalog:777", "catalog:778", "catalog:779"]
        );
    }

    #[test]
    fn a_catalog_like_unliked_under_its_library_key_alone_is_not_held() {
        let facts = Facts {
            twins: vec![("i.s1", "catalog:900")],
            ..Facts::default()
        };
        // Liked as the catalog song, then unliked by the library copy's key while the song list
        // did not name the twin yet.
        let unliked = Named::of(
            &[],
            &[],
            &[like("catalog:900", true), like("i.s1", false)],
            &facts,
        );
        assert!(unliked.tracks.is_empty());
        // Liked again under either key, it counts once more.
        for again in ["catalog:900", "i.s1"] {
            let liked = Named::of(
                &[],
                &[],
                &[
                    like("catalog:900", true),
                    like("i.s1", false),
                    like(again, true),
                ],
                &facts,
            );
            assert_eq!(liked.tracks.len(), 1, "{again}");
        }
        // An unrelated unlike does not matter.
        let other = Named::of(
            &[],
            &[],
            &[like("catalog:900", true), like("i.s2", false)],
            &facts,
        );
        assert_eq!(other.tracks.len(), 1);
    }

    #[test]
    fn a_playlist_entry_whose_library_song_is_gone_names_its_catalog_copy() {
        let mut entries = playlist(&["i.s1", "i.s2"]);
        entries.entries[0].about.catalog_id = Some("900".into());
        entries.entries[1].about.catalog_id = Some("901".into());
        let facts = Facts {
            gone: vec!["i.s1"],
            ..Facts::default()
        };
        let named = Named::of(&[], &[entries], &[], &facts);
        assert_eq!(named.tracks, ["catalog:900"]);
    }

    #[test]
    fn a_playlist_nobody_can_read_is_not_asked_for_again() {
        let named = Named {
            playlists: vec!["pl.cat1".into(), "pl.cat2".into()],
            ..Named::default()
        };
        let mut read = Read::default();
        read.missing_playlists.insert("pl.cat1".into());
        read.missing_playlists.insert("pl.old".into());
        // The memory forgets the playlists no mix names any more.
        assert!(!read.keep_only(&named));
        assert_eq!(
            read.missing_playlists.iter().collect::<Vec<_>>(),
            ["pl.cat1"]
        );
    }

    /// An album of the library, of the catalog album `catalog_id`.
    fn library_album(id: &str, catalog_id: &str) -> LibraryAlbum {
        serde_json::from_value(json!({
            "id": id,
            "type": "library-albums",
            "attributes": {"name": "Night"},
            "relationships": {"catalog": {"data": [{"id": catalog_id, "type": "albums"}]}},
        }))
        .unwrap()
    }

    #[test]
    fn a_song_named_on_its_own_brings_its_album_holding_only_it_and_old_artists_are_read_again() {
        let named = Named {
            albums: vec!["1600".into()],
            artists: vec!["301".into(), "302".into()],
            playlists: vec!["pl.cat1".into()],
            tracks: vec!["catalog:1601".into(), "catalog:900".into()],
        };
        let mut read = Read::default();
        read.albums.insert(
            "1600".into(),
            vec![
                song("1601", Some(("1600", "Remixes")), 2021),
                song("1602", Some(("1600", "Remixes")), 2020),
            ],
        );
        read.tracks.insert(
            "catalog:900".into(),
            song("900", Some(("1800", "Night")), 2019),
        );
        read.tracks.insert(
            "catalog:1601".into(),
            song("1601", Some(("1600", "Remixes")), 2021),
        );
        // A playlist brings the songs it holds now, one of them on no album.
        read.playlists.insert(
            "pl.cat1".into(),
            vec![
                song("2201", Some(("2200", "Picks")), 2022),
                song("2202", None, 2022),
            ],
        );
        // An artist brings the songs a mix of it plays, and is listed once its name is read.
        read.artists.insert("301".into(), "Singer".into());
        read.artist_songs.insert(
            "302".into(),
            vec![
                song("1602", Some(("1600", "Remixes")), 2020),
                song("1901", Some(("1900", "Sessions")), 2018),
            ],
        );
        let held = Held::built(&named, &read);
        let ids: Vec<&str> = held.songs.iter().map(|song| song.id.as_str()).collect();
        assert_eq!(ids, ["1601", "1602", "1901", "2201", "2202", "900"]);
        assert_eq!(
            held.artists(),
            [HeldArtist {
                id: "301".into(),
                name: "Singer".into()
            }]
        );

        // The songs of an artist read a day ago, or at a time not known, are read again.
        let day = ARTIST_SONGS_AGE.as_secs();
        let at = 10 * day;
        read.artist_read.insert("302".into(), at - day + 1);
        assert!(read.stale_artists(at).is_empty());
        read.artist_read.insert("302".into(), at - day);
        assert_eq!(read.stale_artists(at), ["302"]);
        read.artist_read.remove("302");
        assert_eq!(read.stale_artists(at), ["302"]);

        // In the song list, the song of an album the library holds by its catalog id is on that
        // album, the others on the albums Rune holds, and a song the library has a copy of is
        // the library's.
        let own: AppleSongs = serde_json::from_value(json!({
            "songs": [{"id": "i.copy", "type": "library-songs", "attributes": {
                "name": "Copy", "playParams": {"id": "i.copy", "kind": "song", "catalogId": "1602"},
            }}],
            "twins": {"1602": "i.copy"},
        }))
        .unwrap();
        let store = HeldStore::default();
        let library = Arc::new(vec![library_album("l.night", "1800")]);
        let songs = store.songs_with(Arc::new(own), &Arc::new(held), Some(library.clone()));
        let listed: Vec<(&str, Option<&str>)> = songs
            .entries()
            .map(|(track, song)| (track, song.album_id()))
            .collect();
        assert_eq!(
            listed,
            [
                ("i.copy", None),
                ("catalog:1601", Some("rune-album:1600")),
                ("catalog:1901", Some("rune-album:1900")),
                ("catalog:2201", Some("rune-album:2200")),
                ("catalog:2202", None),
                ("catalog:900", Some("l.night")),
            ]
        );
        assert!(songs.holds("catalog:900") && !songs.holds("i.copy"));
        assert_eq!(songs.library_entries().count(), 6);

        // The albums Rune holds follow the library's, each of the songs Rune holds of it, of the
        // year of its earliest song.
        let albums = store.albums_with(library, Some(songs));
        let listed: Vec<_> = albums
            .iter()
            .map(|album| {
                let attributes = album.attributes.as_ref().unwrap();
                (
                    album.id.as_str(),
                    attributes.name.as_deref(),
                    attributes.track_count,
                    release_year(attributes.release_date.as_deref()),
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                ("l.night", Some("Night"), None, None),
                ("rune-album:1600", Some("Remixes"), Some(1), Some(2021)),
                ("rune-album:1900", Some("Sessions"), Some(1), Some(2018)),
                ("rune-album:2200", Some("Picks"), Some(1), Some(2022)),
            ]
        );
    }

    #[test]
    fn a_song_only_a_playlist_lists_moves_into_the_library_once_rune_holds_it() {
        let named = Named {
            albums: Vec::new(),
            artists: Vec::new(),
            playlists: Vec::new(),
            tracks: vec!["catalog:900".into(), "catalog:1602".into()],
        };
        let mut read = Read::default();
        read.tracks.insert(
            "catalog:900".into(),
            song("900", Some(("1800", "Night")), 2019),
        );
        read.tracks.insert(
            "catalog:1602".into(),
            song("1602", Some(("1600", "Remixes")), 2020),
        );
        let held = Held::built(&named, &read);
        // One of the user's playlists lists 900, which the library does not hold, and 1602,
        // which the library holds a copy of.
        let listed = |id: &str| {
            json!({"id": id, "type": "songs", "attributes": {
                "name": id, "playParams": {"id": id, "kind": "song", "catalogId": id},
            }})
        };
        let own: AppleSongs = serde_json::from_value(json!({
            "songs": [
                {"id": "i.copy", "type": "library-songs", "attributes": {
                    "name": "Copy", "playParams": {"id": "i.copy", "kind": "song", "catalogId": "1602"},
                }},
                listed("900"),
                listed("1602"),
                listed("700"),
            ],
            "twins": {"1602": "i.copy"},
        }))
        .unwrap();
        let songs = HeldStore::default().songs_with(Arc::new(own), &Arc::new(held), None);
        let listed: Vec<(&str, Option<&str>)> = songs
            .entries()
            .map(|(track, song)| (track, song.album_id()))
            .collect();
        // 900 is listed once, among the songs Rune holds, on its album; 1602 stays the library's
        // copy; 700 is still only the playlist's.
        assert_eq!(
            listed,
            [
                ("i.copy", None),
                ("catalog:700", None),
                ("catalog:900", Some("rune-album:1800")),
            ]
        );
        assert!(songs.holds("catalog:900") && !songs.holds("catalog:700"));
        let library: Vec<&str> = songs.library_entries().map(|(track, _)| track).collect();
        assert_eq!(library, ["i.copy", "catalog:900"]);
    }

    #[test]
    fn an_album_rune_holds_takes_the_release_of_its_earliest_song() {
        let named = Named {
            albums: vec!["1600".into()],
            ..Named::default()
        };
        let mut read = Read::default();
        read.albums.insert(
            "1600".into(),
            vec![
                song("1601", Some(("1600", "Remixes")), 2021),
                song("1602", Some(("1600", "Remixes")), 2020),
            ],
        );
        let store = HeldStore::default();
        let songs = store.songs_with(Arc::default(), &Arc::new(Held::built(&named, &read)), None);
        let albums = store.albums_with(Arc::default(), Some(songs));
        let attributes = albums[0].attributes.as_ref().unwrap();
        assert_eq!(
            (
                attributes.track_count,
                release_year(attributes.release_date.as_deref())
            ),
            (Some(2), Some(2020))
        );
    }
}
