//! What the library keeps between calls, and how sign-ins and age retire it.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync;

use super::catalog::CatalogFound;
use super::types::{
    Album, CatalogAlbumDetail, CatalogArtistDetail, CatalogArtistPage, CatalogPlaylistDetail,
    RecommendationGroup, SongList, StorefrontLanguages, TrackKey,
};

/// How long the song list, the album list and each rating are kept, so changes made in
/// the Apple Music app or on another device show up.
pub(super) const TTL: Duration = Duration::from_secs(10 * 60);

/// How long a catalog album, artist or playlist is kept.
pub(super) const DETAIL_TTL: Duration = Duration::from_secs(30);

/// How long what a search found in the catalog is kept, so a search repeated meanwhile asks
/// the catalog nothing.
pub(super) const SEARCH_TTL: Duration = Duration::from_secs(2 * 60);

/// How long after a failed refresh in the background no other starts.
pub(super) const REFRESH_RETRY: Duration = Duration::from_secs(60);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
pub(super) struct Caches {
    /// Counts sign-ins with new tokens; reads that started before one are not kept.
    session: AtomicU64,
    /// Set once the lists kept on disk were read into the slots.
    pub(super) restored: sync::OnceCell<()>,
    pub(super) songs: Slot<SongList>,
    pub(super) albums: Slot<Vec<Album>>,
    /// Apple rating by song, 0 for a song known to be unrated.
    ratings: Mutex<HashMap<TrackKey, Rated>>,
    /// The user's recommendation groups, without their library marks.
    pub(super) recommendations: Slot<Vec<RecommendationGroup>>,
    /// The names of the library's artists.
    pub(super) artists: Slot<ArtistNames>,
    pub(super) album_details: Details<CatalogAlbumDetail>,
    pub(super) playlist_details: Details<CatalogPlaylistDetail>,
    pub(super) artist_details: Details<CatalogArtistDetail>,
    /// The first pages of a catalog artist's release views and top songs, without their
    /// library marks.
    pub(super) artist_pages: Details<CatalogArtistPage>,
    /// The languages of each storefront, by storefront id. They do not change with a sign-in.
    pub(super) storefront_languages: Mutex<HashMap<String, Arc<StorefrontLanguages>>>,
    /// What catalog searches found, by their term as `search_key` gives it, kept for
    /// [`SEARCH_TTL`].
    pub(super) searches: Details<CatalogFound>,
}

/// The names of the library's artists, by name key.
#[derive(Clone, Default)]
pub(super) struct ArtistNames {
    /// Each library artist's whole name.
    pub(super) whole: HashSet<String>,
    /// Each artist the names credit, a name of several artists split into theirs.
    pub(super) credited: HashSet<String>,
}

/// Catalog pages kept for [`DETAIL_TTL`], by catalog id, or what catalog searches found, kept
/// for [`SEARCH_TTL`] by term, each with the sign-in it was read under.
pub(super) struct Details<T>(Mutex<HashMap<String, KeptDetail<T>>>);

/// The sign-in, the time of the read and the page.
type KeptDetail<T> = (u64, Instant, Arc<T>);

impl<T> Default for Details<T> {
    fn default() -> Self {
        Self(Mutex::default())
    }
}

impl<T> Details<T> {
    /// The page kept under `id` when it was read under `session` and is younger than
    /// [`DETAIL_TTL`] at `now`.
    pub(super) fn fresh(&self, id: &str, session: u64, now: Instant) -> Option<Arc<T>> {
        self.fresh_for(id, session, now, DETAIL_TTL)
    }

    /// The value kept under `id` when it was read under `session` and is younger than `ttl`
    /// at `now`.
    pub(super) fn fresh_for(
        &self,
        id: &str,
        session: u64,
        now: Instant,
        ttl: Duration,
    ) -> Option<Arc<T>> {
        let kept = lock(&self.0);
        let (kept_session, at, value) = kept.get(id)?;
        (*kept_session == session && now.saturating_duration_since(*at) < ttl)
            .then(|| value.clone())
    }

    fn clear(&self) {
        lock(&self.0).clear();
    }
}

#[derive(Clone, Copy)]
struct Rated {
    value: i8,
    at: Instant,
}

impl Rated {
    pub(super) fn fresh(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.at) < TTL
    }
}

impl Caches {
    pub(super) fn session(&self) -> u64 {
        self.session.load(Ordering::Acquire)
    }

    /// Forgets everything of the previous sign-in, including reads still in flight.
    pub(super) fn start_session(&self) {
        let mut ratings = lock(&self.ratings);
        self.session.fetch_add(1, Ordering::AcqRel);
        self.songs.invalidate();
        self.albums.invalidate();
        self.recommendations.invalidate();
        self.artists.invalidate();
        ratings.clear();
        drop(ratings);
        // Cleared after the new session began, so a page being kept is checked against it.
        self.album_details.clear();
        self.playlist_details.clear();
        self.artist_details.clear();
        self.artist_pages.clear();
        self.searches.clear();
    }

    /// Keeps a catalog page read at `now`, unless a sign-in happened since `session`.
    /// Pages that have grown old are dropped meanwhile.
    pub(super) fn keep_detail<T>(
        &self,
        session: u64,
        details: fn(&Caches) -> &Details<T>,
        id: &str,
        now: Instant,
        value: Arc<T>,
    ) {
        self.keep_for(session, details(self), id, now, value, DETAIL_TTL);
    }

    /// Keeps what a catalog search for the term `key` found at `now`, unless a sign-in
    /// happened since `session`. Searches that have grown old are dropped meanwhile.
    pub(super) fn keep_search(
        &self,
        session: u64,
        key: &str,
        now: Instant,
        value: Arc<CatalogFound>,
    ) {
        self.keep_for(session, &self.searches, key, now, value, SEARCH_TTL);
    }

    fn keep_for<T>(
        &self,
        session: u64,
        details: &Details<T>,
        id: &str,
        now: Instant,
        value: Arc<T>,
        ttl: Duration,
    ) {
        let mut kept = lock(&details.0);
        if self.session() != session {
            return;
        }
        kept.retain(|_, (kept_session, at, _)| {
            *kept_session == session && now.saturating_duration_since(*at) < ttl
        });
        kept.insert(id.to_owned(), (session, now, value));
    }

    /// The kept rating of each of `keys` that has one younger than [`TTL`] at `now`.
    pub(super) fn rated(&self, keys: &[TrackKey], now: Instant) -> HashMap<TrackKey, i8> {
        let ratings = lock(&self.ratings);
        keys.iter()
            .filter_map(|key| {
                let rated = ratings.get(key).filter(|rated| rated.fresh(now))?;
                Some((key.clone(), rated.value))
            })
            .collect()
    }

    /// Keeps ratings that were read, unless a sign-in happened since `session`. A rating
    /// kept meanwhile is newer than the read, so it stays.
    pub(super) fn merge_read_ratings(
        &self,
        session: u64,
        now: Instant,
        read: &HashMap<TrackKey, i8>,
    ) {
        let mut ratings = lock(&self.ratings);
        if self.session() != session {
            return;
        }
        // What is too old to be served is not worth its memory.
        ratings.retain(|_, rated| rated.fresh(now));
        for (key, value) in read {
            match ratings.get(key) {
                Some(kept) if kept.fresh(now) => {}
                _ => {
                    ratings.insert(
                        key.clone(),
                        Rated {
                            value: *value,
                            at: now,
                        },
                    );
                }
            }
        }
    }
}

/// A value built on demand and kept for [`TTL`]. One build runs at a time, and the
/// kept value stays readable while it does.
pub(super) struct Slot<T> {
    /// Bumped to retire the kept value and any build in flight.
    epoch: AtomicU64,
    value: Mutex<Option<Cached<T>>>,
    building: sync::Mutex<()>,
    /// Whether a refresh in the background is running.
    refreshing: AtomicBool,
    /// Whether the kept value was served while no refresh could start, signed out, so the
    /// next sign-in refreshes it.
    owed: AtomicBool,
    /// Counts failed first builds, so a caller that queued behind one can tell.
    build_failures: AtomicU64,
    /// What the last first build failed with, and the epoch it was built under, handed to the
    /// callers of that epoch that queued behind it.
    build_failure: Mutex<Option<(u64, Arc<dyn Any + Send + Sync>)>>,
    /// How the last refresh in the background failed, until one works or a sign-in retires it.
    refresh_failure: Mutex<Option<RefreshFailure>>,
}

/// A refresh in the background that failed.
#[derive(Clone, Copy)]
pub(super) struct RefreshFailure {
    epoch: u64,
    at: Instant,
    /// Whether Apple rejected the tokens, which the kept list can no longer be served over.
    pub(super) rejected: bool,
}

struct Cached<T> {
    epoch: u64,
    /// When it was built; never for a value restored from disk, which is old from the start.
    built: Option<Instant>,
    value: Arc<T>,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            epoch: AtomicU64::new(0),
            value: Mutex::new(None),
            building: sync::Mutex::new(()),
            refreshing: AtomicBool::new(false),
            owed: AtomicBool::new(false),
            build_failures: AtomicU64::new(0),
            build_failure: Mutex::new(None),
            refresh_failure: Mutex::new(None),
        }
    }
}

impl<T: Clone> Slot<T> {
    /// The kept value when it is current and younger than `ttl` at `now`, else one
    /// built by `build`. A failed build keeps nothing; the callers that queued behind it
    /// get its failure instead of building again.
    pub(super) async fn get<E, F, Fut>(
        &self,
        now: Instant,
        ttl: Duration,
        build: F,
    ) -> Result<Arc<T>, Arc<E>>
    where
        E: Send + Sync + 'static,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        if let Some(value) = self.fresh(now, ttl) {
            return Ok(value);
        }
        let failures = self.build_failures.load(Ordering::Acquire);
        let _building = self.building.lock().await;
        if let Some(value) = self.fresh(now, ttl) {
            return Ok(value);
        }
        if self.build_failures.load(Ordering::Acquire) != failures {
            let failure = lock(&self.build_failure).clone();
            let epoch = self.epoch();
            if let Some(failure) = failure
                .filter(|(failed, _)| *failed == epoch)
                .and_then(|(_, failure)| failure.downcast::<E>().ok())
            {
                return Err(failure);
            }
        }
        let epoch = self.epoch();
        match self.build(now, build).await {
            Ok((value, _)) => Ok(value),
            Err(error) => {
                let error = Arc::new(error);
                *lock(&self.build_failure) = Some((epoch, error.clone()));
                self.build_failures.fetch_add(1, Ordering::AcqRel);
                Err(error)
            }
        }
    }

    /// Builds the value again however young the kept one is, and keeps it. `None` when a
    /// sign-in or a write retired the build meanwhile, so nothing was kept.
    pub(super) async fn rebuild<E, F, Fut>(
        &self,
        now: Instant,
        build: F,
    ) -> Result<Option<Arc<T>>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let _building = self.building.lock().await;
        let (value, kept) = self.build(now, build).await?;
        Ok(kept.then_some(value))
    }

    /// Builds the value and keeps it as built at `now`, unless the slot was retired
    /// meanwhile; says whether it was kept. The caller holds `building`.
    async fn build<E, F, Fut>(&self, now: Instant, build: F) -> Result<(Arc<T>, bool), E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let epoch = self.epoch.load(Ordering::Acquire);
        let value = Arc::new(build().await?);
        let mut kept = lock(&self.value);
        // A retired build must not replace what was kept since.
        if self.epoch.load(Ordering::Acquire) != epoch {
            return Ok((value, false));
        }
        *kept = Some(Cached {
            epoch,
            built: Some(now),
            value: value.clone(),
        });
        Ok((value, true))
    }

    pub(super) fn fresh(&self, now: Instant, ttl: Duration) -> Option<Arc<T>> {
        let value = lock(&self.value);
        let cached = value.as_ref()?;
        (cached.epoch == self.epoch.load(Ordering::Acquire)
            && cached
                .built
                .is_some_and(|built| now.saturating_duration_since(built) < ttl))
        .then(|| cached.value.clone())
    }

    /// Keeps a value restored from disk under `epoch`, which the caller took before it
    /// read the value, as old. Only an empty slot that nothing retired meanwhile takes it.
    pub(super) fn restore(&self, epoch: u64, value: T) {
        let mut kept = lock(&self.value);
        let current = self.epoch.load(Ordering::Acquire);
        if current != epoch || kept.as_ref().is_some_and(|cached| cached.epoch == current) {
            return;
        }
        *kept = Some(Cached {
            epoch,
            built: None,
            value: Arc::new(value),
        });
    }

    /// Records that the kept value was served while no refresh could start.
    pub(super) fn owe_refresh(&self) {
        self.owed.store(true, Ordering::Release);
    }

    /// Whether a refresh is owed, which it no longer is.
    pub(super) fn take_owed(&self) -> bool {
        self.owed.swap(false, Ordering::AcqRel)
    }

    /// The kept value of the current epoch, however old.
    pub(super) fn peek(&self) -> Option<Arc<T>> {
        let value = lock(&self.value);
        let cached = value.as_ref()?;
        (cached.epoch == self.epoch.load(Ordering::Acquire)).then(|| cached.value.clone())
    }

    /// Records that a refresh in the background that began under `epoch` failed at `now`, and
    /// whether Apple rejected the tokens. After a sign-in retired the slot, it counts for nothing.
    pub(super) fn refresh_failed(&self, epoch: u64, now: Instant, rejected: bool) {
        *lock(&self.refresh_failure) = Some(RefreshFailure {
            epoch,
            at: now,
            rejected,
        });
    }

    /// Forgets a failed refresh, as a refresh that worked does.
    pub(super) fn refresh_worked(&self) {
        *lock(&self.refresh_failure) = None;
    }

    /// The last failed refresh, unless a sign-in retired the slot since.
    pub(super) fn last_refresh_failure(&self) -> Option<RefreshFailure> {
        (*lock(&self.refresh_failure)).filter(|failure| failure.epoch == self.epoch())
    }

    /// Whether a refresh failed less than [`REFRESH_RETRY`] before `now`.
    pub(super) fn refresh_cooling(&self, now: Instant) -> bool {
        self.last_refresh_failure()
            .is_some_and(|failure| now.saturating_duration_since(failure.at) < REFRESH_RETRY)
    }

    /// Claims the right to refresh in the background; false when one is running.
    pub(super) fn claim_refresh(&self) -> bool {
        !self.refreshing.swap(true, Ordering::AcqRel)
    }

    pub(super) fn end_refresh(&self) {
        self.refreshing.store(false, Ordering::Release);
    }

    pub(super) fn invalidate(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }

    /// The epoch to take before a read whose result is kept.
    pub(super) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// Keeps `value` as built at `now` under `epoch`, which the caller took before it read
    /// what the value is made of. After an invalidation in between, the value is already
    /// stale and counts as nothing kept.
    #[cfg(test)]
    pub(super) fn put(&self, epoch: u64, now: Instant, value: T) {
        *lock(&self.value) = Some(Cached {
            epoch,
            built: Some(now),
            value: Arc::new(value),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::atomic::AtomicU32;

    use super::*;
    use tokio::time;

    async fn build(slot: &Slot<u32>, now: Instant, builds: &AtomicU32) -> u32 {
        let value = slot
            .get::<Infallible, _, _>(now, TTL, || async {
                Ok(builds.fetch_add(1, Ordering::SeqCst) + 1)
            })
            .await;
        *value.unwrap()
    }

    #[tokio::test]
    async fn kept_values_expire_after_the_ttl() {
        let slot = Slot::default();
        let builds = AtomicU32::new(0);
        let start = Instant::now();

        assert_eq!(build(&slot, start, &builds).await, 1);
        assert_eq!(build(&slot, start + TTL / 2, &builds).await, 1);
        assert_eq!(build(&slot, start + TTL, &builds).await, 2);
        // The rebuilt value counts its age from its own build.
        assert_eq!(build(&slot, start + TTL + TTL / 2, &builds).await, 2);
        assert_eq!(builds.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_value_put_after_an_invalidation_under_an_older_epoch_is_stale() {
        let slot = Slot::<u32>::default();
        let start = Instant::now();

        let epoch = slot.epoch();
        slot.put(epoch, start, 1);
        assert_eq!(slot.fresh(start, TTL).as_deref(), Some(&1));

        // A sign-in retires the slot between a read and the store of what it read.
        let epoch = slot.epoch();
        slot.invalidate();
        slot.put(epoch, start, 2);
        assert_eq!(slot.fresh(start, TTL), None);
        assert_eq!(slot.peek(), None);
        // One taken after it counts.
        slot.put(slot.epoch(), start, 3);
        assert_eq!(slot.fresh(start, TTL).as_deref(), Some(&3));
    }

    #[tokio::test]
    async fn invalidation_retires_the_kept_value() {
        let slot = Slot::default();
        let builds = AtomicU32::new(0);
        let start = Instant::now();

        assert_eq!(build(&slot, start, &builds).await, 1);
        slot.invalidate();
        assert_eq!(slot.peek(), None);
        assert_eq!(build(&slot, start, &builds).await, 2);
    }

    #[tokio::test]
    async fn restored_values_are_old_and_retired_rebuilds_keep_nothing() {
        let slot = Slot::<u32>::default();
        let start = Instant::now();

        slot.restore(slot.epoch(), 1);
        assert_eq!(slot.peek().as_deref(), Some(&1));
        assert_eq!(slot.fresh(start, TTL), None);
        // Only an empty slot takes a restored value.
        slot.restore(slot.epoch(), 2);
        assert_eq!(slot.peek().as_deref(), Some(&1));

        // A sign-in retires a rebuild in flight, and a value kept since stays.
        let rebuilt = slot
            .rebuild::<Infallible, _, _>(start, || async {
                slot.invalidate();
                slot.put(slot.epoch(), start, 3);
                Ok(4)
            })
            .await;
        assert_eq!(rebuilt.unwrap(), None);
        assert_eq!(slot.peek().as_deref(), Some(&3));
        let rebuilt = slot
            .rebuild::<Infallible, _, _>(start, || async { Ok(5) })
            .await;
        assert_eq!(rebuilt.unwrap().as_deref(), Some(&5));
        assert_eq!(slot.fresh(start, TTL).as_deref(), Some(&5));

        // A value restored under an epoch retired meanwhile is not kept.
        let epoch = slot.epoch();
        slot.invalidate();
        slot.restore(epoch, 6);
        assert_eq!(slot.peek(), None);
    }

    #[tokio::test]
    async fn callers_queued_behind_a_failed_build_share_its_failure() {
        let slot = Arc::new(Slot::<u32>::default());
        let builds = Arc::new(AtomicU32::new(0));
        let start = Instant::now();
        let callers: Vec<_> = (0..5)
            .map(|_| {
                let (slot, builds) = (slot.clone(), builds.clone());
                tokio::spawn(async move {
                    slot.get(start, TTL, || async {
                        builds.fetch_add(1, Ordering::SeqCst);
                        time::sleep(Duration::from_millis(100)).await;
                        Err::<u32, _>("down".to_owned())
                    })
                    .await
                })
            })
            .collect();
        for caller in callers {
            assert_eq!(*caller.await.unwrap().unwrap_err(), "down");
        }
        assert_eq!(builds.load(Ordering::SeqCst), 1);

        // A caller that comes later builds again.
        let later = slot
            .get::<String, _, _>(start, TTL, || async { Ok(7) })
            .await;
        assert_eq!(*later.unwrap(), 7);
    }

    #[tokio::test]
    async fn a_failed_refresh_is_remembered_until_a_sign_in_retires_it() {
        let slot = Slot::<u32>::default();
        let start = Instant::now();
        assert!(slot.last_refresh_failure().is_none());

        slot.refresh_failed(slot.epoch(), start, true);
        assert!(slot.refresh_cooling(start + REFRESH_RETRY / 2));
        assert!(!slot.refresh_cooling(start + REFRESH_RETRY));
        assert!(
            slot.last_refresh_failure()
                .is_some_and(|failure| failure.rejected)
        );

        slot.refresh_worked();
        assert!(slot.last_refresh_failure().is_none());
        slot.refresh_failed(slot.epoch(), start, true);
        slot.invalidate();
        assert!(slot.last_refresh_failure().is_none());

        // A refresh that began before a sign-in does not fail the slot of the next one.
        let epoch = slot.epoch();
        slot.invalidate();
        slot.refresh_failed(epoch, start, true);
        assert!(slot.last_refresh_failure().is_none());
        assert!(!slot.refresh_cooling(start));
    }

    #[tokio::test]
    async fn a_failed_build_is_not_shared_with_a_caller_of_the_next_sign_in() {
        let slot = Arc::new(Slot::<u32>::default());
        let start = Instant::now();
        let first = {
            let slot = slot.clone();
            tokio::spawn(async move {
                slot.get(start, TTL, || async {
                    time::sleep(Duration::from_millis(100)).await;
                    Err::<u32, _>("old account".to_owned())
                })
                .await
            })
        };
        time::sleep(Duration::from_millis(20)).await;
        slot.invalidate();
        let second = slot
            .get::<String, _, _>(start, TTL, || async { Ok(7) })
            .await;

        assert_eq!(*first.await.unwrap().unwrap_err(), "old account");
        assert_eq!(*second.unwrap(), 7);
    }

    #[test]
    fn ratings_too_old_to_be_served_are_dropped_when_others_are_kept() {
        let caches = Caches::default();
        let start = Instant::now();
        let key = |id: &str| TrackKey::catalog(id).unwrap();
        let session = caches.session();
        caches.merge_read_ratings(session, start, &HashMap::from([(key("1"), 0)]));
        caches.merge_read_ratings(session, start + TTL, &HashMap::from([(key("2"), 1)]));

        assert_eq!(lock(&caches.ratings).len(), 1);
        assert_eq!(caches.rated(&[key("2")], start + TTL).len(), 1);
    }
}
