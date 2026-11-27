//! Resolving a mix query to a list of tracks for a streaming service's library.

use std::collections::HashSet;
use std::hash::Hash;

use anyhow::{Error, Result};
use database::actions::mixes::QueryOperator;
use database::mix_operator::ArtistSection;
use rand::Rng;
use rand::seq::{SliceRandom, index};

use crate::TypedRequest;
use crate::messages::{MixQuery, PlayingItemRef, ResolveMixQueryRequest, ResolveMixQueryResponse};
use crate::request_error::ExpectedRequestError;
use crate::rpc::UnsupportedReason;
use crate::service::ServiceFuture;
use crate::utils::{RESOLVED_TRACK_LIMIT, resolved_start};

/// How many songs of a service's song list are read to keep only those with a cover, when
/// reading the list costs the service requests, so a huge song list is not read whole.
pub(crate) const MAX_COVER_POOL: usize = 8192;

/// The error for a query a service does not answer, naming `request`.
pub(crate) fn unsupported_query(request: &'static str) -> Error {
    ExpectedRequestError::Unsupported {
        request,
        reason: UnsupportedReason::NotImplemented,
    }
    .into()
}

/// A collection whose tracks a query names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MixTarget {
    Album(i32),
    Artist(i32),
    Playlist(i32),
}

/// One item a query names: a collection, or a single track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MixItem {
    Collection(MixTarget),
    Track(i32),
}

/// The order the items of `Plan::Items` are listed in, as a local library lists a mix of
/// several: the single tracks first, in the order the queries name them, then the collections
/// in that order. Each collection lists its songs in its own order, and a song comes once,
/// where it first comes (see `first_of_each_key`). An item named again comes only where it
/// first comes, so a collection is read once.
pub(crate) fn union_order(items: &[MixItem]) -> impl Iterator<Item = MixItem> + '_ {
    let tracks = items
        .iter()
        .filter(|item| matches!(item, MixItem::Track(_)));
    let collections = items
        .iter()
        .filter(|item| matches!(item, MixItem::Collection(_)));
    let mut seen = HashSet::new();
    tracks
        .chain(collections)
        .copied()
        .filter(move |item| seen.insert(*item))
}

/// The songs in order, each key once, where it first comes, as a local library lists each file
/// of a mix once.
pub(crate) fn first_of_each_key<T, K: Eq + Hash>(
    songs: impl IntoIterator<Item = T>,
    key: impl Fn(&T) -> K,
) -> Vec<T> {
    let mut seen = HashSet::new();
    songs
        .into_iter()
        .filter(|song| seen.insert(key(song)))
        .collect()
}

/// One of the lists an artist's page holds the songs of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArtistListing {
    /// The songs of the releases of one section, release by release.
    Section(ArtistSection),
    /// The artist's tracks, in the service's own order.
    Tracks,
}

/// What a service gives the resolver.
pub(crate) trait MixSource: Send + Sync {
    /// The id of the library the tracks belong to.
    fn library_id(&self) -> String;

    /// The track keys of a collection, in order.
    fn collection_tracks(&self, target: MixTarget) -> ServiceFuture<'_, Vec<String>>;

    /// The first `limit` track keys of the whole song list, in the order a song list shows them,
    /// read afresh each time: a list kept a while can differ from the service's anywhere, and
    /// would play or queue tracks it no longer has.
    fn all_tracks(&self, limit: usize) -> ServiceFuture<'_, Vec<String>>;

    /// The track key a track id stands for, or None for an id the service never gave.
    fn track_key(&self, id: i32) -> Result<Option<String>>;

    /// The first `limit` track keys of one list of the page of the artist `artist`. A service
    /// that builds no artist page leaves it unsupported.
    fn artist_page_tracks(
        &self,
        _artist: i32,
        _listing: ArtistListing,
        _limit: usize,
    ) -> ServiceFuture<'_, Vec<String>> {
        Box::pin(async { Err(unsupported_query(ResolveMixQueryRequest::wire_name())) })
    }
}

/// What a non-empty list of queries asks for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Plan {
    /// One album, artist or playlist.
    Collection(MixTarget),
    /// Single tracks, or several items together, in the order the queries name them: the
    /// songs of all of them, listed as `union_order` says.
    Items(Vec<MixItem>),
    /// One list of an artist's page: the artist is the id the UI knows it by.
    ArtistPage { artist: i32, listing: ArtistListing },
    /// Every song.
    All,
    /// This many songs drawn at random from every song, so the same query gives another draw
    /// each time, as for a local library.
    Random(usize),
}

/// A plan and whether only the tracks that have a cover are kept, which a local library does
/// for every plan.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct QueryPlan {
    pub(crate) plan: Plan,
    pub(crate) cover_only: bool,
}

/// `count` distinct indexes below `len`, in random order, as `rng` draws them.
pub(crate) fn sample_indices(rng: &mut impl Rng, len: usize, count: usize) -> Vec<usize> {
    let mut picked = index::sample(rng, len, count.min(len)).into_vec();
    picked.shuffle(rng);
    picked
}

/// The plan for `queries`, or an unsupported error naming `request`. An album may come with
/// `sort::track_number`, which its tracks are listed in already. Artists, albums, playlists and
/// single tracks combine into their union, as for a local library. A list of an artist's page
/// stands alone.
pub(crate) fn plan(queries: &[MixQuery], request: &'static str) -> Result<QueryPlan> {
    let mut parsed = Vec::with_capacity(queries.len());
    for query in queries {
        parsed.push(
            QueryOperator::parse(&query.operator, &query.parameter)
                .map_err(|_| unsupported_query(request))?,
        );
    }
    let in_track_order = parsed.contains(&QueryOperator::SortTrackNumber(true));
    let cover_only = parsed.contains(&QueryOperator::FilterWithCoverArt(true));
    let rest: Vec<&QueryOperator> = parsed
        .iter()
        .filter(|query| {
            !matches!(
                query,
                QueryOperator::SortTrackNumber(true) | QueryOperator::FilterWithCoverArt(true)
            )
        })
        .collect();
    let plan = match rest.as_slice() {
        [QueryOperator::LibAlbum(id)] => Ok(Plan::Collection(MixTarget::Album(*id))),
        [QueryOperator::LibArtist(id)] if !in_track_order => {
            Ok(Plan::Collection(MixTarget::Artist(*id)))
        }
        [QueryOperator::LibPlaylist(id)] if !in_track_order => {
            Ok(Plan::Collection(MixTarget::Playlist(*id)))
        }
        [QueryOperator::LibArtistSection(artist, section)] if !in_track_order => {
            Ok(Plan::ArtistPage {
                artist: *artist,
                listing: ArtistListing::Section(*section),
            })
        }
        [QueryOperator::LibArtistTracks(artist)] if !in_track_order => Ok(Plan::ArtistPage {
            artist: *artist,
            listing: ArtistListing::Tracks,
        }),
        [QueryOperator::LibAll(true)] if !in_track_order => Ok(Plan::All),
        // A local library takes the largest of several counts.
        many if !in_track_order
            && !many.is_empty()
            && many
                .iter()
                .all(|query| matches!(query, QueryOperator::LibRandom(_))) =>
        {
            let count = many
                .iter()
                .filter_map(|query| match query {
                    QueryOperator::LibRandom(count) => Some(*count),
                    _ => None,
                })
                .max()
                .unwrap_or_default();
            Ok(Plan::Random(
                usize::try_from(count)
                    .unwrap_or_default()
                    .min(RESOLVED_TRACK_LIMIT),
            ))
        }
        many if !in_track_order && !many.is_empty() => many
            .iter()
            .map(|query| match query {
                QueryOperator::LibTrack(id) => Ok(MixItem::Track(*id)),
                QueryOperator::LibAlbum(id) => Ok(MixItem::Collection(MixTarget::Album(*id))),
                QueryOperator::LibArtist(id) => Ok(MixItem::Collection(MixTarget::Artist(*id))),
                QueryOperator::LibPlaylist(id) => Ok(MixItem::Collection(MixTarget::Playlist(*id))),
                _ => Err(unsupported_query(request)),
            })
            .collect::<Result<Vec<_>>>()
            .map(Plan::Items),
        _ => Err(unsupported_query(request)),
    }?;
    Ok(QueryPlan { plan, cover_only })
}

/// The keys the plan names, in order and without repeats.
async fn plan_tracks(source: &dyn MixSource, plan: Plan) -> Result<Vec<String>> {
    match plan {
        Plan::Collection(target) => source.collection_tracks(target).await,
        Plan::ArtistPage { artist, listing } => {
            source
                .artist_page_tracks(artist, listing, RESOLVED_TRACK_LIMIT)
                .await
        }
        Plan::All => source.all_tracks(RESOLVED_TRACK_LIMIT).await,
        // A random draw is not a list a resolve can name twice, so it stays refused here.
        Plan::Random(_) => Err(unsupported_query(ResolveMixQueryRequest::wire_name())),
        Plan::Items(items) => {
            let mut keys = Vec::new();
            for item in union_order(&items) {
                match item {
                    // An id the service never gave names no track, as for a local library.
                    MixItem::Track(id) => keys.extend(source.track_key(id)?),
                    MixItem::Collection(target) => {
                        keys.extend(source.collection_tracks(target).await?)
                    }
                }
            }
            Ok(first_of_each_key(keys, Clone::clone))
        }
    }
}

/// Resolves a request the way a local library does: the queried tracks without the caller's
/// own queue, and the index to start from when the caller wants to play.
pub(crate) async fn resolve_mix_query(
    source: &dyn MixSource,
    request: &ResolveMixQueryRequest,
) -> Result<ResolveMixQueryResponse> {
    let items: Vec<PlayingItemRef> = if request.queries.is_empty() {
        request.fallback_playing_items.clone()
    } else {
        let planned = plan(&request.queries, ResolveMixQueryRequest::wire_name())?;
        // The keys carry no cover, so a cover filter cannot be applied to them.
        if planned.cover_only {
            return Err(unsupported_query(ResolveMixQueryRequest::wire_name()));
        }
        let mut keys = plan_tracks(source, planned.plan).await?;
        keys.truncate(RESOLVED_TRACK_LIMIT);
        let library_id = source.library_id();
        let excluded: HashSet<&PlayingItemRef> = request.excluded.items.iter().collect();
        keys.into_iter()
            .map(|track| PlayingItemRef::LibraryTrack {
                library_id: library_id.clone(),
                track,
            })
            .filter(|item| !excluded.contains(item))
            .collect()
    };
    let (items, start_index) = if request.instantly_play {
        let (items, start_index) = resolved_start(
            request.hint_position,
            items,
            request.initial_playback_item.as_ref(),
            request.fallback_playing_items.clone(),
        );
        (items, Some(start_index))
    } else {
        (items, None)
    };
    Ok(ResolveMixQueryResponse {
        call_id: request.call_id,
        playing_items: items,
        start_index: start_index
            .map(u32::try_from)
            .transpose()
            .map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid start index: {error}"))
            })?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queries(list: &[(&str, &str)]) -> Vec<MixQuery> {
        list.iter()
            .map(|(operator, parameter)| MixQuery {
                operator: (*operator).into(),
                parameter: (*parameter).into(),
            })
            .collect()
    }

    fn query_plan_of(list: &[(&str, &str)]) -> Result<QueryPlan> {
        plan(&queries(list), "TestRequest")
    }

    fn plan_of(list: &[(&str, &str)]) -> Result<Plan> {
        query_plan_of(list).map(|planned| planned.plan)
    }

    #[test]
    fn plans_a_random_sample_and_the_cover_filter() {
        assert_eq!(
            query_plan_of(&[("lib::random", "30"), ("filter::with_cover_art", "true")]).unwrap(),
            QueryPlan {
                plan: Plan::Random(30),
                cover_only: true
            }
        );
        assert_eq!(
            query_plan_of(&[("lib::random", "7")]).unwrap(),
            QueryPlan {
                plan: Plan::Random(7),
                cover_only: false
            }
        );
        // The largest count wins, and a huge one is bounded.
        assert_eq!(
            plan_of(&[("lib::random", "3"), ("lib::random", "9")]).unwrap(),
            Plan::Random(9)
        );
        assert_eq!(
            plan_of(&[("lib::random", "999999")]).unwrap(),
            Plan::Random(RESOLVED_TRACK_LIMIT)
        );
        // The filter goes with any plan.
        assert_eq!(
            query_plan_of(&[("lib::album", "4"), ("filter::with_cover_art", "true")]).unwrap(),
            QueryPlan {
                plan: Plan::Collection(MixTarget::Album(4)),
                cover_only: true
            }
        );
    }

    #[test]
    fn plans_one_list_of_an_artists_page_alone() {
        assert_eq!(
            plan_of(&[("lib::artist_section", "4:singles")]).unwrap(),
            Plan::ArtistPage {
                artist: 4,
                listing: ArtistListing::Section(ArtistSection::Singles)
            }
        );
        assert_eq!(
            plan_of(&[("lib::artist_tracks", "4")]).unwrap(),
            Plan::ArtistPage {
                artist: 4,
                listing: ArtistListing::Tracks
            }
        );
        // They do not combine with other items, or with an order of their own.
        assert!(plan_of(&[("lib::artist_tracks", "4"), ("lib::album", "2")]).is_err());
        assert!(plan_of(&[("lib::artist_tracks", "4"), ("sort::track_number", "true")]).is_err());
        assert!(plan_of(&[("lib::artist_section", "4:nothing")]).is_err());
    }

    #[tokio::test]
    async fn a_service_without_an_artist_page_refuses_its_lists() {
        let error = resolve_mix_query(&NoSource, &resolve(&[("lib::artist_tracks", "4")]))
            .await
            .err()
            .expect("a refused query");
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unsupported {
                request: "ResolveMixQueryRequest",
                ..
            })
        ));
    }

    #[test]
    fn a_union_names_each_item_once() {
        let album = MixItem::Collection(MixTarget::Album(1));
        let artist = MixItem::Collection(MixTarget::Artist(2));
        let track = MixItem::Track(5);
        let items = [album, track, artist, album, track, artist];
        assert_eq!(
            union_order(&items).collect::<Vec<_>>(),
            [track, album, artist]
        );
    }

    struct NoSource;

    impl MixSource for NoSource {
        fn library_id(&self) -> String {
            "library".into()
        }

        fn collection_tracks(&self, _: MixTarget) -> ServiceFuture<'_, Vec<String>> {
            Box::pin(async { Ok(vec!["a".into(), "b".into()]) })
        }

        fn all_tracks(&self, _: usize) -> ServiceFuture<'_, Vec<String>> {
            Box::pin(async { Ok(vec!["a".into(), "b".into()]) })
        }

        fn track_key(&self, _: i32) -> Result<Option<String>> {
            Ok(None)
        }
    }

    fn resolve(list: &[(&str, &str)]) -> ResolveMixQueryRequest {
        ResolveMixQueryRequest {
            call_id: Some(3),
            queries: queries(list),
            hint_position: -1,
            initial_playback_item: None,
            instantly_play: false,
            fallback_playing_items: Vec::new(),
            excluded: Default::default(),
        }
    }

    #[tokio::test]
    async fn a_resolve_refuses_a_random_draw_and_the_cover_filter() {
        for list in [
            &[("lib::random", "5")][..],
            &[("lib::all", "true"), ("filter::with_cover_art", "true")],
        ] {
            let error = resolve_mix_query(&NoSource, &resolve(list))
                .await
                .err()
                .expect("a refused query");
            assert!(
                matches!(
                    error.downcast_ref::<ExpectedRequestError>(),
                    Some(ExpectedRequestError::Unsupported {
                        request: "ResolveMixQueryRequest",
                        ..
                    })
                ),
                "{list:?}"
            );
        }
        // What it does take still resolves, and answers the call it was asked in.
        let resolved = resolve_mix_query(&NoSource, &resolve(&[("lib::all", "true")]))
            .await
            .unwrap();
        assert_eq!(resolved.playing_items.len(), 2);
        assert_eq!(resolved.call_id, Some(3));
    }

    #[test]
    fn samples_distinct_indexes_no_more_than_there_are_and_the_same_for_a_seed() {
        use rand::SeedableRng;
        use rand::rngs::StdRng;

        let picked = sample_indices(&mut rand::rng(), 100, 10);
        assert_eq!(picked.len(), 10);
        assert!(picked.iter().all(|index| *index < 100));
        assert_eq!(
            picked.iter().collect::<HashSet<_>>().len(),
            10,
            "no repeats"
        );
        assert_eq!(sample_indices(&mut rand::rng(), 3, 10).len(), 3);
        assert!(sample_indices(&mut rand::rng(), 3, 0).is_empty());
        let seeded = |seed| sample_indices(&mut StdRng::seed_from_u64(seed), 100, 10);
        assert_eq!(seeded(7), seeded(7));
    }

    #[test]
    fn plans_the_queries_the_app_sends() {
        assert_eq!(plan_of(&[("lib::all", "true")]).unwrap(), Plan::All);
        assert_eq!(
            plan_of(&[("lib::album", "4"), ("sort::track_number", "true")]).unwrap(),
            Plan::Collection(MixTarget::Album(4))
        );
        assert_eq!(
            plan_of(&[("lib::playlist", "2")]).unwrap(),
            Plan::Collection(MixTarget::Playlist(2))
        );
        assert_eq!(
            plan_of(&[("lib::track", "3"), ("lib::track", "1")]).unwrap(),
            Plan::Items(vec![MixItem::Track(3), MixItem::Track(1)])
        );
    }

    #[test]
    fn plans_artists_albums_playlists_and_tracks_together_with_the_cover_filter() {
        assert_eq!(
            query_plan_of(&[
                ("lib::artist", "5"),
                ("lib::album", "4"),
                ("filter::with_cover_art", "true"),
                ("lib::track", "3"),
                ("lib::playlist", "2"),
                ("lib::album", "4"),
            ])
            .unwrap(),
            QueryPlan {
                plan: Plan::Items(vec![
                    MixItem::Collection(MixTarget::Artist(5)),
                    MixItem::Collection(MixTarget::Album(4)),
                    MixItem::Track(3),
                    MixItem::Collection(MixTarget::Playlist(2)),
                    MixItem::Collection(MixTarget::Album(4)),
                ]),
                cover_only: true,
            }
        );
    }

    /// Lists each collection as its id followed by a letter per song, and a track by its id;
    /// track 9 is one it never gave.
    struct Collections;

    impl MixSource for Collections {
        fn library_id(&self) -> String {
            "library".into()
        }

        fn collection_tracks(&self, target: MixTarget) -> ServiceFuture<'_, Vec<String>> {
            let id = match target {
                MixTarget::Album(id) | MixTarget::Artist(id) | MixTarget::Playlist(id) => id,
            };
            Box::pin(async move { Ok(vec![format!("{id}a"), format!("{id}b"), "shared".into()]) })
        }

        fn all_tracks(&self, _: usize) -> ServiceFuture<'_, Vec<String>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn track_key(&self, id: i32) -> Result<Option<String>> {
            Ok((id != 9).then(|| format!("t{id}")))
        }
    }

    #[tokio::test]
    async fn a_union_resolves_its_tracks_first_then_each_collection_in_turn_each_song_once() {
        let resolved = resolve_mix_query(
            &Collections,
            &resolve(&[
                ("lib::album", "1"),
                ("lib::track", "7"),
                ("lib::artist", "2"),
                ("lib::track", "9"),
                ("lib::album", "1"),
                ("lib::track", "7"),
                ("lib::track", "3"),
            ]),
        )
        .await
        .unwrap();
        let keys: Vec<&str> = resolved
            .playing_items
            .iter()
            .map(|item| match item {
                PlayingItemRef::LibraryTrack { track, .. } => track.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(keys, ["t7", "t3", "1a", "1b", "shared", "2a", "2b"]);
    }

    #[test]
    fn refuses_queries_it_cannot_answer_and_names_the_request() {
        for list in [
            &[("filter::liked", "true")][..],
            &[("lib::album", "4"), ("lib::genre", "3")],
            &[
                ("lib::album", "4"),
                ("lib::album", "5"),
                ("sort::track_number", "true"),
            ],
            &[("lib::album", "4"), ("lib::directory.deep", "/music")],
            &[("lib::artist", "5"), ("sort::track_number", "true")],
            &[("sort::track_number", "true")],
            &[("lib::all", "false")],
            &[("lib::all", "true"), ("lib::track", "3")],
            &[("lib::nonsense", "1")],
            &[("lib::random", "3"), ("lib::album", "4")],
            &[("lib::random", "3"), ("sort::track_number", "true")],
            &[("filter::with_cover_art", "true")],
            &[("lib::random", "3"), ("filter::with_cover_art", "false")],
        ] {
            let error = plan_of(list).unwrap_err();
            assert!(
                matches!(
                    error.downcast_ref::<ExpectedRequestError>(),
                    Some(ExpectedRequestError::Unsupported {
                        request: "TestRequest",
                        ..
                    })
                ),
                "{list:?}"
            );
        }
    }
}
