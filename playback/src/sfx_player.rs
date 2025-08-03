use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use log::{error, info};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::sfx_internal::{SfxPlayerCommand, SfxPlayerInternal};

#[derive(Debug, Clone)]
pub struct SfxPlayerStatus {
    pub path: Option<PathBuf>,
    pub ready: bool,
    pub volume: f32,
}

pub struct SfxPlayer {
    commands: Arc<Mutex<mpsc::UnboundedSender<SfxPlayerCommand>>>,
    pub current_status: Arc<Mutex<SfxPlayerStatus>>,
    status_sender: broadcast::Sender<SfxPlayerStatus>,
    cancellation_token: CancellationToken,
}

impl Default for SfxPlayer {
    fn default() -> Self {
        Self::new(None)
    }
}

impl SfxPlayer {
    // Create a new Player instance and return the Player and the event receiver
    pub fn new(cancellation_token: Option<CancellationToken>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (status_sender, _) = broadcast::channel(16);
        let (crash_sender, _) = broadcast::channel(16);

        let cancellation_token = cancellation_token.unwrap_or_default();

        let current_status = Arc::new(Mutex::new(SfxPlayerStatus {
            path: None,
            ready: false,
            volume: 1.0,
        }));

        let commands = Arc::new(Mutex::new(cmd_tx));
        let player = SfxPlayer {
            commands: commands.clone(),
            current_status: current_status.clone(),
            status_sender: status_sender.clone(),
            cancellation_token: cancellation_token.clone(),
        };

        let internal_cancellation_token = cancellation_token.clone();
        thread::spawn(move || {
            let mut internal = SfxPlayerInternal::new(cmd_rx, internal_cancellation_token.clone());
            let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
            if let Err(e) = runtime.block_on(internal.run()) {
                error!("PlayerInternal runtime error: {e:?}");

                if let Err(e) = crash_sender.send(format!("{e:#?}")) {
                    error!("Failed to send error report: {e:?}");
                }
            }

            info!("Sfx player finalized.");
        });

        player
    }

    pub fn get_status(&self) -> SfxPlayerStatus {
        self.current_status.lock().unwrap().clone()
    }

    pub fn subscribe_status(&self) -> broadcast::Receiver<SfxPlayerStatus> {
        self.status_sender.subscribe()
    }

    pub fn command(&self, cmd: SfxPlayerCommand) {
        if let Ok(commands) = self.commands.lock() {
            if let Err(e) = commands.send(cmd) {
                error!("Failed to send command: {e:?}");
            }
        } else {
            error!("Failed to lock commands for sending");
        }
    }

    pub fn terminate(&self) {
        self.cancellation_token.cancel();
    }
}

impl SfxPlayer {
    pub fn load(&self, path: PathBuf) {
        self.command(SfxPlayerCommand::Load { path });
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.command(SfxPlayerCommand::SetVolume(volume));
    }
}
