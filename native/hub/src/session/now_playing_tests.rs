use super::*;
use crate::{server::utils::device::RunMode, utils::RinfRustSignal};
use scrobbling::{ScrobblingTrack, manager::ScrobblingService};
use simple_channel::{SimpleReceiver, SimpleSender};

#[derive(Default)]
struct RecordingScrobbler(Vec<String>);

#[async_trait::async_trait]
impl ScrobblingServiceManager for RecordingScrobbler {
    async fn send_login_status(&self) {
        unreachable!()
    }

    async fn authenticate(
        &mut self,
        _: &ScrobblingService,
        _: &str,
        _: &str,
        _: Option<String>,
        _: Option<String>,
        _: bool,
    ) -> Result<()> {
        unreachable!()
    }

    async fn update_now_playing(&mut self, _: &ScrobblingService, _: ScrobblingTrack) {
        unreachable!()
    }

    fn restore_session(&mut self, _: &ScrobblingService, _: String) -> Result<()> {
        unreachable!()
    }

    fn update_now_playing_all(&mut self, track: ScrobblingTrack) {
        self.0.push(track.track);
    }

    async fn scrobble(&mut self, _: ScrobblingService, _: ScrobblingTrack) {
        unreachable!()
    }

    fn scrobble_all(&mut self, _: ScrobblingTrack) {
        unreachable!()
    }

    async fn logout(&mut self, _: ScrobblingService) {
        unreachable!()
    }

    fn subscribe_error(&self) -> SimpleReceiver<ScrobblingError> {
        unreachable!()
    }

    fn subscribe_login_status(&self) -> SimpleReceiver<Vec<LoginStatus>> {
        unreachable!()
    }

    fn error_sender(&self) -> Arc<SimpleSender<ScrobblingError>> {
        unreachable!()
    }
}

struct Quiet;
impl Broadcaster for Quiet {
    fn broadcast(&self, _: &dyn RinfRustSignal) {}
}

#[tokio::test]
async fn now_playing_waits_for_each_tracks_first_start() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().to_string_lossy();
    let broadcaster: Arc<dyn Broadcaster> = Arc::new(Quiet);
    let node = NodeContext::new(
        &path,
        &path,
        broadcaster.clone(),
        false,
        RunMode::App,
        Arc::new(::visualizer::Visualizer::new()),
    )
    .await?;
    let recorder = Arc::new(Mutex::new(RecordingScrobbler::default()));
    let scrobbler: Arc<Mutex<dyn ScrobblingServiceManager>> = recorder.clone();
    let mut item_state = ItemEventState::default();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
    let mut tracks = Vec::new();
    for name in ["first.ogg", "second.ogg"] {
        let file = dir.path().join(name);
        std::fs::copy(&fixture, &file)?;
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
            Some(&scrobbler),
            &broadcaster,
            "test",
        )
        .await;
        assert!(
            recorder.lock().await.0.iter().eq(expected.iter().copied()),
            "{step}: expected {expected:?}, got {:?}",
            recorder.lock().await.0
        );
    }
    node.shutdown().await?;
    Ok(())
}
