use crate::{
    macros::RequestSlots,
    messages::{
        LibraryContentChanged, PlaybackMode, PlaybackState, PlaybackStatus, PlayingItemRef,
        PlaylistItem, PlaylistUpdate, RequestErrorKind, RequestFailedResponse, RequestFailureCause,
        SetRealtimeFFTEnabledRequest,
    },
    node::ReconnectConfig,
    service::{
        ServiceBinding, ServiceFuture, ServicePlayer,
        covers::{CoverSource, ServiceCoverCache, cover_cache_root},
        folder::prepare_service_library_folder,
        ids::RemoteIdMap,
        lists::{KeptList, ListSpec, ServiceLists, keep_value, kept_value, lists_folder},
    },
    utils::Broadcaster,
    visualizer::RANDOM_COVERS,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use visualizer::{Visualizer, VisualizerInputs};

use anyhow::Result;
use database::credentials::Credential;
use database::library_registry::ServiceKind;
use database::playing_item::PlayingItemMetadataSummary;
use futures::{FutureExt, SinkExt, StreamExt, future::BoxFuture};
use playback::player::{
    AudioPlaybackNote, AudioRequest, AudioSourceMode, PlayingItem, RemoteAudioError,
    RemoteAudioSource, ServiceTarget, UnavailableReason,
};
use subsonic::{
    SubsonicAlbum, SubsonicArtist, SubsonicClient, SubsonicCredentials, SubsonicJukebox,
    SubsonicProbe, SubsonicSong,
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::backends::remote::{deliver_visualizer_feed_frame, encode_message};
use crate::subsonic_translate::{
    jukebox_error_for_refresh, read_albums_by_name, read_newest_albums, read_song_list,
};
use anyhow::Error;
use reqwest::{StatusCode, header};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::iter;
use std::sync as std_sync;
use subsonic::StreamError;
use subsonic::SubsonicError;
use subsonic::SubsonicServerError;
use tokio::time as tokio_time;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum SubsonicIdKind {
    Artist,
    Album,
    Track,
    Playlist,
    CoverArt,
}

/// How many fetched songs a binding keeps at most.
const MAX_FETCHED_SONGS: usize = 2000;

/// How long a list read from the server stays current. Past it, the kept list is still given, and
/// read again in the background.
const LIST_TTL: Duration = Duration::from_secs(60);

/// The first songs of the song list, at most `mixes::MAX_COVER_POOL`, each once.
const SONGS: ListSpec = ListSpec {
    name: "songs",
    max_age: LIST_TTL,
};

/// Every album, by name.
const ALBUMS: ListSpec = ListSpec {
    name: "albums",
    max_age: LIST_TTL,
};

/// Every album, newest added first.
const NEWEST_ALBUMS: ListSpec = ListSpec {
    name: "newest-albums",
    max_age: LIST_TTL,
};

/// Every artist, in the order the server lists them.
const ARTISTS: ListSpec = ListSpec {
    name: "artists",
    max_age: LIST_TTL,
};

/// The name the probe is kept beside the lists under.
const PROBE: &str = "probe";

pub(crate) struct SubsonicBinding {
    pub(crate) library_id: Uuid,
    pub(crate) play_on_this_device: bool,
    pub(crate) client: Arc<SubsonicClient>,
    /// What the last check of the server found. A library opened from what it kept checks the
    /// server in the background, and the probe it gets replaces this one.
    pub(crate) probe: Mutex<SubsonicProbe>,
    /// Cancelled once the sign-in is renewed, as what a check of the server found under the
    /// sign-in before says nothing of the new one, or the library closes, which takes nothing
    /// from a check any more.
    check_over: CancellationToken,
    pub(crate) catalog: Arc<Mutex<SubsonicCatalog>>,
    pub(crate) lists: SubsonicLists,
    pub(crate) covers: Arc<ServiceCoverCache>,
    jukebox_poll: Mutex<Option<CancellationToken>>,
    /// Whether the jukebox is followed, as the UI's player, until the library closes.
    followed: AtomicBool,
    jukebox_state: Arc<Mutex<Option<JukeboxPollState>>>,
    playback_starts: Mutex<HashMap<String, u64>>,
    /// Songs fetched by id, kept so that what is asked about a song right after it was fetched,
    /// such as the cover of the track that just started, needs no second request.
    fetched_songs: Mutex<HashMap<String, SubsonicSong>>,
    visualizer_feed: Mutex<Option<ActiveFeed>>,
    /// Counts feed link changes, so the node can republish the visualizer inputs.
    feed_link_changes: watch::Sender<u64>,
    /// The visualizer of the node this jukebox plays for, which its feed writes.
    visualizer: Arc<Visualizer>,
    /// What the random groups of the home are drawn with, rotated over time.
    pub(crate) home_seed: u64,
}

/// The running visualizer feed connection loop.
struct ActiveFeed {
    cancel: CancellationToken,
    link: Arc<Mutex<FeedLink>>,
}

/// Whether the feed is expected to deliver data. A feed that is starting counts
/// as live, so opening the library does not flicker the scene; only a failed
/// connect or a dropped connection makes it lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FeedLink {
    Live,
    Lost,
}

/// One feed connection ended.
enum FeedSessionEnd {
    Cancelled,
    /// The server could not be reached or refused the subscription.
    NotConnected,
    /// A working connection ended.
    Dropped,
}

#[derive(Clone, Debug, PartialEq)]
struct JukeboxPollState {
    jukebox: SubsonicJukebox,
    /// The cached cover of the current song when this state was published.
    cover_art_path: Option<String>,
    // The server exposes no queue revision, so this local revision is best effort.
    revision: u64,
}

/// What differs from the jukebox state published before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JukeboxChange {
    /// The queue or the playback status changed.
    Queue,
    /// Only the cover of the current song became available.
    CoverOnly,
}

fn current_song(jukebox: &SubsonicJukebox) -> Option<&SubsonicSong> {
    jukebox
        .status
        .current_index
        .and_then(|index| jukebox.playlist.songs.get(index as usize))
}

fn current_cover_id(jukebox: &SubsonicJukebox) -> Option<&str> {
    current_song(jukebox)?.cover_art.as_deref()
}

/// Remembers what was published. `None` means the state lock is poisoned;
/// `Some((revision, None))` means nothing changed.
fn record_jukebox(
    state: &Mutex<Option<JukeboxPollState>>,
    jukebox: &SubsonicJukebox,
    cover_art_path: Option<String>,
) -> Option<(u64, Option<JukeboxChange>)> {
    let mut state = state.lock().ok()?;
    let (revision, change) = match state.as_ref() {
        Some(current)
            if current.jukebox == *jukebox && current.cover_art_path == cover_art_path =>
        {
            return Some((current.revision, None));
        }
        Some(current) if current.jukebox == *jukebox => {
            (current.revision, JukeboxChange::CoverOnly)
        }
        Some(current) => (current.revision.saturating_add(1), JukeboxChange::Queue),
        None => (1, JukeboxChange::Queue),
    };
    *state = Some(JukeboxPollState {
        jukebox: jukebox.clone(),
        cover_art_path,
        revision,
    });
    Some((revision, Some(change)))
}

fn broadcast_jukebox(
    broadcaster: &dyn Broadcaster,
    library_id: Uuid,
    jukebox: &SubsonicJukebox,
    revision: u64,
    cover_art_path: Option<String>,
    change: JukeboxChange,
) {
    if change == JukeboxChange::Queue {
        broadcast_jukebox_queue(broadcaster, library_id, jukebox, revision);
    }
    broadcaster.broadcast(&jukebox_playback_status(
        library_id,
        jukebox,
        None,
        cover_art_path,
    ));
}

fn broadcast_jukebox_queue(
    broadcaster: &dyn Broadcaster,
    library_id: Uuid,
    jukebox: &SubsonicJukebox,
    revision: u64,
) {
    let session_id = library_id.to_string();
    let items = jukebox
        .playlist
        .songs
        .iter()
        .map(|song| PlaylistItem {
            unavailable: None,
            item: PlayingItemRef::LibraryTrack {
                library_id: session_id.clone(),
                track: song.id.clone(),
            },
            artist: song.artist.clone().unwrap_or_default(),
            album: song.album.clone().unwrap_or_default(),
            title: song.title.clone(),
            duration: f64::from(song.duration.unwrap_or_default()),
        })
        .collect();
    broadcaster.broadcast(&PlaylistUpdate {
        items,
        session_id: session_id.clone(),
        revision,
    });
}

fn jukebox_playback_status(
    library_id: Uuid,
    jukebox: &SubsonicJukebox,
    feed_position: Option<(PlaybackState, f32)>,
    cover_art_path: Option<String>,
) -> PlaybackStatus {
    let current = current_song(jukebox);
    let polled_position = jukebox.status.position_secs.unwrap_or_default() as f32;
    let position = feed_position.map_or(polled_position, |(_, position)| position);
    let duration = current
        .and_then(|song| song.duration)
        .map(f64::from)
        .unwrap_or_default();
    PlaybackStatus {
        session_id: library_id.to_string(),
        state: feed_position.map_or_else(
            || {
                if jukebox.status.playing {
                    PlaybackState::Playing
                } else if current.is_some() {
                    PlaybackState::Paused
                } else {
                    PlaybackState::Stopped
                }
            },
            |(state, _)| state,
        ),
        progress_seconds: position,
        progress_percentage: if duration > 0.0 {
            position / duration as f32
        } else {
            0.0
        },
        artist: current.and_then(|song| song.artist.clone()),
        album: current.and_then(|song| song.album.clone()),
        title: current.map(|song| song.title.clone()),
        duration,
        index: current
            .and(jukebox.status.current_index)
            .and_then(|index| i32::try_from(index).ok()),
        item: current.map(|song| song.id.clone()),
        item_request: current.map(|song| PlayingItemRef::LibraryTrack {
            library_id: library_id.to_string(),
            track: song.id.clone(),
        }),
        playback_mode: PlaybackMode::Sequential,
        ready: current.is_some(),
        cover_art_path,
        lib_path: String::new(),
    }
}

/// The only writer of a feed's link. The node republishes the visualizer inputs
/// when it hears of a change.
fn set_feed_link(
    binding: &Weak<SubsonicBinding>,
    link: &Mutex<FeedLink>,
    cancellation: &CancellationToken,
    next: FeedLink,
) {
    if cancellation.is_cancelled() {
        return;
    }
    {
        let Ok(mut current) = link.lock() else {
            return;
        };
        if *current == next {
            return;
        }
        *current = next;
    }
    if let Some(binding) = binding.upgrade() {
        binding.feed_link_changes.send_modify(|count| *count += 1);
    }
}

/// Connects to the feed and delivers its events until the connection ends.
async fn run_feed_session(
    binding: &Weak<SubsonicBinding>,
    visualizer: &Visualizer,
    client: &SubsonicClient,
    probe: &subsonic::SubsonicProbe,
    broadcaster: &Arc<dyn Broadcaster>,
    link: &Mutex<FeedLink>,
    cancellation: &CancellationToken,
) -> FeedSessionEnd {
    let connect = async {
        let url = match client.rune_visualizer_feed_url(probe).await {
            Ok(url) => url,
            Err(error) => {
                log::warn!("Could not resolve Subsonic visualizer feed: {error}");
                return None;
            }
        };
        match connect_async(url.as_str()).await {
            Ok((socket, _)) => Some(socket),
            Err(error) => {
                log::warn!("Could not connect to Subsonic visualizer feed: {error}");
                None
            }
        }
    };
    let socket = tokio::select! {
        _ = cancellation.cancelled() => return FeedSessionEnd::Cancelled,
        socket = connect => socket,
    };
    let Some(mut socket) = socket else {
        return FeedSessionEnd::NotConnected;
    };
    let fft_payload = match rinf::serialize(&SetRealtimeFFTEnabledRequest {
        enabled: true,
        call_id: None,
    }) {
        Ok(payload) => payload,
        Err(error) => {
            log::warn!("Could not encode Subsonic visualizer feed request: {error}");
            return FeedSessionEnd::NotConnected;
        }
    };
    for (name, body) in [
        ("SubscribeVisualizerFeedRequest", Vec::new()),
        ("SetRealtimeFFTEnabledRequest", fft_payload),
    ] {
        let frame = encode_message(name, &body, None);
        if let Err(error) = socket.send(Message::Binary(frame.into())).await {
            log::warn!("Could not subscribe to Subsonic visualizer feed: {error}");
            return FeedSessionEnd::NotConnected;
        }
    }
    set_feed_link(binding, link, cancellation, FeedLink::Live);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => {
                let frame = encode_message("UnsubscribeVisualizerFeedRequest", &[], None);
                let _ = socket.send(Message::Binary(frame.into())).await;
                let _ = socket.close(None).await;
                return FeedSessionEnd::Cancelled;
            }
            message = socket.next() => match message {
                Some(Ok(Message::Binary(payload))) => {
                    if let Err(error) = deliver_visualizer_feed_frame(
                        visualizer,
                        &payload,
                        &**broadcaster,
                        |status| {
                            if let Some(binding) = binding.upgrade() {
                                binding.broadcast_visualizer_feed_status(status);
                            }
                        },
                    ) {
                        log::debug!("Ignoring Subsonic visualizer feed frame: {error:#}");
                    }
                }
                Some(Ok(Message::Close(_))) | None => return FeedSessionEnd::Dropped,
                Some(Err(error)) => {
                    log::debug!("Subsonic visualizer feed ended: {error}");
                    return FeedSessionEnd::Dropped;
                }
                Some(Ok(_)) => {}
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct SubsonicCatalog {
    /// Opaque catalog IDs are scoped to the lifetime of the open Subsonic binding.
    pub(crate) ids: RemoteIdMap<SubsonicIdKind>,
    pub(crate) artists: HashMap<i32, SubsonicArtist>,
    pub(crate) albums: HashMap<i32, SubsonicAlbum>,
    pub(crate) songs: HashMap<i32, SubsonicSong>,
}

impl SubsonicCatalog {
    fn keep_songs(&mut self, songs: &[SubsonicSong]) {
        keep_read(
            &mut self.ids,
            &mut self.songs,
            SubsonicIdKind::Track,
            songs,
            |song| song.id.as_str(),
        );
    }

    fn keep_albums(&mut self, albums: &[SubsonicAlbum]) {
        keep_read(
            &mut self.ids,
            &mut self.albums,
            SubsonicIdKind::Album,
            albums,
            |album| album.id.as_str(),
        );
    }

    /// Forgets what the server told of the items, which another account may not be let see. The
    /// ids stay, so an id the UI holds names the same item, which the server is asked about.
    fn forget_metadata(&mut self) {
        self.artists.clear();
        self.albums.clear();
        self.songs.clear();
    }

    fn keep_artists(&mut self, artists: &[SubsonicArtist]) {
        keep_read(
            &mut self.ids,
            &mut self.artists,
            SubsonicIdKind::Artist,
            artists,
            |artist| artist.id.as_str(),
        );
    }
}

/// Keeps the items of a list read from the server, under the ids this library gives them, in
/// place of what was kept of them. An item no id can be given to any more is left out here: the
/// request that shows it numbers it, and tells of it.
fn keep_read<T: Clone>(
    ids: &mut RemoteIdMap<SubsonicIdKind>,
    held: &mut HashMap<i32, T>,
    kind: SubsonicIdKind,
    items: &[T],
    remote_id: fn(&T) -> &str,
) {
    for item in items {
        if let Ok(id) = ids.intern(kind, remote_id(item)) {
            held.insert(id, item.clone());
        }
    }
}

/// The lists a Subsonic library keeps, each read as the server lists it. A read keeps what it
/// brings in the catalog too, in place of what was kept of the same items.
pub(crate) struct SubsonicLists {
    kept: ServiceLists,
    pub(crate) songs: KeptList<Vec<SubsonicSong>, SubsonicError>,
    pub(crate) albums: KeptList<Vec<SubsonicAlbum>, SubsonicError>,
    pub(crate) newest_albums: KeptList<Vec<SubsonicAlbum>, SubsonicError>,
    pub(crate) artists: KeptList<Vec<SubsonicArtist>, SubsonicError>,
}

/// Reads a list with the client.
type ReadList<T> = fn(Arc<SubsonicClient>) -> BoxFuture<'static, Result<Vec<T>, SubsonicError>>;

impl SubsonicLists {
    fn new(
        dir: PathBuf,
        client: &Arc<SubsonicClient>,
        catalog: &Arc<Mutex<SubsonicCatalog>>,
        broadcaster: Arc<dyn Broadcaster>,
    ) -> Self {
        // The lists are the account's that signs in, which the username names on a server. A page
        // that shows a list reads it again once a read brings another.
        let kept = ServiceLists::new(dir, client.user().as_deref(), move || {
            broadcaster.broadcast(&LibraryContentChanged {
                path: String::new(),
            });
        });
        Self {
            songs: declare_list(
                &kept,
                SONGS,
                (client, catalog),
                |client| async move { read_song_list(&client).await }.boxed(),
                SubsonicCatalog::keep_songs,
            ),
            albums: declare_list(
                &kept,
                ALBUMS,
                (client, catalog),
                |client| async move { read_albums_by_name(&client).await }.boxed(),
                SubsonicCatalog::keep_albums,
            ),
            newest_albums: declare_list(
                &kept,
                NEWEST_ALBUMS,
                (client, catalog),
                |client| async move { read_newest_albums(&client, usize::MAX).await }.boxed(),
                SubsonicCatalog::keep_albums,
            ),
            artists: declare_list(
                &kept,
                ARTISTS,
                (client, catalog),
                |client| async move { client.artists().await }.boxed(),
                SubsonicCatalog::keep_artists,
            ),
            kept,
        }
    }
}

/// Declares the list `spec` names on `lists`, which `read` reads with the client, and whose items
/// the catalog keeps with `keep` once the list keeps them.
fn declare_list<T>(
    lists: &ServiceLists,
    spec: ListSpec,
    (client, catalog): (&Arc<SubsonicClient>, &Arc<Mutex<SubsonicCatalog>>),
    read: ReadList<T>,
    keep: fn(&mut SubsonicCatalog, &[T]),
) -> KeptList<Vec<T>, SubsonicError>
where
    T: Serialize + DeserializeOwned + PartialEq + Send + Sync + 'static,
{
    let (client, catalog) = (Arc::clone(client), Arc::clone(catalog));
    lists.declare(
        spec,
        move || read(Arc::clone(&client)),
        // Only a list the list keeps, so never one read for the account signed in before.
        move |list: &Vec<T>| {
            keep(
                &mut catalog.lock().unwrap_or_else(PoisonError::into_inner),
                list,
            )
        },
    )
}

impl SubsonicBinding {
    pub(crate) fn new(
        client: SubsonicClient,
        probe: subsonic::SubsonicProbe,
        config_dir: PathBuf,
        library_id: Uuid,
        play_on_this_device: bool,
        broadcaster: Arc<dyn Broadcaster>,
        visualizer: Arc<Visualizer>,
    ) -> Self {
        prepare_service_library_folder(&config_dir, library_id);
        let client = Arc::new(client);
        let catalog = Arc::new(Mutex::new(SubsonicCatalog::default()));
        Self {
            library_id,
            play_on_this_device,
            lists: SubsonicLists::new(
                lists_folder(&config_dir, library_id),
                &client,
                &catalog,
                Arc::clone(&broadcaster),
            ),
            client,
            probe: Mutex::new(probe),
            check_over: CancellationToken::new(),
            catalog,
            covers: Arc::new(ServiceCoverCache::new(
                cover_cache_root(&config_dir, library_id),
                broadcaster,
            )),
            jukebox_poll: Mutex::new(None),
            followed: AtomicBool::new(false),
            jukebox_state: Arc::new(Mutex::new(None)),
            playback_starts: Mutex::new(HashMap::new()),
            fetched_songs: Mutex::new(HashMap::new()),
            visualizer_feed: Mutex::new(None),
            feed_link_changes: watch::channel(0).0,
            visualizer,
            home_seed: rand::random(),
        }
    }

    /// What the last check of the server of the library `library_id` found for the account
    /// `user`, kept when the library was opened before with that account. It reads the file, so
    /// it blocks.
    pub(crate) fn kept_probe(
        config_dir: &Path,
        library_id: Uuid,
        user: &str,
    ) -> Option<SubsonicProbe> {
        kept_value(config_dir, library_id, Some(user), PROBE)
    }

    /// Keeps `probe`, what the check of a new sign-in to the server of the library `library_id` as
    /// `user` found, so the library opens with it next time.
    pub(crate) fn keep_verified_probe(
        config_dir: &Path,
        library_id: Uuid,
        user: &str,
        probe: SubsonicProbe,
    ) {
        keep_value(config_dir, library_id, Some(user), PROBE, probe);
    }

    /// What the last check of the server found: the roles of the account and what the server
    /// declares.
    pub(crate) fn probe(&self) -> SubsonicProbe {
        self.probe
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Keeps the probe on disk, so the library opens from it next time.
    pub(crate) fn keep_probe(&self) {
        self.lists.kept.keep(PROBE, self.probe());
    }

    /// Takes what the check of the server, made under the sign-in the library opened with,
    /// found, unless the sign-in was renewed or the library closed since: a probe replaces the
    /// kept one, on disk too, and a sign-in the server rejects asks for a new one. A jukebox that
    /// is followed is followed again as the probe has it now. Says whether the probe changed.
    pub(crate) fn take_check(
        self: &Arc<Self>,
        probed: Result<SubsonicProbe, SubsonicError>,
    ) -> bool {
        // A renewal and a close cancel under this lock, so each comes wholly before the check is
        // taken, which it then ends, or wholly after.
        let mut kept = self.probe.lock().unwrap_or_else(PoisonError::into_inner);
        if self.check_over.is_cancelled() {
            return false;
        }
        match probed {
            Ok(probe) => {
                let changed = *kept != probe;
                let follow_again = probe.roles.jukebox_role != kept.roles.jukebox_role
                    || probe.supports_rune_visualizer_feed()
                        != kept.supports_rune_visualizer_feed();
                *kept = probe.clone();
                // Kept on disk under the lock, so a renewal to another account clears it after.
                self.lists.kept.keep(PROBE, probe);
                drop(kept);
                if follow_again && self.followed.load(Ordering::Acquire) {
                    // Followed again as the probe has it now: polled, with the feed the server
                    // declares, while the account has the role, and not at all without it.
                    self.stop_jukebox_polling();
                    self.start_jukebox_polling();
                    // A close meanwhile stops what this started.
                    if !self.followed.load(Ordering::Acquire) {
                        self.stop_jukebox_polling();
                    }
                }
                changed
            }
            // A server in front of Subsonic may refuse the sign-in by its HTTP status, as the
            // stream treats it.
            Err(
                SubsonicError::Server {
                    kind:
                        SubsonicServerError::WrongCredentials
                        | SubsonicServerError::NotAuthorized
                        | SubsonicServerError::InvalidApiKey,
                    ..
                }
                | SubsonicError::HttpStatus(StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
            ) => {
                self.ask_for_sign_in("Subsonic server check");
                false
            }
            Err(error) => {
                log::info!("Could not check the Subsonic server: {error}");
                false
            }
        }
    }

    /// Cancelled once the sign-in is renewed or the library closes, after which the check of the
    /// server under the sign-in it opened with is over.
    pub(crate) fn check_over(&self) -> CancellationToken {
        self.check_over.clone()
    }

    /// Asks the UI for a new sign-in, as the server rejects the one the library keeps, which
    /// `request` found.
    pub(crate) fn ask_for_sign_in(&self, request: &str) {
        self.covers.broadcaster.broadcast(&RequestFailedResponse {
            request: request.into(),
            call_id: None,
            kind: RequestErrorKind::PermissionDenied,
            detail: "Subsonic account is not authorized for this request".into(),
            cause: Some(RequestFailureCause::ServiceSignInRequired {
                record_id: self.library_id.to_string(),
            }),
        });
    }

    pub(crate) fn start_jukebox_polling(self: &Arc<Self>) {
        if self.play_on_this_device {
            return;
        }
        if !self.probe().roles.jukebox_role {
            return;
        }
        let Ok(mut poll) = self.jukebox_poll.lock() else {
            return;
        };
        if let Some(token) = poll.take() {
            token.cancel();
        }
        let cancellation = self.covers.cancellation.child_token();
        *poll = Some(cancellation.clone());
        self.start_visualizer_feed();
        let client = Arc::clone(&self.client);
        let library_id = self.library_id;
        let broadcaster = Arc::clone(&self.covers.broadcaster);
        let covers = Arc::clone(&self.covers);
        let binding_state = Arc::clone(&self.jukebox_state);
        tokio::spawn(async move {
            let mut interval = tokio_time::interval(Duration::from_secs(3));
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => break,
                    _ = interval.tick() => {
                        let result = tokio::select! {
                            _ = cancellation.cancelled() => break,
                            result = async {
                                let status = client.jukebox_status().await?;
                                let mut jukebox = client.jukebox_get().await?;
                                jukebox.status = status;
                                Ok::<_, subsonic::SubsonicError>(jukebox)
                            } => result,
                        };
                        match result {
                            Ok(jukebox) => {
                                let cover_id = current_cover_id(&jukebox);
                                let cover = match cover_id {
                                    Some(id) => covers.cached_path(id).await.ok().flatten(),
                                    None => None,
                                };
                                let Some((revision, Some(change))) =
                                    record_jukebox(&binding_state, &jukebox, cover.clone())
                                else {
                                    continue;
                                };
                                if change == JukeboxChange::Queue {
                                    let uncached = cover_id.filter(|_| cover.is_none());
                                    covers.schedule_playing(
                                        Arc::clone(&client),
                                        uncached.map(str::to_owned),
                                    );
                                }
                                broadcast_jukebox(
                                    &*broadcaster,
                                    library_id,
                                    &jukebox,
                                    revision,
                                    cover,
                                    change,
                                );
                            }
                            Err(error) => log::debug!("Subsonic jukebox status poll failed: {error}"),
                        }
                    }
                }
            }
        });
    }

    pub(crate) fn stop_jukebox_polling(&self) {
        if let Ok(mut poll) = self.jukebox_poll.lock()
            && let Some(token) = poll.take()
        {
            token.cancel();
        }
        if let Ok(mut feed) = self.visualizer_feed.lock()
            && let Some(active) = feed.take()
        {
            active.cancel.cancel();
        }
    }

    fn broadcast_visualizer_feed_status(&self, status: &PlaybackStatus) {
        let polled = self
            .jukebox_state
            .lock()
            .ok()
            .and_then(|state| state.as_ref().cloned());
        let Some(polled) = polled else {
            return;
        };
        let Some(current) = current_song(&polled.jukebox) else {
            return;
        };
        let matches_current_track = matches!(
            status.item_request.as_ref(),
            Some(PlayingItemRef::LibraryTrack { track, .. }) if track == &current.id
        );
        if !matches_current_track {
            return;
        }
        self.covers.broadcaster.broadcast(&jukebox_playback_status(
            self.library_id,
            &polled.jukebox,
            Some((status.state, status.progress_seconds)),
            polled.cover_art_path.clone(),
        ));
    }

    fn start_visualizer_feed(self: &Arc<Self>) {
        let probe = self.probe();
        if !probe.supports_rune_visualizer_feed() {
            return;
        }
        let Ok(mut feed) = self.visualizer_feed.lock() else {
            return;
        };
        if let Some(active) = feed.take() {
            active.cancel.cancel();
        }
        let cancellation = self.covers.cancellation.child_token();
        let link = Arc::new(Mutex::new(FeedLink::Live));
        *feed = Some(ActiveFeed {
            cancel: cancellation.clone(),
            link: Arc::clone(&link),
        });
        let client = Arc::clone(&self.client);
        let broadcaster = Arc::clone(&self.covers.broadcaster);
        let visualizer = Arc::clone(&self.visualizer);
        let binding = Arc::downgrade(self);
        tokio::spawn(async move {
            // Same waits as a Rune peer's reconnect; a feed has no deadline because it
            // lives as long as the jukebox is the current player.
            let config = ReconnectConfig::default();
            let backoff = || {
                config
                    .delays
                    .into_iter()
                    .chain(iter::repeat(config.interval))
            };
            let mut delays = backoff();
            loop {
                let end = run_feed_session(
                    &binding,
                    &visualizer,
                    &client,
                    &probe,
                    &broadcaster,
                    &link,
                    &cancellation,
                )
                .await;
                match end {
                    FeedSessionEnd::Cancelled => break,
                    FeedSessionEnd::Dropped => delays = backoff(),
                    FeedSessionEnd::NotConnected => {}
                }
                set_feed_link(&binding, &link, &cancellation, FeedLink::Lost);
                let delay = delays.next().unwrap_or(config.interval);
                tokio::select! {
                    _ = cancellation.cancelled() => break,
                    _ = tokio_time::sleep(delay) => {}
                }
            }
        });
    }

    pub(crate) async fn report_track_started_at(&self, track_id: &str, start_time: u64) {
        if !self.play_on_this_device {
            return;
        }
        if let Ok(mut starts) = self.playback_starts.lock() {
            starts.insert(track_id.to_owned(), start_time);
            while starts.len() > 64 {
                let Some(oldest_track) = starts
                    .iter()
                    .min_by_key(|(_, time)| *time)
                    .map(|(track, _)| track.clone())
                else {
                    break;
                };
                starts.remove(&oldest_track);
            }
        }
        self.report_scrobble(track_id, None, false).await;
    }

    pub(crate) async fn report_track_played(&self, track_id: &str) {
        let start_time = self
            .playback_starts
            .lock()
            .ok()
            .and_then(|mut starts| starts.remove(track_id));
        if let Some(start_time) = start_time {
            self.report_scrobble(track_id, Some(start_time), true).await;
        }
    }

    async fn report_scrobble(&self, track_id: &str, time: Option<u64>, submission: bool) {
        if !self.play_on_this_device {
            return;
        }
        if let Err(error) = self.client.scrobble(track_id, time, submission).await {
            log::warn!("Subsonic scrobble failed: {error:?}");
            if matches!(
                error,
                SubsonicError::Server {
                    kind: SubsonicServerError::WrongCredentials
                        | SubsonicServerError::NotAuthorized
                        | SubsonicServerError::InvalidApiKey,
                    ..
                }
            ) {
                self.ask_for_sign_in("Subsonic scrobble");
            }
        }
    }

    pub(crate) async fn jukebox_snapshot(&self) -> Result<(SubsonicJukebox, u64)> {
        let jukebox = self
            .client
            .jukebox_get()
            .await
            .map_err(|error| jukebox_error_for_refresh(self, error))?;
        let cover_id = current_cover_id(&jukebox);
        let cover = match cover_id {
            Some(id) => self.covers.cached_path(id).await.ok().flatten(),
            None => None,
        };
        let (revision, change) = record_jukebox(&self.jukebox_state, &jukebox, cover.clone())
            .ok_or_else(|| anyhow::anyhow!("Subsonic jukebox state lock poisoned"))?;
        if let Some(change) = change {
            if change == JukeboxChange::Queue {
                let uncached = cover_id.filter(|_| cover.is_none());
                self.covers
                    .schedule_playing(Arc::clone(&self.client), uncached.map(str::to_owned));
            }
            broadcast_jukebox(
                &*self.covers.broadcaster,
                self.library_id,
                &jukebox,
                revision,
                cover,
                change,
            );
        }
        Ok((jukebox, revision))
    }

    pub(crate) async fn publish_jukebox_snapshot(&self) -> Result<()> {
        self.jukebox_snapshot().await.map(|_| ())
    }

    pub(crate) async fn song(&self, track_id: &str) -> Result<SubsonicSong> {
        let song = self
            .client
            .song(track_id)
            .await
            .map_err(subsonic_playback_error)?;
        let mut fetched = self
            .fetched_songs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // The songs kept only spare the server a repeat, so a full set starts over.
        if fetched.len() >= MAX_FETCHED_SONGS {
            fetched.clear();
        }
        fetched.insert(track_id.to_owned(), song.clone());
        Ok(song)
    }

    /// A song this binding holds already, from an earlier fetch or from the catalog.
    fn known_song(&self, track_id: &str) -> Option<SubsonicSong> {
        if let Some(song) = self
            .fetched_songs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(track_id)
        {
            return Some(song.clone());
        }
        let catalog = self.catalog.lock().unwrap_or_else(PoisonError::into_inner);
        let id = catalog.ids.lookup(SubsonicIdKind::Track, track_id)?;
        catalog.songs.get(&id).cloned()
    }

    fn song_summary(&self, track_id: &str, song: SubsonicSong) -> PlayingItemMetadataSummary {
        PlayingItemMetadataSummary {
            item: PlayingItem::LibraryTrack {
                library_id: self.library_id,
                track: track_id.to_owned(),
            },
            artist: song.artist.unwrap_or_else(|| "Unknown Artist".to_owned()),
            album: song.album.unwrap_or_else(|| "Unknown Album".to_owned()),
            title: if song.title.is_empty() {
                "Unknown Title".to_owned()
            } else {
                song.title
            },
            track_number: song
                .track
                .and_then(|number| i32::try_from(number).ok())
                .unwrap_or_default(),
            duration: f64::from(song.duration.unwrap_or_default()),
        }
    }

    pub(crate) async fn cover_path(&self, cover_id: Option<&str>) -> Option<String> {
        let cover_id = cover_id?;
        self.covers
            .schedule(Arc::clone(&self.client), cover_id.to_owned());
        self.covers.cached_path(cover_id).await.ok().flatten()
    }

    /// The path of a cover once it is cached, fetching it first if it is not.
    pub(crate) async fn cached_cover(&self, cover_id: &str) -> Result<Option<String>> {
        self.covers
            .ensure_cached(Arc::clone(&self.client), cover_id)
            .await
    }
}

impl ServiceBinding for SubsonicBinding {
    fn library_id(&self) -> Uuid {
        self.library_id
    }

    fn slots(&self) -> RequestSlots {
        RequestSlots::subsonic(&self.probe())
    }

    fn service(&self) -> ServiceKind {
        ServiceKind::Subsonic
    }

    fn item_metadata<'a>(
        &'a self,
        track: &'a str,
        with_cover: bool,
    ) -> ServiceFuture<'a, (PlayingItemMetadataSummary, Option<String>)> {
        Box::pin(async move {
            let song = self.song(track).await?;
            let cover = if with_cover {
                self.cover_path(song.cover_art.as_deref()).await
            } else {
                None
            };
            Ok((self.song_summary(track, song), cover))
        })
    }

    fn cached_summary(&self, track: &str) -> Option<PlayingItemMetadataSummary> {
        self.known_song(track)
            .map(|song| self.song_summary(track, song))
    }

    fn cover_when_cached<'a>(&'a self, track: &'a str) -> ServiceFuture<'a, Option<String>> {
        Box::pin(async move {
            let song = match self.known_song(track) {
                Some(song) => song,
                None => self.song(track).await?,
            };
            match song.cover_art {
                Some(cover_id) => self.cached_cover(&cover_id).await,
                None => Ok(None),
            }
        })
    }

    fn visualizer_covers<'a>(
        &'a self,
        track: &'a str,
    ) -> ServiceFuture<'a, (Option<String>, Vec<String>)> {
        Box::pin(async move {
            let wall_covers = RANDOM_COVERS;
            let song = self.song(track).await?;
            let playing = match song.cover_art.as_deref() {
                Some(id) => {
                    self.covers
                        .ensure_cached(Arc::clone(&self.client), id)
                        .await?
                }
                None => None,
            };
            let albums = self.client.album_list(wall_covers as u32, 0).await?;
            let mut wall = Vec::with_capacity(wall_covers);
            let mut seen = HashSet::new();
            for album in albums.into_iter().take(wall_covers) {
                let Some(id) = album.cover_art else { continue };
                if song.cover_art.as_ref() == Some(&id) || !seen.insert(id.clone()) {
                    continue;
                }
                if let Some(path) = self
                    .covers
                    .ensure_cached(Arc::clone(&self.client), &id)
                    .await?
                {
                    wall.push(path);
                }
            }
            Ok((playing, wall))
        })
    }

    fn playback_source<'a>(&'a self, track: &'a str) -> ServiceFuture<'a, ServiceTarget> {
        Box::pin(async move {
            let song = self
                .client
                .song(track)
                .await
                .map_err(subsonic_playback_error)?;
            let extension = song
                .suffix
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "audio".to_owned());
            let cancellation = CancellationToken::new();
            let failure_reason = Arc::new(Mutex::new(None));
            Ok(ServiceTarget::Stream(RemoteAudioSource {
                request: Arc::new(SubsonicAudioRequest {
                    client: Arc::clone(&self.client),
                    track_id: track.to_owned(),
                    failure_reason: failure_reason.clone(),
                }),
                // The request capability owns the authenticated URL. This parse-only URL has no
                // query, credentials, or server identity and is never requested directly.
                url: "https://subsonic.invalid/rest/stream".to_owned(),
                extension,
                size: song.size.unwrap_or_default(),
                cancellation,
                failure_reason,
            }))
        })
    }

    fn track_started<'a>(&'a self, track: &'a str, start_time: u64) -> BoxFuture<'a, ()> {
        Box::pin(self.report_track_started_at(track, start_time))
    }

    fn track_played_through<'a>(&'a self, track: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(self.report_track_played(track))
    }

    fn own_player(self: Arc<Self>) -> Option<Arc<dyn ServicePlayer>> {
        if self.play_on_this_device {
            None
        } else {
            Some(self)
        }
    }

    fn renew_sign_in(&self, credential: &Credential) {
        // The kept credential holds the password a sign-in was verified with.
        let Ok(password) = str::from_utf8(credential.secret.as_bytes()) else {
            return;
        };
        // Another username is another account, whose lists the kept ones are not. A new
        // password for the same account keeps them.
        let other_account = self.client.user().as_deref() != Some(credential.username.as_str());
        {
            // Under the probe lock, so a check of the server under the sign-in before is taken
            // wholly before this or not at all.
            let _probe = self.probe.lock().unwrap_or_else(PoisonError::into_inner);
            self.client
                .replace_credentials(SubsonicCredentials::Password {
                    user: credential.username.clone(),
                    password: password.to_owned(),
                });
            self.check_over.cancel();
        }
        if other_account {
            // The lists first, so no read of the account before keeps what it brings after.
            self.lists.kept.clear(Some(&credential.username));
            self.catalog
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .forget_metadata();
            self.fetched_songs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
        }
    }

    fn close(&self) {
        {
            // Under the probe lock, so a check of the server is taken wholly before this or not at
            // all, and asks for no sign-in for a library that is closed.
            let _probe = self.probe.lock().unwrap_or_else(PoisonError::into_inner);
            self.check_over.cancel();
        }
        self.followed.store(false, Ordering::Release);
        self.stop_jukebox_polling();
    }
}

impl ServicePlayer for SubsonicBinding {
    fn visualizer_inputs(&self) -> VisualizerInputs {
        if !self.probe().supports_rune_visualizer_feed() {
            return VisualizerInputs::None;
        }
        let lost = self.visualizer_feed.lock().is_ok_and(|feed| {
            feed.as_ref().is_some_and(|active| {
                active
                    .link
                    .lock()
                    .map_or(true, |link| *link == FeedLink::Lost)
            })
        });
        if lost {
            VisualizerInputs::None
        } else {
            VisualizerInputs::Full
        }
    }

    fn visualizer_input_changes(&self) -> watch::Receiver<u64> {
        self.feed_link_changes.subscribe()
    }

    fn follow(self: Arc<Self>) {
        self.followed.store(true, Ordering::Release);
        self.start_jukebox_polling();
    }
}

impl CoverSource for SubsonicClient {
    fn cover_bytes<'a>(&'a self, cover_id: &'a str) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(async move { Ok(self.cover_art(cover_id, None).await?) })
    }
}

struct SubsonicAudioRequest {
    client: Arc<SubsonicClient>,
    track_id: String,
    failure_reason: Arc<std_sync::Mutex<Option<UnavailableReason>>>,
}

impl AudioRequest for SubsonicAudioRequest {
    fn prepare(&self) -> BoxFuture<'static, Result<AudioSourceMode>> {
        let client = Arc::clone(&self.client);
        let track_id = self.track_id.clone();
        let failure_reason = self.failure_reason.clone();
        Box::pin(async move {
            let response = client
                .stream(&track_id, None)
                .await
                .map_err(|error| stream_playback_error(error, &failure_reason))?;
            let supports_range = response
                .headers()
                .get(header::ACCEPT_RANGES)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.eq_ignore_ascii_case("bytes"));
            if supports_range {
                return Ok(AudioSourceMode::Ranged);
            }
            let bytes = response.bytes().await.map_err(|_| {
                if let Ok(mut reason) = failure_reason.lock() {
                    *reason = Some(UnavailableReason::SourceUnreachable);
                }
                Error::new(RemoteAudioError::unreachable("Subsonic server unreachable"))
            })?;
            Ok(AudioSourceMode::Buffered {
                bytes: bytes.to_vec(),
                note: AudioPlaybackNote::RangeUnsupportedBuffered,
            })
        })
    }

    fn get(
        &self,
        range: Option<(u64, Option<u64>)>,
    ) -> BoxFuture<'static, Result<reqwest::Response>> {
        let client = Arc::clone(&self.client);
        let track_id = self.track_id.clone();
        let failure_reason = self.failure_reason.clone();
        Box::pin(async move {
            client
                .stream(&track_id, range)
                .await
                .map_err(|error| stream_playback_error(error, &failure_reason))
        })
    }
}

fn stream_playback_error(
    error: subsonic::StreamError,
    failure_reason: &std_sync::Mutex<Option<UnavailableReason>>,
) -> anyhow::Error {
    let (reason, failure) = match error {
        StreamError::Unreachable => (
            UnavailableReason::SourceUnreachable,
            RemoteAudioError::unreachable("Subsonic server unreachable"),
        ),
        StreamError::CredentialsRejected => (
            UnavailableReason::CredentialsRejected,
            RemoteAudioError::CredentialsRejected,
        ),
        StreamError::TrackGone => (UnavailableReason::TrackGone, RemoteAudioError::TrackGone),
        StreamError::Rejected => (
            UnavailableReason::SourceUnreachable,
            RemoteAudioError::unreachable("Subsonic stream rejected"),
        ),
    };
    if let Ok(mut stored) = failure_reason.lock() {
        *stored = Some(reason);
    }
    Error::new(failure)
}

fn subsonic_playback_error(error: subsonic::SubsonicError) -> anyhow::Error {
    match error {
        SubsonicError::Transport(_) | SubsonicError::HttpStatus(_) => {
            Error::new(RemoteAudioError::unreachable("Subsonic server unreachable"))
        }
        SubsonicError::Server {
            kind:
                SubsonicServerError::WrongCredentials
                | SubsonicServerError::NotAuthorized
                | SubsonicServerError::InvalidApiKey,
            ..
        } => Error::new(RemoteAudioError::CredentialsRejected),
        SubsonicError::Server {
            kind: SubsonicServerError::NotFound,
            ..
        } => Error::new(RemoteAudioError::TrackGone),
        _ => Error::new(RemoteAudioError::TrackGone),
    }
}

impl Drop for SubsonicBinding {
    fn drop(&mut self) {
        self.stop_jukebox_polling();
        self.covers.cancellation.cancel();
    }
}

#[cfg(test)]
mod visualizer_feed_tests {
    use bincode::config;
    use bincode::serde as bincode_serde;
    use reqwest::Client;
    use std::collections::HashMap;
    use std::fs;
    use std::path::Path as StdPath;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use std::time::Duration;
    use tokio::task;
    use tokio::time as tokio_time;

    use crate::request_owner::CurrentPlayer;
    use axum::{
        Router,
        extract::{
            Path, Query, State,
            ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade},
        },
        http::StatusCode,
        response::IntoResponse,
        routing::get,
    };
    use serde::Serialize;
    use tempfile::TempDir;
    use tokio::{net::TcpListener, sync::Notify, time::timeout};
    use url::Url;
    use uuid::Uuid;
    use visualizer::{Visualizer, VisualizerInputs};

    use super::SubsonicBinding;
    use crate::test_wait::TEST_DEADLINE;
    use crate::test_wait::wait_until;
    use crate::{
        backends::remote::{decode_message, encode_message},
        messages::{
            PlaybackMode, PlaybackState, PlaybackStatus, PlayingItemRef, RealtimeFFT,
            VisualizerFeedChart, VisualizerFeedCovers, VisualizerInputs as PushedInputs,
            VisualizerInputsChanged,
        },
        utils::{Broadcaster, RinfRustSignal},
    };
    use subsonic::{SubsonicClient, SubsonicCredentials};

    #[derive(Clone)]
    struct MockState {
        open: bool,
        declares_feed: bool,
        connections: Arc<AtomicUsize>,
        closed: Arc<AtomicBool>,
        auth: Arc<Mutex<Option<HashMap<String, String>>>>,
        requests: Arc<Mutex<Vec<String>>>,
        jukebox: Arc<Mutex<MockJukebox>>,
        feed_status: Arc<Mutex<PlaybackStatus>>,
        feed_release: Arc<Notify>,
        /// Ends the feed connection from the server side.
        feed_drop: Arc<Notify>,
        /// Makes the server answer feed connections with an error.
        refuse_feed: Arc<AtomicBool>,
        cover_calls: Arc<AtomicUsize>,
        cover_started: Arc<Notify>,
        release_cover: Arc<Notify>,
        delay_cover: Arc<AtomicBool>,
        fail_cover_once: Arc<AtomicBool>,
        /// The cover id the jukebox's song carries, if any.
        song_cover: Arc<Mutex<Option<String>>>,
    }

    #[derive(Clone)]
    struct MockJukebox {
        playing: bool,
        current_index: Option<u32>,
        has_song: bool,
        song_id: String,
        title: String,
        duration: u32,
    }

    impl MockJukebox {
        fn song(song_id: &str, title: &str, duration: u32, playing: bool) -> Self {
            Self {
                playing,
                current_index: Some(0),
                has_song: true,
                song_id: song_id.into(),
                title: title.into(),
                duration,
            }
        }
    }

    #[derive(Default)]
    struct Signals {
        names: Mutex<Vec<String>>,
        playback_states: Mutex<Vec<PlaybackState>>,
        playback_statuses: Mutex<Vec<PlaybackStatus>>,
        pushed_inputs: Mutex<Vec<PushedInputs>>,
    }

    #[derive(Serialize)]
    struct FeedChart {
        beats: Vec<FeedBeat>,
        beats_per_bar: f64,
        meter_confidence: f64,
        sections: Vec<FeedSection>,
        song: Vec<FeedSongSection>,
        streams: [Vec<(f64, f64)>; 5],
        lead: Vec<FeedLead>,
        curves: FeedCurves,
        song_feel: f64,
    }

    #[derive(Serialize)]
    struct FeedBeat {
        t: f64,
        bar: i64,
        pos: i64,
        strength: f64,
        confidence: f64,
    }

    #[derive(Serialize)]
    struct FeedSection {
        start: f64,
        end: f64,
        label: f64,
        energy: f64,
        repeat: f64,
        confidence: f64,
        rank: f64,
    }

    #[derive(Serialize)]
    struct FeedSongSection {
        start: f32,
        end: f32,
        level: f32,
    }

    #[derive(Serialize)]
    struct FeedLead {
        start: f64,
        end: f64,
        stream: f64,
    }

    #[derive(Serialize)]
    struct FeedCurves {
        hop: f64,
        loudness: Vec<f64>,
        brightness: Vec<f64>,
        percussive: Vec<f64>,
        beat_conf: Vec<f64>,
        feel: Vec<f64>,
    }

    fn empty_feed_chart() -> Vec<u8> {
        bincode_serde::encode_to_vec(
            FeedChart {
                beats: Vec::new(),
                beats_per_bar: 4.0,
                meter_confidence: 0.0,
                sections: Vec::new(),
                song: Vec::new(),
                streams: [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()],
                lead: Vec::new(),
                curves: FeedCurves {
                    hop: 0.0,
                    loudness: Vec::new(),
                    brightness: Vec::new(),
                    percussive: Vec::new(),
                    beat_conf: Vec::new(),
                    feel: Vec::new(),
                },
                song_feel: 0.0,
            },
            config::standard(),
        )
        .unwrap()
    }

    impl Broadcaster for Signals {
        fn broadcast(&self, message: &dyn RinfRustSignal) {
            self.names.lock().unwrap().push(message.name());
            if let Some(status) = message.as_any().downcast_ref::<PlaybackStatus>() {
                self.playback_states.lock().unwrap().push(status.state);
                self.playback_statuses.lock().unwrap().push(status.clone());
            }
            if let Some(changed) = message.as_any().downcast_ref::<VisualizerInputsChanged>() {
                self.pushed_inputs.lock().unwrap().push(changed.inputs);
            }
        }
    }

    fn default_feed_status() -> PlaybackStatus {
        PlaybackStatus {
            session_id: "feed".into(),
            state: PlaybackState::Playing,
            progress_seconds: 1.0,
            progress_percentage: 0.1,
            artist: Some("feed artist".into()),
            album: Some("feed album".into()),
            title: Some("feed title".into()),
            duration: 10.0,
            index: Some(9),
            item: Some("feed-item".into()),
            item_request: Some(PlayingItemRef::LibraryTrack {
                library_id: Uuid::new_v4().to_string(),
                track: "track-1".into(),
            }),
            playback_mode: PlaybackMode::Shuffle,
            ready: false,
            cover_art_path: Some("/feed/cover".into()),
            lib_path: "/feed/library".into(),
        }
    }

    async fn start_active_binding(
        jukebox: MockJukebox,
        feed_status: PlaybackStatus,
    ) -> (
        MockState,
        Arc<SubsonicBinding>,
        Arc<Signals>,
        TempDir,
        task::JoinHandle<()>,
    ) {
        let (url, state, server) = server(true, true).await;
        *state.jukebox.lock().unwrap() = jukebox;
        *state.feed_status.lock().unwrap() = feed_status;
        let root = TempDir::new().unwrap();
        let signals = Arc::new(Signals::default());
        let binding = open_binding(url, &root, signals.clone()).await;
        binding.start_jukebox_polling();
        let library_id = binding.library_id.to_string();
        wait_until("the initial jukebox poll", || async {
            let statuses = signals.playback_statuses.lock().unwrap();
            Ok(statuses
                .iter()
                .any(|status| status.session_id == library_id)
                .then_some(()))
        })
        .await
        .unwrap();
        (state, binding, signals, root, server)
    }

    async fn server(open: bool, declares_feed: bool) -> (Url, MockState, task::JoinHandle<()>) {
        let state = MockState {
            open,
            declares_feed,
            connections: Arc::new(AtomicUsize::new(0)),
            closed: Arc::new(AtomicBool::new(false)),
            auth: Arc::new(Mutex::new(None)),
            requests: Arc::new(Mutex::new(Vec::new())),
            jukebox: Arc::new(Mutex::new(MockJukebox {
                playing: false,
                current_index: Some(0),
                has_song: true,
                song_id: "track-1".into(),
                title: "song".into(),
                duration: 10,
            })),
            feed_status: Arc::new(Mutex::new(default_feed_status())),
            feed_release: Arc::new(Notify::new()),
            feed_drop: Arc::new(Notify::new()),
            refuse_feed: Arc::new(AtomicBool::new(false)),
            cover_calls: Arc::new(AtomicUsize::new(0)),
            cover_started: Arc::new(Notify::new()),
            release_cover: Arc::new(Notify::new()),
            delay_cover: Arc::new(AtomicBool::new(false)),
            fail_cover_once: Arc::new(AtomicBool::new(false)),
            song_cover: Arc::new(Mutex::new(None)),
        };
        let app = Router::new()
            .route("/rest/{*endpoint}", get(http))
            .route("/feed", get(feed))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (
            Url::parse(&format!("http://{address}")).unwrap(),
            state,
            task,
        )
    }

    async fn http(
        State(state): State<MockState>,
        Path(endpoint): Path<String>,
        Query(query): Query<HashMap<String, String>>,
    ) -> impl IntoResponse {
        let endpoint = endpoint.trim_end_matches(".view");
        if endpoint == "getCoverArt" {
            state.cover_calls.fetch_add(1, Ordering::SeqCst);
            state.cover_started.notify_one();
            if state.delay_cover.load(Ordering::SeqCst) {
                state.release_cover.notified().await;
            }
            if state.fail_cover_once.swap(false, Ordering::SeqCst) {
                return (StatusCode::INTERNAL_SERVER_ERROR, "cover fetch failed").into_response();
            }
            return (
                StatusCode::OK,
                [("content-type", "image/png")],
                vec![1_u8, 2, 3],
            )
                .into_response();
        }
        let response = match endpoint {
            "ping" => format!(r#"{{"subsonic-response":{{"status":"ok","version":"1.16.1","serverVersion":"1","openSubsonic":{}}}}}"#, state.open),
            "getOpenSubsonicExtensions" => {
                let ext = if state.declares_feed { r#",{"name":"runeVisualizerFeed","versions":[1]}"# } else { "" };
                format!(r#"{{"subsonic-response":{{"status":"ok","openSubsonicExtensions":[{{"name":"apiKeyAuthentication","versions":[1]}}{ext}]}}}}"#)
            }
            "getUser" => r#"{"subsonic-response":{"status":"ok","user":{"username":"alice","jukeboxRole":true}}}"#.into(),
            "getRuneVisualizerFeed" => r#"{"subsonic-response":{"status":"ok","runeVisualizerFeed":{"url":"/feed"}}}"#.into(),
            "jukeboxControl" if query.get("action").is_some_and(|action| action == "status") => {
                let jukebox = state.jukebox.lock().unwrap().clone();
                let current_index = jukebox.current_index.map(|index| format!(r#", "currentIndex": {index}"#)).unwrap_or_default();
                format!(r#"{{"subsonic-response":{{"status":"ok","jukeboxStatus":{{"playing":{},"position":0{current_index}}}}}}}"#, jukebox.playing)
            }
            "jukeboxControl" => {
                let jukebox = state.jukebox.lock().unwrap().clone();
                let current_index = jukebox.current_index.map(|index| format!(r#", "currentIndex": {index}"#)).unwrap_or_default();
                let cover = state.song_cover.lock().unwrap().as_ref().map(|id| format!(r#","coverArt":"{id}""#)).unwrap_or_default();
                let entry = if jukebox.has_song { format!(r#", "entry": {{"id":"{}","title":"{}","duration":{}{cover}}}"#, jukebox.song_id, jukebox.title, jukebox.duration) } else { String::new() };
                format!(r#"{{"subsonic-response":{{"status":"ok","jukeboxPlaylist":{{"playing":{},"position":0{current_index}{entry}}}}}}}"#, jukebox.playing)
            }
            _ => r#"{"subsonic-response":{"status":"ok","jukeboxStatus":{"playing":false}}}"#.into(),
        };
        (StatusCode::OK, response).into_response()
    }

    async fn feed(
        State(state): State<MockState>,
        Query(query): Query<HashMap<String, String>>,
        ws: WebSocketUpgrade,
    ) -> impl IntoResponse {
        if state.refuse_feed.load(Ordering::SeqCst) {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        *state.auth.lock().unwrap() = Some(query);
        ws.on_upgrade(move |socket| async move {
            let dropped = state.feed_drop.clone();
            tokio::select! {
                _ = dropped.notified() => {}
                _ = feed_socket(socket, state) => {}
            }
        })
        .into_response()
    }

    async fn feed_socket(mut socket: WebSocket, state: MockState) {
        state.connections.fetch_add(1, Ordering::SeqCst);
        state.feed_release.notified().await;
        let status = state.feed_status.lock().unwrap().clone();
        let item = status
            .item_request
            .clone()
            .unwrap_or_else(|| PlayingItemRef::LibraryTrack {
                library_id: Uuid::new_v4().to_string(),
                track: "track-1".into(),
            });
        let events = [
            event("PlaybackStatus", &status),
            event(
                "VisualizerFeedChart",
                &VisualizerFeedChart {
                    item: item.clone(),
                    chart: empty_feed_chart(),
                },
            ),
            event(
                "VisualizerFeedCovers",
                &VisualizerFeedCovers {
                    item,
                    playing: Some(vec![4]),
                    random: vec![vec![5]],
                },
            ),
            event(
                "RealtimeFFT",
                &RealtimeFFT {
                    value: vec![0.25, 0.5],
                },
            ),
        ];
        for event in events {
            if socket
                .send(AxumMessage::Binary(event.into()))
                .await
                .is_err()
            {
                return;
            }
        }
        while let Some(Ok(message)) = socket.recv().await {
            if let AxumMessage::Binary(payload) = &message
                && let Some((name, _, _)) = decode_message(payload)
            {
                state.requests.lock().unwrap().push(name);
            }
            if matches!(message, AxumMessage::Close(_)) {
                state.closed.store(true, Ordering::SeqCst);
                break;
            }
        }
        state.closed.store(true, Ordering::SeqCst);
    }

    fn event<T: Serialize>(name: &str, value: &T) -> Vec<u8> {
        encode_message(name, &rinf::serialize(value).unwrap(), None)
    }

    async fn open_binding(
        url: Url,
        root: &TempDir,
        broadcaster: Arc<Signals>,
    ) -> Arc<SubsonicBinding> {
        open_binding_for(url, root, broadcaster, Arc::new(Visualizer::new())).await
    }

    /// A binding whose feed writes `visualizer`, as a node's does.
    async fn open_binding_for(
        url: Url,
        root: &TempDir,
        broadcaster: Arc<Signals>,
        visualizer: Arc<Visualizer>,
    ) -> Arc<SubsonicBinding> {
        let client = SubsonicClient::new(
            url,
            SubsonicCredentials::Password {
                user: "alice".into(),
                password: "secret".into(),
            },
            Client::new(),
        );
        let probe = client.probe().await.unwrap();
        Arc::new(SubsonicBinding::new(
            client,
            probe,
            root.path().to_path_buf(),
            Uuid::new_v4(),
            false,
            broadcaster,
            visualizer,
        ))
    }

    #[tokio::test]
    async fn cancelled_cover_fetch_does_not_block_the_next_request() {
        let (url, state, server) = server(true, false).await;
        state.delay_cover.store(true, Ordering::SeqCst);
        let root = TempDir::new().unwrap();
        let binding = open_binding(url, &root, Arc::new(Signals::default())).await;
        let first_cache = binding.covers.clone();
        let first_client = binding.client.clone();
        let first =
            tokio::spawn(async move { first_cache.ensure_cached(first_client, "cover-1").await });
        timeout(TEST_DEADLINE, state.cover_started.notified())
            .await
            .unwrap();
        first.abort();
        let _ = first.await;

        let next_cache = binding.covers.clone();
        let next_client = binding.client.clone();
        let next =
            tokio::spawn(async move { next_cache.ensure_cached(next_client, "cover-1").await });
        state.release_cover.notify_one();
        state.release_cover.notify_one();
        let result = timeout(TEST_DEADLINE, next)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(result.is_some());
        assert_eq!(state.cover_calls.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn concurrent_cover_requests_share_one_fetch() {
        let (url, state, server) = server(true, false).await;
        state.delay_cover.store(true, Ordering::SeqCst);
        let root = TempDir::new().unwrap();
        let binding = open_binding(url, &root, Arc::new(Signals::default())).await;
        let first_cache = binding.covers.clone();
        let first_client = binding.client.clone();
        let first =
            tokio::spawn(async move { first_cache.ensure_cached(first_client, "cover-1").await });
        timeout(TEST_DEADLINE, state.cover_started.notified())
            .await
            .unwrap();
        let second_cache = binding.covers.clone();
        let second_client = binding.client.clone();
        let second =
            tokio::spawn(async move { second_cache.ensure_cached(second_client, "cover-1").await });
        state.release_cover.notify_one();
        state.release_cover.notify_one();
        let first_result = timeout(TEST_DEADLINE, first)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let second_result = timeout(TEST_DEADLINE, second)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(first_result.is_some());
        assert_eq!(first_result, second_result);
        assert_eq!(state.cover_calls.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn failed_cover_fetch_is_retried_by_the_next_request() {
        let (url, state, server) = server(true, false).await;
        state.fail_cover_once.store(true, Ordering::SeqCst);
        let root = TempDir::new().unwrap();
        let binding = open_binding(url, &root, Arc::new(Signals::default())).await;
        assert!(
            binding
                .covers
                .ensure_cached(binding.client.clone(), "cover-1")
                .await
                .is_err()
        );
        let result = binding
            .covers
            .ensure_cached(binding.client.clone(), "cover-1")
            .await
            .unwrap();
        assert!(result.is_some());
        assert_eq!(state.cover_calls.load(Ordering::SeqCst), 2);
        server.abort();
    }

    #[tokio::test]
    async fn jukebox_status_carries_the_cover_once_it_is_cached() {
        let (url, state, server) = server(true, false).await;
        *state.song_cover.lock().unwrap() = Some("cover-1".into());
        state.delay_cover.store(true, Ordering::SeqCst);
        let root = TempDir::new().unwrap();
        let signals = Arc::new(Signals::default());
        let binding = open_binding(url, &root, signals.clone()).await;
        binding.start_jukebox_polling();

        // The status goes out while the cover fetch is still held back.
        wait_until("a jukebox status for the song", || async {
            Ok(signals
                .playback_statuses
                .lock()
                .unwrap()
                .iter()
                .any(|status| status.title.as_deref() == Some("song"))
                .then_some(()))
        })
        .await
        .unwrap();
        assert!(
            signals
                .playback_statuses
                .lock()
                .unwrap()
                .iter()
                .all(|status| status.cover_art_path.is_none())
        );

        state.release_cover.notify_one();
        let expected = binding.covers.path_for("cover-1");
        let path = wait_until("a jukebox status with the cover", || async {
            Ok(signals
                .playback_statuses
                .lock()
                .unwrap()
                .iter()
                .find_map(|status| status.cover_art_path.clone()))
        })
        .await
        .unwrap();
        assert_eq!(StdPath::new(&path), expected);
        assert!(fs::metadata(&path).unwrap().len() > 0);
        assert_eq!(state.cover_calls.load(Ordering::SeqCst), 1);

        binding.stop_jukebox_polling();
        binding.covers.cancellation.cancel();
        server.abort();
    }

    #[tokio::test]
    async fn jukebox_visualizer_inputs_follow_the_mock_probe_declaration() {
        for (declares_feed, expected) in [
            (true, VisualizerInputs::Full),
            (false, VisualizerInputs::None),
        ] {
            let (url, _, server) = server(true, declares_feed).await;
            let root = TempDir::new().unwrap();
            let binding = open_binding(url, &root, Arc::new(Signals::default())).await;
            let player = CurrentPlayer::ServicePlayer(binding.clone());
            assert_eq!(player.visualizer_inputs(), expected);
            binding.stop_jukebox_polling();
            server.abort();
        }
    }

    #[tokio::test]
    async fn jukebox_broadcast_state_distinguishes_playing_paused_and_stopped() {
        let (url, state, server) = server(true, false).await;
        let root = TempDir::new().unwrap();
        let signals = Arc::new(Signals::default());
        let binding = open_binding(url, &root, signals.clone()).await;
        binding.start_jukebox_polling();

        async fn wait_for_state(
            signals: &Signals,
            previous_count: usize,
            expected: PlaybackState,
        ) -> anyhow::Result<usize> {
            wait_until("jukebox playback state", || async {
                let (count, actual) = {
                    let states = signals.playback_states.lock().unwrap();
                    (states.len(), states.last().copied())
                };
                if count > previous_count {
                    assert!(actual == Some(expected));
                    Ok(Some(count))
                } else {
                    Ok(None)
                }
            })
            .await
        }

        let count = wait_for_state(&signals, 0, PlaybackState::Paused)
            .await
            .unwrap();
        *state.jukebox.lock().unwrap() = MockJukebox {
            playing: true,
            current_index: Some(0),
            has_song: true,
            song_id: "track-1".into(),
            title: "song".into(),
            duration: 10,
        };
        let count = wait_for_state(&signals, count, PlaybackState::Playing)
            .await
            .unwrap();
        *state.jukebox.lock().unwrap() = MockJukebox {
            playing: false,
            current_index: None,
            has_song: false,
            song_id: "track-1".into(),
            title: "song".into(),
            duration: 10,
        };
        let count = wait_for_state(&signals, count, PlaybackState::Stopped)
            .await
            .unwrap();
        *state.jukebox.lock().unwrap() = MockJukebox {
            playing: false,
            current_index: Some(7),
            has_song: true,
            song_id: "track-1".into(),
            title: "song".into(),
            duration: 10,
        };
        wait_for_state(&signals, count, PlaybackState::Stopped)
            .await
            .unwrap();

        binding.stop_jukebox_polling();
        binding.covers.cancellation.cancel();
        server.abort();
    }

    #[tokio::test]
    async fn subsonic_visualizer_feed_delivers_events_authenticates_and_closes_with_library() {
        let (url, state, server) = server(true, true).await;
        let root = TempDir::new().unwrap();
        let signals = Arc::new(Signals::default());
        let binding = open_binding(url.clone(), &root, signals.clone()).await;
        binding.start_jukebox_polling();
        wait_until("visualizer feed connection", || async {
            Ok((state.connections.load(Ordering::SeqCst) == 1).then_some(()))
        })
        .await
        .unwrap();
        let library_id = binding.library_id.to_string();
        wait_until("initial jukebox status", || async {
            let statuses = signals.playback_statuses.lock().unwrap();
            Ok(statuses
                .iter()
                .any(|status| status.session_id == library_id)
                .then_some(()))
        })
        .await
        .unwrap();
        state.feed_release.notify_one();
        assert_eq!(state.auth.lock().unwrap().as_ref().unwrap()["u"], "alice");
        assert!(
            state
                .auth
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains_key("t")
        );
        // Which charts and covers a feed's visualizer keeps is the visualizer crate's to test.
        wait_until("feed events reached the binding's visualizer", || async {
            let names = signals.names.lock().unwrap();
            let fft = names.iter().any(|name| name == "RealtimeFFT");
            Ok((fft && binding.visualizer.is_feed_playback()).then_some(()))
        })
        .await
        .unwrap();
        assert!(
            !signals
                .playback_statuses
                .lock()
                .unwrap()
                .iter()
                .any(|status| status.session_id == "feed")
        );
        {
            let names = signals.names.lock().unwrap();
            assert!(names.iter().any(|name| name == "RealtimeFFT"));
            assert!(!names.iter().any(|name| name == "VisualizerFeedChart"));
            assert!(!names.iter().any(|name| name == "VisualizerFeedCovers"));
        }
        binding.stop_jukebox_polling();
        wait_until("visualizer feed close", || async {
            Ok(state.closed.load(Ordering::SeqCst).then_some(()))
        })
        .await
        .unwrap();
        wait_until("visualizer feed unsubscribe request", || async {
            let requests = state.requests.lock().unwrap();
            Ok(requests
                .iter()
                .any(|name| name == "UnsubscribeVisualizerFeedRequest")
                .then_some(()))
        })
        .await
        .unwrap();
        let requests = state.requests.lock().unwrap();
        assert!(
            requests
                .iter()
                .any(|name| name == "SubscribeVisualizerFeedRequest")
        );
        assert!(
            requests
                .iter()
                .any(|name| name == "SetRealtimeFFTEnabledRequest")
        );
        assert!(
            requests
                .iter()
                .any(|name| name == "UnsubscribeVisualizerFeedRequest")
        );
        drop(requests);
        binding.covers.cancellation.cancel();
        server.abort();
    }

    #[tokio::test]
    async fn feed_playback_status_keeps_the_polled_jukebox_identity() {
        let feed_status = PlaybackStatus {
            session_id: "third-party-session".into(),
            state: PlaybackState::Paused,
            progress_seconds: 8.0,
            progress_percentage: 0.8,
            artist: Some("third-party-artist".into()),
            album: Some("third-party-album".into()),
            title: Some("third-party-title".into()),
            duration: 99.0,
            index: Some(99),
            item: Some("third-party-item".into()),
            item_request: Some(PlayingItemRef::LibraryTrack {
                library_id: "third-party-library".into(),
                track: "song-a".into(),
            }),
            playback_mode: PlaybackMode::Shuffle,
            ready: false,
            cover_art_path: Some("/third-party/cover".into()),
            lib_path: "/third-party/library".into(),
        };
        let (state, binding, signals, _root, server) =
            start_active_binding(MockJukebox::song("song-a", "Song A", 20, true), feed_status)
                .await;
        let before = signals.playback_statuses.lock().unwrap().len();
        state.feed_release.notify_one();
        let status = wait_until("binding status updated from matching feed", || async {
            let statuses = signals.playback_statuses.lock().unwrap();
            Ok((statuses.len() > before).then_some(statuses.last().unwrap().clone()))
        })
        .await
        .unwrap();
        assert_eq!(status.session_id, binding.library_id.to_string());
        assert!(status.state == PlaybackState::Paused);
        assert_eq!(status.progress_seconds, 8.0);
        assert_eq!(status.progress_percentage, 0.4);
        assert_eq!(status.artist, None);
        assert_eq!(status.album, None);
        assert_eq!(status.title.as_deref(), Some("Song A"));
        assert_eq!(status.duration, 20.0);
        assert_eq!(status.index, Some(0));
        assert_eq!(status.item.as_deref(), Some("song-a"));
        assert!(
            status.item_request
                == Some(PlayingItemRef::LibraryTrack {
                    library_id: binding.library_id.to_string(),
                    track: "song-a".into(),
                })
        );
        assert_eq!(status.playback_mode, PlaybackMode::Sequential);
        assert!(status.ready);
        assert_eq!(status.cover_art_path, None);
        assert_eq!(status.lib_path, "");
        binding.stop_jukebox_polling();
        binding.covers.cancellation.cancel();
        server.abort();
    }

    #[tokio::test]
    async fn feed_playback_status_for_a_different_track_does_not_change_jukebox_status() {
        let mut feed_status = default_feed_status();
        feed_status.session_id = "third-party-session".into();
        feed_status.item_request = Some(PlayingItemRef::LibraryTrack {
            library_id: "third-party-library".into(),
            track: "song-b".into(),
        });
        let (state, binding, signals, _root, server) =
            start_active_binding(MockJukebox::song("song-a", "Song A", 20, true), feed_status)
                .await;
        let before = signals.playback_statuses.lock().unwrap().len();
        state.feed_release.notify_one();
        wait_until("feed FFT after mismatched track status", || async {
            let names = signals.names.lock().unwrap();
            Ok(names.iter().any(|name| name == "RealtimeFFT").then_some(()))
        })
        .await
        .unwrap();
        assert_eq!(signals.playback_statuses.lock().unwrap().len(), before);
        binding.stop_jukebox_polling();
        binding.covers.cancellation.cancel();
        server.abort();
    }

    async fn feed_fft_count(signals: &Signals) -> usize {
        signals
            .names
            .lock()
            .unwrap()
            .iter()
            .filter(|name| *name == "RealtimeFFT")
            .count()
    }

    #[tokio::test]
    async fn feed_reconnects_after_the_server_closes_it() {
        let (url, state, server) = server(true, true).await;
        let root = TempDir::new().unwrap();
        let signals = Arc::new(Signals::default());
        let binding = open_binding(url, &root, signals.clone()).await;
        binding.start_jukebox_polling();
        wait_until("the first feed connection", || async {
            Ok((state.connections.load(Ordering::SeqCst) == 1).then_some(()))
        })
        .await
        .unwrap();
        state.feed_release.notify_one();
        wait_until("feed data from the first connection", || async {
            Ok((feed_fft_count(&signals).await >= 1).then_some(()))
        })
        .await
        .unwrap();
        let before = feed_fft_count(&signals).await;

        state.feed_drop.notify_one();
        wait_until("the second feed connection", || async {
            Ok((state.connections.load(Ordering::SeqCst) == 2).then_some(()))
        })
        .await
        .unwrap();
        state.feed_release.notify_one();
        wait_until("feed data from the second connection", || async {
            Ok((feed_fft_count(&signals).await > before).then_some(()))
        })
        .await
        .unwrap();

        binding.stop_jukebox_polling();
        binding.covers.cancellation.cancel();
        server.abort();
    }

    #[tokio::test]
    async fn visualizer_inputs_drop_while_the_feed_is_down_and_return_after_it() {
        let (url, state, server) = server(true, true).await;
        let root = TempDir::new().unwrap();
        let binding = open_binding(url, &root, Arc::new(Signals::default())).await;
        let player = CurrentPlayer::ServicePlayer(binding.clone());
        binding.start_jukebox_polling();
        wait_until("the feed connection", || async {
            Ok((state.connections.load(Ordering::SeqCst) == 1).then_some(()))
        })
        .await
        .unwrap();
        assert_eq!(player.visualizer_inputs(), VisualizerInputs::Full);

        state.refuse_feed.store(true, Ordering::SeqCst);
        state.feed_drop.notify_one();
        wait_until("inputs to leave Full while the feed is down", || async {
            Ok((player.visualizer_inputs() == VisualizerInputs::None).then_some(()))
        })
        .await
        .unwrap();

        state.refuse_feed.store(false, Ordering::SeqCst);
        wait_until("inputs to return once the feed is back", || async {
            Ok((player.visualizer_inputs() == VisualizerInputs::Full).then_some(()))
        })
        .await
        .unwrap();
        assert_eq!(state.connections.load(Ordering::SeqCst), 2);

        binding.stop_jukebox_polling();
        binding.covers.cancellation.cancel();
        server.abort();
    }

    /// The node publishes the jukebox's inputs to the visualizer, so a feed that
    /// drops or returns must reach the visualizer through the node.
    #[tokio::test]
    async fn node_publishes_the_jukebox_inputs_as_the_feed_drops_and_returns() {
        use crate::node::NodeContext;
        use crate::server::utils::device::RunMode;

        let (url, state, server) = server(true, true).await;
        let node_dir = TempDir::new().unwrap();
        let path = node_dir.path().to_string_lossy().into_owned();
        crate::test_node::disable_sync_listener(node_dir.path()).unwrap();
        let node = NodeContext::new(
            &path,
            &path,
            Arc::new(Signals::default()),
            false,
            RunMode::App,
            Arc::new(Visualizer::new()),
        )
        .await
        .unwrap();
        let root = TempDir::new().unwrap();
        let binding = open_binding_for(
            url,
            &root,
            Arc::new(Signals::default()),
            Arc::clone(node.visualizer()),
        )
        .await;
        node.set_open_service_catalog(None, binding.clone())
            .await
            .unwrap();
        assert!(matches!(
            node.current_player().unwrap(),
            CurrentPlayer::ServicePlayer(_)
        ));
        wait_until("the feed connection", || async {
            Ok((state.connections.load(Ordering::SeqCst) == 1).then_some(()))
        })
        .await
        .unwrap();
        wait_until("the visualizer to see the live feed", || async {
            Ok((node.visualizer().status().inputs == VisualizerInputs::Full).then_some(()))
        })
        .await
        .unwrap();

        state.refuse_feed.store(true, Ordering::SeqCst);
        state.feed_drop.notify_one();
        wait_until("the visualizer to lose its inputs", || async {
            Ok((node.visualizer().status().inputs == VisualizerInputs::None).then_some(()))
        })
        .await
        .unwrap();

        state.refuse_feed.store(false, Ordering::SeqCst);
        wait_until("the visualizer to get its inputs back", || async {
            Ok((node.visualizer().status().inputs == VisualizerInputs::Full).then_some(()))
        })
        .await
        .unwrap();

        node.close_ui(&node.token).await;
        node.shutdown().await.unwrap();
        server.abort();
    }

    /// The UI follows what the jukebox's feed gives the visualizer, as the node tells it each
    /// time that changes, also while no visualizer runs to be asked.
    #[tokio::test]
    async fn the_ui_hears_the_jukebox_inputs_as_the_feed_drops_and_returns() {
        use crate::node::NodeContext;
        use crate::server::utils::device::RunMode;

        let (url, state, server) = server(true, true).await;
        let node_dir = TempDir::new().unwrap();
        let path = node_dir.path().to_string_lossy().into_owned();
        crate::test_node::disable_sync_listener(node_dir.path()).unwrap();
        let signals = Arc::new(Signals::default());
        let node = NodeContext::new(
            &path,
            &path,
            signals.clone(),
            false,
            RunMode::App,
            Arc::new(Visualizer::new()),
        )
        .await
        .unwrap();
        let root = TempDir::new().unwrap();
        let binding = open_binding_for(
            url,
            &root,
            Arc::new(Signals::default()),
            Arc::clone(node.visualizer()),
        )
        .await;
        node.set_open_service_catalog(None, binding.clone())
            .await
            .unwrap();
        let last_pushed = || signals.pushed_inputs.lock().unwrap().last().copied();
        // The feed that drops is one the server took.
        wait_until("the feed connection", || async {
            Ok((state.connections.load(Ordering::SeqCst) == 1).then_some(()))
        })
        .await
        .unwrap();
        wait_until("the UI to hear the live feed", || async {
            Ok((last_pushed() == Some(PushedInputs::Full)).then_some(()))
        })
        .await
        .unwrap();

        state.refuse_feed.store(true, Ordering::SeqCst);
        state.feed_drop.notify_one();
        wait_until("the UI to hear that the feed is gone", || async {
            Ok((last_pushed() == Some(PushedInputs::None)).then_some(()))
        })
        .await
        .unwrap();

        state.refuse_feed.store(false, Ordering::SeqCst);
        wait_until("the UI to hear that the feed is back", || async {
            Ok((last_pushed() == Some(PushedInputs::Full)).then_some(()))
        })
        .await
        .unwrap();
        assert_eq!(state.connections.load(Ordering::SeqCst), 2);
        // Each change is told once, however often the inputs were published again.
        let pushed = signals.pushed_inputs.lock().unwrap().clone();
        assert!(
            pushed.windows(2).all(|pair| pair[0] != pair[1]),
            "{pushed:?}"
        );

        node.close_ui(&node.token).await;
        node.shutdown().await.unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn feed_connect_failure_is_retried() {
        let (url, state, server) = server(true, true).await;
        state.refuse_feed.store(true, Ordering::SeqCst);
        let root = TempDir::new().unwrap();
        let binding = open_binding(url, &root, Arc::new(Signals::default())).await;
        let player = CurrentPlayer::ServicePlayer(binding.clone());
        binding.start_jukebox_polling();
        wait_until("inputs to leave Full after the failed connect", || async {
            Ok((player.visualizer_inputs() == VisualizerInputs::None).then_some(()))
        })
        .await
        .unwrap();
        assert_eq!(state.connections.load(Ordering::SeqCst), 0);

        state.refuse_feed.store(false, Ordering::SeqCst);
        wait_until("the retried feed connection", || async {
            Ok((state.connections.load(Ordering::SeqCst) == 1).then_some(()))
        })
        .await
        .unwrap();
        wait_until("inputs to return", || async {
            Ok((player.visualizer_inputs() == VisualizerInputs::Full).then_some(()))
        })
        .await
        .unwrap();

        binding.stop_jukebox_polling();
        binding.covers.cancellation.cancel();
        server.abort();
    }

    #[tokio::test]
    async fn subsonic_visualizer_feed_is_not_connected_without_open_subsonic_declaration() {
        for (open, declares_feed) in [(false, true), (true, false)] {
            let (url, state, server) = server(open, declares_feed).await;
            let root = TempDir::new().unwrap();
            let binding = open_binding(url, &root, Arc::new(Signals::default())).await;
            binding.start_jukebox_polling();
            tokio_time::sleep(Duration::from_millis(100)).await;
            assert_eq!(state.connections.load(Ordering::SeqCst), 0);
            binding.stop_jukebox_polling();
            binding.covers.cancellation.cancel();
            server.abort();
        }
    }

    /// A rejection of the sign-in the server check found, as the server answers it.
    fn rejected() -> subsonic::SubsonicError {
        subsonic::SubsonicError::Server {
            kind: subsonic::SubsonicServerError::WrongCredentials,
            message: "Wrong username or password".into(),
        }
    }

    fn asked_for_sign_in(signals: &Signals) -> bool {
        signals
            .names
            .lock()
            .unwrap()
            .iter()
            .any(|name| name == "RequestFailedResponse")
    }

    #[tokio::test]
    async fn a_library_closed_before_its_server_check_ends_takes_nothing_from_it() {
        use crate::service::ServiceBinding;

        let (url, _state, server) = server(true, false).await;
        let root = TempDir::new().unwrap();
        let signals = Arc::new(Signals::default());
        let binding = open_binding(url, &root, signals.clone()).await;
        binding.close();
        assert!(!binding.take_check(Err(rejected())));
        assert!(!asked_for_sign_in(&signals));
        server.abort();
    }

    #[tokio::test]
    async fn a_server_check_refused_with_http_401_or_403_asks_for_a_sign_in() {
        use reqwest::StatusCode as HttpStatus;

        for status in [HttpStatus::UNAUTHORIZED, HttpStatus::FORBIDDEN] {
            let (url, _state, server) = server(true, false).await;
            let root = TempDir::new().unwrap();
            let signals = Arc::new(Signals::default());
            let binding = open_binding(url, &root, signals.clone()).await;
            binding.take_check(Err(subsonic::SubsonicError::HttpStatus(status)));
            assert!(asked_for_sign_in(&signals), "{status} asked for no sign-in");
            server.abort();
        }
    }

    #[tokio::test]
    async fn a_feed_the_server_check_finds_declared_starts_for_the_followed_jukebox() {
        use crate::service::ServicePlayer;

        let (url, state, server) = server(true, true).await;
        let root = TempDir::new().unwrap();
        let binding = open_binding(url, &root, Arc::new(Signals::default())).await;
        let declared = binding.client.probe().await.unwrap();
        // The library opens from a probe kept before the server declared the feed.
        binding
            .probe
            .lock()
            .unwrap()
            .extensions
            .retain(|(name, _)| name != "runeVisualizerFeed");
        Arc::clone(&binding).follow();
        tokio_time::sleep(Duration::from_millis(100)).await;
        assert_eq!(state.connections.load(Ordering::SeqCst), 0);

        binding.take_check(Ok(declared));
        wait_until("the feed of the followed jukebox", || async {
            Ok((state.connections.load(Ordering::SeqCst) == 1).then_some(()))
        })
        .await
        .unwrap();
        binding.stop_jukebox_polling();
        server.abort();
    }

    /// #895: a cover whose download ends after its library's record is removed must not bring
    /// the library's folder back.
    #[tokio::test]
    async fn a_cover_download_that_ends_after_its_record_is_removed_leaves_no_folder() {
        use crate::Signal;
        use crate::messages::RemoveLibraryRecordRequest;
        use crate::node::NodeContext;
        use crate::server::utils::device::RunMode;
        use crate::service::folder::service_library_folder;
        use database::credentials::{Credential, Secret};
        use database::library_registry::{
            CatalogPlacement, LibraryRecord, LibraryRecordId, LibraryRegistry, ProtocolAdapter,
            SubsonicOpenMode,
        };

        let (url, state, server) = server(true, false).await;
        let root = TempDir::new().unwrap();
        let config = root.path().to_string_lossy().into_owned();
        crate::test_node::disable_sync_listener(root.path()).unwrap();
        let node = NodeContext::new(
            &config,
            &config,
            Arc::new(Signals::default()),
            false,
            RunMode::App,
            Arc::new(Visualizer::new()),
        )
        .await
        .unwrap();
        let binding = open_binding(url.clone(), &root, Arc::new(Signals::default())).await;
        let library_id = binding.library_id;
        node.register_cover_cache(library_id, &binding.covers);
        let credential = node
            .credentials
            .put(Credential {
                username: "alice".into(),
                secret: Secret::new(b"secret".to_vec()),
            })
            .await
            .unwrap();
        LibraryRegistry::update(root.path(), |registry| {
            registry.libraries.push(LibraryRecord {
                record_id: LibraryRecordId(library_id),
                alias: "Subsonic".into(),
                library_id: None,
                protocol: ProtocolAdapter::SubsonicTranslate {
                    endpoint: url.to_string(),
                    credential,
                    mode: SubsonicOpenMode::PlayOnThisDevice,
                },
                fs: Vec::new(),
                catalog: CatalogPlacement::None,
                background_sync: false,
            });
            Ok(())
        })
        .await
        .unwrap();

        // The mock holds the cover's response while the record is removed.
        state.delay_cover.store(true, Ordering::SeqCst);
        let download = tokio::spawn({
            let (covers, client) = (Arc::clone(&binding.covers), Arc::clone(&binding.client));
            async move { covers.ensure_cached(client, "cover").await }
        });
        timeout(TEST_DEADLINE, state.cover_started.notified())
            .await
            .expect("the cover request reaches the server");
        let request = RemoveLibraryRecordRequest {
            call_id: None,
            record_id: library_id.to_string(),
        };
        let removed = tokio::spawn({
            let node = Arc::clone(&node);
            async move { request.handle(node, None, &request).await }
        });
        // The server answers only now, after the removal began.
        state.release_cover.notify_one();
        timeout(TEST_DEADLINE, removed)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let _ = timeout(TEST_DEADLINE, download).await.unwrap().unwrap();

        assert!(!service_library_folder(root.path(), library_id).exists());
        server.abort();
    }

    #[tokio::test]
    async fn a_jukebox_whose_role_the_server_check_finds_taken_stops_being_followed() {
        use crate::service::ServicePlayer;

        let (url, state, server) = server(true, true).await;
        let root = TempDir::new().unwrap();
        let binding = open_binding(url, &root, Arc::new(Signals::default())).await;
        Arc::clone(&binding).follow();
        wait_until("the feed of the followed jukebox", || async {
            Ok((state.connections.load(Ordering::SeqCst) == 1).then_some(()))
        })
        .await
        .unwrap();
        state.feed_release.notify_one();

        let mut taken = binding.probe();
        taken.roles.jukebox_role = false;
        binding.take_check(Ok(taken));
        wait_until("the feed closed", || async {
            Ok(state.closed.load(Ordering::SeqCst).then_some(()))
        })
        .await
        .unwrap();
        assert!(binding.jukebox_poll.lock().unwrap().is_none());
        server.abort();
    }
}
