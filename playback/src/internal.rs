use crate::stream_utils::ManagedStream;
use std::{
    collections::HashMap,
    fmt::Debug,
    future::Future,
    io::{BufReader, Error, ErrorKind},
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use fsio::{FileStream, FsIo};
use log::{debug, error, info, warn};
use rodio::{PlayError, Sink, Source, source::SeekError};
use tokio::{
    sync::mpsc,
    time::{Instant, interval, sleep_until},
};
use tokio_util::sync::CancellationToken;

use crate::buffered::{RuneBuffered, rune_buffered};
use crate::decoder::RuneDecoder;
use crate::output_stream::{RuneOutputStream, RuneOutputStreamHandle};
use crate::pcm_tap::PcmTap;
use crate::player::{
    PlaybackSource, PlaybackSourceResolver, PlayingItem, RemoteAudioError, UnavailableReason,
};
use crate::realtime_fft::RealTimeFFT;
use crate::shared_source::SharedSource;
use crate::strategies::{
    AddMode, PlaybackStrategy, RepeatAllStrategy, RepeatOneStrategy, SequentialStrategy,
    ShuffleStrategy, UpdateReason,
};

const SOURCE_START_TIMEOUT: Duration = Duration::from_secs(30);

async fn bound_source_start<T>(
    limit: Duration,
    source: impl Future<Output = Result<T>>,
) -> Result<T> {
    match tokio::time::timeout(limit, source).await {
        Ok(result) => result,
        Err(_) => Err(anyhow!(RemoteAudioError::unreachable(Error::new(
            ErrorKind::TimedOut,
            "audio source did not start before the deadline",
        )))),
    }
}

async fn probe_decoder<R>(
    reader: R,
    extension: Option<String>,
    known_len: Option<u64>,
) -> Result<RuneDecoder<R>>
where
    R: std::io::Read + std::io::Seek + Send + Sync + 'static,
{
    tokio::task::spawn_blocking(move || {
        RuneDecoder::new_with_length(reader, extension.as_deref(), known_len)
    })
    .await
    .context("decoder probe task failed")?
}

pub enum AnySource {
    Local(RuneBuffered<RuneDecoder<BufReader<Box<dyn FileStream>>>>),
    Online(
        RuneBuffered<RuneDecoder<ManagedStream>>,
        Arc<Mutex<Option<UnavailableReason>>>,
    ),
}

async fn open_library_stream(
    resolver: &Arc<dyn PlaybackSourceResolver>,
    id: i32,
) -> Result<(AnySource, PathBuf)> {
    debug!("Resolving remote audio source for library track {id}");
    let remote = resolver.resolve(id).await?;
    let size = remote.size;
    let guard = remote.cancellation.clone().drop_guard();
    debug!("Opening initial HTTP range for library track {id}");
    let reader =
        crate::stream_utils::create_stream_with_authorizer(remote.request, &remote.url).await?;
    let failed = remote.failure_reason;
    let reader = ManagedStream::new_with_failure(reader, remote.cancellation, failed.clone());
    debug!("Probing decoder for library track {id}");
    let decoder = probe_decoder(reader, Some(remote.extension), Some(size)).await?;
    guard.disarm();
    debug!("Decoder ready for library track {id}");
    Ok((
        AnySource::Online(rune_buffered(decoder), failed),
        PathBuf::new(),
    ))
}

impl Debug for AnySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnySource::Local(_) => f.debug_struct("AnySource::Local").finish(),
            AnySource::Online(_, _) => f.debug_struct("AnySource::Online").finish(),
        }
    }
}

impl AnySource {
    fn current_samples(&self) -> Option<Vec<i16>> {
        match self {
            AnySource::Local(s) => s.current_samples(),
            AnySource::Online(s, _) => s.current_samples(),
        }
    }
}

impl Iterator for AnySource {
    type Item = i16;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            AnySource::Local(s) => s.next(),
            AnySource::Online(s, _) => s.next(),
        }
    }
}

impl Source for AnySource {
    fn current_frame_len(&self) -> Option<usize> {
        match self {
            AnySource::Local(s) => s.current_frame_len(),
            AnySource::Online(s, _) => s.current_frame_len(),
        }
    }

    fn channels(&self) -> u16 {
        match self {
            AnySource::Local(s) => s.channels(),
            AnySource::Online(s, _) => s.channels(),
        }
    }

    fn sample_rate(&self) -> u32 {
        match self {
            AnySource::Local(s) => s.sample_rate(),
            AnySource::Online(s, _) => s.sample_rate(),
        }
    }

    fn total_duration(&self) -> Option<Duration> {
        match self {
            AnySource::Local(s) => s.total_duration(),
            AnySource::Online(s, _) => s.total_duration(),
        }
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        match self {
            AnySource::Local(s) => s.try_seek(pos),
            AnySource::Online(s, _) => s.try_seek(pos),
        }
    }
}

impl AnySource {
    fn stream_failure(&self) -> Option<Arc<Mutex<Option<UnavailableReason>>>> {
        match self {
            AnySource::Local(_) => None,
            AnySource::Online(_, failed) => Some(failed.clone()),
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
    Restore {
        tracks: Vec<(PlayingItem, PlaybackSource)>,
        index: Option<usize>,
        position: Duration,
        mode: PlaybackMode,
    },
    Load {
        index: usize,
    },
    LoadAtPosition {
        index: usize,
        position: Duration,
        play: bool,
    },
    LoadComplete {
        generation: u64,
        result: Box<Result<AnySource>>,
        item: PlayingItem,
        original_item: PlayingItem,
        index: usize,
        playlist_index: usize,
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
        tracks: Vec<(PlayingItem, PlaybackSource)>,
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
    Unavailable(PlayingItem, UnavailableReason),
    UnavailableCleared(Option<PlayingItem>),
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
    pub source: PlaybackSource,
}

#[derive(Debug, PartialEq)]
enum InternalPlaybackState {
    Playing,
    Paused,
    Stopped,
    Loading,
}

#[derive(Debug, Clone, Copy)]
struct PendingLoad {
    index: usize,
    play: bool,
    mapped: bool,
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
    fsio: Arc<FsIo>,
    source_resolver: Arc<RwLock<Option<Arc<dyn PlaybackSourceResolver>>>>,
    realtime_fft: Arc<Mutex<RealTimeFFT>>,
    fft_enabled: Arc<Mutex<bool>>,
    pcm_tap: Arc<PcmTap>,
    playlist: Vec<PlaylistItem>,
    current_item: Option<PlayingItem>,
    current_track_index: Option<usize>,
    current_track_path: Option<PathBuf>,
    active_stream_failure: Option<Arc<Mutex<Option<UnavailableReason>>>>,
    sink: Option<Sink>,
    _stream: Option<RuneOutputStream>,
    state: InternalPlaybackState,
    pending_load: Option<PendingLoad>,
    load_generation: u64,
    debounce_timer: Option<Instant>,
    cancellation_token: CancellationToken,
    playback_mode: PlaybackMode,
    playback_strategy: Box<dyn PlaybackStrategy>,
    volume: f32,
    stream_error_sender: mpsc::UnboundedSender<String>,
    stream_error_receiver: mpsc::UnboundedReceiver<String>,
    stream_retry_count: usize,
    adaptive_switching: bool,
    restore_position: Option<Duration>,
    unavailable: HashMap<PlayingItem, UnavailableReason>,
}

impl PlayerInternal {
    pub fn new(
        commands: mpsc::UnboundedReceiver<PlayerCommand>,
        event_sender: mpsc::UnboundedSender<PlayerEvent>,
        cancellation_token: CancellationToken,
        commands_sender: mpsc::UnboundedSender<PlayerCommand>,
        fsio: Arc<FsIo>,
        pcm_tap: Arc<PcmTap>,
        source_resolver: Arc<RwLock<Option<Arc<dyn PlaybackSourceResolver>>>>,
    ) -> Self {
        let (stream_error_sender, stream_error_receiver) = mpsc::unbounded_channel();
        Self {
            commands,
            commands_sender,
            event_sender,
            fsio,
            source_resolver,
            playlist: Vec::new(),
            current_item: None,
            current_track_index: None,
            current_track_path: None,
            active_stream_failure: None,
            sink: None,
            _stream: None,
            realtime_fft: Arc::new(Mutex::new(RealTimeFFT::new(512))),
            state: InternalPlaybackState::Stopped,
            pending_load: None,
            load_generation: 0,
            debounce_timer: None,
            cancellation_token,
            playback_mode: PlaybackMode::Sequential,
            playback_strategy: Box::new(SequentialStrategy),
            volume: 1.0,
            fft_enabled: Arc::new(Mutex::new(false)),
            pcm_tap,
            stream_error_sender,
            stream_error_receiver,
            stream_retry_count: 0,
            adaptive_switching: false,
            restore_position: None,
            unavailable: HashMap::new(),
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
                        PlayerCommand::Restore { tracks, index, position, mode } => {
                            self.clear_unavailable(None)?;
                            self.add_to_playlist(tracks, AddMode::AppendToEnd);
                            self.debounce_timer = None;
                            self.send_playlist_updated()?;
                            self.set_playback_mode(mode)?;
                            self.restore_position = Some(position);
                            self.load(index, false, false)?;
                        },
                        PlayerCommand::Load { index } => {
                            if index < self.playlist.len() {
                                let mapped = self.get_mapped_track_index(index);
                                self.clear_unavailable(Some(self.playlist[mapped].item.clone()))?;
                            }
                            self.load(Some(index), false, true)?;
                        },
                        PlayerCommand::LoadAtPosition { index, position, play } => {
                            if index < self.playlist.len() {
                                let mapped = self.get_mapped_track_index(index);
                                self.clear_unavailable(Some(self.playlist[mapped].item.clone()))?;
                            }
                            self.restore_position = Some(position);
                            self.load(Some(index), play, true)?;
                        },
                        PlayerCommand::LoadComplete { generation, result, item, original_item, index, playlist_index, path, play } => {
                            if generation != self.load_generation {
                                debug!("Dropping stale load completion generation {generation}; current generation is {}", self.load_generation);
                                continue;
                            }
                            self.state = InternalPlaybackState::Stopped;
                            if let Some(pending) = self.pending_load.take() {
                                debug!("Load {index} completed after a newer request; starting index {}", pending.index);
                                self.load(Some(pending.index), pending.play, pending.mapped)?;
                                continue;
                            }
                            match *result {
                                Ok(source) => {
                                    if let Some(entry) = self.playlist.get_mut(playlist_index)
                                        && entry.item == original_item && entry.item != item
                                    {
                                        entry.item = item.clone();
                                        self.schedule_playlist_update();
                                    }
                                    self.setup_sink(source, item, index, path, play)?;
                                    if let Some(position) = self.restore_position.take() { self.seek(position.as_secs_f64())?; }
                                }
                                Err(e) => {
                                    error!("Failed to load track: {e:?}");
                                    self.restore_position = None;
                                    let reason = UnavailableReason::classify(&e);
                                    self.unavailable.insert(item.clone(), reason);
                                    self.event_sender.send(PlayerEvent::Unavailable(item, reason))?;
                                    // Advance from the failed load, including when no track has loaded yet.
                                    let next = self.find_available_after(playlist_index);
                                    if let Some(next) = next { self.load(Some(next), play, true)?; }
                                    else { self.stop()?; self.event_sender.send(PlayerEvent::EndOfPlaylist)?; }
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
            if let Some(index) = index {
                debug!("Load {index} queued while another track is loading; newest request wins");
                self.pending_load = Some(PendingLoad {
                    index,
                    play,
                    mapped,
                });
            }
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

            self.load_generation = self.load_generation.wrapping_add(1);
            let generation = self.load_generation;

            let item = self.playlist[mapped_index].clone();
            debug!("Starting track load at playlist index {mapped_index} (request index {index})");
            let commands_sender = self.commands_sender.clone();
            let fsio = Arc::clone(&self.fsio);
            let resolver = match self.source_resolver.read() {
                Ok(slot) => slot.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            };
            let load_cancel = self.cancellation_token.clone();

            tokio::spawn(async move {
                let original_item = item.item.clone();
                let source_future = async {
                    let mut fsio = fsio;
                    let mut resolver = resolver;
                    if let (PlayingItem::InLibrary(identity), Some(root)) =
                        (&item.item, resolver.as_ref())
                        && let Some(library) =
                            root.for_library(identity.library_id.to_string()).await?
                    {
                        fsio = library.fsio;
                        resolver = Some(library.resolver);
                    }
                    let local_file_id = if let (PlayingItem::InLibrary(identity), Some(resolver)) =
                        (&item.item, resolver.as_ref())
                    {
                        debug!("Resolving source identity for track load {index}");
                        Some(
                            resolver
                                .resolve_id(identity.file_uuid)
                                .await?
                                .context("library track no longer exists")?,
                        )
                    } else {
                        None
                    };
                    let resolved_item = item.item.clone();
                    match &resolved_item {
                        PlayingItem::IndependentFile(_) | PlayingItem::InLibrary(_) => {
                            let path = if let (Some(id), Some(resolver)) =
                                (local_file_id, resolver.as_ref())
                            {
                                let requires_stream = item
                                    .source
                                    .requires_stream_at_load(id, resolver.as_ref())
                                    .await?;
                                debug!(
                                    "Track load {index} requires remote streaming: {requires_stream}"
                                );
                                if requires_stream {
                                    let (source, path) = open_library_stream(resolver, id).await?;
                                    return Ok((source, path, resolved_item.clone()));
                                }
                                resolver
                                    .local_path(id)
                                    .await?
                                    .context("current local audio is unavailable")?
                            } else {
                                match &item.source {
                                    PlaybackSource::Local(path) => path.clone(),
                                    PlaybackSource::LibraryStream => {
                                        bail!("library stream resolver unavailable")
                                    }
                                }
                            };
                            let stream = fsio
                                .open(&path, "r")
                                .with_context(|| format!("Failed to open file: {path:?}"))?;
                            let ext = path.extension().and_then(|e| e.to_str());
                            let decoder =
                                probe_decoder(BufReader::new(stream), ext.map(str::to_owned), None)
                                    .await?;
                            Ok((
                                AnySource::Local(rune_buffered(decoder)),
                                path,
                                resolved_item.clone(),
                            ))
                        }
                        PlayingItem::Online(url, _) => {
                            info!("Downloading from url: {url}");
                            let ext = url.split('?').next().and_then(|u| u.rsplit('.').next());
                            let reader = crate::stream_utils::create_stream_from_url(url).await?;
                            let failed = Arc::new(Mutex::new(None));
                            let reader = ManagedStream::new_with_failure(
                                reader,
                                CancellationToken::new(),
                                failed.clone(),
                            );
                            let decoder =
                                probe_decoder(reader, ext.map(str::to_owned), None).await?;
                            Ok((
                                AnySource::Online(rune_buffered(decoder), failed),
                                PathBuf::new(),
                                resolved_item.clone(),
                            ))
                        }
                        PlayingItem::Unknown => {
                            bail!("Cannot load unknown item");
                        }
                    }
                };
                let source_result = tokio::select! {
                    _ = load_cancel.cancelled() => Err(anyhow!("library closed")),
                    result = bound_source_start(SOURCE_START_TIMEOUT, source_future) => result,
                };

                debug!(
                    "Track load {index} source resolution finished: {}",
                    if source_result.is_ok() {
                        "ready"
                    } else {
                        "failed"
                    }
                );

                let (source_result, path, resolved_item) = match source_result {
                    Ok((source, path, resolved_item)) => (Ok(source), path, resolved_item),
                    Err(error) => (Err(error), PathBuf::new(), item.item),
                };
                let cmd = PlayerCommand::LoadComplete {
                    generation,
                    result: Box::new(source_result),
                    item: resolved_item,
                    original_item,
                    index,
                    playlist_index: mapped_index,
                    path,
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
        self.active_stream_failure = source.stream_failure();
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
        let source = source.periodic_access(
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
        );
        sink.append(self.pcm_tap.wrap(source));

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
        self.load_generation = self.load_generation.wrapping_add(1);
        self.pending_load = None;
        self.state = InternalPlaybackState::Stopped;
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
        self.active_stream_failure = None;
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
            self.clear_unavailable(Some(self.playlist[index].item.clone()))?;
            self.load(Some(index), true, false)?;
        } else {
            warn!("Switch command received but index {index} is out of bounds");
        }

        Ok(())
    }

    fn seek(&mut self, position: f64) -> Result<()> {
        if let Some(sink) = &self.sink {
            match sink.try_seek(Duration::from_secs_f64(position)) {
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

    fn add_to_playlist(&mut self, tracks: Vec<(PlayingItem, PlaybackSource)>, mode: AddMode) {
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
                        source: track.1,
                    },
                );
            }
        } else {
            self.playlist
                .extend(tracks.into_iter().map(|track| PlaylistItem {
                    item: track.0,
                    source: track.1,
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
            let removed = self.playlist.remove(index);
            self.clear_unavailable(Some(removed.item))?;
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
        self.load_generation = self.load_generation.wrapping_add(1);
        self.clear_unavailable(None)?;
        self.playlist.clear();
        self.playback_strategy
            .on_playlist_updated(0, UpdateReason::ClearPlaylist);
        self.current_track_index = None;
        self.active_stream_failure = None;
        self.pending_load = None;
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

    fn find_available_after(&self, failed_playlist_index: usize) -> Option<usize> {
        let playlist_len = self.playlist.len();
        if playlist_len == 0 {
            return None;
        }
        let strategy_index = (0..playlist_len)
            .find(|index| self.get_mapped_track_index(*index) == failed_playlist_index)?;
        let repeat = matches!(
            self.playback_mode,
            PlaybackMode::RepeatAll | PlaybackMode::Shuffle
        );
        for offset in 1..=playlist_len {
            if !repeat && strategy_index + offset >= playlist_len {
                break;
            }
            let candidate = (strategy_index + offset) % playlist_len;
            let mapped = self.get_mapped_track_index(candidate);
            if !self.unavailable.contains_key(&self.playlist[mapped].item) {
                return Some(candidate);
            }
        }
        None
    }

    fn clear_unavailable(&mut self, item: Option<PlayingItem>) -> Result<()> {
        let changed = match &item {
            Some(item) => self.unavailable.remove(item).is_some(),
            None => {
                let changed = !self.unavailable.is_empty();
                self.unavailable.clear();
                changed
            }
        };
        if changed {
            self.event_sender
                .send(PlayerEvent::UnavailableCleared(item))
                .context("Failed to clear unavailable status")?;
        }
        Ok(())
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
                if self
                    .active_stream_failure
                    .as_ref()
                    .is_some_and(|failed| failed.lock().is_ok_and(|reason| reason.is_some()))
                {
                    let reason = self
                        .active_stream_failure
                        .as_ref()
                        .and_then(|failed| failed.lock().ok().and_then(|reason| *reason))
                        .unwrap_or(UnavailableReason::SourceUnreachable);
                    let item = id.context("failed stream has no current item")?;
                    let failed_index = index.context("failed stream has no current index")?;
                    self.stop()?;
                    self.unavailable.insert(item.clone(), reason);
                    self.event_sender
                        .send(PlayerEvent::Unavailable(item, reason))?;
                    if let Some(next) = self.find_available_after(failed_index) {
                        self.load(Some(next), true, true)?;
                    } else {
                        self.event_sender.send(PlayerEvent::EndOfPlaylist)?;
                    }
                    return Ok(());
                }
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

#[cfg(test)]
mod stream_open_tests {
    use super::*;
    use crate::player::{AudioRequest, LibraryItem, LibraryPlaybackSource, RemoteAudioSource};
    use futures::future::BoxFuture;
    use std::collections::{HashMap, HashSet};
    use std::io::{Read, Seek, SeekFrom};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, channel};
    use tokio::sync::Notify;
    use tokio::task::LocalSet;
    use uuid::Uuid;

    struct FailingRequest;
    impl AudioRequest for FailingRequest {
        fn get(
            &self,
            _: Option<(u64, Option<u64>)>,
        ) -> BoxFuture<'static, Result<reqwest::Response>> {
            Box::pin(async { anyhow::bail!("request failed") })
        }
    }
    struct Resolver(CancellationToken);
    impl PlaybackSourceResolver for Resolver {
        fn resolve(&self, _: i32) -> BoxFuture<'static, Result<RemoteAudioSource>> {
            let cancellation = self.0.clone();
            Box::pin(async move {
                Ok(RemoteAudioSource {
                    request: Arc::new(FailingRequest),
                    url: "http://localhost/audio".into(),
                    extension: "mp3".into(),
                    size: 1,
                    cancellation,
                    failure_reason: Arc::new(Mutex::new(None)),
                })
            })
        }
    }
    #[tokio::test]
    async fn failed_stream_creation_cancels_watcher() {
        let token = CancellationToken::new();
        let resolver: Arc<dyn PlaybackSourceResolver> = Arc::new(Resolver(token.clone()));
        assert!(open_library_stream(&resolver, 1).await.is_err());
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn source_start_timeout_uses_unavailable_network_reason() {
        let error = bound_source_start(Duration::from_millis(1), async {
            std::future::pending::<Result<()>>().await
        })
        .await
        .expect_err("a source that never starts must time out");
        assert_eq!(
            UnavailableReason::classify(&error),
            UnavailableReason::SourceUnreachable
        );
    }

    struct StalledReader {
        release: Arc<Mutex<Receiver<()>>>,
        released: Arc<AtomicBool>,
        probe_finished: Arc<Notify>,
    }

    impl Read for StalledReader {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            if !self.released.swap(true, Ordering::SeqCst) {
                self.release.lock().unwrap().recv().unwrap();
            }
            self.probe_finished.notify_one();
            Ok(0)
        }
    }

    impl Seek for StalledReader {
        fn seek(&mut self, _: SeekFrom) -> std::io::Result<u64> {
            Ok(0)
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocking_decoder_probe_respects_source_start_timeout() -> Result<()> {
        let (release_sender, release_receiver) = channel();
        let probe_finished = Arc::new(Notify::new());
        let release_done = probe_finished.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            release_sender.send(()).unwrap();
            release_done.notified().await;
        });

        let reader = StalledReader {
            release: Arc::new(Mutex::new(release_receiver)),
            released: Arc::new(AtomicBool::new(false)),
            probe_finished: probe_finished.clone(),
        };
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            bound_source_start(
                Duration::from_millis(25),
                probe_decoder(reader, Some("ogg".to_owned()), None),
            ),
        )
        .await?;
        let error = match result {
            Ok(_) => panic!("blocking decoder probe must be bounded"),
            Err(error) => error,
        };
        assert_eq!(
            UnavailableReason::classify(&error),
            UnavailableReason::SourceUnreachable
        );
        tokio::time::timeout(Duration::from_secs(2), probe_finished.notified()).await?;
        Ok(())
    }

    struct DelayedResolver {
        ids: Arc<Mutex<Vec<i32>>>,
        started: Arc<Notify>,
        release: Arc<Notify>,
        file_ids: Arc<HashMap<Uuid, i32>>,
        file_path: PathBuf,
        blocked_ids: Arc<HashSet<i32>>,
    }

    impl PlaybackSourceResolver for DelayedResolver {
        fn for_library(
            &self,
            _: String,
        ) -> BoxFuture<'static, Result<Option<LibraryPlaybackSource>>> {
            let ids = self.ids.clone();
            let started = self.started.clone();
            let release = self.release.clone();
            let file_ids = self.file_ids.clone();
            let file_path = self.file_path.clone();
            let blocked_ids = self.blocked_ids.clone();
            Box::pin(async move {
                Ok(Some(LibraryPlaybackSource {
                    fsio: Arc::new(FsIo::new()),
                    resolver: Arc::new(DelayedLibraryResolver {
                        ids,
                        started,
                        release,
                        file_ids,
                        file_path,
                        blocked_ids,
                    }),
                }))
            })
        }

        fn resolve(&self, _: i32) -> BoxFuture<'static, Result<RemoteAudioSource>> {
            Box::pin(async { anyhow::bail!("unexpected remote source resolution") })
        }
    }

    struct DelayedLibraryResolver {
        ids: Arc<Mutex<Vec<i32>>>,
        started: Arc<Notify>,
        release: Arc<Notify>,
        file_ids: Arc<HashMap<Uuid, i32>>,
        file_path: PathBuf,
        blocked_ids: Arc<HashSet<i32>>,
    }

    impl PlaybackSourceResolver for DelayedLibraryResolver {
        fn resolve_id(&self, file_uuid: Uuid) -> BoxFuture<'static, Result<Option<i32>>> {
            let id = self.file_ids.get(&file_uuid).copied();
            Box::pin(async move { Ok(id) })
        }

        fn requires_stream(&self, id: i32) -> BoxFuture<'static, Result<bool>> {
            let ids = self.ids.clone();
            let started = self.started.clone();
            let release = self.release.clone();
            let blocked_ids = self.blocked_ids.clone();
            Box::pin(async move {
                ids.lock().unwrap().push(id);
                if blocked_ids.contains(&id) {
                    started.notify_one();
                    release.notified().await;
                }
                Ok(false)
            })
        }

        fn local_path(&self, _: i32) -> BoxFuture<'static, Result<Option<PathBuf>>> {
            let path = self.file_path.clone();
            Box::pin(async move { Ok(Some(path)) })
        }

        fn resolve(&self, _: i32) -> BoxFuture<'static, Result<RemoteAudioSource>> {
            Box::pin(async { anyhow::bail!("unexpected remote source resolution") })
        }
    }

    fn delayed_resolver_fixture(
        file_uuids: &[Uuid],
        ids: Arc<Mutex<Vec<i32>>>,
        started: Arc<Notify>,
        release: Arc<Notify>,
        blocked_ids: &[i32],
    ) -> (Arc<DelayedResolver>, PathBuf) {
        let wav_path = std::env::temp_dir().join(format!("playback-{}.wav", Uuid::new_v4()));
        let mut wav = Vec::with_capacity(48);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&40_u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&8_000_u32.to_le_bytes());
        wav.extend_from_slice(&16_000_u32.to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&4_u32.to_le_bytes());
        wav.extend_from_slice(&[0; 4]);
        std::fs::write(&wav_path, wav).unwrap();
        let resolver = Arc::new(DelayedResolver {
            ids,
            started,
            release,
            file_ids: Arc::new(
                file_uuids
                    .iter()
                    .enumerate()
                    .map(|(index, file_uuid)| (*file_uuid, index as i32 + 1))
                    .collect(),
            ),
            file_path: wav_path.clone(),
            blocked_ids: Arc::new(blocked_ids.iter().copied().collect()),
        });
        (resolver, wav_path)
    }

    #[tokio::test]
    async fn newest_switch_during_load_replaces_the_pending_track() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let ids = Arc::new(Mutex::new(Vec::new()));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) = delayed_resolver_fixture(
            &file_uuids,
            ids.clone(),
            started.clone(),
            release.clone(),
            &[1],
        );
        let (commands_sender, commands) = mpsc::unbounded_channel();
        let (event_sender, _events) = mpsc::unbounded_channel();
        let mut internal = PlayerInternal::new(
            commands,
            event_sender,
            CancellationToken::new(),
            commands_sender.clone(),
            Arc::new(FsIo::new_noop()),
            PcmTap::new(),
            Arc::new(RwLock::new(Some(resolver))),
        );
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    commands_sender.send(PlayerCommand::AddToPlaylist {
                        tracks: file_uuids
                            .iter()
                            .map(|file_uuid| {
                                (
                                    PlayingItem::InLibrary(LibraryItem {
                                        library_id,
                                        file_uuid: *file_uuid,
                                    }),
                                    PlaybackSource::LibraryStream,
                                )
                            })
                            .collect(),
                        mode: AddMode::AppendToEnd,
                    })?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::Switch(2))?;
                    release.notify_one();

                    tokio::time::timeout(Duration::from_secs(2), async {
                        loop {
                            if ids.lock().unwrap().len() >= 2 {
                                break;
                            }
                            tokio::task::yield_now().await;
                        }
                    })
                    .await?;
                    assert_eq!(&*ids.lock().unwrap(), &[1, 3]);

                    runner.abort();
                    let _ = std::fs::remove_file(wav_path);
                    Ok::<(), anyhow::Error>(())
                }
                .await
            })
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn clear_replace_during_load_finishes_on_the_new_first_track() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let ids = Arc::new(Mutex::new(Vec::new()));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) = delayed_resolver_fixture(
            &file_uuids,
            ids.clone(),
            started.clone(),
            release.clone(),
            &[1, 2],
        );
        let (commands_sender, commands) = mpsc::unbounded_channel();
        let (event_sender, mut events) = mpsc::unbounded_channel();
        let mut internal = PlayerInternal::new(
            commands,
            event_sender,
            CancellationToken::new(),
            commands_sender.clone(),
            Arc::new(FsIo::new_noop()),
            PcmTap::new(),
            Arc::new(RwLock::new(Some(resolver))),
        );
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let track = |file_uuid| {
                        (
                            PlayingItem::InLibrary(LibraryItem {
                                library_id,
                                file_uuid,
                            }),
                            PlaybackSource::LibraryStream,
                        )
                    };
                    commands_sender.send(PlayerCommand::AddToPlaylist {
                        tracks: vec![track(file_uuids[0])],
                        mode: AddMode::AppendToEnd,
                    })?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::ClearPlaylist)?;
                    commands_sender.send(PlayerCommand::AddToPlaylist {
                        tracks: vec![track(file_uuids[1]), track(file_uuids[2])],
                        mode: AddMode::AppendToEnd,
                    })?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    commands_sender.send(PlayerCommand::Play)?;
                    tokio::time::timeout(Duration::from_secs(2), async {
                        loop {
                            if ids.lock().unwrap().len() >= 2 {
                                break;
                            }
                            tokio::task::yield_now().await;
                        }
                    })
                    .await?;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    release.notify_one();

                    tokio::time::sleep(Duration::from_millis(100)).await;
                    assert_eq!(&*ids.lock().unwrap(), &[1, 2]);
                    release.notify_one();
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    release.notify_one();

                    let expected = PlayingItem::InLibrary(LibraryItem {
                        library_id,
                        file_uuid: file_uuids[1],
                    });
                    tokio::time::timeout(Duration::from_secs(3), async {
                        loop {
                            if let Some(PlayerEvent::Playing { item, .. }) = events.recv().await
                                && item == expected
                            {
                                break;
                            }
                        }
                    })
                    .await?;
                    assert_eq!(&*ids.lock().unwrap(), &[1, 2, 2]);

                    runner.abort();
                    let _ = std::fs::remove_file(wav_path);
                    Ok::<(), anyhow::Error>(())
                }
                .await
            })
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn stop_during_load_drops_the_late_completion() -> Result<()> {
        let file_uuids = [Uuid::new_v4()];
        let ids = Arc::new(Mutex::new(Vec::new()));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) =
            delayed_resolver_fixture(&file_uuids, ids, started.clone(), release.clone(), &[1]);
        let (commands_sender, commands) = mpsc::unbounded_channel();
        let (event_sender, mut events) = mpsc::unbounded_channel();
        let mut internal = PlayerInternal::new(
            commands,
            event_sender,
            CancellationToken::new(),
            commands_sender.clone(),
            Arc::new(FsIo::new_noop()),
            PcmTap::new(),
            Arc::new(RwLock::new(Some(resolver))),
        );
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let expected = PlayingItem::InLibrary(LibraryItem {
                        library_id,
                        file_uuid: file_uuids[0],
                    });
                    commands_sender.send(PlayerCommand::AddToPlaylist {
                        tracks: vec![(expected.clone(), PlaybackSource::LibraryStream)],
                        mode: AddMode::AppendToEnd,
                    })?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::Stop)?;
                    release.notify_one();
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    while let Ok(event) = events.try_recv() {
                        assert!(!matches!(
                            event,
                            PlayerEvent::Loaded { item, .. } | PlayerEvent::Playing { item, .. }
                                if item == expected
                        ));
                    }

                    runner.abort();
                    let _ = std::fs::remove_file(wav_path);
                    Ok::<(), anyhow::Error>(())
                }
                .await
            })
            .await?;
        Ok(())
    }
}
