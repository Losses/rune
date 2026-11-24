//! How Apple Music resources become the library's types.

use std::time::Duration;

use super::types::{
    Album, AlbumItem, Artist, ArtistItem, ArtistRelease, Artwork, ItemId, Playlist, PlaylistItem,
    RecommendationGroup, RecommendationKind, RecommendedItem, Track, TrackKey,
};
use crate::models::{
    self, LibraryAlbum, LibraryArtist, LibraryPlaylist, LibrarySong, LibrarySongAttributes,
    Recommendation, Relationship, Song, SongAttributes,
};

fn artwork(artwork: Option<&models::Artwork>) -> Option<Artwork> {
    let artwork = artwork.filter(|artwork| !artwork.url.is_empty())?;
    Some(Artwork {
        template: artwork.url.clone(),
        max_size: artwork.width,
    })
}

/// The attributes library and catalog songs share.
struct Attributes<'a> {
    name: Option<&'a str>,
    album: Option<&'a str>,
    artist: Option<&'a str>,
    track: Option<u32>,
    disc: Option<u32>,
    release_date: Option<&'a str>,
    genres: &'a [String],
    duration_ms: Option<u64>,
    artwork: Option<&'a models::Artwork>,
}

impl<'a> From<&'a LibrarySongAttributes> for Attributes<'a> {
    fn from(attributes: &'a LibrarySongAttributes) -> Self {
        Self {
            name: attributes.name.as_deref(),
            album: attributes.album_name.as_deref(),
            artist: attributes.artist_name.as_deref(),
            track: attributes.track_number,
            disc: attributes.disc_number,
            release_date: attributes.release_date.as_deref(),
            genres: &attributes.genre_names,
            duration_ms: attributes.duration_in_millis,
            artwork: attributes.artwork.as_ref(),
        }
    }
}

impl<'a> From<&'a SongAttributes> for Attributes<'a> {
    fn from(attributes: &'a SongAttributes) -> Self {
        Self {
            name: attributes.name.as_deref(),
            album: attributes.album_name.as_deref(),
            artist: attributes.artist_name.as_deref(),
            track: attributes.track_number,
            disc: attributes.disc_number,
            release_date: attributes.release_date.as_deref(),
            genres: &attributes.genre_names,
            duration_ms: attributes.duration_in_millis,
            artwork: attributes.artwork.as_ref(),
        }
    }
}

fn track(
    key: TrackKey,
    attributes: Attributes<'_>,
    album_id: Option<String>,
    artist_id: Option<String>,
    in_library: bool,
    catalog_id: Option<String>,
) -> Track {
    Track {
        key,
        title: attributes.name.unwrap_or_default().to_owned(),
        album: attributes.album.map(str::to_owned),
        artist: attributes.artist.map(str::to_owned),
        album_id,
        artist_id,
        track_number: attributes.track,
        disc_number: attributes.disc,
        year: year(attributes.release_date),
        genre: attributes.genres.first().cloned(),
        duration: attributes.duration_ms.map(Duration::from_millis),
        artwork: artwork(attributes.artwork),
        in_library,
        catalog_id,
    }
}

/// A library song or playlist track; `None` for a music video, and for an entry that is
/// neither a library song nor known in the catalog, which could not be played or rated.
/// Entries that are not library songs (playlist tracks outside the library) become
/// catalog songs when their catalog id is known, so they play and rate as such.
pub(super) fn library_track(entry: &LibrarySong) -> Option<Track> {
    if entry.kind.contains("music-video") {
        return None;
    }
    let store_id = entry.store_id();
    let (key, in_library) = match TrackKey::library(&entry.id) {
        Ok(key) => (key, true),
        Err(_) => (TrackKey::catalog(store_id?).ok()?, false),
    };
    let relationships = entry.relationships.as_ref();
    let album_id = first_id(relationships.and_then(|r| r.albums.as_ref()));
    let artist_id = first_id(relationships.and_then(|r| r.artists.as_ref()));
    let attributes = entry.attributes.clone().unwrap_or_default();
    Some(track(
        key,
        Attributes::from(&attributes),
        album_id,
        artist_id,
        in_library,
        store_id.map(str::to_owned),
    ))
}

fn first_id<A>(relationship: Option<&Relationship<A>>) -> Option<String> {
    relationship
        .and_then(|relationship| relationship.data.first())
        .map(|resource| resource.id.clone())
}

/// A catalog song, outside the library until marked otherwise. Its catalog album and
/// artist are known only when Apple sends the song's relationships.
pub(super) fn catalog_track(entry: &Song) -> Option<Track> {
    let attributes = entry.attributes.clone().unwrap_or_default();
    let related = |relationship: &str| {
        let id = entry
            .relationships
            .as_ref()?
            .pointer(&format!("/{relationship}/data/0/id"))?
            .as_str()?;
        Some(id.to_owned())
    };
    Some(track(
        TrackKey::catalog(&entry.id).ok()?,
        Attributes::from(&attributes),
        related("albums"),
        related("artists"),
        false,
        Some(entry.id.clone()),
    ))
}

pub(super) fn album(entry: &LibraryAlbum) -> Album {
    let attributes = entry.attributes.clone().unwrap_or_default();
    Album {
        id: entry.id.clone(),
        name: attributes.name.unwrap_or_default(),
        artist: attributes.artist_name,
        artist_id: entry
            .relationships
            .as_ref()
            .and_then(|relationships| relationships.pointer("/artists/data/0/id"))
            .and_then(|id| id.as_str())
            .map(str::to_owned),
        track_count: attributes.track_count,
        year: year(attributes.release_date.as_deref()),
        genre: attributes.genre_names.first().cloned(),
        artwork: artwork(attributes.artwork.as_ref()),
        catalog_id: entry.catalog_id().map(str::to_owned),
    }
}

pub(super) fn artist(entry: &LibraryArtist) -> Artist {
    // A library artist has no picture of its own; its catalog artists do. Each is read on
    // its own, so one Apple sent oddly does not cost the others.
    let catalog: Vec<ArtistItem> = entry
        .relationships
        .as_ref()
        .and_then(|relationships| relationships.pointer("/catalog/data"))
        .and_then(|data| data.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| serde_json::from_value::<models::Artist>(value.clone()).ok())
        .map(|catalog| ArtistItem {
            in_library: true,
            ..catalog_artist_item(&catalog)
        })
        .collect();
    Artist {
        id: entry.id.clone(),
        name: entry
            .attributes
            .as_ref()
            .and_then(|attributes| attributes.name.clone())
            .unwrap_or_default(),
        artwork: catalog.first().and_then(|first| first.artwork.clone()),
        catalog,
    }
}

pub(super) fn playlist(entry: &LibraryPlaylist) -> Playlist {
    let attributes = entry.attributes.clone().unwrap_or_default();
    Playlist {
        id: entry.id.clone(),
        name: attributes.name.clone().unwrap_or_default(),
        // Apple's API can only append to a playlist; Favorites and subscribed ones not even that.
        can_append: can_append(entry),
        is_public: attributes.is_public,
        artwork: artwork(attributes.artwork.as_ref()),
    }
}

pub(super) fn can_append(entry: &LibraryPlaylist) -> bool {
    entry
        .attributes
        .as_ref()
        .is_some_and(|attributes| attributes.can_edit == Some(true))
}

/// Whether a playlist's songs belong to the user's own music: playlists the user made
/// (editable, or without a catalog counterpart, as Favorites) do; subscribed catalog
/// playlists do not.
pub(super) fn joins_song_list(entry: &LibraryPlaylist) -> bool {
    let attributes = entry.attributes.as_ref();
    can_append(entry) || attributes.and_then(|attributes| attributes.has_catalog) != Some(true)
}

fn year(release_date: Option<&str>) -> Option<u32> {
    release_date?.get(..4)?.parse().ok()
}

// Items ----------------------------------------------------------------

/// A library album as an item, in the library by definition.
pub(super) fn library_album_item(entry: &LibraryAlbum) -> AlbumItem {
    let album = album(entry);
    AlbumItem {
        id: ItemId::library_apple(album.id),
        name: album.name,
        artist: album.artist,
        artist_id: album.artist_id.map(ItemId::library_apple),
        track_count: album.track_count,
        year: album.year,
        genre: album.genre,
        artwork: album.artwork,
        in_library: true,
    }
}

/// A library artist as an item.
pub(super) fn library_artist_item(entry: &LibraryArtist) -> ArtistItem {
    let artist = artist(entry);
    ArtistItem {
        id: ItemId::library_apple(artist.id),
        name: artist.name,
        artwork: None,
        in_library: true,
    }
}

/// A library playlist as an item.
pub(super) fn library_playlist_item(entry: &LibraryPlaylist) -> PlaylistItem {
    let playlist = playlist(entry);
    PlaylistItem {
        id: ItemId::library_apple(playlist.id),
        name: playlist.name,
        curator: None,
        can_append: playlist.can_append,
        artwork: playlist.artwork,
        in_library: true,
    }
}

/// A catalog album, outside the library until marked otherwise.
pub(super) fn catalog_album_item(entry: &models::Album) -> AlbumItem {
    let attributes = entry.attributes.clone().unwrap_or_default();
    AlbumItem {
        id: ItemId::catalog_apple(entry.id.clone()),
        name: attributes.name.unwrap_or_default(),
        artist: attributes.artist_name,
        artist_id: entry
            .relationships
            .as_ref()
            .and_then(|relationships| first_id(relationships.artists.as_ref()))
            .map(ItemId::catalog_apple),
        track_count: attributes.track_count,
        year: year(attributes.release_date.as_deref()),
        genre: attributes.genre_names.first().cloned(),
        artwork: artwork(attributes.artwork.as_ref()),
        in_library: false,
    }
}

/// Whether a release of an artist's `singles` view is an EP rather than a single. Apple names
/// an EP "<title> - EP" and a single "<title> - Single", so a trailing "EP" after a dash
/// decides, and so does "Single"; a name that says neither is told by the album's `isSingle`
/// attribute, which is false for an EP.
fn is_ep(name: &str, is_single: Option<bool>) -> bool {
    let name = name.trim();
    let tail = name
        .rsplit_once(['-', '\u{2013}', '\u{2014}'])
        .map(|(_, tail)| tail.trim());
    match tail {
        Some(tail) if tail.eq_ignore_ascii_case("ep") => true,
        Some(tail) if tail.eq_ignore_ascii_case("single") => false,
        _ => is_single == Some(false),
    }
}

/// A release of a catalog artist's view, outside the library until marked otherwise. `singles`
/// is whether it comes from the `singles` view, the only one that tells EPs from singles.
pub(super) fn artist_release(entry: &models::Album, singles: bool) -> ArtistRelease {
    let attributes = entry.attributes.as_ref();
    ArtistRelease {
        album: catalog_album_item(entry),
        ep: singles
            && is_ep(
                attributes
                    .and_then(|attributes| attributes.name.as_deref())
                    .unwrap_or_default(),
                attributes.and_then(|attributes| attributes.is_single),
            ),
    }
}

/// A catalog artist, outside the library until marked otherwise.
pub(super) fn catalog_artist_item(entry: &models::Artist) -> ArtistItem {
    let attributes = entry.attributes.as_ref();
    ArtistItem {
        id: ItemId::catalog_apple(entry.id.clone()),
        name: attributes
            .and_then(|attributes| attributes.name.clone())
            .unwrap_or_default(),
        artwork: artwork(attributes.and_then(|attributes| attributes.artwork.as_ref())),
        in_library: false,
    }
}

/// A catalog playlist: nothing of it can be changed through Apple's API.
pub(super) fn catalog_playlist_item(entry: &models::Playlist) -> PlaylistItem {
    let attributes = entry.attributes.clone().unwrap_or_default();
    PlaylistItem {
        id: ItemId::catalog_apple(entry.id.clone()),
        name: attributes.name.unwrap_or_default(),
        curator: attributes.curator_name,
        can_append: false,
        artwork: artwork(attributes.artwork.as_ref()),
        in_library: false,
    }
}

/// A recommendation as a group of albums and playlists in Apple's order, or `None` when it
/// holds neither. Stations and other kinds of item are left out, and so are albums and playlists
/// Apple gives without a name, as it can give a playlist a user shared: there is nothing to list
/// them by. Whether they are in the library is marked by the caller.
pub(super) fn recommendation_group(entry: &Recommendation) -> Option<RecommendationGroup> {
    let attributes = entry.attributes.clone().unwrap_or_default();
    let mut group = RecommendationGroup {
        id: entry.id.clone(),
        title: attributes
            .title
            .and_then(|title| title.string_for_display)
            .unwrap_or_default(),
        kind: recommendation_kind(attributes.kind.as_deref()),
        albums: Vec::new(),
        playlists: Vec::new(),
        order: Vec::new(),
    };
    let items = entry
        .relationships
        .as_ref()
        .and_then(|relationships| relationships.contents.as_ref())
        .map(|contents| contents.data.as_slice())
        .unwrap_or_default();
    for item in items {
        let attributes = item.attributes.clone().unwrap_or_default();
        let (library, catalog) = match item.kind.as_str() {
            "albums" | "playlists" => (false, true),
            // Not documented as recommendation contents; taken as library items if they come.
            "library-albums" | "library-playlists" => (true, false),
            _ => (false, false),
        };
        if !library && !catalog {
            continue;
        }
        let id = if library {
            ItemId::library_apple(item.id.clone())
        } else {
            ItemId::catalog_apple(item.id.clone())
        };
        let name = attributes.name.unwrap_or_default();
        if name.trim().is_empty() {
            continue;
        }
        let cover = artwork(attributes.artwork.as_ref());
        if item.kind.ends_with("albums") {
            group.order.push(RecommendedItem::Album(group.albums.len()));
            group.albums.push(AlbumItem {
                id,
                name,
                artist: attributes.artist_name,
                artist_id: None,
                track_count: None,
                year: None,
                genre: None,
                artwork: cover,
                in_library: library,
            });
        } else {
            group
                .order
                .push(RecommendedItem::Playlist(group.playlists.len()));
            group.playlists.push(PlaylistItem {
                id,
                name,
                curator: attributes.curator_name,
                // How far a library playlist can be edited is known only once it is read.
                can_append: false,
                artwork: cover,
                in_library: library,
            });
        }
    }
    (!group.albums.is_empty() || !group.playlists.is_empty()).then_some(group)
}

/// Apple's recommendation kinds onto ours. Apple documents `music-recommendations` and
/// `recently-played`; the others are guesses at undocumented values.
fn recommendation_kind(kind: Option<&str>) -> RecommendationKind {
    match kind.unwrap_or_default() {
        "music-recommendations" => RecommendationKind::Personal,
        "recently-played" => RecommendationKind::Recent,
        // Undocumented.
        "heavy-rotation" => RecommendationKind::HeavyRotation,
        // Undocumented.
        kind if kind.contains("genre") => RecommendationKind::Genre,
        _ => RecommendationKind::Other,
    }
}

/// A name as it is compared across library and catalog: case and spacing ignored.
pub(super) fn name_key(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::types::Recommended;

    #[test]
    fn names_compare_without_case_or_spacing() {
        assert_eq!(name_key("  Dawn   Album "), name_key("dawn album"));
        assert_ne!(name_key("Dawn Album"), name_key("Dusk Album"));
    }

    #[test]
    fn albums_and_playlists_without_a_name_are_left_out_and_so_is_a_group_left_empty() {
        let entry = |items: serde_json::Value| -> Recommendation {
            serde_json::from_value(serde_json::json!({
                "id": "rec-1",
                "type": "personal-recommendation",
                "attributes": {"title": {"stringForDisplay": "Recently Played"}},
                "relationships": {"contents": {"data": items}},
            }))
            .unwrap()
        };
        let unnamed = serde_json::json!([
            {"id": "pl.u-gone", "type": "playlists", "attributes": {"name": ""}},
            {"id": "a.blank", "type": "albums", "attributes": {"name": "  "}},
            {"id": "pl.bare", "type": "playlists"},
        ]);
        assert!(recommendation_group(&entry(unnamed.clone())).is_none());
        let mut mixed = unnamed.as_array().unwrap().clone();
        mixed.push(
            serde_json::json!({"id": "a1", "type": "albums", "attributes": {"name": "Dawn"}}),
        );
        let group = recommendation_group(&entry(serde_json::Value::Array(mixed))).unwrap();
        assert!(group.playlists.is_empty());
        assert_eq!(group.albums.len(), 1);
        assert_eq!(group.order, [RecommendedItem::Album(0)]);
    }

    #[test]
    fn a_recommendation_keeps_apples_order_of_albums_and_playlists() {
        let item = |id: &str, kind: &str| serde_json::json!({"id": id, "type": kind, "attributes": {"name": id}});
        let entry: Recommendation = serde_json::from_value(serde_json::json!({
            "id": "rec-1",
            "type": "personal-recommendation",
            "attributes": {"title": {"stringForDisplay": "Recently Played"}},
            "relationships": {"contents": {"data": [
                item("pl.1", "playlists"),
                item("a1", "albums"),
                item("ra.1", "stations"),
                item("pl.2", "playlists"),
                item("a2", "albums"),
            ]}},
        }))
        .unwrap();
        let group = recommendation_group(&entry).unwrap();
        let names: Vec<&str> = group
            .items()
            .map(|item| match item {
                Recommended::Album(album) => album.name.as_str(),
                Recommended::Playlist(playlist) => playlist.name.as_str(),
            })
            .collect();
        assert_eq!(names, ["pl.1", "a1", "pl.2", "a2"]);
        assert_eq!(
            group.order,
            [
                RecommendedItem::Playlist(0),
                RecommendedItem::Album(0),
                RecommendedItem::Playlist(1),
                RecommendedItem::Album(1),
            ]
        );
        assert_eq!((group.albums.len(), group.playlists.len()), (2, 2));
    }

    #[test]
    fn recommendation_kinds_map_as_far_as_known() {
        for (kind, expected) in [
            (Some("music-recommendations"), RecommendationKind::Personal),
            (Some("recently-played"), RecommendationKind::Recent),
            (Some("heavy-rotation"), RecommendationKind::HeavyRotation),
            (Some("by-genre"), RecommendationKind::Genre),
            (Some("other"), RecommendationKind::Other),
            (None, RecommendationKind::Other),
        ] {
            assert_eq!(recommendation_kind(kind), expected, "{kind:?}");
        }
    }

    #[test]
    fn an_ep_is_told_by_its_name_and_else_by_is_single() {
        assert!(is_ep("Midnights - EP", Some(true)));
        assert!(is_ep("Midnights \u{2013} EP", None));
        assert!(is_ep("Under the Sun - ep", Some(false)));
        assert!(!is_ep("Midnights - Single", Some(false)));
        assert!(!is_ep("Hello - Single", Some(true)));
        // Dashes inside a title do not make an EP, and a name that says neither follows
        // the attribute.
        assert!(!is_ep("Hello-Goodbye", Some(true)));
        assert!(is_ep("Hello-Goodbye", Some(false)));
        assert!(!is_ep("Hello", None));
        assert!(!is_ep("Sleep - Deep", Some(true)));
    }
}
