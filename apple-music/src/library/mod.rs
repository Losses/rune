//! The user's Apple Music library and the catalog songs around it, through a neutral
//! API: plain types in, plain types out, with the library's own cache.
//!
//! Sign-in is explicit: a host hands over the developer token and the Music User Token
//! with [`AppleLibrary::sign_in`], and they live in memory only. Until then, and whenever
//! Apple rejects them, calls fail with [`LibraryError::SignedOut`].
//!
//! Songs are named by [`TrackKey`]: library songs by Apple's library id (`i.…`), catalog
//! songs as `catalog:<id>`. Library albums, artists and playlists keep Apple's library ids.
//!
//! The catalog sits beside the library: [`AppleLibrary::search`] merges both, catalog
//! albums, artists and playlists open by [`ItemId::Catalog`] ids.
//!
//! The song list, the album list and the recommendations are kept for a while, served at
//! once while they are refreshed in the background once old, and, with a [`ListStore`],
//! kept on disk across restarts (see [`AppleLibrary::with_store`]).

mod cache;
mod catalog;
mod map;
mod store;
mod types;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::result;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::future::{BoxFuture, join, try_join};
use futures_util::{FutureExt, StreamExt, TryStreamExt, stream};
use serde::Serialize;
use tokio::runtime;
use tokio::task;

use crate::ID_BATCH;
use crate::client::{AppleMusicClient, ClientBuilder};
use crate::error::Error;
use crate::models::{LibraryEntry, LibraryPlaylist, LibrarySong, Storefront};
use crate::tokens::TokenSource;
use cache::{Caches, Slot, TTL};
pub use catalog::{DEFAULT_RECOMMENDATIONS, SEARCH_WAIT};
pub use store::{ListStore, StoreGeneration};
use types::is_library_id;
pub use types::{
    Album, AlbumDetail, AlbumItem, Artist, ArtistDetail, ArtistItem, ArtistRelease, Artwork,
    CatalogAlbumDetail, CatalogAlbumFacts, CatalogArtistDetail, CatalogArtistPage,
    CatalogPlaylistDetail, CatalogSongRelations, ItemId, ParseItemIdError, ParseTrackKeyError,
    Playlist, PlaylistItem, RecommendationGroup, RecommendationKind, Recommended, RecommendedItem,
    ReleasePage, ReleaseView, SearchResults, SearchScope, SongList, SongPage, StorefrontLanguages,
    Track, TrackKey,
};

/// Relationships read with library songs, for their album and artist ids.
const SONG_INCLUDE: &[&str] = &["albums", "artists"];
/// Library pages, playlists or batches of ratings read at once.
const PARALLEL_READS: usize = 4;

/// What a call could not find.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// A song.
    Song,
    /// An album.
    Album,
    /// An artist.
    Artist,
    /// A playlist.
    Playlist,
}

impl fmt::Display for Missing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Song => "song",
            Self::Album => "album",
            Self::Artist => "artist",
            Self::Playlist => "playlist",
        })
    }
}

/// Why a call failed.
#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    /// No tokens were handed over, or Apple rejects them: the user has to sign in again.
    #[error("sign in to Apple Music")]
    SignedOut,
    /// The tokens changed twice while the call ran, so it has no answer that belongs to
    /// one sign-in. Asking again is safe.
    #[error("the Apple Music sign-in changed during the call")]
    SessionChanged,
    /// Apple refused this request though the tokens work, as it does for writes its
    /// policy or the subscription forbids.
    #[error("Apple Music refused the request")]
    Refused,
    /// The thing asked for is not in the library or the catalog.
    #[error("Apple Music {0} not found")]
    NotFound(Missing),
    /// Apple Music or the network cannot answer now: rate limited, a server error that
    /// outlasted the retries, or a transport failure. Trying later may work.
    #[error("Apple Music is unavailable: {0}")]
    Unavailable(Error),
    /// Any other failure of the Apple Music API.
    #[error(transparent)]
    Apple(Error),
    /// Apple Music answered none of the parts of a call in the time it allows, as a search
    /// does (see [`SEARCH_WAIT`]). Trying again may work.
    #[error("Apple Music did not answer in time")]
    TimedOut,
}

impl LibraryError {
    /// Another error that says the same, for callers that share one failure.
    fn echo(&self) -> Self {
        match self {
            Self::SignedOut => Self::SignedOut,
            Self::SessionChanged => Self::SessionChanged,
            Self::Refused => Self::Refused,
            Self::NotFound(missing) => Self::NotFound(*missing),
            Self::Unavailable(error) => Self::Unavailable(error.echo()),
            Self::Apple(error) => Self::Apple(error.echo()),
            Self::TimedOut => Self::TimedOut,
        }
    }
}

/// What a library call returns.
pub type Result<T> = result::Result<T, LibraryError>;

/// A failure inside a call. Apple's are mapped once the call is over, when a rejected
/// token can be told from a refused request.
enum Failure {
    Apple(Error),
    Library(LibraryError),
    /// A failure several callers share, as the ones that waited for the same build do.
    Shared(Arc<Failure>),
}

impl Failure {
    /// The failure itself, whoever shares it.
    fn root(&self) -> &Self {
        match self {
            Self::Shared(shared) => shared.root(),
            other => other,
        }
    }
}

impl From<Arc<Failure>> for Failure {
    fn from(failure: Arc<Failure>) -> Self {
        Self::Shared(failure)
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Apple(error)
    }
}

impl From<LibraryError> for Failure {
    fn from(error: LibraryError) -> Self {
        Self::Library(error)
    }
}

type Outcome<T> = result::Result<T, Failure>;

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Apple(error) => error.fmt(f),
            Self::Library(error) => error.fmt(f),
            Self::Shared(shared) => shared.fmt(f),
        }
    }
}

/// The time the caches age by. Tests replace it to move time without waiting.
#[doc(hidden)]
pub trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> Instant;
}

/// The system's monotonic clock.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A token is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("both the developer token and the Music User Token are needed")]
pub struct IncompleteTokens;

/// The two tokens of a sign-in.
#[derive(Clone, PartialEq, Eq)]
pub struct Tokens {
    developer: String,
    user: String,
}

impl Tokens {
    /// The developer token and the Music User Token. Both must be given.
    pub fn new(
        developer: impl Into<String>,
        user: impl Into<String>,
    ) -> result::Result<Self, IncompleteTokens> {
        let (developer, user) = (developer.into(), user.into());
        if developer.is_empty() || user.is_empty() {
            return Err(IncompleteTokens);
        }
        Ok(Self { developer, user })
    }
}

impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens")
            .field("developer", &"<redacted>")
            .field("user", &"<redacted>")
            .finish()
    }
}

/// What a sign-in did to what the library kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInChange {
    /// The same tokens as before: nothing was forgotten.
    Unchanged,
    /// The first tokens the library holds since it was made. Nothing was read before them;
    /// what was restored from disk stays, and what of it was served is refreshed in the
    /// background.
    First,
    /// New tokens, which may belong to another account: everything kept was dropped, on
    /// disk too.
    Changed,
}

/// One of the lists the library keeps, refreshes in the background once old and keeps on
/// disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptList {
    /// The song list.
    Songs,
    /// The album list.
    Albums,
    /// The recommendation groups.
    Recommendations,
}

impl KeptList {
    /// The name the list is saved as in the store.
    fn name(self) -> &'static str {
        match self {
            Self::Songs => "songs",
            Self::Albums => "albums",
            Self::Recommendations => "recommendations",
        }
    }
}

/// Hears that a refresh in the background changed a list.
type ChangeListener = Arc<dyn Fn(KeptList) + Send + Sync>;

/// How the library keeps one of its lists: where, and how it is built.
struct ListSpec<T: 'static> {
    list: KeptList,
    slot: fn(&Caches) -> &Slot<T>,
    build: fn(AppleLibrary) -> BoxFuture<'static, Outcome<T>>,
}

const SONGS: ListSpec<SongList> = ListSpec {
    list: KeptList::Songs,
    slot: |caches| &caches.songs,
    build: |library| async move { library.build_song_list().await }.boxed(),
};

const ALBUMS: ListSpec<Vec<Album>> = ListSpec {
    list: KeptList::Albums,
    slot: |caches| &caches.albums,
    build: |library| async move { library.build_album_list().await }.boxed(),
};

const RECOMMENDATIONS: ListSpec<Vec<RecommendationGroup>> = ListSpec {
    list: KeptList::Recommendations,
    slot: |caches| &caches.recommendations,
    build: |library| async move { library.build_recommendations().await }.boxed(),
};

/// The user's songs, albums, artists and playlists from Apple Music.
///
/// Clones are handles to the same sign-in and caches.
#[derive(Clone)]
pub struct AppleLibrary {
    client: AppleMusicClient,
    tokens: Arc<SessionTokens>,
    caches: Arc<Caches>,
    clock: Arc<dyn Clock>,
    /// The names of the artists a library artist's name credits.
    split_artists: fn(&str) -> Vec<String>,
    /// Where the kept lists are stored across restarts.
    store: Option<ListStore>,
    listener: Option<ChangeListener>,
    /// How long a search waits for the library and the catalog (see [`SEARCH_WAIT`]).
    search_wait: Duration,
}

impl fmt::Debug for AppleLibrary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppleLibrary")
            .field("signed_in", &self.tokens.signed_in())
            .finish_non_exhaustive()
    }
}

impl Default for AppleLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl AppleLibrary {
    /// A library for the production Apple Music API, signed out.
    pub fn new() -> Self {
        Self::with_client(|builder| builder)
    }

    /// A library whose client is configured by `configure`, for another host, retry
    /// policy or HTTP client. Its tokens always come from [`sign_in`](Self::sign_in).
    pub fn with_client(configure: impl FnOnce(ClientBuilder) -> ClientBuilder) -> Self {
        let tokens = Arc::new(SessionTokens::default());
        let client = configure(AppleMusicClient::builder(tokens.clone())).build();
        Self {
            client,
            tokens,
            caches: Arc::default(),
            clock: Arc::new(SystemClock),
            split_artists: one_artist,
            store: None,
            listener: None,
            search_wait: SEARCH_WAIT,
        }
    }

    /// Waits `wait` for a search instead of [`SEARCH_WAIT`].
    #[doc(hidden)]
    #[must_use]
    pub fn with_search_wait(mut self, wait: Duration) -> Self {
        self.search_wait = wait;
        self
    }

    /// Ages what is kept by `clock` instead of the system's.
    #[doc(hidden)]
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Keeps the song list, the album list and the recommendations in `store` across
    /// restarts. A new library reads them from it, and answers from them at once, even before
    /// its first sign-in; they count as old, so they are refreshed in the background as soon
    /// as they are served signed in. Signing out, or in with other tokens, deletes what the
    /// store holds.
    #[must_use]
    pub fn with_store(mut self, store: ListStore) -> Self {
        self.store = Some(store);
        self
    }

    /// Calls `listener` whenever a refresh in the background brings a list that differs from
    /// the one served before it. It is called on the task of the refresh, and must not block.
    #[must_use]
    pub fn with_change_listener(
        mut self,
        listener: impl Fn(KeptList) + Send + Sync + 'static,
    ) -> Self {
        self.listener = Some(Arc::new(listener));
        self
    }

    /// Hands over the tokens of a sign-in. The same tokens as before keep what the
    /// library has read, and so do the first tokens it holds, which keep what it restored;
    /// new ones, which may belong to another account, drop it, and reads still in flight
    /// from the old sign-in are not kept.
    pub fn sign_in(&self, tokens: Tokens) -> SignInChange {
        match self.tokens.replace(Some(tokens)) {
            Replaced::Same => SignInChange::Unchanged,
            Replaced::First => {
                self.refresh_owed();
                SignInChange::First
            }
            Replaced::Other => {
                // The storefront first: a read that starts after the new session began must
                // not find the old account's.
                self.client.forget_storefront();
                self.caches.start_session();
                // After the session, so a list read for the old one is no longer saved.
                self.clear_store();
                SignInChange::Changed
            }
        }
    }

    /// Whether the library holds tokens. It says nothing about whether Apple still accepts
    /// them; [`check`](Self::check) asks.
    pub fn is_signed_in(&self) -> bool {
        self.tokens.signed_in()
    }

    /// Forgets the tokens and everything kept, on disk too. Calls fail with
    /// [`LibraryError::SignedOut`] until the next sign-in.
    pub fn sign_out(&self) {
        if self.tokens.replace(None) != Replaced::Same {
            self.client.forget_storefront();
        }
        // Lists restored before any sign-in go too.
        self.caches.start_session();
        self.clear_store();
    }

    fn clear_store(&self) {
        if let Some(store) = &self.store {
            store.clear();
        }
    }

    /// Whether Apple accepts the tokens: the user's storefront, read with them.
    pub async fn check(&self) -> Result<Storefront> {
        if !self.tokens.signed_in() {
            return Err(LibraryError::SignedOut);
        }
        self.client.user_storefront().await.map_err(|error| {
            if rejects_tokens(&error) {
                LibraryError::SignedOut
            } else if matches!(error, Error::NotFound(_)) {
                LibraryError::Apple(error)
            } else {
                library_error(error, Missing::Song)
            }
        })
    }

    /// Runs a call that needs Apple once: the sign-in is checked before anything else,
    /// so every such call answers the same way without one.
    async fn signed_in<T, Fut>(&self, what: Missing, call: impl FnOnce() -> Fut) -> Result<T>
    where
        Fut: Future<Output = Outcome<T>>,
    {
        if !self.tokens.signed_in() {
            return Err(LibraryError::SignedOut);
        }
        match call().await {
            Ok(value) => Ok(value),
            Err(failure) => Err(match failure.root() {
                Failure::Library(error) => error.echo(),
                Failure::Apple(error) => {
                    // A shared failure is judged again by each caller, so it is copied.
                    self.apple_error(error.echo(), what).await
                }
                Failure::Shared(_) => unreachable!("root never answers a shared failure"),
            }),
        }
    }

    /// Like [`signed_in`](Self::signed_in) for a read. An answer that arrives after the
    /// tokens changed belongs to the old account, so the read is repeated under the new
    /// tokens, once.
    async fn signed_in_read<T, Fut>(&self, what: Missing, call: impl Fn() -> Fut) -> Result<T>
    where
        Fut: Future<Output = Outcome<T>>,
    {
        for _ in 0..2 {
            let session = self.caches.session();
            let result = self.signed_in(what, &call).await;
            if self.caches.session() == session {
                return result;
            }
        }
        Err(LibraryError::SessionChanged)
    }

    /// A 401 or 403 means an expired sign-in only when a plain read with the same tokens
    /// fails too; otherwise Apple refused just this request.
    async fn apple_error(&self, error: Error, what: Missing) -> LibraryError {
        if matches!(error, Error::Unauthorized | Error::Forbidden)
            && matches!(self.client.user_storefront().await, Err(ref again) if rejects_tokens(again))
        {
            return LibraryError::SignedOut;
        }
        library_error(error, what)
    }

    /// Whether `failure` means Apple rejects the sign-in, not just one request.
    async fn rejected_sign_in(&self, failure: &Failure) -> bool {
        match failure.root() {
            Failure::Library(error) => matches!(error, LibraryError::SignedOut),
            Failure::Apple(Error::Unauthorized | Error::Forbidden) => {
                matches!(self.client.user_storefront().await, Err(ref again) if rejects_tokens(again))
            }
            Failure::Apple(error) => rejects_tokens(error),
            Failure::Shared(_) => false,
        }
    }

    /// A kept list for a public call. Before the first sign-in, a list restored from disk
    /// answers, and is refreshed once the library is signed in.
    async fn kept_list<T>(&self, spec: &'static ListSpec<T>, what: Missing) -> Result<Arc<T>>
    where
        T: Clone + PartialEq + Serialize + Send + Sync + 'static,
    {
        if !self.tokens.signed_in() {
            self.restored().await;
            let slot = (spec.slot)(&self.caches);
            return match slot.peek() {
                Some(value) => {
                    slot.owe_refresh();
                    Ok(value)
                }
                None => Err(LibraryError::SignedOut),
            };
        }
        self.signed_in_read(what, || self.kept(spec)).await
    }

    /// The value of a slot: the kept one while it is young. Once it is old, a kept value
    /// is still returned at once and refreshed in the background; only the first build
    /// waits.
    async fn kept<T>(&self, spec: &'static ListSpec<T>) -> Outcome<Arc<T>>
    where
        T: Clone + PartialEq + Serialize + Send + Sync + 'static,
    {
        self.restored().await;
        let now = self.clock.now();
        let kept = (spec.slot)(&self.caches);
        if let Some(value) = kept.fresh(now, TTL) {
            return Ok(value);
        }
        if let Some(old) = kept.peek() {
            self.refresh_in_background(spec);
            // A list that cannot be refreshed because Apple rejects the sign-in is not
            // served as if it were current.
            if kept
                .last_refresh_failure()
                .is_some_and(|failure| failure.rejected)
            {
                return Err(Failure::Library(LibraryError::SignedOut));
            }
            return Ok(old);
        }
        let generation = self.store_generation();
        let value = kept.get(now, TTL, || (spec.build)(self.clone())).await?;
        if kept.peek().is_some_and(|kept| Arc::ptr_eq(&kept, &value)) {
            self.store_list(spec, generation);
        }
        Ok(value)
    }

    /// Starts a refresh of a kept list in the background, unless one runs. Signed out, the
    /// next sign-in starts it instead. A refresh that brings another list tells the change
    /// listener; one that fails keeps the old list, which is served until a later one works.
    fn refresh_in_background<T>(&self, spec: &'static ListSpec<T>)
    where
        T: Clone + PartialEq + Serialize + Send + Sync + 'static,
    {
        let slot = (spec.slot)(&self.caches);
        if !self.tokens.signed_in() {
            slot.owe_refresh();
            return;
        }
        if slot.refresh_cooling(self.clock.now()) || !slot.claim_refresh() {
            return;
        }
        let library = self.clone();
        tokio::spawn(async move {
            let _end = EndRefresh {
                caches: library.caches.clone(),
                slot: spec.slot,
            };
            let slot = (spec.slot)(&library.caches);
            let epoch = slot.epoch();
            let served = slot.peek();
            let generation = library.store_generation();
            let now = library.clock.now();
            match slot.rebuild(now, || (spec.build)(library.clone())).await {
                Ok(Some(value)) => {
                    slot.refresh_worked();
                    library.store_list(spec, generation);
                    if served.as_deref() != Some(&*value)
                        && let Some(listener) = &library.listener
                    {
                        listener(spec.list);
                    }
                }
                // A sign-in or a write retired it: it was not kept.
                Ok(None) => {}
                Err(failure) => {
                    log::warn!(
                        "Could not refresh the Apple Music {}: {}",
                        spec.list.name(),
                        failure
                    );
                    let rejected = library.rejected_sign_in(&failure).await;
                    slot.refresh_failed(epoch, library.clock.now(), rejected);
                }
            }
        });
    }

    /// Refreshes the lists served before the first sign-in. Outside a runtime nothing can be
    /// started, and they are refreshed when they are next served.
    fn refresh_owed(&self) {
        if runtime::Handle::try_current().is_err() {
            return;
        }
        if self.caches.songs.take_owed() {
            self.refresh_in_background(&SONGS);
        }
        if self.caches.albums.take_owed() {
            self.refresh_in_background(&ALBUMS);
        }
        if self.caches.recommendations.take_owed() {
            self.refresh_in_background(&RECOMMENDATIONS);
        }
    }

    /// Reads the lists kept on disk into their slots, once. A slot a sign-in or a write
    /// retired meanwhile, or one that holds a list already, keeps what it has.
    async fn restored(&self) {
        let Some(store) = &self.store else {
            return;
        };
        self.caches
            .restored
            .get_or_init(|| async {
                let caches = &self.caches;
                let epochs = (
                    caches.songs.epoch(),
                    caches.albums.epoch(),
                    caches.recommendations.epoch(),
                );
                let store = store.clone();
                let loaded = task::spawn_blocking(move || {
                    (
                        store.load::<SongList>(SONGS.list.name()),
                        store.load::<Vec<Album>>(ALBUMS.list.name()),
                        store.load::<Vec<RecommendationGroup>>(RECOMMENDATIONS.list.name()),
                    )
                })
                .await;
                let Ok((songs, albums, recommendations)) = loaded else {
                    return;
                };
                if let Some(songs) = songs {
                    caches.songs.restore(epochs.0, songs);
                }
                if let Some(albums) = albums {
                    caches.albums.restore(epochs.1, albums);
                }
                if let Some(recommendations) = recommendations {
                    caches.recommendations.restore(epochs.2, recommendations);
                }
            })
            .await;
    }

    /// The generation of the store to take before a read whose result is saved.
    fn store_generation(&self) -> Option<StoreGeneration> {
        self.store.as_ref().map(ListStore::generation)
    }

    /// Saves the kept list of `spec`, unless the store was cleared since `generation`.
    fn store_list<T>(&self, spec: &ListSpec<T>, generation: Option<StoreGeneration>)
    where
        T: Clone + Serialize + Send + Sync + 'static,
    {
        let (Some(store), Some(generation)) = (&self.store, generation) else {
            return;
        };
        if let Some(value) = (spec.slot)(&self.caches).peek() {
            store.save(generation, spec.list.name(), value);
        }
    }

    // All songs ------------------------------------------------------------

    /// The most complete list of the user's songs: library songs, then the songs of the
    /// user's own playlists and Favorites that are not in the library. Kept for ten
    /// minutes; the kept list answers [`track`](Self::track) meanwhile it is rebuilt.
    pub async fn songs(&self) -> Result<Arc<SongList>> {
        self.kept_list(&SONGS, Missing::Song).await
    }

    /// The song list as last read, however old, without asking Apple. None before the first
    /// read, and after a sign-in change or a sign-out.
    pub fn cached_songs(&self) -> Option<Arc<SongList>> {
        self.caches.songs.peek()
    }

    /// Like [`cached_songs`](Self::cached_songs), and before the first read the list kept on
    /// disk, if there is one. Nothing is asked of Apple.
    pub async fn kept_songs(&self) -> Option<Arc<SongList>> {
        self.restored().await;
        self.caches.songs.peek()
    }

    async fn build_song_list(&self) -> Outcome<SongList> {
        let mut list = SongList::default();
        // Library songs whose catalog id only their `catalog` relationship names.
        let mut unresolved = Vec::new();
        let library = async {
            let mut pages = self.client.library_song_pages(SONG_INCLUDE, PARALLEL_READS);
            while let Some(page) = pages.try_next().await? {
                for entry in &page {
                    list.extend(map::library_track(entry));
                    match (TrackKey::library(&entry.id), entry.store_id()) {
                        (Ok(key), Some(catalog_id)) => list.add_library_twin(catalog_id, &key),
                        (Ok(_), None) => unresolved.push(entry.id.clone()),
                        (Err(_), _) => {}
                    }
                }
            }
            Ok::<_, Error>(())
        };
        let ((), playlists) = try_join(
            library,
            self.client.library_playlists().try_collect::<Vec<_>>(),
        )
        .await?;

        // Owned ids: closures taking borrowed items here fail the compiler's Send check.
        let joining: Vec<String> = playlists
            .into_iter()
            .filter(map::joins_song_list)
            .map(|playlist| playlist.id)
            .collect();
        let mut tracks = stream::iter(joining)
            .map(|id| async move {
                match self.playlist_songs(&id).await {
                    Ok(tracks) => Ok(tracks
                        .iter()
                        .filter_map(map::library_track)
                        .collect::<Vec<_>>()),
                    Err(error) if rejects_tokens(&error) => Err(error),
                    // One playlist that cannot be read does not cost the whole song list.
                    Err(error) => {
                        log::warn!(
                            "Skipped an Apple Music playlist that could not be read: {error}"
                        );
                        Ok(Vec::new())
                    }
                }
            })
            .buffered(PARALLEL_READS);
        let mut extra = Vec::new();
        while let Some(songs) = tracks.try_next().await? {
            extra.extend(songs.into_iter().filter(|song| !list.lists(&song.key)));
        }
        drop(tracks);

        let catalog_copies = extra.iter().any(|song| !song.key.is_library());
        if catalog_copies && !unresolved.is_empty() {
            // Best effort: without these ids a catalog copy of such a song is listed too.
            if let Ok(songs) = self
                .client
                .library_songs_by_ids(&unresolved, &["catalog"])
                .await
            {
                for song in &songs {
                    if let (Ok(key), Some(catalog_id)) =
                        (TrackKey::library(&song.id), song.store_id())
                    {
                        list.add_library_twin(catalog_id, &key);
                    }
                }
            }
        }
        // Their catalog ids first, so the catalog copy of such a song is not listed too.
        for song in &extra {
            if let Some(catalog_id) = song.catalog_id.as_deref().filter(|_| song.key.is_library()) {
                list.add_library_twin(catalog_id, &song.key);
            }
        }
        list.extend(extra);
        Ok(list)
    }

    /// The raw tracks of a library playlist.
    async fn playlist_songs(&self, id: &str) -> result::Result<Vec<LibrarySong>, Error> {
        match self.client.library_playlist_tracks(id).try_collect().await {
            Ok(tracks) => Ok(tracks),
            // Apple answers 404 for the tracks of an empty playlist.
            Err(Error::NotFound(_)) => Ok(Vec::new()),
            Err(error) => Err(error),
        }
    }

    /// A song of the kept list, without asking Apple.
    fn known_song(&self, key: &TrackKey) -> Option<Track> {
        self.caches.songs.peek()?.get(key).cloned()
    }

    /// One song: from the kept song list when it is listed there, else read from Apple.
    pub async fn track(&self, key: &TrackKey) -> Result<Track> {
        self.signed_in_read(Missing::Song, || async {
            match self.known_song(key) {
                Some(song) => Ok(song),
                None => self.read_song(key).await,
            }
        })
        .await
    }

    async fn read_song(&self, key: &TrackKey) -> Outcome<Track> {
        let found = match (key.library_id(), key.catalog_id()) {
            (_, Some(catalog_id)) => self
                .client
                .catalog_songs(&[catalog_id.to_owned()])
                .await
                .map(|songs| {
                    songs.first().and_then(map::catalog_track).map(|mut song| {
                        song.in_library = self
                            .caches
                            .songs
                            .peek()
                            .is_some_and(|list| list.in_library(key));
                        song
                    })
                }),
            (Some(library_id), _) => match self
                .client
                .library_songs_by_ids(&[library_id.to_owned()], SONG_INCLUDE)
                .await
            {
                Ok(songs) => Ok(songs.iter().find_map(map::library_track)),
                Err(Error::NotFound(_)) => Ok(None),
                Err(error) => Err(error),
            },
            _ => Ok(None),
        };
        // The list may know a song the lookup missed, or have been built meanwhile.
        match found {
            Ok(Some(song)) => Ok(song),
            Ok(None) => self
                .known_song(key)
                .ok_or_else(|| LibraryError::NotFound(Missing::Song).into()),
            Err(error) => self.known_song(key).ok_or(Failure::Apple(error)),
        }
    }

    /// The ids of the catalog album and artist Apple names for the catalog song `key`, which a
    /// song found by a catalog search does not carry. Each is `None` when Apple names none, and
    /// both are when Apple knows no such song.
    pub async fn catalog_song_relations(&self, key: &TrackKey) -> Result<CatalogSongRelations> {
        let Some(catalog_id) = key.catalog_id() else {
            return Ok(CatalogSongRelations::default());
        };
        self.signed_in_read(Missing::Song, || async {
            let song = self.client.catalog_song_with_relations(catalog_id).await?;
            let song = song.as_ref().and_then(map::catalog_track);
            Ok(CatalogSongRelations {
                album_id: song.as_ref().and_then(|song| song.album_id.clone()),
                artist_id: song.and_then(|song| song.artist_id),
            })
        })
        .await
    }

    /// The songs of `keys`: those of the kept song list, the others read from Apple, library
    /// songs and catalog songs each in batches of 300. A song Apple does not know is absent
    /// from the result.
    pub async fn tracks(&self, keys: &[TrackKey]) -> Result<HashMap<TrackKey, Track>> {
        self.signed_in_read(Missing::Song, || async {
            let mut found = HashMap::new();
            let (mut library, mut catalog) = (Vec::new(), Vec::new());
            for key in keys {
                match (self.known_song(key), key.library_id(), key.catalog_id()) {
                    (Some(song), _, _) => {
                        found.insert(key.clone(), song);
                    }
                    (None, Some(id), _) => library.push(id.to_owned()),
                    (None, _, Some(id)) => catalog.push(id.to_owned()),
                    (None, None, None) => {}
                }
            }
            for ids in [&mut library, &mut catalog] {
                ids.sort_unstable();
                ids.dedup();
            }
            let (library, catalog) = try_join(
                async {
                    if library.is_empty() {
                        return Ok(Vec::new());
                    }
                    self.client
                        .library_songs_by_ids(&library, SONG_INCLUDE)
                        .await
                },
                async {
                    if catalog.is_empty() {
                        return Ok(Vec::new());
                    }
                    self.client.catalog_songs(&catalog).await
                },
            )
            .await?;
            let listed = self.caches.songs.peek();
            let catalog = catalog.iter().filter_map(map::catalog_track).map(|song| {
                let in_library = listed
                    .as_ref()
                    .is_some_and(|list| list.in_library(&song.key));
                Track { in_library, ..song }
            });
            // An answer may carry songs that were not asked for.
            for song in library.iter().filter_map(map::library_track).chain(catalog) {
                if keys.contains(&song.key) {
                    found.entry(song.key.clone()).or_insert(song);
                }
            }
            Ok(found)
        })
        .await
    }

    // Likes ----------------------------------------------------------------

    /// Whether each of `keys` is liked. Ratings are read in batches of 100, four at a
    /// time, and kept for ten minutes; liking and unliking here keeps them current.
    pub async fn liked(&self, keys: &[TrackKey]) -> Result<HashMap<TrackKey, bool>> {
        self.signed_in_read(Missing::Song, || async {
            let mut ratings = self.caches.rated(keys, self.clock.now());
            let mut seen = HashSet::new();
            let unread: Vec<TrackKey> = keys
                .iter()
                .filter(|key| !ratings.contains_key(*key) && seen.insert(*key))
                .cloned()
                .collect();
            if !unread.is_empty() {
                let read = self.read_ratings(&unread).await?;
                // A like set while the read ran is newer than the read: the kept rating
                // wins over it.
                ratings.extend(self.caches.rated(&unread, self.clock.now()));
                for (key, value) in read {
                    ratings.entry(key).or_insert(value);
                }
            }
            Ok(keys
                .iter()
                .map(|key| (key.clone(), ratings.get(key) == Some(&1)))
                .collect())
        })
        .await
    }

    /// Reads the ratings of `keys` in batches and keeps them; unrated songs read 0.
    async fn read_ratings(&self, keys: &[TrackKey]) -> Outcome<HashMap<TrackKey, i8>> {
        let session = self.caches.session();
        let (library, catalog): (Vec<&TrackKey>, Vec<&TrackKey>) =
            keys.iter().partition(|key| key.is_library());
        let ids = |keys: Vec<&TrackKey>| -> Vec<String> {
            keys.into_iter().map(ToString::to_string).collect()
        };
        let library = ids(library);
        let catalog: Vec<String> = catalog
            .into_iter()
            .filter_map(|key| key.catalog_id().map(str::to_owned))
            .collect();
        let batches: Vec<(bool, Vec<String>)> = library
            .chunks(ID_BATCH)
            .map(|batch| (false, batch.to_vec()))
            .chain(catalog.chunks(ID_BATCH).map(|batch| (true, batch.to_vec())))
            .collect();
        let read: Vec<_> = stream::iter(batches)
            .map(|(is_catalog, batch)| async move {
                let found = if is_catalog {
                    self.client.catalog_song_ratings(&batch).await
                } else {
                    self.client.library_song_ratings(&batch).await
                };
                match found {
                    Ok(found) => Ok((is_catalog, batch, found)),
                    // Apple answers an unrated song's rating with an error or no data at
                    // all: both mean it is not liked.
                    Err(Error::NotFound(_)) => Ok((is_catalog, batch, Vec::new())),
                    Err(error) => Err(Failure::from(error)),
                }
            })
            .buffer_unordered(PARALLEL_READS)
            .try_collect()
            .await?;
        let mut read_ratings = HashMap::new();
        for (is_catalog, batch, found) in read {
            let key = |id: String| {
                if is_catalog {
                    TrackKey::catalog(id)
                } else {
                    TrackKey::library(id)
                }
            };
            for id in &batch {
                read_ratings.extend(key(id.clone()).ok().map(|key| (key, 0)));
            }
            for rating in found {
                let value = rating.attributes.and_then(|attributes| attributes.value);
                read_ratings.extend(key(rating.id).ok().map(|key| (key, value.unwrap_or(0))));
            }
        }
        self.caches
            .merge_read_ratings(session, self.clock.now(), &read_ratings);
        Ok(read_ratings)
    }

    // Browsing -------------------------------------------------------------

    /// The library's artists.
    pub async fn artists(&self) -> Result<Vec<Artist>> {
        self.signed_in_read(Missing::Artist, || async {
            let artists: Vec<_> = self.client.library_artists().try_collect().await?;
            Ok(artists.iter().map(map::artist).collect())
        })
        .await
    }

    /// One library artist with their albums.
    pub async fn artist(&self, id: &str) -> Result<ArtistDetail> {
        self.signed_in_read(Missing::Artist, || async {
            let id = library_id(id, Missing::Artist)?;
            let (artist, albums) = try_join(
                self.client.library_artist(id),
                self.client
                    .library_artist_albums(id)
                    .try_collect::<Vec<_>>(),
            )
            .await?;
            let artist = map::artist(&artist);
            let albums = albums
                .iter()
                .map(|album| {
                    let mut album = map::album(album);
                    album.artist_id.get_or_insert_with(|| artist.id.clone());
                    album.artist.get_or_insert_with(|| artist.name.clone());
                    album
                })
                .collect();
            Ok(ArtistDetail { artist, albums })
        })
        .await
    }

    /// One library album with its songs.
    pub async fn album(&self, id: &str) -> Result<AlbumDetail> {
        self.signed_in_read(Missing::Album, || async {
            let id = library_id(id, Missing::Album)?;
            let (album, tracks) = try_join(
                self.client.library_album(id, &["artists"]),
                self.client.library_album_tracks(id).try_collect::<Vec<_>>(),
            )
            .await?;
            let album = map::album(&album);
            let tracks = tracks
                .iter()
                .filter_map(map::library_track)
                .map(|mut song| {
                    song.album_id.get_or_insert_with(|| album.id.clone());
                    if song.artist_id.is_none() && song.artist == album.artist {
                        song.artist_id.clone_from(&album.artist_id);
                    }
                    if song.artwork.is_none() {
                        song.artwork.clone_from(&album.artwork);
                    }
                    song
                })
                .collect();
            Ok(AlbumDetail { album, tracks })
        })
        .await
    }

    /// Every album of the library: recently added ones first, in Apple's order, then the
    /// rest. The list is kept for ten minutes, and refreshed in the background once old, so
    /// it is shared behind an `Arc`; the shorter lists of artists and playlists are read
    /// afresh on every call.
    pub async fn albums(&self) -> Result<Arc<Vec<Album>>> {
        self.kept_list(&ALBUMS, Missing::Album).await
    }

    /// The album list as last read, however old, or else as kept on disk; None when there is
    /// none. Nothing is asked of Apple.
    pub async fn kept_albums(&self) -> Option<Arc<Vec<Album>>> {
        self.restored().await;
        self.caches.albums.peek()
    }

    async fn build_album_list(&self) -> Outcome<Vec<Album>> {
        let library = async {
            let mut library = Vec::new();
            // A library album names its catalog album only through the catalog
            // relationship, which the artist page's classification keys on.
            let mut pages = self
                .client
                .library_album_pages(&["artists", "catalog"], PARALLEL_READS);
            while let Some(page) = pages.try_next().await? {
                library.extend(page.iter().map(map::album));
            }
            Ok::<_, Error>(library)
        };
        let (recent, library) = join(
            self.client.recently_added().try_collect::<Vec<_>>(),
            library,
        )
        .await;
        let library = library?;
        // Recently added only orders the list; without it the list is still complete.
        let recent = recent.unwrap_or_default();
        let by_id: HashMap<&str, &Album> = library
            .iter()
            .map(|album| (album.id.as_str(), album))
            .collect();
        let mut seen = HashSet::new();
        let mut albums = Vec::with_capacity(library.len());
        for entry in &recent {
            let LibraryEntry::Album(album) = entry else {
                continue;
            };
            if seen.insert(album.id.clone()) {
                albums.push(match by_id.get(album.id.as_str()) {
                    Some(known) => (*known).clone(),
                    None => map::album(album),
                });
            }
        }
        albums.extend(
            library
                .iter()
                .filter(|album| seen.insert(album.id.clone()))
                .cloned(),
        );
        Ok(albums)
    }

    // Playlists ------------------------------------------------------------

    /// The library's playlists, with how far each can be edited.
    pub async fn playlists(&self) -> Result<Vec<Playlist>> {
        self.signed_in_read(Missing::Playlist, || async {
            let playlists: Vec<LibraryPlaylist> =
                self.client.library_playlists().try_collect().await?;
            Ok(playlists.iter().map(map::playlist).collect())
        })
        .await
    }

    /// The songs of a library playlist, in order; none for an empty one.
    pub async fn playlist_tracks(&self, id: &str) -> Result<Vec<Track>> {
        self.signed_in_read(Missing::Playlist, || async {
            let id = library_id(id, Missing::Playlist)?;
            let tracks = self.playlist_songs(id).await?;
            Ok(tracks.iter().filter_map(map::library_track).collect())
        })
        .await
    }
}

/// A name as one artist, for a library that is not told how to split names.
fn one_artist(name: &str) -> Vec<String> {
    vec![name.to_owned()]
}

/// An Apple library id (`l.` album, `r.` artist, `p.` playlist). Catalog ids name nothing
/// in the library.
fn library_id(id: &str, what: Missing) -> result::Result<&str, LibraryError> {
    if is_library_id(id) {
        Ok(id)
    } else {
        Err(LibraryError::NotFound(what))
    }
}

/// Whether an error says Apple does not take the tokens.
fn rejects_tokens(error: &Error) -> bool {
    matches!(
        error,
        Error::Unauthorized | Error::Forbidden | Error::MissingUserToken | Error::Token(_)
    )
}

/// Maps a client error without deciding what a 401 or 403 means: the library does that
/// beforehand, by reading with the same tokens.
fn library_error(error: Error, what: Missing) -> LibraryError {
    match error {
        Error::MissingUserToken | Error::Token(_) => LibraryError::SignedOut,
        Error::Unauthorized | Error::Forbidden => LibraryError::Refused,
        Error::NotFound(_) => LibraryError::NotFound(what),
        Error::RateLimited { .. } | Error::Server { .. } | Error::Http(_) => {
            LibraryError::Unavailable(error)
        }
        error => LibraryError::Apple(error),
    }
}

/// Ends a refresh in the background, however it ends.
struct EndRefresh<T: Clone + 'static> {
    caches: Arc<Caches>,
    slot: fn(&Caches) -> &Slot<T>,
}

impl<T: Clone> Drop for EndRefresh<T> {
    fn drop(&mut self) {
        (self.slot)(&self.caches).end_refresh();
    }
}

/// The tokens of the current sign-in, in memory only. Nothing newer can be fetched on
/// refresh, so a rejected token stays rejected until the host signs in again.
#[derive(Default)]
struct SessionTokens(Mutex<Held>);

#[derive(Default)]
struct Held {
    tokens: Option<Tokens>,
    /// Whether any tokens were held before.
    ever: bool,
}

/// What a change of tokens replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Replaced {
    /// The tokens already held.
    Same,
    /// No tokens, and none were ever held before.
    First,
    /// Other tokens, or none.
    Other,
}

impl SessionTokens {
    /// Keeps `tokens`, or none, and tells what they replaced.
    fn replace(&self, tokens: Option<Tokens>) -> Replaced {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if held.tokens == tokens {
            return Replaced::Same;
        }
        let first = !held.ever;
        held.ever |= tokens.is_some();
        held.tokens = tokens;
        if first && held.ever {
            Replaced::First
        } else {
            Replaced::Other
        }
    }

    fn signed_in(&self) -> bool {
        self.read(|_| ()).is_some()
    }

    fn read<T>(&self, pick: impl FnOnce(&Tokens) -> T) -> Option<T> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .tokens
            .as_ref()
            .map(pick)
    }
}

#[async_trait]
impl TokenSource for SessionTokens {
    async fn developer_token(&self, _refresh: bool) -> crate::Result<String> {
        self.read(|tokens| tokens.developer.clone())
            .ok_or_else(|| Error::Token("not signed in to Apple Music".into()))
    }

    async fn user_token(&self, _refresh: bool) -> crate::Result<Option<String>> {
        Ok(self.read(|tokens| tokens.user.clone()))
    }
}
