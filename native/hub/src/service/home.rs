//! The library home of a streaming service's library: the groups a home screen asks for.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Error, Result};
use futures::future::try_join_all;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;

use crate::messages::{
    CollectionType, ComplexQueryEntry, ComplexQueryGroup, ComplexQueryRequest,
    ComplexQueryResponse, LibraryHomeListMode, LibraryHomeQuery, MixQuery, RequestErrorKind,
};
use crate::request_error::classify;
use crate::service::ServiceFuture;

/// How many entries a group holds, as for a local library.
pub(crate) const GROUP_LIMIT: usize = 25;

/// How long a random group keeps its entries, which lets covers settle.
const SEED_SECONDS: u64 = 600;

/// The seed a random group uses at `seconds` since the epoch: the same for a stretch of time,
/// then another.
fn rotated_seed(base: u64, seconds: u64) -> u64 {
    base.wrapping_add(seconds / SEED_SECONDS)
}

/// A source's random seed for now, from its fixed `base`.
pub(crate) fn rotating_seed(base: u64) -> u64 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    rotated_seed(base, seconds)
}

/// Whether the error says the account may not read the library, as when the service rejects
/// its sign-in or needs a new one: the caller needs to hear of it.
pub(crate) fn denies_the_account(error: &Error) -> bool {
    classify(error) == RequestErrorKind::PermissionDenied
}

/// An album, artist or playlist as listed, with the address of its cover if it has one.
#[derive(Clone)]
pub(crate) struct HomeItem {
    pub(crate) id: i32,
    pub(crate) name: String,
    pub(crate) cover: Option<String>,
}

/// A mix, with the queries that list its tracks: one the service recommends, which is its own and
/// locked, or one of Rune's kept beside them, as [`HomeSource::readonly`] tells.
#[derive(Clone)]
pub(crate) struct HomeMix {
    pub(crate) id: i32,
    pub(crate) name: String,
    /// Its covers, each by an id of its own, as a mix's tile turns through several.
    pub(crate) covers: Vec<(i32, String)>,
    pub(crate) queries: Vec<MixQuery>,
}

/// A track as listed, not numbered yet: only the tracks a group shows are.
#[derive(Clone)]
pub(crate) struct HomeTrack {
    /// The service's own id of the track.
    pub(crate) remote: String,
    pub(crate) name: String,
    pub(crate) cover: Option<String>,
}

/// What a service lists for the home. Every list comes oldest added first where the service
/// knows when things were added, and in the service's own order where it does not. The home
/// reverses a list to show the newest first.
pub(crate) trait HomeSource: Send + Sync {
    /// Every album, artist or playlist.
    fn collections(&self, collection_type: CollectionType) -> ServiceFuture<'_, Vec<HomeItem>>;

    /// Whether the user can rename and remove the albums, artists or playlists of the type, as
    /// the service's own pages let them.
    fn editable(&self, collection_type: CollectionType) -> bool;

    /// Whether the album, artist, playlist or mix `id` of the type cannot be changed here. By
    /// default the type tells, but a library may hold some of Rune's beside the service's own.
    fn readonly(&self, collection_type: CollectionType, _id: i32) -> bool {
        !self.editable(collection_type)
    }

    /// The `limit` newest albums, artists or playlists, newest first, when the service can list
    /// them so without reading every one. None leaves the home to read every one and reverse
    /// the list.
    fn newest(
        &self,
        _collection_type: CollectionType,
        _limit: usize,
    ) -> ServiceFuture<'_, Option<Vec<HomeItem>>> {
        Box::pin(async { Ok(None) })
    }

    /// Tracks to pick the home's tracks from, at least `limit` of them if the library has so
    /// many. A service that cannot list its whole library cheaply gives a sample of `limit`
    /// tracks, and the group is that sample.
    fn tracks(&self, limit: usize) -> ServiceFuture<'_, Vec<HomeTrack>>;

    /// The `limit` newest tracks, newest first, chosen from every track rather than from the
    /// ones `tracks` gives.
    fn newest_tracks(&self, limit: usize) -> ServiceFuture<'_, Vec<HomeTrack>>;

    /// The tracks the user likes.
    fn liked(&self) -> ServiceFuture<'_, Vec<HomeTrack>>;

    /// The mixes: Rune's own kept beside the service's, then those the service recommends, in
    /// its order. A service without either has none.
    fn mixes(&self) -> ServiceFuture<'_, Vec<HomeMix>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// The ids of the tracks, which are numbered if they were not.
    fn number(&self, tracks: Vec<HomeTrack>) -> Result<Vec<HomeItem>>;

    /// The cached covers among `(object id, cover address)` pairs.
    fn cover_paths(&self, covers: Vec<(i32, String)>) -> ServiceFuture<'_, HashMap<i32, String>>;

    /// A number that stays the same for a while, so a random group shows the same entries
    /// each time the home is read, and changes now and then. `rotating_seed` makes it.
    fn random_seed(&self) -> u64;

    /// Whether the source holds its sign-in. One that does not yet answers from what it kept,
    /// so a group that needs the sign-in is empty, as one that fails for any other reason is,
    /// rather than failing the home.
    fn holds_sign_in(&self) -> bool {
        true
    }
}

/// Whether a failure to read a group fails the whole home: one that denies an account the
/// source holds, which the caller needs to hear of.
fn fails_the_home(source: &dyn HomeSource, error: &Error) -> bool {
    denies_the_account(error) && source.holds_sign_in()
}

/// The entries a group shows, taken from `items` as the list mode asks.
fn pick<T>(
    mut items: Vec<T>,
    mode: LibraryHomeListMode,
    seed: u64,
    name: impl Fn(&T) -> &str,
) -> Vec<T> {
    match mode {
        LibraryHomeListMode::Name => items.sort_by(|left, right| name(left).cmp(name(right))),
        LibraryHomeListMode::Forward => {}
        LibraryHomeListMode::Reverse => items.reverse(),
        LibraryHomeListMode::Random => items.shuffle(&mut StdRng::seed_from_u64(seed)),
    }
    items.truncate(GROUP_LIMIT);
    items
}

fn query_of(operator: &str, id: i32) -> MixQuery {
    MixQuery {
        operator: operator.to_owned(),
        parameter: id.to_string(),
    }
}

/// Entries for albums, artists or playlists, each opening its own tracks.
async fn collection_entries(
    source: &dyn HomeSource,
    collection_type: CollectionType,
    operator: &str,
    mode: LibraryHomeListMode,
) -> Result<Vec<ComplexQueryEntry>> {
    let newest = match mode {
        LibraryHomeListMode::Reverse => source.newest(collection_type, GROUP_LIMIT).await?,
        _ => None,
    };
    let items = match newest {
        Some(mut items) => {
            items.truncate(GROUP_LIMIT);
            items
        }
        None => pick(
            source.collections(collection_type).await?,
            mode,
            source.random_seed(),
            |item| &item.name,
        ),
    };
    let covers = source.cover_paths(cover_pairs(&items)).await?;
    Ok(items
        .into_iter()
        .map(|item| ComplexQueryEntry {
            queries: vec![query_of(operator, item.id)],
            cover_art_map: cover_of(&covers, item.id),
            readonly: source.readonly(collection_type, item.id),
            id: item.id,
            name: item.name,
            collection_type,
        })
        .collect())
}

/// Entries for the mixes, each opening its own queries. They are an extra: when they cannot be
/// read, the group is empty and does not count as a failure of the home, unless the account
/// may not read the library (see `fails_the_home`).
async fn mix_entries(
    source: &dyn HomeSource,
    mode: LibraryHomeListMode,
) -> Result<Vec<ComplexQueryEntry>> {
    let mixes = match source.mixes().await {
        Ok(mixes) => mixes,
        Err(error) if fails_the_home(source, &error) => return Err(error),
        Err(error) => {
            log::warn!("Could not read the mixes: {error:#}");
            return Ok(Vec::new());
        }
    };
    if mixes.is_empty() {
        return Ok(Vec::new());
    }
    let mixes = pick(mixes, mode, source.random_seed(), |mix| &mix.name);
    let covers = source
        .cover_paths(
            mixes
                .iter()
                .flat_map(|mix| mix.covers.iter().cloned())
                .collect(),
        )
        .await?;
    Ok(mixes
        .into_iter()
        .map(|mix| ComplexQueryEntry {
            cover_art_map: mix
                .covers
                .iter()
                .filter_map(|(id, _)| covers.get(id).map(|path| (*id, path.clone())))
                .collect(),
            readonly: source.readonly(CollectionType::Mix, mix.id),
            id: mix.id,
            name: mix.name,
            queries: mix.queries,
            collection_type: CollectionType::Mix,
        })
        .collect())
}

/// Entries for tracks. Each plays the listed tracks, starting from itself, as a local
/// library's do.
async fn track_entries(
    source: &dyn HomeSource,
    tracks: Vec<HomeTrack>,
    mode: LibraryHomeListMode,
) -> Result<Vec<ComplexQueryEntry>> {
    let picked = pick(tracks, mode, source.random_seed(), |track| &track.name);
    let items = source.number(picked)?;
    let covers = source.cover_paths(cover_pairs(&items)).await?;
    let all: Vec<i32> = items.iter().map(|item| item.id).collect();
    Ok(items
        .into_iter()
        .enumerate()
        .map(|(index, item)| ComplexQueryEntry {
            queries: all
                .iter()
                .cycle()
                .skip(index)
                .take(all.len())
                .map(|id| query_of("lib::track", *id))
                .collect(),
            cover_art_map: cover_of(&covers, item.id),
            id: item.id,
            name: item.name,
            collection_type: CollectionType::Track,
            readonly: false,
        })
        .collect())
}

fn cover_pairs(items: &[HomeItem]) -> Vec<(i32, String)> {
    items
        .iter()
        .filter_map(|item| item.cover.clone().map(|cover| (item.id, cover)))
        .collect()
}

fn cover_of(covers: &HashMap<i32, String>, id: i32) -> HashMap<i32, String> {
    covers
        .get(&id)
        .map(|path| HashMap::from([(id, path.clone())]))
        .unwrap_or_default()
}

async fn group_entries(
    source: &dyn HomeSource,
    query: LibraryHomeQuery,
) -> Result<Vec<ComplexQueryEntry>> {
    match query {
        LibraryHomeQuery::Artists(mode) => {
            collection_entries(source, CollectionType::Artist, "lib::artist", mode).await
        }
        LibraryHomeQuery::Albums(mode) => {
            collection_entries(source, CollectionType::Album, "lib::album", mode).await
        }
        LibraryHomeQuery::Playlists(mode) => {
            collection_entries(source, CollectionType::Playlist, "lib::playlist", mode).await
        }
        // The newest tracks are chosen before the group is cut, as the first ones reversed
        // would leave out every track past them.
        LibraryHomeQuery::Tracks(LibraryHomeListMode::Reverse) => {
            let newest = source.newest_tracks(GROUP_LIMIT).await?;
            track_entries(source, newest, LibraryHomeListMode::Forward).await
        }
        LibraryHomeQuery::Tracks(mode) => {
            // A local library lists tracks by name in the order they were added.
            let mode = match mode {
                LibraryHomeListMode::Name => LibraryHomeListMode::Forward,
                mode => mode,
            };
            track_entries(source, source.tracks(GROUP_LIMIT).await?, mode).await
        }
        LibraryHomeQuery::Liked => {
            track_entries(source, source.liked().await?, LibraryHomeListMode::Forward).await
        }
        LibraryHomeQuery::Mixes(mode) => mix_entries(source, mode).await,
        LibraryHomeQuery::Genres(_) | LibraryHomeQuery::Most => Ok(Vec::new()),
    }
}

/// Whether a home group's answer tells that the service answered. Genres and the most played are
/// left empty without asking it, and mixes are an extra that a failure leaves empty (see
/// `mix_entries`).
fn asks_the_service(query: LibraryHomeQuery) -> bool {
    !matches!(
        query,
        LibraryHomeQuery::Genres(_) | LibraryHomeQuery::Mixes(_) | LibraryHomeQuery::Most
    )
}

/// Answers every group asked for, in order. A group the service cannot answer is empty and
/// logged. A failure that denies the account fails the whole request (see `fails_the_home`),
/// and so does a failure when no group that asks the service got an answer: the groups that
/// never ask it, such as genres, and the mixes, which are an extra, would otherwise hide an
/// unreachable server. An answer with nothing in it still counts, so an empty library has an
/// empty home.
pub(crate) async fn complex_query(
    source: &dyn HomeSource,
    request: &ComplexQueryRequest,
) -> Result<ComplexQueryResponse> {
    let groups = request.queries.iter().map(|query| async move {
        let (entries, failure) = match group_entries(source, query.query).await {
            Ok(entries) => (entries, None),
            Err(error) if fails_the_home(source, &error) => return Err(error),
            Err(error) => {
                log::warn!("Could not list the home group {}: {error:#}", query.title);
                (Vec::new(), Some(error))
            }
        };
        let answered = failure.is_none() && asks_the_service(query.query);
        Ok((
            ComplexQueryGroup {
                domain: query.query.domain(),
                title: query.title.clone(),
                entries,
            },
            (failure, answered),
        ))
    });
    let (groups, outcomes): (Vec<_>, Vec<_>) = try_join_all(groups).await?.into_iter().unzip();
    let answered = outcomes.iter().any(|(_, answered)| *answered);
    let failures: Vec<Error> = outcomes
        .into_iter()
        .filter_map(|(failure, _)| failure)
        .collect();
    if !answered && let Some(error) = failures.into_iter().next() {
        return Err(error);
    }
    Ok(ComplexQueryResponse {
        call_id: None,
        result: groups,
    })
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::*;
    use crate::messages::{CallerQueue, ComplexQuery};
    use crate::request_error::ExpectedRequestError;

    fn numbers(count: i32) -> Vec<i32> {
        (0..count).collect()
    }

    fn picked(items: Vec<i32>, mode: LibraryHomeListMode, seed: u64) -> Vec<i32> {
        pick(items, mode, seed, |_| "")
    }

    #[test]
    fn reverse_lists_the_newest_first_and_stops_at_the_cap() {
        let found = picked(numbers(40), LibraryHomeListMode::Reverse, 0);
        assert_eq!(found.len(), GROUP_LIMIT);
        assert_eq!(found[0], 39);
        assert_eq!(found[24], 15);
        let few = picked(numbers(3), LibraryHomeListMode::Reverse, 0);
        assert_eq!(few, [2, 1, 0]);
        let forward = picked(numbers(40), LibraryHomeListMode::Forward, 0);
        assert_eq!(forward, numbers(25));
    }

    #[test]
    fn random_picks_are_bounded_stable_for_a_seed_and_from_the_list() {
        let first = picked(numbers(100), LibraryHomeListMode::Random, 7);
        assert_eq!(first.len(), GROUP_LIMIT);
        assert!(first.iter().all(|item| (0..100).contains(item)));
        let mut distinct = first.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), GROUP_LIMIT);
        assert_eq!(first, picked(numbers(100), LibraryHomeListMode::Random, 7));
        // A short list is all there, in some order.
        let mut all = picked(numbers(5), LibraryHomeListMode::Random, 7);
        all.sort_unstable();
        assert_eq!(all, numbers(5));
    }

    #[test]
    fn name_sorts_by_the_name() {
        let items = vec!["b", "c", "a"];
        let found = pick(items, LibraryHomeListMode::Name, 0, |item| item);
        assert_eq!(found, ["a", "b", "c"]);
    }

    /// What a source's artists do: answer, answer with none, or fail as the variant says. Its
    /// albums always fail.
    #[derive(Clone, Copy)]
    enum Artists {
        Answer,
        Nothing,
        SignedOut,
        Unreachable,
    }

    /// What a source's mixes do: there are none, or two, or reading them fails as the variant
    /// says.
    #[derive(Clone, Copy)]
    enum Mixes {
        None,
        Answer,
        SignedOut,
        Unreachable,
    }

    struct Failing {
        artists: Artists,
        mixes: Mixes,
        /// Whether the source holds its sign-in.
        signed_in: bool,
    }

    impl HomeSource for Failing {
        fn collections(&self, collection_type: CollectionType) -> ServiceFuture<'_, Vec<HomeItem>> {
            let artists = self.artists;
            Box::pin(async move {
                match (collection_type, artists) {
                    (CollectionType::Album, _) | (_, Artists::Unreachable) => {
                        Err(anyhow!("the service is unreachable"))
                    }
                    (_, Artists::SignedOut) => Err(ExpectedRequestError::ServiceSignInRequired {
                        record_id: "record".into(),
                    }
                    .into()),
                    (_, Artists::Answer) => Ok(vec![HomeItem {
                        id: 1,
                        name: "Artist".into(),
                        cover: None,
                    }]),
                    (_, Artists::Nothing) => Ok(Vec::new()),
                }
            })
        }

        fn editable(&self, _: CollectionType) -> bool {
            false
        }

        fn tracks(&self, _: usize) -> ServiceFuture<'_, Vec<HomeTrack>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn newest_tracks(&self, _: usize) -> ServiceFuture<'_, Vec<HomeTrack>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn liked(&self) -> ServiceFuture<'_, Vec<HomeTrack>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn mixes(&self) -> ServiceFuture<'_, Vec<HomeMix>> {
            let mixes = self.mixes;
            Box::pin(async move {
                let mix = |id: i32, name: &str| HomeMix {
                    id,
                    name: name.into(),
                    covers: vec![(id, format!("cover-{id}"))],
                    queries: vec![query_of("lib::album", id * 10)],
                };
                match mixes {
                    Mixes::None => Ok(Vec::new()),
                    Mixes::Answer => Ok(vec![mix(7, "Morning"), mix(8, "Evening")]),
                    Mixes::SignedOut => Err(ExpectedRequestError::ServiceSignInRequired {
                        record_id: "record".into(),
                    }
                    .into()),
                    Mixes::Unreachable => Err(anyhow!("the service is unreachable")),
                }
            })
        }

        fn number(&self, _: Vec<HomeTrack>) -> Result<Vec<HomeItem>> {
            Ok(Vec::new())
        }

        /// Every cover is cached, at a path named after its address.
        fn cover_paths(
            &self,
            covers: Vec<(i32, String)>,
        ) -> ServiceFuture<'_, HashMap<i32, String>> {
            Box::pin(async move {
                Ok(covers
                    .into_iter()
                    .map(|(id, url)| (id, format!("/cache/{url}")))
                    .collect())
            })
        }

        fn random_seed(&self) -> u64 {
            0
        }

        fn holds_sign_in(&self) -> bool {
            self.signed_in
        }
    }

    fn asks() -> ComplexQueryRequest {
        let mode = LibraryHomeListMode::Forward;
        ComplexQueryRequest {
            queries: vec![
                ComplexQuery {
                    title: "Albums".into(),
                    query: LibraryHomeQuery::Albums(mode),
                },
                ComplexQuery {
                    title: "Artists".into(),
                    query: LibraryHomeQuery::Artists(mode),
                },
            ],
            queue: CallerQueue::default(),
            call_id: None,
        }
    }

    #[tokio::test]
    async fn a_group_that_fails_is_empty_and_the_others_still_answer() {
        let response = complex_query(
            &Failing {
                artists: Artists::Answer,
                mixes: Mixes::None,
                signed_in: true,
            },
            &asks(),
        )
        .await
        .unwrap();
        assert_eq!(response.result[0].title, "Albums");
        assert!(response.result[0].entries.is_empty());
        assert_eq!(response.result[1].entries[0].name, "Artist");
    }

    #[tokio::test]
    async fn a_sign_in_failure_fails_the_whole_request() {
        let error = complex_query(
            &Failing {
                artists: Artists::SignedOut,
                mixes: Mixes::None,
                signed_in: true,
            },
            &asks(),
        )
        .await
        .err()
        .unwrap();
        // The albums fail first, so only the sign-in failure can have failed the request.
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::ServiceSignInRequired { .. })
        ));
    }

    #[tokio::test]
    async fn a_service_that_answers_no_group_fails_the_request() {
        // The mixes are an extra, which does not count as an answer, so they do not hide the
        // failure.
        let mut request = asks();
        request.queries.push(ComplexQuery {
            title: "Mixes".into(),
            query: LibraryHomeQuery::Mixes(LibraryHomeListMode::Forward),
        });
        let error = complex_query(
            &Failing {
                artists: Artists::Unreachable,
                mixes: Mixes::None,
                signed_in: true,
            },
            &request,
        )
        .await
        .err()
        .unwrap();
        assert!(format!("{error:#}").contains("unreachable"));
    }

    #[tokio::test]
    async fn a_service_that_answers_a_group_with_nothing_has_an_empty_home() {
        let response = complex_query(
            &Failing {
                artists: Artists::Nothing,
                mixes: Mixes::None,
                signed_in: true,
            },
            &asks(),
        )
        .await
        .unwrap();
        assert!(response.result.iter().all(|group| group.entries.is_empty()));
    }

    #[tokio::test]
    async fn a_source_not_signed_in_yet_answers_what_it_kept_and_fails_only_when_nothing() {
        let mut request = asks();
        request.queries.push(ComplexQuery {
            title: "Playlists".into(),
            query: LibraryHomeQuery::Playlists(LibraryHomeListMode::Forward),
        });
        // The artists answer from what was kept, and the groups that need the sign-in are
        // empty.
        let source = Failing {
            artists: Artists::Answer,
            mixes: Mixes::None,
            signed_in: false,
        };
        let response = complex_query(&source, &request).await.unwrap();
        assert!(response.result[0].entries.is_empty());
        assert_eq!(response.result[1].entries[0].name, "Artist");
        // With nothing kept, the home fails with the sign-in it waits for.
        let source = Failing {
            artists: Artists::SignedOut,
            mixes: Mixes::None,
            signed_in: false,
        };
        request.queries.remove(0);
        let error = complex_query(&source, &request).await.err().unwrap();
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::ServiceSignInRequired { .. })
        ));
    }

    fn mixes_only(mode: LibraryHomeListMode) -> ComplexQueryRequest {
        ComplexQueryRequest {
            queries: vec![ComplexQuery {
                title: "Mixes".into(),
                query: LibraryHomeQuery::Mixes(mode),
            }],
            queue: CallerQueue::default(),
            call_id: None,
        }
    }

    #[tokio::test]
    async fn mixes_are_entries_in_the_order_asked_with_their_covers() {
        let source = Failing {
            artists: Artists::Answer,
            mixes: Mixes::Answer,
            signed_in: true,
        };
        let forward = complex_query(&source, &mixes_only(LibraryHomeListMode::Forward))
            .await
            .unwrap();
        let entries = &forward.result[0].entries;
        let listed: Vec<_> = entries
            .iter()
            .map(|entry| (entry.id, entry.name.as_str()))
            .collect();
        assert_eq!(listed, [(7, "Morning"), (8, "Evening")]);
        for entry in entries {
            assert_eq!(entry.collection_type, CollectionType::Mix);
            // The source tells nothing can be changed here.
            assert!(entry.readonly);
            assert_eq!(entry.queries[0].operator, "lib::album");
            assert_eq!(entry.queries[0].parameter, (entry.id * 10).to_string());
            assert_eq!(
                entry.cover_art_map,
                HashMap::from([(entry.id, format!("/cache/cover-{}", entry.id))])
            );
        }
        let reverse = complex_query(&source, &mixes_only(LibraryHomeListMode::Reverse))
            .await
            .unwrap();
        assert_eq!(reverse.result[0].entries[0].name, "Evening");
    }

    #[tokio::test]
    async fn a_source_without_mixes_has_an_empty_mixes_group() {
        let source = Failing {
            artists: Artists::Answer,
            mixes: Mixes::None,
            signed_in: true,
        };
        let mut request = mixes_only(LibraryHomeListMode::Forward);
        request.queries.extend(asks().queries);
        let response = complex_query(&source, &request).await.unwrap();
        assert!(response.result[0].entries.is_empty());
        assert_eq!(response.result[2].entries[0].name, "Artist");
    }

    #[tokio::test]
    async fn mixes_that_cannot_be_read_are_empty_and_fail_no_home() {
        let source = Failing {
            artists: Artists::Answer,
            mixes: Mixes::Unreachable,
            signed_in: true,
        };
        let mut request = mixes_only(LibraryHomeListMode::Forward);
        request.queries.extend(asks().queries);
        let response = complex_query(&source, &request).await.unwrap();
        assert_eq!(response.result[0].title, "Mixes");
        assert!(response.result[0].entries.is_empty());
        assert_eq!(response.result[2].entries[0].name, "Artist");
        // They count as no answer of the service, so they hide no failure of the others.
        let source = Failing {
            artists: Artists::Unreachable,
            mixes: Mixes::Unreachable,
            signed_in: true,
        };
        let error = complex_query(&source, &request).await.err().unwrap();
        assert!(format!("{error:#}").contains("unreachable"));
    }

    #[tokio::test]
    async fn a_sign_in_failure_on_the_mixes_fails_the_whole_request_unless_none_is_held() {
        let mut request = asks();
        request
            .queries
            .extend(mixes_only(LibraryHomeListMode::Forward).queries);
        let source = Failing {
            artists: Artists::Answer,
            mixes: Mixes::SignedOut,
            signed_in: true,
        };
        let error = complex_query(&source, &request).await.err().unwrap();
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::ServiceSignInRequired { .. })
        ));
        // A source that holds no sign-in yet answers what it kept, without the mixes.
        let source = Failing {
            artists: Artists::Answer,
            mixes: Mixes::SignedOut,
            signed_in: false,
        };
        let response = complex_query(&source, &request).await.unwrap();
        assert_eq!(response.result[1].entries[0].name, "Artist");
        assert!(response.result[2].entries.is_empty());
    }

    #[test]
    fn the_seed_holds_for_a_while_and_then_changes() {
        assert_eq!(rotated_seed(5, 0), rotated_seed(5, SEED_SECONDS - 1));
        assert_ne!(rotated_seed(5, 0), rotated_seed(5, SEED_SECONDS));
        let now = picked(
            numbers(100),
            LibraryHomeListMode::Random,
            rotated_seed(5, 0),
        );
        let later = picked(
            numbers(100),
            LibraryHomeListMode::Random,
            rotated_seed(5, SEED_SECONDS),
        );
        assert_ne!(now, later);
    }
}
