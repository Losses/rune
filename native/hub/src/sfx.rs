use std::sync::Arc;

use anyhow::Result;
use playback::sfx_player::SfxPlayer;
use rinf::DartSignal;
use tokio::sync::Mutex;

use crate::SfxPlayRequest;

pub async fn sfx_play_request(
    sfx_player: Arc<Mutex<SfxPlayer>>,
    dart_signal: DartSignal<SfxPlayRequest>,
) -> Result<()> {
    sfx_player.lock().await.load(dart_signal.message.path.into());
    Ok(())
}
