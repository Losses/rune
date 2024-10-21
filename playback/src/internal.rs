use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use log::{debug, error, info, warn};
use rodio::{Decoder, OutputStream, Sink, Source};
use tokio::sync::mpsc;
use tokio::time::{interval, sleep_until, Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::realtime_fft::RealTimeFFT;
use crate::strategies::{
    AddMode, PlaybackStrategy, RepeatAllStrategy, RepeatOneStrategy, SequentialStrategy, ShuffleStrategy, UpdateReason
};

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
        mode: AddMode,
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
    playback_strategy: Box<dyn PlaybackStrategy>,
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
            playback_strategy: Box::new(SequentialStrategy),
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
                        PlayerCommand::AddToPlaylist { tracks, mode } => {
                            self.add_to_playlist(tracks, mode).await;
                        }
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
            if let Some(next_index) = self.playback_strategy.next(index, self.playlist.len()) {
                self.load(Some(next_index), true);
            } else {
                info!("End of playlist reached");
                self.event_sender.send(PlayerEvent::EndOfPlaylist).unwrap();
                self.state = InternalPlaybackState::Paused;

                if let Some(start_index) =
                    self.playback_strategy.on_playlist_end(self.playlist.len())
                {
                    self.load(Some(start_index), false);
                }
            }
        }
    }

    fn previous(&mut self) {
        if let Some(index) = self.current_track_index {
            if let Some(prev_index) = self.playback_strategy.previous(index, self.playlist.len()) {
                self.load(Some(prev_index), true);
            } else {
                info!("Beginning of playlist reached");
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

    async fn add_to_playlist(&mut self, tracks: Vec<(i32, std::path::PathBuf)>, mode: AddMode) {
        debug!("Adding tracks to playlist with mode: {:?}", mode);
        let insert_index = match mode {
            AddMode::PlayNext => {
                if let Some(current_index) = self.current_track_index {
                    Some(current_index + 1)
                } else {
                    Some(self.playlist.len())
                }
            }
            AddMode::AppendToEnd => None,
        };

        if let Some(index) = insert_index {
            for (i, track) in tracks.into_iter().enumerate() {
                self.playlist.insert(
                    index + i,
                    PlaylistItem {
                        id: track.0,
                        path: track.1,
                    },
                );
            }
        } else {
            self.playlist
                .extend(tracks.into_iter().map(|track| PlaylistItem {
                    id: track.0,
                    path: track.1,
                }));
        }

        self.playback_strategy.on_playlist_updated(
            self.playlist.len(),
            UpdateReason::AddToPlaylist {
                mode,
                index: insert_index,
            },
        );
        self.schedule_playlist_update();
    }

    async fn remove_from_playlist(&mut self, index: usize) {
        if index < self.playlist.len() {
            debug!("Removing from playlist at index: {}", index);
            self.playlist.remove(index);
            self.playback_strategy.on_playlist_updated(
                self.playlist.len(),
                UpdateReason::RemoveFromPlaylist { index },
            );
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
        self.playback_strategy
            .on_playlist_updated(0, UpdateReason::ClearPlaylist);
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
        self.playback_strategy = match mode {
            PlaybackMode::Sequential => Box::new(SequentialStrategy),
            PlaybackMode::RepeatOne => Box::new(RepeatOneStrategy),
            PlaybackMode::RepeatAll => Box::new(RepeatAllStrategy),
            PlaybackMode::Shuffle => Box::new(ShuffleStrategy::new(self.playlist.len())),
        };
        self.send_progress();
        info!("Playback mode set to {:?}", mode);
    }

    fn get_mapped_track_index(&self, index: usize) -> usize {
        self.playback_strategy
            .get_mapped_track_index(index, self.playlist.len())
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

        // Update the playback strategy to reflect changes in the playlist
        self.playback_strategy.on_playlist_updated(
            self.playlist.len(),
            UpdateReason::MovePlaylistItem {
                old_index,
                new_index,
            },
        );

        // Adjust the current track index if necessary
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
