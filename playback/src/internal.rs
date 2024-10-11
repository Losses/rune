use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use log::{debug, error, info, warn};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use rodio::{Decoder, OutputStream, Sink, Source};
use tokio::sync::mpsc;
use tokio::time::{interval, sleep_until, Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::realtime_fft::RealTimeFFT;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlaybackMode {
    Sequential,
    RepeatOne,
    RepeatAll,
    Shuffle,
}

impl From<u32> for PlaybackMode {
    fn from(value: u32) -> Self {
        match value {
            0 => PlaybackMode::Sequential,
            1 => PlaybackMode::RepeatOne,
            2 => PlaybackMode::RepeatAll,
            3 => PlaybackMode::Shuffle,
            _ => PlaybackMode::Sequential,
        }
    }
}

impl From<PlaybackMode> for u32 {
    fn from(mode: PlaybackMode) -> Self {
        match mode {
            PlaybackMode::Sequential => 0,
            PlaybackMode::RepeatOne => 1,
            PlaybackMode::RepeatAll => 2,
            PlaybackMode::Shuffle => 3,
        }
    }
}

#[derive(Debug)]
pub enum PlayerCommand {
    Load {
        index: usize,
    },
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    Switch(usize),
    Seek(f64),
    AddToPlaylist {
        tracks: Vec<(i32, std::path::PathBuf)>,
    },
    RemoveFromPlaylist {
        index: usize,
    },
    ClearPlaylist,
    MovePlayListItem {
        old_index: usize,
        new_index: usize,
    },
    SetPlaybackMode(PlaybackMode),
    SetVolume(f32),
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Stopped,
    Playing {
        id: i32,
        index: usize,
        path: PathBuf,
        playback_mode: PlaybackMode,
        position: Duration,
    },
    Paused {
        id: i32,
        index: usize,
        path: PathBuf,
        playback_mode: PlaybackMode,
        position: Duration,
    },
    EndOfPlaylist,
    EndOfTrack {
        id: i32,
        index: usize,
        path: PathBuf,
        playback_mode: PlaybackMode,
    },
    Error {
        id: i32,
        index: usize,
        path: PathBuf,
        error: String,
    },
    Progress {
        id: Option<i32>,
        index: Option<usize>,
        path: Option<PathBuf>,
        position: Duration,
        playback_mode: PlaybackMode,
        ready: bool,
    },
    VolumeUpdate(f32),
    PlaylistUpdated(Vec<i32>),
    RealtimeFFT(Vec<f32>),
}

#[derive(Debug, Clone)]
pub struct PlaylistItem {
    pub id: i32,
    pub path: PathBuf,
}

#[derive(Debug, PartialEq)]
enum InternalPlaybackState {
    Playing,
    Paused,
    Stopped,
}

/// Generates a random sequence from 1 to max_value and returns the nth value
///
/// # Parameters
///
/// * `seed` - The seed for the random number generator
/// * `max_value` - The maximum value of the sequence
/// * `n` - The nth value to return (1-based index)
///
/// # Returns
///
/// Returns the nth value, or None if n is out of range
fn get_random_sequence(seed: u64, max_value: usize) -> Vec<usize> {
    // Create a sequence from 1 to max_value
    let mut values: Vec<usize> = (1..=max_value).collect();

    // Create a random number generator with the given seed
    let mut rng = StdRng::seed_from_u64(seed);

    // Shuffle the sequence
    values.shuffle(&mut rng);

    values
}

pub(crate) struct PlayerInternal {
    commands: mpsc::UnboundedReceiver<PlayerCommand>,
    event_sender: mpsc::UnboundedSender<PlayerEvent>,
    realtime_fft: Arc<Mutex<RealTimeFFT>>,
    playlist: Vec<PlaylistItem>,
    current_track_id: Option<i32>,
    current_track_index: Option<usize>,
    current_track_path: Option<PathBuf>,
    sink: Option<Sink>,
    _stream: Option<OutputStream>,
    state: InternalPlaybackState,
    debounce_timer: Option<Instant>,
    cancellation_token: CancellationToken,
    playback_mode: PlaybackMode,
    random_map: Vec<usize>,
    volume: f32,
}

impl PlayerInternal {
    pub fn new(
        commands: mpsc::UnboundedReceiver<PlayerCommand>,
        event_sender: mpsc::UnboundedSender<PlayerEvent>,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            commands,
            event_sender,
            playlist: Vec::new(),
            current_track_id: None,
            current_track_index: None,
            current_track_path: None,
            sink: None,
            _stream: None,
            realtime_fft: Arc::new(Mutex::new(RealTimeFFT::new(512))),
            state: InternalPlaybackState::Stopped,
            debounce_timer: None,
            cancellation_token,
            playback_mode: PlaybackMode::Sequential,
            random_map: [].to_vec(),
            volume: 1.0,
        }
    }

    pub async fn run(&mut self) {
        let mut progress_interval = interval(Duration::from_millis(100));

        let mut fft_receiver = self.realtime_fft.lock().unwrap().subscribe();
        loop {
            tokio::select! {
                Some(cmd) = self.commands.recv() => {
                    if self.cancellation_token.is_cancelled() {
                        debug!("Cancellation token triggered, exiting run loop");

                        self.stop();
                        break;
                    }

                    debug!("Received command: {:?}", cmd);
                    match cmd {
                        PlayerCommand::Load { index } => self.load(Some(index), false),
                        PlayerCommand::Play => self.play(),
                        PlayerCommand::Pause => self.pause(),
                        PlayerCommand::Stop => self.stop(),
                        PlayerCommand::Next => self.next(),
                        PlayerCommand::Previous => self.previous(),
                        PlayerCommand::Switch(index) => self.switch(index),
                        PlayerCommand::Seek(position) => self.seek(position),
                        PlayerCommand::AddToPlaylist{ tracks } => self.add_to_playlist(tracks).await,
                        PlayerCommand::RemoveFromPlaylist { index } => self.remove_from_playlist(index).await,
                        PlayerCommand::ClearPlaylist => self.clear_playlist().await,
                        PlayerCommand::MovePlayListItem {old_index, new_index} => self.move_playlist_item(old_index, new_index).await,
                        PlayerCommand::SetPlaybackMode(mode) => self.set_playback_mode(mode),
                        PlayerCommand::SetVolume(volume) => self.set_volume(volume),
                    }
                },
                Ok(fft_data) = fft_receiver.recv() => {
                    self.event_sender.send(PlayerEvent::RealtimeFFT(fft_data)).unwrap();
                },
                _ = progress_interval.tick() => {
                    if self.state != InternalPlaybackState::Stopped {
                        self.send_progress();
                    }
                },
                _ = async {
                    if let Some(timer) = self.debounce_timer {
                        sleep_until(timer).await;
                        true
                    } else {
                        false
                    }
                }, if self.debounce_timer.is_some() => {
                    self.debounce_timer = None;
                    self.send_playlist_updated();
                },
                _ = self.cancellation_token.cancelled() => {
                    debug!("Cancellation token triggered, exiting run loop");
                    self.stop();
                    break;
                }
            }
        }
    }

    fn load(&mut self, index: Option<usize>, play: bool) {
        if let Some(index) = index {
            debug!("Loading track at index: {}", index);
            let mapped_index = self.get_mapped_track_index(index);
            let item = &self.playlist[mapped_index];
            let file = File::open(item.path.clone());
            match file {
                Ok(file) => {
                    let source = Decoder::new(BufReader::new(file));

                    match source {
                        Ok(source) => {
                            let (stream, stream_handle) = OutputStream::try_default().unwrap();
                            let sink = Sink::try_new(&stream_handle).unwrap();
                            // Create a channel to transfer FFT data
                            let (fft_tx, mut fft_rx) = mpsc::unbounded_channel();

                            // Create a new thread for calculating realtime FFT
                            let realtime_fft = Arc::clone(&self.realtime_fft);
                            tokio::spawn(async move {
                                while let Some(data) = fft_rx.recv().await {
                                    realtime_fft.lock().unwrap().add_data(data);
                                }
                            });

                            sink.append(source.periodic_access(
                                Duration::from_millis(16),
                                move |sample| {
                                    let data: Vec<i16> =
                                        sample.take(sample.channels() as usize).collect();
                                    fft_tx.send(data).unwrap();
                                },
                            ));

                            sink.set_volume(self.volume);
                            if !play {
                                sink.pause();
                            }

                            self.sink = Some(sink);
                            self._stream = Some(stream);
                            self.current_track_index = Some(index);
                            self.current_track_id = Some(item.id);
                            self.current_track_path = Some(item.path.clone());
                            info!("Track loaded: {:?}", item.path);

                            if play {
                                self.event_sender
                                    .send(PlayerEvent::Playing {
                                        id: self.current_track_id.unwrap(),
                                        index: mapped_index,
                                        path: self.current_track_path.clone().unwrap(),
                                        playback_mode: self.playback_mode,
                                        position: Duration::new(0, 0),
                                    })
                                    .unwrap();
                                self.state = InternalPlaybackState::Playing;
                            }
                        }
                        Err(e) => {
                            error!("Failed to decode audio: {:?}", e);
                            self.event_sender
                                .send(PlayerEvent::Error {
                                    id: self.current_track_id.unwrap(),
                                    index: mapped_index,
                                    path: item.path.clone(),
                                    error: "Failed to decode audio".to_string(),
                                })
                                .unwrap();
                            self.state = InternalPlaybackState::Stopped;
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to open file: {:?}", e);
                    self.event_sender
                        .send(PlayerEvent::Error {
                            id: self.current_track_id.unwrap(),
                            index: mapped_index,
                            path: item.path.clone(),
                            error: "Failed to open file".to_string(),
                        })
                        .unwrap();
                    self.state = InternalPlaybackState::Stopped;
                }
            }
        } else {
            error!("Load command received without index");
        }
    }

    fn play(&mut self) {
        if let Some(sink) = &self.sink {
            sink.play();
            info!("Playback started");

            let track_index = self.current_track_index.unwrap();
            let track_index = self.get_mapped_track_index(track_index);
            self.event_sender
                .send(PlayerEvent::Playing {
                    id: self.current_track_id.unwrap(),
                    index: track_index,
                    path: self.current_track_path.clone().unwrap(),
                    playback_mode: self.playback_mode,
                    position: Duration::new(0, 0),
                })
                .unwrap();
            self.state = InternalPlaybackState::Playing;
        } else {
            info!("Loading the first track");
            self.load(Some(0), true);
            self.play();
        }
    }

    fn pause(&mut self) {
        if let Some(sink) = &self.sink {
            sink.pause();
            info!("Playback paused");

            let position = sink.get_pos();
            let track_index = self.current_track_index.unwrap();
            let track_index = self.get_mapped_track_index(track_index);
            self.event_sender
                .send(PlayerEvent::Paused {
                    id: self.current_track_id.unwrap(),
                    index: track_index,
                    path: self.current_track_path.clone().unwrap(),
                    playback_mode: self.playback_mode,
                    position,
                })
                .unwrap();
            self.state = InternalPlaybackState::Paused;
        }
    }

    fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
            info!("Playback stopped");
            self.event_sender.send(PlayerEvent::Stopped).unwrap();
            self.state = InternalPlaybackState::Stopped;
        } else {
            warn!("Stop command received but no track is loaded");
        }
    }

    fn next(&mut self) {
        if let Some(index) = self.current_track_index {
            match self.playback_mode {
                PlaybackMode::Sequential | PlaybackMode::RepeatOne => {
                    if index + 1 < self.playlist.len() {
                        self.load(Some(index + 1), true);
                    } else {
                        info!("End of playlist reached");
                        self.event_sender.send(PlayerEvent::EndOfPlaylist).unwrap();
                        self.state = InternalPlaybackState::Stopped;

                        self.load(Some(0), false);
                    }
                }
                PlaybackMode::RepeatAll => {
                    if index + 1 < self.playlist.len() {
                        self.load(Some(index + 1), true);
                    } else {
                        self.load(Some(0), true);
                    }
                }
                PlaybackMode::Shuffle => {
                    if index + 1 < self.playlist.len() {
                        self.load(Some(index + 1), true);
                    } else {
                        self.update_random_map();
                        self.load(Some(0), true);
                    }
                }
            }
        }
    }

    fn previous(&mut self) {
        if let Some(index) = self.current_track_index {
            match self.playback_mode {
                PlaybackMode::Sequential | PlaybackMode::RepeatOne => {
                    if index > 0 {
                        self.load(Some(index - 1), true);
                    } else {
                        info!("Begining of playlist reached");
                    }
                }
                PlaybackMode::RepeatAll => {
                    if index > 0 {
                        self.load(Some(index - 1), true);
                    } else {
                        self.load(Some(self.playlist.len() - 1), true);
                    }
                }
                PlaybackMode::Shuffle => {
                    if index > 0 {
                        self.load(Some(index - 1), true);
                    } else {
                        self.update_random_map();
                        self.load(Some(self.playlist.len() - 1), true);
                    }
                }
            }
        }
    }

    fn switch(&mut self, index: usize) {
        if index > 0 || index < self.playlist.len() {
            self.current_track_index = Some(index);
            debug!("Moving to previous track: {}", index);
            self.load(Some(index), true);
        } else {
            warn!("Previous command received but already at the first track");
        }
    }

    fn seek(&mut self, position: f64) {
        if let Some(sink) = &self.sink {
            match sink.try_seek(std::time::Duration::from_secs(position as u64)) {
                Ok(_) => {
                    info!("Seeking to position: {} s", position);

                    let position = sink.get_pos();
                    let track_index = self.current_track_index.unwrap();
                    let track_index = self.get_mapped_track_index(track_index);
                    match self.event_sender.send(PlayerEvent::Playing {
                        id: self.current_track_id.unwrap(),
                        index: track_index,
                        path: self.current_track_path.clone().unwrap(),
                        playback_mode: self.playback_mode,
                        position,
                    }) {
                        Ok(_) => (),
                        Err(e) => error!("Failed to send Playing event: {:?}", e),
                    }
                    self.state = InternalPlaybackState::Playing;
                }
                Err(e) => error!("Failed to seek: {:?}", e),
            }
        } else {
            warn!("Seek command received but no track is loaded");
        }
    }

    async fn add_to_playlist(&mut self, tracks: Vec<(i32, std::path::PathBuf)>) {
        debug!("Adding tracks to playlist");
        for track in tracks {
            self.playlist.push(PlaylistItem {
                id: track.0,
                path: track.1,
            });
        }
        self.update_random_map();
        self.schedule_playlist_update();
    }

    async fn remove_from_playlist(&mut self, index: usize) {
        if index < self.playlist.len() {
            debug!("Removing from playlist at index: {}", index);
            self.playlist.remove(index);
            self.update_random_map();
            self.schedule_playlist_update();
        } else {
            error!(
                "Remove command received but index {} is out of bounds",
                index
            );
        }
    }

    async fn clear_playlist(&mut self) {
        self.playlist.clear();
        self.update_random_map();
        self.current_track_index = None;
        self.sink = None;
        self._stream = None;
        info!("Playlist cleared");
        self.event_sender.send(PlayerEvent::Stopped).unwrap();
        self.schedule_playlist_update();
        self.state = InternalPlaybackState::Stopped;
    }

    fn set_playback_mode(&mut self, mode: PlaybackMode) {
        self.playback_mode = mode;
        if mode == PlaybackMode::Shuffle {
            self.update_random_map();
        }
        self.send_progress();
        info!("Playback mode set to {:?}", mode);
    }

    fn update_random_map(&mut self) {
        let shuffle_seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.random_map = get_random_sequence(shuffle_seed, self.playlist.len());
    }

    fn get_mapped_track_index(&mut self, index: usize) -> usize {
        if self.playback_mode == PlaybackMode::Shuffle {
            self.random_map[index]
        } else {
            index
        }
    }

    fn send_progress(&mut self) {
        let id = self.current_track_id;
        let index = self.current_track_index;
        let index = index.map(|x| self.get_mapped_track_index(x));
        let path = self.current_track_path.clone();
        let playback_mode = self.playback_mode;

        if let Some(sink) = &self.sink {
            let position = sink.get_pos();

            if sink.empty() {
                self.event_sender
                    .send(PlayerEvent::EndOfTrack {
                        id: id.unwrap(),
                        index: index.unwrap(),
                        path: path.unwrap(),
                        playback_mode,
                    })
                    .unwrap();

                if self.state != InternalPlaybackState::Stopped {
                    self.next();
                }
            } else {
                self.event_sender
                    .send(PlayerEvent::Progress {
                        id,
                        index,
                        path,
                        playback_mode,
                        position,
                        ready: true,
                    })
                    .unwrap();
            }
        } else {
            self.event_sender
                .send(PlayerEvent::Progress {
                    id,
                    index,
                    path,
                    playback_mode,
                    position: Duration::from_secs(0),
                    ready: false,
                })
                .unwrap();
        }
    }

    async fn move_playlist_item(&mut self, old_index: usize, new_index: usize) {
        if old_index >= self.playlist.len() || new_index >= self.playlist.len() {
            error!("Move command received but index is out of bounds");
            return;
        }

        if old_index == new_index {
            debug!("Move command received but old_index is the same as new_index");
            return;
        }

        debug!(
            "Moving playlist item from index {} to index {}",
            old_index, new_index
        );

        let item = self.playlist.remove(old_index);
        self.playlist.insert(new_index, item);
        self.update_random_map();

        // Adjust current track index if necessary
        if let Some(current_index) = self.current_track_index {
            if old_index == current_index {
                // The currently playing track was moved
                self.current_track_index = Some(new_index);
            } else if old_index < current_index && new_index >= current_index {
                // The track was moved past the current track
                self.current_track_index = Some(current_index - 1);
            } else if old_index > current_index && new_index <= current_index {
                // The track was moved before the current track
                self.current_track_index = Some(current_index + 1);
            }
        }

        self.schedule_playlist_update();
    }

    fn schedule_playlist_update(&mut self) {
        let debounce_duration = Duration::from_millis(60);
        self.debounce_timer = Some(Instant::now() + debounce_duration);
    }

    fn send_playlist_updated(&self) {
        self.event_sender
            .send(PlayerEvent::PlaylistUpdated(
                self.playlist.clone().into_iter().map(|x| x.id).collect(),
            ))
            .unwrap();
    }

    fn set_volume(&mut self, volume: f32) {
        if let Some(sink) = &self.sink {
            sink.set_volume(volume);
            self.volume = volume;
        }
    }
}
