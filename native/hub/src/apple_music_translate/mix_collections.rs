//! The mixes of an open Apple Music library: Rune's own, kept on this device by Apple's keys of
//! what they play, and the mixes of the albums and playlists Apple recommends, which are locked.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use apple_music::models::{
    Artwork, LibraryAlbum, Recommendation, RecommendationItemAttributes, Resource,
};
use database::actions::mixes::QueryOperator;
use database::error::ExpectedError;
use database::mix_operator::{MixOperator, MixTarget as StoredTarget};

use super::{CATALOG_PREFIX, album_artwork};
use crate::TypedRequest;
use crate::apple_music_artists::NamedArtists;
use crate::apple_music_binding::{
    AppleIdKind, AppleMusicBinding, AppleSongs, MixGroups, MixRef, PlaylistRef, RecommendedMix,
    SongRef, cover_url,
};
use crate::messages::{
    AddItemToMixRequest, AddItemToMixResponse, Collection, CollectionGroup, CollectionGroupSummary,
    CollectionGroupSummaryResponse, CollectionType, CreateMixRequest, CreateMixResponse,
    FetchAllMixesRequest, FetchAllMixesResponse, FetchCollectionByIdsResponse,
    FetchCollectionGroupsResponse, FetchMixQueriesRequest, FetchMixQueriesResponse,
    GetMixByIdRequest, GetMixByIdResponse, Mix, MixQuery, RemoveMixRequest, RemoveMixResponse,
    StoredMixQuery, UpdateMixRequest, UpdateMixResponse,
};
use crate::service::{
    home::{HomeMix, denies_the_account},
    mixes::{self, MixTarget, unsupported_query},
    rune_mixes::{StoredMix, StoredQuery},
};

// The mixes Apple recommends ----------------------------------------------------------------

/// What a recommended mix's id names before the id of the album or playlist it plays.
const MIX_OF_ALBUM: &str = "album";
const MIX_OF_PLAYLIST: &str = "playlist";

/// Apple's kind of the recommendation groups of recent plays.
const RECENTLY_PLAYED_KIND: &str = "recently-played";

/// The title of the recommendation group of recent plays, whatever Apple titled them. The app
/// shows a reserved group of this title in its own language (see
/// `lib/screens/collection/utils/group_title.dart`).
pub(crate) const RECENTLY_PLAYED_GROUP: &str = "recently-played";

/// The title of a recommendation group Apple gave no title. The app shows a reserved group of
/// this title in its own language, as it does `RECENTLY_PLAYED_GROUP`.
pub(crate) const UNTITLED_GROUP: &str = "untitled";

fn mix_not_found() -> anyhow::Error {
    ExpectedError::NotFound { what: "mix" }.into()
}

/// The title of a recommendation group: Apple's, except that every group of recent plays is
/// titled `RECENTLY_PLAYED_GROUP`, so they are one group, and a group Apple gave no title is
/// titled `UNTITLED_GROUP`. Apple's kind of a group tells which it is, never its title.
fn group_title(recommendation: &Recommendation) -> String {
    let attributes = recommendation.attributes.as_ref();
    if attributes.and_then(|attributes| attributes.kind.as_deref()) == Some(RECENTLY_PLAYED_KIND) {
        return RECENTLY_PLAYED_GROUP.to_owned();
    }
    let title = attributes
        .and_then(|attributes| attributes.title.as_ref())
        .and_then(|title| title.string_for_display.as_deref())
        .unwrap_or_default();
    if title.trim().is_empty() {
        UNTITLED_GROUP.to_owned()
    } else {
        title.to_owned()
    }
}

/// The album or playlist a recommended mix plays, which its id names.
fn mix_target(binding: &AppleMusicBinding, id: i32) -> Result<MixTarget> {
    let remote = binding.resolve(AppleIdKind::Mix, id)?;
    match remote.split_once(':') {
        Some((MIX_OF_ALBUM, album)) => {
            Ok(MixTarget::Album(binding.intern(AppleIdKind::Album, album)?))
        }
        Some((MIX_OF_PLAYLIST, playlist)) => Ok(MixTarget::Playlist(
            binding.intern(AppleIdKind::Playlist, playlist)?,
        )),
        _ => Err(mix_not_found()),
    }
}

/// The query that lists the songs of a recommended mix's album or playlist, as their own pages
/// ask.
fn mix_query(target: MixTarget) -> MixQuery {
    let (operator, id) = match target {
        MixTarget::Album(id) => ("lib::album", id),
        MixTarget::Artist(id) => ("lib::artist", id),
        MixTarget::Playlist(id) => ("lib::playlist", id),
    };
    MixQuery {
        operator: operator.to_owned(),
        parameter: id.to_string(),
    }
}

/// A recommended album or playlist, which makes a mix.
struct RecommendedItem<'a> {
    /// What the mix's id names before the id of what it plays.
    of: &'static str,
    kind: AppleIdKind,
    /// Its catalog id.
    id: &'a str,
    name: &'a str,
    artwork: Option<&'a Artwork>,
}

/// The recommended item that makes a mix; None for a station or anything else, and for an item
/// without a name.
fn recommended_item(item: &Resource<RecommendationItemAttributes>) -> Option<RecommendedItem<'_>> {
    let (of, kind) = match item.kind.as_str() {
        "albums" => (MIX_OF_ALBUM, AppleIdKind::Album),
        "playlists" => (MIX_OF_PLAYLIST, AppleIdKind::Playlist),
        _ => return None,
    };
    let attributes = item.attributes.as_ref()?;
    Some(RecommendedItem {
        of,
        kind,
        id: &item.id,
        name: attributes.name.as_deref().filter(|name| !name.is_empty())?,
        artwork: attributes.artwork.as_ref(),
    })
}

/// The groups of `recommendations`, read under `generation`, in Apple's order, each with the
/// mixes of the albums and playlists it recommends in Apple's order, and those mixes by id.
/// Groups of the same title are one (see `group_title`), a mix comes once in a group, and a group
/// left without a mix goes. A mix recommended in several groups is listed in each, and keeps the
/// first group's title. The albums and playlists are remembered, so they open by the ids their
/// mixes' queries name.
fn recommended_groups(
    binding: &AppleMusicBinding,
    generation: u64,
    recommendations: &[Recommendation],
) -> Result<(MixGroups, HashMap<i32, RecommendedMix>)> {
    let mut groups: MixGroups = Vec::new();
    let mut mixes = HashMap::new();
    for recommendation in recommendations {
        let title = group_title(recommendation);
        let index = match groups.iter().position(|(known, _)| *known == title) {
            Some(index) => index,
            None => {
                groups.push((title, Vec::new()));
                groups.len() - 1
            }
        };
        let (title, ids) = &mut groups[index];
        let items = recommendation
            .relationships
            .iter()
            .filter_map(|relationships| relationships.contents.as_ref())
            .flat_map(|contents| &contents.data);
        for item in items.filter_map(recommended_item) {
            let collection = format!("{CATALOG_PREFIX}{}", item.id);
            let collection_id = binding.intern(item.kind, &collection)?;
            binding.remember_collection(
                generation,
                item.kind,
                collection_id,
                item.name,
                item.artwork,
            );
            let id = binding.intern(AppleIdKind::Mix, &format!("{}:{collection}", item.of))?;
            mixes.entry(id).or_insert_with(|| RecommendedMix {
                name: item.name.to_owned(),
                group: title.clone(),
                cover: cover_url(item.artwork),
            });
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    groups.retain(|(_, ids)| !ids.is_empty());
    Ok((groups, mixes))
}

/// The recommendation groups built from what Apple recommends, as kept or else read, and kept
/// with their mixes under this sign-in. They are built once for each read, unless `rebuild`
/// asks for them to be built again, as for a mix that the last build did not keep.
async fn recommendation_groups(
    binding: &AppleMusicBinding,
    rebuild: bool,
) -> Result<Arc<MixGroups>> {
    let generation = binding.generation();
    let recommendations = binding.recommendations().await?;
    if !rebuild && let Some(groups) = binding.built_mix_groups(&recommendations) {
        return Ok(groups);
    }
    let (groups, mixes) = recommended_groups(binding, generation, &recommendations)?;
    Ok(binding.keep_recommendations(generation, &recommendations, groups, mixes))
}

/// A recommended mix by id: as a build under this sign-in listed it, or else as what Apple
/// recommends lists it now. None for one it does not list.
async fn known_mix(binding: &AppleMusicBinding, id: i32) -> Result<Option<RecommendedMix>> {
    if let Some(mix) = binding.recommended_mix(id) {
        return Ok(Some(mix));
    }
    recommendation_groups(binding, true).await?;
    Ok(binding.recommended_mix(id))
}

/// A recommended mix as the mixes are listed: one query naming what it plays, locked.
async fn mix_collection(
    binding: &AppleMusicBinding,
    id: i32,
    mix: &RecommendedMix,
) -> Result<Collection> {
    let covers = mix
        .cover
        .clone()
        .map(|url| vec![(id, url)])
        .unwrap_or_default();
    Ok(Collection {
        id,
        name: mix.name.clone(),
        queries: vec![mix_query(mix_target(binding, id)?)],
        collection_type: CollectionType::Mix,
        cover_art_map: binding.cover_map(covers).await?,
        readonly: true,
    })
}

/// What a read of what Apple recommends gives beside Rune's own mixes: when there are some of
/// those, a read that fails gives none, so Rune's still show, unless it says the account may not
/// read the library, which a binding that holds its sign-in must hear of.
fn beside_own<T: Default>(read: Result<T>, own: bool, signed_in: bool) -> Result<T> {
    match read {
        Err(error) if own && !(signed_in && denies_the_account(&error)) => {
            log::warn!("Could not read the recommended mixes: {error:#}");
            Ok(T::default())
        }
        read => read,
    }
}

/// The recommendation groups, as `recommendation_groups` builds them, beside Rune's own mixes
/// when `own` says there are some.
async fn recommendations_beside_own(
    binding: &AppleMusicBinding,
    own: bool,
) -> Result<Arc<MixGroups>> {
    beside_own(
        recommendation_groups(binding, false).await,
        own,
        binding.is_signed_in(),
    )
}

/// Counts `count` more mixes in the group `title` of `summary`, which takes the group the first
/// time. A group is reserved once a recommendation group of its title is.
fn count_in(summary: &mut Vec<CollectionGroupSummary>, title: &str, count: usize, reserved: bool) {
    let count = i32::try_from(count).unwrap_or(i32::MAX);
    match summary.iter_mut().find(|known| known.group_title == title) {
        Some(known) => {
            known.count = known.count.saturating_add(count);
            known.reserved |= reserved;
        }
        None => summary.push(CollectionGroupSummary {
            group_title: title.to_owned(),
            count,
            reserved,
        }),
    }
}

/// The groups of the Mixes page: those of Rune's own mixes in the order they were first used,
/// then the recommendation groups, which the library fills itself. A title of more than one is
/// one group.
pub(super) async fn mix_group_summary(
    binding: &AppleMusicBinding,
) -> Result<CollectionGroupSummaryResponse> {
    let own = binding.rune_mixes.listed().await?;
    let groups = recommendations_beside_own(binding, !own.is_empty()).await?;
    let mut summary = Vec::new();
    for mix in own.iter() {
        count_in(&mut summary, &mix.group, 1, false);
    }
    for (title, ids) in groups.iter() {
        count_in(&mut summary, title, ids.len(), true);
    }
    Ok(CollectionGroupSummaryResponse {
        call_id: None,
        collection_type: CollectionType::Mix,
        groups: summary,
    })
}

/// The mixes of the groups `titles` names: Rune's own by name, then the recommended ones as
/// last built. A title they do not have is an empty group.
pub(super) async fn mix_collection_groups(
    binding: &AppleMusicBinding,
    titles: &[String],
) -> Result<FetchCollectionGroupsResponse> {
    let own = binding.rune_mixes.listed().await?;
    let groups = recommendations_beside_own(binding, !own.is_empty()).await?;
    let kept = KeptCovers::read(binding).await;
    let mut answered = Vec::with_capacity(titles.len());
    for title in titles {
        let mut members: Vec<&StoredMix> = own.iter().filter(|mix| &mix.group == title).collect();
        members.sort_by(|left, right| left.name.cmp(&right.name));
        let mut collections = Vec::with_capacity(members.len());
        for mix in members {
            collections.push(rune_mix_collection(binding, &kept, mix).await?);
        }
        let ids = groups
            .iter()
            .find(|(known, _)| known == title)
            .map(|(_, ids)| ids.as_slice())
            .unwrap_or_default();
        for &id in ids {
            if let Some(mix) = binding.recommended_mix(id) {
                collections.push(mix_collection(binding, id, &mix).await?);
            }
        }
        answered.push(CollectionGroup {
            group_title: title.clone(),
            collections,
        });
    }
    Ok(FetchCollectionGroupsResponse {
        call_id: None,
        groups: answered,
    })
}

/// The mixes among `ids`: Rune's own as kept, and the recommended ones, built again once when
/// some are not known. Those that are gone are left out, as a local library leaves them out.
pub(super) async fn mixes_by_ids(
    binding: &AppleMusicBinding,
    ids: &[i32],
) -> Result<FetchCollectionByIdsResponse> {
    let refs: Vec<MixRef> = ids.iter().map(|id| binding.mix_ref(*id)).collect();
    if refs
        .iter()
        .any(|mix| matches!(mix, MixRef::Recommended(id) if binding.recommended_mix(*id).is_none()))
    {
        recommendation_groups(binding, true).await?;
    }
    let kept = KeptCovers::read(binding).await;
    let mut result = Vec::with_capacity(ids.len());
    for mix in refs {
        match mix {
            MixRef::Rune(stored) => {
                if let Some(mix) = rune_mix_if_kept(binding, &stored).await? {
                    result.push(rune_mix_collection(binding, &kept, &mix).await?);
                }
            }
            MixRef::Recommended(id) => {
                if let Some(mix) = binding.recommended_mix(id) {
                    result.push(mix_collection(binding, id, &mix).await?);
                }
            }
        }
    }
    Ok(FetchCollectionByIdsResponse {
        call_id: None,
        collection_type: CollectionType::Mix,
        result,
    })
}

/// A mix by id: a Rune mix as kept, or a recommended one, which is locked, in the group it was
/// first listed in, and plays in the player's mode as an album or playlist page does.
pub(crate) async fn get_mix_by_id_request(
    binding: &AppleMusicBinding,
    request: &GetMixByIdRequest,
) -> Result<Option<GetMixByIdResponse>> {
    let mix = match binding.mix_ref(request.mix_id) {
        MixRef::Rune(stored) => rune_mix_of(binding, &binding.rune_mixes.mix(&stored).await?)?,
        MixRef::Recommended(id) => {
            let mix = known_mix(binding, id).await?.ok_or_else(mix_not_found)?;
            Mix {
                id,
                name: mix.name,
                group: mix.group,
                locked: true,
                mode: None,
            }
        }
    };
    Ok(Some(GetMixByIdResponse { mix, call_id: None }))
}

/// The queries a mix plays: a Rune mix's as kept, with the ids the UI knows their items by, and
/// the one query naming what a recommended mix plays.
pub(crate) async fn fetch_mix_queries_request(
    binding: &AppleMusicBinding,
    request: &FetchMixQueriesRequest,
) -> Result<Option<FetchMixQueriesResponse>> {
    let queries = match binding.mix_ref(request.mix_id) {
        MixRef::Rune(stored) => {
            let mix = binding.rune_mixes.mix(&stored).await?;
            session_queries(binding, &mix.queries)?
        }
        MixRef::Recommended(id) => {
            known_mix(binding, id).await?.ok_or_else(mix_not_found)?;
            vec![mix_query(mix_target(binding, id)?)]
        }
    };
    Ok(Some(FetchMixQueriesResponse {
        result: queries
            .into_iter()
            .enumerate()
            .map(|(index, query)| StoredMixQuery {
                clause_id: format!("{}:{index}", request.mix_id),
                operator: query.operator,
                parameter: query.parameter,
                target_uuid: None,
                available: true,
            })
            .collect(),
        call_id: None,
    }))
}

// Rune's own mixes --------------------------------------------------------------------------

/// What a Rune mix's query names a Rune playlist by, before the id its store keeps it under. No
/// key of Apple's starts with it.
const RUNE_PLAYLIST: &str = "rune-playlist:";

/// The queries of a Rune mix, parsed, as long as the mix query translation answers them; any
/// other is refused as unsupported by `request`. A mix made without queries, as a track's menu
/// makes one before it adds the track, has none to check.
fn mix_operators(queries: &[MixQuery], request: &'static str) -> Result<Vec<QueryOperator>> {
    if !queries.is_empty() {
        mixes::plan(queries, request)?;
    }
    queries
        .iter()
        .map(|query| {
            QueryOperator::parse(&query.operator, &query.parameter)
                .map_err(|_| unsupported_query(request))
        })
        .collect()
}

/// The queries as the store keeps them, each item by Apple's key of it, as a mix holds it:
/// catalog songs, albums and playlists are kept as they are, and nothing is added to the user's
/// Apple Music library. `operators` are the queries parsed. The keys are Apple's, not the ids
/// the UI numbered for the session, so they name the same items after a restart. A Rune playlist
/// is kept by the id its store keeps it under.
async fn keepable_queries(
    binding: &AppleMusicBinding,
    queries: &[MixQuery],
    operators: &[QueryOperator],
) -> Result<Vec<StoredQuery>> {
    let mut stored = Vec::with_capacity(queries.len());
    for (query, operator) in queries.iter().zip(operators) {
        let parameter = match *operator {
            QueryOperator::LibArtist(id) => stable_artist(binding, id).await?,
            QueryOperator::LibAlbum(id) => binding.resolve(AppleIdKind::Album, id)?,
            QueryOperator::LibTrack(id) => binding.resolve_track(id)?,
            QueryOperator::LibPlaylist(id) => match binding.playlist_ref(id) {
                PlaylistRef::Rune(stored) => format!("{RUNE_PLAYLIST}{stored}"),
                PlaylistRef::Catalog(id) => binding.resolve(AppleIdKind::Playlist, id)?,
            },
            _ => query.parameter.clone(),
        };
        stored.push(StoredQuery {
            operator: query.operator.clone(),
            parameter,
        });
    }
    Ok(stored)
}

/// The key an artist the UI numbered `id` is kept by: the artist of the library's artist list it
/// is, by its catalog artist where Apple names one, and an id from before the list told its
/// artists apart that way by the artist its spelling goes with now. Any other artist is kept by
/// Apple's id of it.
async fn stable_artist(binding: &AppleMusicBinding, id: i32) -> Result<String> {
    Ok(match binding.named_artist_key(AppleIdKind::Artist, id) {
        Some(remote) => match binding.named_artist(&remote).await {
            Ok(artist) => artist.remote(),
            Err(_) => remote,
        },
        None => binding.resolve(AppleIdKind::Artist, id)?,
    })
}

/// A Rune mix's queries as the UI sends them, each item by the id the UI knows it by, which is
/// given afresh.
fn session_queries(binding: &AppleMusicBinding, stored: &[StoredQuery]) -> Result<Vec<MixQuery>> {
    stored
        .iter()
        .map(|query| {
            let target = MixOperator::from_syntax(&query.operator)
                .ok()
                .and_then(MixOperator::target_collection);
            let id = match target {
                Some(StoredTarget::Artists) => {
                    binding.intern(AppleIdKind::Artist, &query.parameter)?
                }
                Some(StoredTarget::Albums) => {
                    binding.intern(AppleIdKind::Album, &query.parameter)?
                }
                Some(StoredTarget::Tracks) => binding.intern_track(&query.parameter)?,
                Some(StoredTarget::Playlists) => {
                    match query.parameter.strip_prefix(RUNE_PLAYLIST) {
                        Some(stored) => binding.intern(AppleIdKind::RunePlaylist, stored)?,
                        None => binding.intern(AppleIdKind::Playlist, &query.parameter)?,
                    }
                }
                // Genres are never kept: the translation answers no query of them.
                Some(StoredTarget::Genres) | None => {
                    return Ok(MixQuery {
                        operator: query.operator.clone(),
                        parameter: query.parameter.clone(),
                    });
                }
            };
            Ok(MixQuery {
                operator: query.operator.clone(),
                parameter: id.to_string(),
            })
        })
        .collect()
}

/// The store's id of the Rune mix a write names. One Apple recommends is locked, so the write is
/// refused as unsupported by `request`, and an id that names no mix is an input error.
fn rune_mix_to_change(
    binding: &AppleMusicBinding,
    id: i32,
    request: &'static str,
) -> Result<String> {
    match binding.mix_ref(id) {
        MixRef::Rune(stored) => Ok(stored),
        MixRef::Recommended(id) => {
            binding.resolve(AppleIdKind::Mix, id)?;
            Err(unsupported_query(request))
        }
    }
}

/// A Rune mix in wire form.
fn rune_mix_of(binding: &AppleMusicBinding, mix: &StoredMix) -> Result<Mix> {
    Ok(Mix {
        id: binding.intern(AppleIdKind::RuneMix, &mix.id)?,
        name: mix.name.clone(),
        group: mix.group.clone(),
        locked: false,
        mode: mix.mode,
    })
}

/// The most covers a Rune mix is listed with, as many as a local library's mixes are.
const MIX_COVERS: usize = 18;

/// What is kept of the library for the covers of Rune mixes, here or on disk. Nothing is read
/// from Apple for it.
struct KeptCovers {
    songs: Option<Arc<AppleSongs>>,
    albums: Option<Arc<Vec<LibraryAlbum>>>,
    artists: Option<Arc<NamedArtists>>,
}

impl KeptCovers {
    async fn read(binding: &AppleMusicBinding) -> Self {
        Self {
            songs: binding.kept_songs().await,
            albums: binding.kept_albums().await,
            artists: binding.kept_named_artists().await,
        }
    }

    /// A song's cover, as the binding holds the song.
    fn track(binding: &AppleMusicBinding, track: &str) -> Option<String> {
        cover_url(binding.known_song(track)?.as_ref().artwork())
    }

    /// An album's cover: as it was listed, or as the album list or a song of it names it.
    fn album(&self, binding: &AppleMusicBinding, id: i32) -> Option<String> {
        if let Some(cover) = binding
            .collection_info(AppleIdKind::Album, id)
            .and_then(|info| info.cover)
        {
            return Some(cover);
        }
        let remote = binding.resolve(AppleIdKind::Album, id).ok()?;
        let listed = self
            .albums
            .as_deref()
            .and_then(|albums| albums.iter().find(|album| album.id == remote));
        if let Some(cover) = listed.and_then(|album| cover_url(album_artwork(album))) {
            return Some(cover);
        }
        self.songs.as_deref()?.songs().iter().find_map(|song| {
            (song.album_id() == Some(remote.as_str()))
                .then(|| cover_url(SongRef::Library(song).artwork()))
                .flatten()
        })
    }

    /// An artist's picture, or else the covers of the songs it is credited with, an album's
    /// once.
    fn artist(&self, binding: &AppleMusicBinding, id: i32) -> Vec<String> {
        let Some(remote) = binding.named_artist_key(AppleIdKind::Artist, id) else {
            return binding
                .collection_info(AppleIdKind::Artist, id)
                .and_then(|info| info.cover)
                .into_iter()
                .collect();
        };
        let Some(artist) = self.artists.as_deref().and_then(|named| named.get(&remote)) else {
            return Vec::new();
        };
        if let Some(cover) = cover_url(artist.artwork.as_ref()) {
            return vec![cover];
        }
        let Some(songs) = self.songs.as_deref() else {
            return Vec::new();
        };
        distinct_covers(
            songs
                .entries()
                .filter(|(track, _)| artist.credits_track(track))
                .filter_map(|(_, song)| cover_url(SongRef::Library(song).artwork())),
        )
    }

    /// A playlist's covers: those of a Rune playlist's songs, or a catalog playlist's own.
    async fn playlist(binding: &AppleMusicBinding, id: i32) -> Vec<String> {
        match binding.playlist_ref(id) {
            PlaylistRef::Rune(stored) => {
                let Ok(playlist) = binding.rune_playlists.playlist(&stored).await else {
                    return Vec::new();
                };
                distinct_covers(
                    playlist
                        .entries
                        .iter()
                        .filter_map(|entry| Self::track(binding, &entry.track)),
                )
            }
            PlaylistRef::Catalog(id) => binding
                .collection_info(AppleIdKind::Playlist, id)
                .and_then(|info| info.cover)
                .into_iter()
                .collect(),
        }
    }

    /// The covers of the item `query` names; none for a query that names no item.
    async fn item(&self, binding: &AppleMusicBinding, query: &MixQuery) -> Option<Vec<String>> {
        Some(
            match QueryOperator::parse(&query.operator, &query.parameter).ok()? {
                QueryOperator::LibTrack(id) => binding
                    .resolve_track(id)
                    .ok()
                    .and_then(|track| Self::track(binding, &track))
                    .into_iter()
                    .collect(),
                QueryOperator::LibAlbum(id) => self.album(binding, id).into_iter().collect(),
                QueryOperator::LibArtist(id) => self.artist(binding, id),
                QueryOperator::LibPlaylist(id) => Self::playlist(binding, id).await,
                _ => return None,
            },
        )
    }
}

/// The first `MIX_COVERS` of `covers`, each once, in order.
fn distinct_covers(covers: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    covers
        .filter(|cover| seen.insert(cover.clone()))
        .take(MIX_COVERS)
        .collect()
}

/// The covers of a Rune mix, each by the id of its cover art, from what is `kept` (read once for
/// a whole listing): nothing is read from Apple for them. A mix of one item has each cover of
/// that item, so its tile turns through them as a local mix's does; a mix of several items has
/// the first cover of each, in the order its queries name them.
async fn rune_mix_covers(
    binding: &AppleMusicBinding,
    kept: &KeptCovers,
    queries: &[MixQuery],
) -> Result<Vec<(i32, String)>> {
    let mut items = Vec::new();
    for query in queries {
        if let Some(covers) = kept.item(binding, query).await {
            items.push(covers);
        }
    }
    let covers = if items.len() == 1 {
        distinct_covers(items.into_iter().flatten())
    } else {
        distinct_covers(
            items
                .into_iter()
                .filter_map(|covers| covers.into_iter().next()),
        )
    };
    covers
        .into_iter()
        .map(|url| Ok((binding.intern(AppleIdKind::CoverArt, &url)?, url)))
        .collect()
}

/// A Rune mix as the mixes are listed, with the queries that play it.
async fn rune_mix_collection(
    binding: &AppleMusicBinding,
    kept: &KeptCovers,
    mix: &StoredMix,
) -> Result<Collection> {
    let queries = session_queries(binding, &mix.queries)?;
    let covers = rune_mix_covers(binding, kept, &queries).await?;
    Ok(Collection {
        id: binding.intern(AppleIdKind::RuneMix, &mix.id)?,
        name: mix.name.clone(),
        queries,
        collection_type: CollectionType::Mix,
        cover_art_map: binding.cover_map(covers).await?,
        readonly: false,
    })
}

/// The Rune mix the store keeps under `stored`, unless it was removed or its file cannot be
/// read: a list leaves it out either way.
async fn rune_mix_if_kept(binding: &AppleMusicBinding, stored: &str) -> Result<Option<StoredMix>> {
    Ok(binding
        .rune_mixes
        .listed()
        .await?
        .iter()
        .find(|mix| mix.id == stored)
        .cloned())
}

/// The mixes a track or a collection can be added to, which is what the UI lists them for:
/// Rune's own. The recommended ones are locked, and listed on the Mixes page.
pub(crate) async fn fetch_all_mixes_request(
    binding: &AppleMusicBinding,
    _: &FetchAllMixesRequest,
) -> Result<Option<FetchAllMixesResponse>> {
    let mixes = binding
        .rune_mixes
        .listed()
        .await?
        .iter()
        .map(|mix| rune_mix_of(binding, mix))
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(FetchAllMixesResponse {
        mixes,
        call_id: None,
    }))
}

/// Makes a Rune mix, kept on this device beside the recommended ones. What its queries name of
/// the catalog is kept as it is, and nothing is added to the library.
pub(crate) async fn create_mix_request(
    binding: &AppleMusicBinding,
    request: &CreateMixRequest,
) -> Result<Option<CreateMixResponse>> {
    let operators = mix_operators(&request.queries, CreateMixRequest::wire_name())?;
    let queries = keepable_queries(binding, &request.queries, &operators).await?;
    let made = binding
        .rune_mixes
        .create(&request.name, &request.group, request.mode, queries)
        .await?;
    binding.content_changed();
    Ok(Some(CreateMixResponse {
        mix: rune_mix_of(binding, &made)?,
        call_id: None,
    }))
}

/// Names, groups and fills a Rune mix anew, as `create_mix_request` makes one.
pub(crate) async fn update_mix_request(
    binding: &AppleMusicBinding,
    request: &UpdateMixRequest,
) -> Result<Option<UpdateMixResponse>> {
    let wire_name = UpdateMixRequest::wire_name();
    let stored = rune_mix_to_change(binding, request.mix_id, wire_name)?;
    let operators = mix_operators(&request.queries, wire_name)?;
    let queries = keepable_queries(binding, &request.queries, &operators).await?;
    let mix = binding
        .rune_mixes
        .update(
            &stored,
            &request.name,
            &request.group,
            request.mode,
            queries,
        )
        .await?;
    binding.content_changed();
    Ok(Some(UpdateMixResponse {
        mix: rune_mix_of(binding, &mix)?,
        call_id: None,
    }))
}

pub(crate) async fn remove_mix_request(
    binding: &AppleMusicBinding,
    request: &RemoveMixRequest,
) -> Result<Option<RemoveMixResponse>> {
    let stored = rune_mix_to_change(binding, request.mix_id, RemoveMixRequest::wire_name())?;
    binding.rune_mixes.remove(&stored).await?;
    binding.content_changed();
    Ok(Some(RemoveMixResponse {
        mix_id: request.mix_id,
        call_id: None,
    }))
}

/// Adds an item to the end of a Rune mix, as long as the mix's queries are still ones the mix
/// query translation answers. What it names of the catalog is kept as it is, and nothing is
/// added to the library.
pub(crate) async fn add_item_to_mix_request(
    binding: &AppleMusicBinding,
    request: &AddItemToMixRequest,
) -> Result<Option<AddItemToMixResponse>> {
    let wire_name = AddItemToMixRequest::wire_name();
    let stored = rune_mix_to_change(binding, request.mix_id, wire_name)?;
    let mix = binding.rune_mixes.mix(&stored).await?;
    let added = MixQuery {
        operator: request.operator.clone(),
        parameter: request.parameter.clone(),
    };
    let mut queries = session_queries(binding, &mix.queries)?;
    queries.push(added.clone());
    let mut operators = mix_operators(&queries, wire_name)?;
    let operators = operators.split_off(operators.len() - 1);
    let mut kept = keepable_queries(binding, std::slice::from_ref(&added), &operators).await?;
    binding
        .rune_mixes
        .add_query(&stored, kept.remove(0))
        .await?;
    binding.content_changed();
    Ok(Some(AddItemToMixResponse { call_id: None }))
}

/// The mixes of the home: Rune's own in the order they were made, with the covers of what they
/// play as kept, then the recommended ones in Apple's order, each once.
pub(super) async fn home_mixes(binding: &AppleMusicBinding) -> Result<Vec<HomeMix>> {
    let own = binding.rune_mixes.listed().await?;
    let kept = KeptCovers::read(binding).await;
    let mut listed = Vec::new();
    for mix in own.iter() {
        let queries = session_queries(binding, &mix.queries)?;
        listed.push(HomeMix {
            id: binding.intern(AppleIdKind::RuneMix, &mix.id)?,
            name: mix.name.clone(),
            covers: rune_mix_covers(binding, &kept, &queries).await?,
            queries,
        });
    }
    let groups = recommendations_beside_own(binding, !own.is_empty()).await?;
    let mut seen = HashSet::new();
    for &id in groups.iter().flat_map(|(_, ids)| ids) {
        if !seen.insert(id) {
            continue;
        }
        let Some(mix) = binding.recommended_mix(id) else {
            continue;
        };
        listed.push(HomeMix {
            id,
            name: mix.name,
            covers: mix.cover.map(|cover| vec![(id, cover)]).unwrap_or_default(),
            queries: vec![mix_query(mix_target(binding, id)?)],
        });
    }
    Ok(listed)
}

#[cfg(test)]
mod tests {
    use apple_music::models::Recommendation;
    use serde_json::json;

    use super::{RECENTLY_PLAYED_GROUP, UNTITLED_GROUP, group_title};

    fn titled(title: Option<&str>, kind: &str) -> String {
        let mut attributes = json!({"kind": kind});
        if let Some(title) = title {
            attributes["title"] = json!({"stringForDisplay": title});
        }
        let recommendation: Recommendation = serde_json::from_value(json!({
            "id": "rec",
            "type": "personal-recommendation",
            "attributes": attributes,
        }))
        .unwrap();
        group_title(&recommendation)
    }

    #[test]
    fn groups_are_titled_by_their_kind_when_untitled_and_never_by_their_title() {
        // Recent plays are one group whatever Apple titled them.
        for title in [
            None,
            Some(" "),
            Some("Recently Played"),
            Some("Zuletzt gespielt"),
        ] {
            assert_eq!(titled(title, "recently-played"), RECENTLY_PLAYED_GROUP);
        }
        for kind in ["music-recommendations", "heavy-rotation", "unknown"] {
            assert_eq!(titled(None, kind), UNTITLED_GROUP);
            assert_eq!(titled(Some("  "), kind), UNTITLED_GROUP);
            assert_eq!(titled(Some("Made for You"), kind), "Made for You");
            // A title that names recent plays, or no title, is a title like any other.
            assert_eq!(titled(Some("Recently Played"), kind), "Recently Played");
            assert_eq!(titled(Some("Untitled"), kind), "Untitled");
        }
    }
}
