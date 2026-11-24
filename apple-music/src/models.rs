//! Typed models of Apple Music API resources.
//!
//! Every attribute is optional and unknown fields are ignored, so responses
//! keep decoding when Apple adds or omits fields.

use std::collections::HashMap;

use serde::de;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// A resource object: `id`, `type`, `href`, `attributes`, `relationships`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource<A, R = Value> {
    /// The resource identifier. Library ids (`i.`, `p.`, `l.`, `a.`) differ from catalog ids.
    pub id: String,
    /// The resource type, for example `songs` or `library-songs`.
    #[serde(rename = "type", default)]
    pub kind: String,
    /// Relative location of the resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    /// The resource attributes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<A>,
    /// Included relationships.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationships: Option<R>,
}

/// A relationship to other resources.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relationship<A> {
    /// Location to fetch the relationship directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    /// Subpath of the next page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    /// The related resources (attributes are present only when included).
    #[serde(default = "Vec::new")]
    pub data: Vec<Resource<A>>,
}

/// One page of a resource collection.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Page<T> {
    /// The resources of this page.
    #[serde(default = "Vec::new")]
    pub data: Vec<T>,
    /// Subpath of the next page; absent on the last page.
    #[serde(default)]
    pub next: Option<String>,
    /// Facts about the whole collection, when Apple sends them.
    #[serde(default)]
    pub meta: Option<PageMeta>,
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            next: None,
            meta: None,
        }
    }
}

/// The `meta` object of a collection page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct PageMeta {
    /// Number of resources in the whole collection.
    #[serde(default)]
    pub total: Option<u32>,
}

/// An image with a URL template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artwork {
    /// Maximum width available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Maximum height available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// URL template; `{w}` and `{h}` stand for the requested size.
    #[serde(default)]
    pub url: String,
    /// Average background color as hex without `#`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg_color: Option<String>,
    /// Primary text color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_color1: Option<String>,
    /// Secondary text color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_color2: Option<String>,
    /// Tertiary text color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_color3: Option<String>,
    /// Quaternary text color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_color4: Option<String>,
}

/// Parameters a player needs to start playback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayParams {
    /// The id to play.
    #[serde(default)]
    pub id: String,
    /// The kind of content, for example `song`, `album` or `playlist`.
    #[serde(default)]
    pub kind: String,
    /// Whether `id` is a library id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_library: Option<bool>,
    /// The catalog id behind a library item, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_id: Option<String>,
    /// The store id a purchased library item was bought under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purchased_id: Option<String>,
    /// Whether plays should be reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reporting: Option<bool>,
    /// The id used for play reporting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reporting_id: Option<String>,
    /// Any other keys, kept so the map can be handed to a player unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A 30 second preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preview {
    /// URL of the preview audio.
    #[serde(default)]
    pub url: String,
}

/// Localised description text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Description {
    /// The full text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standard: Option<String>,
    /// The short text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short: Option<String>,
}

/// Attributes of a catalog song.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SongAttributes {
    /// Album name.
    pub album_name: Option<String>,
    /// Artist name.
    pub artist_name: Option<String>,
    /// Album artwork.
    pub artwork: Option<Artwork>,
    /// Composer name.
    pub composer_name: Option<String>,
    /// `clean` or `explicit`; absent when unrated.
    pub content_rating: Option<String>,
    /// Disc number.
    pub disc_number: Option<u32>,
    /// Duration in milliseconds.
    pub duration_in_millis: Option<u64>,
    /// Genre names.
    #[serde(default)]
    pub genre_names: Vec<String>,
    /// Whether lyrics exist.
    pub has_lyrics: Option<bool>,
    /// International Standard Recording Code.
    pub isrc: Option<String>,
    /// Song name.
    pub name: Option<String>,
    /// Playback parameters; present when the song can be played.
    pub play_params: Option<PlayParams>,
    /// Previews.
    #[serde(default)]
    pub previews: Vec<Preview>,
    /// Release date, `YYYY-MM-DD` or `YYYY`.
    pub release_date: Option<String>,
    /// Track number.
    pub track_number: Option<u32>,
    /// Apple Music web URL.
    pub url: Option<String>,
}

/// A catalog song.
pub type Song = Resource<SongAttributes>;

/// Relationships of a catalog song, present when requested with `include`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SongRelationships {
    /// The song's catalog artists, in Apple's order.
    pub artists: Option<Relationship<ArtistAttributes>>,
}

/// A catalog song read with its artists.
pub type SongWithArtists = Resource<SongAttributes, SongRelationships>;

/// Attributes of a library song.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySongAttributes {
    /// Album name.
    pub album_name: Option<String>,
    /// Artist name.
    pub artist_name: Option<String>,
    /// Album artwork.
    pub artwork: Option<Artwork>,
    /// `clean` or `explicit`; absent when unrated.
    pub content_rating: Option<String>,
    /// Disc number.
    pub disc_number: Option<u32>,
    /// Duration in milliseconds.
    pub duration_in_millis: Option<u64>,
    /// Genre names.
    #[serde(default)]
    pub genre_names: Vec<String>,
    /// Whether lyrics exist in the catalog.
    pub has_lyrics: Option<bool>,
    /// Whether the catalog song is in the person's favorites.
    pub in_favorites: Option<bool>,
    /// Song name.
    pub name: Option<String>,
    /// Playback parameters; present when the song can be played.
    pub play_params: Option<PlayParams>,
    /// Release date.
    pub release_date: Option<String>,
    /// Track number.
    pub track_number: Option<u32>,
}

/// Relationships of a library song, present when requested with `include`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LibrarySongRelationships {
    /// The matching catalog song, when known.
    pub catalog: Option<Relationship<SongAttributes>>,
    /// The library albums the song is on.
    pub albums: Option<Relationship<LibraryAlbumAttributes>>,
    /// The library artists of the song.
    pub artists: Option<Relationship<LibraryArtistAttributes>>,
}

/// A song in the user's library. Library tracks of a playlist use the same type;
/// check [`Resource::kind`] for `library-music-videos`.
pub type LibrarySong = Resource<LibrarySongAttributes, LibrarySongRelationships>;

impl LibrarySong {
    /// The catalog id of this song: `playParams.catalogId`, else the id of the
    /// `catalog` relationship when it was included.
    pub fn catalog_id(&self) -> Option<&str> {
        self.attributes
            .as_ref()
            .and_then(|attributes| attributes.play_params.as_ref())
            .and_then(|params| params.catalog_id.as_deref())
            .or_else(|| {
                self.relationships
                    .as_ref()?
                    .catalog
                    .as_ref()?
                    .data
                    .first()
                    .map(|r| r.id.as_str())
            })
    }

    /// The store item behind this song: [`Self::catalog_id`], else the id it was
    /// purchased under, which names the same catalog song.
    pub fn store_id(&self) -> Option<&str> {
        self.catalog_id().or_else(|| {
            self.attributes
                .as_ref()?
                .play_params
                .as_ref()?
                .purchased_id
                .as_deref()
        })
    }

    /// Whether this is a music video, which has no song to play or rate.
    pub fn is_music_video(&self) -> bool {
        self.kind.contains("music-video")
    }

    /// The id of the first library album the song is on, when the relationship was included.
    pub fn album_id(&self) -> Option<&str> {
        first_id(self.relationships.as_ref()?.albums.as_ref())
    }

    /// The id of the first library artist of the song, when the relationship was included.
    pub fn artist_id(&self) -> Option<&str> {
        first_id(self.relationships.as_ref()?.artists.as_ref())
    }
}

impl Song {
    /// The id of the first catalog album the song is on, when the relationship was included.
    pub fn album_id(&self) -> Option<&str> {
        self.related_id("albums")
    }

    /// The id of the first catalog artist of the song, when the relationship was included.
    pub fn artist_id(&self) -> Option<&str> {
        self.related_id("artists")
    }

    fn related_id(&self, relationship: &str) -> Option<&str> {
        self.relationships
            .as_ref()?
            .pointer(&format!("/{relationship}/data/0/id"))?
            .as_str()
    }
}

fn first_id<A>(relationship: Option<&Relationship<A>>) -> Option<&str> {
    relationship?
        .data
        .first()
        .map(|resource| resource.id.as_str())
}

/// Attributes of a library album.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryAlbumAttributes {
    /// Artist name.
    pub artist_name: Option<String>,
    /// Artwork.
    pub artwork: Option<Artwork>,
    /// `clean` or `explicit`; absent when unrated.
    pub content_rating: Option<String>,
    /// When the album was added to the library.
    pub date_added: Option<String>,
    /// Genre names.
    #[serde(default)]
    pub genre_names: Vec<String>,
    /// Whether the catalog album is in the person's favorites.
    pub in_favorites: Option<bool>,
    /// Album name.
    pub name: Option<String>,
    /// Playback parameters.
    pub play_params: Option<PlayParams>,
    /// Release date.
    pub release_date: Option<String>,
    /// Number of tracks.
    pub track_count: Option<u32>,
}

/// An album in the user's library.
pub type LibraryAlbum = Resource<LibraryAlbumAttributes>;

impl LibraryAlbum {
    /// The catalog id of this album: `playParams.catalogId`, else the id of the `catalog`
    /// relationship when it was included.
    pub fn catalog_id(&self) -> Option<&str> {
        self.attributes
            .as_ref()
            .and_then(|attributes| attributes.play_params.as_ref())
            .and_then(|params| params.catalog_id.as_deref())
            .or_else(|| {
                self.relationships
                    .as_ref()?
                    .pointer("/catalog/data/0/id")?
                    .as_str()
            })
    }

    /// The id of the first library artist of the album, when the relationship was included.
    pub fn artist_id(&self) -> Option<&str> {
        self.relationships
            .as_ref()?
            .pointer("/artists/data/0/id")?
            .as_str()
    }
}

/// Attributes of a library artist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LibraryArtistAttributes {
    /// Artist name.
    pub name: Option<String>,
}

/// An artist in the user's library; its id starts with `r.`.
pub type LibraryArtist = Resource<LibraryArtistAttributes>;

impl LibraryArtist {
    /// The catalog artists behind this library artist, when the `catalog` relationship was
    /// included. Each is read on its own, so one Apple sent oddly does not cost the others.
    pub fn catalog_artists(&self) -> Vec<Artist> {
        self.relationships
            .as_ref()
            .and_then(|relationships| relationships.pointer("/catalog/data"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|value| serde_json::from_value(value.clone()).ok())
            .collect()
    }
}

/// Attributes of a library playlist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryPlaylistAttributes {
    /// Artwork.
    pub artwork: Option<Artwork>,
    /// Whether the playlist can be edited.
    pub can_edit: Option<bool>,
    /// When the playlist was added.
    pub date_added: Option<String>,
    /// Description.
    pub description: Option<Description>,
    /// Whether the playlist has a catalog counterpart.
    pub has_catalog: Option<bool>,
    /// Whether the playlist is public.
    pub is_public: Option<bool>,
    /// Playlist name.
    pub name: Option<String>,
    /// Playback parameters.
    pub play_params: Option<PlayParams>,
}

/// Relationships of a library playlist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LibraryPlaylistRelationships {
    /// The catalog playlist, when there is one.
    pub catalog: Option<Relationship<Value>>,
    /// The tracks; only when fetching a single playlist with `include=tracks`.
    pub tracks: Option<Relationship<LibrarySongAttributes>>,
}

/// A playlist in the user's library.
pub type LibraryPlaylist = Resource<LibraryPlaylistAttributes, LibraryPlaylistRelationships>;

impl LibraryPlaylist {
    /// Whether songs can be appended to the playlist. Apple's API can only append, and
    /// Favorites and subscribed playlists not even that.
    pub fn can_append(&self) -> bool {
        self.attributes
            .as_ref()
            .is_some_and(|attributes| attributes.can_edit == Some(true))
    }

    /// Whether the playlist's songs belong to the user's own music: playlists the user made
    /// (editable, or without a catalog counterpart, as Favorites) do; subscribed catalog
    /// playlists do not.
    pub fn holds_own_songs(&self) -> bool {
        let has_catalog = self
            .attributes
            .as_ref()
            .and_then(|attributes| attributes.has_catalog);
        self.can_append() || has_catalog != Some(true)
    }
}

/// Attributes of a storefront.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorefrontAttributes {
    /// Default language tag.
    pub default_language_tag: Option<String>,
    /// Storefront name.
    pub name: Option<String>,
    /// Supported language tags.
    #[serde(default)]
    pub supported_language_tags: Vec<String>,
    /// Explicit content policy.
    pub explicit_content_policy: Option<String>,
}

/// A storefront; its `id` is the lowercase country code, such as `us`.
pub type Storefront = Resource<StorefrontAttributes>;

/// Attributes of a rating.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RatingAttributes {
    /// `1` for a like, `-1` for a dislike; absent when unrated.
    pub value: Option<i8>,
}

/// A personal rating of a song.
pub type Rating = Resource<RatingAttributes>;

/// An item of a mixed listing such as recently added or folder children.
#[derive(Debug, Clone, PartialEq)]
pub enum LibraryEntry {
    /// A library album.
    Album(LibraryAlbum),
    /// A library playlist.
    Playlist(LibraryPlaylist),
    /// A library song.
    Song(LibrarySong),
    /// Any other resource type, kept as raw JSON.
    Other(Value),
}

impl<'de> Deserialize<'de> for LibraryEntry {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = Value::deserialize(d)?;
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        let entry = match kind {
            "library-albums" => {
                LibraryEntry::Album(serde_json::from_value(value).map_err(Error::custom)?)
            }
            "library-playlists" => {
                LibraryEntry::Playlist(serde_json::from_value(value).map_err(Error::custom)?)
            }
            "library-songs" => {
                LibraryEntry::Song(serde_json::from_value(value).map_err(Error::custom)?)
            }
            _ => LibraryEntry::Other(value),
        };
        Ok(entry)
    }
}

/// A resource type the library search can return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibrarySearchType {
    /// `library-songs`.
    Songs,
    /// `library-albums`.
    Albums,
    /// `library-artists`.
    Artists,
    /// `library-playlists`.
    Playlists,
}

impl LibrarySearchType {
    /// The `types` value of the request.
    pub fn as_str(self) -> &'static str {
        match self {
            LibrarySearchType::Songs => "library-songs",
            LibrarySearchType::Albums => "library-albums",
            LibrarySearchType::Artists => "library-artists",
            LibrarySearchType::Playlists => "library-playlists",
        }
    }
}

/// What a library search found, one page per requested type. Types that were not
/// requested or matched nothing are empty.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LibrarySearchResults {
    /// Matching library songs.
    pub songs: Page<LibrarySong>,
    /// Matching library albums.
    pub albums: Page<LibraryAlbum>,
    /// Matching library artists.
    pub artists: Page<LibraryArtist>,
    /// Matching library playlists.
    pub playlists: Page<LibraryPlaylist>,
}

/// The title of a recommendation group.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationTitle {
    /// The localised title to show.
    pub string_for_display: Option<String>,
}

/// Attributes of a personal recommendation, a titled group of items.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationAttributes {
    /// The group title.
    pub title: Option<RecommendationTitle>,
    /// What the group is, for example `music-recommendations` or `recently-played`.
    pub kind: Option<String>,
    /// The resource types of the items, for example `playlists`, `albums`, `stations`.
    #[serde(default)]
    pub resource_types: Vec<String>,
    /// Whether the group changes over time.
    pub is_group_recommendation: Option<bool>,
}

/// Attributes of an item inside a recommendation. Albums have an artist, playlists a curator.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationItemAttributes {
    /// Item name.
    pub name: Option<String>,
    /// Artist of an album.
    pub artist_name: Option<String>,
    /// Curator of a playlist.
    pub curator_name: Option<String>,
    /// Artwork.
    pub artwork: Option<Artwork>,
    /// Playback parameters.
    pub play_params: Option<PlayParams>,
}

/// Relationships of a recommendation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecommendationRelationships {
    /// The items of the group. Check [`Resource::kind`]: `playlists`, `albums` or `stations`.
    pub contents: Option<Relationship<RecommendationItemAttributes>>,
}

/// A personal recommendation: a titled group of playlists, albums and stations.
pub type Recommendation = Resource<RecommendationAttributes, RecommendationRelationships>;

/// Attributes of a catalog album.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumAttributes {
    /// Artist name.
    pub artist_name: Option<String>,
    /// Artwork.
    pub artwork: Option<Artwork>,
    /// `clean` or `explicit`; absent when unrated.
    pub content_rating: Option<String>,
    /// Genre names.
    #[serde(default)]
    pub genre_names: Vec<String>,
    /// Whether the album is a compilation.
    pub is_compilation: Option<bool>,
    /// Whether the album is a single.
    pub is_single: Option<bool>,
    /// Album name.
    pub name: Option<String>,
    /// Playback parameters; present when the album can be played.
    pub play_params: Option<PlayParams>,
    /// Record label.
    pub record_label: Option<String>,
    /// Release date, `YYYY-MM-DD` or `YYYY`.
    pub release_date: Option<String>,
    /// Number of tracks.
    pub track_count: Option<u32>,
    /// Apple Music web URL.
    pub url: Option<String>,
}

/// Relationships of a catalog album, present when requested with `include`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlbumRelationships {
    /// The album's artists.
    pub artists: Option<Relationship<ArtistAttributes>>,
}

/// A catalog album.
pub type Album = Resource<AlbumAttributes, AlbumRelationships>;

impl Album {
    /// The id of the first artist of the album, when the relationship was included.
    pub fn artist_id(&self) -> Option<&str> {
        first_id(self.relationships.as_ref()?.artists.as_ref())
    }

    /// Whether the album is an EP rather than a single. Apple names an EP "<title> - EP" and
    /// a single "<title> - Single", so a trailing "EP" or "Single" after a dash decides; a
    /// name that says neither is told by `isSingle`, which is false for an EP. Only meant
    /// for releases of an artist's `singles` view.
    pub fn is_ep(&self) -> bool {
        let attributes = self.attributes.as_ref();
        let name = attributes
            .and_then(|attributes| attributes.name.as_deref())
            .unwrap_or_default()
            .trim();
        let tail = name
            .rsplit_once(['-', '\u{2013}', '\u{2014}'])
            .map(|(_, tail)| tail.trim());
        match tail {
            Some(tail) if tail.eq_ignore_ascii_case("ep") => true,
            Some(tail) if tail.eq_ignore_ascii_case("single") => false,
            _ => attributes.and_then(|attributes| attributes.is_single) == Some(false),
        }
    }
}

/// Relationships of a catalog album read with its artists and its songs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlbumWithTracksRelationships {
    /// The album's artists.
    pub artists: Option<Relationship<ArtistAttributes>>,
    /// The album's songs and music videos, in order: as many as Apple gives with the album,
    /// with the subpath of the rest in `next`.
    pub tracks: Option<Relationship<SongAttributes>>,
}

/// A catalog album read with its artists and its songs.
pub type AlbumWithTracks = Resource<AlbumAttributes, AlbumWithTracksRelationships>;

/// A catalog artist with the first page of each view asked for by `?views=`: lists of related
/// resources, such as `full-albums` or `top-songs`, each with the subpath of its next page.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ArtistWithViews {
    /// The artist's catalog id.
    pub id: String,
    /// The artist's attributes.
    #[serde(default)]
    pub attributes: Option<ArtistAttributes>,
    /// The views Apple answered, by name. Their items are kept as they came, so one that
    /// does not decode costs only itself (see [`Self::albums`] and [`Self::songs`]).
    #[serde(default)]
    pub views: HashMap<String, View>,
}

/// One view of a resource: a list of related resources.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct View {
    /// Subpath of the next page; absent on the last page.
    #[serde(default)]
    pub next: Option<String>,
    /// The items of this page.
    #[serde(default)]
    pub data: Vec<Value>,
    /// What Apple says of the view itself, on the first page of it only.
    #[serde(default)]
    pub attributes: Option<ViewAttributes>,
}

/// What Apple says of a view of a resource.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ViewAttributes {
    /// The view's title, such as "Singles & EPs", in the language of the request's storefront.
    #[serde(default)]
    pub title: Option<String>,
}

impl View {
    /// The items that are albums, those that do not decode left out.
    pub fn albums(&self) -> Vec<Album> {
        decoded(&self.data, "albums")
    }

    /// The items that are songs, music videos and the items that do not decode left out.
    pub fn songs(&self) -> Vec<Song> {
        decoded(&self.data, "songs")
    }
}

impl ArtistWithViews {
    /// The view `name`, or an empty one when Apple answered none.
    pub fn view(&self, name: &str) -> View {
        self.views.get(name).cloned().unwrap_or_default()
    }
}

/// The items of `kind` among `data`, in order.
fn decoded<T: de::DeserializeOwned>(data: &[Value], kind: &str) -> Vec<T> {
    data.iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some(kind))
        .filter_map(|item| serde_json::from_value(item.clone()).ok())
        .collect()
}

/// Attributes of a catalog artist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistAttributes {
    /// Artwork, when the artist has an image.
    pub artwork: Option<Artwork>,
    /// Genre names.
    #[serde(default)]
    pub genre_names: Vec<String>,
    /// Artist name.
    pub name: Option<String>,
    /// Apple Music web URL.
    pub url: Option<String>,
}

/// A catalog artist.
pub type Artist = Resource<ArtistAttributes>;

/// Attributes of a catalog playlist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistAttributes {
    /// Artwork.
    pub artwork: Option<Artwork>,
    /// Who curated the playlist.
    pub curator_name: Option<String>,
    /// Description.
    pub description: Option<Description>,
    /// When the playlist last changed.
    pub last_modified_date: Option<String>,
    /// Playlist name.
    pub name: Option<String>,
    /// Playback parameters.
    pub play_params: Option<PlayParams>,
    /// `editorial`, `external`, `personal-mix`, `replay` or `user-shared`.
    pub playlist_type: Option<String>,
    /// Apple Music web URL.
    pub url: Option<String>,
}

/// A catalog playlist.
pub type Playlist = Resource<PlaylistAttributes>;

/// A resource type the catalog search can return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogSearchType {
    /// `songs`.
    Songs,
    /// `albums`.
    Albums,
    /// `artists`.
    Artists,
}

impl CatalogSearchType {
    /// The `types` value of the request.
    pub fn as_str(self) -> &'static str {
        match self {
            CatalogSearchType::Songs => "songs",
            CatalogSearchType::Albums => "albums",
            CatalogSearchType::Artists => "artists",
        }
    }
}

/// What a catalog search found, one page per requested type. Types that were not
/// requested or matched nothing are empty.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CatalogSearchResults {
    /// Matching songs.
    pub songs: Page<Song>,
    /// Matching albums.
    pub albums: Page<Album>,
    /// Matching artists.
    pub artists: Page<Artist>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn read<T: de::DeserializeOwned>(value: Value) -> T {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn library_song_reads_its_related_ids() {
        let song: LibrarySong = read(json!({
            "id": "i.1", "type": "library-songs",
            "relationships": {
                "albums": {"data": [{"id": "l.1", "type": "library-albums"}]},
                "artists": {"data": [{"id": "r.1", "type": "library-artists"}]}
            }
        }));
        assert_eq!(song.album_id(), Some("l.1"));
        assert_eq!(song.artist_id(), Some("r.1"));
        assert!(!song.is_music_video());

        let bare: LibrarySong = read(json!({"id": "i.2", "type": "library-music-videos"}));
        assert_eq!(bare.album_id(), None);
        assert_eq!(bare.artist_id(), None);
        assert!(bare.is_music_video());
    }

    #[test]
    fn catalog_song_reads_its_related_ids() {
        let song: Song = read(json!({
            "id": "100", "type": "songs",
            "relationships": {
                "albums": {"data": [{"id": "200", "type": "albums"}]},
                "artists": {"data": []}
            }
        }));
        assert_eq!(song.album_id(), Some("200"));
        assert_eq!(song.artist_id(), None);
        let bare: Song = read(json!({"id": "101", "type": "songs"}));
        assert_eq!(bare.album_id(), None);
    }

    #[test]
    fn albums_read_their_artist_ids() {
        let library: LibraryAlbum = read(json!({
            "id": "l.1", "type": "library-albums",
            "relationships": {"artists": {"data": [{"id": "r.9", "type": "library-artists"}]}}
        }));
        assert_eq!(library.artist_id(), Some("r.9"));
        let catalog: Album = read(json!({
            "id": "200", "type": "albums",
            "relationships": {"artists": {"data": [{"id": "300", "type": "artists"}]}}
        }));
        assert_eq!(catalog.artist_id(), Some("300"));
        let bare: Album = read(json!({"id": "201", "type": "albums"}));
        assert_eq!(bare.artist_id(), None);
    }

    #[test]
    fn library_artist_lists_its_catalog_artists_and_skips_malformed_ones() {
        let artist: LibraryArtist = read(json!({
            "id": "r.1", "type": "library-artists",
            "relationships": {"catalog": {"data": [
                {"id": "300", "type": "artists", "attributes": {"name": "Nine"}},
                {"type": "artists"}
            ]}}
        }));
        let catalog = artist.catalog_artists();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].id, "300");
        let bare: LibraryArtist = read(json!({"id": "r.2", "type": "library-artists"}));
        assert!(bare.catalog_artists().is_empty());
    }

    fn playlist(can_edit: Option<bool>, has_catalog: Option<bool>) -> LibraryPlaylist {
        read(json!({
            "id": "p.1", "type": "library-playlists",
            "attributes": {"canEdit": can_edit, "hasCatalog": has_catalog}
        }))
    }

    #[test]
    fn playlist_append_and_ownership() {
        let mine = playlist(Some(true), Some(true));
        assert!(mine.can_append());
        assert!(mine.holds_own_songs());
        let favorites = playlist(Some(false), Some(false));
        assert!(!favorites.can_append());
        assert!(favorites.holds_own_songs());
        let subscribed = playlist(Some(false), Some(true));
        assert!(!subscribed.can_append());
        assert!(!subscribed.holds_own_songs());
        let unknown: LibraryPlaylist = read(json!({"id": "p.2", "type": "library-playlists"}));
        assert!(!unknown.can_append());
        assert!(unknown.holds_own_songs());
    }

    fn album(name: &str, is_single: Option<bool>) -> Album {
        read(json!({
            "id": "1", "type": "albums",
            "attributes": {"name": name, "isSingle": is_single}
        }))
    }

    #[test]
    fn ep_is_told_from_single_by_name_then_by_flag() {
        assert!(album("Dawn - EP", None).is_ep());
        assert!(album("Dawn \u{2013} ep", Some(true)).is_ep());
        assert!(!album("Dawn - Single", Some(false)).is_ep());
        assert!(album("Dawn", Some(false)).is_ep());
        assert!(!album("Dawn", Some(true)).is_ep());
        assert!(!album("Dawn", None).is_ep());
    }
}
