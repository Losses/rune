use super::*;
use crate::{
    request_owner::CurrentPlayer, server::utils::device::RunMode, service::registry::ServiceSetup,
    test_wait::wait_until, utils::RinfRustSignal,
};
use database::library_registry::ServiceKind;
use playback::pcm_tap::PcmTap;
use playback::player::{
    ClearHandoffBlocksResult, HandoffLoadResult, MockPlayer, PausedPlayback, PlaybackEvent,
    PlayerCommandError,
};
use playback::strategies::AddMode;
use playback::{InternalLog, PlayerCommand, SimpleReceiver};
use simple_channel::{SimpleChannel, SimpleSender};
use std::fs;
use std::result;
use visualizer::Visualizer;

struct Quiet;
impl Broadcaster for Quiet {
    fn broadcast(&self, _: &dyn RinfRustSignal) {}
}

#[tokio::test]
async fn now_playing_waits_for_each_tracks_first_start() -> Result<()> {
    let dir = tempfile::tempdir()?;
    crate::test_node::disable_media_server(dir.path())?;
    let path = dir.path().to_string_lossy();
    let broadcaster: Arc<dyn Broadcaster> = Arc::new(Quiet);
    let node = NodeContext::new(
        &path,
        &path,
        broadcaster.clone(),
        false,
        RunMode::App,
        Arc::new(Visualizer::new()),
    )
    .await?;
    let (now_playing_tx, mut now_playing_rx) = mpsc::unbounded_channel();
    let mut recorded = Vec::new();
    let mut item_state = ItemEventState::default();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
    let mut tracks = Vec::new();
    for name in ["first.ogg", "second.ogg"] {
        let file = dir.path().join(name);
        fs::copy(&fixture, &file)?;
        let item = PlayingItem::IndependentFile(file.to_string_lossy().into_owned());
        let (metadata, _) = item_metadata(&node, &item, false).await?;
        tracks.push((item, metadata.title));
    }
    let (first, first_title) = &tracks[0];
    let (second, second_title) = &tracks[1];
    assert_ne!(first_title, second_title);

    use PlaybackState::{Paused, Playing};
    for (step, item, state, expected) in [
        ("paused load", first, Paused, &[][..]),
        ("output open failed; still paused", first, Paused, &[]),
        ("first successful start", first, Playing, &[first_title]),
        ("pause", first, Paused, &[first_title]),
        ("resume", first, Playing, &[first_title]),
        ("switch while paused", second, Paused, &[first_title]),
        (
            "new track starts",
            second,
            Playing,
            &[first_title, second_title],
        ),
    ] {
        handle_status_event(
            &node,
            PlayerStatus {
                item: Some(item.clone()),
                index: Some(0),
                path: None,
                position: Duration::ZERO,
                state,
                playlist: vec![item.clone().into()],
                unavailable: Vec::new(),
                unavailable_reasons: HashMap::new(),
                playback_mode: PlaybackMode::Sequential,
                ready: true,
                volume: 1.0,
                hosted_by: None,
                host_shows_in_media_controls: false,
                host_buffering: false,
            },
            &mut item_state,
            &None,
            Some(&now_playing_tx),
            &broadcaster,
            "test",
        )
        .await;
        while let Ok(track) = now_playing_rx.try_recv() {
            recorded.push(track.track);
        }
        assert!(
            recorded.iter().eq(expected.iter().copied()),
            "{step}: expected {expected:?}, got {recorded:?}"
        );
    }
    node.shutdown().await?;
    Ok(())
}

/// The system shows a position against the metadata it was last given, so the position of an item
/// not named yet waits for its name, which would otherwise go out against the item before. Its
/// state goes out at once, so that a pause shows even when the item is never named.
#[test]
fn the_position_of_an_item_waits_for_its_name_and_its_state_does_not() {
    use PlaybackState::{Paused, Playing, Stopped};

    let item = PlayingItem::LibraryTrack {
        library_id: Uuid::new_v4(),
        track: "slow".to_owned(),
    };
    let status = |state| PlayerStatus {
        item: Some(item.clone()),
        index: Some(0),
        path: None,
        position: Duration::from_secs(7),
        state,
        playlist: vec![item.clone().into()],
        unavailable: Vec::new(),
        unavailable_reasons: HashMap::new(),
        playback_mode: PlaybackMode::Sequential,
        ready: true,
        volume: 1.0,
        hosted_by: None,
        host_shows_in_media_controls: false,
        host_buffering: false,
    };
    let at_seven = |progress: Option<MediaPosition>| {
        progress.is_some_and(|position| position.0 == Duration::from_secs(7))
    };
    assert!(matches!(
        now_playing_playback(&status(Playing), true),
        (MediaPlayback::Playing { progress }, false) if at_seven(progress)
    ));
    assert!(matches!(
        now_playing_playback(&status(Paused), true),
        (MediaPlayback::Paused { progress }, false) if at_seven(progress)
    ));
    assert!(matches!(
        now_playing_playback(&status(Playing), false),
        (MediaPlayback::Playing { progress: None }, false)
    ));
    assert!(matches!(
        now_playing_playback(&status(Paused), false),
        (MediaPlayback::Paused { progress: None }, false)
    ));
    assert!(matches!(
        now_playing_playback(&status(Stopped), false),
        (MediaPlayback::Stopped, false)
    ));
}

/// A platform host's track that is to play but does not move on yet goes to the media controls
/// as playing where it stands, and as buffering, which they show where they show such a track.
#[test]
fn a_hosts_track_that_buffers_goes_out_as_buffering_where_it_stands() {
    use PlaybackState::{Paused, Playing};

    let item = PlayingItem::LibraryTrack {
        library_id: Uuid::new_v4(),
        track: "hosted".to_owned(),
    };
    let status = |state| PlayerStatus {
        item: Some(item.clone()),
        index: Some(0),
        path: Some(PathBuf::from("hosted://host/hosted")),
        position: Duration::from_secs(7),
        state,
        playlist: vec![item.clone().into()],
        unavailable: Vec::new(),
        unavailable_reasons: HashMap::new(),
        playback_mode: PlaybackMode::Sequential,
        ready: true,
        volume: 1.0,
        hosted_by: Some("host".to_owned()),
        host_shows_in_media_controls: false,
        host_buffering: true,
    };
    assert!(matches!(
        now_playing_playback(&status(Playing), true),
        (MediaPlayback::Playing { progress: Some(MediaPosition(position)) }, true)
            if position == Duration::from_secs(7)
    ));
    // A paused track buffers nothing.
    assert!(matches!(
        now_playing_playback(&status(Paused), true),
        (MediaPlayback::Paused { .. }, false)
    ));
}

/// A service that names the track `slow` only after `delay`, and any other at once, and holds the
/// names from then on.
struct SlowService {
    library_id: Uuid,
    delay: Duration,
    known: std::sync::Mutex<bool>,
    /// How often the service was asked for the track, and told that it started.
    lookups: AtomicUsize,
    started: AtomicUsize,
    /// Each start time the service was told, in the order told.
    starts_at: std::sync::Mutex<Vec<u64>>,
    /// How often the service was told that a track played through.
    played: AtomicUsize,
    /// How many lookups ran to the end, so a lookup that was stopped is not counted.
    answered: AtomicUsize,
    /// Held by a test to keep the session waiting as it tells the service that a track started.
    starts: tokio::sync::RwLock<()>,
}

impl SlowService {
    fn summary(&self, track: &str) -> PlayingItemMetadataSummary {
        PlayingItemMetadataSummary {
            item: PlayingItem::LibraryTrack {
                library_id: self.library_id,
                track: track.to_owned(),
            },
            title: "Late Name".to_owned(),
            ..PlayingItemMetadataSummary::default()
        }
    }
}

impl crate::macros::RequestTranslator for SlowService {}

impl ServiceBinding for SlowService {
    fn library_id(&self) -> Uuid {
        self.library_id
    }

    fn slots(&self) -> crate::macros::RequestSlots {
        crate::macros::RequestSlots::unimplemented()
    }

    fn service(&self) -> ServiceKind {
        ServiceKind::Subsonic
    }

    fn item_metadata<'a>(
        &'a self,
        track: &'a str,
        _with_cover: bool,
    ) -> crate::service::ServiceFuture<'a, (PlayingItemMetadataSummary, Option<String>)> {
        Box::pin(async move {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            if track == "slow" {
                tokio::time::sleep(self.delay).await;
            }
            *self.known.lock().unwrap() = true;
            self.answered.fetch_add(1, Ordering::SeqCst);
            Ok((self.summary(track), None))
        })
    }

    fn cached_summary(&self, track: &str) -> Option<PlayingItemMetadataSummary> {
        self.known.lock().unwrap().then(|| self.summary(track))
    }

    fn track_started<'a>(&'a self, _track: &'a str, start_time: u64) -> BoxFuture<'a, ()> {
        self.started.fetch_add(1, Ordering::SeqCst);
        self.starts_at.lock().unwrap().push(start_time);
        Box::pin(async move {
            drop(self.starts.read().await);
        })
    }

    fn track_played_through<'a>(&'a self, _track: &'a str) -> BoxFuture<'a, ()> {
        self.played.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {})
    }

    fn visualizer_covers<'a>(
        &'a self,
        _track: &'a str,
    ) -> crate::service::ServiceFuture<'a, (Option<String>, Vec<String>)> {
        Box::pin(async { anyhow::bail!("no covers") })
    }

    fn playback_source<'a>(
        &'a self,
        _track: &'a str,
    ) -> crate::service::ServiceFuture<'a, playback::player::ServiceTarget> {
        Box::pin(async { anyhow::bail!("no playback") })
    }
}

#[tokio::test(start_paused = true)]
async fn a_track_that_a_slow_service_names_late_is_named_again() -> Result<()> {
    let library_id = Uuid::new_v4();
    let service = Arc::new(SlowService {
        library_id,
        delay: STATUS_METADATA_WAIT + Duration::from_secs(2),
        known: std::sync::Mutex::new(false),
        lookups: AtomicUsize::new(0),
        started: AtomicUsize::new(0),
        answered: AtomicUsize::new(0),
        starts: tokio::sync::RwLock::new(()),
        starts_at: std::sync::Mutex::new(Vec::new()),
        played: AtomicUsize::new(0),
    });
    let binding: Arc<dyn ServiceBinding> = service.clone();
    let item = PlayingItem::LibraryTrack {
        library_id,
        track: "slow".to_owned(),
    };
    let late_names = Arc::new(Notify::new());

    // The status does not wait for the service, and goes out without a name.
    let started = tokio::time::Instant::now();
    let unnamed = service_track_metadata(&binding, &item, "slow", None, Some(&late_names)).await;
    assert!(unnamed.is_err());
    assert_eq!(started.elapsed(), STATUS_METADATA_WAIT);
    assert!(service.cached_summary("slow").is_none());

    // The lookup goes on, and the session is told to name the track again once it lands.
    timeout(Duration::from_secs(10), late_names.notified()).await?;
    assert_eq!(started.elapsed(), service.delay);
    assert_eq!(service.lookups.load(Ordering::SeqCst), 1);

    // The session names the track from what the binding holds now, and asks the service nothing.
    let mut item_state = ItemEventState {
        last_item: Some(item.clone()),
        library: Some(ResolvedLibrary {
            library_id,
            fsio: None,
            media_root: String::new(),
            implementation: Implementation::Service(Arc::clone(&binding)),
        }),
        ..ItemEventState::default()
    };
    name_late_track(&mut item_state, &None, true).await;
    assert_eq!(item_state.metadata.title, "Late Name");
    assert_eq!(service.lookups.load(Ordering::SeqCst), 1);

    // A later status of the track, as when it starts again, asks the service once more, and takes
    // the name the binding holds when the service is as slow as before. That lookup is dropped
    // then, so it tells the session nothing.
    let (named, _) =
        service_track_metadata(&binding, &item, "slow", None, Some(&late_names)).await?;
    assert_eq!(named.title, "Late Name");
    assert_eq!(service.lookups.load(Ordering::SeqCst), 2);
    assert!(
        timeout(Duration::from_secs(30), late_names.notified())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_slow_lookup_stops_when_nothing_follows_the_player() -> Result<()> {
    let library_id = Uuid::new_v4();
    let service = Arc::new(SlowService {
        library_id,
        delay: STATUS_METADATA_WAIT + Duration::from_secs(2),
        known: std::sync::Mutex::new(false),
        lookups: AtomicUsize::new(0),
        started: AtomicUsize::new(0),
        answered: AtomicUsize::new(0),
        starts: tokio::sync::RwLock::new(()),
        starts_at: std::sync::Mutex::new(Vec::new()),
        played: AtomicUsize::new(0),
    });
    let binding: Arc<dyn ServiceBinding> = service.clone();
    let item = PlayingItem::LibraryTrack {
        library_id,
        track: "slow".to_owned(),
    };
    assert!(
        service_track_metadata(&binding, &item, "slow", None, None)
            .await
            .is_err()
    );
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert!(service.cached_summary("slow").is_none());
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_slow_lookup_stops_when_the_binding_already_names_the_track() -> Result<()> {
    let library_id = Uuid::new_v4();
    let service = Arc::new(SlowService {
        library_id,
        delay: STATUS_METADATA_WAIT + Duration::from_secs(2),
        known: std::sync::Mutex::new(true),
        lookups: AtomicUsize::new(0),
        started: AtomicUsize::new(0),
        answered: AtomicUsize::new(0),
        starts: tokio::sync::RwLock::new(()),
        starts_at: std::sync::Mutex::new(Vec::new()),
        played: AtomicUsize::new(0),
    });
    let binding: Arc<dyn ServiceBinding> = service.clone();
    let item = PlayingItem::LibraryTrack {
        library_id,
        track: "slow".to_owned(),
    };
    let late_names = Arc::new(Notify::new());

    // The status goes out with what the binding holds, once the wait has run out.
    let started = tokio::time::Instant::now();
    let (named, _) =
        service_track_metadata(&binding, &item, "slow", None, Some(&late_names)).await?;
    assert_eq!(named.title, "Late Name");
    assert_eq!(started.elapsed(), STATUS_METADATA_WAIT);
    assert_eq!(service.lookups.load(Ordering::SeqCst), 1);

    // The request still running is stopped: it never answers, nor tells the session anything.
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(service.answered.load(Ordering::SeqCst), 0);
    assert!(
        timeout(Duration::from_secs(10), late_names.notified())
            .await
            .is_err()
    );
    Ok(())
}

/// Once a slow service has named a track that went out unnamed, the name comes from the binding
/// that looked it up. The track keeps its place meanwhile: it is not started again, nor is its
/// service asked again, which a service as slow as before would not answer in time either.
#[tokio::test]
async fn a_late_name_comes_from_the_binding_that_looked_it_up() -> Result<()> {
    let dir = tempfile::tempdir()?;
    crate::test_node::disable_media_server(dir.path())?;
    let path = dir.path().to_string_lossy();
    let broadcaster: Arc<dyn Broadcaster> = Arc::new(Quiet);
    let node = NodeContext::new(
        &path,
        &path,
        broadcaster.clone(),
        false,
        RunMode::App,
        Arc::new(Visualizer::new()),
    )
    .await?;
    let library_id = Uuid::new_v4();
    let service = Arc::new(SlowService {
        library_id,
        delay: STATUS_METADATA_WAIT + Duration::from_secs(2),
        known: std::sync::Mutex::new(true),
        lookups: AtomicUsize::new(0),
        started: AtomicUsize::new(0),
        answered: AtomicUsize::new(0),
        starts: tokio::sync::RwLock::new(()),
        starts_at: std::sync::Mutex::new(Vec::new()),
        played: AtomicUsize::new(0),
    });
    let binding: Arc<dyn ServiceBinding> = service.clone();
    let item = PlayingItem::LibraryTrack {
        library_id,
        track: "slow".to_owned(),
    };
    // The track started unnamed: its library was had, but its service had not answered.
    let mut item_state = ItemEventState {
        last_item: Some(item.clone()),
        library: Some(ResolvedLibrary {
            library_id,
            fsio: None,
            media_root: String::new(),
            implementation: Implementation::Service(Arc::clone(&binding)),
        }),
        track_started: true,
        ..ItemEventState::default()
    };

    name_late_track(&mut item_state, &None, true).await;
    handle_status_event(
        &node,
        PlayerStatus {
            item: Some(item.clone()),
            index: Some(0),
            path: None,
            position: Duration::ZERO,
            state: PlaybackState::Playing,
            playlist: vec![item.clone().into()],
            unavailable: Vec::new(),
            unavailable_reasons: HashMap::new(),
            playback_mode: PlaybackMode::Sequential,
            ready: true,
            volume: 1.0,
            hosted_by: None,
            host_shows_in_media_controls: false,
            host_buffering: false,
        },
        &mut item_state,
        &None,
        None,
        &broadcaster,
        "test",
    )
    .await;

    assert_eq!(item_state.metadata.title, "Late Name");
    assert!(item_state.metadata_found);
    assert!(matches!(
        &item_state.library,
        Some(ResolvedLibrary { implementation: Implementation::Service(held), .. })
            if Arc::ptr_eq(held, &binding)
    ));
    assert!(item_state.track_started);
    assert_eq!(service.started.load(Ordering::SeqCst), 0);
    assert_eq!(service.lookups.load(Ordering::SeqCst), 0);
    node.shutdown().await?;
    Ok(())
}

/// A library of `service` that holds the name of every track already.
struct NamedService {
    library_id: Uuid,
    service: ServiceKind,
}

impl crate::macros::RequestTranslator for NamedService {}

impl ServiceBinding for NamedService {
    fn library_id(&self) -> Uuid {
        self.library_id
    }

    fn slots(&self) -> crate::macros::RequestSlots {
        crate::macros::RequestSlots::unimplemented()
    }

    fn service(&self) -> ServiceKind {
        self.service
    }

    fn item_metadata<'a>(
        &'a self,
        _track: &'a str,
        _with_cover: bool,
    ) -> crate::service::ServiceFuture<'a, (PlayingItemMetadataSummary, Option<String>)> {
        Box::pin(async { anyhow::bail!("named from what the binding holds") })
    }

    fn cached_summary(&self, track: &str) -> Option<PlayingItemMetadataSummary> {
        Some(PlayingItemMetadataSummary {
            item: PlayingItem::LibraryTrack {
                library_id: self.library_id,
                track: track.to_owned(),
            },
            title: "Late Name".to_owned(),
            ..PlayingItemMetadataSummary::default()
        })
    }

    fn visualizer_covers<'a>(
        &'a self,
        _track: &'a str,
    ) -> crate::service::ServiceFuture<'a, (Option<String>, Vec<String>)> {
        Box::pin(async { anyhow::bail!("no covers") })
    }

    fn playback_source<'a>(
        &'a self,
        _track: &'a str,
    ) -> crate::service::ServiceFuture<'a, playback::player::ServiceTarget> {
        Box::pin(async { anyhow::bail!("no playback") })
    }
}

/// A track named late tells Rune's scrobblers it plays, unless its service counts the plays of
/// its tracks itself, as it does for a track named at once.
#[tokio::test]
async fn a_track_named_late_goes_to_runes_scrobblers_unless_its_service_counts_its_plays()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    crate::test_node::disable_media_server(dir.path())?;
    let path = dir.path().to_string_lossy();
    let broadcaster: Arc<dyn Broadcaster> = Arc::new(Quiet);
    let node = NodeContext::new(
        &path,
        &path,
        broadcaster.clone(),
        false,
        RunMode::App,
        Arc::new(Visualizer::new()),
    )
    .await?;
    for (service, scrobbled) in [
        (ServiceKind::Subsonic, true),
        (ServiceKind::AppleMusic, false),
    ] {
        let library_id = Uuid::new_v4();
        let binding: Arc<dyn ServiceBinding> = Arc::new(NamedService {
            library_id,
            service,
        });
        let item = PlayingItem::LibraryTrack {
            library_id,
            track: "late".to_owned(),
        };
        // The track went out unnamed, and its service has named it since.
        let mut item_state = ItemEventState {
            last_item: Some(item.clone()),
            library: Some(ResolvedLibrary {
                library_id,
                fsio: None,
                media_root: String::new(),
                implementation: Implementation::Service(binding),
            }),
            track_started: true,
            ..ItemEventState::default()
        };
        let (now_playing, mut told) = mpsc::unbounded_channel();

        name_late_track(&mut item_state, &None, true).await;
        handle_status_event(
            &node,
            PlayerStatus {
                item: Some(item.clone()),
                index: Some(0),
                path: None,
                position: Duration::ZERO,
                state: PlaybackState::Playing,
                playlist: vec![item.clone().into()],
                unavailable: Vec::new(),
                unavailable_reasons: HashMap::new(),
                playback_mode: PlaybackMode::Sequential,
                ready: true,
                volume: 1.0,
                hosted_by: None,
                host_shows_in_media_controls: false,
                host_buffering: false,
            },
            &mut item_state,
            &None,
            Some(&now_playing),
            &broadcaster,
            "test",
        )
        .await;

        assert_eq!(item_state.metadata.title, "Late Name", "{service:?}");
        let sent = told.try_recv().ok();
        assert_eq!(
            sent.is_some(),
            scrobbled,
            "{service:?} told Rune's scrobblers: {sent:?}"
        );
    }
    node.shutdown().await?;
    Ok(())
}

/// A player whose status reports the test makes. Asked for its status after what it did, it
/// first reports what `reported_on_barrier` holds, as a player whose reports of what it did were
/// still to be queued when it was asked.
struct ScriptedPlayer {
    reports: SimpleSender<PlayerStatus>,
    played: SimpleSender<PlayedThrough>,
    /// The statuses and completions in the order the reports make them, as the player's own
    /// stream of events keeps the causal order the separate channels cannot.
    events: SimpleSender<PlaybackEvent>,
    status: std::sync::Mutex<PlayerStatus>,
    reported_on_barrier: std::sync::Mutex<Vec<PlayerStatus>>,
    /// What it reports right after it answered a barrier, as a player that moved on just then.
    reported_after_barrier: std::sync::Mutex<Vec<PlayerStatus>>,
}

impl ScriptedPlayer {
    fn new() -> Self {
        Self {
            reports: SimpleChannel::unbounded().0,
            played: SimpleChannel::unbounded().0,
            events: SimpleChannel::unbounded().0,
            status: std::sync::Mutex::new(MockPlayer.get_status()),
            reported_on_barrier: std::sync::Mutex::new(Vec::new()),
            reported_after_barrier: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Reports `status`, which is the player's status from then on.
    fn report(&self, status: PlayerStatus) {
        *self.status.lock().unwrap() = status.clone();
        self.reports.send(status.clone());
        self.events.send(PlaybackEvent::Status(status));
    }

    /// Reports the completion of `item`, as the player does before the next occurrence starts.
    fn plays_through(&self, item: &PlayingItem) {
        let played = PlayedThrough {
            item: item.clone(),
            listened: Duration::from_secs(10),
            started_at: Some(SystemTime::now()),
        };
        self.played.send(played.clone());
        self.events.send(PlaybackEvent::Played(played));
    }

    /// Moves on to `status` without reporting it yet, as a player whose report of the move is
    /// still to be queued.
    fn moves(&self, status: PlayerStatus) {
        *self.status.lock().unwrap() = status;
    }
}

impl Playable for ScriptedPlayer {
    fn command_with_completion(
        &mut self,
        command: PlayerCommand,
    ) -> BoxFuture<'static, result::Result<(), PlayerCommandError>> {
        MockPlayer.command_with_completion(command)
    }
    fn command_completion(
        &mut self,
    ) -> BoxFuture<'static, result::Result<PlayerStatus, PlayerCommandError>> {
        Box::pin(ready(Ok(self.get_status())))
    }
    fn load(&self, _index: usize) {}
    fn play(&self) {}
    fn pause(&self) {}
    fn stop(&self) {}
    fn next(&self) {}
    fn previous(&self) {}
    fn switch(&self, _index: usize) {}
    fn seek(&self, _position_seconds: f64) {}
    fn add_to_playlist(&self, _tracks: Vec<(PlayingItem, PathBuf)>, _mode: AddMode) {}
    fn handoff_load(
        &self,
        tracks: Vec<(PlayingItem, PlaybackSource)>,
        block: HandoffBlockId,
        start_index: Option<usize>,
        position: Duration,
        play: bool,
        playback_mode: Option<PlaybackMode>,
    ) -> oneshot::Receiver<HandoffLoadResult> {
        MockPlayer.handoff_load(tracks, block, start_index, position, play, playback_mode)
    }
    fn take_back_block(
        &self,
        block: HandoffBlockId,
        interrupted: InterruptedItem,
        resume: bool,
    ) -> oneshot::Receiver<Option<TakeBackBlockResult>> {
        let _ = resume;
        MockPlayer.take_back_block(block, interrupted, resume)
    }
    fn clear_handoff_blocks(&self) -> oneshot::Receiver<ClearHandoffBlocksResult> {
        MockPlayer.clear_handoff_blocks()
    }
    fn pause_reporting(&self) -> oneshot::Receiver<PausedPlayback> {
        MockPlayer.pause_reporting()
    }
    fn remove_from_playlist(&self, _index: usize) {}
    fn restore_tagged(
        &self,
        _entries: Vec<QueueEntry>,
        _index: Option<usize>,
        _position: Duration,
        _mode: PlaybackMode,
        _revision: u64,
    ) {
    }
    fn clear_playlist(&self) {}
    fn move_playlist_item(&self, _old_index: usize, _new_index: usize) {}
    fn set_playback_mode(&mut self, _mode: PlaybackMode) {}
    fn set_volume(&mut self, _volume: f32) {}
    fn set_realtime_fft_enabled(&mut self, _enabled: bool) {}
    fn pcm_tap(&self) -> Arc<PcmTap> {
        MockPlayer.pcm_tap()
    }
    fn set_adaptive_switching_enabled(&mut self, _enabled: bool) {}
    fn set_mix_with_others(&mut self, _enabled: bool) {}
    fn terminate(&self) {}
    fn get_status(&self) -> PlayerStatus {
        self.status.lock().unwrap().clone()
    }
    fn status_after_pending(&self) -> BoxFuture<'static, PlayerStatus> {
        let queued = std::mem::take(&mut *self.reported_on_barrier.lock().unwrap());
        for status in queued {
            self.report(status);
        }
        let answer = self.get_status();
        let later = std::mem::take(&mut *self.reported_after_barrier.lock().unwrap());
        for status in later {
            self.report(status);
        }
        Box::pin(ready(answer))
    }
    fn get_playlist(&self) -> Vec<PlayingItem> {
        let playlist = self.get_status().playlist;
        playlist.into_iter().map(|entry| entry.item).collect()
    }
    fn subscribe_status(&self) -> SimpleReceiver<PlayerStatus> {
        self.reports.subscribe()
    }
    fn subscribe_played_through(&self) -> SimpleReceiver<PlayedThrough> {
        self.played.subscribe()
    }
    fn subscribe_playback_events(&self) -> SimpleReceiver<PlaybackEvent> {
        self.events.subscribe()
    }
    fn subscribe_playlist(&self) -> SimpleReceiver<PlaylistStatus> {
        MockPlayer.subscribe_playlist()
    }
    fn subscribe_realtime_fft(&self) -> SimpleReceiver<Vec<f32>> {
        MockPlayer.subscribe_realtime_fft()
    }
    fn subscribe_crash(&self) -> SimpleReceiver<String> {
        MockPlayer.subscribe_crash()
    }
    fn subscribe_log(&self) -> SimpleReceiver<InternalLog> {
        MockPlayer.subscribe_log()
    }
}

/// Keeps the statuses and queue updates the UI is told, with the session each is for.
#[derive(Default)]
struct ToldStatuses {
    /// Each status as the session it is for, the item it names and that item's title.
    statuses: std::sync::Mutex<Vec<(String, Option<PlayingItemRef>, String)>>,
    queue_updates: std::sync::Mutex<Vec<String>>,
}

impl ToldStatuses {
    /// The items the statuses of `session_id` named, with their titles, in the order told.
    fn statuses(&self, session_id: &str) -> Vec<(Option<PlayingItemRef>, String)> {
        let statuses = self.statuses.lock().unwrap();
        statuses
            .iter()
            .filter(|(session, _, _)| session == session_id)
            .map(|(_, item, title)| (item.clone(), title.clone()))
            .collect()
    }

    fn queue_updates(&self, session_id: &str) -> usize {
        let updates = self.queue_updates.lock().unwrap();
        updates
            .iter()
            .filter(|session| *session == session_id)
            .count()
    }
}

impl Broadcaster for ToldStatuses {
    fn broadcast(&self, signal: &dyn RinfRustSignal) {
        if let Some(status) = signal.as_any().downcast_ref::<PlaybackStatus>() {
            self.statuses.lock().unwrap().push((
                status.session_id.clone(),
                status.item_request.clone(),
                status.title.clone().unwrap_or_default(),
            ));
        } else if let Some(update) = signal.as_any().downcast_ref::<PlaylistUpdate>() {
            self.queue_updates
                .lock()
                .unwrap()
                .push(update.session_id.clone());
        }
    }
}

/// A node with the slow service's library open, and a session of its own that plays through a
/// scripted player, with what the UI is told meanwhile.
struct ScriptedSession {
    _dir: tempfile::TempDir,
    node: Arc<NodeContext>,
    session: Arc<PlayerSession>,
    session_id: String,
    player: Arc<Mutex<ScriptedPlayer>>,
    service: Arc<SlowService>,
    told: Arc<ToldStatuses>,
}

impl ScriptedSession {
    async fn start() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        crate::test_node::disable_media_server(dir.path())?;
        let path = dir.path().to_string_lossy();
        let told = Arc::new(ToldStatuses::default());
        let node = NodeContext::new(
            &path,
            &path,
            told.clone(),
            false,
            RunMode::App,
            Arc::new(Visualizer::new()),
        )
        .await?;
        let service = Arc::new(SlowService {
            library_id: Uuid::new_v4(),
            delay: STATUS_METADATA_WAIT + Duration::from_secs(1),
            known: std::sync::Mutex::new(false),
            lookups: AtomicUsize::new(0),
            started: AtomicUsize::new(0),
            answered: AtomicUsize::new(0),
            starts: tokio::sync::RwLock::new(()),
            starts_at: std::sync::Mutex::new(Vec::new()),
            played: AtomicUsize::new(0),
        });
        let binding: Arc<dyn ServiceBinding> = service.clone();
        node.set_open_service_catalog(None, binding).await?;
        let player = Arc::new(Mutex::new(ScriptedPlayer::new()));
        let played: Arc<Mutex<dyn Playable>> = player.clone();
        let session = PlayerSession::new(
            &node,
            "scripted",
            false,
            Box::new(move |real| {
                real.terminate();
                played
            }),
        )
        .await?;
        let session_id = session.session_id().await;
        Ok(Self {
            _dir: dir,
            node,
            session,
            session_id,
            player,
            service,
            told,
        })
    }

    fn track(&self, track: &str) -> PlayingItem {
        PlayingItem::LibraryTrack {
            library_id: self.service.library_id,
            track: track.to_owned(),
        }
    }

    /// The status of the player as it plays `item` at `position`.
    fn playing(item: &PlayingItem, position: Duration) -> PlayerStatus {
        PlayerStatus {
            item: Some(item.clone()),
            index: Some(0),
            path: None,
            position,
            state: PlaybackState::Playing,
            playlist: vec![item.clone().into()],
            unavailable: Vec::new(),
            unavailable_reasons: HashMap::new(),
            playback_mode: PlaybackMode::Sequential,
            ready: true,
            volume: 1.0,
            hosted_by: None,
            host_shows_in_media_controls: false,
            host_buffering: false,
        }
    }

    /// The player plays `item` at `position` and reports it.
    async fn plays(&self, item: &PlayingItem, position: Duration) {
        self.player
            .lock()
            .await
            .report(Self::playing(item, position));
    }

    /// The tracks the statuses of the session named, with their titles, in the order told.
    fn told(&self) -> Vec<(String, String)> {
        self.told
            .statuses(&self.session_id)
            .into_iter()
            .filter_map(|(item, title)| match item {
                Some(PlayingItemRef::LibraryTrack { track, .. }) => Some((track, title)),
                _ => None,
            })
            .collect()
    }

    /// Waits until a status names `track` with `title`.
    async fn until_told(&self, track: &str, title: &str) -> Result<()> {
        let wanted = (track.to_owned(), title.to_owned());
        wait_until(&format!("{track} told as {title:?}"), || {
            let told = self.told().contains(&wanted);
            async move { Ok(told.then_some(())) }
        })
        .await
    }

    async fn stop(self) -> Result<()> {
        self.session.shutdown().await?;
        self.node.shutdown().await
    }
}

/// A track a slow service named late goes out named, and the queue is not read again for it:
/// reading it asks the service about every queued track, which holds every later report up while
/// the service stays slow.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_late_name_does_not_read_the_queue_again() -> Result<()> {
    let session = ScriptedSession::start().await?;
    let slow = session.track("slow");
    session.plays(&slow, Duration::from_secs(1)).await;
    session.until_told("slow", "Unknown Title").await?;
    session.until_told("slow", "Late Name").await?;
    // Reports are handled in turn, so once a later one is told, whatever the late name sent is
    // told too.
    session.plays(&slow, Duration::from_secs(2)).await;
    wait_until("the later report", || {
        let told = session.told().len();
        async move { Ok((told >= 3).then_some(())) }
    })
    .await?;
    assert_eq!(session.told.queue_updates(&session.session_id), 0);
    session.stop().await
}

/// A late name goes out on the status the session handled last. The reports the player queued
/// before it come first and in their order, so the UI never shows again a track that playback has
/// left, and no track starts twice.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_late_name_never_goes_out_ahead_of_reports_queued_before_it() -> Result<()> {
    let session = ScriptedSession::start().await?;
    let [slow, next, last] = ["slow", "next", "last"].map(|track| session.track(track));
    // Were the player asked where it is once the name lands, it would have moved on twice by
    // then, and queued those reports before it answered.
    *session
        .player
        .lock()
        .await
        .reported_on_barrier
        .lock()
        .unwrap() = vec![
        ScriptedSession::playing(&next, Duration::ZERO),
        ScriptedSession::playing(&last, Duration::ZERO),
    ];
    session.plays(&slow, Duration::from_secs(1)).await;
    session.until_told("slow", "Unknown Title").await?;
    // Nor is it the player's status when the name lands: the player has moved on by then, and
    // its reports of that are still to be queued.
    session
        .player
        .lock()
        .await
        .moves(ScriptedSession::playing(&last, Duration::ZERO));
    session.until_told("slow", "Late Name").await?;
    session.plays(&next, Duration::ZERO).await;
    session.plays(&last, Duration::ZERO).await;
    session.until_told("last", "Late Name").await?;
    let told = |track: &str, title: &str| (track.to_owned(), title.to_owned());
    assert_eq!(
        session.told(),
        [
            told("slow", "Unknown Title"),
            told("slow", "Late Name"),
            told("next", "Late Name"),
            told("last", "Late Name"),
        ]
    );
    assert_eq!(session.service.started.load(Ordering::SeqCst), 3);
    session.stop().await
}

/// A late name and the reports the player queued meanwhile can all be ready by the time the
/// session gets to them. The reports go first and in their order, and the track they left is not
/// shown again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reports_ready_with_a_late_name_go_out_first() -> Result<()> {
    let session = ScriptedSession::start().await?;
    let [slow, next, last] = ["slow", "next", "last"].map(|track| session.track(track));
    // The session waits as it tells the service that `slow` started, while the name lands and the
    // player moves on twice.
    let held = session.service.starts.write().await;
    session.plays(&slow, Duration::from_secs(1)).await;
    wait_until("the late lookup", || {
        let answered = session.service.answered.load(Ordering::SeqCst);
        async move { Ok((answered == 1).then_some(())) }
    })
    .await?;
    // The lookup tells the session right after it lands.
    tokio::time::sleep(Duration::from_millis(100)).await;
    session.plays(&next, Duration::ZERO).await;
    session.plays(&last, Duration::ZERO).await;
    drop(held);
    session.until_told("last", "Late Name").await?;
    let told = |track: &str, title: &str| (track.to_owned(), title.to_owned());
    assert_eq!(
        session.told(),
        [
            told("slow", "Unknown Title"),
            told("next", "Late Name"),
            told("last", "Late Name"),
        ]
    );
    session.stop().await
}

/// Keeps what the UI is told the playing item supports.
#[derive(Default)]
struct ToldCapabilities(std::sync::Mutex<Vec<crate::messages::PlayingItemCapabilities>>);

impl Broadcaster for ToldCapabilities {
    fn broadcast(&self, signal: &dyn RinfRustSignal) {
        if let Some(capabilities) = signal
            .as_any()
            .downcast_ref::<crate::messages::PlayingItemCapabilities>()
        {
            self.0.lock().unwrap().push(capabilities.clone());
        }
    }
}

/// What the item supports is told on a later status of the same item when it could not be told
/// as the item started, as while the library records could not be read.
#[tokio::test]
async fn what_the_item_supports_is_told_once_it_can_be() -> Result<()> {
    let dir = tempfile::tempdir()?;
    crate::test_node::disable_media_server(dir.path())?;
    let path = dir.path().to_string_lossy();
    let told = Arc::new(ToldCapabilities::default());
    let broadcaster: Arc<dyn Broadcaster> = told.clone();
    let node = NodeContext::new(
        &path,
        &path,
        broadcaster.clone(),
        false,
        RunMode::App,
        Arc::new(Visualizer::new()),
    )
    .await?;
    let item = PlayingItem::LibraryTrack {
        library_id: Uuid::new_v4(),
        track: "track".to_owned(),
    };
    let status = || PlayerStatus {
        item: Some(item.clone()),
        index: Some(0),
        path: None,
        position: Duration::ZERO,
        state: PlaybackState::Paused,
        playlist: vec![item.clone().into()],
        unavailable: Vec::new(),
        unavailable_reasons: HashMap::new(),
        playback_mode: PlaybackMode::Sequential,
        ready: true,
        volume: 1.0,
        hosted_by: None,
        host_shows_in_media_controls: false,
        host_buffering: false,
    };
    let registry = dir.path().join("libraries.json");
    let _ = fs::remove_file(&registry);
    fs::create_dir(&registry)?;
    let mut item_state = ItemEventState::default();
    handle_status_event(
        &node,
        status(),
        &mut item_state,
        &None,
        None,
        &broadcaster,
        "test",
    )
    .await;
    assert!(told.0.lock().unwrap().is_empty());

    fs::remove_dir(&registry)?;
    handle_status_event(
        &node,
        status(),
        &mut item_state,
        &None,
        None,
        &broadcaster,
        "test",
    )
    .await;
    assert_eq!(told.0.lock().unwrap().len(), 1);
    // Told once, it is not told again while it stays the same.
    handle_status_event(
        &node,
        status(),
        &mut item_state,
        &None,
        None,
        &broadcaster,
        "test",
    )
    .await;
    assert_eq!(told.0.lock().unwrap().len(), 1);
    node.shutdown().await?;
    Ok(())
}

/// Keeps whether the UI was told, each time, that a platform host plays the track.
#[derive(Default)]
struct ToldHosts(std::sync::Mutex<Vec<bool>>);

impl Broadcaster for ToldHosts {
    fn broadcast(&self, signal: &dyn RinfRustSignal) {
        if let Some(changed) = signal
            .as_any()
            .downcast_ref::<crate::messages::PlayerHostChanged>()
        {
            self.0.lock().unwrap().push(changed.hosted);
        }
    }
}

/// The session shows again with what the player reported last. A stop the player reported after
/// it answered the barrier is not undone by the answer, so the volume control does not go back
/// to the host of a track that is gone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_report_after_the_barrier_answered_is_what_the_session_shows_again_with() -> Result<()> {
    let dir = tempfile::tempdir()?;
    crate::test_node::disable_media_server(dir.path())?;
    let path = dir.path().to_string_lossy();
    let told = Arc::new(ToldHosts::default());
    let player = Arc::new(Mutex::new(ScriptedPlayer::new()));
    let played: Arc<Mutex<dyn Playable>> = player.clone();
    let node = NodeContext::with_reconnect_config(
        &path,
        &path,
        told.clone(),
        false,
        RunMode::App,
        crate::node::ReconnectConfig::default(),
        Box::new(move |real| {
            real.terminate();
            played
        }),
        Arc::new(Visualizer::new()),
        ServiceSetup::default(),
    )
    .await?;
    let hosted_told = |hosted: bool| {
        let told = told.clone();
        move || {
            let last = told.0.lock().unwrap().last().copied();
            async move { Ok((last == Some(hosted)).then_some(())) }
        }
    };
    let item = PlayingItem::IndependentFile("hosted".to_owned());
    player.lock().await.report(PlayerStatus {
        path: Some(PathBuf::from("hosted://host/key")),
        state: PlaybackState::Paused,
        ..ScriptedSession::playing(&item, Duration::ZERO)
    });
    wait_until("the host told", hosted_told(true)).await?;

    let away = CurrentPlayer::RunePeer {
        link: None,
        reconnecting: None,
    };
    node.transition_current_player(CurrentPlayer::Local, away.clone())
        .await?;
    wait_until("the host left", hosted_told(false)).await?;
    // The player stops just after it answers the barrier of the way back.
    *player.lock().await.reported_after_barrier.lock().unwrap() = vec![PlayerStatus {
        item: None,
        index: None,
        path: None,
        state: PlaybackState::Stopped,
        playlist: Vec::new(),
        ..ScriptedSession::playing(&item, Duration::ZERO)
    }];
    node.transition_current_player(away, CurrentPlayer::Local)
        .await?;
    assert_eq!(told.0.lock().unwrap().last(), Some(&false));
    node.shutdown().await
}

/// The player's status and completion events of one queue fed through the session's own event
/// loop, as playback reports them, with the starts and completions the library service was
/// told. `names` holds the track names the service answers late.
struct ReportingPlayback {
    _dir: tempfile::TempDir,
    node: Arc<NodeContext>,
    session: Arc<PlayerSession>,
    player: Arc<Mutex<ScriptedPlayer>>,
    service: Arc<SlowService>,
    /// The item events report next, as one loaded queue reports it through Repeat One or
    /// adjacent entries.
    item: PlayingItem,
    playlist: Vec<QueueEntry>,
}

impl ReportingPlayback {
    async fn start(tracks: &[&str]) -> Result<Self> {
        let dir = tempfile::tempdir()?;
        crate::test_node::disable_media_server(dir.path())?;
        let path = dir.path().to_string_lossy();
        let node = NodeContext::new(
            &path,
            &path,
            Arc::new(Quiet),
            false,
            RunMode::App,
            Arc::new(Visualizer::new()),
        )
        .await?;
        let library_id = Uuid::new_v4();
        let service = Arc::new(SlowService {
            library_id,
            delay: Duration::ZERO,
            known: std::sync::Mutex::new(false),
            lookups: AtomicUsize::new(0),
            started: AtomicUsize::new(0),
            answered: AtomicUsize::new(0),
            starts: tokio::sync::RwLock::new(()),
            starts_at: std::sync::Mutex::new(Vec::new()),
            played: AtomicUsize::new(0),
        });
        let binding: Arc<dyn ServiceBinding> = service.clone();
        node.set_open_service_catalog(None, binding).await?;
        let player = Arc::new(Mutex::new(ScriptedPlayer::new()));
        let played: Arc<Mutex<dyn Playable>> = player.clone();
        let session = PlayerSession::new(
            &node,
            "repeating",
            false,
            Box::new(move |real| {
                real.terminate();
                played
            }),
        )
        .await?;
        let item = PlayingItem::LibraryTrack {
            library_id,
            track: tracks[0].to_owned(),
        };
        Ok(Self {
            playlist: tracks
                .iter()
                .map(|track| {
                    PlayingItem::LibraryTrack {
                        library_id,
                        track: (*track).to_owned(),
                    }
                    .into()
                })
                .collect(),
            _dir: dir,
            node,
            session,
            player,
            service,
            item,
        })
    }

    /// One status of the queue's entries, as the player reports it.
    fn status(&self, item: &PlayingItem, index: usize, state: PlaybackState) -> PlayerStatus {
        PlayerStatus {
            item: Some(item.clone()),
            index: Some(index),
            path: None,
            position: Duration::ZERO,
            state,
            playlist: self.playlist.clone(),
            unavailable: Vec::new(),
            unavailable_reasons: HashMap::new(),
            playback_mode: PlaybackMode::RepeatOne,
            ready: true,
            volume: 1.0,
            hosted_by: None,
            host_shows_in_media_controls: false,
            host_buffering: false,
        }
    }

    /// The player reports a status, as it does while playback moves on.
    async fn reports(&self, item: &PlayingItem, index: usize, state: PlaybackState) {
        self.player
            .lock()
            .await
            .report(self.status(item, index, state));
    }

    /// The player reports the completion of `item`, as it does when the track played through.
    async fn plays_through(&self, item: &PlayingItem) {
        self.player.lock().await.plays_through(item);
    }

    /// Waits until the service was told that `count` starts began.
    async fn until_started(&self, count: usize) -> Result<()> {
        let service = self.service.clone();
        wait_until("the starts reported", move || {
            let started = service.started.load(Ordering::SeqCst);
            async move { Ok((started >= count).then_some(())) }
        })
        .await
    }

    /// Waits until the service was told that `count` tracks played through.
    async fn until_played(&self, count: usize) -> Result<()> {
        let service = self.service.clone();
        wait_until("the completions reported", move || {
            let played = service.played.load(Ordering::SeqCst);
            async move { Ok((played >= count).then_some(())) }
        })
        .await
    }

    async fn stop(self) -> Result<()> {
        self.session.shutdown().await?;
        self.node.shutdown().await
    }
}

/// A track played again right after it played through starts again: each occurrence reports
/// its own start and submits its own completion. The queue holds the same track next to itself,
/// as Repeat One plays it, so the item never changes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_track_played_again_starts_and_completes_again() -> Result<()> {
    let playback = ReportingPlayback::start(&["track", "track"]).await?;
    let track = playback.item.clone();

    // The first occurrence starts and ends.
    playback.reports(&track, 0, PlaybackState::Playing).await;
    playback.until_started(1).await?;
    playback.plays_through(&track).await;
    playback.until_played(1).await?;

    // The second occurrence of the same item starts and ends: its own report and completion.
    playback.reports(&track, 1, PlaybackState::Playing).await;
    assert!(
        playback.until_started(2).await.is_ok(),
        "the repeated occurrence starts its own now-playing report"
    );
    playback.plays_through(&track).await;
    assert!(
        playback.until_played(2).await.is_ok(),
        "the repeated occurrence submits its own completion"
    );

    playback.stop().await
}

/// Pausing and resuming the same occurrence does not start another report, and a different
/// track starts and completes as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pausing_and_resuming_does_not_start_the_track_again() -> Result<()> {
    let playback = ReportingPlayback::start(&["track", "other"]).await?;
    let track = playback.item.clone();
    let other = PlayingItem::LibraryTrack {
        library_id: playback.service.library_id,
        track: "other".to_owned(),
    };

    playback.reports(&track, 0, PlaybackState::Playing).await;
    playback.until_started(1).await?;
    playback.reports(&track, 0, PlaybackState::Paused).await;
    playback.reports(&track, 0, PlaybackState::Playing).await;
    playback.plays_through(&track).await;
    playback.until_played(1).await?;
    assert_eq!(
        playback.service.started.load(Ordering::SeqCst),
        1,
        "pausing and resuming does not start the occurrence again"
    );

    // A different track starts and completes as before.
    playback.reports(&other, 1, PlaybackState::Playing).await;
    playback.until_started(2).await?;
    playback.plays_through(&other).await;
    playback.until_played(2).await?;
    assert_eq!(
        playback.service.started.load(Ordering::SeqCst),
        2,
        "only the two occurrences report their starts"
    );

    playback.stop().await
}
