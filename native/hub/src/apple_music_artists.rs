//! The artist list of an Apple Music library. Apple lists every credit as an artist of its own,
//! "A & B" beside "A", and the same person under every spelling the credits use, so the list
//! shows the artists Apple's catalog names instead, one per person. The text of a credit is
//! never split: a credit is cut into artists only by the catalog artists Apple names for a song
//! or for a library artist. A song and a library artist Apple names no catalog artist for stand
//! under their credit as it is, whole, until Apple names some.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::mem;
use std::sync::Arc;

use apple_music::models::{Artist, Artwork, LibraryArtist, LibrarySong};
use serde::{Deserialize, Serialize};

use crate::apple_music_binding::listed_track;

/// What the ids of the artists of the library's artist list that Apple names no catalog artist
/// for hold before the artist's name key: the key of its name, or of one of its other
/// spellings. Apple's library ids start with a letter and a dot and its catalog ids are digits,
/// so no id Apple gives starts with it.
pub(crate) const NAMED_ARTIST: &str = "name:";

/// What the ids of the artists of the library's artist list that Apple names a catalog artist
/// for hold before the catalog artist's id, which no id Apple gives starts with either.
pub(crate) const CATALOG_ARTIST: &str = "artist:";

/// How the library's artist list compares names: case and the spaces around them ignored.
fn artist_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The id, before it is numbered, of the artist of the library's artist list that a credit
/// names as a whole, as Apple spells it; `None` when the credit names nobody. The credit is
/// not split: this is the artist a song stands under while Apple names no catalog artist for
/// it.
pub(crate) fn named_artist_remote(credit: &str) -> Option<String> {
    let key = artist_key(credit);
    (!key.is_empty()).then(|| format!("{NAMED_ARTIST}{key}"))
}

/// Whether `remote` is the id of an artist of the library's artist list.
pub(crate) fn is_named_artist(remote: &str) -> bool {
    remote.starts_with(NAMED_ARTIST) || remote.starts_with(CATALOG_ARTIST)
}

/// The id of the artist of the library's artist list that Apple's catalog artist `catalog_id`
/// is.
pub(crate) fn catalog_artist_remote(catalog_id: &str) -> String {
    format!("{CATALOG_ARTIST}{catalog_id}")
}

/// The name of a library artist.
fn library_artist_name(artist: &LibraryArtist) -> &str {
    artist
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.name.as_deref())
        .unwrap_or_default()
}

/// The name of a catalog artist.
fn catalog_artist_name(artist: &Artist) -> &str {
    artist
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.name.as_deref())
        .unwrap_or_default()
}

/// The picture of a catalog artist.
fn catalog_artist_artwork(artist: &Artist) -> Option<&Artwork> {
    artist.attributes.as_ref()?.artwork.as_ref()
}

/// The credit of a song, as Apple spells it.
fn song_credit(song: &LibrarySong) -> Option<&str> {
    song.attributes.as_ref()?.artist_name.as_deref()
}

/// A catalog artist as Apple names it for songs of the library.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct CatalogArtist {
    /// Its name, as the storefront spells it.
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) artwork: Option<Artwork>,
}

/// The catalog artists Apple names for the catalog songs of the library, so the artist list
/// tells artists apart by Apple's identity rather than by the text of their credits. A song is
/// asked about once: neither its artists nor its catalog id change.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct SongArtists {
    /// Each catalog artist named, by its catalog id.
    pub(crate) artists: BTreeMap<String, CatalogArtist>,
    /// The catalog artists of each catalog song asked about, by the song's catalog id, in
    /// Apple's order; none for a song the catalog does not know.
    pub(crate) songs: BTreeMap<String, Vec<String>>,
}

impl SongArtists {
    /// The catalog id a song of the song list is asked about by: the catalog song a library
    /// song is a copy of, or the catalog song an entry of a playlist is.
    fn song_id(song: &LibrarySong) -> Option<&str> {
        song.store_id()
    }

    /// The catalog ids of `songs`, the library's songs, not asked about yet, each once.
    pub(crate) fn missing<'a>(
        &self,
        songs: impl IntoIterator<Item = &'a LibrarySong>,
    ) -> Vec<String> {
        let mut seen = HashSet::new();
        songs
            .into_iter()
            .filter_map(Self::song_id)
            .filter(|id| !self.songs.contains_key(*id) && seen.insert(*id))
            .map(str::to_owned)
            .collect()
    }

    /// Takes in what Apple answered for the songs `asked`: their artists, and none for a song it
    /// left out, which it does not know. An artist Apple names by its id alone keeps the name and
    /// the picture it was named with before.
    pub(crate) fn add(&mut self, asked: &[String], found: Vec<(String, Vec<Artist>)>) {
        for id in asked {
            self.songs.entry(id.clone()).or_default();
        }
        for (song, artists) in found {
            let mut ids = Vec::with_capacity(artists.len());
            for artist in artists {
                if artist.id.is_empty() {
                    continue;
                }
                let (name, artwork) = (
                    catalog_artist_name(&artist),
                    catalog_artist_artwork(&artist),
                );
                let kept = self
                    .artists
                    .entry(artist.id.clone())
                    .or_insert_with(|| CatalogArtist {
                        name: String::new(),
                        artwork: None,
                    });
                if !name.is_empty() {
                    name.clone_into(&mut kept.name);
                }
                if let Some(artwork) = artwork {
                    kept.artwork = Some(artwork.clone());
                }
                ids.push(artist.id);
            }
            self.songs.insert(song, ids);
        }
    }

    /// The catalog artists Apple named for a song; `None` for one it was not asked about, does
    /// not know, or names none for.
    pub(crate) fn of(&self, song: &LibrarySong) -> Option<&[String]> {
        self.songs
            .get(Self::song_id(song)?)
            .map(Vec::as_slice)
            .filter(|ids| !ids.is_empty())
    }
}

/// An artist of the library's artist list: a catalog artist Apple names, or else a credit
/// Apple names none for, as a whole.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NamedArtist {
    /// Apple's catalog id of the artist, when Apple names one for it.
    catalog: Option<String>,
    /// The key of its name.
    key: String,
    /// The name shown: as the catalog names the catalog artist, or else as the credit is first
    /// spelled, whole.
    pub(crate) name: String,
    /// The other spellings its credits use, as "サワノヒロユキ" for "Hiroyuki Sawano", each once.
    pub(crate) aliases: Vec<String>,
    /// The library artists that are it: those Apple maps to its catalog artist, or those whose
    /// whole names are its name, in the order Apple lists them.
    pub(crate) credits: Vec<String>,
    /// Its picture: the catalog artist's, or else the one of a library artist that is it.
    pub(crate) artwork: Option<Artwork>,
    /// The tracks of the song list it was built from that credit it.
    songs: Arc<HashSet<String>>,
}

impl NamedArtist {
    /// The id it is numbered by: by its catalog artist, or else by the key of its name.
    pub(crate) fn remote(&self) -> String {
        match &self.catalog {
            Some(id) => catalog_artist_remote(id),
            None => format!("{NAMED_ARTIST}{}", self.key),
        }
    }

    /// Whether the song of `track` credits it, as the song list it was built from says.
    pub(crate) fn credits_track(&self, track: &str) -> bool {
        self.songs.contains(track)
    }

    /// What an artist moved out of the build leaves behind.
    fn placeholder() -> Self {
        Self {
            catalog: None,
            key: String::new(),
            name: String::new(),
            aliases: Vec::new(),
            credits: Vec::new(),
            artwork: None,
            songs: Arc::default(),
        }
    }
}

/// The library's artist list, in the order its artists are first credited: by the library's
/// artists, then by the songs.
#[derive(Debug, Default)]
pub(crate) struct NamedArtists {
    artists: Vec<NamedArtist>,
    /// Where each artist with a catalog artist is in `artists`, by its catalog id.
    by_catalog: HashMap<String, usize>,
    /// Where each artist is in `artists`, by the key of its name and of each of its other
    /// spellings. A catalog artist's name comes before a spelling of another.
    by_key: HashMap<String, usize>,
}

/// What `NamedArtists::new` builds before it puts the artists in the order of their first
/// credit.
#[derive(Default)]
struct Building {
    artists: Vec<NamedArtist>,
    songs: Vec<HashSet<String>>,
    by_catalog: HashMap<String, usize>,
    by_key: HashMap<String, usize>,
    /// The order the artists are first credited in, and whether each was.
    order: Vec<usize>,
    credited: Vec<bool>,
}

impl Building {
    fn push(&mut self, artist: NamedArtist) -> usize {
        self.artists.push(artist);
        self.songs.push(HashSet::new());
        self.credited.push(false);
        self.artists.len() - 1
    }

    /// The artist of Apple's catalog artist `id`, made when it is new.
    fn catalog_artist(&mut self, id: &str, name: &str, artwork: Option<&Artwork>) -> usize {
        if let Some(&at) = self.by_catalog.get(id) {
            let entry = &mut self.artists[at];
            if entry.artwork.is_none() {
                entry.artwork = artwork.cloned();
            }
            return at;
        }
        let key = artist_key(name);
        let at = self.push(NamedArtist {
            catalog: Some(id.to_owned()),
            key: key.clone(),
            name: name.trim().to_owned(),
            aliases: Vec::new(),
            credits: Vec::new(),
            artwork: artwork.cloned(),
            songs: Arc::default(),
        });
        self.by_catalog.insert(id.to_owned(), at);
        self.by_key.entry(key).or_insert(at);
        at
    }

    /// Notes `spelling` as another spelling of the artist at `at`, unless its key names an
    /// artist already.
    fn alias(&mut self, at: usize, spelling: &str) {
        let key = artist_key(spelling);
        if key.is_empty() || self.by_key.contains_key(&key) {
            return;
        }
        self.by_key.insert(key, at);
        self.artists[at].aliases.push(spelling.trim().to_owned());
    }

    /// The artist a whole credit names: one of that name or spelling, or else one known by the
    /// credit alone, made when it is new.
    fn named(&mut self, name: &str) -> Option<usize> {
        let key = artist_key(name);
        if key.is_empty() {
            return None;
        }
        if let Some(&at) = self.by_key.get(&key) {
            return Some(at);
        }
        let at = self.push(NamedArtist {
            catalog: None,
            key: key.clone(),
            name: name.trim().to_owned(),
            aliases: Vec::new(),
            credits: Vec::new(),
            artwork: None,
            songs: Arc::default(),
        });
        self.by_key.insert(key, at);
        Some(at)
    }

    /// Notes the artist at `at` as credited, so the list shows it in the order of first credit.
    fn credited(&mut self, at: usize) {
        if !self.credited[at] {
            self.credited[at] = true;
            self.order.push(at);
        }
    }

    fn credit_song(&mut self, at: usize, song: &LibrarySong) {
        self.credited(at);
        if let Some(track) = listed_track(song) {
            self.songs[at].insert(track);
        }
    }
}

/// The catalog artists Apple names for a library artist, with their ids.
fn catalog_of(artist: &LibraryArtist) -> Vec<Artist> {
    artist
        .catalog_artists()
        .into_iter()
        .filter(|catalog| !catalog.id.is_empty())
        .collect()
}

/// Which library artists the songs are credited to, by the id of the library artist and by the
/// key of the credit's name, and which of those have a song Apple names no catalog artist for.
/// Built once, so telling whether an artist's songs are all catalogued is a lookup.
struct SongCredits<'a> {
    by_id: HashSet<&'a str>,
    by_key: HashSet<String>,
    uncatalogued_by_id: HashSet<&'a str>,
    uncatalogued_by_key: HashSet<String>,
}

impl<'a> SongCredits<'a> {
    fn new(songs: &[&'a LibrarySong], song_artists: &SongArtists) -> Self {
        let mut credits = Self {
            by_id: HashSet::new(),
            by_key: HashSet::new(),
            uncatalogued_by_id: HashSet::new(),
            uncatalogued_by_key: HashSet::new(),
        };
        for song in songs {
            let catalogued = song_artists.of(song).is_some();
            let id = song.artist_id();
            let key = song_credit(song).map(artist_key);
            if let Some(id) = id {
                credits.by_id.insert(id);
                if !catalogued {
                    credits.uncatalogued_by_id.insert(id);
                }
            }
            if let Some(key) = key {
                if !catalogued {
                    credits.uncatalogued_by_key.insert(key.clone());
                }
                credits.by_key.insert(key);
            }
        }
        credits
    }

    /// Whether the library artist has songs among those, and Apple names catalog artists for all
    /// of them, so the artists those name stand for it.
    fn all_catalogued(&self, artist: &LibraryArtist) -> bool {
        let key = artist_key(library_artist_name(artist));
        let id = artist.id.as_str();
        let has_songs = self.by_id.contains(id) || self.by_key.contains(&key);
        let lacks = self.uncatalogued_by_id.contains(id) || self.uncatalogued_by_key.contains(&key);
        has_songs && !lacks
    }
}

impl NamedArtists {
    /// The artist list of the library's artists `library` and its songs, telling who is who by
    /// the catalog artists Apple named for the songs (`song_artists`) and for the library's
    /// artists. No credit is split by its text:
    /// - One artist per catalog artist, named as the catalog names it, with each song Apple
    ///   names it for, so a song of several artists is listed under each and under no other.
    /// - A library artist Apple names catalog artists for is those artists, as Apple maps it.
    /// - A credit, of a song or of a library artist, that names exactly one catalog artist is
    ///   another spelling of it, as "サワノヒロユキ" is of "Hiroyuki Sawano". A credit that names
    ///   several gives no spelling: it could only be told apart by cutting its text.
    /// - A song Apple names no catalog artist for stands under the catalog artists of its
    ///   library artist when Apple maps that to some.
    /// - Any other song, and a library artist that names none, stand
    ///   under one artist of the credit as a whole: the artist of that name or spelling, or
    ///   else one known by the credit alone. A library artist whose songs all have catalog
    ///   artists is left out then, as the artists they name stand for it.
    ///
    /// Only songs in the library count, so `songs` are the library's songs: the song list also
    /// holds songs of playlists that are not (see `AppleSongs::library_entries`).
    pub(crate) fn new<'a>(
        library: &[LibraryArtist],
        songs: impl IntoIterator<Item = &'a LibrarySong>,
        song_artists: &SongArtists,
    ) -> Self {
        let mut building = Building::default();
        let songs: Vec<&LibrarySong> = songs.into_iter().collect();
        let library: Vec<(&LibraryArtist, Vec<Artist>)> = library
            .iter()
            .map(|artist| (artist, catalog_of(artist)))
            .collect();
        // The catalog artists of the songs and of the library's artists, before any spelling
        // is taken for another artist.
        for song in &songs {
            for id in song_artists.of(song).unwrap_or_default() {
                if let Some(artist) = song_artists.artists.get(id) {
                    building.catalog_artist(id, &artist.name, artist.artwork.as_ref());
                }
            }
        }
        for (_, catalog) in &library {
            for item in catalog {
                building.catalog_artist(
                    &item.id,
                    catalog_artist_name(item),
                    catalog_artist_artwork(item),
                );
            }
        }
        // The other spellings the credits use: a credit naming one catalog artist.
        for song in &songs {
            if let (Some([id]), Some(credit)) = (song_artists.of(song), song_credit(song))
                && let Some(&at) = building.by_catalog.get(id)
            {
                building.alias(at, credit);
            }
        }
        for (artist, catalog) in &library {
            if let [item] = &catalog[..]
                && let Some(&at) = building.by_catalog.get(&item.id)
            {
                building.alias(at, library_artist_name(artist));
            }
        }
        // The library's artists: those of catalog artists are them, the others stand by their
        // names as a whole.
        let mut mapped: HashMap<&str, Vec<usize>> = HashMap::new();
        let credits = SongCredits::new(&songs, song_artists);
        for (artist, catalog) in &library {
            // A library artist's picture is the one of its first catalog artist.
            let picture = catalog.first().and_then(catalog_artist_artwork);
            let catalog: Vec<usize> = catalog
                .iter()
                .filter_map(|item| building.by_catalog.get(&item.id).copied())
                .collect();
            let named: Vec<usize> = if catalog.is_empty() {
                if credits.all_catalogued(artist) {
                    continue;
                }
                building
                    .named(library_artist_name(artist))
                    .into_iter()
                    .collect()
            } else {
                mapped.insert(artist.id.as_str(), catalog.clone());
                catalog
            };
            for at in named {
                building.credited(at);
                let entry = &mut building.artists[at];
                if !entry.credits.contains(&artist.id) {
                    entry.credits.push(artist.id.clone());
                }
                // A library artist gives its artist its picture when it has none.
                if entry.artwork.is_none() {
                    entry.artwork = picture.cloned();
                }
            }
        }
        // Each song stands under its catalog artists, or else under the catalog artists of its
        // library artist, or else under its credit as a whole.
        for song in &songs {
            match song_artists.of(song) {
                Some(ids) => {
                    for id in ids {
                        if let Some(&at) = building.by_catalog.get(id) {
                            building.credit_song(at, song);
                        }
                    }
                }
                None => {
                    let by_library = song.artist_id().and_then(|id| mapped.get(id));
                    if let Some(by_library) = by_library {
                        for &at in by_library {
                            building.credit_song(at, song);
                        }
                    } else if let Some(at) = song_credit(song).and_then(|name| building.named(name))
                    {
                        building.credit_song(at, song);
                    }
                }
            }
        }
        // In the order of first credit, leaving out the catalog artists nothing credits.
        let mut named = Self::default();
        let Building {
            mut artists,
            mut songs,
            order,
            ..
        } = building;
        for at in order {
            let mut artist = mem::replace(&mut artists[at], NamedArtist::placeholder());
            artist.songs = Arc::new(mem::take(&mut songs[at]));
            let index = named.artists.len();
            if let Some(id) = &artist.catalog {
                named.by_catalog.insert(id.clone(), index);
            }
            named.artists.push(artist);
        }
        // The keys go to the artists the build gave them to, or to none when that one is left
        // out: a catalog artist's name first, then the other spellings.
        for (index, artist) in named.artists.iter().enumerate() {
            named.by_key.entry(artist.key.clone()).or_insert(index);
        }
        for (index, artist) in named.artists.iter().enumerate() {
            for alias in &artist.aliases {
                named.by_key.entry(artist_key(alias)).or_insert(index);
            }
        }
        named
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &NamedArtist> {
        self.artists.iter()
    }

    /// The artist an id of the list names: by its catalog artist, or by the key of its name
    /// or of another spelling its credits use. An id from before the list told artists apart
    /// by their catalog artists names the artist that spelling goes with now; one that names
    /// none, as the name of a part of a credit Apple's catalog does not name, finds nothing.
    pub(crate) fn get(&self, remote: &str) -> Option<&NamedArtist> {
        let at = if let Some(id) = remote.strip_prefix(CATALOG_ARTIST) {
            self.by_catalog.get(id)
        } else {
            self.by_key.get(remote.strip_prefix(NAMED_ARTIST)?)
        };
        at.map(|&at| &self.artists[at])
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use apple_music::models::{Artist, LibraryArtist, LibrarySong};
    use serde_json::{Value, json};

    use super::*;
    use crate::apple_music_binding::artwork_url;

    pub(crate) fn catalog(id: &str, name: &str) -> Artist {
        serde_json::from_value(json!({
            "id": id,
            "type": "artists",
            "attributes": {
                "name": name,
                "artwork": {"url": format!("https://covers.invalid/{id}/{{w}}x{{h}}{{c}}.{{f}}")},
            },
        }))
        .unwrap()
    }

    fn library(id: &str, name: &str, catalog: Vec<Artist>) -> LibraryArtist {
        serde_json::from_value(json!({
            "id": id,
            "type": "library-artists",
            "attributes": {"name": name},
            "relationships": {"catalog": {"data": catalog}},
        }))
        .unwrap()
    }

    /// A song of the library credited `credit`, of the catalog song `catalog_id` when it has
    /// one, and of the library artist `artist_id` when it names one.
    pub(crate) fn song(
        id: &str,
        credit: &str,
        catalog_id: Option<&str>,
        artist_id: Option<&str>,
    ) -> LibrarySong {
        let mut play_params = json!({"id": id, "kind": "song"});
        if let Some(catalog_id) = catalog_id {
            play_params["catalogId"] = Value::from(catalog_id);
        }
        let mut song = json!({
            "id": id,
            "type": "library-songs",
            "attributes": {"name": id, "artistName": credit, "playParams": play_params},
        });
        if let Some(artist_id) = artist_id {
            song["relationships"] = json!({
                "artists": {"data": [{"id": artist_id, "type": "library-artists"}]},
            });
        }
        serde_json::from_value(song).unwrap()
    }

    /// The ids of the songs the artist `remote` names credit it.
    fn songs_of(named: &NamedArtists, remote: &str, songs: &[LibrarySong]) -> Vec<String> {
        let artist = named.get(remote).unwrap_or_else(|| panic!("{remote}"));
        songs
            .iter()
            .filter(|song| artist.credits_track(&song.id))
            .map(|song| song.id.clone())
            .collect()
    }

    fn listed(named: &NamedArtists) -> Vec<(String, String)> {
        named
            .iter()
            .map(|artist| (artist.name.clone(), artist.remote()))
            .collect()
    }

    fn pair(name: &str, remote: &str) -> (String, String) {
        (name.to_owned(), remote.to_owned())
    }

    #[test]
    fn a_library_artist_is_the_catalog_artist_apple_maps_it_to_without_splitting_its_name() {
        let named = NamedArtists::new(
            &[
                // Apple maps the credit to one artist: it is that artist, whatever its text.
                library(
                    "r.1",
                    "lead singer & Guest Voice",
                    vec![catalog("1", "Lead Singer")],
                ),
                // Named otherwise in the catalog, while its name is the name of another
                // catalog artist already: the mapping decides.
                library("r.2", "Lead Singer", vec![catalog("2", "Chanteur")]),
                library("r.3", "Lone Voice", Vec::new()),
                library("r.4", "Composer", vec![catalog("4", "Compositeur")]),
                // No mapping: one artist by its whole name, never split.
                library("r.5", "AC/DC", Vec::new()),
                library("r.6", "Ann & Bob", Vec::new()),
            ],
            &[],
            &SongArtists::default(),
        );
        let rows: Vec<_> = named
            .iter()
            .map(|artist| {
                (
                    artist.name.as_str(),
                    artist.remote(),
                    artist.credits.clone(),
                    artist
                        .artwork
                        .as_ref()
                        .and_then(|artwork| artwork_url(artwork, 1)),
                )
            })
            .collect();
        let cover = |id: &str| Some(format!("https://covers.invalid/{id}/1x1bb.jpg"));
        assert_eq!(
            rows,
            [
                (
                    "Lead Singer",
                    "artist:1".to_owned(),
                    vec!["r.1".to_owned()],
                    cover("1"),
                ),
                (
                    "Chanteur",
                    "artist:2".to_owned(),
                    vec!["r.2".to_owned()],
                    cover("2"),
                ),
                (
                    "Lone Voice",
                    "name:lone voice".to_owned(),
                    vec!["r.3".to_owned()],
                    None
                ),
                (
                    "Compositeur",
                    "artist:4".to_owned(),
                    vec!["r.4".to_owned()],
                    cover("4")
                ),
                (
                    "AC/DC",
                    "name:ac/dc".to_owned(),
                    vec!["r.5".to_owned()],
                    None
                ),
                (
                    "Ann & Bob",
                    "name:ann & bob".to_owned(),
                    vec!["r.6".to_owned()],
                    None
                ),
            ]
        );
        // The spellings of a library artist Apple maps to one catalog artist find it, and the
        // ids of before name it now.
        assert_eq!(
            named.get("name:composer").map(NamedArtist::remote),
            Some("artist:4".to_owned())
        );
        assert_eq!(
            named
                .get("name:lead singer & guest voice")
                .map(NamedArtist::remote),
            Some("artist:1".to_owned())
        );
        // Nothing names the parts of a credit that was not split.
        assert!(named.get("name:ann").is_none());
        assert!(named.get("name:guest voice").is_none());
        assert_eq!(
            named_artist_remote("Lead Singer "),
            Some("name:lead singer".to_owned())
        );
        assert_eq!(
            named_artist_remote("Lead Singer & Guest Voice"),
            Some("name:lead singer & guest voice".to_owned())
        );
        assert_eq!(named_artist_remote("  "), None);
    }

    #[test]
    fn a_library_artist_is_left_to_its_catalog_artists_only_when_all_its_songs_have_them() {
        let mut song_artists = SongArtists::default();
        let ids = ["c.a"].map(str::to_owned);
        song_artists.add(&ids, vec![("c.a".into(), vec![catalog("10", "Alpha")])]);
        let songs = [
            song("i.a", "Alpha", Some("c.a"), None),
            song("i.b", "Beta", None, None),
            song("i.byid", "Spelled Otherwise", None, Some("r.beta")),
        ];
        let songs: Vec<&LibrarySong> = songs.iter().collect();
        let credits = SongCredits::new(&songs, &song_artists);
        // All of Alpha's songs have catalog artists, so the artists they name stand for it.
        assert!(credits.all_catalogued(&library("r.alpha", " ALPHA ", Vec::new())));
        // One of Beta's has none, whether it is found by the artist's id or by its name.
        assert!(!credits.all_catalogued(&library("r.beta", "Beta", Vec::new())));
        assert!(!credits.all_catalogued(&library("r.beta", "Other Name", Vec::new())));
        // An artist with no songs has none to be catalogued.
        assert!(!credits.all_catalogued(&library("r.none", "Nobody", Vec::new())));
    }

    #[test]
    fn credits_text_splitting_gets_wrong_have_the_artists_the_catalog_names() {
        let mut song_artists = SongArtists::default();
        let ids = ["c.ac", "c.kh", "c.ks"].map(str::to_owned);
        song_artists.add(
            &ids,
            vec![
                ("c.ac".into(), vec![catalog("10", "AC/DC")]),
                (
                    "c.kh".into(),
                    vec![
                        catalog("120", "KOHTA YAMAMOTO"),
                        catalog("156", "Hiroyuki Sawano"),
                    ],
                ),
                (
                    "c.ks".into(),
                    vec![
                        catalog("156", "Hiroyuki Sawano"),
                        catalog("120", "KOHTA YAMAMOTO"),
                    ],
                ),
            ],
        );
        let songs = [
            song("i.ac", "AC/DC", Some("c.ac"), None),
            song("i.kh", "KOHTA YAMAMOTO,HIROYUKI SAWANO", Some("c.kh"), None),
            song("i.ks", "サワノヒロユキ/KOHTA YAMAMOTO", Some("c.ks"), None),
        ];
        let named = NamedArtists::new(&[], &songs, &song_artists);
        assert_eq!(
            listed(&named),
            [
                pair("AC/DC", "artist:10"),
                pair("KOHTA YAMAMOTO", "artist:120"),
                pair("Hiroyuki Sawano", "artist:156"),
            ]
        );
        // A joint song is listed under each of its artists, and under no other.
        assert_eq!(songs_of(&named, "artist:10", &songs), ["i.ac"]);
        assert_eq!(songs_of(&named, "artist:120", &songs), ["i.kh", "i.ks"]);
        assert_eq!(songs_of(&named, "artist:156", &songs), ["i.kh", "i.ks"]);
        // A credit of several artists gives no spelling; one of one does.
        assert_eq!(
            named.get("artist:10").unwrap().aliases,
            Vec::<String>::new()
        );
        assert!(named.get("artist:120").unwrap().aliases.is_empty());
        assert!(named.get("artist:156").unwrap().aliases.is_empty());
        assert!(named.get("name:kohta yamamoto,hiroyuki sawano").is_none());
    }

    #[test]
    fn the_same_person_under_other_spellings_is_one_artist_when_the_catalog_says_so() {
        let mut song_artists = SongArtists::default();
        let asked = ["c.ja", "c.en", "c.rev"].map(str::to_owned);
        song_artists.add(
            &asked,
            vec![
                ("c.ja".into(), vec![catalog("156", "Hiroyuki Sawano")]),
                ("c.en".into(), vec![catalog("156", "Hiroyuki Sawano")]),
                ("c.rev".into(), vec![catalog("156", "Hiroyuki Sawano")]),
            ],
        );
        let songs = [
            song("i.ja", "サワノヒロユキ", Some("c.ja"), None),
            song("i.en", "Hiroyuki Sawano", Some("c.en"), None),
            song("i.rev", "Sawano Hiroyuki", Some("c.rev"), None),
        ];
        let named = NamedArtists::new(&[], &songs, &song_artists);
        assert_eq!(listed(&named), [pair("Hiroyuki Sawano", "artist:156")]);
        assert_eq!(
            songs_of(&named, "artist:156", &songs),
            ["i.ja", "i.en", "i.rev"]
        );
        let sawano = named.get("artist:156").unwrap();
        assert_eq!(sawano.aliases, ["サワノヒロユキ", "Sawano Hiroyuki"]);
        // Its other spellings name it by the ids of before.
        for old in [
            "name:サワノヒロユキ",
            "name:sawano hiroyuki",
            "name:hiroyuki sawano",
        ] {
            assert_eq!(
                named.get(old).map(NamedArtist::remote),
                Some("artist:156".to_owned()),
                "{old}"
            );
        }
    }

    #[test]
    fn a_song_without_catalog_artists_stands_under_its_whole_credit_until_they_are_read() {
        let songs = [
            // The catalog does not know it, and another has no catalog id.
            song("i.gone", "Ann & Dee", Some("c.gone"), None),
            song("i.nc", "サワノヒロユキ/KOHTA YAMAMOTO", None, None),
            song("i.slash", "AC/DC", None, None),
        ];
        let mut song_artists = SongArtists::default();
        song_artists.add(&["c.gone".to_owned()], Vec::new());
        assert!(song_artists.missing(&songs).is_empty());
        let named = NamedArtists::new(&[], &songs, &song_artists);
        assert_eq!(
            listed(&named),
            [
                pair("Ann & Dee", "name:ann & dee"),
                pair(
                    "サワノヒロユキ/KOHTA YAMAMOTO",
                    "name:サワノヒロユキ/kohta yamamoto"
                ),
                pair("AC/DC", "name:ac/dc"),
            ]
        );
        assert_eq!(songs_of(&named, "name:ann & dee", &songs), ["i.gone"]);
        // Not read yet is the same: the song is asked about, not answered.
        let named = NamedArtists::new(&[], &songs, &SongArtists::default());
        assert_eq!(listed(&named).len(), 3);
        assert!(named.get("name:ann").is_none());

        // Once Apple names its catalog artists the song moves to them, and the credit's
        // artist is gone.
        let mut song_artists = SongArtists::default();
        song_artists.add(
            &["c.kh".to_owned()],
            vec![(
                "c.kh".into(),
                vec![
                    catalog("156", "Hiroyuki Sawano"),
                    catalog("120", "KOHTA YAMAMOTO"),
                ],
            )],
        );
        let songs = [song(
            "i.nc",
            "サワノヒロユキ/KOHTA YAMAMOTO",
            Some("c.kh"),
            None,
        )];
        let named = NamedArtists::new(&[], &songs, &song_artists);
        assert_eq!(
            listed(&named),
            [
                pair("Hiroyuki Sawano", "artist:156"),
                pair("KOHTA YAMAMOTO", "artist:120"),
            ]
        );
        assert!(named.get("name:サワノヒロユキ/kohta yamamoto").is_none());
    }

    #[test]
    fn an_artist_named_by_its_id_alone_keeps_its_name_and_picture() {
        let mut song_artists = SongArtists::default();
        let bare: Artist = serde_json::from_value(json!({"id": "156", "type": "artists"})).unwrap();
        song_artists.add(
            &["c.1".to_owned(), "c.2".to_owned()],
            vec![
                ("c.1".into(), vec![catalog("156", "Hiroyuki Sawano")]),
                ("c.2".into(), vec![bare]),
            ],
        );
        let kept = &song_artists.artists["156"];
        assert_eq!(kept.name, "Hiroyuki Sawano");
        assert!(kept.artwork.is_some());
        assert_eq!(song_artists.songs["c.2"], ["156"]);
    }

    #[test]
    fn a_library_artist_whose_songs_all_have_catalog_artists_is_left_out() {
        let mut song_artists = SongArtists::default();
        song_artists.add(
            &["c.kh".to_owned()],
            vec![(
                "c.kh".into(),
                vec![
                    catalog("120", "KOHTA YAMAMOTO"),
                    catalog("156", "Hiroyuki Sawano"),
                ],
            )],
        );
        let songs = [song(
            "i.kh",
            "KOHTA YAMAMOTO,HIROYUKI SAWANO",
            Some("c.kh"),
            Some("r.kh"),
        )];
        let named = NamedArtists::new(
            &[
                library("r.kh", "KOHTA YAMAMOTO,HIROYUKI SAWANO", Vec::new()),
                library(
                    "r.ja",
                    "サワノヒロユキ",
                    vec![catalog("156", "Hiroyuki Sawano")],
                ),
            ],
            &songs,
            &song_artists,
        );
        assert_eq!(
            listed(&named),
            [
                pair("Hiroyuki Sawano", "artist:156"),
                pair("KOHTA YAMAMOTO", "artist:120"),
            ]
        );
        let sawano = named.get("artist:156").unwrap();
        assert_eq!(sawano.aliases, ["サワノヒロユキ"]);
        assert_eq!(sawano.credits, ["r.ja"]);
        assert_eq!(songs_of(&named, "artist:156", &songs), ["i.kh"]);
    }
}
