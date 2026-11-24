use super::*;
use crate::{server::utils::device::RunMode, utils::RinfRustSignal};
use database::library_registry::ServiceKind;
use std::fs;
use visualizer::Visualizer;

struct Quiet;
impl Broadcaster for Quiet {
    fn broadcast(&self, _: &dyn RinfRustSignal) {}
}

#[tokio::test]
async fn now_playing_waits_for_each_tracks_first_start() -> Result<()> {
    let dir = tempfile::tempdir()?;
    crate::test_node::disable_sync_listener(dir.path())?;
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

/// A service that names its track only after `delay`, and holds the name from then on.
struct SlowService {
    library_id: Uuid,
    delay: Duration,
    known: std::sync::Mutex<bool>,
    /// How often the service was asked for the track, and told that it started.
    lookups: AtomicUsize,
    started: AtomicUsize,
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
            tokio::time::sleep(self.delay).await;
            *self.known.lock().unwrap() = true;
            Ok((self.summary(track), None))
        })
    }

    fn cached_summary(&self, track: &str) -> Option<PlayingItemMetadataSummary> {
        self.known.lock().unwrap().then(|| self.summary(track))
    }

    fn track_started<'a>(&'a self, _track: &'a str, _start_time: u64) -> BoxFuture<'a, ()> {
        self.started.fetch_add(1, Ordering::SeqCst);
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
    ) -> crate::service::ServiceFuture<'a, playback::player::RemoteAudioSource> {
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

/// Once a slow service has named a track that went out unnamed, the name comes from the binding
/// that looked it up. The track keeps its place meanwhile: it is not started again, nor is its
/// service asked again, which a service as slow as before would not answer in time either.
#[tokio::test]
async fn a_late_name_comes_from_the_binding_that_looked_it_up() -> Result<()> {
    let dir = tempfile::tempdir()?;
    crate::test_node::disable_sync_listener(dir.path())?;
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
    crate::test_node::disable_sync_listener(dir.path())?;
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
