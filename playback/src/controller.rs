use std::sync::Arc;

use anyhow::{Context, Result};
use log::{debug, error, info};
use souvlaki::{MediaControlEvent, MediaControls, PlatformConfig};
use tokio::sync::Mutex;

use crate::player::Player;

pub struct MediaControlManager {
    pub controls: MediaControls,
    player: Arc<Mutex<Player>>,
}

impl MediaControlManager {
    pub fn new(player: Arc<Mutex<Player>>) -> Result<Self> {
        let config = PlatformConfig {
            dbus_name: "rune_player",
            display_name: "Rune",
            hwnd: None,
        };

        let controls = MediaControls::new(config).context("Failed to create MediaControls")?;

        Ok(Self { controls, player })
    }

    pub async fn initialize(&mut self) -> Result<()> {
        info!("Initializing media controls");

        let player_clone = Arc::clone(&self.player);
        self.controls
            .attach(move |event: MediaControlEvent| {
                let player = player_clone.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_media_control_event(player, event).await {
                        error!("Error handling media control event: {:?}", e);
                    }
                });
            })
            .context("Failed to attach media control event handler")?;

        Ok(())
    }
}

async fn handle_media_control_event(
    player: Arc<Mutex<Player>>,
    event: MediaControlEvent,
) -> Result<()> {
    debug!("Received media control event: {:?}", event);

    match event {
        MediaControlEvent::Play => player.lock().await.play(),
        MediaControlEvent::Pause => player.lock().await.pause(),
        // MediaControlEvent::Toggle => player.lock().await.toggle(),
        MediaControlEvent::Next => player.lock().await.next(),
        MediaControlEvent::Previous => player.lock().await.previous(),
        MediaControlEvent::Stop => player.lock().await.stop(),
        MediaControlEvent::Seek(direction) => {
            let seek_seconds: f64 = match direction {
                souvlaki::SeekDirection::Forward => 10.0,
                souvlaki::SeekDirection::Backward => -10.0,
            };

            let current_position = player.lock().await.current_status.lock().unwrap().position;

            player
                .lock()
                .await
                .seek(current_position.as_millis() as f64 + seek_seconds * 1000.0);
        }
        MediaControlEvent::SetPosition(position) => {
            player.lock().await.seek(position.0.as_millis() as f64)
        }
        _ => debug!("Unhandled media control event: {:?}", event),
    }

    Ok(())
}
