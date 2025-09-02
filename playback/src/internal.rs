use std::{
    fmt::Debug,
    fs::File,
    io::BufReader,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use log::{debug, error, info, warn};
use rodio::{Decoder, PlayError, Sink, Source, source::SeekError};
use stream_download::{StreamDownload, storage::temp::TempStorageProvider};
use tokio::{
    sync::mpsc,
    time::{Instant, interval, sleep_until},
};
use tokio_util::sync::CancellationToken;

use crate::buffered::{RuneBuffered, rune_buffered};
use crate::output_stream::{RuneOutputStream, RuneOutputStreamHandle};
use crate::player::PlayingItem;
use crate::realtime_fft::RealTimeFFT;
use crate::shared_source::SharedSource;
use crate::strategies::{
    AddMode, PlaybackStrategy, RepeatAllStrategy, RepeatOneStrategy, SequentialStrategy,
    ShuffleStrategy, UpdateReason,
};

pub enum AnySource {
    Local(RuneBuffered<Decoder<BufReader<File>>>),
    Online(RuneBuffered<Decoder<StreamDownload<TempStorageProvider>>>),
}

impl Debug for AnySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnySource::Local(_) => f.debug_struct("AnySource::Local").finish(),
            AnySource::Online(_) => f.debug_struct("AnySource::Online").finish(),
        }
    }
}

impl AnySource {
    fn current_samples(&self) -> Option<Vec<i16>> {
        match self {
            AnySource::Local(s) => s.current_samples(),
            AnySource::Online(s) => s.current_samples(),
        }
    }
}

impl Iterator for AnySource {
    type Item = i16;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            AnySource::Local(s) => s.next(),
            AnySource::Online(s) => s.next(),
        }
    }
}

impl Source for AnySource {
    fn current_frame_len(&self) -> Option<usize> {
        match self {
            AnySource::Local(s) => s.current_frame_len(),
            AnySource::Online(s) => s.current_frame_len(),
        }
    }

    fn channels(&self) -> u16 {
        match self {
            AnySource::Local(s) => s.channels(),
            AnySource::Online(s) => s.channels(),
        }
    }

    fn sample_rate(&self) -> u32 {
        match self {
            AnySource::Local(s) => s.sample_rate(),
            AnySource::Online(s) => s.sample_rate(),
        }
    }

    fn total_duration(&self) -> Option<Duration> {
        match self {
            AnySource::Local(s) => s.total_duration(),
            AnySource::Online(s) => s.total_duration(),
        }
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        match self {
            AnySource::Local(s) => s.try_seek(pos),
            AnySource::Online(s) => s.try_seek(pos),
        }
    }
}

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
    LoadComplete {
        result: Box<Result<AnySource>>,
        item: PlayingItem,
        index: usize,
        path: PathBuf,
        play: bool,
    },
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    Switch(usize),
    Seek(f64),
    AddToPlaylist {
        tracks: Vec<(PlayingItem, std::path::PathBuf)>,
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
    SetAdaptiveSwitchingEnabled(bool),
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Stopped,
    Loaded {
        item: PlayingItem,
        index: usize,
        path: PathBuf,
        playback_mode: PlaybackMode,
        position: Duration,
    },
    Playing {
        item: PlayingItem,
        index: usize,
        path: PathBuf,
        playback_mode: PlaybackMode,
        position: Duration,
    },
    Paused {
        item: PlayingItem,
        index: usize,
        path: PathBuf,
        playback_mode: PlaybackMode,
        position: Duration,
    },
    EndOfPlaylist,
    EndOfTrack {
        item: PlayingItem,
        index: usize,
        path: PathBuf,
        playback_mode: PlaybackMode,
    },
    Error {
        item: PlayingItem,
        index: usize,
        path: PathBuf,
        error: String,
    },
    Progress {
        item: Option<PlayingItem>,
        index: Option<usize>,
        path: Option<PathBuf>,
        position: Duration,
        playback_mode: PlaybackMode,
        ready: bool,
    },
    VolumeUpdate(f32),
    PlaylistUpdated(Vec<PlayingItem>),
    RealtimeFFT(Vec<f32>),
    Log(InternalLog),
}

#[derive(Debug, Clone)]
pub struct PlaylistItem {
    pub item: PlayingItem,
    pub path: PathBuf,
}

#[derive(Debug, PartialEq)]
enum InternalPlaybackState {
    Playing,
    Paused,
    Stopped,
    Loading,
}

fn try_new_sink(stream: &RuneOutputStreamHandle) -> Result<Sink, PlayError> {
    let (sink, queue_rx) = Sink::new_idle();
    stream.play_raw(queue_rx)?;
    Ok(sink)
}

#[derive(Debug, Clone)]
pub struct InternalLog {
    pub domain: String,
    pub error: String,
}

impl<S> Source for SharedSource<S>
where
    S: Source,
    S::Item: rodio::Sample,
{
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.lock().unwrap().current_frame_len()
    }

    fn channels(&self) -> u16 {
        self.inner.lock().unwrap().channels()
    }

    fn sample_rate(&self) -> u32 {
        self.inner.lock().unwrap().sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.lock().unwrap().total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.inner.lock().unwrap().try_seek(pos)
    }
}

pub(crate) struct PlayerInternal {
    commands: mpsc::UnboundedReceiver<PlayerCommand>,
    commands_sender: mpsc::UnboundedSender<PlayerCommand>,
    event_sender: mpsc::UnboundedSender<PlayerEvent>,
    realtime_fft: Arc<Mutex<RealTimeFFT>>,
    fft_enabled: Arc<Mutex<bool>>,
    playlist: Vec<PlaylistItem>,
    current_item: Option<PlayingItem>,
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
    adaptive_switching: bool,
}

impl PlayerInternal {
    pub fn new(
        commands: mpsc::UnboundedReceiver<PlayerCommand>,
        event_sender: mpsc::UnboundedSender<PlayerEvent>,
        cancellation_token: CancellationToken,
        commands_sender: mpsc::UnboundedSender<PlayerCommand>,
    ) -> Self {
        let (stream_error_sender, stream_error_receiver) = mpsc::unbounded_channel();
        Self {
            commands,
            commands_sender,
            event_sender,
            playlist: Vec::new(),
            current_item: None,
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
            adaptive_switching: false,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut progress_interval = interval(Duration::from_millis(100));

        let fft_receiver = match self.realtime_fft.lock() {
            Ok(fft) => fft.subscribe(),
            Err(e) => {
                bail!("Failed to lock realtime FFT: {e:?}");
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

                    debug!("Received command: {cmd:?}");
                    match cmd {
                        PlayerCommand::Load { index } => self.load(Some(index), false, true)?,
                        PlayerCommand::LoadComplete { result, item, index, path, play } => {
                            self.state = InternalPlaybackState::Stopped;
                            match *result {
                                Ok(source) => {
                                    self.setup_sink(source, item, index, path, play)?;
                                }
                                Err(e) => {
                                    error!("Failed to load track: {e:?}");
                                    self.next()?;
                                }
                            }
                        },
                        PlayerCommand::Play => self.play()?,
                        PlayerCommand::Pause => self.pause()?,
                        PlayerCommand::Stop => self.stop()?,
                        PlayerCommand::Next => self.next()?,
                        PlayerCommand::Previous => self.previous()?,
                        PlayerCommand::Switch(index) => self.switch(index)?,
                        PlayerCommand::Seek(position) => self.seek(position)?,
                        PlayerCommand::AddToPlaylist { tracks, mode } => {
                            self.add_to_playlist(tracks, mode);
                        },
                        PlayerCommand::RemoveFromPlaylist { index } => self.remove_from_playlist(index)?,
                        PlayerCommand::ClearPlaylist => self.clear_playlist()?,
                        PlayerCommand::MovePlayListItem {old_index, new_index} => {
                            self.move_playlist_item(old_index, new_index);
                        },
                        PlayerCommand::SetPlaybackMode(mode) => self.set_playback_mode(mode)?,
                        PlayerCommand::SetVolume(volume) => self.set_volume(volume)?,
                        PlayerCommand::SetRealtimeFFTEnabled(enabled) => self.set_realtime_fft_enabled(enabled)?,
                        PlayerCommand::SetAdaptiveSwitchingEnabled(enabled) => self.set_adaptive_switching(enabled)?,
                    };
                },
                Ok(fft_data) = fft_receiver.recv() => {
                    if let Err(e) = self.event_sender.send(PlayerEvent::RealtimeFFT(fft_data)) {
                        error!("Failed to send FFT data: {e:?}");
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
                    error!("Received error message: {error_message}");

                    if self.stream_retry_count < 5 {
                        if let Some(index) = self.current_track_index {
                            self.stream_retry_count += 1;
                            info!("Retrying to load track (attempt {} of 5)", self.stream_retry_count);
                            if let Err(e) = self.load(Some(index), false, true) {
                                error!("Retry failed: {e:?}");
                            }
                        }
                    } else {
                        error!("Max retry attempts reached, reporting error");
                        self.event_sender.send(PlayerEvent::Error {
                            item: self.current_item.clone().unwrap_or(PlayingItem::Unknown),
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
        if self.state == InternalPlaybackState::Loading {
            warn!("Already loading a track, ignoring new load request.");
            return Ok(());
        }

        if let Some(index) = index {
            self.state = InternalPlaybackState::Loading;

            let mapped_index = if mapped {
                self.get_mapped_track_index(index)
            } else {
                index
            };

            if mapped_index >= self.playlist.len() {
                warn!("Index {mapped_index} exceed the boundary");
                self.state = InternalPlaybackState::Stopped;
                return Ok(());
            }

            let item = self.playlist[mapped_index].clone();
            let commands_sender = self.commands_sender.clone();

            tokio::spawn(async move {
                let source_result = async {
                    match &item.item {
                        PlayingItem::IndependentFile(_) | PlayingItem::InLibrary(_) => {
                            let file = File::open(item.path.clone())
                                .with_context(|| format!("Failed to open file: {:?}", item.path))?;
                            let decoder = Decoder::new(BufReader::new(file))?;
                            Ok(AnySource::Local(rune_buffered(decoder)))
                        }
                        PlayingItem::Online(url, _) => {
                            info!("Downloading from url: {url}");
                            let reader = crate::stream_utils::create_stream_from_url(url).await?;
                            let decoder = Decoder::new(reader)?;
                            Ok(AnySource::Online(rune_buffered(decoder)))
                        }
                        PlayingItem::Unknown => {
                            bail!("Cannot load unknown item");
                        }
                    }
                }
                .await;

                let cmd = PlayerCommand::LoadComplete {
                    result: Box::new(source_result),
                    item: item.item,
                    index,
                    path: item.path,
                    play,
                };
                if commands_sender.send(cmd).is_err() {
                    error!("Failed to send LoadComplete command");
                }
            });
        }
        Ok(())
    }

    fn setup_sink(
        &mut self,
        source: AnySource,
        item: PlayingItem,
        index: usize,
        path: PathBuf,
        play: bool,
    ) -> Result<()> {
        let source = SharedSource::new(source);
        let source_for_fft = Arc::clone(&source.inner);

        let (stream, stream_handle) = RuneOutputStream::try_default_with_callback({
            let error_sender = self.stream_error_sender.clone();
            move |error| {
                let _ = error_sender.send(error.to_string());
            }
        })
        .context("Failed to create output stream")?;
        let sink = try_new_sink(&stream_handle).context("Failed to create sink")?;

        let (fft_tx, mut fft_rx) = mpsc::unbounded_channel();

        let realtime_fft = Arc::clone(&self.realtime_fft);
        let fft_enabled = Arc::clone(&self.fft_enabled);
        tokio::spawn(async move {
            while let Some(data) = fft_rx.recv().await {
                if let Ok(enabled) = fft_enabled.lock()
                    && *enabled
                    && let Ok(fft) = realtime_fft.lock()
                {
                    fft.add_data(data);
                }
            }
        });

        sink.set_volume(self.volume);
        sink.append(source.periodic_access(
            Duration::from_millis(12),
            move |_sample: &mut SharedSource<_>| {
                if let Ok(guard) = source_for_fft.lock() {
                    let data: Option<Vec<i16>> = guard.current_samples();
                    if let Some(data) = data
                        && fft_tx.send(data).is_err()
                    {
                        error!("Failed to send FFT data");
                    }
                }
            },
        ));

        if !play {
            sink.pause();
        }

        self.sink = Some(sink);
        self._stream = Some(stream);
        self.current_track_index = Some(index);
        self.current_item = Some(item.clone());
        self.current_track_path = Some(path.clone());
        info!("Track loaded: {path:?}");

        if play {
            self.event_sender
                .send(PlayerEvent::Playing {
                    item,
                    index,
                    path,
                    playback_mode: self.playback_mode,
                    position: Duration::new(0, 0),
                })
                .context("Failed to send Playing event")?;
            self.state = InternalPlaybackState::Playing;
        } else {
            self.event_sender
                .send(PlayerEvent::Loaded {
                    item,
                    index,
                    path,
                    playback_mode: self.playback_mode,
                    position: Duration::new(0, 0),
                })
                .context("Failed to send Playing event")?;
            self.state = InternalPlaybackState::Stopped;
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
                    item: self
                        .current_item
                        .clone()
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
            self.load(Some(0), true, true)?;
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
                    item: self.current_item.clone().unwrap_or(PlayingItem::Unknown),
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
                self.load(Some(next_index), true, true)?;
            } else {
                info!("End of playlist reached");
                self.event_sender
                    .send(PlayerEvent::EndOfPlaylist)
                    .with_context(|| "Failed to send EndOfPlaylist event")?;

                if let Some(start_index) =
                    self.playback_strategy.on_playlist_end(self.playlist.len())
                {
                    self.load(Some(start_index), false, true)?;
                } else {
                    self.stop()?;
                }
            }
        }

        Ok(())
    }

    fn do_switch_back(&mut self, index: usize) -> Result<()> {
        if let Some(prev_index) = self.playback_strategy.previous(index, self.playlist.len()) {
            self.load(Some(prev_index), true, true)?;
        } else {
            info!("Beginning of playlist reached");
        }

        Ok(())
    }

    fn previous(&mut self) -> Result<()> {
        if let Some(index) = self.current_track_index {
            match &self.sink {
                Some(sink) => {
                    let need_adaptive = sink.get_pos() > Duration::from_secs(3);

                    if self.adaptive_switching && need_adaptive {
                        self.load(Some(index), true, true)?;
                    } else {
                        return self.do_switch_back(index);
                    }
                    return Ok(());
                }
                None => return self.do_switch_back(index),
            }
        }

        Ok(())
    }

    fn switch(&mut self, index: usize) -> Result<()> {
        if index < self.playlist.len() {
            debug!("Switching to track at index: {}", { index });
            self.load(Some(index), true, false)?;
        } else {
            warn!("Switch command received but index {index} is out of bounds");
        }

        Ok(())
    }

    fn seek(&mut self, position: f64) -> Result<()> {
        if let Some(sink) = &self.sink {
            match sink.try_seek(std::time::Duration::from_secs_f64(position)) {
                Ok(_) => {
                    info!("Seeking to position: {position} s");

                    let position = sink.get_pos();
                    if let Some(track_index) = self.current_track_index {
                        let track_index = self.get_mapped_track_index(track_index);

                        if self.state == InternalPlaybackState::Playing {
                            self.event_sender
                                .send(PlayerEvent::Playing {
                                    item: self
                                        .current_item
                                        .clone()
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
                        } else {
                            self.event_sender
                                .send(PlayerEvent::Paused {
                                    item: self
                                        .current_item
                                        .clone()
                                        .ok_or(anyhow!("Current track id unavailable"))?,
                                    index: track_index,
                                    path: self
                                        .current_track_path
                                        .clone()
                                        .ok_or(anyhow!("Current track path unavailable"))?,
                                    playback_mode: self.playback_mode,
                                    position,
                                })
                                .with_context(|| "Failed to send Paused event")?;
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to seek: {e:#?}");
                }
            }
        } else {
            warn!("Seek command received but no track is loaded");
        }

        Ok(())
    }

    fn add_to_playlist(&mut self, tracks: Vec<(PlayingItem, std::path::PathBuf)>, mode: AddMode) {
        debug!("Adding tracks to playlist with mode: {:?}", { mode });
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
                        item: track.0,
                        path: track.1,
                    },
                );
            }
        } else {
            self.playlist
                .extend(tracks.into_iter().map(|track| PlaylistItem {
                    item: track.0,
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
            debug!("Removing from playlist at index: {}", { index });
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
        info!("Playback mode set to {:?}", { mode });

        Ok(())
    }

    fn get_mapped_track_index(&self, index: usize) -> usize {
        self.playback_strategy
            .get_mapped_track_index(index, self.playlist.len())
    }

    fn send_progress(&mut self) -> Result<()> {
        let id = self.current_item.clone();
        let index = self.current_track_index;
        let index = index.map(|x| self.get_mapped_track_index(x));
        let path = self.current_track_path.clone();
        let playback_mode = self.playback_mode;

        if let Some(sink) = &self.sink {
            let position = sink.get_pos();

            if sink.empty() {
                self.event_sender
                    .send(PlayerEvent::EndOfTrack {
                        item: id.unwrap(),
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
                        item: id,
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
                    item: id,
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

        debug!("Moving playlist item from index {old_index} to index {new_index}");

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
        let playlist_items: Vec<PlayingItem> =
            self.playlist.iter().map(|item| item.item.clone()).collect();
        self.event_sender
            .send(PlayerEvent::PlaylistUpdated(playlist_items))
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

    fn set_adaptive_switching(&mut self, x: bool) -> Result<()> {
        self.adaptive_switching = x;

        info!("Adaptive switching status changed: {:#?}", { x });

        Ok(())
    }
}
