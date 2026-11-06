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

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

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
    Output(OutputEvent),
}

/// What happened to this device's audio output. The player decides from its own state whether to
/// pause, resume or open the output again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputEvent {
    /// Something else took over the output, such as a call or an alarm, and the current stream
    /// stopped.
    Interrupted,
    /// The output is free again after an interruption. `resume` says whether playback that the
    /// interruption paused should go on.
    InterruptionEnded { resume: bool },
    /// The device that was playing went away, such as headphones that were unplugged or turned
    /// off.
    DeviceLost,
    /// A new device became available and the output moves to it, such as headphones that were
    /// plugged in.
    DeviceAdded,
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
    restore_position: Option<Duration>,
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
    // Whether the load in flight starts playing when it completes. Play and Pause change it
    // while the track is still loading.
    play_when_loaded: bool,
    debounce_timer: Option<Instant>,
    cancellation_token: CancellationToken,
    playback_mode: PlaybackMode,
    playback_strategy: Box<dyn PlaybackStrategy>,
    volume: f32,
    stream_error_sender: mpsc::UnboundedSender<String>,
    stream_error_receiver: mpsc::UnboundedReceiver<String>,
    stream_retry_count: usize,
    adaptive_switching: bool,
    // Where the current track starts once its output opens. Set when a load starts and taken
    // when the output opens. It stays while the track has no output that plays, so that the next
    // play opens one there: the track came up paused, no output could be opened, or a stale
    // output cannot seek.
    restore_position: Option<Duration>,
    // The current track has no output that can play: it came up paused and none was opened, none
    // could be opened, or its stream stopped. The next play opens one where the track stands.
    output_stale: bool,
    // An interruption paused playback, so playback goes on when the interruption ends.
    resume_after_interruption: bool,
    unavailable: HashMap<PlayingItem, UnavailableReason>,
    // How many of the next attempts to open an output fail.
    #[cfg(test)]
    failing_opens: Arc<AtomicUsize>,
    // How many times the player tried to open an output.
    #[cfg(test)]
    output_opens: Arc<AtomicUsize>,
    // How many times the player stopped an output while paused, and tried to start one again.
    #[cfg(test)]
    output_suspends: Arc<AtomicUsize>,
    #[cfg(test)]
    output_resumes: Arc<AtomicUsize>,
    // How many of the next attempts to start a stopped output fail.
    #[cfg(test)]
    failing_resumes: Arc<AtomicUsize>,
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
            play_when_loaded: false,
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
            output_stale: false,
            resume_after_interruption: false,
            unavailable: HashMap::new(),
            #[cfg(test)]
            failing_opens: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            output_opens: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            output_suspends: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            output_resumes: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            failing_resumes: Arc::new(AtomicUsize::new(0)),
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
                            self.load_at(index, false, false, Some(position))?;
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
                        PlayerCommand::LoadComplete { generation, result, item, original_item, index, playlist_index, path } => {
                            if generation != self.load_generation {
                                debug!("Dropping stale load completion generation {generation}; current generation is {}", self.load_generation);
                                continue;
                            }
                            let play = self.play_when_loaded;
                            self.state = InternalPlaybackState::Stopped;
                            if let Some(pending) = self.pending_load.take() {
                                debug!("Load {index} completed after a newer request; starting index {}", pending.index);
                                self.load_at(Some(pending.index), pending.play, pending.mapped, pending.restore_position)?;
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
                                    let start = self.restore_position.take();
                                    if play {
                                        if let Err(e) = self.setup_sink(source, item.clone(), index, path.clone(), start) {
                                            self.output_failed(e, item, index, path, start.unwrap_or_default())?;
                                        }
                                    } else {
                                        self.loaded_paused(item, index, path, start.unwrap_or_default())?;
                                    }
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
                        PlayerCommand::Pause => {
                            // A pause asked for during an interruption holds after it ends.
                            self.resume_after_interruption = false;
                            self.pause()?;
                        },
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
                        PlayerCommand::Output(event) => self.output_event(event)?,
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
                    error!("Received error message: {error_message}");

                    if self.stream_retry_count < 5 {
                        // The stream cannot play any more. A playing track goes on from the same
                        // place on a new stream now, and any other track gets one on the next play.
                        self.mark_output_stale();
                        if self.state == InternalPlaybackState::Playing {
                            self.stream_retry_count += 1;
                            info!("Opening the audio output again (attempt {} of 5)", self.stream_retry_count);
                            if let Err(e) = self.reopen_output() {
                                error!("Retry failed: {e:?}");
                            }
                        }
                    } else {
                        self.stop()?;
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
        self.load_at(index, play, mapped, None)
    }

    // Loads a track that seeks to `restore_position` once it is ready. The position belongs to
    // this request only: a newer request replaces it, whether it starts now or is queued.
    fn load_at(
        &mut self,
        index: Option<usize>,
        play: bool,
        mapped: bool,
        restore_position: Option<Duration>,
    ) -> Result<()> {
        if self.state == InternalPlaybackState::Loading {
            if let Some(index) = index {
                debug!("Load {index} queued while another track is loading; newest request wins");
                self.pending_load = Some(PendingLoad {
                    index,
                    play,
                    mapped,
                    restore_position,
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
            self.play_when_loaded = play;
            self.restore_position = restore_position;

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
                };
                if commands_sender.send(cmd).is_err() {
                    error!("Failed to send LoadComplete command");
                }
            });
        }
        Ok(())
    }

    fn open_output(&self) -> Result<(RuneOutputStream, RuneOutputStreamHandle)> {
        #[cfg(test)]
        {
            self.output_opens.fetch_add(1, Ordering::SeqCst);
            if self
                .failing_opens
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
            {
                bail!("the test output refused to open");
            }
        }
        // Every playback after an interruption opens a new stream, so whatever asked to play, the
        // session is set up and active again before the stream starts. A session that cannot be
        // activated, as during a call, fails the open, and the track waits paused for the next
        // play.
        #[cfg(target_os = "ios")]
        if !crate::ios_session::activate() {
            bail!("the audio session could not be activated");
        }
        RuneOutputStream::try_default_with_callback({
            let error_sender = self.stream_error_sender.clone();
            move |error| {
                let _ = error_sender.send(error.to_string());
            }
        })
        .context("Failed to create output stream")
    }

    // Opens an output for a loaded track and plays it from `start`. Only playback opens an
    // output; a track that comes up paused waits without one.
    fn setup_sink(
        &mut self,
        source: AnySource,
        item: PlayingItem,
        index: usize,
        path: PathBuf,
        start: Option<Duration>,
    ) -> Result<()> {
        let (stream, stream_handle) = self.open_output()?;
        let sink = try_new_sink(&stream_handle).context("Failed to create sink")?;
        self.output_stale = false;

        self.active_stream_failure = source.stream_failure();
        let source = SharedSource::new(source);
        let source_for_fft = Arc::clone(&source.inner);

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
        // Hold the track until it stands where it starts, so that nothing before that is heard.
        sink.pause();
        sink.append(self.pcm_tap.wrap(source));
        if let Some(start) = start
            && let Err(e) = sink.try_seek(start)
        {
            error!("Failed to seek to the start position {start:?}: {e:#?}");
        }
        sink.play();
        let position = sink.get_pos();

        self.sink = Some(sink);
        self._stream = Some(stream);
        self.current_track_index = Some(index);
        self.current_item = Some(item.clone());
        self.current_track_path = Some(path.clone());
        info!("Track loaded: {path:?}");

        self.event_sender
            .send(PlayerEvent::Playing {
                item,
                index,
                path,
                playback_mode: self.playback_mode,
                position,
            })
            .context("Failed to send Playing event")?;
        self.state = InternalPlaybackState::Playing;

        Ok(())
    }

    // A track that came up paused stays current without an output, as `Loaded` at `position`.
    // Opening one now would take the audio from other apps on iOS and keep an idle stream open
    // elsewhere. The first play loads the track again and opens an output there.
    fn loaded_paused(
        &mut self,
        item: PlayingItem,
        index: usize,
        path: PathBuf,
        position: Duration,
    ) -> Result<()> {
        info!("Track loaded paused: {path:?}");
        self.keep_without_output(item.clone(), index, path.clone(), position);
        self.event_sender
            .send(PlayerEvent::Loaded {
                item,
                index,
                path,
                playback_mode: self.playback_mode,
                position,
            })
            .context("Failed to send Loaded event")?;
        Ok(())
    }

    // Keeps a track current with no output, paused where it stands, and the next play opens one
    // there. Stopped inside, so that no progress is reported without a sink.
    fn keep_without_output(
        &mut self,
        item: PlayingItem,
        index: usize,
        path: PathBuf,
        position: Duration,
    ) {
        self.sink = None;
        self._stream = None;
        self.active_stream_failure = None;
        self.output_stale = true;
        self.restore_position = Some(position);
        self.current_track_index = Some(index);
        self.current_item = Some(item);
        self.current_track_path = Some(path);
        self.state = InternalPlaybackState::Stopped;
    }

    fn play(&mut self) -> Result<()> {
        // While a track loads, the previous track keeps the sink. It follows Play and Pause, but
        // the state stays Loading so that newer requests still queue behind the load.
        let loading = self.state == InternalPlaybackState::Loading;
        if loading {
            self.set_play_when_loaded(true);
        }

        if !loading && self.output_stale && self.current_track_index.is_some() {
            return self.reopen_output();
        }

        // The output stopped while paused, and starts again before the sink plays. One that cannot
        // start is of no more use: a loaded track opens a new one where it stands, and a track that
        // is loading opens its own.
        if self.output_suspended()
            && let Err(e) = self.resume_output()
        {
            error!("Failed to start the audio output again: {e:?}");
            self.mark_output_stale();
            return if loading {
                Ok(())
            } else {
                self.reopen_output()
            };
        }

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
                if !loading {
                    self.state = InternalPlaybackState::Playing;
                }
            }
        } else if !loading {
            info!("Loading the first track");
            self.load(Some(0), true, true)?;
        }

        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        let loading = self.state == InternalPlaybackState::Loading;
        if loading {
            self.set_play_when_loaded(false);
        }

        if let Some(sink) = &self.sink {
            sink.pause();
            self.suspend_output();
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
                if !loading {
                    self.state = InternalPlaybackState::Paused;
                }
            }
        }

        Ok(())
    }

    // Decides whether the track that is loading, and a newer one queued behind it, comes up
    // playing or paused.
    fn set_play_when_loaded(&mut self, play: bool) {
        self.play_when_loaded = play;
        if let Some(pending) = &mut self.pending_load {
            pending.play = play;
        }
    }

    fn output_event(&mut self, event: OutputEvent) -> Result<()> {
        info!("Audio output event: {event:?}");
        match event {
            OutputEvent::Interrupted => {
                let playing = match self.state {
                    InternalPlaybackState::Playing => true,
                    InternalPlaybackState::Loading => self.play_when_loaded,
                    _ => false,
                };
                if playing {
                    self.pause()?;
                    self.resume_after_interruption = true;
                }
                self.mark_output_stale();
            }
            OutputEvent::InterruptionEnded { resume } => {
                if std::mem::take(&mut self.resume_after_interruption) && resume {
                    self.play()?;
                }
            }
            OutputEvent::DeviceLost => {
                self.pause()?;
                self.resume_after_interruption = false;
                self.mark_output_stale();
            }
            OutputEvent::DeviceAdded => {
                self.mark_output_stale();
                // Move what is playing to the new device now. A track that is loading opens its
                // stream there anyway, and a paused one does on the next play.
                if self.state == InternalPlaybackState::Playing {
                    self.reopen_output()?;
                }
            }
        }
        Ok(())
    }

    fn mark_output_stale(&mut self) {
        if self.sink.is_some() {
            self.output_stale = true;
        }
    }

    // Stops the output while nothing plays from it, which only iOS needs: it shows the player as
    // playing for as long as its output runs. See `RuneOutputStream::suspend`.
    fn suspend_output(&self) {
        let Some(stream) = &self._stream else {
            return;
        };
        if stream.is_suspended() {
            return;
        }
        if let Err(e) = stream.suspend() {
            warn!("Failed to stop the audio output while nothing plays: {e:?}");
        }
        #[cfg(test)]
        if stream.is_suspended() {
            self.output_suspends.fetch_add(1, Ordering::SeqCst);
        }
    }

    // Whether the sink's output is stopped and can start again. A stale output is not started:
    // the next play opens a new one.
    fn output_suspended(&self) -> bool {
        self.sink.is_some()
            && !self.output_stale
            && self
                ._stream
                .as_ref()
                .is_some_and(RuneOutputStream::is_suspended)
    }

    // Starts a stopped output again. An interruption marks the output stale, so that a play after
    // one opens a new stream. The session is still made active first, as before any output
    // starts: a suspended app may wake to a remote command before the interruption reaches it.
    fn resume_output(&self) -> Result<()> {
        let Some(stream) = &self._stream else {
            return Ok(());
        };
        #[cfg(target_os = "ios")]
        if !crate::ios_session::activate() {
            bail!("The audio session could not be made active");
        }
        #[cfg(test)]
        {
            self.output_resumes.fetch_add(1, Ordering::SeqCst);
            if self
                .failing_resumes
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
            {
                bail!("the test output refused to start again");
            }
        }
        stream
            .resume()
            .context("Failed to start the output stream again")
    }

    // Opens a new output stream for the current track and plays on from where it stands. It loads
    // the track again, so the old stream is never used for a seek, which would wait for a stream
    // that no longer runs.
    fn reopen_output(&mut self) -> Result<()> {
        let Some(index) = self.current_track_index else {
            return Ok(());
        };
        let position = self.current_position().unwrap_or_default();
        if let Some(sink) = &self.sink {
            sink.pause();
        }
        info!("Opening the audio output again at {position:?}");
        self.load_at(Some(index), true, true, Some(position))
    }

    // Keeps the player running when no output stream could be opened, for example while another
    // app holds the audio. The track stays current and paused where it was, and the next play
    // tries again from there.
    fn output_failed(
        &mut self,
        error: anyhow::Error,
        item: PlayingItem,
        index: usize,
        path: PathBuf,
        position: Duration,
    ) -> Result<()> {
        error!("Failed to open the audio output: {error:?}");
        self.keep_without_output(item.clone(), index, path.clone(), position);
        self.event_sender
            .send(PlayerEvent::Paused {
                item,
                index: self.get_mapped_track_index(index),
                path,
                playback_mode: self.playback_mode,
                position,
            })
            .context("Failed to send Paused event")?;
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.load_generation = self.load_generation.wrapping_add(1);
        self.pending_load = None;
        self.state = InternalPlaybackState::Stopped;
        // The output plays nothing more: the next track opens one of its own.
        self.suspend_output();
        if let Some(sink) = self.sink.take() {
            sink.stop();
            info!("Playback stopped");
            self.event_sender
                .send(PlayerEvent::Stopped)
                .with_context(|| "Failed to send Stopped event")?;
            self.state = InternalPlaybackState::Stopped;
        } else if self.output_stale {
            // A track whose output failed to open still shows as paused.
            self.event_sender
                .send(PlayerEvent::Stopped)
                .with_context(|| "Failed to send Stopped event")?;
        } else {
            warn!("Stop command received but no track is loaded");
        }
        self.active_stream_failure = None;
        self.output_stale = false;
        self.resume_after_interruption = false;
        self.restore_position = None;
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
            match self.current_position() {
                Some(position) => {
                    let need_adaptive = position > Duration::from_secs(3);

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

    // Where the current track stands: where it starts once an output opens while it has none
    // that plays, or else where its sink is.
    fn current_position(&self) -> Option<Duration> {
        if self.output_stale
            && let Some(position) = self.restore_position
        {
            return Some(position);
        }
        match &self.sink {
            Some(sink) => Some(sink.get_pos()),
            None if self.output_stale => Some(Duration::ZERO),
            None => None,
        }
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
        // A stopped output pulls no samples, so the sink would wait for the seek forever. The
        // output runs while the paused sink takes the seek, and stops again. One that cannot start
        // is stale, and the seek goes as on any stale output.
        let suspended = self.output_suspended();
        if suspended && let Err(e) = self.resume_output() {
            error!("Failed to start the audio output for a seek: {e:?}");
            self.mark_output_stale();
        }

        if self.output_stale {
            // A stale stream never takes the seek, and waiting for it would stall the player.
            // Keep the place instead, and the next play opens a new stream there.
            if self.state == InternalPlaybackState::Loading {
                warn!("Seek ignored while the track that is loading replaces the audio output");
                return Ok(());
            }
            if let (Some(index), Some(item), Some(path)) = (
                self.current_track_index,
                self.current_item.clone(),
                self.current_track_path.clone(),
            ) {
                let position = Duration::from_secs_f64(position);
                self.restore_position = Some(position);
                self.event_sender
                    .send(PlayerEvent::Paused {
                        item,
                        index: self.get_mapped_track_index(index),
                        path,
                        playback_mode: self.playback_mode,
                        position,
                    })
                    .context("Failed to send Paused event")?;
            }
            return Ok(());
        }

        if let Some(sink) = &self.sink {
            let result = sink.try_seek(Duration::from_secs_f64(position));
            if suspended {
                self.suspend_output();
            }
            match result {
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
        self.restore_position = None;
        self.output_stale = false;
        self.resume_after_interruption = false;
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

            // While a track loads, the previous one keeps the sink and may still run out. Its end
            // either started this load or was skipped by the listener, so it must not move the
            // queue again.
            if sink.empty() && self.state != InternalPlaybackState::Loading {
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
            // A track without an output that plays is still ready, where it starts.
            self.event_sender
                .send(PlayerEvent::Progress {
                    item: id,
                    index,
                    path,
                    playback_mode,
                    position: self.current_position().unwrap_or_default(),
                    ready: self.output_stale && self.current_track_index.is_some(),
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
        delayed_resolver_fixture_with_samples(file_uuids, ids, started, release, blocked_ids, 2)
    }

    // Every track is the same silent 8 kHz mono WAV file with `samples` samples.
    fn delayed_resolver_fixture_with_samples(
        file_uuids: &[Uuid],
        ids: Arc<Mutex<Vec<i32>>>,
        started: Arc<Notify>,
        release: Arc<Notify>,
        blocked_ids: &[i32],
        samples: u32,
    ) -> (Arc<DelayedResolver>, PathBuf) {
        let wav_path = std::env::temp_dir().join(format!("playback-{}.wav", Uuid::new_v4()));
        let data_len = samples * 2;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&8_000_u32.to_le_bytes());
        wav.extend_from_slice(&16_000_u32.to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.resize(wav.len() + data_len as usize, 0);
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
                    assert_eq!(&*ids.lock().unwrap(), &[1, 2]);

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

    fn library_engine(
        resolver: Arc<DelayedResolver>,
    ) -> (
        PlayerInternal,
        mpsc::UnboundedSender<PlayerCommand>,
        mpsc::UnboundedReceiver<PlayerEvent>,
    ) {
        let (commands_sender, commands) = mpsc::unbounded_channel();
        let (event_sender, events) = mpsc::unbounded_channel();
        let internal = PlayerInternal::new(
            commands,
            event_sender,
            CancellationToken::new(),
            commands_sender.clone(),
            Arc::new(FsIo::new_noop()),
            PcmTap::new(),
            Arc::new(RwLock::new(Some(resolver))),
        );
        (internal, commands_sender, events)
    }

    fn library_item(library_id: Uuid, file_uuid: Uuid) -> PlayingItem {
        PlayingItem::InLibrary(LibraryItem {
            library_id,
            file_uuid,
        })
    }

    fn library_tracks(library_id: Uuid, file_uuids: &[Uuid]) -> Vec<(PlayingItem, PlaybackSource)> {
        file_uuids
            .iter()
            .map(|file_uuid| {
                (
                    library_item(library_id, *file_uuid),
                    PlaybackSource::LibraryStream,
                )
            })
            .collect()
    }

    fn add_library_tracks(library_id: Uuid, file_uuids: &[Uuid]) -> PlayerCommand {
        PlayerCommand::AddToPlaylist {
            tracks: library_tracks(library_id, file_uuids),
            mode: AddMode::AppendToEnd,
        }
    }

    // Waits for the next event that brings a track up, plays it or pauses it.
    async fn next_track_state(
        events: &mut mpsc::UnboundedReceiver<PlayerEvent>,
    ) -> Result<PlayerEvent> {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match events.recv().await {
                    Some(
                        event @ (PlayerEvent::Loaded { .. }
                        | PlayerEvent::Playing { .. }
                        | PlayerEvent::Paused { .. }),
                    ) => return Ok(event),
                    Some(_) => {}
                    None => bail!("player events closed"),
                }
            }
        })
        .await?
    }

    // Samples in a five-second track, long enough for positions to mean something.
    const LONG_TRACK_SAMPLES: u32 = 5 * 8_000;

    fn position_of(event: &PlayerEvent) -> Duration {
        match event {
            PlayerEvent::Loaded { position, .. }
            | PlayerEvent::Playing { position, .. }
            | PlayerEvent::Paused { position, .. } => *position,
            _ => panic!("{event:?} has no position"),
        }
    }

    // The output thread may have moved on by a few milliseconds when a position is read.
    fn assert_about(position: Duration, expected: Duration) {
        assert!(
            position + Duration::from_millis(50) >= expected
                && position <= expected + Duration::from_millis(300),
            "{position:?} is not about {expected:?}"
        );
    }

    async fn expect_playing(
        events: &mut mpsc::UnboundedReceiver<PlayerEvent>,
        track: &PlayingItem,
    ) -> Result<Duration> {
        let event = next_track_state(events).await?;
        assert!(
            matches!(&event, PlayerEvent::Playing { item, .. } if item == track),
            "{event:?}"
        );
        Ok(position_of(&event))
    }

    async fn expect_paused(
        events: &mut mpsc::UnboundedReceiver<PlayerEvent>,
        track: &PlayingItem,
    ) -> Result<Duration> {
        let event = next_track_state(events).await?;
        assert!(
            matches!(&event, PlayerEvent::Paused { item, .. } if item == track),
            "{event:?}"
        );
        Ok(position_of(&event))
    }

    // Waits for the next progress report of a loaded track and returns its position.
    async fn next_progress(events: &mut mpsc::UnboundedReceiver<PlayerEvent>) -> Result<Duration> {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match events.recv().await {
                    Some(PlayerEvent::Progress {
                        position,
                        ready: true,
                        ..
                    }) => return Ok(position),
                    Some(_) => {}
                    None => bail!("player events closed"),
                }
            }
        })
        .await?
    }

    // An engine whose tracks are five seconds long and load at once.
    fn long_track_engine(
        file_uuids: &[Uuid],
    ) -> (
        PlayerInternal,
        mpsc::UnboundedSender<PlayerCommand>,
        mpsc::UnboundedReceiver<PlayerEvent>,
        PathBuf,
    ) {
        let (resolver, wav_path) = delayed_resolver_fixture_with_samples(
            file_uuids,
            Arc::new(Mutex::new(Vec::new())),
            Arc::new(Notify::new()),
            Arc::new(Notify::new()),
            &[],
            LONG_TRACK_SAMPLES,
        );
        let (internal, commands_sender, events) = library_engine(resolver);
        (internal, commands_sender, events, wav_path)
    }

    // Queues one track and plays it for a while, so that it stands clearly past its start.
    async fn start_playing(
        commands_sender: &mpsc::UnboundedSender<PlayerCommand>,
        events: &mut mpsc::UnboundedReceiver<PlayerEvent>,
        file_uuid: Uuid,
    ) -> Result<PlayingItem> {
        let library_id = Uuid::new_v4();
        let track = library_item(library_id, file_uuid);
        commands_sender.send(add_library_tracks(library_id, &[file_uuid]))?;
        commands_sender.send(PlayerCommand::Switch(0))?;
        expect_playing(events, &track).await?;
        tokio::time::sleep(Duration::from_millis(400)).await;
        while events.try_recv().is_ok() {}
        Ok(track)
    }

    async fn assert_nothing_plays(events: &mut mpsc::UnboundedReceiver<PlayerEvent>) {
        tokio::time::sleep(Duration::from_millis(150)).await;
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, PlayerEvent::Playing { .. }), "{event:?}");
        }
    }

    #[tokio::test]
    async fn pause_during_load_brings_the_track_up_paused() -> Result<()> {
        let file_uuids = [Uuid::new_v4()];
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) = delayed_resolver_fixture(
            &file_uuids,
            Arc::new(Mutex::new(Vec::new())),
            started.clone(),
            release.clone(),
            &[1],
        );
        let (mut internal, commands_sender, mut events) = library_engine(resolver);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let expected = library_item(library_id, file_uuids[0]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::Pause)?;
                    release.notify_one();

                    let event = next_track_state(&mut events).await?;
                    assert!(
                        matches!(&event, PlayerEvent::Loaded { item, .. } if *item == expected),
                        "{event:?}"
                    );
                    assert_nothing_plays(&mut events).await;

                    // Play loads the track again to open its output, and the resolver holds it.
                    commands_sender.send(PlayerCommand::Play)?;
                    release.notify_one();
                    let event = next_track_state(&mut events).await?;
                    assert!(
                        matches!(&event, PlayerEvent::Playing { item, .. } if *item == expected),
                        "{event:?}"
                    );

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
    async fn pause_during_load_also_pauses_the_queued_track() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4()];
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
        let (mut internal, commands_sender, mut events) = library_engine(resolver);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let expected = library_item(library_id, file_uuids[1]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::Switch(1))?;
                    commands_sender.send(PlayerCommand::Pause)?;
                    release.notify_one();

                    let event = next_track_state(&mut events).await?;
                    assert!(
                        matches!(&event, PlayerEvent::Loaded { item, .. } if *item == expected),
                        "{event:?}"
                    );
                    assert_nothing_plays(&mut events).await;
                    assert_eq!(&*ids.lock().unwrap(), &[1, 2]);

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
    async fn pause_while_switching_tracks_brings_the_new_track_up_paused() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4()];
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) = delayed_resolver_fixture_with_samples(
            &file_uuids,
            Arc::new(Mutex::new(Vec::new())),
            started.clone(),
            release.clone(),
            &[2],
            LONG_TRACK_SAMPLES,
        );
        let (mut internal, commands_sender, mut events) = library_engine(resolver);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let current = library_item(library_id, file_uuids[0]);
                    let expected = library_item(library_id, file_uuids[1]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    expect_playing(&mut events, &current).await?;

                    commands_sender.send(PlayerCommand::Switch(1))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::Pause)?;
                    let event = next_track_state(&mut events).await?;
                    assert!(
                        matches!(&event, PlayerEvent::Paused { item, .. } if *item == current),
                        "{event:?}"
                    );
                    release.notify_one();

                    let event = next_track_state(&mut events).await?;
                    assert!(
                        matches!(&event, PlayerEvent::Loaded { item, .. } if *item == expected),
                        "{event:?}"
                    );
                    assert_nothing_plays(&mut events).await;

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
    async fn play_after_pause_during_load_plays_the_loading_track() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4()];
        let ids = Arc::new(Mutex::new(Vec::new()));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) = delayed_resolver_fixture(
            &file_uuids,
            ids.clone(),
            started.clone(),
            release.clone(),
            &[2],
        );
        let (mut internal, commands_sender, mut events) = library_engine(resolver);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let expected = library_item(library_id, file_uuids[1]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                    commands_sender.send(PlayerCommand::Switch(1))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::Pause)?;
                    commands_sender.send(PlayerCommand::Play)?;
                    release.notify_one();

                    let event = next_track_state(&mut events).await?;
                    assert!(
                        matches!(&event, PlayerEvent::Playing { item, .. } if *item == expected),
                        "{event:?}"
                    );
                    assert_eq!(&*ids.lock().unwrap(), &[2]);

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
    async fn play_during_load_keeps_the_requested_track() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4()];
        let ids = Arc::new(Mutex::new(Vec::new()));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) = delayed_resolver_fixture(
            &file_uuids,
            ids.clone(),
            started.clone(),
            release.clone(),
            &[2],
        );
        let (mut internal, commands_sender, mut events) = library_engine(resolver);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let expected = library_item(library_id, file_uuids[1]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                    commands_sender.send(PlayerCommand::Switch(1))?;
                    commands_sender.send(PlayerCommand::Play)?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    release.notify_one();

                    let event = next_track_state(&mut events).await?;
                    assert!(
                        matches!(&event, PlayerEvent::Playing { item, .. } if *item == expected),
                        "{event:?}"
                    );
                    assert_eq!(&*ids.lock().unwrap(), &[2]);

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
    async fn load_without_pause_still_plays() -> Result<()> {
        let file_uuids = [Uuid::new_v4()];
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) = delayed_resolver_fixture(
            &file_uuids,
            Arc::new(Mutex::new(Vec::new())),
            started.clone(),
            release.clone(),
            &[1],
        );
        let (mut internal, commands_sender, mut events) = library_engine(resolver);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let expected = library_item(library_id, file_uuids[0]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    release.notify_one();

                    let event = next_track_state(&mut events).await?;
                    assert!(
                        matches!(&event, PlayerEvent::Playing { item, .. } if *item == expected),
                        "{event:?}"
                    );

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
    async fn old_track_ending_during_a_load_keeps_the_chosen_track() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let ids = Arc::new(Mutex::new(Vec::new()));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) = delayed_resolver_fixture_with_samples(
            &file_uuids,
            ids.clone(),
            started.clone(),
            release.clone(),
            &[3],
            LONG_TRACK_SAMPLES,
        );
        let (mut internal, commands_sender, mut events) = library_engine(resolver);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let current = library_item(library_id, file_uuids[0]);
                    let expected = library_item(library_id, file_uuids[2]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    expect_playing(&mut events, &current).await?;

                    // The old track plays on while the chosen one loads, and runs out first.
                    commands_sender.send(PlayerCommand::Switch(2))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::Seek(4.9))?;
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    while events.try_recv().is_ok() {}
                    release.notify_one();

                    let event = next_track_state(&mut events).await?;
                    assert!(
                        matches!(&event, PlayerEvent::Playing { item, .. } if *item == expected),
                        "{event:?}"
                    );
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
    async fn track_end_advances_once_while_the_next_track_loads() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4()];
        let ids = Arc::new(Mutex::new(Vec::new()));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) = delayed_resolver_fixture(
            &file_uuids,
            ids.clone(),
            started.clone(),
            release.clone(),
            &[2],
        );
        let (mut internal, commands_sender, mut events) = library_engine(resolver);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let first = library_item(library_id, file_uuids[0]);
                    let second = library_item(library_id, file_uuids[1]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    release.notify_one();

                    let mut ends = 0;
                    tokio::time::timeout(Duration::from_secs(2), async {
                        loop {
                            match events.recv().await {
                                Some(PlayerEvent::EndOfTrack { item, .. }) if item == first => {
                                    ends += 1
                                }
                                Some(PlayerEvent::Playing { item, .. }) if item == second => break,
                                Some(_) => {}
                                None => bail!("player events closed"),
                            }
                        }
                        Ok(())
                    })
                    .await??;
                    assert_eq!(ends, 1);
                    assert_eq!(&*ids.lock().unwrap(), &[1, 2]);

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
    async fn restore_position_is_not_applied_to_a_newer_track() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4()];
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) = delayed_resolver_fixture_with_samples(
            &file_uuids,
            Arc::new(Mutex::new(Vec::new())),
            started.clone(),
            release.clone(),
            &[1],
            LONG_TRACK_SAMPLES,
        );
        let (mut internal, commands_sender, mut events) = library_engine(resolver);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let expected = library_item(library_id, file_uuids[1]);
                    commands_sender.send(PlayerCommand::Restore {
                        tracks: library_tracks(library_id, &file_uuids),
                        index: Some(0),
                        position: Duration::from_secs(1),
                        mode: PlaybackMode::Sequential,
                    })?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::Load { index: 1 })?;
                    release.notify_one();

                    let event = next_track_state(&mut events).await?;
                    assert!(
                        matches!(&event, PlayerEvent::Loaded { item, position, .. }
                            if *item == expected && position.is_zero()),
                        "{event:?}"
                    );

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
    async fn restore_during_load_keeps_its_position_after_the_hand_off() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4()];
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) = delayed_resolver_fixture_with_samples(
            &file_uuids,
            Arc::new(Mutex::new(Vec::new())),
            started.clone(),
            release.clone(),
            &[1],
            LONG_TRACK_SAMPLES,
        );
        let (mut internal, commands_sender, mut events) = library_engine(resolver);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let expected = library_item(library_id, file_uuids[1]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids[..1]))?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::Restore {
                        tracks: library_tracks(library_id, &file_uuids[1..]),
                        index: Some(1),
                        position: Duration::from_secs(1),
                        mode: PlaybackMode::Sequential,
                    })?;
                    release.notify_one();

                    let event = next_track_state(&mut events).await?;
                    assert!(
                        matches!(&event, PlayerEvent::Loaded { item, .. } if *item == expected),
                        "{event:?}"
                    );
                    assert_about(position_of(&event), Duration::from_secs(1));

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
    async fn interruption_that_ends_with_resume_plays_on_from_the_same_place() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let track = start_playing(&commands_sender, &mut events, file_uuid).await?;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::Interrupted))?;
                    let paused_at = expect_paused(&mut events, &track).await?;
                    assert!(paused_at >= Duration::from_millis(300), "{paused_at:?}");

                    commands_sender.send(PlayerCommand::Output(
                        OutputEvent::InterruptionEnded { resume: true },
                    ))?;
                    assert_about(expect_playing(&mut events, &track).await?, paused_at);
                    assert_eq!(opens.load(Ordering::SeqCst), 2);

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
    async fn interruption_that_ends_without_resume_stays_paused_until_play() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let track = start_playing(&commands_sender, &mut events, file_uuid).await?;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::Interrupted))?;
                    let paused_at = expect_paused(&mut events, &track).await?;
                    commands_sender.send(PlayerCommand::Output(
                        OutputEvent::InterruptionEnded { resume: false },
                    ))?;
                    assert_nothing_plays(&mut events).await;
                    assert_eq!(opens.load(Ordering::SeqCst), 1);

                    commands_sender.send(PlayerCommand::Play)?;
                    assert_about(expect_playing(&mut events, &track).await?, paused_at);
                    assert_eq!(opens.load(Ordering::SeqCst), 2);

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
    async fn paused_track_reopens_on_play_after_an_interruption() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        let resumes = internal.output_resumes.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let track = start_playing(&commands_sender, &mut events, file_uuid).await?;
                    commands_sender.send(PlayerCommand::Pause)?;
                    let paused_at = expect_paused(&mut events, &track).await?;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::Interrupted))?;
                    commands_sender.send(PlayerCommand::Output(
                        OutputEvent::InterruptionEnded { resume: true },
                    ))?;
                    assert_nothing_plays(&mut events).await;
                    assert_eq!(opens.load(Ordering::SeqCst), 1);

                    // The stale output is not started again: a new one opens.
                    commands_sender.send(PlayerCommand::Play)?;
                    assert_about(expect_playing(&mut events, &track).await?, paused_at);
                    assert_eq!(opens.load(Ordering::SeqCst), 2);
                    assert_eq!(resumes.load(Ordering::SeqCst), 0);

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
    async fn device_lost_pauses_and_cancels_the_resume_after_an_interruption() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let track = start_playing(&commands_sender, &mut events, file_uuid).await?;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::DeviceLost))?;
                    let paused_at = expect_paused(&mut events, &track).await?;
                    commands_sender.send(PlayerCommand::Play)?;
                    assert_about(expect_playing(&mut events, &track).await?, paused_at);

                    commands_sender.send(PlayerCommand::Output(OutputEvent::Interrupted))?;
                    expect_paused(&mut events, &track).await?;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::DeviceLost))?;
                    commands_sender.send(PlayerCommand::Output(
                        OutputEvent::InterruptionEnded { resume: true },
                    ))?;
                    assert_nothing_plays(&mut events).await;
                    assert_eq!(opens.load(Ordering::SeqCst), 2);

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
    async fn device_lost_during_a_load_brings_the_track_up_paused() -> Result<()> {
        let file_uuids = [Uuid::new_v4()];
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) = delayed_resolver_fixture_with_samples(
            &file_uuids,
            Arc::new(Mutex::new(Vec::new())),
            started.clone(),
            release.clone(),
            &[1],
            LONG_TRACK_SAMPLES,
        );
        let (mut internal, commands_sender, mut events) = library_engine(resolver);
        let opens = internal.output_opens.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let track = library_item(library_id, file_uuids[0]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::DeviceLost))?;
                    release.notify_one();

                    let event = next_track_state(&mut events).await?;
                    assert!(
                        matches!(&event, PlayerEvent::Loaded { item, .. } if *item == track),
                        "{event:?}"
                    );
                    assert_nothing_plays(&mut events).await;

                    // Play loads the track again, which the resolver holds, and opens the only
                    // output.
                    assert_eq!(opens.load(Ordering::SeqCst), 0);
                    commands_sender.send(PlayerCommand::Play)?;
                    release.notify_one();
                    expect_playing(&mut events, &track).await?;
                    assert_eq!(opens.load(Ordering::SeqCst), 1);

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
    async fn device_added_while_playing_moves_playback_in_place() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let track = start_playing(&commands_sender, &mut events, file_uuid).await?;
                    let playing_at = next_progress(&mut events).await?;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::DeviceAdded))?;
                    assert_about(expect_playing(&mut events, &track).await?, playing_at);
                    assert_eq!(opens.load(Ordering::SeqCst), 2);

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
    async fn device_added_while_paused_waits_for_play() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let track = start_playing(&commands_sender, &mut events, file_uuid).await?;
                    commands_sender.send(PlayerCommand::Pause)?;
                    let paused_at = expect_paused(&mut events, &track).await?;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::DeviceAdded))?;
                    assert_nothing_plays(&mut events).await;
                    assert_eq!(opens.load(Ordering::SeqCst), 1);

                    commands_sender.send(PlayerCommand::Play)?;
                    assert_about(expect_playing(&mut events, &track).await?, paused_at);
                    assert_eq!(opens.load(Ordering::SeqCst), 2);

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
    async fn failed_open_keeps_the_track_paused_in_place() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        let failing_opens = internal.failing_opens.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let track = start_playing(&commands_sender, &mut events, file_uuid).await?;
                    let playing_at = next_progress(&mut events).await?;
                    failing_opens.store(1, Ordering::SeqCst);
                    commands_sender.send(PlayerCommand::Output(OutputEvent::DeviceAdded))?;
                    let paused_at = expect_paused(&mut events, &track).await?;
                    assert_about(paused_at, playing_at);

                    // The player still answers, and reports no progress without an output.
                    commands_sender.send(PlayerCommand::SetVolume(0.5))?;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    let mut volume_updated = false;
                    while let Ok(event) = events.try_recv() {
                        assert!(!matches!(event, PlayerEvent::Progress { .. }), "{event:?}");
                        volume_updated |= matches!(event, PlayerEvent::VolumeUpdate(_));
                    }
                    assert!(volume_updated);

                    commands_sender.send(PlayerCommand::Play)?;
                    assert_about(expect_playing(&mut events, &track).await?, paused_at);
                    assert_eq!(opens.load(Ordering::SeqCst), 3);

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
    async fn stream_error_while_playing_goes_on_in_place() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        let stream_errors = internal.stream_error_sender.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let track = start_playing(&commands_sender, &mut events, file_uuid).await?;
                    let playing_at = next_progress(&mut events).await?;
                    stream_errors.send("the device is no longer available".into())?;
                    assert_about(expect_playing(&mut events, &track).await?, playing_at);
                    assert_eq!(opens.load(Ordering::SeqCst), 2);

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
    async fn seek_while_the_output_is_stale_resumes_there() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let track = start_playing(&commands_sender, &mut events, file_uuid).await?;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::Interrupted))?;
                    expect_paused(&mut events, &track).await?;
                    commands_sender.send(PlayerCommand::Seek(2.0))?;
                    assert_about(
                        expect_paused(&mut events, &track).await?,
                        Duration::from_secs(2),
                    );

                    commands_sender.send(PlayerCommand::Output(
                        OutputEvent::InterruptionEnded { resume: true },
                    ))?;
                    assert_about(
                        expect_playing(&mut events, &track).await?,
                        Duration::from_secs(2),
                    );
                    assert_eq!(opens.load(Ordering::SeqCst), 2);

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
    async fn load_during_an_interruption_opens_no_output_until_play() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let track = library_item(library_id, file_uuid);
                    commands_sender.send(PlayerCommand::Output(OutputEvent::Interrupted))?;
                    commands_sender.send(add_library_tracks(library_id, &[file_uuid]))?;
                    commands_sender.send(PlayerCommand::Load { index: 0 })?;
                    let event = next_track_state(&mut events).await?;
                    assert!(
                        matches!(&event, PlayerEvent::Loaded { item, .. } if *item == track),
                        "{event:?}"
                    );
                    assert_eq!(opens.load(Ordering::SeqCst), 0);

                    commands_sender.send(PlayerCommand::Output(
                        OutputEvent::InterruptionEnded { resume: false },
                    ))?;
                    assert_nothing_plays(&mut events).await;
                    commands_sender.send(PlayerCommand::Play)?;
                    expect_playing(&mut events, &track).await?;
                    assert_eq!(opens.load(Ordering::SeqCst), 1);

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
    async fn interruption_without_an_end_is_over_once_an_output_opens_to_play() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let track = start_playing(&commands_sender, &mut events, file_uuid).await?;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::Interrupted))?;
                    let paused_at = expect_paused(&mut events, &track).await?;

                    // The end of the interruption never comes.
                    commands_sender.send(PlayerCommand::Play)?;
                    assert_about(expect_playing(&mut events, &track).await?, paused_at);
                    assert_eq!(opens.load(Ordering::SeqCst), 2);

                    commands_sender.send(PlayerCommand::Pause)?;
                    expect_paused(&mut events, &track).await?;
                    commands_sender.send(PlayerCommand::Play)?;
                    expect_playing(&mut events, &track).await?;
                    assert_eq!(opens.load(Ordering::SeqCst), 2);

                    runner.abort();
                    let _ = std::fs::remove_file(wav_path);
                    Ok::<(), anyhow::Error>(())
                }
                .await
            })
            .await?;
        Ok(())
    }

    fn restore(library_id: Uuid, file_uuids: &[Uuid], index: usize, seconds: f64) -> PlayerCommand {
        PlayerCommand::Restore {
            tracks: library_tracks(library_id, file_uuids),
            index: Some(index),
            position: Duration::from_secs_f64(seconds),
            mode: PlaybackMode::Sequential,
        }
    }

    async fn expect_loaded(
        events: &mut mpsc::UnboundedReceiver<PlayerEvent>,
        track: &PlayingItem,
    ) -> Result<Duration> {
        let event = next_track_state(events).await?;
        assert!(
            matches!(&event, PlayerEvent::Loaded { item, .. } if item == track),
            "{event:?}"
        );
        Ok(position_of(&event))
    }

    #[tokio::test]
    async fn paused_load_opens_no_output_until_play() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let track = library_item(library_id, file_uuid);
                    commands_sender.send(restore(library_id, &[file_uuid], 0, 1.5))?;
                    assert_about(
                        expect_loaded(&mut events, &track).await?,
                        Duration::from_millis(1500),
                    );
                    assert_nothing_plays(&mut events).await;
                    assert_eq!(opens.load(Ordering::SeqCst), 0);

                    commands_sender.send(PlayerCommand::Play)?;
                    assert_about(
                        expect_playing(&mut events, &track).await?,
                        Duration::from_millis(1500),
                    );
                    assert_eq!(opens.load(Ordering::SeqCst), 1);

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
    async fn seek_before_the_first_play_is_where_playback_starts() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let track = library_item(library_id, file_uuid);
                    commands_sender.send(restore(library_id, &[file_uuid], 0, 1.0))?;
                    expect_loaded(&mut events, &track).await?;
                    commands_sender.send(PlayerCommand::Seek(3.0))?;
                    assert_about(
                        expect_paused(&mut events, &track).await?,
                        Duration::from_secs(3),
                    );
                    assert_eq!(opens.load(Ordering::SeqCst), 0);

                    commands_sender.send(PlayerCommand::Play)?;
                    assert_about(
                        expect_playing(&mut events, &track).await?,
                        Duration::from_secs(3),
                    );
                    assert_eq!(opens.load(Ordering::SeqCst), 1);

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
    async fn paused_track_reports_that_it_is_ready_where_it_stands() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let track = library_item(library_id, file_uuid);
                    commands_sender.send(restore(library_id, &[file_uuid], 0, 2.0))?;
                    expect_loaded(&mut events, &track).await?;
                    // Changing the mode reports progress, which must keep the controls usable.
                    commands_sender
                        .send(PlayerCommand::SetPlaybackMode(PlaybackMode::RepeatAll))?;
                    assert_about(next_progress(&mut events).await?, Duration::from_secs(2));

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
    async fn another_paused_load_opens_nothing_and_next_opens_one_output() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&file_uuids);
        let opens = internal.output_opens.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    commands_sender.send(restore(library_id, &file_uuids, 0, 1.0))?;
                    expect_loaded(&mut events, &library_item(library_id, file_uuids[0])).await?;
                    commands_sender.send(PlayerCommand::Load { index: 1 })?;
                    expect_loaded(&mut events, &library_item(library_id, file_uuids[1])).await?;
                    assert_eq!(opens.load(Ordering::SeqCst), 0);

                    commands_sender.send(PlayerCommand::Next)?;
                    expect_playing(&mut events, &library_item(library_id, file_uuids[2])).await?;
                    assert_eq!(opens.load(Ordering::SeqCst), 1);

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
    async fn previous_restarts_a_paused_track_that_stands_past_three_seconds() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4()];
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&file_uuids);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let track = library_item(library_id, file_uuids[1]);
                    commands_sender.send(PlayerCommand::SetAdaptiveSwitchingEnabled(true))?;
                    commands_sender.send(restore(library_id, &file_uuids, 1, 4.0))?;
                    expect_loaded(&mut events, &track).await?;
                    commands_sender.send(PlayerCommand::Previous)?;
                    assert_about(expect_playing(&mut events, &track).await?, Duration::ZERO);

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
    async fn volume_set_before_the_first_play_applies() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let script = async {
            let library_id = Uuid::new_v4();
            let track = library_item(library_id, file_uuid);
            commands_sender.send(add_library_tracks(library_id, &[file_uuid]))?;
            commands_sender.send(PlayerCommand::Load { index: 0 })?;
            expect_loaded(&mut events, &track).await?;
            commands_sender.send(PlayerCommand::SetVolume(0.25))?;
            commands_sender.send(PlayerCommand::Play)?;
            expect_playing(&mut events, &track).await?;
            Ok::<(), anyhow::Error>(())
        };
        // The test runs the player itself, so that it can look at the sink afterwards.
        tokio::select! {
            result = internal.run() => bail!("the player stopped: {result:?}"),
            result = script => result?,
        }
        assert_eq!(internal.sink.as_ref().map(Sink::volume), Some(0.25));
        let _ = std::fs::remove_file(wav_path);
        Ok(())
    }

    #[tokio::test]
    async fn pause_stops_the_output_and_play_starts_it_again() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        let suspends = internal.output_suspends.clone();
        let resumes = internal.output_resumes.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let track = start_playing(&commands_sender, &mut events, file_uuid).await?;
                    assert_eq!(suspends.load(Ordering::SeqCst), 0);
                    commands_sender.send(PlayerCommand::Pause)?;
                    let paused_at = expect_paused(&mut events, &track).await?;
                    assert_eq!(suspends.load(Ordering::SeqCst), 1);
                    assert_eq!(resumes.load(Ordering::SeqCst), 0);

                    // The same output plays on from where the track was paused.
                    commands_sender.send(PlayerCommand::Play)?;
                    expect_playing(&mut events, &track).await?;
                    assert_eq!(resumes.load(Ordering::SeqCst), 1);
                    assert_eq!(opens.load(Ordering::SeqCst), 1);
                    assert_about(next_progress(&mut events).await?, paused_at);
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    while events.try_recv().is_ok() {}
                    let later = next_progress(&mut events).await?;
                    assert!(later >= paused_at + Duration::from_millis(300), "{later:?}");

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
    async fn seek_while_paused_runs_the_output_only_for_the_seek() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        // The player runs on a thread of its own, so that a seek that waits on a stopped output
        // fails the test on a timeout instead of hanging it.
        let (setup_sender, setup) = channel();
        std::thread::spawn(move || {
            let (mut internal, commands_sender, events, wav_path) = long_track_engine(&[file_uuid]);
            let hooks = (
                internal.cancellation_token.clone(),
                internal.output_opens.clone(),
                internal.output_suspends.clone(),
                internal.output_resumes.clone(),
            );
            setup_sender
                .send((commands_sender, events, wav_path, hooks))
                .unwrap();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let _ = runtime.block_on(internal.run());
        });
        let (commands_sender, mut events, wav_path, (cancel, opens, suspends, resumes)) =
            setup.recv()?;

        let track = start_playing(&commands_sender, &mut events, file_uuid).await?;
        commands_sender.send(PlayerCommand::Pause)?;
        expect_paused(&mut events, &track).await?;
        commands_sender.send(PlayerCommand::Seek(3.0))?;
        assert_about(
            expect_paused(&mut events, &track).await?,
            Duration::from_secs(3),
        );
        assert_eq!(resumes.load(Ordering::SeqCst), 1);
        assert_eq!(suspends.load(Ordering::SeqCst), 2);
        assert_nothing_plays(&mut events).await;

        commands_sender.send(PlayerCommand::Play)?;
        expect_playing(&mut events, &track).await?;
        assert_about(next_progress(&mut events).await?, Duration::from_secs(3));
        assert_eq!(opens.load(Ordering::SeqCst), 1);

        cancel.cancel();
        let _ = std::fs::remove_file(wav_path);
        Ok(())
    }

    #[tokio::test]
    async fn failed_resume_opens_a_new_output_where_the_track_was_paused() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        let resumes = internal.output_resumes.clone();
        let failing_resumes = internal.failing_resumes.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let track = start_playing(&commands_sender, &mut events, file_uuid).await?;
                    commands_sender.send(PlayerCommand::Pause)?;
                    let paused_at = expect_paused(&mut events, &track).await?;
                    failing_resumes.store(1, Ordering::SeqCst);

                    commands_sender.send(PlayerCommand::Play)?;
                    assert_about(expect_playing(&mut events, &track).await?, paused_at);
                    assert_eq!(resumes.load(Ordering::SeqCst), 1);
                    assert_eq!(opens.load(Ordering::SeqCst), 2);

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
    async fn pause_and_play_during_a_switch_stop_and_start_the_old_output() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4()];
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) = delayed_resolver_fixture_with_samples(
            &file_uuids,
            Arc::new(Mutex::new(Vec::new())),
            started.clone(),
            release.clone(),
            &[2],
            LONG_TRACK_SAMPLES,
        );
        let (mut internal, commands_sender, mut events) = library_engine(resolver);
        let opens = internal.output_opens.clone();
        let suspends = internal.output_suspends.clone();
        let resumes = internal.output_resumes.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let current = library_item(library_id, file_uuids[0]);
                    let expected = library_item(library_id, file_uuids[1]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    expect_playing(&mut events, &current).await?;

                    // The old track keeps its output while the new one loads.
                    commands_sender.send(PlayerCommand::Switch(1))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::Pause)?;
                    expect_paused(&mut events, &current).await?;
                    assert_eq!(suspends.load(Ordering::SeqCst), 1);
                    commands_sender.send(PlayerCommand::Play)?;
                    expect_playing(&mut events, &current).await?;
                    assert_eq!(resumes.load(Ordering::SeqCst), 1);

                    release.notify_one();
                    expect_playing(&mut events, &expected).await?;
                    assert_eq!(opens.load(Ordering::SeqCst), 2);

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
    async fn failed_resume_during_a_switch_leaves_playing_to_the_new_track() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4()];
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) = delayed_resolver_fixture_with_samples(
            &file_uuids,
            Arc::new(Mutex::new(Vec::new())),
            started.clone(),
            release.clone(),
            &[2],
            LONG_TRACK_SAMPLES,
        );
        let (mut internal, commands_sender, mut events) = library_engine(resolver);
        let opens = internal.output_opens.clone();
        let failing_resumes = internal.failing_resumes.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let current = library_item(library_id, file_uuids[0]);
                    let expected = library_item(library_id, file_uuids[1]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    expect_playing(&mut events, &current).await?;

                    commands_sender.send(PlayerCommand::Switch(1))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::Pause)?;
                    expect_paused(&mut events, &current).await?;
                    failing_resumes.store(1, Ordering::SeqCst);
                    commands_sender.send(PlayerCommand::Play)?;
                    assert_nothing_plays(&mut events).await;

                    release.notify_one();
                    expect_playing(&mut events, &expected).await?;
                    assert_eq!(opens.load(Ordering::SeqCst), 2);

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
    async fn stop_stops_the_output() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let suspends = internal.output_suspends.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    start_playing(&commands_sender, &mut events, file_uuid).await?;
                    commands_sender.send(PlayerCommand::Stop)?;
                    tokio::time::timeout(Duration::from_secs(2), async {
                        while !matches!(events.recv().await, Some(PlayerEvent::Stopped) | None) {}
                    })
                    .await?;
                    assert_eq!(suspends.load(Ordering::SeqCst), 1);

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
