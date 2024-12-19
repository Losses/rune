use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{fmt, thread};

use log::{debug, error};
use simple_channel::{SimpleChannel, SimpleReceiver, SimpleSender};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::internal::{PlaybackMode, PlayerCommand, PlayerEvent, PlayerInternal};
use crate::strategies::AddMode;

#[derive(Debug, Clone)]
pub struct PlayerStatus {
    pub item: Option<PlayingItem>,
    pub index: Option<usize>,
    pub path: Option<PathBuf>,
    pub position: Duration,
    pub state: PlaybackState,
    pub playlist: Vec<PlayingItem>,
    pub playback_mode: PlaybackMode,
    pub ready: bool,
    pub volume: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PlayingItem {
    InLibrary(i32),
    IndependentFile(PathBuf),
    Unknown,
}

impl fmt::Display for PlayingItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlayingItem::InLibrary(id) => write!(f, "PlayingItem::InLibrary({})", id),
            PlayingItem::IndependentFile(path) => {
                write!(
                    f,
                    "PlayingItem::IndependentFile({})",
                    path.to_string_lossy()
                )
            }
            PlayingItem::Unknown => write!(f, "PlayingItem::Unknown()"),
        }
    }
}

impl From<PlayingItem> for String {
    fn from(item: PlayingItem) -> Self {
        item.to_string()
    }
}

#[derive(Debug, Clone)]
pub struct PlaylistStatus {
    pub items: Vec<PlayingItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
}

impl std::fmt::Display for PlaybackState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state_str = match self {
            PlaybackState::Playing => "Playing",
            PlaybackState::Paused => "Paused",
            PlaybackState::Stopped => "Stopped",
        };
        write!(f, "{}", state_str)
    }
}

// Define the Player struct, which includes a channel sender for sending commands
pub struct Player {
    commands: Arc<Mutex<mpsc::UnboundedSender<PlayerCommand>>>,
    pub current_status: Arc<Mutex<PlayerStatus>>,
    status_sender: SimpleSender<PlayerStatus>,
    playlist_sender: SimpleSender<PlaylistStatus>,
    played_through_sender: SimpleSender<PlayingItem>,
    realtime_fft_sender: SimpleSender<Vec<f32>>,
    crash_sender: SimpleSender<String>,
    cancellation_token: CancellationToken,
}

impl Default for Player {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Player {
    // Create a new Player instance and return the Player and the event receiver
    pub fn new(cancellation_token: Option<CancellationToken>) -> Self {
        // Create an unbounded channel for sending commands
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        // Create an unbounded channel for receiving events
        let (event_sender, mut event_receiver) = mpsc::unbounded_channel();
        // Create a broadcast channel for status updates
        let (status_sender, _) = SimpleChannel::channel(16);
        // Create a broadcast channel for played through update
        let (played_through_sender, _) = SimpleChannel::channel(16);
        // Create a broadcast channel for playlist updates
        let (playlist_sender, _) = SimpleChannel::channel(16);
        // Create a broadcast channel for realtime FFT updates
        let (realtime_fft_sender, _) = SimpleChannel::channel(32);
        // Create a broadcast channel player crash report
        let (crash_sender, _) = SimpleChannel::channel(16);

        // Create a cancellation token
        let cancellation_token = cancellation_token.unwrap_or_default();

        // Create internal status for the whole player
        let current_status = Arc::new(Mutex::new(PlayerStatus {
            item: None,
            index: None,
            path: None,
            position: Duration::new(0, 0),
            state: PlaybackState::Stopped,
            playback_mode: PlaybackMode::Sequential,
            playlist: Vec::new(),
            ready: false,
            volume: 1.0,
        }));

        let commands = Arc::new(Mutex::new(cmd_tx));
        // Create the Player instance and wrap the command sender in Arc<Mutex>
        let player = Player {
            commands: commands.clone(),
            current_status: current_status.clone(),
            status_sender: status_sender.clone(),
            playlist_sender: playlist_sender.clone(),
            played_through_sender: played_through_sender.clone(),
            realtime_fft_sender: realtime_fft_sender.clone(),
            crash_sender: crash_sender.clone(),
            cancellation_token: cancellation_token.clone(),
        };

        // Start a new thread to run the PlayerInternal logic
        let internal_cancellation_token = cancellation_token.clone();
        thread::spawn(move || {
            // Create a PlayerInternal instance, passing in the command receiver and event sender
            let mut internal =
                PlayerInternal::new(cmd_rx, event_sender, internal_cancellation_token.clone());
            // Create a new Tokio runtime for asynchronous tasks
            let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
            // Run the main loop of PlayerInternal within the Tokio runtime
            if let Err(e) = runtime.block_on(internal.run()) {
                error!("PlayerInternal runtime error: {:?}", e);

                crash_sender.send(format!("{:#?}", e));
            }
        });

        // Start a new thread to handle events and update the status
        let status_clone = current_status.clone();
        let status_sender_clone = status_sender.clone();
        let playlist_sender_clone = playlist_sender.clone();
        let realtime_fft_sender_clone = realtime_fft_sender.clone();
        thread::spawn(move || {
            while let Some(event) = event_receiver.blocking_recv() {
                let mut status = status_clone.lock().unwrap();
                match event {
                    PlayerEvent::Loaded {
                        item,
                        index,
                        path,
                        playback_mode,
                        position,
                    } => {
                        status.item = Some(item);
                        status.index = Some(index);
                        status.path = Some(path);
                        status.playback_mode = playback_mode;
                        status.position = position;
                        status.ready = true;
                        status.state = PlaybackState::Paused;
                    }
                    PlayerEvent::Playing {
                        item,
                        index,
                        path,
                        playback_mode,
                        position,
                    } => {
                        status.item = Some(item);
                        status.index = Some(index);
                        status.path = Some(path);
                        status.playback_mode = playback_mode;
                        status.position = position;
                        status.state = PlaybackState::Playing;
                    }
                    PlayerEvent::Paused {
                        item,
                        index,
                        path,
                        playback_mode,
                        position,
                    } => {
                        status.item = Some(item);
                        status.index = Some(index);
                        status.path = Some(path);
                        status.playback_mode = playback_mode;
                        status.position = position;
                        status.state = PlaybackState::Paused;
                    }
                    PlayerEvent::Stopped {} => {
                        status.item = None;
                        status.index = None;
                        status.path = None;
                        status.position = Duration::new(0, 0);
                        status.state = PlaybackState::Stopped;
                    }
                    PlayerEvent::Progress {
                        item,
                        index,
                        path,
                        playback_mode,
                        position,
                        ready,
                    } => {
                        status.item = item;
                        status.index = index;
                        status.path = path;
                        status.playback_mode = playback_mode;
                        status.position = position;
                        status.ready = ready;
                    }
                    PlayerEvent::EndOfPlaylist => {
                        status.index = None;
                        status.path = None;
                        status.position = Duration::new(0, 0);
                        status.state = PlaybackState::Stopped;
                    }
                    PlayerEvent::EndOfTrack {
                        item,
                        index,
                        path,
                        playback_mode,
                    } => {
                        status.item = Some(item.clone());
                        status.index = Some(index);
                        status.path = Some(path);
                        status.playback_mode = playback_mode;
                        played_through_sender.send(item);
                    }
                    PlayerEvent::Error {
                        item,
                        index,
                        path,
                        error,
                    } => {
                        error!(
                            "Error at index {}({:?}): {:?} - {}",
                            index, item, path, error
                        );
                    }
                    PlayerEvent::PlaylistUpdated(playlist) => {
                        status.playlist = playlist.clone();
                        debug!("Sending playlist status");
                        playlist_sender_clone.send(PlaylistStatus {
                            items: playlist.clone(),
                        });
                    }
                    PlayerEvent::RealtimeFFT(data) => {
                        realtime_fft_sender_clone.send(data);
                    }
                    PlayerEvent::VolumeUpdate(value) => {
                        status.volume = value;
                    }
                }
                status_sender_clone.send(status.clone());
            }
        });

        player
    }

    pub fn get_status(&self) -> PlayerStatus {
        self.current_status.lock().unwrap().clone()
    }

    pub fn get_playlist(&self) -> Vec<PlayingItem> {
        self.current_status.lock().unwrap().playlist.clone()
    }

    pub fn subscribe_status(&self) -> SimpleReceiver<PlayerStatus> {
        self.status_sender.subscribe()
    }

    pub fn subscribe_played_through(&self) -> SimpleReceiver<PlayingItem> {
        self.played_through_sender.subscribe()
    }

    pub fn subscribe_playlist(&self) -> SimpleReceiver<PlaylistStatus> {
        self.playlist_sender.subscribe()
    }

    pub fn subscribe_realtime_fft(&self) -> SimpleReceiver<Vec<f32>> {
        self.realtime_fft_sender.subscribe()
    }

    pub fn subscribe_crash(&self) -> SimpleReceiver<String> {
        self.crash_sender.subscribe()
    }

    // Send a command to the internal player
    pub fn command(&self, cmd: PlayerCommand) {
        // Acquire the lock and send the command
        if let Ok(commands) = self.commands.lock() {
            if let Err(e) = commands.send(cmd) {
                error!("Failed to send command: {:?}", e);
            }
        } else {
            error!("Failed to lock commands for sending");
        }
    }

    pub fn terminate(&self) {
        self.cancellation_token.cancel();
    }
}

impl Player {
    pub fn load(&self, index: usize) {
        self.command(PlayerCommand::Load { index });
    }

    pub fn play(&self) {
        self.command(PlayerCommand::Play);
    }

    pub fn pause(&self) {
        self.command(PlayerCommand::Pause);
    }

    pub fn stop(&self) {
        self.command(PlayerCommand::Stop);
    }

    pub fn next(&self) {
        self.command(PlayerCommand::Next);
    }

    pub fn previous(&self) {
        self.command(PlayerCommand::Previous);
    }

    pub fn switch(&self, index: usize) {
        self.command(PlayerCommand::Switch(index));
    }

    pub fn seek(&self, position_ms: f64) {
        self.command(PlayerCommand::Seek(position_ms));
    }

    pub fn add_to_playlist(&self, tracks: Vec<(PlayingItem, PathBuf)>, mode: AddMode) {
        self.command(PlayerCommand::AddToPlaylist { tracks, mode });
    }

    pub fn remove_from_playlist(&self, index: usize) {
        self.command(PlayerCommand::RemoveFromPlaylist { index });
    }

    pub fn clear_playlist(&self) {
        self.command(PlayerCommand::ClearPlaylist);
    }

    pub fn move_playlist_item(&self, old_index: usize, new_index: usize) {
        self.command(PlayerCommand::MovePlayListItem {
            old_index,
            new_index,
        });
    }

    pub fn set_playback_mode(&mut self, mode: PlaybackMode) {
        self.command(PlayerCommand::SetPlaybackMode(mode));
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.command(PlayerCommand::SetVolume(volume));
    }

    pub fn set_realtime_fft_enabled(&mut self, enabled: bool) {
        self.command(PlayerCommand::SetRealtimeFFTEnabled(enabled));
    }

    pub fn set_adaptive_switching_enabled(&mut self, enabled: bool) {
        self.command(PlayerCommand::SetAdaptiveSwitchingEnabled(enabled));
    }
}
