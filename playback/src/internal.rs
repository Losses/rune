use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use log::{debug, error, info, warn};
use rodio::{Decoder, PlayError, Sink, Source};
use tokio::sync::mpsc;
use tokio::time::{interval, sleep_until, Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::output_stream::{RuneOutputStream, RuneOutputStreamHandle};
use crate::realtime_fft::RealTimeFFT;
use crate::strategies::{
    AddMode, PlaybackStrategy, RepeatAllStrategy, RepeatOneStrategy, SequentialStrategy,
    ShuffleStrategy, UpdateReason,
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
    SetRealtimeFFTEnabled(bool),
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

fn try_new_sink(stream: &RuneOutputStreamHandle) -> Result<Sink, PlayError> {
    let (sink, queue_rx) = Sink::new_idle();
    stream.play_raw(queue_rx)?;
    Ok(sink)
}

pub(crate) struct PlayerInternal {
    commands: mpsc::UnboundedReceiver<PlayerCommand>,
    event_sender: mpsc::UnboundedSender<PlayerEvent>,
    realtime_fft: Arc<Mutex<RealTimeFFT>>,
    fft_enabled: Arc<Mutex<bool>>,
    playlist: Vec<PlaylistItem>,
    current_track_id: Option<i32>,
    current_track_index: Option<usize>,
    current_track_path: Option<PathBuf>,
    sink: Option<Sink>,
    _stream: Option<RuneOutputStream>,
    state: InternalPlaybackState,
    debounce_timer: Option<Instant>,
    cancellation_token: CancellationToken,
    playback_mode: PlaybackMode,
    playback_strategy: Box<dyn PlaybackStrategy>,
    volume: f32,
    stream_error_sender: mpsc::UnboundedSender<String>,
    stream_error_receiver: mpsc::UnboundedReceiver<String>,
    stream_retry_count: usize,
}

impl PlayerInternal {
    pub fn new(
        commands: mpsc::UnboundedReceiver<PlayerCommand>,
        event_sender: mpsc::UnboundedSender<PlayerEvent>,
        cancellation_token: CancellationToken,
    ) -> Self {
        let (stream_error_sender, stream_error_receiver) = mpsc::unbounded_channel();
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
            fft_enabled: Arc::new(Mutex::new(false)),
            stream_error_sender,
            stream_error_receiver,
            stream_retry_count: 0,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut progress_interval = interval(Duration::from_millis(100));

        let mut fft_receiver = match self.realtime_fft.lock() {
            Ok(fft) => fft.subscribe(),
            Err(e) => {
                bail!("Failed to lock realtime FFT: {:?}", e);
            }
        };

        loop {
            tokio::select! {
                Some(cmd) = self.commands.recv() => {
                    if self.cancellation_token.is_cancelled() {
                        debug!("Cancellation token triggered, exiting run loop");
                        self.stop()?;
                        break;
                    }

                    debug!("Received command: {:?}", cmd);
                    match cmd {
                        PlayerCommand::Load { index } => self.load(Some(index), false, true),
                        PlayerCommand::Play => self.play(),
                        PlayerCommand::Pause => self.pause(),
                        PlayerCommand::Stop => self.stop(),
                        PlayerCommand::Next => self.next(),
                        PlayerCommand::Previous => self.previous(),
                        PlayerCommand::Switch(index) => self.switch(index),
                        PlayerCommand::Seek(position) => self.seek(position),
                        PlayerCommand::AddToPlaylist { tracks, mode } => {
                            self.add_to_playlist(tracks, mode);
                            Ok(())
                        },
                        PlayerCommand::RemoveFromPlaylist { index } => self.remove_from_playlist(index),
                        PlayerCommand::ClearPlaylist => self.clear_playlist(),
                        PlayerCommand::MovePlayListItem {old_index, new_index} => {
                            self.move_playlist_item(old_index, new_index);
                            Ok(())
                        },
                        PlayerCommand::SetPlaybackMode(mode) => self.set_playback_mode(mode),
                        PlayerCommand::SetVolume(volume) => self.set_volume(volume),
                        PlayerCommand::SetRealtimeFFTEnabled(enabled) => self.set_realtime_fft_enabled(enabled),
                    }?;
                },
                Ok(fft_data) = fft_receiver.recv() => {
                    if let Err(e) = self.event_sender.send(PlayerEvent::RealtimeFFT(fft_data)) {
                        error!("Failed to send FFT data: {:?}", e);
                    }
                },
                _ = progress_interval.tick() => {
                    if self.state != InternalPlaybackState::Stopped {
                        self.send_progress()?;
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
                    self.send_playlist_updated()?;
                },
                Some(error_message) = self.stream_error_receiver.recv() => {
                    self.stop()?;
                    error!("Received error message: {}", error_message);

                    if self.stream_retry_count < 5 {
                        if let Some(index) = self.current_track_index {
                            self.stream_retry_count += 1;
                            info!("Retrying to load track (attempt {} of 5)", self.stream_retry_count);
                            if let Err(e) = self.load(Some(index), false, true) {
                                error!("Retry failed: {:?}", e);
                            }
                        }
                    } else {
                        error!("Max retry attempts reached, reporting error");
                        self.event_sender.send(PlayerEvent::Error {
                            id: self.current_track_id.unwrap_or(-1),
                            index: self.current_track_index.unwrap_or(0),
                            path: self.current_track_path.clone().unwrap_or_default(),
                            error: error_message,
                        }).context("Failed to send Error event")?;
                        self.stream_retry_count = 0;
                    }
                },
                _ = self.cancellation_token.cancelled() => {
                    debug!("Cancellation token triggered, exiting run loop");
                    self.stop()?;
                    break;
                }
            }
        }

        Ok(())
    }

    fn load(&mut self, index: Option<usize>, play: bool, mapped: bool) -> Result<()> {
        if let Some(index) = index {
            debug!("Loading track at index: {}", index);
            let mapped_index = if mapped {
                self.get_mapped_track_index(index)
            } else {
                index
            };
            let item = &self.playlist[mapped_index];
            let file = File::open(item.path.clone())
                .with_context(|| format!("Failed to open file: {:?}", item.path))?;
            let source =
                Decoder::new(BufReader::new(file)).with_context(|| "Failed to decode audio")?;

            let (stream, stream_handle) = RuneOutputStream::try_default_with_callback({
                let error_sender = self.stream_error_sender.clone();
                move |error| {
                    let _ = error_sender.send(error.to_string());
                }
            })
            .context("Failed to create output stream")?;
            let sink = try_new_sink(&stream_handle).context("Failed to create sink")?;

            // Create a channel to transfer FFT data
            let (fft_tx, mut fft_rx) = mpsc::unbounded_channel();

            // Create a new thread for calculating realtime FFT
            let realtime_fft = Arc::clone(&self.realtime_fft);
            let fft_enabled = Arc::clone(&self.fft_enabled);
            tokio::spawn(async move {
                while let Some(data) = fft_rx.recv().await {
                    if let Ok(enabled) = fft_enabled.lock() {
                        if *enabled {
                            if let Ok(fft) = realtime_fft.lock() {
                                fft.add_data(data);
                            }
                        }
                    }
                }
            });

            sink.set_volume(self.volume);
            sink.append(
                source.periodic_access(Duration::from_millis(16), move |sample| {
                    let data: Vec<i16> = sample.take(sample.channels() as usize).collect();
                    if fft_tx.send(data).is_err() {
                        error!("Failed to send FFT data");
                    }
                }),
            );

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
                        id: self
                            .current_track_id
                            .ok_or(anyhow!("Track id unavailable"))?,
                        index: mapped_index,
                        path: self
                            .current_track_path
                            .clone()
                            .ok_or(anyhow!("Current track id unavailable"))?,
                        playback_mode: self.playback_mode,
                        position: Duration::new(0, 0),
                    })
                    .context("Failed to send Playing event")?;
                self.state = InternalPlaybackState::Playing;
            }
        } else {
            error!("Load command received without index");
        }
        Ok(())
    }

    fn play(&mut self) -> Result<()> {
        if let Some(sink) = &self.sink {
            sink.play();
            info!("Playback started");

            if let Some(track_index) = self.current_track_index {
                let track_index = self.get_mapped_track_index(track_index);
                self.event_sender.send(PlayerEvent::Playing {
                    id: self
                        .current_track_id
                        .ok_or(anyhow!("Current track id unavailable"))?,
                    index: track_index,
                    path: self
                        .current_track_path
                        .clone()
                        .ok_or(anyhow!("Current track path unavailable"))?,
                    playback_mode: self.playback_mode,
                    position: Duration::new(0, 0),
                })?;
                self.state = InternalPlaybackState::Playing;
            }
        } else {
            info!("Loading the first track");
            self.load(Some(0), true, true)
                .with_context(|| "Failed to load the first track")?;
            self.play()?;
        }

        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        if let Some(sink) = &self.sink {
            sink.pause();
            info!("Playback paused");

            let position = sink.get_pos();
            if let Some(track_index) = self.current_track_index {
                let track_index = self.get_mapped_track_index(track_index);
                self.event_sender.send(PlayerEvent::Paused {
                    id: self.current_track_id.unwrap(),
                    index: track_index,
                    path: self.current_track_path.clone().unwrap(),
                    playback_mode: self.playback_mode,
                    position,
                })?;
                self.state = InternalPlaybackState::Paused;
            }
        }

        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        if let Some(sink) = self.sink.take() {
            sink.stop();
            info!("Playback stopped");
            self.event_sender
                .send(PlayerEvent::Stopped)
                .with_context(|| "Failed to send Stopped event")?;
            self.state = InternalPlaybackState::Stopped;
        } else {
            warn!("Stop command received but no track is loaded");
        }

        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        if let Some(index) = self.current_track_index {
            if let Some(next_index) = self.playback_strategy.next(index, self.playlist.len()) {
                self.load(Some(next_index), true, true)
                    .with_context(|| "Failed to load next track")?;
            } else {
                info!("End of playlist reached");
                self.event_sender
                    .send(PlayerEvent::EndOfPlaylist)
                    .with_context(|| "Failed to send EndOfPlaylist event")?;

                if let Some(start_index) =
                    self.playback_strategy.on_playlist_end(self.playlist.len())
                {
                    self.load(Some(start_index), false, true)
                        .with_context(|| "Failed to load track at start index")?;
                } else {
                    self.stop()?;
                }
            }
        }

        Ok(())
    }

    fn previous(&mut self) -> Result<()> {
        if let Some(index) = self.current_track_index {
            if let Some(prev_index) = self.playback_strategy.previous(index, self.playlist.len()) {
                self.load(Some(prev_index), true, true)
                    .with_context(|| "Failed to load previous track")?;
            } else {
                info!("Beginning of playlist reached");
            }
        }

        Ok(())
    }

    fn switch(&mut self, index: usize) -> Result<()> {
        if index < self.playlist.len() {
            debug!("Switching to track at index: {}", index);
            self.load(Some(index), true, false)
                .with_context(|| "Failed to switch track")?;
        } else {
            warn!(
                "Switch command received but index {} is out of bounds",
                index
            );
        }

        Ok(())
    }

    fn seek(&mut self, position: f64) -> Result<()> {
        if let Some(sink) = &self.sink {
            if sink
                .try_seek(std::time::Duration::from_secs_f64(position))
                .is_ok()
            {
                info!("Seeking to position: {} s", position);

                let position = sink.get_pos();
                if let Some(track_index) = self.current_track_index {
                    let track_index = self.get_mapped_track_index(track_index);

                    self.event_sender
                        .send(PlayerEvent::Playing {
                            id: self
                                .current_track_id
                                .ok_or(anyhow!("Current track id unavailable"))?,
                            index: track_index,
                            path: self
                                .current_track_path
                                .clone()
                                .ok_or(anyhow!("Current track path unavailable"))?,
                            playback_mode: self.playback_mode,
                            position,
                        })
                        .with_context(|| "Failed to send Playing event")?;

                    self.state = InternalPlaybackState::Playing;
                }
            } else {
                error!("Failed to seek");
            }
        } else {
            warn!("Seek command received but no track is loaded");
        }

        Ok(())
    }

    fn add_to_playlist(&mut self, tracks: Vec<(i32, std::path::PathBuf)>, mode: AddMode) {
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

    fn remove_from_playlist(&mut self, index: usize) -> Result<()> {
        if index < self.playlist.len() {
            debug!("Removing from playlist at index: {}", index);
            self.playlist.remove(index);
            self.playback_strategy.on_playlist_updated(
                self.playlist.len(),
                UpdateReason::RemoveFromPlaylist { index },
            );
            self.schedule_playlist_update();
        } else {
            bail!(
                "Remove command received but index {} is out of bounds",
                index
            );
        }

        Ok(())
    }

    fn clear_playlist(&mut self) -> Result<()> {
        self.playlist.clear();
        self.playback_strategy
            .on_playlist_updated(0, UpdateReason::ClearPlaylist);
        self.current_track_index = None;
        self.sink = None;
        self._stream = None;
        info!("Playlist cleared");
        self.event_sender
            .send(PlayerEvent::Stopped)
            .with_context(|| "Failed to send Stopped event")?;
        self.schedule_playlist_update();
        self.state = InternalPlaybackState::Stopped;

        Ok(())
    }

    fn set_playback_mode(&mut self, mode: PlaybackMode) -> Result<()> {
        self.playback_mode = mode;
        self.playback_strategy = match mode {
            PlaybackMode::Sequential => Box::new(SequentialStrategy),
            PlaybackMode::RepeatOne => Box::new(RepeatOneStrategy),
            PlaybackMode::RepeatAll => Box::new(RepeatAllStrategy),
            PlaybackMode::Shuffle => Box::new(ShuffleStrategy::new(self.playlist.len())),
        };
        self.send_progress()?;
        info!("Playback mode set to {:?}", mode);

        Ok(())
    }

    fn get_mapped_track_index(&self, index: usize) -> usize {
        self.playback_strategy
            .get_mapped_track_index(index, self.playlist.len())
    }

    fn send_progress(&mut self) -> Result<()> {
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
                    .with_context(|| "Failed to send EndOfTrack event")?;

                if self.state != InternalPlaybackState::Stopped {
                    self.next()?;
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
                    .with_context(|| "Failed to send Progress event")?;
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
                .with_context(|| "Failed to send Progress event")?;
        }

        Ok(())
    }

    fn move_playlist_item(&mut self, old_index: usize, new_index: usize) {
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
        self.debounce_timer = Some(Instant::now() + Duration::from_millis(100));
    }

    fn send_playlist_updated(&self) -> Result<()> {
        let playlist_ids: Vec<i32> = self.playlist.iter().map(|item| item.id).collect();
        self.event_sender
            .send(PlayerEvent::PlaylistUpdated(playlist_ids))
            .with_context(|| "Failed to send PlaylistUpdated event")?;

        Ok(())
    }

    fn set_volume(&mut self, volume: f32) -> Result<()> {
        self.volume = volume;
        if let Some(sink) = &self.sink {
            sink.set_volume(volume);
        }
        self.event_sender
            .send(PlayerEvent::VolumeUpdate(volume))
            .with_context(|| "Failed to send VolumeUpdate event")?;

        Ok(())
    }

    fn set_realtime_fft_enabled(&mut self, x: bool) -> Result<()> {
        if let Ok(mut enabled) = self.fft_enabled.lock() {
            *enabled = x;
        }

        Ok(())
    }
}
