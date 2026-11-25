//! Answers the library requests of an open Apple Music library from the lists its binding keeps,
//! from Rune's own playlists and mixes it keeps on this device and from Apple's API, mapping
//! Apple's models to the hub's messages.

pub(crate) mod mix_collections;
pub(crate) mod search;

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use apple_music::models::{Artwork, LibraryAlbum, LibrarySong};
use database::actions::playlists::OCCURRENCE_PAGE_SIZES;
use database::actions::utils::generate_group_name;
use database::error::ExpectedError;
use futures::{StreamExt, TryStreamExt, stream};
use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::TypedRequest;
use crate::apple_music_artists::{NamedArtist, catalog_artist_remote, named_artist_remote};
use crate::apple_music_binding::{
    AppleIdKind, AppleMusicBinding, AppleSong, AppleSongs, CatalogCollection, CollectionInfo,
    MixRef, PlaylistRef, SongRef, catalog_track, cover_url, is_library_song, is_library_song_id,
    song_of_track,
};
use crate::apple_music_held::is_rune_album;
use crate::macros::{RequestSlots, RequestTranslator};
use crate::messages::{
    AddItemToMixRequest, AddItemToPlaylistRequest, AddItemToPlaylistResponse, Album, Artist,
    AudioAvailability, AudioOrigin, Collection, CollectionGroup, CollectionGroupSummary,
    CollectionGroupSummaryResponse, CollectionType, ComplexQueryRequest, ComplexQueryResponse,
    CreateMixRequest, CreatePlaylistRequest, CreatePlaylistResponse, FetchAllMixesRequest,
    FetchAllPlaylistsRequest, FetchAllPlaylistsResponse, FetchCollectionByIdsRequest,
    FetchCollectionByIdsResponse, FetchCollectionGroupSummaryRequest, FetchCollectionGroupsRequest,
    FetchCollectionGroupsResponse, FetchMediaFileByIdsRequest, FetchMediaFileByIdsResponse,
    FetchMediaFilesRequest, FetchMediaFilesResponse, FetchMixQueriesRequest,
    FetchParsedMediaFileRequest, FetchParsedMediaFileResponse, FetchPlaylistOccurrencesRequest,
    FetchPlaylistOccurrencesResponse, GetLikedRequest, GetLikedResponse, GetLyricByTrackIdRequest,
    GetLyricByTrackIdResponse, GetMediaFilesCountRequest, GetMediaFilesCountResponse,
    GetMixByIdRequest, GetPlaylistByIdRequest, GetPlaylistByIdResponse,
    GetPrimaryColorByTrackIdRequest, MediaFile, MediaFileSummary, MixQueryRequest,
    MixQueryResponse, PlayingItemRef, Playlist, PlaylistOccurrence, PlaylistOccurrenceCursor,
    RemoveItemFromPlaylistRequest, RemoveItemFromPlaylistResponse, RemoveMixRequest,
    RemovePlaylistRequest, RemovePlaylistResponse, ReorderPlaylistItemPositionRequest,
    ReorderPlaylistItemPositionResponse, ResolveMixQueryRequest, ResolveMixQueryResponse,
    SearchCollectionSummaryRequest, SearchCollectionSummaryResponse, SearchForRequest,
    SearchMediaFileSummaryRequest, SearchMediaFileSummaryResponse, SetLikedRequest,
    SetLikedResponse, UpdateMixRequest, UpdatePlaylistRequest, UpdatePlaylistResponse,
};
use crate::request_error::ExpectedRequestError;
use crate::rpc::{RequestTranslationFuture, Slot, UnsupportedReason};
use crate::service::{
    ServiceBinding, ServiceFuture,
    home::{self, HomeItem, HomeMix, HomeSource, HomeTrack},
    lrclib,
    mixes::{self, MixItem, MixSource, MixTarget, Plan, unsupported_query},
    positional_occurrences::{listed_after, occurrence_page, occurrences_response},
    rune_playlists::{StoredEntry, StoredPlaylist, TrackAbout},
};
use mix_collections::{
    add_item_to_mix_request, create_mix_request, fetch_all_mixes_request,
    fetch_mix_queries_request, get_mix_by_id_request, remove_mix_request, update_mix_request,
};
use search::search_for_request;

/// How many songs are looked up at once when a request names songs the binding lacks.
const LOOKUP_PARALLELISM: usize = 4;

/// How many albums of an artist are read at once.
const ALBUM_READS: usize = 4;

/// How many of a Rune playlist's first tracks are looked at for its cover.
const RUNE_COVER_CANDIDATES: usize = 10;

/// How many tracks of a Rune playlist are looked up together while a mix page is filled.
const RUNE_PAGE_STEP: usize = 64;

/// What the ids of albums, artists and playlists of the catalog hold before Apple's id, so they
/// never pass for those of the library.
pub(crate) const CATALOG_PREFIX: &str = "catalog:";

/// The group a playlist of the catalog is named with when a page opens one.
pub(crate) const APPLE_MUSIC_PLAYLIST_GROUP: &str = "Apple Music";

fn id_kind(collection_type: CollectionType, request: &'static str) -> Result<AppleIdKind> {
    match collection_type {
        CollectionType::Album => Ok(AppleIdKind::Album),
        CollectionType::Artist => Ok(AppleIdKind::Artist),
        // The playlists are Rune's own: Apple's are not listed.
        CollectionType::Playlist => Ok(AppleIdKind::RunePlaylist),
        // The mixes are answered on their own (see `mix_collections`), and genres and folders
        // are not served.
        _ => Err(unsupported_query(request)),
    }
}

fn to_count(value: i32) -> usize {
    usize::try_from(value.max(0)).unwrap_or(usize::MAX)
}

/// The name of a library album.
fn album_name(album: &LibraryAlbum) -> &str {
    album
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.name.as_deref())
        .unwrap_or_default()
}

/// The cover of a library album.
fn album_artwork(album: &LibraryAlbum) -> Option<&Artwork> {
    album.attributes.as_ref()?.artwork.as_ref()
}

/// One album, artist or playlist as listed, with the id the UI knows it by.
struct Entry {
    id: i32,
    name: String,
    cover: Option<String>,
    /// The group it is listed in: an album's or an artist's first letter, or a playlist's own
    /// group.
    group: String,
    /// Whether it cannot be changed here, as only Rune's own playlists can.
    readonly: bool,
}

/// Every library collection of one kind, numbered and remembered for later pages. The playlists
/// are Rune's own, kept on this device.
async fn entries(
    binding: &AppleMusicBinding,
    kind: AppleIdKind,
    request: &'static str,
) -> Result<Vec<Entry>> {
    let mut found = Vec::new();
    let generation = binding.generation();
    let mut add = |remote: &str, name: &str, artwork: Option<&Artwork>| {
        let id = binding.intern(kind, remote)?;
        binding.remember_collection(generation, kind, id, name, artwork);
        // A cover an earlier listing gave stays, when this one has none.
        let cover = binding
            .collection_info(kind, id)
            .and_then(|info| info.cover)
            .or_else(|| cover_url(artwork));
        found.push(Entry {
            id,
            name: name.to_owned(),
            cover,
            group: generate_group_name(name),
            readonly: true,
        });
        Ok::<_, anyhow::Error>(())
    };
    match kind {
        AppleIdKind::Album => {
            for album in binding.albums().await?.iter() {
                add(&album.id, album_name(album), album_artwork(album))?;
            }
        }
        AppleIdKind::Artist => {
            let named = binding.named_artists().await?;
            for artist in named.iter() {
                add(&artist.remote(), &artist.name, artist.artwork.as_ref())?;
            }
        }
        AppleIdKind::RunePlaylist => {
            for playlist in binding.rune_playlists.listed().await?.iter() {
                found.push(rune_entry(binding, playlist)?);
            }
        }
        AppleIdKind::LibrarySong
        | AppleIdKind::CatalogSong
        | AppleIdKind::CoverArt
        | AppleIdKind::Playlist
        | AppleIdKind::RuneMix
        | AppleIdKind::Mix => {
            return Err(unsupported_query(request));
        }
    }
    Ok(found)
}

/// A Rune playlist as listed. Its cover is that of the first of its first few tracks the binding
/// holds with a cover; no song is read for it.
fn rune_entry(binding: &AppleMusicBinding, playlist: &StoredPlaylist) -> Result<Entry> {
    let cover = playlist
        .entries
        .iter()
        .take(RUNE_COVER_CANDIDATES)
        .find_map(|entry| cover_url(binding.known_song(&entry.track)?.as_ref().artwork()));
    Ok(Entry {
        id: binding.intern(AppleIdKind::RunePlaylist, &playlist.id)?,
        name: playlist.name.clone(),
        cover,
        group: playlist.group.clone(),
        readonly: false,
    })
}

async fn collection_of(
    binding: &AppleMusicBinding,
    entry: &Entry,
    collection_type: CollectionType,
) -> Result<Collection> {
    let covers = entry
        .cover
        .clone()
        .map(|url| vec![(entry.id, url)])
        .unwrap_or_default();
    Ok(Collection {
        id: entry.id,
        name: entry.name.clone(),
        queries: Vec::new(),
        collection_type,
        cover_art_map: binding.cover_map(covers).await?,
        readonly: entry.readonly,
    })
}

pub(crate) async fn fetch_collection_group_summary_request(
    binding: &AppleMusicBinding,
    request: &FetchCollectionGroupSummaryRequest,
) -> Result<Option<CollectionGroupSummaryResponse>> {
    if request.collection_type == CollectionType::Mix {
        return mix_collections::mix_group_summary(binding).await.map(Some);
    }
    let kind = id_kind(
        request.collection_type,
        FetchCollectionGroupSummaryRequest::wire_name(),
    )?;
    let entries = entries(
        binding,
        kind,
        FetchCollectionGroupSummaryRequest::wire_name(),
    )
    .await?;
    let mut groups = BTreeMap::<String, i32>::new();
    for entry in &entries {
        *groups.entry(entry.group.clone()).or_default() += 1;
    }
    Ok(Some(CollectionGroupSummaryResponse {
        call_id: None,
        collection_type: request.collection_type,
        groups: groups
            .into_iter()
            .map(|(group_title, count)| CollectionGroupSummary {
                // The library fills the group of Apple Music's playlists itself, with the
                // playlists it imported.
                reserved: kind == AppleIdKind::RunePlaylist
                    && group_title == APPLE_MUSIC_PLAYLIST_GROUP,
                group_title,
                count,
            })
            .collect(),
    }))
}

pub(crate) async fn fetch_collection_groups_request(
    binding: &AppleMusicBinding,
    request: &FetchCollectionGroupsRequest,
) -> Result<Option<FetchCollectionGroupsResponse>> {
    if request.collection_type == CollectionType::Mix {
        return mix_collections::mix_collection_groups(binding, &request.group_titles)
            .await
            .map(Some);
    }
    let kind = id_kind(
        request.collection_type,
        FetchCollectionGroupsRequest::wire_name(),
    )?;
    let entries = entries(binding, kind, FetchCollectionGroupsRequest::wire_name()).await?;
    let mut groups = Vec::with_capacity(request.group_titles.len());
    for title in &request.group_titles {
        let mut members: Vec<&Entry> = entries
            .iter()
            .filter(|entry| &entry.group == title)
            .collect();
        members.sort_by(|left, right| left.name.cmp(&right.name));
        let mut collections = Vec::with_capacity(members.len());
        for entry in members {
            collections.push(collection_of(binding, entry, request.collection_type).await?);
        }
        groups.push(CollectionGroup {
            group_title: title.clone(),
            collections,
        });
    }
    Ok(Some(FetchCollectionGroupsResponse {
        call_id: None,
        groups,
    }))
}

fn not_apple_collection() -> anyhow::Error {
    ExpectedRequestError::invalid_input("Not an Apple Music collection id").into()
}

fn collection_not_found() -> ExpectedError {
    ExpectedError::NotFound {
        what: "Apple Music collection",
    }
}

/// The library album or library artist a collection id the UI holds stands for, by Apple's id.
/// An artist of the library's artist list is a name Apple gives no id for, so its id is refused
/// here, and every request that takes artist ids looks for it first. A collection of the catalog
/// is not the library's, and is not found.
fn library_collection(binding: &AppleMusicBinding, kind: AppleIdKind, id: i32) -> Result<String> {
    if binding.named_artist_key(kind, id).is_some() {
        return Err(not_apple_collection());
    }
    let remote = binding.resolve(kind, id)?;
    if remote.starts_with(CATALOG_PREFIX) {
        return Err(collection_not_found().into());
    }
    Ok(remote)
}

/// What a library artist is listed with, as the library's artists were read. The artist list
/// shows the artists their names credit instead, so a library artist the UI holds is looked for
/// among them. The artist is remembered.
async fn library_artist_info(
    binding: &AppleMusicBinding,
    id: i32,
    remote: &str,
) -> Result<CollectionInfo> {
    let generation = binding.generation();
    let artists = binding.artists().await?;
    let artist = artists
        .iter()
        .find(|artist| artist.id == remote)
        .ok_or(collection_not_found())?;
    let name = artist
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.name.clone())
        .unwrap_or_default();
    let catalog = artist.catalog_artists();
    let artwork = catalog
        .first()
        .and_then(|catalog| catalog.attributes.as_ref()?.artwork.as_ref());
    binding.remember_collection(generation, AppleIdKind::Artist, id, &name, artwork);
    Ok(CollectionInfo {
        name,
        cover: cover_url(artwork),
    })
}

pub(crate) async fn fetch_collection_by_ids_request(
    binding: &AppleMusicBinding,
    request: &FetchCollectionByIdsRequest,
) -> Result<Option<FetchCollectionByIdsResponse>> {
    let wire_name = FetchCollectionByIdsRequest::wire_name();
    // The mix editor asks for every kind its queries may name, genres too, with no ids for the
    // kinds a mix does not name.
    if request.ids.is_empty() {
        return Ok(Some(FetchCollectionByIdsResponse {
            call_id: None,
            collection_type: request.collection_type,
            result: Vec::new(),
        }));
    }
    if request.collection_type == CollectionType::Mix {
        return mix_collections::mixes_by_ids(binding, &request.ids)
            .await
            .map(Some);
    }
    let kind = id_kind(request.collection_type, wire_name)?;
    if kind == AppleIdKind::RunePlaylist {
        return Ok(Some(FetchCollectionByIdsResponse {
            call_id: None,
            collection_type: request.collection_type,
            result: playlists_by_ids(binding, &request.ids).await?,
        }));
    }
    let mut result = Vec::with_capacity(request.ids.len());
    for id in &request.ids {
        let info = match binding.collection_info(kind, *id) {
            Some(info) => info,
            None => {
                if let Some(remote) = binding.named_artist_key(kind, *id) {
                    // An id from before the list told its artists apart by their catalog
                    // artists, which the list does not number, opens as the artist its
                    // spelling goes with now.
                    let artist = match binding.named_artist(&remote).await {
                        Ok(artist) => artist,
                        // The list no longer holds it; the rest still answers.
                        Err(error) if is_not_found(&error) => {
                            log::warn!(
                                "Skipping the Apple Music artist {remote}: the library no longer lists it"
                            );
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                    binding.remember_collection(
                        binding.generation(),
                        kind,
                        *id,
                        &artist.name,
                        artist.artwork.as_ref(),
                    );
                    CollectionInfo {
                        name: artist.name.clone(),
                        cover: cover_url(artist.artwork.as_ref()),
                    }
                } else if let Some((collection, catalog_id)) = catalog_album(binding, kind, *id)? {
                    catalog_collection_info(binding, collection, &catalog_id).await?
                } else {
                    let remote = library_collection(binding, kind, *id)?;
                    if kind == AppleIdKind::Artist {
                        library_artist_info(binding, *id, &remote).await?
                    } else {
                        // Reading the list remembers every entry in it, this one included.
                        entries(binding, kind, wire_name).await?;
                        match binding.collection_info(kind, *id) {
                            Some(info) => info,
                            None => {
                                // Something stored earlier may name a collection that is gone,
                                // as an album the library no longer holds; the rest still
                                // answers.
                                log::warn!(
                                    "Skipping the Apple Music {kind:?} {remote}: it names no collection"
                                );
                                continue;
                            }
                        }
                    }
                }
            }
        };
        let entry = Entry {
            id: *id,
            group: generate_group_name(&info.name),
            name: info.name,
            cover: info.cover,
            readonly: true,
        };
        result.push(collection_of(binding, &entry, request.collection_type).await?);
    }
    Ok(Some(FetchCollectionByIdsResponse {
        call_id: None,
        collection_type: request.collection_type,
        result,
    }))
}

/// The playlists among `ids`: Rune's as they are listed, and those of the catalog that a page
/// opened. A Rune playlist removed meanwhile is left out, as a local library leaves it out, and
/// so is every one while their file cannot be read.
async fn playlists_by_ids(binding: &AppleMusicBinding, ids: &[i32]) -> Result<Vec<Collection>> {
    let kept = binding.rune_playlists.listed().await?;
    let mut result = Vec::with_capacity(ids.len());
    for id in ids {
        let entry = match binding.playlist_ref(*id) {
            PlaylistRef::Rune(stored) => match kept.iter().find(|playlist| playlist.id == stored) {
                Some(playlist) => rune_entry(binding, playlist)?,
                None => continue,
            },
            PlaylistRef::Catalog(id) => {
                let catalog_id = catalog_playlist_id(binding, id)?;
                let collection = CatalogCollection::Playlist(id);
                let info = catalog_collection_info(binding, collection, &catalog_id).await?;
                Entry {
                    id,
                    name: info.name,
                    cover: info.cover,
                    group: APPLE_MUSIC_PLAYLIST_GROUP.to_owned(),
                    readonly: true,
                }
            }
        };
        result.push(collection_of(binding, &entry, CollectionType::Playlist).await?);
    }
    Ok(result)
}

/// The catalog id of the catalog playlist the UI numbered `id`.
fn catalog_playlist_id(binding: &AppleMusicBinding, id: i32) -> Result<String> {
    let remote = binding.resolve(AppleIdKind::Playlist, id)?;
    remote
        .strip_prefix(CATALOG_PREFIX)
        .map(str::to_owned)
        .ok_or_else(not_apple_collection)
}

/// The catalog album or playlist a mix target names, with its catalog id; None for a
/// collection of the library and a Rune playlist.
fn catalog_collection(
    binding: &AppleMusicBinding,
    target: MixTarget,
) -> Result<Option<(CatalogCollection, String)>> {
    let (collection, remote) = match target {
        MixTarget::Album(id) => (
            CatalogCollection::Album(id),
            binding.resolve(AppleIdKind::Album, id)?,
        ),
        MixTarget::Playlist(id) => match binding.playlist_ref(id) {
            PlaylistRef::Rune(_) => return Ok(None),
            PlaylistRef::Catalog(id) => (
                CatalogCollection::Playlist(id),
                binding.resolve(AppleIdKind::Playlist, id)?,
            ),
        },
        MixTarget::Artist(_) => return Ok(None),
    };
    Ok(remote
        .strip_prefix(CATALOG_PREFIX)
        .map(|catalog_id| (collection, catalog_id.to_owned())))
}

/// The catalog album the collection id `id` of `kind` names, with its catalog id; None for any
/// other collection.
fn catalog_album(
    binding: &AppleMusicBinding,
    kind: AppleIdKind,
    id: i32,
) -> Result<Option<(CatalogCollection, String)>> {
    if kind != AppleIdKind::Album {
        return Ok(None);
    }
    catalog_collection(binding, MixTarget::Album(id))
}

/// What a catalog album or playlist is listed with: as the recommendations or an earlier read
/// listed it, or else read from Apple's catalog, and remembered.
async fn catalog_collection_info(
    binding: &AppleMusicBinding,
    collection: CatalogCollection,
    catalog_id: &str,
) -> Result<CollectionInfo> {
    let (kind, id) = collection.kind_and_id();
    if let Some(info) = binding.collection_info(kind, id) {
        return Ok(info);
    }
    let generation = binding.generation();
    let (name, artwork) = match collection {
        CatalogCollection::Album(_) => {
            let album = binding.read_catalog_album(catalog_id).await?;
            let attributes = album.attributes.unwrap_or_default();
            (attributes.name, attributes.artwork)
        }
        CatalogCollection::Playlist(_) => {
            let playlist = binding.read_catalog_playlist(catalog_id).await?;
            let attributes = playlist.attributes.unwrap_or_default();
            (attributes.name, attributes.artwork)
        }
    };
    let name = name.unwrap_or_default();
    binding.remember_collection(generation, kind, id, &name, artwork.as_ref());
    Ok(CollectionInfo {
        name,
        cover: cover_url(artwork.as_ref()),
    })
}

/// The songs of a catalog album or playlist as tracks of the catalog, as
/// `catalog_collection_songs` gives them.
async fn catalog_songs(
    binding: &AppleMusicBinding,
    collection: CatalogCollection,
    catalog_id: &str,
    afresh: bool,
) -> Result<Vec<Listed>> {
    Ok(binding
        .catalog_collection_songs(collection, catalog_id, afresh)
        .await?
        .iter()
        .map(|song| (catalog_track(&song.id), AppleSong::Catalog(song.clone())))
        .collect())
}

pub(crate) async fn search_collection_summary_request(
    binding: &AppleMusicBinding,
    request: &SearchCollectionSummaryRequest,
) -> Result<Option<SearchCollectionSummaryResponse>> {
    let wire_name = SearchCollectionSummaryRequest::wire_name();
    let collection_type = request.collection_type.ok_or_else(|| {
        ExpectedRequestError::invalid_input("Collection summary requires a collection type")
    })?;
    let kind = id_kind(collection_type, wire_name)?;
    let entries = entries(binding, kind, wire_name).await?;
    let mut result = Vec::new();
    for entry in entries.iter().take(to_count(request.n)) {
        result.push(collection_of(binding, entry, collection_type).await?);
    }
    Ok(Some(SearchCollectionSummaryResponse {
        collection_type,
        result,
        call_id: request.call_id,
    }))
}

/// The wire form of a song. `track` is how the caller named it, which a library copy of a
/// catalog song must keep.
fn media_file(
    binding: &AppleMusicBinding,
    id: i32,
    track: &str,
    song: SongRef<'_>,
    cover_art_id: Option<i32>,
) -> MediaFile {
    MediaFile {
        id,
        library_id: ServiceBinding::library_id(binding).to_string(),
        track: track.to_owned(),
        path: String::new(),
        artist: song.artist().unwrap_or_default().to_owned(),
        album: song.album().unwrap_or_default().to_owned(),
        title: song.title().to_owned(),
        duration: song.duration(),
        cover_art_id,
        track_number: song
            .track_number()
            .and_then(|number| i32::try_from(number).ok()),
        audio_availability: binding.audio_availability(track),
        audio_origin: AudioOrigin::Remote,
    }
}

/// Songs in wire form, and the `(media file id, address)` pairs of their covers.
type WireSongs = (Vec<MediaFile>, Vec<(i32, String)>);

/// Numbers songs and their covers and builds their wire form.
fn media_files<'a>(
    binding: &AppleMusicBinding,
    songs: impl IntoIterator<Item = (&'a str, SongRef<'a>)>,
) -> Result<WireSongs> {
    let mut files = Vec::new();
    let mut covers = Vec::new();
    for (track, song) in songs {
        let id = binding.intern_track(track)?;
        let cover = cover_url(song.artwork());
        let cover_art_id = cover
            .as_deref()
            .map(|url| binding.intern(AppleIdKind::CoverArt, url))
            .transpose()?;
        files.push(media_file(binding, id, track, song, cover_art_id));
        covers.extend(cover.map(|url| (id, url)));
    }
    Ok((files, covers))
}

/// The tracks and songs of owned pairs, as `media_files` takes them.
fn song_refs(songs: &[(String, AppleSong)]) -> impl Iterator<Item = (&str, SongRef<'_>)> {
    songs
        .iter()
        .map(|(track, song)| (track.as_str(), song.as_ref()))
}

/// The entries of the song list, as `media_files` takes them.
fn listed<'a>(
    entries: impl IntoIterator<Item = (&'a str, &'a LibrarySong)>,
) -> impl Iterator<Item = (&'a str, SongRef<'a>)> {
    entries
        .into_iter()
        .map(|(track, song)| (track, SongRef::Library(song)))
}

/// Whether a listed song has a cover.
fn has_cover(song: &LibrarySong) -> bool {
    song.attributes
        .as_ref()
        .is_some_and(|attributes| attributes.artwork.is_some())
}

pub(crate) async fn fetch_media_files_request(
    binding: &AppleMusicBinding,
    request: &FetchMediaFilesRequest,
) -> Result<Option<FetchMediaFilesResponse>> {
    let offset = usize::try_from(request.cursor)
        .map_err(|error| ExpectedRequestError::invalid_input(format!("Invalid cursor: {error}")))?;
    let count = usize::try_from(request.page_size).map_err(|error| {
        ExpectedRequestError::invalid_input(format!("Invalid page size: {error}"))
    })?;
    let list = binding.songs().await?;
    let (media_files, covers) =
        media_files(binding, listed(list.entries().skip(offset).take(count)))?;
    let cover_art_map = if request.bake_cover_arts {
        binding.cover_map(covers).await?
    } else {
        HashMap::new()
    };
    Ok(Some(FetchMediaFilesResponse {
        media_files,
        cover_art_map,
        call_id: request.call_id,
    }))
}

pub(crate) async fn fetch_media_file_by_ids_request(
    binding: &AppleMusicBinding,
    request: &FetchMediaFileByIdsRequest,
) -> Result<Option<FetchMediaFileByIdsResponse>> {
    let tracks = request
        .ids
        .iter()
        .map(|id| binding.resolve_track(*id))
        .collect::<Result<Vec<_>>>()?;
    // What the binding holds answers at once; Apple is asked for the rest together. A song
    // Apple no longer has is left out, so one gone song does not fail the others.
    binding.read_unknown_tracks(tracks.iter().cloned()).await;
    let songs: Vec<(String, AppleSong)> = stream::iter(tracks)
        .map(|track| async move {
            let song = apple_song(binding, &track).await?;
            Ok::<_, anyhow::Error>(song.map(|song| (track, song)))
        })
        .buffered(LOOKUP_PARALLELISM)
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .flatten()
        .collect();
    let (media_files, covers) = media_files(binding, song_refs(&songs))?;
    Ok(Some(FetchMediaFileByIdsResponse {
        call_id: None,
        media_files,
        cover_art_map: binding.cover_map(covers).await?,
    }))
}

pub(crate) async fn get_media_files_count_request(
    binding: &AppleMusicBinding,
    request: &GetMediaFilesCountRequest,
) -> Result<Option<GetMediaFilesCountResponse>> {
    let list = binding.songs().await?;
    Ok(Some(GetMediaFilesCountResponse {
        count: i32::try_from(list.len()).map_err(|error| {
            ExpectedRequestError::invalid_input(format!("Invalid song count: {error}"))
        })?,
        call_id: request.call_id,
    }))
}

pub(crate) async fn fetch_parsed_media_file_request(
    binding: &AppleMusicBinding,
    request: &FetchParsedMediaFileRequest,
) -> Result<Option<FetchParsedMediaFileResponse>> {
    let track = binding.resolve_track(request.id)?;
    let song = binding.song(&track).await?;
    // A catalog song a search found names no album yet, which the album's page needs.
    let song = binding.with_catalog_album(&track, song).await;
    let song_ref = song.as_ref();
    // A catalog song names catalog albums and artists, which carry their own prefix so they
    // never pass for a library one. A song Rune holds in the library is on an album of the
    // library or one Rune holds.
    let in_library = match &song {
        AppleSong::Library(song) => {
            is_library_song(song) || binding.known_songs().is_some_and(|list| list.holds(&track))
        }
        AppleSong::Catalog(_) => false,
    };
    let collection = |id: &str| {
        if in_library {
            id.to_owned()
        } else {
            format!("{CATALOG_PREFIX}{id}")
        }
    };
    // The catalog artists Apple names for the song are the artists the artist list shows for
    // it, each as the catalog names it.
    let song_artists = binding.kept_song_artists().await;
    let catalog_artists = match (&song, song_artists.as_deref()) {
        (AppleSong::Library(listed), Some(song_artists)) => {
            song_artists.of(listed).map(|ids| (song_artists, ids))
        }
        _ => None,
    };
    let artists = if let Some((song_artists, ids)) = catalog_artists {
        ids.iter()
            .filter_map(|id| Some((id, song_artists.artists.get(id)?)))
            .map(|(id, artist)| {
                Ok::<_, anyhow::Error>(Artist {
                    id: binding.intern(AppleIdKind::Artist, &catalog_artist_remote(id))?,
                    name: artist.name.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?
    } else if let Some(name) = song_ref.artist() {
        // A song Apple names no catalog artist for links to the artist of its credit as a
        // whole, which the artist list shows. A catalog song with an artist relationship links
        // to that catalog artist.
        let remote = match (in_library, song_ref.artist_id()) {
            (false, Some(id)) => Some(collection(id)),
            _ => named_artist_remote(name),
        };
        // A credit that names nobody has nothing to link to, and id 0 is no artist.
        remote
            .map(|remote| {
                Ok::<_, anyhow::Error>(Artist {
                    id: binding.intern(AppleIdKind::Artist, &remote)?,
                    name: name.to_owned(),
                })
            })
            .transpose()?
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    let album_id = song_ref
        .album_id()
        .map(|id| binding.intern(AppleIdKind::Album, &collection(id)))
        .transpose()?
        .unwrap_or_default();
    let album = Album {
        id: album_id,
        name: song_ref.album().unwrap_or_default().to_owned(),
    };
    let (mut files, _) = media_files(binding, [(track.as_str(), song_ref)])?;
    Ok(Some(FetchParsedMediaFileResponse {
        file: files.remove(0),
        artists,
        album,
        call_id: request.call_id,
    }))
}

pub(crate) async fn search_media_file_summary_request(
    binding: &AppleMusicBinding,
    request: &SearchMediaFileSummaryRequest,
) -> Result<Option<SearchMediaFileSummaryResponse>> {
    let count = usize::try_from(request.n)
        .map_err(|error| ExpectedRequestError::invalid_input(format!("Invalid N: {error}")))?;
    let list = binding.songs().await?;
    let mut result = Vec::new();
    for (track, song) in list.entries().take(count) {
        let song = SongRef::Library(song);
        let id = binding.intern_track(track)?;
        let cover_art_id = cover_url(song.artwork())
            .as_deref()
            .map(|url| binding.intern(AppleIdKind::CoverArt, url))
            .transpose()?;
        result.push(MediaFileSummary {
            id,
            name: song.title().to_owned(),
            cover_art_id,
            audio_availability: binding.audio_availability(track),
        });
    }
    Ok(Some(SearchMediaFileSummaryResponse {
        result,
        call_id: request.call_id,
    }))
}

/// The id the store keeps the Rune playlist a write names under. Rune writes to no playlist of
/// Apple's: one of the catalog is refused as unsupported by `request`, and an id that names no
/// playlist is an input error.
fn rune_playlist_to_change(
    binding: &AppleMusicBinding,
    id: i32,
    request: &'static str,
) -> Result<String> {
    match binding.playlist_ref(id) {
        PlaylistRef::Rune(stored) => Ok(stored),
        PlaylistRef::Catalog(id) => {
            binding.resolve(AppleIdKind::Playlist, id)?;
            Err(unsupported_query(request))
        }
    }
}

/// The store's id of the Rune playlist a mix target names; None for any other target.
fn rune_playlist_target(binding: &AppleMusicBinding, target: MixTarget) -> Option<String> {
    match target {
        MixTarget::Playlist(id) => match binding.playlist_ref(id) {
            PlaylistRef::Rune(stored) => Some(stored),
            PlaylistRef::Catalog(_) => None,
        },
        MixTarget::Album(_) | MixTarget::Artist(_) => None,
    }
}

/// A Rune playlist in wire form.
fn rune_playlist_of(binding: &AppleMusicBinding, playlist: &StoredPlaylist) -> Result<Playlist> {
    Ok(Playlist {
        id: binding.intern(AppleIdKind::RunePlaylist, &playlist.id)?,
        name: playlist.name.clone(),
        group: playlist.group.clone(),
    })
}

/// A position a request gives, which may not be negative.
fn position(value: i32) -> Result<usize> {
    usize::try_from(value)
        .map_err(|_| ExpectedRequestError::invalid_input("Invalid playlist position").into())
}

/// A position as a response gives it.
fn wire_position(index: usize) -> Result<i32> {
    i32::try_from(index)
        .map_err(|_| ExpectedRequestError::invalid_input("Playlist is too long").into())
}

/// The playlists a song can be added to, which is what the UI lists them for: Rune's own, by
/// group, as a local library lists them. Apple's own are not listed, and never written to.
pub(crate) async fn fetch_all_playlists_request(
    binding: &AppleMusicBinding,
    _: &FetchAllPlaylistsRequest,
) -> Result<Option<FetchAllPlaylistsResponse>> {
    let mut playlists = binding
        .rune_playlists
        .listed()
        .await?
        .iter()
        .map(|playlist| rune_playlist_of(binding, playlist))
        .collect::<Result<Vec<_>>>()?;
    playlists.sort_by(|left, right| left.group.cmp(&right.group));
    Ok(Some(FetchAllPlaylistsResponse {
        playlists,
        call_id: None,
    }))
}

/// A playlist by id: a Rune playlist, or one of the catalog that a page opened, as a recommended
/// mix of it does.
pub(crate) async fn get_playlist_by_id_request(
    binding: &AppleMusicBinding,
    request: &GetPlaylistByIdRequest,
) -> Result<Option<GetPlaylistByIdResponse>> {
    let playlist = match binding.playlist_ref(request.playlist_id) {
        PlaylistRef::Rune(stored) => {
            rune_playlist_of(binding, &binding.rune_playlists.playlist(&stored).await?)?
        }
        PlaylistRef::Catalog(id) => {
            let catalog_id = catalog_playlist_id(binding, id)?;
            let collection = CatalogCollection::Playlist(id);
            let info = catalog_collection_info(binding, collection, &catalog_id).await?;
            Playlist {
                id,
                name: info.name,
                group: APPLE_MUSIC_PLAYLIST_GROUP.to_owned(),
            }
        }
    };
    Ok(Some(GetPlaylistByIdResponse {
        playlist,
        call_id: None,
    }))
}

/// The group a Rune playlist is put in, which may not be the one that holds the playlists
/// imported from Apple Music: the library fills that one itself.
fn rune_group(group: &str) -> Result<&str> {
    if group
        .trim()
        .eq_ignore_ascii_case(APPLE_MUSIC_PLAYLIST_GROUP)
    {
        return Err(ExpectedRequestError::invalid_input(format!(
            "The group {APPLE_MUSIC_PLAYLIST_GROUP} holds the playlists of Apple Music"
        ))
        .into());
    }
    Ok(group)
}

/// Makes a Rune playlist, kept on this device.
pub(crate) async fn create_playlist_request(
    binding: &AppleMusicBinding,
    request: &CreatePlaylistRequest,
) -> Result<Option<CreatePlaylistResponse>> {
    let group = rune_group(&request.group)?;
    let made = binding.rune_playlists.create(&request.name, group).await?;
    binding.content_changed();
    Ok(Some(CreatePlaylistResponse {
        playlist: rune_playlist_of(binding, &made)?,
        call_id: None,
    }))
}

pub(crate) async fn update_playlist_request(
    binding: &AppleMusicBinding,
    request: &UpdatePlaylistRequest,
) -> Result<Option<UpdatePlaylistResponse>> {
    let stored = rune_playlist_to_change(
        binding,
        request.playlist_id,
        UpdatePlaylistRequest::wire_name(),
    )?;
    // An imported playlist is in the group that holds the imported playlists, which it may stay
    // in as it is renamed, but which no playlist may be moved to.
    let current = binding.rune_playlists.playlist(&stored).await?;
    let group = if request.group == current.group {
        request.group.as_str()
    } else {
        rune_group(&request.group)?
    };
    let playlist = binding
        .rune_playlists
        .update(&stored, &request.name, group)
        .await?;
    binding.content_changed();
    Ok(Some(UpdatePlaylistResponse {
        playlist: rune_playlist_of(binding, &playlist)?,
        call_id: None,
    }))
}

pub(crate) async fn remove_playlist_request(
    binding: &AppleMusicBinding,
    request: &RemovePlaylistRequest,
) -> Result<Option<RemovePlaylistResponse>> {
    let stored = rune_playlist_to_change(
        binding,
        request.playlist_id,
        RemovePlaylistRequest::wire_name(),
    )?;
    binding.rune_playlists.remove(&stored).await?;
    binding.content_changed();
    Ok(Some(RemovePlaylistResponse {
        playlist_id: request.playlist_id,
        call_id: None,
    }))
}

/// What is known of the song of `track`, as a Rune playlist keeps it with its entry: enough to
/// play it once its library copy is gone, or to name it once Apple no longer has it.
pub(crate) fn track_about(track: &str, song: SongRef<'_>) -> TrackAbout {
    let catalog_id = match song {
        SongRef::Library(song) if is_library_song_id(track) => song.store_id().map(str::to_owned),
        _ => None,
    };
    TrackAbout {
        catalog_id,
        title: Some(song.title())
            .filter(|title| !title.is_empty())
            .map(str::to_owned),
        artist: song.artist().map(str::to_owned),
        album: song.album().map(str::to_owned),
    }
}

/// Puts a song of the library or of the catalog into a Rune playlist, at the position asked or
/// last. A catalog song is kept as it is, and nothing is added to the library. Its entry keeps
/// the occurrence id answered for as long as it is in the playlist, and what the binding holds
/// of the song.
pub(crate) async fn add_item_to_playlist_request(
    binding: &AppleMusicBinding,
    request: &AddItemToPlaylistRequest,
) -> Result<Option<AddItemToPlaylistResponse>> {
    let stored = rune_playlist_to_change(
        binding,
        request.playlist_id,
        AddItemToPlaylistRequest::wire_name(),
    )?;
    let track = binding.resolve_track(request.media_file_id)?;
    let at = request.position.map(position).transpose()?;
    // The UI just listed it, so Apple has it.
    binding.found_again(&track);
    let about = binding
        .known_song(&track)
        .map(|song| track_about(&track, song.as_ref()))
        .unwrap_or_default();
    let occurrence_id = binding
        .rune_playlists
        .add(&stored, &track, about, at)
        .await?;
    binding.content_changed();
    Ok(Some(AddItemToPlaylistResponse {
        occurrence_id,
        call_id: None,
    }))
}

/// Moves a track of a Rune playlist so that it ends up at the position asked, or last.
pub(crate) async fn reorder_playlist_item_position_request(
    binding: &AppleMusicBinding,
    request: &ReorderPlaylistItemPositionRequest,
) -> Result<Option<ReorderPlaylistItemPositionResponse>> {
    let stored = rune_playlist_to_change(
        binding,
        request.playlist_id,
        ReorderPlaylistItemPositionRequest::wire_name(),
    )?;
    let to = position(request.new_position)?;
    binding
        .rune_playlists
        .move_entry(&stored, &request.occurrence_id, to)
        .await?;
    binding.content_changed();
    Ok(Some(ReorderPlaylistItemPositionResponse { call_id: None }))
}

pub(crate) async fn remove_item_from_playlist_request(
    binding: &AppleMusicBinding,
    request: &RemoveItemFromPlaylistRequest,
) -> Result<Option<RemoveItemFromPlaylistResponse>> {
    let stored = rune_playlist_to_change(
        binding,
        request.playlist_id,
        RemoveItemFromPlaylistRequest::wire_name(),
    )?;
    binding
        .rune_playlists
        .remove_entry(&stored, &request.occurrence_id)
        .await?;
    binding.content_changed();
    Ok(Some(RemoveItemFromPlaylistResponse { call_id: None }))
}

/// The catalog song an entry's library song was a copy of, when that was known when it was
/// added. It plays the entry once the library copy is gone.
pub(crate) fn catalog_copy(entry: &StoredEntry) -> Option<String> {
    if !is_library_song_id(&entry.track) {
        return None;
    }
    entry.about.catalog_id.as_deref().map(catalog_track)
}

/// Whether Apple said it does not have the song of `track`, and nothing listed it since.
pub(crate) fn gone(binding: &AppleMusicBinding, track: &str) -> bool {
    binding.known_missing(track) && binding.known_song(track).is_none()
}

/// The track a Rune playlist's entry plays: its own, or the catalog song its library song was a
/// copy of once Apple said the library copy is gone. Nothing is read for it.
fn playing_track(binding: &AppleMusicBinding, entry: &StoredEntry) -> String {
    if gone(binding, &entry.track)
        && let Some(copy) = catalog_copy(entry)
    {
        return copy;
    }
    entry.track.clone()
}

/// The songs of a Rune playlist's entries, in order, with the tracks they play: those the binding
/// holds, then those it lacks, read together. An entry whose library song is gone plays the
/// catalog song it was a copy of, when that is known. A track Apple no longer has, or that names
/// no song, is None, so its entry is still listed and can be removed; Apple is not asked for it
/// again under this sign-in.
async fn rune_songs(binding: &AppleMusicBinding, entries: &[StoredEntry]) -> Vec<Option<Listed>> {
    let mut read = binding
        .read_unknown_tracks(entries.iter().map(|entry| entry.track.clone()))
        .await;
    // A library song that read found gone plays its catalog copy, which is read too.
    let copies: Vec<String> = entries
        .iter()
        .map(|entry| playing_track(binding, entry))
        .zip(entries)
        .filter(|(track, entry)| *track != entry.track)
        .map(|(track, _)| track)
        .collect();
    if !copies.is_empty() {
        read.extend(binding.read_unknown_tracks(copies).await);
    }
    entries
        .iter()
        .map(|entry| rune_song(binding, &read, entry))
        .collect()
}

/// The song of one entry of a Rune playlist, as `rune_songs` reads them: from what was read, else
/// from what the binding holds.
fn rune_song(
    binding: &AppleMusicBinding,
    read: &HashMap<String, AppleSong>,
    entry: &StoredEntry,
) -> Option<Listed> {
    let held = |track: &str| {
        if gone(binding, track) {
            return None;
        }
        read.get(track)
            .cloned()
            .or_else(|| binding.known_song(track))
    };
    if let Some(song) = held(&entry.track) {
        return Some((entry.track.clone(), song));
    }
    let copy = catalog_copy(entry)?;
    held(&copy).map(|song| (copy, song))
}

/// An entry whose song Apple no longer has, named as it was when it was added; None when nothing
/// was known of it. It cannot play, and its id is 0, as the UI numbers an entry without a file.
fn remembered_file(binding: &AppleMusicBinding, entry: &StoredEntry) -> Option<MediaFile> {
    let title = entry.about.title.clone()?;
    Some(MediaFile {
        id: 0,
        library_id: ServiceBinding::library_id(binding).to_string(),
        track: entry.track.clone(),
        path: String::new(),
        artist: entry.about.artist.clone().unwrap_or_default(),
        album: entry.about.album.clone().unwrap_or_default(),
        title,
        duration: 0.0,
        cover_art_id: None,
        track_number: None,
        audio_availability: AudioAvailability::Unavailable,
        audio_origin: AudioOrigin::Remote,
    })
}

/// Where a page of `entries` starts: after the entry the page before ended with, or, when that
/// entry was taken out meanwhile, at the position it had.
fn page_start(entries: &[StoredEntry], after: Option<&PlaylistOccurrenceCursor>) -> Result<usize> {
    let Some(after) = after else {
        return Ok(0);
    };
    let start = match entries
        .iter()
        .position(|entry| entry.occurrence == after.occurrence_id)
    {
        Some(index) => index + 1,
        None => usize::try_from(after.position)
            .map_err(|_| ExpectedRequestError::invalid_input("Invalid occurrence cursor"))?,
    };
    Ok(start.min(entries.len()))
}

/// The entries of `page`, which starts at the position `start`, as a playlist page lists them,
/// and the `(media file id, address)` pairs of their covers. An entry that plays its catalog copy
/// is named by that song's track.
async fn listed_occurrences(
    binding: &AppleMusicBinding,
    page: &[StoredEntry],
    start: usize,
) -> Result<(Vec<PlaylistOccurrence>, Vec<(i32, String)>)> {
    let songs = rune_songs(binding, page).await;
    let found: Vec<Listed> = songs.iter().flatten().cloned().collect();
    let (files, covers) = media_files(binding, song_refs(&found))?;
    let mut files = files.into_iter();
    let listed = page
        .iter()
        .zip(&songs)
        .enumerate()
        .map(|(offset, (entry, song))| {
            let (track_uuid, file) = match song {
                Some((track, _)) => (track.clone(), files.next()),
                None => (entry.track.clone(), remembered_file(binding, entry)),
            };
            Ok(PlaylistOccurrence {
                occurrence_id: entry.occurrence.clone(),
                position: wire_position(start + offset)?,
                track_uuid,
                file,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((listed, covers))
}

/// Where the page after one that ends before the position `end` of `entries` starts; None when
/// that page ends the playlist.
fn next_cursor(entries: &[StoredEntry], end: usize) -> Result<Option<PlaylistOccurrenceCursor>> {
    match entries[..end].last() {
        Some(last) if end < entries.len() => Ok(Some(PlaylistOccurrenceCursor {
            position: wire_position(end - 1)?,
            occurrence_id: last.occurrence.clone(),
        })),
        _ => Ok(None),
    }
}

/// A page of a playlist: of a Rune playlist, or of one of the catalog that a page opened.
pub(crate) async fn fetch_playlist_occurrences_request(
    binding: &AppleMusicBinding,
    request: &FetchPlaylistOccurrencesRequest,
) -> Result<Option<FetchPlaylistOccurrencesResponse>> {
    match binding.playlist_ref(request.playlist_id) {
        PlaylistRef::Rune(stored) => rune_occurrences(binding, &stored, request).await,
        PlaylistRef::Catalog(id) => catalog_playlist_occurrences(binding, id, request).await,
    }
}

/// A page of a playlist of the catalog, whose entries are named by their positions in the order
/// they were listed in (see `positional_occurrences`). The first page reads the playlist as Apple
/// has it now; the later ones page through what is kept.
async fn catalog_playlist_occurrences(
    binding: &AppleMusicBinding,
    id: i32,
    request: &FetchPlaylistOccurrencesRequest,
) -> Result<Option<FetchPlaylistOccurrencesResponse>> {
    let after = listed_after(request)?;
    let catalog_id = catalog_playlist_id(binding, id)?;
    let collection = CatalogCollection::Playlist(id);
    let songs = catalog_songs(binding, collection, &catalog_id, after.is_none()).await?;
    let page = occurrence_page(
        after,
        songs.iter().map(|(track, _)| track.as_str()),
        request.page_size,
    )?;
    let (files, covers) = media_files(binding, song_refs(&songs[page.range.clone()]))?;
    let cover_art_map = if request.bake_cover_arts {
        binding.cover_map(covers).await?
    } else {
        HashMap::new()
    };
    Ok(Some(occurrences_response(
        request,
        page,
        files,
        cover_art_map,
    )?))
}

/// A page of the Rune playlist `stored`. Its entries keep their occurrence ids however the others
/// move, so a page starts after the entry the one before ended with. An entry whose song Apple no
/// longer has is listed too, so it can be removed.
async fn rune_occurrences(
    binding: &AppleMusicBinding,
    stored: &str,
    request: &FetchPlaylistOccurrencesRequest,
) -> Result<Option<FetchPlaylistOccurrencesResponse>> {
    if !OCCURRENCE_PAGE_SIZES.contains(&request.page_size) {
        return Err(ExpectedRequestError::invalid_input(format!(
            "Playlist page size must be {}..={}",
            OCCURRENCE_PAGE_SIZES.start(),
            OCCURRENCE_PAGE_SIZES.end()
        ))
        .into());
    }
    let playlist = binding.rune_playlists.playlist(stored).await?;
    let entries = &playlist.entries;
    let start = page_start(entries, request.after.as_ref())?;
    let end = start
        .saturating_add(usize::try_from(request.page_size)?)
        .min(entries.len());
    let (listed, covers) = listed_occurrences(binding, &entries[start..end], start).await?;
    let cover_art_map = if request.bake_cover_arts {
        binding.cover_map(covers).await?
    } else {
        HashMap::new()
    };
    Ok(Some(FetchPlaylistOccurrencesResponse {
        request_id: request.request_id.clone(),
        entries: listed,
        next: next_cursor(entries, end)?,
        cover_art_map,
        call_id: request.call_id,
    }))
}

/// The songs of the Rune playlist `stored` that Apple still has, in order.
async fn rune_playlist_songs(binding: &AppleMusicBinding, stored: &str) -> Result<Vec<Listed>> {
    let playlist = binding.rune_playlists.playlist(stored).await?;
    Ok(rune_songs(binding, &playlist.entries)
        .await
        .into_iter()
        .flatten()
        .collect())
}

/// A page of the songs of the Rune playlist `stored` that Apple still has, and that have a cover
/// when `cover_only` is set, which is what a mix query pages through. Its tracks are
/// looked up a step at a time until the page is full or the playlist ends, so a page is short
/// only at the end. The pages before it are looked up again to count the songs that are left
/// out; the binding holds what it found, and what is gone, so that costs Apple nothing the
/// second time.
async fn rune_playlist_page(
    binding: &AppleMusicBinding,
    stored: &str,
    cover_only: bool,
    cursor: usize,
    count: usize,
) -> Result<Vec<Listed>> {
    let playlist = binding.rune_playlists.playlist(stored).await?;
    let wanted = cursor.saturating_add(count);
    let mut found = Vec::new();
    for step in playlist.entries.chunks(RUNE_PAGE_STEP) {
        if found.len() >= wanted {
            break;
        }
        found.extend(
            rune_songs(binding, step)
                .await
                .into_iter()
                .flatten()
                .filter(|(_, song)| !cover_only || song_has_cover(song)),
        );
    }
    Ok(found.into_iter().skip(cursor).take(count).collect())
}

/// The collection of the library a `MixQuery` asks the songs of. A playlist is one of Rune's
/// own, which `rune_playlist_songs` lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DetailQuery {
    Artist(i32),
    Album(i32),
}

impl DetailQuery {
    /// The kind and the id of the collection.
    fn collection(self) -> (AppleIdKind, i32) {
        match self {
            Self::Album(id) => (AppleIdKind::Album, id),
            Self::Artist(id) => (AppleIdKind::Artist, id),
        }
    }
}

/// A song as a collection lists it: its track and the song.
type Listed = (String, AppleSong);

/// The songs of the albums of library artists, read from Apple album by album in the order
/// Apple lists them, each album and each song once.
async fn library_artist_songs(
    binding: &AppleMusicBinding,
    artists: &[String],
) -> Result<Vec<Listed>> {
    // Owned ids: closures taking borrowed items here fail the compiler's Send check.
    let albums = stream::iter(artists.to_vec())
        .map(|artist| async move { binding.read_artist_albums(&artist).await })
        .buffered(ALBUM_READS)
        .try_collect::<Vec<_>>()
        .await?;
    let mut seen_albums = HashSet::new();
    let albums: Vec<String> = albums
        .into_iter()
        .flatten()
        .map(|album| album.id)
        .filter(|id| seen_albums.insert(id.clone()))
        .collect();
    let songs = stream::iter(albums)
        .map(|album| async move { binding.read_album_songs(&album).await })
        .buffered(ALBUM_READS)
        .try_collect::<Vec<_>>()
        .await?;
    let mut seen = HashSet::new();
    Ok(songs
        .into_iter()
        .flatten()
        .filter(|song| seen.insert(song.id.clone()))
        .map(|song| (song.id.clone(), AppleSong::Library(song)))
        .collect())
}

/// The sort keys of a listed song: disc, number and title.
fn disc_number_title(song: &LibrarySong) -> (u32, u32, String) {
    let song = SongRef::Library(song);
    (
        song.disc_number().unwrap_or_default(),
        song.track_number().unwrap_or_default(),
        song.title().to_lowercase(),
    )
}

/// The songs of the kept song list that an artist of the library's artist list is credited
/// with, album by album, each song once. Albums go by year, then name, and an album's songs by
/// disc, number and title. Songs without an album come last.
fn artist_songs_from_list(list: &AppleSongs, artist: &NamedArtist) -> Vec<Listed> {
    let mut seen = HashSet::new();
    let mut found: Vec<(&str, &LibrarySong)> = list
        // The library's songs only: the list also holds songs of playlists that are not.
        .library_entries()
        .filter(|(track, _)| artist.credits_track(track) && seen.insert(*track))
        .collect();
    // An album is told by its id, or else by its name. Its year is its earliest song's, as songs
    // of one album may disagree.
    let album_of = |song: &LibrarySong| match song.album_id() {
        Some(id) => (Some(id.to_owned()), String::new()),
        None => (
            None,
            SongRef::Library(song)
                .album()
                .unwrap_or_default()
                .to_lowercase(),
        ),
    };
    let mut years: HashMap<(Option<String>, String), u32> = HashMap::new();
    for (_, song) in &found {
        let year = SongRef::Library(song).year().unwrap_or(u32::MAX);
        let slot = years.entry(album_of(song)).or_insert(year);
        *slot = (*slot).min(year);
    }
    found.sort_by_cached_key(|(_, song)| {
        let listed = SongRef::Library(song);
        let (disc, number, title) = disc_number_title(song);
        (
            listed.album().is_none() && song.album_id().is_none(),
            years[&album_of(song)],
            listed.album().unwrap_or_default().to_lowercase(),
            song.album_id().map(str::to_owned),
            disc,
            number,
            title,
        )
    });
    found
        .into_iter()
        .map(|(track, song)| (track.to_owned(), AppleSong::Library(song.clone())))
        .collect()
}

/// The songs of a library album in the kept song list, by disc, number and title.
fn album_songs_from_list(list: &AppleSongs, album: &str) -> Vec<Listed> {
    let mut found: Vec<(&str, &LibrarySong)> = list
        .entries()
        .filter(|(_, song)| song.album_id() == Some(album))
        .collect();
    found.sort_by_cached_key(|(_, song)| disc_number_title(song));
    found
        .into_iter()
        .map(|(track, song)| (track.to_owned(), AppleSong::Library(song.clone())))
        .collect()
}

/// The songs of an artist of the library's artist list or of a library album, taken from the
/// song list, which is kept whole and read again in the background, so these pages need no
/// reads of their own. `None` when the list cannot be had or holds none of them, as an album
/// it does not list; the collection is then read from Apple. An album Rune holds is no album of
/// Apple's, and lists what the song list holds of it, which may be nothing.
async fn song_list_songs(
    binding: &AppleMusicBinding,
    query: DetailQuery,
) -> Result<Option<Vec<Listed>>> {
    let (kind, id) = query.collection();
    let artist = match binding.named_artist_key(kind, id) {
        Some(key) => Some(binding.named_artist(&key).await?),
        None => None,
    };
    let album = match (&artist, query) {
        (None, DetailQuery::Album(_)) => Some(library_collection(binding, kind, id)?),
        _ => None,
    };
    if artist.is_none() && album.is_none() {
        return Ok(None);
    }
    let rune_album = album.as_deref().is_some_and(is_rune_album);
    let Ok(list) = binding.songs().await else {
        return Ok(rune_album.then(Vec::new));
    };
    let songs = match (&artist, &album) {
        (Some(artist), _) => artist_songs_from_list(&list, artist),
        (None, Some(album)) => album_songs_from_list(&list, album),
        (None, None) => return Ok(None),
    };
    Ok((rune_album || !songs.is_empty()).then_some(songs))
}

/// The songs of a collection of the library: from the song list where it lists them, or else
/// read from Apple. An album lists its songs by disc and number, and an artist of the library's
/// artist list the songs of every library artist that credits it.
async fn detail_songs(binding: &AppleMusicBinding, query: DetailQuery) -> Result<Vec<Listed>> {
    if let Some(songs) = song_list_songs(binding, query).await? {
        return Ok(songs);
    }
    let (kind, id) = query.collection();
    if let Some(key) = binding.named_artist_key(kind, id) {
        let artist = binding.named_artist(&key).await?;
        return library_artist_songs(binding, &artist.credits).await;
    }
    let remote = library_collection(binding, kind, id)?;
    match query {
        DetailQuery::Album(_) => Ok(binding
            .read_album_songs(&remote)
            .await?
            .into_iter()
            .map(|song| (song.id.clone(), AppleSong::Library(song)))
            .collect()),
        DetailQuery::Artist(_) => library_artist_songs(binding, &[remote]).await,
    }
}

/// The songs of a collection that a mix query names, as `detail_songs` gives them. A mix kept
/// before the library's artist list told its artists apart as it does now may name an artist
/// the list no longer holds. That query is logged and plays nothing, so the rest of the mix
/// plays and a mix of nothing else is empty. Any other failure is the caller's.
async fn mix_detail_songs(binding: &AppleMusicBinding, query: DetailQuery) -> Result<Vec<Listed>> {
    let (kind, id) = query.collection();
    if let Some(key) = binding.named_artist_key(kind, id)
        && let Err(error) = binding.named_artist(&key).await
    {
        if is_not_found(&error) {
            log::warn!("Skipping the Apple Music artist {key} of a mix, which is gone");
            return Ok(Vec::new());
        }
        return Err(error);
    }
    detail_songs(binding, query).await
}

/// The songs of a collection that a mix query names: a catalog album's or playlist's, read
/// afresh when `afresh` is set and otherwise as kept (see `catalog_collection_songs`); a library
/// album's or artist's as `mix_detail_songs` gives them; or those of a Rune playlist that Apple
/// still has.
async fn collection_songs(
    binding: &AppleMusicBinding,
    target: MixTarget,
    afresh: bool,
) -> Result<Vec<Listed>> {
    if let Some(stored) = rune_playlist_target(binding, target) {
        return rune_playlist_songs(binding, &stored).await;
    }
    if let Some((collection, catalog_id)) = catalog_collection(binding, target)? {
        return catalog_songs(binding, collection, &catalog_id, afresh).await;
    }
    let query = match target {
        MixTarget::Album(id) => DetailQuery::Album(id),
        MixTarget::Artist(id) => DetailQuery::Artist(id),
        // A playlist of the catalog is named by its catalog id.
        MixTarget::Playlist(_) => return Err(not_apple_collection()),
    };
    mix_detail_songs(binding, query).await
}

/// Whether a song has a cover.
fn song_has_cover(song: &AppleSong) -> bool {
    song.as_ref().artwork().is_some()
}

pub(crate) async fn mix_query_request(
    binding: &AppleMusicBinding,
    request: &MixQueryRequest,
) -> Result<Option<MixQueryResponse>> {
    let planned = mixes::plan(&request.queries, MixQueryRequest::wire_name())?;
    let cursor = usize::try_from(request.cursor)
        .map_err(|error| ExpectedRequestError::invalid_input(format!("Invalid cursor: {error}")))?;
    let count = usize::try_from(request.page_size).map_err(|error| {
        ExpectedRequestError::invalid_input(format!("Invalid page size: {error}"))
    })?;
    // Songs without a cover are dropped before the page is cut, so a page is full of covers.
    // The cover wall draws at random and shows only covers, which it reads once.
    let cover_wall = planned.cover_only && matches!(planned.plan, Plan::Random(_));
    let mut page: Vec<Listed> = match planned.plan {
        Plan::Collection(target) => match rune_playlist_target(binding, target) {
            Some(stored) => {
                rune_playlist_page(binding, &stored, planned.cover_only, cursor, count).await?
            }
            // A catalog album or playlist is read afresh for the first page.
            None => collection_songs(binding, target, cursor == 0)
                .await?
                .into_iter()
                .filter(|(_, song)| !planned.cover_only || song_has_cover(song))
                .skip(cursor)
                .take(count)
                .collect(),
        },
        Plan::All => binding
            .songs()
            .await?
            .entries()
            .filter(|(_, song)| !planned.cover_only || has_cover(song))
            .skip(cursor)
            .take(count)
            .map(|(track, song)| (track.to_owned(), AppleSong::Library(song.clone())))
            .collect(),
        Plan::Random(wanted) => {
            let list = binding.songs().await?;
            let pool: Vec<(&str, &LibrarySong)> = list
                .entries()
                .filter(|(_, song)| !planned.cover_only || has_cover(song))
                .collect();
            // Every query draws afresh, as for a local library, so the page cuts this draw.
            // A query that names a seed draws the same each time, so its pages agree. Only the
            // songs picked are cloned.
            let picked = match request.random_seed {
                Some(seed) => {
                    mixes::sample_indices(&mut StdRng::seed_from_u64(seed), pool.len(), wanted)
                }
                None => mixes::sample_indices(&mut rand::thread_rng(), pool.len(), wanted),
            };
            picked
                .into_iter()
                .skip(cursor)
                .take(count)
                .map(|index| {
                    let (track, song) = pool[index];
                    (track.to_owned(), AppleSong::Library(song.clone()))
                })
                .collect()
        }
        // Items together are read whole, to join them, and cut after.
        Plan::Items(items) => items_songs(binding, &items, cursor == 0)
            .await?
            .into_iter()
            .filter(|(_, song)| !planned.cover_only || song_has_cover(song))
            .skip(cursor)
            .take(count)
            .collect(),
    };
    // The wall's covers must be on disk before it gets the tracks. A song whose cover cannot be
    // had is left out. Track lists show a cover whenever its download lands.
    if cover_wall {
        let cached = binding
            .cache_covers(
                page.iter()
                    .filter_map(|(_, song)| cover_url(song.as_ref().artwork()))
                    .collect(),
            )
            .await;
        page.retain(|(_, song)| {
            cover_url(song.as_ref().artwork()).is_some_and(|url| cached.contains(&url))
        });
    }
    let (mut files, covers) = media_files(binding, song_refs(&page))?;
    let cover_art_map = binding.cover_map(covers).await?;
    // A batch larger than the cache can evict a cover cached above before the map is built.
    // The wall reads its covers once, so a song whose cover is gone is left out.
    if cover_wall {
        files.retain(|file| cover_art_map.contains_key(&file.id));
    }
    Ok(Some(MixQueryResponse {
        call_id: None,
        files,
        cover_art_map,
    }))
}

/// The songs of single tracks and collections together, as `mixes::union_order` lists them:
/// each collection's as its own page lists them, a catalog album or playlist read afresh when
/// `afresh` is set, and each song once. An id the binding never gave names no song, as for a
/// local library, and a song Apple no longer has is left out, as is an artist the library's
/// artist list no longer holds (`mix_detail_songs`).
async fn items_songs(
    binding: &AppleMusicBinding,
    items: &[MixItem],
    afresh: bool,
) -> Result<Vec<Listed>> {
    // The single tracks the binding lacks are read together first.
    let tracks: Vec<String> = items
        .iter()
        .filter_map(|item| match item {
            MixItem::Track(id) => binding.resolve_track(*id).ok(),
            MixItem::Collection(_) => None,
        })
        .collect();
    binding.read_unknown_tracks(tracks).await;
    let mut songs = Vec::new();
    for item in mixes::union_order(items) {
        match item {
            MixItem::Track(id) => {
                if let Ok(track) = binding.resolve_track(id)
                    && let Some(song) = apple_song(binding, &track).await?
                {
                    songs.push((track, song));
                }
            }
            MixItem::Collection(target) => {
                songs.extend(collection_songs(binding, target, afresh).await?);
            }
        }
    }
    Ok(mixes::first_of_each_key(songs, |(track, _)| track.clone()))
}

impl MixSource for AppleMusicBinding {
    fn library_id(&self) -> String {
        ServiceBinding::library_id(self).to_string()
    }

    fn collection_tracks(&self, target: MixTarget) -> ServiceFuture<'_, Vec<String>> {
        Box::pin(async move {
            // A Rune playlist names the track each entry plays, without a read.
            if let Some(stored) = rune_playlist_target(self, target) {
                let playlist = self.rune_playlists.playlist(&stored).await?;
                return Ok(playlist
                    .entries
                    .iter()
                    .map(|entry| playing_track(self, entry))
                    .collect());
            }
            let songs = collection_songs(self, target, false).await?;
            Ok(songs.into_iter().map(|(track, _)| track).collect())
        })
    }

    fn all_tracks(&self, limit: usize) -> ServiceFuture<'_, Vec<String>> {
        Box::pin(async move {
            let list = self.current_songs().await?;
            Ok(list
                .entries()
                .take(limit)
                .map(|(track, _)| track.to_owned())
                .collect())
        })
    }

    fn track_key(&self, id: i32) -> Result<Option<String>> {
        Ok(self.resolve_track(id).ok())
    }
}

pub(crate) async fn resolve_mix_query_request(
    binding: &AppleMusicBinding,
    request: &ResolveMixQueryRequest,
) -> Result<Option<ResolveMixQueryResponse>> {
    Ok(Some(mixes::resolve_mix_query(binding, request).await?))
}

fn home_track(track: &str, song: &LibrarySong) -> HomeTrack {
    let song = SongRef::Library(song);
    HomeTrack {
        remote: track.to_owned(),
        name: song.title().to_owned(),
        cover: cover_url(song.artwork()),
    }
}

impl HomeSource for AppleMusicBinding {
    fn collections(&self, collection_type: CollectionType) -> ServiceFuture<'_, Vec<HomeItem>> {
        Box::pin(async move {
            let kind = id_kind(collection_type, ComplexQueryRequest::wire_name())?;
            let mut items: Vec<HomeItem> = entries(self, kind, ComplexQueryRequest::wire_name())
                .await?
                .into_iter()
                .map(|entry| HomeItem {
                    id: entry.id,
                    name: entry.name,
                    cover: entry.cover,
                })
                .collect();
            // Apple lists the albums it added last first.
            if kind == AppleIdKind::Album {
                items.reverse();
            }
            Ok(items)
        })
    }

    /// Rune's own playlists and mixes can be renamed and removed here; nothing of Apple's can,
    /// which `readonly` tells apart.
    fn editable(&self, collection_type: CollectionType) -> bool {
        matches!(
            collection_type,
            CollectionType::Playlist | CollectionType::Mix
        )
    }

    fn readonly(&self, collection_type: CollectionType, id: i32) -> bool {
        match collection_type {
            CollectionType::Playlist => !matches!(self.playlist_ref(id), PlaylistRef::Rune(_)),
            CollectionType::Mix => !matches!(self.mix_ref(id), MixRef::Rune(_)),
            _ => true,
        }
    }

    fn tracks(&self, _limit: usize) -> ServiceFuture<'_, Vec<HomeTrack>> {
        Box::pin(async move {
            let list = self.songs().await?;
            // The library's own songs, in the order Apple lists them, which is the order they
            // were added in; the songs only a playlist holds come after them and are left out.
            Ok(list
                .entries()
                .filter(|(_, song)| is_library_song(song))
                .map(|(track, song)| home_track(track, song))
                .collect())
        })
    }

    /// The library's own songs, the ones added last first.
    fn newest_tracks(&self, limit: usize) -> ServiceFuture<'_, Vec<HomeTrack>> {
        Box::pin(async move {
            let list = self.songs().await?;
            Ok(list
                .entries()
                .rev()
                .filter(|(_, song)| is_library_song(song))
                .take(limit)
                .map(|(track, song)| home_track(track, song))
                .collect())
        })
    }

    /// The songs liked in Rune, the one liked last first, as Rune's likes keep them: Apple is
    /// not asked for ratings. A song is named by the library's copy of it when the library has
    /// one, and comes once however many keys it was liked under. Each is named by the songs the
    /// binding knows, which are the library's and what was read for it lately; one it cannot
    /// name is left out.
    fn liked(&self) -> ServiceFuture<'_, Vec<HomeTrack>> {
        Box::pin(async move {
            // The song list names the songs, and tells which catalog song a library copy is.
            let _ = self.songs().await?;
            let mut found = Vec::new();
            let mut seen = HashSet::new();
            for track in self.rune_likes.newest(usize::MAX).await? {
                let track = self.library_copy_of(&track);
                if !seen.insert(track.clone())
                    || !self.rune_likes.is_liked(&self.like_keys(&track)).await?
                {
                    continue;
                }
                let Some(song) = self.known_song(&track) else {
                    continue;
                };
                found.push(HomeTrack {
                    name: song.as_ref().title().to_owned(),
                    cover: cover_url(song.as_ref().artwork()),
                    remote: track,
                });
                if found.len() >= home::GROUP_LIMIT {
                    break;
                }
            }
            Ok(found)
        })
    }

    fn mixes(&self) -> ServiceFuture<'_, Vec<HomeMix>> {
        Box::pin(mix_collections::home_mixes(self))
    }

    fn number(&self, tracks: Vec<HomeTrack>) -> Result<Vec<HomeItem>> {
        tracks
            .into_iter()
            .map(|track| {
                Ok(HomeItem {
                    id: self.intern_track(&track.remote)?,
                    name: track.name,
                    cover: track.cover,
                })
            })
            .collect()
    }

    fn cover_paths(&self, covers: Vec<(i32, String)>) -> ServiceFuture<'_, HashMap<i32, String>> {
        Box::pin(self.cover_map(covers))
    }

    fn random_seed(&self) -> u64 {
        home::rotating_seed(self.home_seed)
    }

    fn holds_sign_in(&self) -> bool {
        self.is_signed_in()
    }
}

pub(crate) async fn complex_query_request(
    binding: &AppleMusicBinding,
    request: &ComplexQueryRequest,
) -> Result<Option<ComplexQueryResponse>> {
    Ok(Some(home::complex_query(binding, request).await?))
}

/// Whether an error says the thing asked for does not exist.
pub(crate) fn is_not_found(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<ExpectedError>(),
        Some(ExpectedError::NotFound { .. })
    )
}

/// The song of `track`, unless Apple said it does not have it and nothing listed it since. A
/// song Apple answers it does not have is remembered as missing.
async fn apple_song(binding: &AppleMusicBinding, track: &str) -> Result<Option<AppleSong>> {
    if gone(binding, track) {
        return Ok(None);
    }
    let generation = binding.generation();
    match binding.song(track).await {
        Ok(song) => Ok(Some(song)),
        Err(error) if is_not_found(&error) => {
            binding.remember_missing(generation, track);
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// The track a request about one item names, which must be one of this library's.
fn item_track(binding: &AppleMusicBinding, item: &Option<PlayingItemRef>) -> Result<String> {
    match item {
        Some(PlayingItemRef::LibraryTrack { library_id, track })
            if *library_id == ServiceBinding::library_id(binding).to_string() =>
        {
            song_of_track(track).map(|_| track.clone()).ok_or_else(|| {
                ExpectedRequestError::invalid_input("Not an Apple Music track").into()
            })
        }
        Some(PlayingItemRef::LibraryTrack { .. }) => {
            Err(ExpectedRequestError::invalid_input("Item belongs to another library").into())
        }
        _ => {
            Err(ExpectedRequestError::invalid_input("Expected an Apple Music library track").into())
        }
    }
}

/// Likes or unlikes a song in Rune's likes on this device. Nothing is sent to Apple: what Rune
/// changes stays in Rune. A like belongs to the song: when the library copy and the catalog song
/// are known to be one song, both are liked or unliked, and the lists that show the likes reload.
pub(crate) async fn set_liked_request(
    binding: &AppleMusicBinding,
    request: &SetLikedRequest,
) -> Result<Option<SetLikedResponse>> {
    let track = item_track(binding, &request.item)?;
    binding
        .rune_likes
        .set_liked(&binding.like_keys(&track), request.liked)
        .await?;
    binding.content_changed();
    Ok(request.item.clone().map(|item| SetLikedResponse {
        item,
        liked: request.liked,
        call_id: request.call_id,
    }))
}

/// Answers whether a song is liked from Rune's likes on this device, without asking Apple. The
/// library copy of a song and its catalog song are liked together.
pub(crate) async fn get_liked_request(
    binding: &AppleMusicBinding,
    request: &GetLikedRequest,
) -> Result<Option<GetLikedResponse>> {
    let track = item_track(binding, &request.item)?;
    let liked = binding
        .rune_likes
        .is_liked(&binding.like_keys(&track))
        .await?;
    Ok(request.item.clone().map(|item| GetLikedResponse {
        item,
        liked,
        call_id: request.call_id,
    }))
}

/// Answers the lyrics of a song from LRCLIB, by the song's title, credit, album and length.
/// Apple gives Rune no lyrics. A song without lyrics, or whose lyrics could not be looked up,
/// has no lines, as a song of a local library without lyrics has none. A library the user has
/// not allowed to look lyrics up has none for its songs, as its item slots say.
pub(crate) async fn get_lyric_by_track_id_request(
    binding: &AppleMusicBinding,
    request: &GetLyricByTrackIdRequest,
) -> Result<Option<GetLyricByTrackIdResponse>> {
    if !binding.looks_up_lyrics() {
        return Err(ExpectedRequestError::Unsupported {
            request: GetLyricByTrackIdRequest::wire_name(),
            reason: UnsupportedReason::NotImplemented,
        }
        .into());
    }
    let track = item_track(binding, &request.item)?;
    let lines = match binding.song(&track).await {
        Ok(song) => binding
            .lyrics_of(&track, song.as_ref())
            .await
            .map(|lyrics| lrclib::lines(&lyrics))
            .unwrap_or_default(),
        Err(error) => {
            log::warn!("Lyrics were not looked up, as the song could not be read: {error:#}");
            Vec::new()
        }
    };
    Ok(request.item.clone().map(|item| GetLyricByTrackIdResponse {
        item,
        lines,
        call_id: request.call_id,
    }))
}

macro_rules! apple_music_requests {
    (
        supported { $($request:ty => $handler:ident as $method:ident),* $(,)? }
        routed_native { $($native_request:ty),* $(,)? }
        items { $($item_request:ty => $item_slot:ident),* $(,)? }
    ) => {
        $(const _: () = assert!(
            <$item_request as crate::rpc::TargetRequest>::ROUTED_BY_ITEM,
            "item slots are only for requests routed by item",
        );)*

        impl RequestTranslator for AppleMusicBinding {
            $(
                fn $method<'a>(
                    &'a self,
                    request: &'a $request,
                ) -> RequestTranslationFuture<'a, <$request as crate::TypedRequest>::Response> {
                    Box::pin($handler(self, request))
                }
            )*
        }

        impl RequestSlots {
            /// What an open Apple Music library answers itself, and what each device item
            /// decides for itself; everything else is unsupported.
            pub(crate) fn apple_music() -> Self {
                let mut slots = Self::unimplemented();
                $(slots.set::<$request>(Slot::Translate);)*
                $(slots.set::<$native_request>(Slot::Native);)*
                slots
            }

            /// What an Apple Music library answers about its own items, of the requests
            /// routed by item, whichever library is open and whether or not it is signed in;
            /// the others are unsupported for its items.
            pub(crate) fn apple_music_items() -> Self {
                let mut slots = Self::unimplemented();
                $(slots.set::<$item_request>(Slot::$item_slot);)*
                slots
            }
        }
    };
}

apple_music_requests! {
    supported {
        FetchMediaFilesRequest => fetch_media_files_request as translate_fetch_media_files_request,
        FetchMediaFileByIdsRequest => fetch_media_file_by_ids_request as translate_fetch_media_file_by_ids_request,
        FetchParsedMediaFileRequest => fetch_parsed_media_file_request as translate_fetch_parsed_media_file_request,
        SearchMediaFileSummaryRequest => search_media_file_summary_request as translate_search_media_file_summary_request,
        GetMediaFilesCountRequest => get_media_files_count_request as translate_get_media_files_count_request,
        FetchCollectionGroupSummaryRequest => fetch_collection_group_summary_request as translate_fetch_collection_group_summary_request,
        FetchCollectionGroupsRequest => fetch_collection_groups_request as translate_fetch_collection_groups_request,
        FetchCollectionByIdsRequest => fetch_collection_by_ids_request as translate_fetch_collection_by_ids_request,
        SearchCollectionSummaryRequest => search_collection_summary_request as translate_search_collection_summary_request,
        MixQueryRequest => mix_query_request as translate_mix_query_request,
        ResolveMixQueryRequest => resolve_mix_query_request as translate_resolve_mix_query_request,
        ComplexQueryRequest => complex_query_request as translate_complex_query_request,
        FetchAllPlaylistsRequest => fetch_all_playlists_request as translate_fetch_all_playlists_request,
        GetPlaylistByIdRequest => get_playlist_by_id_request as translate_get_playlist_by_id_request,
        CreatePlaylistRequest => create_playlist_request as translate_create_playlist_request,
        UpdatePlaylistRequest => update_playlist_request as translate_update_playlist_request,
        RemovePlaylistRequest => remove_playlist_request as translate_remove_playlist_request,
        AddItemToPlaylistRequest => add_item_to_playlist_request as translate_add_item_to_playlist_request,
        ReorderPlaylistItemPositionRequest => reorder_playlist_item_position_request as translate_reorder_playlist_item_position_request,
        RemoveItemFromPlaylistRequest => remove_item_from_playlist_request as translate_remove_item_from_playlist_request,
        FetchPlaylistOccurrencesRequest => fetch_playlist_occurrences_request as translate_fetch_playlist_occurrences_request,
        FetchAllMixesRequest => fetch_all_mixes_request as translate_fetch_all_mixes_request,
        GetMixByIdRequest => get_mix_by_id_request as translate_get_mix_by_id_request,
        FetchMixQueriesRequest => fetch_mix_queries_request as translate_fetch_mix_queries_request,
        CreateMixRequest => create_mix_request as translate_create_mix_request,
        UpdateMixRequest => update_mix_request as translate_update_mix_request,
        RemoveMixRequest => remove_mix_request as translate_remove_mix_request,
        AddItemToMixRequest => add_item_to_mix_request as translate_add_item_to_mix_request,
        SetLikedRequest => set_liked_request as translate_set_liked_request,
        GetLikedRequest => get_liked_request as translate_get_liked_request,
        GetLyricByTrackIdRequest => get_lyric_by_track_id_request as translate_get_lyric_by_track_id_request,
        SearchForRequest => search_for_request as translate_search_for_request,
    }
    // This device answers these by the library of the item each names, whichever library is
    // open: the items of another library are that library's to answer.
    routed_native {
        SetLikedRequest,
        GetLikedRequest,
        GetLyricByTrackIdRequest,
        GetPrimaryColorByTrackIdRequest,
    }
    items {
        // Likes are kept in Rune, on this device, so an item is liked whichever library is open.
        SetLikedRequest => Translate,
        GetLikedRequest => Translate,
        // Apple gives Rune no lyrics, so the library looks them up at LRCLIB, once the user
        // allows it (see `service::item_slots`).
        GetLyricByTrackIdRequest => Translate,
        // The hub takes the color of a streaming item from its cover in the library's cover cache.
        GetPrimaryColorByTrackIdRequest => Native,
    }
}

#[cfg(test)]
mod tests {
    use apple_music::models::LibrarySong;
    use serde_json::{Value, json};

    use super::{album_songs_from_list, artist_songs_from_list};
    use crate::apple_music_artists::tests::catalog;
    use crate::apple_music_artists::{NamedArtists, SongArtists};
    use crate::apple_music_binding::{AppleSongs, catalog_track};

    /// A library song of the library artist credited `artist`, on the album `(id, year)`.
    fn song(
        id: &str,
        title: &str,
        artist: &str,
        album: Option<(&str, u32)>,
        at: u32,
        catalog_id: Option<&str>,
    ) -> LibrarySong {
        let mut attributes = json!({
            "name": title,
            "artistName": artist,
            "trackNumber": at,
            "discNumber": 1,
            "playParams": {"id": id, "kind": "song"},
        });
        if let Some(catalog_id) = catalog_id {
            attributes["playParams"]["catalogId"] = Value::from(catalog_id);
        }
        let mut song = json!({"id": id, "type": "library-songs", "attributes": attributes});
        if let Some((album, year)) = album {
            song["attributes"]["albumName"] = Value::from(format!("Album {album}"));
            song["attributes"]["releaseDate"] = Value::from(format!("{year}-01-01"));
            song["relationships"] =
                json!({"albums": {"data": [{"id": album, "type": "library-albums"}]}});
        }
        serde_json::from_value(song).unwrap()
    }

    fn library(id: &str, name: &str) -> apple_music::models::LibraryArtist {
        serde_json::from_value(
            json!({"id": id, "type": "library-artists", "attributes": {"name": name}}),
        )
        .unwrap()
    }

    fn list(songs: Vec<LibrarySong>) -> AppleSongs {
        serde_json::from_value(json!({"songs": songs, "twins": {}})).unwrap()
    }

    #[test]
    fn an_artists_songs_come_from_the_song_list_by_album_year_then_name_and_number() {
        let songs = vec![
            song("i.1", "Late", "Ann", Some(("l.b", 2020)), 1, None),
            song("i.2", "Loose", "ANN", None, 1, None),
            // The songs of two artists are those Apple's catalog names both artists for.
            song(
                "i.3",
                "Second",
                "Ann & Bob",
                Some(("l.a", 2020)),
                2,
                Some("c.3"),
            ),
            song(
                "i.4",
                "First",
                "Bob, Ann",
                Some(("l.a", 2020)),
                1,
                Some("c.4"),
            ),
            song("i.5", "Early", "Ann", Some(("l.c", 2010)), 1, None),
            song("i.6", "Not Ann", "Bob", Some(("l.a", 2020)), 3, None),
            // The same song listed again is shown once.
            song("i.1", "Late", "Ann", Some(("l.b", 2020)), 1, None),
        ];
        let mut song_artists = SongArtists::default();
        song_artists.add(
            &["c.3".to_owned(), "c.4".to_owned()],
            vec![
                ("c.3".into(), vec![catalog("1", "Ann"), catalog("2", "Bob")]),
                ("c.4".into(), vec![catalog("2", "Bob"), catalog("1", "Ann")]),
            ],
        );
        let named = NamedArtists::new(
            &[
                library("r.1", "Ann"),
                library("r.2", "Ann & Bob"),
                library("r.3", "Bob"),
            ],
            &songs,
            &song_artists,
        );
        let list = list(songs);
        let ann = named.get("artist:1").unwrap();
        let titles = |found: Vec<(String, crate::apple_music_binding::AppleSong)>| {
            found
                .into_iter()
                .map(|(_, song)| song.as_ref().title().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            titles(artist_songs_from_list(&list, ann)),
            ["Early", "First", "Second", "Late", "Loose"]
        );
        assert_eq!(
            titles(album_songs_from_list(&list, "l.a")),
            ["First", "Second", "Not Ann"]
        );
    }

    #[test]
    fn albums_that_name_no_album_id_go_by_their_own_years() {
        let unrelated = |id, title, album| {
            let mut song = song(id, title, "Ann", Some(album), 1, None);
            song.relationships = None;
            song
        };
        let songs = vec![
            unrelated("i.1", "Newer", ("Alpha", 2020)),
            unrelated("i.2", "Older", ("Zulu", 2000)),
            song("i.3", "Loose", "Ann", None, 1, None),
        ];
        let named = NamedArtists::new(&[library("r.1", "Ann")], &songs, &SongArtists::default());
        let ann = named.iter().find(|artist| artist.name == "Ann").unwrap();
        let titles: Vec<String> = artist_songs_from_list(&list(songs.clone()), ann)
            .into_iter()
            .map(|(_, song)| song.as_ref().title().to_owned())
            .collect();
        assert_eq!(titles, ["Older", "Newer", "Loose"]);
    }

    #[test]
    fn a_catalog_track_is_listed_once_as_the_library_copy_it_has() {
        let copy = song("i.copy", "Copy", "Ann", None, 1, Some("900"));
        let mut playlist_entry = song("p.900", "Copy", "Ann", None, 1, Some("900"));
        playlist_entry.kind = "songs".to_owned();
        let list: AppleSongs = serde_json::from_value(json!({
            "songs": [copy, playlist_entry],
            "twins": {"900": "i.copy"},
        }))
        .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list.library_twin(&catalog_track("900")), Some("i.copy"));
    }
}
