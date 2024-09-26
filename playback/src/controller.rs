use std::{sync::Arc, thread};

use anyhow::{Context, Result};
use log::{debug, error, info};
use souvlaki::{MediaControlEvent, MediaControls, PlatformConfig};
use tokio::sync::{broadcast, Mutex};

use crate::player::{PlaybackState, Player};

pub struct MediaControlManager {
    pub controls: MediaControls,
    event_sender: broadcast::Sender<MediaControlEvent>,
}

impl MediaControlManager {
    pub fn new() -> Result<Self> {
        let config = PlatformConfig {
            dbus_name: "rune_player",
            display_name: "Rune",
            hwnd: None,
        };

        let controls = MediaControls::new(config).context("Failed to create MediaControls")?;
        let (event_sender, _) = broadcast::channel(32);

        Ok(Self {
            controls,
            event_sender,
        })
    }

    pub fn initialize(&mut self) -> Result<()> {
        info!("Initializing media controls");

        let event_sender = self.event_sender.clone();
        self.controls
            .attach(move |event: MediaControlEvent| {
                let event_sender = event_sender.clone();
                thread::spawn(move || {
                    if let Err(e) = event_sender.send(event) {
                        error!("Error sending media control event: {:?}", e);
                    }
                });
            })
            .context("Failed to attach media control event handler")?;

        Ok(())
    }

    pub fn subscribe_controller_events(&self) -> broadcast::Receiver<MediaControlEvent> {
        self.event_sender.subscribe()
    }
}

pub async fn handle_media_control_event(
    player: &Arc<Mutex<Player>>,
    event: MediaControlEvent,
) -> Result<()> {
    debug!("Received media control event: {:?}", event);

    match event {
        MediaControlEvent::Play => player.lock().await.play(),
        MediaControlEvent::Pause => player.lock().await.pause(),
        MediaControlEvent::Toggle => {
            if player.lock().await.get_status().state == PlaybackState::Playing {
                player.lock().await.pause();
            } else {
                player.lock().await.play();
            }
        }
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
