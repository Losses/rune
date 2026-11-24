//! The plain types the library layer hands out.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use serde::de;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Prefix of the string form of a catalog key.
pub(super) const CATALOG_PREFIX: &str = "catalog:";
/// Prefix of Apple's library song ids.
const LIBRARY_SONG_PREFIX: &str = "i.";
/// Apple's crop code for a cover fitted into the requested box.
const COVER_CROP: &str = "bb";

/// Names a song for playing and rating: a song of the user's library, or a catalog song.
///
/// The string form is the library id (`i.…`) or `catalog:<id>`, and parsing it gives the
/// same key back.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackKey {
    library: bool,
    id: String,
}

impl TrackKey {
    /// The key of the library song with Apple's library id `id`, which starts with `i.`.
    pub fn library(id: impl Into<String>) -> Result<Self, ParseTrackKeyError> {
        let id = id.into();
        if is_library_song_id(&id) {
            Ok(Self { library: true, id })
        } else {
            Err(ParseTrackKeyError(id))
        }
    }

    /// The key of the catalog song with catalog id `id`, which is not empty and does not
    /// start with `catalog:`.
    pub fn catalog(id: impl Into<String>) -> Result<Self, ParseTrackKeyError> {
        let id = id.into();
        if id.is_empty() || id.starts_with(CATALOG_PREFIX) {
            Err(ParseTrackKeyError(id))
        } else {
            Ok(Self { library: false, id })
        }
    }

    /// Whether this names a song of the library.
    pub fn is_library(&self) -> bool {
        self.library
    }

    /// Apple's library id, for a library song.
    pub fn library_id(&self) -> Option<&str> {
        self.library.then_some(self.id.as_str())
    }

    /// The catalog id, for a catalog song.
    pub fn catalog_id(&self) -> Option<&str> {
        (!self.library).then_some(self.id.as_str())
    }
}

/// Whether an Apple id names something in the user's library: catalog ids name nothing there.
pub(super) fn is_library_id(id: &str) -> bool {
    !id.is_empty() && !id.starts_with(CATALOG_PREFIX)
}

/// Whether an Apple id names a song in the user's library.
fn is_library_song_id(id: &str) -> bool {
    id.len() > LIBRARY_SONG_PREFIX.len() && id.starts_with(LIBRARY_SONG_PREFIX)
}

impl fmt::Display for TrackKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.library {
            f.write_str(&self.id)
        } else {
            write!(f, "{CATALOG_PREFIX}{}", self.id)
        }
    }
}

/// Stored by its string form.
impl Serialize for TrackKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for TrackKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

/// The text is neither a library song id nor `catalog:<id>`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a track key: {0:?}")]
pub struct ParseTrackKeyError(String);

impl FromStr for TrackKey {
    type Err = ParseTrackKeyError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.strip_prefix(CATALOG_PREFIX) {
            Some(id) => Self::catalog(id).map_err(|_| ParseTrackKeyError(text.to_owned())),
            None => Self::library(text),
        }
    }
}

/// An image Apple serves in any size up to a limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artwork {
    /// URL template with `{w}`, `{h}`, `{c}` and `{f}` placeholders.
    pub template: String,
    /// Largest side offered, in pixels.
    pub max_size: Option<u32>,
}

impl Artwork {
    /// The image URL for a square of `size` pixels, never above the largest size offered.
    pub fn url(&self, size: u32) -> String {
        let side = size.min(self.max_size.unwrap_or(u32::MAX)).max(1);
        let side = side.to_string();
        self.template
            .replace("{w}", &side)
            .replace("{h}", &side)
            .replace("{c}", COVER_CROP)
            .replace("{f}", "jpg")
    }
}

/// The catalog album and artist Apple names for a catalog song, by id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogSongRelations {
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
}

/// A song of the library or of the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    /// How the song is played and rated.
    pub key: TrackKey,
    /// The song's name.
    pub title: String,
    /// The album's name.
    pub album: Option<String>,
    /// The artist's name.
    pub artist: Option<String>,
    /// The library album's id, or the catalog album's for a catalog song.
    pub album_id: Option<String>,
    /// The library artist's id, or the catalog artist's for a catalog song.
    pub artist_id: Option<String>,
    /// Number on its disc.
    pub track_number: Option<u32>,
    /// Number of its disc.
    pub disc_number: Option<u32>,
    /// Year of release.
    pub year: Option<u32>,
    /// First genre.
    pub genre: Option<String>,
    /// Length of the song.
    pub duration: Option<Duration>,
    /// The cover.
    pub artwork: Option<Artwork>,
    /// Whether the song is in the user's library.
    pub in_library: bool,
    /// The catalog id of the song, when Apple names one.
    pub catalog_id: Option<String>,
}

/// An album in the user's library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Album {
    /// Apple's library id.
    pub id: String,
    /// The album's name.
    pub name: String,
    /// The artist's name.
    pub artist: Option<String>,
    /// The library artist's id.
    pub artist_id: Option<String>,
    /// Number of songs.
    pub track_count: Option<u32>,
    /// Year of release.
    pub year: Option<u32>,
    /// First genre.
    pub genre: Option<String>,
    /// The cover.
    pub artwork: Option<Artwork>,
    /// The catalog id of the album, when Apple names one.
    pub catalog_id: Option<String>,
}

/// An album with its songs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumDetail {
    /// The album.
    pub album: Album,
    /// Its songs, in order.
    pub tracks: Vec<Track>,
}

/// An artist in the user's library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artist {
    /// Apple's library id.
    pub id: String,
    /// The artist's name.
    pub name: String,
    /// The picture of the first catalog artist, when Apple named one with the listing.
    pub artwork: Option<Artwork>,
    /// The catalog artists Apple named with the listing, in its order, each in the library
    /// as this artist's counterpart. Apple documents one; a name that credits several
    /// artists may come with more.
    pub catalog: Vec<ArtistItem>,
}

/// An artist with their albums in the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistDetail {
    /// The artist.
    pub artist: Artist,
    /// Their albums.
    pub albums: Vec<Album>,
}

/// A playlist in the user's library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    /// Apple's library id.
    pub id: String,
    /// The playlist's name.
    pub name: String,
    /// Whether songs can be added. Apple's API cannot rename, reorder or remove, and
    /// Favorites and subscribed playlists accept nothing.
    pub can_append: bool,
    /// Whether the playlist is public.
    pub is_public: Option<bool>,
    /// The cover.
    pub artwork: Option<Artwork>,
}

/// Names an album, artist or playlist of the user's library or of Apple's catalog.
///
/// Songs are named by [`TrackKey`], which also says how a song plays and rates and links a
/// catalog song to the library's copy of it. Albums, artists and playlists are only named,
/// so this just keeps Apple's id and which of the two it is.
///
/// The string form is the library id, or `catalog:<id>`, and parsing it gives the same
/// item back.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemId {
    catalog: bool,
    id: String,
}

impl ItemId {
    /// The item with Apple's library id `id`, which is not empty and does not start with
    /// `catalog:`.
    pub fn library(id: impl Into<String>) -> Result<Self, ParseItemIdError> {
        Self::checked(false, id.into())
    }

    /// The item with catalog id `id`, which is not empty and does not start with
    /// `catalog:`.
    pub fn catalog(id: impl Into<String>) -> Result<Self, ParseItemIdError> {
        Self::checked(true, id.into())
    }

    fn checked(catalog: bool, id: String) -> Result<Self, ParseItemIdError> {
        if is_library_id(&id) {
            Ok(Self { catalog, id })
        } else {
            Err(ParseItemIdError(id))
        }
    }

    /// An id as Apple sent it, which is taken as it is.
    pub(super) fn from_apple(catalog: bool, id: String) -> Self {
        Self { catalog, id }
    }

    pub(super) fn library_apple(id: String) -> Self {
        Self::from_apple(false, id)
    }

    pub(super) fn catalog_apple(id: String) -> Self {
        Self::from_apple(true, id)
    }

    /// The id as Apple writes it, without saying which of the two it is.
    pub fn as_str(&self) -> &str {
        &self.id
    }

    /// The catalog id, for a catalog item.
    pub fn catalog_id(&self) -> Option<&str> {
        self.catalog.then_some(self.id.as_str())
    }

    /// Apple's library id, for a library item.
    pub fn library_id(&self) -> Option<&str> {
        (!self.catalog).then_some(self.id.as_str())
    }

    /// Whether this names a catalog item.
    pub fn is_catalog(&self) -> bool {
        self.catalog
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.catalog {
            write!(f, "{CATALOG_PREFIX}{}", self.id)
        } else {
            f.write_str(&self.id)
        }
    }
}

/// Stored by its string form.
impl Serialize for ItemId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Read back as it was stored: an id Apple sent is taken as it is, as when it was first read.
impl<'de> Deserialize<'de> for ItemId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(match text.strip_prefix(CATALOG_PREFIX) {
            Some(id) => Self::catalog_apple(id.to_owned()),
            None => Self::library_apple(text),
        })
    }
}

/// The text is neither a library id nor `catalog:<id>`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not an item id: {0:?}")]
pub struct ParseItemIdError(String);

impl FromStr for ItemId {
    type Err = ParseItemIdError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.strip_prefix(CATALOG_PREFIX) {
            Some(id) => Self::catalog(id).map_err(|_| ParseItemIdError(text.to_owned())),
            None => Self::library(text),
        }
    }
}

/// An album in a search, a recommendation or a catalog page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumItem {
    /// The library album's id or the catalog album's.
    pub id: ItemId,
    /// The album's name.
    pub name: String,
    /// The artist's name.
    pub artist: Option<String>,
    /// The artist's id, of the same kind as `id`, when Apple names one.
    pub artist_id: Option<ItemId>,
    /// Number of songs.
    pub track_count: Option<u32>,
    /// Year of release.
    pub year: Option<u32>,
    /// First genre.
    pub genre: Option<String>,
    /// The cover.
    pub artwork: Option<Artwork>,
    /// Whether the user's library has the album.
    pub in_library: bool,
}

/// What the catalog says of a catalog album that tells where it goes on an artist's page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogAlbumFacts {
    /// The catalog id of the album.
    pub id: String,
    /// The album's name.
    pub name: String,
    /// Apple's `isSingle`, when it gives one.
    pub is_single: Option<bool>,
    /// Apple's `isCompilation`, when it gives one.
    pub is_compilation: Option<bool>,
    /// The catalog ids of the album's artists, in Apple's order.
    pub artists: Vec<String>,
}

/// An artist in a search or on a catalog page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistItem {
    /// The library artist's id or the catalog artist's.
    pub id: ItemId,
    /// The artist's name.
    pub name: String,
    /// The artist's picture.
    pub artwork: Option<Artwork>,
    /// Whether the user's library has the artist.
    pub in_library: bool,
}

/// A playlist in a search, a recommendation or a catalog page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistItem {
    /// The library playlist's id or the catalog playlist's.
    pub id: ItemId,
    /// The playlist's name.
    pub name: String,
    /// Who curated a catalog playlist.
    pub curator: Option<String>,
    /// Whether songs can be added: never for a catalog playlist.
    pub can_append: bool,
    /// The cover.
    pub artwork: Option<Artwork>,
    /// Whether the user's library has the playlist.
    pub in_library: bool,
}

/// Where a search looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    /// The user's library.
    Library,
    /// Apple's catalog.
    Catalog,
    /// The library first, then what the catalog adds to it.
    All,
}

/// What a search found, each list in order of relevance with the library's matches first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchResults {
    /// Matching songs.
    pub songs: Vec<Track>,
    /// Matching albums.
    pub albums: Vec<AlbumItem>,
    /// Matching artists.
    pub artists: Vec<ArtistItem>,
    /// Matching playlists. The catalog search does not ask for them, so these are the
    /// library's.
    pub playlists: Vec<PlaylistItem>,
}

/// A catalog album with its songs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogAlbumDetail {
    /// The album.
    pub album: AlbumItem,
    /// Its songs, in order.
    pub tracks: Vec<Track>,
}

/// A catalog playlist with its songs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPlaylistDetail {
    /// The playlist.
    pub playlist: PlaylistItem,
    /// Its songs, in order.
    pub tracks: Vec<Track>,
}

/// A catalog artist with their albums.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogArtistDetail {
    /// The artist.
    pub artist: ArtistItem,
    /// Their albums.
    pub albums: Vec<AlbumItem>,
}

/// One of the lists of releases Apple keeps for a catalog artist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReleaseView {
    /// `latest-release`: the artist's newest release.
    LatestRelease,
    /// `full-albums`.
    FullAlbums,
    /// `singles`, which also holds the artist's EPs (see [`ArtistRelease::ep`]).
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
    /// Every view, in the order an artist's page lists them.
    pub const ALL: [ReleaseView; 7] = [
        Self::LatestRelease,
        Self::FullAlbums,
        Self::Singles,
        Self::FeaturedAlbums,
        Self::LiveAlbums,
        Self::CompilationAlbums,
        Self::AppearsOnAlbums,
    ];

    /// The name Apple gives the view.
    pub fn name(self) -> &'static str {
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

/// A release in one of a catalog artist's views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistRelease {
    /// The album.
    pub album: AlbumItem,
    /// Whether it is an EP rather than a single. Only told for a release of the `singles`
    /// view, by the name Apple gives it (a trailing "- EP" or "- Single") or else by its
    /// `isSingle` attribute being false; false for every other view.
    pub ep: bool,
}

/// A page of a catalog artist's release view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleasePage {
    /// Apple's title of the view, such as "Singles & EPs", in the language of the
    /// storefront's default; `None` when it gives none, and on the pages after the first.
    pub title: Option<String>,
    /// The releases of this page, in Apple's order.
    pub releases: Vec<ArtistRelease>,
    /// Apple's subpath of the next page, to be handed back to
    /// [`AppleLibrary::catalog_releases_after`](super::AppleLibrary::catalog_releases_after)
    /// unchanged; `None` on the last page.
    pub next: Option<String>,
}

/// A page of a catalog artist's top songs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongPage {
    /// The songs of this page, in Apple's order, each marked as in the library or not.
    pub songs: Vec<Track>,
    /// Apple's subpath of the next page, to be handed back to
    /// [`AppleLibrary::catalog_top_songs_after`](super::AppleLibrary::catalog_top_songs_after)
    /// unchanged; `None` on the last page.
    pub next: Option<String>,
}

/// A catalog artist's page: the first page of each release view Apple has releases in, and
/// of the top songs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogArtistPage {
    /// The artist.
    pub artist: ArtistItem,
    /// The views that hold releases, in the order of [`ReleaseView::ALL`].
    pub views: Vec<(ReleaseView, ReleasePage)>,
    /// The artist's most popular songs in the storefront.
    pub top_songs: SongPage,
}

/// The languages a storefront's catalog texts come in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorefrontLanguages {
    /// The language tag Apple answers in when none is asked for or the one asked for is not
    /// supported.
    pub default: Option<String>,
    /// The language tags `l=` accepts, such as `en-GB` or `zh-Hans-CN`.
    pub supported: Vec<String>,
}

/// What a recommendation group is, as far as Apple's kind says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecommendationKind {
    /// Made for the user.
    Personal,
    /// What the user played lately.
    Recent,
    /// Music of a genre.
    Genre,
    /// Anything else.
    Other,
    /// What the user plays most. Apple does not document the kind, so it is kept apart from
    /// [`Recent`](Self::Recent) to be shown under whatever title Apple gives it.
    HeavyRotation,
}

/// A titled group of recommended albums and playlists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecommendationGroup {
    /// Apple's id of the group.
    pub id: String,
    /// The title to show.
    pub title: String,
    /// What the group is.
    pub kind: RecommendationKind,
    /// Its albums.
    pub albums: Vec<AlbumItem>,
    /// Its playlists.
    pub playlists: Vec<PlaylistItem>,
    /// Its albums and playlists in Apple's order, by their places in the two lists, so those
    /// lists may be changed in place but never reordered or filtered.
    pub order: Vec<RecommendedItem>,
}

impl RecommendationGroup {
    /// Its albums and playlists in Apple's order.
    pub fn items(&self) -> impl Iterator<Item = Recommended<'_>> {
        self.order.iter().filter_map(|item| match *item {
            RecommendedItem::Album(index) => self.albums.get(index).map(Recommended::Album),
            RecommendedItem::Playlist(index) => {
                self.playlists.get(index).map(Recommended::Playlist)
            }
        })
    }
}

/// Where an item of a [`RecommendationGroup`] is in the group's lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecommendedItem {
    /// The album at this place in [`RecommendationGroup::albums`].
    Album(usize),
    /// The playlist at this place in [`RecommendationGroup::playlists`].
    Playlist(usize),
}

/// An item of a [`RecommendationGroup`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recommended<'a> {
    /// A recommended album.
    Album(&'a AlbumItem),
    /// A recommended playlist.
    Playlist(&'a PlaylistItem),
}

/// The user's songs in order: library songs first, then the songs of the user's own
/// playlists and Favorites that are not in the library. No song is listed twice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SongList {
    songs: Vec<Track>,
    index: HashMap<TrackKey, usize>,
    /// Library song ids by catalog id, so a catalog copy of a library song is not listed twice.
    library_twins: HashMap<String, String>,
}

impl SongList {
    /// The songs, in order.
    pub fn songs(&self) -> &[Track] {
        &self.songs
    }

    /// Number of songs.
    pub fn len(&self) -> usize {
        self.songs.len()
    }

    /// Whether there are no songs.
    pub fn is_empty(&self) -> bool {
        self.songs.is_empty()
    }

    /// The listed song with `key`.
    pub fn get(&self, key: &TrackKey) -> Option<&Track> {
        self.index.get(key).map(|&at| &self.songs[at])
    }

    /// Whether `key`, or the library song a catalog key stands for, is listed.
    pub fn lists(&self, key: &TrackKey) -> bool {
        self.index.contains_key(key) || self.library_twin(key).is_some()
    }

    /// Whether the song is in the user's library: a listed library song, or a catalog
    /// song the library has a copy of.
    pub fn in_library(&self, key: &TrackKey) -> bool {
        (key.is_library() && self.index.contains_key(key)) || self.library_twin(key).is_some()
    }

    /// Whether the library has a copy of the catalog song `catalog_id`.
    pub(super) fn has_library_copy(&self, catalog_id: &str) -> bool {
        self.library_twins.contains_key(catalog_id)
    }

    /// The library song that is the library's copy of catalog song `key`.
    pub fn library_twin(&self, key: &TrackKey) -> Option<TrackKey> {
        let id = self.library_twins.get(key.catalog_id()?)?;
        TrackKey::library(id.clone()).ok()
    }

    /// The catalog song that the library song `key` is the library's copy of. Library songs
    /// that share a catalog id have one twin between them: only one of them finds it.
    pub fn catalog_twin(&self, key: &TrackKey) -> Option<TrackKey> {
        let id = key.library_id()?;
        let (catalog_id, _) = self
            .library_twins
            .iter()
            .find(|(_, library)| *library == id)?;
        TrackKey::catalog(catalog_id.clone()).ok()
    }

    /// A copy of this list in which each of `songs` that it lists under the same key takes the
    /// place of the one listed, and the others follow at its end in order, each once and unless
    /// the library has a copy of it.
    pub fn with_songs(&self, songs: impl IntoIterator<Item = Track>) -> Self {
        let mut list = self.clone();
        for song in songs {
            match self.index.get(&song.key) {
                Some(&at) => list.songs[at] = song,
                None => list.push(song),
            }
        }
        list
    }

    pub(crate) fn push(&mut self, song: Track) {
        if !self.lists(&song.key) {
            self.index.insert(song.key.clone(), self.songs.len());
            self.songs.push(song);
        }
    }

    /// Records that catalog song `catalog_id` is the library song `library`.
    pub(crate) fn add_library_twin(&mut self, catalog_id: &str, library: &TrackKey) {
        if let Some(id) = library.library_id() {
            self.library_twins
                .entry(catalog_id.to_owned())
                .or_insert_with(|| id.to_owned());
        }
    }
}

/// What a song list is stored as: its songs in order and the library copies of catalog
/// songs. The index is built again when it is read back.
#[derive(Serialize)]
struct StoredSongs<'a> {
    songs: &'a [Track],
    library_twins: &'a HashMap<String, String>,
}

#[derive(Deserialize)]
struct StoredSongList {
    songs: Vec<Track>,
    library_twins: HashMap<String, String>,
}

impl Serialize for SongList {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        StoredSongs {
            songs: &self.songs,
            library_twins: &self.library_twins,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SongList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let stored = StoredSongList::deserialize(deserializer)?;
        let mut list = Self {
            library_twins: stored.library_twins,
            ..Self::default()
        };
        for song in stored.songs {
            list.index
                .entry(song.key.clone())
                .or_insert(list.songs.len());
            list.songs.push(song);
        }
        Ok(list)
    }
}

impl Extend<Track> for SongList {
    fn extend<I: IntoIterator<Item = Track>>(&mut self, songs: I) {
        for song in songs {
            self.push(song);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_keys_round_trip_through_text() {
        for text in ["i.abc", "catalog:123", "catalog:i.abc", "catalog:a:b"] {
            let key: TrackKey = text.parse().unwrap();
            assert_eq!(key.to_string(), text);
        }
        let library = TrackKey::library("i.abc").unwrap();
        assert_eq!(library.library_id(), Some("i.abc"));
        assert_eq!(library.catalog_id(), None);
        let catalog = TrackKey::catalog("123").unwrap();
        assert_eq!(catalog.catalog_id(), Some("123"));
        assert!(!catalog.is_library());
        for key in [library, catalog, TrackKey::catalog("i.abc").unwrap()] {
            assert_eq!(key.to_string().parse(), Ok(key));
        }
        // Nothing that cannot be written back can be built.
        for id in ["", "i.", "a.123", "123", "catalog:1"] {
            assert!(TrackKey::library(id).is_err(), "{id}");
        }
        for id in ["", "catalog:1"] {
            assert!(TrackKey::catalog(id).is_err(), "{id}");
        }
        for text in [
            "",
            "i.",
            "catalog:",
            "catalog:catalog:1",
            "a.123",
            "123",
            "p.1",
            "l.1",
        ] {
            assert!(text.parse::<TrackKey>().is_err(), "{text}");
        }
    }

    fn listed_song(key: &str, title: &str, in_library: bool) -> Track {
        Track {
            key: key.parse().unwrap(),
            title: title.to_owned(),
            album: None,
            artist: None,
            album_id: None,
            artist_id: None,
            track_number: None,
            disc_number: None,
            year: None,
            genre: None,
            duration: None,
            artwork: None,
            in_library,
            catalog_id: None,
        }
    }

    #[test]
    fn songs_added_to_a_list_replace_their_own_key_and_follow_the_rest_once() {
        let mut list = SongList::default();
        list.extend([
            listed_song("i.a", "A", true),
            listed_song("catalog:2", "Playlist Only", false),
        ]);
        list.add_library_twin("1", &TrackKey::library("i.a").unwrap());
        let with = list.with_songs([
            listed_song("catalog:2", "Two", true),
            // The library's copy stands for it.
            listed_song("catalog:1", "Copy", true),
            listed_song("catalog:3", "Three", true),
            listed_song("catalog:3", "Again", true),
        ]);
        let titles: Vec<&str> = with
            .songs()
            .iter()
            .map(|song| song.title.as_str())
            .collect();
        assert_eq!(titles, ["A", "Two", "Three"]);
        assert!(with.in_library(&TrackKey::catalog("1").unwrap()));
        assert!(
            with.get(&TrackKey::catalog("2").unwrap())
                .unwrap()
                .in_library
        );
        // The list it was made from is as it was.
        assert_eq!(list.len(), 2);
        assert!(
            !list
                .get(&TrackKey::catalog("2").unwrap())
                .unwrap()
                .in_library
        );
    }

    #[test]
    fn artwork_fills_the_size_up_to_its_limit() {
        let artwork = Artwork {
            template: "https://a.invalid/{w}x{h}{c}.{f}".into(),
            max_size: Some(1000),
        };
        assert_eq!(artwork.url(300), "https://a.invalid/300x300bb.jpg");
        assert_eq!(artwork.url(3000), "https://a.invalid/1000x1000bb.jpg");
        assert_eq!(artwork.url(0), "https://a.invalid/1x1bb.jpg");
    }
}
