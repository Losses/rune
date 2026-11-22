use super::*;
use crate::{server::utils::device::RunMode, utils::RinfRustSignal};
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
