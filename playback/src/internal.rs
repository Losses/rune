use crate::stream_utils::ManagedStream;
use std::{
    collections::HashMap,
    fmt::Debug,
    future::Future,
    io::{BufReader, Cursor, Error, ErrorKind},
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use fsio::{FileIoError, FileStream, FsIo};
use log::{debug, error, info, warn};
use rodio::{PlayError, Sink, Source, source::SeekError};
use tokio::{
    sync::{mpsc, oneshot},
    time::{Instant, interval, sleep_until},
};
use tokio_util::sync::CancellationToken;

use crate::buffered::{RuneBuffered, rune_buffered};
use crate::decoder::RuneDecoder;
use crate::output_stream::{RuneOutputStream, RuneOutputStreamHandle};
use crate::pcm_tap::PcmTap;
use crate::player::{
    AudioPlaybackNote, AudioSourceMode, LibraryTrackKey, PlaybackSource, PlaybackSourceResolver,
    PlayerStatus, PlayingItem, PlaylistEdit, PlaylistPlacement, RemoteAudioError, SubsonicItem,
    UnavailableReason,
};
use crate::realtime_fft::RealTimeFFT;
use crate::shared_source::SharedSource;
use crate::strategies::{
    AddMode, PlaybackStrategy, RepeatAllStrategy, RepeatOneStrategy, SequentialStrategy,
    ShuffleStrategy, UpdateReason,
};
use crate::stream_utils::create_stream_with_authorizer;

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

async fn open_local_source(fsio: Arc<FsIo>, path: PathBuf, remote: bool) -> Result<AnySource> {
    let open_path = path.clone();
    let stream = tokio::task::spawn_blocking(move || fsio.open(&open_path, "r"))
        .await
        .context("audio file open task failed")?
        .map_err(|error| {
            if remote {
                return match error {
                    FileIoError::PermissionDenied(_) => anyhow!(RemoteAudioError::AccessRevoked),
                    error @ (FileIoError::Remote(_) | FileIoError::PathNotFound(_)) => {
                        anyhow!(RemoteAudioError::unreachable(error))
                    }
                    other => anyhow::Error::new(other),
                };
            }
            anyhow::Error::new(error)
        })?;
    let (stream, failure) = if remote {
        let failure = Arc::new(Mutex::new(None));
        (
            Box::new(FailureTrackingFile {
                inner: stream,
                failure: failure.clone(),
            }) as Box<dyn FileStream>,
            Some(failure),
        )
    } else {
        (stream, None)
    };
    let extension = path.extension().and_then(|value| value.to_str());
    let decoder = match probe_decoder(BufReader::new(stream), extension.map(str::to_owned), None)
        .await
    {
        Ok(decoder) => decoder,
        Err(error) => {
            if let Some(reason) = failure
                .as_ref()
                .and_then(|failure| failure.lock().ok().and_then(|value| *value))
            {
                return Err(match reason {
                    UnavailableReason::AccessRevoked => anyhow!(RemoteAudioError::AccessRevoked),
                    UnavailableReason::SourceUnreachable => {
                        anyhow!(RemoteAudioError::unreachable(error))
                    }
                    UnavailableReason::LibraryUnavailable => {
                        anyhow!(RemoteAudioError::LibraryUnavailable)
                    }
                    UnavailableReason::Unreadable => error,
                    UnavailableReason::CredentialsRejected => {
                        anyhow!(RemoteAudioError::CredentialsRejected)
                    }
                    UnavailableReason::TrackGone => anyhow!(RemoteAudioError::TrackGone),
                });
            }
            return Err(error);
        }
    };
    Ok(AnySource::Local(rune_buffered(decoder), failure))
}

struct FailureTrackingFile {
    inner: Box<dyn FileStream>,
    failure: Arc<Mutex<Option<UnavailableReason>>>,
}

impl FailureTrackingFile {
    fn record(&self, error: &std::io::Error) {
        let reason = match error.kind() {
            ErrorKind::PermissionDenied => UnavailableReason::AccessRevoked,
            ErrorKind::NotFound => UnavailableReason::SourceUnreachable,
            _ => UnavailableReason::SourceUnreachable,
        };
        if let Ok(mut failure) = self.failure.lock() {
            *failure = Some(reason);
        }
    }
}

impl std::io::Read for FailureTrackingFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let result = self.inner.read(buf);
        if let Err(error) = &result {
            self.record(error);
        }
        result
    }
}

impl std::io::Seek for FailureTrackingFile {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        let result = self.inner.seek(pos);
        if let Err(error) = &result {
            self.record(error);
        }
        result
    }
}

impl std::io::Write for FailureTrackingFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl FileStream for FailureTrackingFile {
    fn sync_all(&mut self) -> Result<(), FileIoError> {
        self.inner.sync_all()
    }
}

pub enum AnySource {
    Local(
        RuneBuffered<RuneDecoder<BufReader<Box<dyn FileStream>>>>,
        Option<Arc<Mutex<Option<UnavailableReason>>>>,
    ),
    Online(
        RuneBuffered<RuneDecoder<ManagedStream>>,
        Arc<Mutex<Option<UnavailableReason>>>,
    ),
    Buffered(
        RuneBuffered<RuneDecoder<Cursor<Vec<u8>>>>,
        AudioPlaybackNote,
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

async fn open_subsonic_stream(
    resolver: &Arc<dyn PlaybackSourceResolver>,
    item: crate::player::SubsonicItem,
) -> Result<(AnySource, PathBuf)> {
    let remote = resolver.resolve_subsonic(item).await?;
    let size = remote.size;
    let guard = remote.cancellation.clone().drop_guard();
    let failed = remote.failure_reason;
    match remote.request.prepare().await? {
        AudioSourceMode::Buffered { bytes, note } => {
            let size = u64::try_from(bytes.len()).ok();
            let decoder = probe_decoder(Cursor::new(bytes), Some(remote.extension), size).await?;
            guard.disarm();
            return Ok((
                AnySource::Buffered(rune_buffered(decoder), note),
                PathBuf::new(),
            ));
        }
        AudioSourceMode::Ranged => {}
    }
    let reader = match create_stream_with_authorizer(remote.request, &remote.url).await {
        Ok(reader) => reader,
        Err(error) => {
            let reason = failed.lock().ok().and_then(|reason| *reason);
            return Err(match reason {
                Some(UnavailableReason::AccessRevoked) => {
                    anyhow!(RemoteAudioError::AccessRevoked)
                }
                Some(UnavailableReason::CredentialsRejected) => {
                    anyhow!(RemoteAudioError::CredentialsRejected)
                }
                Some(UnavailableReason::Unreadable | UnavailableReason::TrackGone) => {
                    anyhow!(RemoteAudioError::TrackGone)
                }
                _ => error,
            });
        }
    };
    let reader = ManagedStream::new_with_failure(reader, remote.cancellation, failed.clone());
    let decoder =
        probe_decoder(reader, Some(remote.extension), (size != 0).then_some(size)).await?;
    guard.disarm();
    Ok((
        AnySource::Online(rune_buffered(decoder), failed),
        PathBuf::new(),
    ))
}

impl Debug for AnySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnySource::Local(_, _) => f.debug_struct("AnySource::Local").finish(),
            AnySource::Online(_, _) => f.debug_struct("AnySource::Online").finish(),
            AnySource::Buffered(_, note) => {
                f.debug_tuple("AnySource::Buffered").field(note).finish()
            }
        }
    }
}

impl AnySource {
    fn current_samples(&self) -> Option<Vec<i16>> {
        match self {
            AnySource::Local(s, _) => s.current_samples(),
            AnySource::Online(s, _) => s.current_samples(),
            AnySource::Buffered(s, _) => s.current_samples(),
        }
    }
}

impl Iterator for AnySource {
    type Item = i16;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            AnySource::Local(s, _) => s.next(),
            AnySource::Online(s, _) => s.next(),
            AnySource::Buffered(s, _) => s.next(),
        }
    }
}

impl Source for AnySource {
    fn current_frame_len(&self) -> Option<usize> {
        match self {
            AnySource::Local(s, _) => s.current_frame_len(),
            AnySource::Online(s, _) => s.current_frame_len(),
            AnySource::Buffered(s, _) => s.current_frame_len(),
        }
    }

    fn channels(&self) -> u16 {
        match self {
            AnySource::Local(s, _) => s.channels(),
            AnySource::Online(s, _) => s.channels(),
            AnySource::Buffered(s, _) => s.channels(),
        }
    }

    fn sample_rate(&self) -> u32 {
        match self {
            AnySource::Local(s, _) => s.sample_rate(),
            AnySource::Online(s, _) => s.sample_rate(),
            AnySource::Buffered(s, _) => s.sample_rate(),
        }
    }

    fn total_duration(&self) -> Option<Duration> {
        match self {
            AnySource::Local(s, _) => s.total_duration(),
            AnySource::Online(s, _) => s.total_duration(),
            AnySource::Buffered(s, _) => s.total_duration(),
        }
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        match self {
            AnySource::Local(s, _) => s.try_seek(pos),
            AnySource::Online(s, _) => s.try_seek(pos),
            AnySource::Buffered(s, _) => s.try_seek(pos),
        }
    }
}

impl AnySource {
    fn stream_failure(&self) -> Option<Arc<Mutex<Option<UnavailableReason>>>> {
        match self {
            AnySource::Local(_, failed) => failed.clone(),
            AnySource::Online(_, failed) => Some(failed.clone()),
            AnySource::Buffered(_, _) => None,
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
        mapped: bool,
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
    EditPlaylist(PlaylistEdit),
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
    SetMixWithOthers(bool),
    StatusAfterPending(oneshot::Sender<PlayerStatus>),
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

#[derive(Debug)]
pub enum PlayerEvent {
    StatusAfterPending(oneshot::Sender<PlayerStatus>),
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

/// A track that finished loading, as the player keeps it once it is current.
struct LoadedTrack {
    item: PlayingItem,
    // The index the load asked for, and whether the playback mode maps it to a playlist position.
    // Next and Previous go on from it, and a reopen asks for it again.
    index: usize,
    mapped: bool,
    // Where the track stands in the playlist, which the events report.
    playlist_index: usize,
    path: PathBuf,
}

/// The place a seek asks for, in seconds. A place before the start is the start, and a value
/// that names no place, such as NaN or infinity, is none.
fn seek_position(seconds: f64) -> Option<Duration> {
    if seconds.is_nan() {
        return None;
    }
    Duration::try_from_secs_f64(seconds.max(0.0)).ok()
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
    // Whether `current_track_index` is an index the playback mode maps to a playlist position, as
    // Next, Previous and Load ask for, or a playlist position itself, as Switch and Restore ask
    // for.
    current_track_mapped: bool,
    current_track_path: Option<PathBuf>,
    active_stream_failure: Option<Arc<Mutex<Option<UnavailableReason>>>>,
    sink: Option<Sink>,
    _stream: Option<RuneOutputStream>,
    state: InternalPlaybackState,
    pending_load: Option<PendingLoad>,
    // The playlist position of the track that is loading.
    loading_index: Option<usize>,
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
    adaptive_switching: bool,
    // iOS: other apps keep playing while this one does. The audio session reads it; elsewhere it
    // is only kept.
    mix_with_others: bool,
    // Where the current track starts once its output opens. Set when a load starts and taken
    // when the output opens. It stays while the track has no output that plays, so that the next
    // play opens one there: the track came up paused, no output could be opened, or a stale
    // output cannot seek.
    restore_position: Option<Duration>,
    // Where the sink goes once its stopped output runs again. A stopped output pulls no samples,
    // so the sink would wait for a seek forever, and the next play seeks once the output runs.
    pending_seek: Option<Duration>,
    // The current track has no output that can play: it came up paused and none was opened, none
    // could be opened, its stream failed or its device went away. The next play opens one where
    // the track stands.
    output_stale: bool,
    // An interruption paused playback, so playback goes on when the interruption ends. A play,
    // or a track that starts playing, settles it first.
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
            current_track_mapped: false,
            current_track_path: None,
            active_stream_failure: None,
            sink: None,
            _stream: None,
            realtime_fft: Arc::new(Mutex::new(RealTimeFFT::new(512))),
            state: InternalPlaybackState::Stopped,
            pending_load: None,
            loading_index: None,
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
            adaptive_switching: false,
            mix_with_others: false,
            restore_position: None,
            pending_seek: None,
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
                                self.clear_unavailable(Some(self.playlist[index].item.clone()))?;
                            }
                            self.load_at(Some(index), play, false, Some(position))?;
                        },
                        PlayerCommand::LoadComplete { generation, result, item, original_item, index, mapped, playlist_index, path } => {
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
                                    let track = LoadedTrack { item, index, mapped, playlist_index, path };
                                    if play {
                                        self.setup_sink(source, track, start)?;
                                    } else {
                                        self.loaded_paused(track, start.unwrap_or_default())?;
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
                        PlayerCommand::Play => {
                            // A play asked for during an interruption settles it: a late end of
                            // the interruption must not start playback again on its own.
                            self.resume_after_interruption = false;
                            self.play()?;
                        },
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
                        PlayerCommand::EditPlaylist(edit) => {
                            let add_mode = match edit.placement {
                                PlaylistPlacement::Replace | PlaylistPlacement::AppendToEnd => {
                                    AddMode::AppendToEnd
                                }
                                PlaylistPlacement::PlayNext => AddMode::PlayNext,
                            };
                            let insertion = match edit.placement {
                                PlaylistPlacement::Replace => {
                                    self.clear_playlist()?;
                                    0
                                }
                                PlaylistPlacement::AppendToEnd => self.playlist.len(),
                                PlaylistPlacement::PlayNext => self.current_queue_edit_index()
                                    .map_or(self.playlist.len(), |index| index + 1),
                            };
                            let added = !edit.tracks.is_empty();
                            if added {
                                self.add_to_playlist(edit.tracks, add_mode);
                            }
                            if let Some(mode) = edit.playback_mode {
                                self.set_playback_mode(mode)?;
                            }
                            if let Some(load) = edit.load {
                                self.load_at(
                                    Some(insertion + load.start_index),
                                    load.play,
                                    false,
                                    Some(load.position),
                                )?;
                            } else if added
                                && edit.placement == PlaylistPlacement::PlayNext
                                && self.current_queue_edit_index().is_none()
                            {
                                self.load_at(Some(insertion), true, false, Some(Duration::ZERO))?;
                            }
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
                        PlayerCommand::SetMixWithOthers(enabled) => self.set_mix_with_others(enabled)?,
                        PlayerCommand::StatusAfterPending(response) => {
                            if self.debounce_timer.take().is_some() {
                                self.send_playlist_updated()?;
                            }
                            self.event_sender
                                .send(PlayerEvent::StatusAfterPending(response))
                                .context("Failed to send player status barrier")?;
                        },
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
                    self.stream_failed(&error_message)?;
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
            self.loading_index = Some(mapped_index);

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
                    let mut remote_storage = false;
                    let library_track_key =
                        if let (PlayingItem::LibraryTrack { library_id, track }, Some(resolver)) =
                            (&item.item, resolver.as_ref())
                        {
                            Some(
                                resolver
                                    .resolve_track(library_id.to_string(), track.clone())
                                    .await?,
                            )
                        } else {
                            None
                        };
                    if matches!(library_track_key, Some(LibraryTrackKey::Rune(_)))
                        && let (PlayingItem::LibraryTrack { library_id, .. }, Some(root)) =
                            (&item.item, resolver.as_ref())
                        && let Some(library) = root.for_library(library_id.to_string()).await?
                    {
                        remote_storage = library.fsio.name() == "OpenDAL";
                        fsio = library.fsio;
                        resolver = Some(library.resolver);
                    }
                    let local_file_id =
                        if let (Some(LibraryTrackKey::Rune(file_uuid)), Some(resolver)) =
                            (&library_track_key, resolver.as_ref())
                        {
                            debug!("Resolving source identity for track load {index}");
                            Some(
                                resolver
                                    .resolve_id(*file_uuid)
                                    .await?
                                    .context("library track no longer exists")?,
                            )
                        } else {
                            None
                        };
                    let resolved_item = item.item.clone();
                    match &resolved_item {
                        PlayingItem::IndependentFile(_) => {
                            let path = match &item.source {
                                PlaybackSource::Local(path) => path.clone(),
                                PlaybackSource::LibraryStream => {
                                    bail!("library stream resolver unavailable")
                                }
                            };
                            let source = open_local_source(fsio, path.clone(), remote_storage)
                                .await
                                .with_context(|| format!("Failed to open file: {path:?}"))?;
                            Ok((source, path, resolved_item.clone()))
                        }
                        PlayingItem::LibraryTrack { .. }
                            if matches!(library_track_key, Some(LibraryTrackKey::Rune(_))) =>
                        {
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
                            let source = open_local_source(fsio, path.clone(), remote_storage)
                                .await
                                .with_context(|| format!("Failed to open file: {path:?}"))?;
                            Ok((source, path, resolved_item.clone()))
                        }
                        PlayingItem::LibraryTrack { library_id, .. }
                            if matches!(library_track_key, Some(LibraryTrackKey::Subsonic(_))) =>
                        {
                            let resolver = resolver
                                .as_ref()
                                .context("Subsonic playback resolver unavailable")?;
                            let item = SubsonicItem {
                                library_id: *library_id,
                                track_id: match library_track_key.as_ref() {
                                    Some(LibraryTrackKey::Subsonic(track)) => track.clone(),
                                    _ => bail!("Subsonic track key unavailable"),
                                },
                            };
                            let (source, path) = open_subsonic_stream(resolver, item).await?;
                            Ok((source, path, resolved_item.clone()))
                        }
                        PlayingItem::LibraryTrack { .. } => {
                            bail!("library track resolver unavailable")
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
                    mapped,
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
        // Whatever asked to play, the session is set up and active again before a stream starts:
        // an interruption or a reset of the media services leaves it inactive. A session that
        // cannot be activated, as during a call, fails the open, and the track waits paused for
        // the next play.
        #[cfg(target_os = "ios")]
        if !crate::ios_session::activate(self.mix_with_others) {
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
    // output; a track that comes up paused waits without one, as does one whose output cannot
    // open.
    fn setup_sink(
        &mut self,
        source: AnySource,
        track: LoadedTrack,
        start: Option<Duration>,
    ) -> Result<()> {
        let output = self.open_output().and_then(|(stream, stream_handle)| {
            let sink = try_new_sink(&stream_handle).context("Failed to create sink")?;
            Ok((stream, sink))
        });
        let (stream, sink) = match output {
            Ok(output) => output,
            Err(e) => return self.output_failed(e, track, start.unwrap_or_default()),
        };
        self.output_stale = false;
        // The new output is the end of any interruption, so a late end must not start playback
        // again on its own.
        self.resume_after_interruption = false;
        self.pending_seek = None;

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
        let LoadedTrack {
            item,
            index,
            mapped,
            playlist_index,
            path,
        } = track;
        self.current_track_index = Some(index);
        self.current_track_mapped = mapped;
        self.current_item = Some(item.clone());
        self.current_track_path = Some(path.clone());
        info!("Track loaded: {path:?}");

        self.event_sender
            .send(PlayerEvent::Playing {
                item,
                index: playlist_index,
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
    // elsewhere. The first play loads the track again and opens an output there. `Loaded` also
    // tells the UI that the track is ready to play.
    fn loaded_paused(&mut self, track: LoadedTrack, position: Duration) -> Result<()> {
        info!("Track loaded paused: {:?}", track.path);
        let LoadedTrack {
            item,
            index,
            mapped,
            playlist_index,
            path,
        } = track;
        self.keep_without_output(item.clone(), index, mapped, path.clone(), position);
        self.event_sender
            .send(PlayerEvent::Loaded {
                item,
                index: playlist_index,
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
        mapped: bool,
        path: PathBuf,
        position: Duration,
    ) {
        self.sink = None;
        self._stream = None;
        self.active_stream_failure = None;
        self.output_stale = true;
        self.restore_position = Some(position);
        self.pending_seek = None;
        self.current_track_index = Some(index);
        self.current_track_mapped = mapped;
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

        let Some(sink) = &self.sink else {
            if !loading {
                info!("Loading the first track");
                self.load(Some(0), true, true)?;
            }
            return Ok(());
        };
        // A seek that came while the output was stopped goes through now that the output runs. A
        // stale output never runs again, and the track that replaces it starts at the place.
        if !self.output_stale
            && let Some(position) = self.pending_seek.take()
            && let Err(e) = sink.try_seek(position)
        {
            error!("Failed to seek to {position:?}: {e:#?}");
        }
        sink.play();
        info!("Playback started");

        if self.current_track_index.is_some() {
            let position = self.current_position().unwrap_or_default();
            self.send_track_state(true, position)?;
            if !loading {
                self.state = InternalPlaybackState::Playing;
            }
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

            if self.current_track_index.is_some() {
                let position = self.current_position().unwrap_or_default();
                self.send_track_state(false, position)?;
                if !loading {
                    self.state = InternalPlaybackState::Paused;
                }
            }
        }

        Ok(())
    }

    // Reports the current track as playing or paused at `position`. Without a current track
    // nothing is reported.
    fn send_track_state(&self, playing: bool, position: Duration) -> Result<()> {
        let (Some(item), Some(index), Some(path)) = (
            self.current_item.clone(),
            self.current_playlist_index(),
            self.current_track_path.clone(),
        ) else {
            return Ok(());
        };
        let playback_mode = self.playback_mode;
        let event = if playing {
            PlayerEvent::Playing {
                item,
                index,
                path,
                playback_mode,
                position,
            }
        } else {
            PlayerEvent::Paused {
                item,
                index,
                path,
                playback_mode,
                position,
            }
        };
        self.event_sender
            .send(event)
            .context("Failed to send the state of the track")?;
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
                // The system stopped the output, and pausing stops it on this side too. The same
                // output starts again on the next play, so an interruption does not make it stale.
                // Playback goes on after the interruption only if it was wanted; anything else
                // that was heard, such as the track before one that loads paused, only pauses.
                let wanted = match self.state {
                    InternalPlaybackState::Playing => true,
                    InternalPlaybackState::Loading => self.play_when_loaded,
                    _ => false,
                };
                let heard = self.sink.as_ref().is_some_and(|sink| !sink.is_paused());
                if wanted || heard {
                    self.pause()?;
                }
                // Only a stream that stopped with the pause can start again: one that runs on
                // after the system stopped its unit plays nothing, so it gives way to a new one.
                if self
                    ._stream
                    .as_ref()
                    .is_some_and(|stream| !stream.is_suspended())
                {
                    self.mark_output_stale();
                }
                if wanted {
                    self.resume_after_interruption = true;
                }
            }
            OutputEvent::InterruptionEnded { resume } => {
                if std::mem::take(&mut self.resume_after_interruption) && resume {
                    self.play()?;
                }
            }
            OutputEvent::DeviceLost => self.output_lost()?,
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

    // The output can play no more, as when its device went away. Playback pauses where it was,
    // nothing resumes on its own, and the next play opens a new stream there.
    fn output_lost(&mut self) -> Result<()> {
        self.pause()?;
        self.resume_after_interruption = false;
        self.mark_output_stale();
        Ok(())
    }

    // The output stream reported an error, such as headphones that were disconnected. The track
    // pauses in place rather than playing on through another device. A stream that fails tends
    // to report it more than once, and a stale stream is done with already.
    fn stream_failed(&mut self, error: &str) -> Result<()> {
        error!("The audio output failed: {error}");
        if self.sink.is_none() || self.output_stale {
            return Ok(());
        }
        self.output_lost()
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

    // Starts a stopped output again, after a pause or an interruption. The session is made active
    // first, as before any output starts: an interruption leaves it inactive, and a suspended app
    // may wake to a remote command before the end of the interruption reaches it.
    fn resume_output(&self) -> Result<()> {
        let Some(stream) = &self._stream else {
            return Ok(());
        };
        #[cfg(target_os = "ios")]
        if !crate::ios_session::activate(self.mix_with_others) {
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
        if self.current_track_index.is_none() {
            return Ok(());
        }
        let position = self.current_position().unwrap_or_default();
        // The load starts at the place, so the old sink no longer keeps a seek of its own.
        self.pending_seek = None;
        if let Some(sink) = &self.sink {
            sink.pause();
        }
        let Some((index, mapped)) = self.current_track_request() else {
            // Nothing is left to load, and a track shown as paused would never play.
            warn!("The current track is no longer in the playlist, so playback stops");
            return self.stop();
        };
        info!("Opening the audio output again at {position:?}");
        self.load_at(Some(index), true, mapped, Some(position))
    }

    // How to load the current track again: the way it was loaded, while that still leads to it,
    // or else by where it is in the playlist now. Removing tracks before it moves it, and the
    // shuffled order is drawn again. None once it left the playlist.
    fn current_track_request(&self) -> Option<(usize, bool)> {
        let item = self.current_item.as_ref()?;
        let holds_item = |position: Option<usize>| {
            position
                .and_then(|position| self.playlist.get(position))
                .is_some_and(|entry| &entry.item == item)
        };
        if let Some(index) = self.current_track_index
            && holds_item(self.resolve_index(index, self.current_track_mapped))
        {
            return Some((index, self.current_track_mapped));
        }
        self.playlist
            .iter()
            .position(|entry| &entry.item == item)
            .map(|position| (position, false))
    }

    // The playlist position that a load of `index` leads to: the playback mode maps an index
    // asked for as `mapped`, and any other index is a position already. None past the end of
    // the playlist.
    fn resolve_index(&self, index: usize, mapped: bool) -> Option<usize> {
        if index >= self.playlist.len() {
            return None;
        }
        let position = if mapped {
            self.get_mapped_track_index(index)
        } else {
            index
        };
        (position < self.playlist.len()).then_some(position)
    }

    // Where the current track stands in the playlist, which the events report. An index left out
    // of range by removed tracks is reported as it is.
    fn current_playlist_index(&self) -> Option<usize> {
        let index = self.current_track_index?;
        Some(
            self.resolve_index(index, self.current_track_mapped)
                .unwrap_or(index),
        )
    }

    // Keeps the player running when no output stream could be opened, for example while another
    // app holds the audio. The track stays current and paused where it was, ready to play, and
    // the next play tries again from there.
    fn output_failed(
        &mut self,
        error: anyhow::Error,
        track: LoadedTrack,
        position: Duration,
    ) -> Result<()> {
        error!("Failed to open the audio output: {error:?}");
        self.loaded_paused(track, position)
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
        self.pending_seek = None;
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

    // Where the current track stands: where a seek put it while its output was stopped, where it
    // starts once an output opens while it has none that plays, or else where its sink is.
    fn current_position(&self) -> Option<Duration> {
        if let Some(position) = self.pending_seek {
            return Some(position);
        }
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

    fn seek(&mut self, seconds: f64) -> Result<()> {
        let Some(position) = seek_position(seconds) else {
            warn!("Seek to {seconds} s ignored: it is no place in a track");
            return Ok(());
        };

        if self.output_stale {
            // A stale stream never takes the seek, and waiting for it would stall the player.
            if self.state == InternalPlaybackState::Loading {
                // The load in flight brings a new output. If it brings the current track back, as
                // a reopen does, it starts at the new place.
                if !self.move_reload_to(position) {
                    warn!("Seek ignored while another track loads to replace the audio output");
                }
                return Ok(());
            }
            // Keep the place instead, and the next play opens a new stream there.
            self.pending_seek = None;
            if self.current_track_index.is_some() {
                self.restore_position = Some(position);
                self.send_track_state(false, position)?;
            }
            return Ok(());
        }

        if self.output_suspended() {
            // Seeking on a stopped output would wait forever. The track keeps the place, and the
            // next play seeks there once the output runs again.
            self.pending_seek = Some(position);
            self.send_track_state(false, position)?;
            return Ok(());
        }

        let Some(sink) = &self.sink else {
            warn!("Seek command received but no track is loaded");
            return Ok(());
        };
        if let Err(e) = sink.try_seek(position) {
            error!("Failed to seek: {e:#?}");
            return Ok(());
        }
        info!("Seeking to position: {seconds} s");
        let position = sink.get_pos();
        self.send_track_state(self.state == InternalPlaybackState::Playing, position)
    }

    // Makes a load in flight that brings the current track back, such as a reopen, start at
    // `position`. A newer load queued behind it takes over once it completes, so that one moves
    // instead. Returns whether the load is of the current track.
    fn move_reload_to(&mut self, position: Duration) -> bool {
        let target = match self.pending_load {
            Some(pending) => self.resolve_index(pending.index, pending.mapped),
            None => self.loading_index,
        };
        let reloads_current = target
            .and_then(|target| self.playlist.get(target))
            .is_some_and(|entry| Some(&entry.item) == self.current_item.as_ref());
        if !reloads_current {
            return false;
        }
        match &mut self.pending_load {
            Some(pending) => pending.restore_position = Some(position),
            None => self.restore_position = Some(position),
        }
        true
    }

    fn add_to_playlist(&mut self, tracks: Vec<(PlayingItem, PlaybackSource)>, mode: AddMode) {
        debug!("Adding tracks to playlist with mode: {:?}", { mode });
        let count = tracks.len();
        let insert_index = match mode {
            AddMode::PlayNext => {
                if let Some(current_index) = self.current_queue_edit_index() {
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
                count,
            },
        );
        self.schedule_playlist_update();
    }

    fn current_queue_edit_index(&self) -> Option<usize> {
        (self.state != InternalPlaybackState::Stopped)
            .then_some(self.current_track_index)
            .flatten()
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
        self.pending_seek = None;
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
        let index = self.current_playlist_index();
        let path = self.current_track_path.clone();
        let playback_mode = self.playback_mode;
        // A track reports where a seek put it even while its output does not run, so the
        // reports do not take the slider back.
        let position = self.current_position().unwrap_or_default();

        if let Some(sink) = &self.sink {
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
                    position,
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

    // Applies at once. iOS settles whether the app may be the Now Playing app, the one the lock
    // screen and Control Center control, when its session becomes active, and a session only
    // goes inactive while no output runs. So any output that runs stops, including the one an
    // older track keeps while a newer one loads, the session goes down and comes up again with
    // the new category, and the output of a track that plays starts again.
    fn set_mix_with_others(&mut self, x: bool) -> Result<()> {
        if self.mix_with_others == x {
            return Ok(());
        }
        self.mix_with_others = x;
        info!("Mix with other apps status changed: {x}");

        let running = self
            ._stream
            .as_ref()
            .is_some_and(|stream| !stream.is_suspended());
        if running {
            self.suspend_output();
        }
        #[cfg(target_os = "ios")]
        {
            crate::ios_session::deactivate();
            crate::ios_session::configure(x);
        }
        let playing = running && self.sink.as_ref().is_some_and(|sink| !sink.is_paused());
        if playing
            && self.output_suspended()
            && let Err(e) = self.resume_output()
        {
            error!("Failed to start the audio output again: {e:?}");
            self.mark_output_stale();
            // A track that is loading opens an output of its own.
            if self.state == InternalPlaybackState::Playing {
                self.reopen_output()?;
            }
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
    use crate::player::{AudioRequest, LibraryPlaybackSource, RemoteAudioSource};
    use futures::future::BoxFuture;
    use std::collections::{HashMap, HashSet};
    use std::io::{Read, Seek, SeekFrom};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, channel};
    use tokio::runtime::Handle;

    fn opendal_fsio(config: fsio::OpenDalConfig) -> Result<Arc<FsIo>> {
        Ok(Arc::new(FsIo::new_opendal(config, Handle::current())?))
    }

    async fn decode_opendal_asset(fsio: Arc<FsIo>) -> Result<()> {
        let source = open_local_source(fsio, PathBuf::from("startup_0.ogg"), true).await?;
        let duration = source
            .total_duration()
            .context("decoded duration missing")?;
        anyhow::ensure!(duration > Duration::from_secs(1));
        let samples = tokio::task::spawn_blocking(move || -> Result<usize> {
            let mut source = source;
            source
                .try_seek(duration.saturating_sub(Duration::from_millis(500)))
                .map_err(|error| anyhow!("seek failed: {error}"))?;
            let mut samples = 0usize;
            while source.next().is_some() {
                samples += 1;
            }
            Ok(samples)
        })
        .await??;
        anyhow::ensure!(samples > 0, "seek near the end produced no samples");
        Ok(())
    }

    #[tokio::test]
    async fn opendal_track_decodes_seeks_and_finishes_on_current_thread() -> Result<()> {
        use fsio::{OpenDalConfig, test_fixtures::WebDavFixture};

        tokio::time::timeout(Duration::from_secs(30), async {
            let asset = include_bytes!("../../assets/startup_0.ogg").to_vec();
            let fixture = WebDavFixture::start(vec![("startup_0.ogg".into(), asset)]).await?;
            let fsio = opendal_fsio(OpenDalConfig::WebDav {
                endpoint: fixture.endpoint.clone(),
                root: "/dav/library/".into(),
                username: "user".into(),
                password: "pass".into(),
            })?;
            decode_opendal_asset(fsio).await?;
            anyhow::ensure!(fixture.write_requests().is_empty());
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("current-thread OpenDAL decode timed out")??;
        Ok(())
    }

    #[tokio::test]
    async fn opendal_s3_track_decodes_seeks_and_finishes_on_current_thread() -> Result<()> {
        use fsio::{OpenDalConfig, test_fixtures::S3Fixture};

        tokio::time::timeout(Duration::from_secs(30), async {
            let fixture = S3Fixture::start(vec![(
                "startup_0.ogg".into(),
                include_bytes!("../../assets/startup_0.ogg").to_vec(),
            )])
            .await?;
            let fsio = opendal_fsio(OpenDalConfig::S3 {
                endpoint: fixture.endpoint.clone(),
                bucket: "bucket".into(),
                region: "us-east-1".into(),
                root: "/library/".into(),
                access_key_id: "access".into(),
                secret_access_key: "secret".into(),
            })?;
            decode_opendal_asset(fsio).await
        })
        .await
        .context("current-thread OpenDAL S3 decode timed out")??;
        Ok(())
    }

    #[tokio::test]
    async fn rejected_opendal_credentials_are_a_typed_source_failure() -> Result<()> {
        use fsio::{OpenDalConfig, test_fixtures::WebDavFixture};

        tokio::time::timeout(Duration::from_secs(15), async {
            let fixture = WebDavFixture::start(vec![(
                "startup_0.ogg".into(),
                include_bytes!("../../assets/startup_0.ogg").to_vec(),
            )])
            .await?;
            let fsio = opendal_fsio(OpenDalConfig::WebDav {
                endpoint: fixture.endpoint.clone(),
                root: "/dav/library/".into(),
                username: "user".into(),
                password: "wrong".into(),
            })?;
            let error = open_local_source(fsio, PathBuf::from("startup_0.ogg"), true)
                .await
                .expect_err("rejected credentials must fail the source");
            anyhow::ensure!(
                UnavailableReason::classify(&error) == UnavailableReason::AccessRevoked,
                "OpenDAL error was not classified as a typed access failure: {error:#}"
            );
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("current-thread credential failure timed out")??;
        Ok(())
    }
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

    pub(super) struct DelayedResolver {
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

    pub(super) fn delayed_resolver_fixture(
        file_uuids: &[Uuid],
        ids: Arc<Mutex<Vec<i32>>>,
        started: Arc<Notify>,
        release: Arc<Notify>,
        blocked_ids: &[i32],
    ) -> (Arc<DelayedResolver>, PathBuf) {
        delayed_resolver_fixture_with_samples(file_uuids, ids, started, release, blocked_ids, 2)
    }

    // Every track is the same silent 8 kHz mono WAV file with `samples` samples.
    pub(super) fn delayed_resolver_fixture_with_samples(
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
                                    PlayingItem::LibraryTrack {
                                        library_id,
                                        track: file_uuid.to_string(),
                                    },
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
                    let track = |file_uuid: Uuid| {
                        (
                            PlayingItem::LibraryTrack {
                                library_id,
                                track: file_uuid.to_string(),
                            },
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

                    let expected = PlayingItem::LibraryTrack {
                        library_id,
                        track: file_uuids[1].to_string(),
                    };
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
                    let expected = PlayingItem::LibraryTrack {
                        library_id,
                        track: file_uuids[0].to_string(),
                    };
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

// The player's state machine: loads, play and pause, seeks, and what happens to the output.
#[cfg(test)]
mod playback_state_tests {
    use super::stream_open_tests::{
        DelayedResolver, delayed_resolver_fixture, delayed_resolver_fixture_with_samples,
    };
    use super::*;
    use crate::player::PlaylistLoad;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::channel;
    use tokio::runtime::Builder;
    use tokio::sync::Notify;
    use tokio::task::LocalSet;
    use uuid::Uuid;

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
        PlayingItem::LibraryTrack {
            library_id,
            track: file_uuid.to_string(),
        }
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

    #[tokio::test]
    async fn playlist_edit_uses_current_index_when_the_command_runs() -> Result<()> {
        let file_uuids = (0..8).map(|_| Uuid::new_v4()).collect::<Vec<_>>();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&file_uuids);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let original = library_tracks(library_id, &file_uuids[..5]);
                    commands_sender.send(PlayerCommand::AddToPlaylist {
                        tracks: original,
                        mode: AddMode::AppendToEnd,
                    })?;
                    commands_sender.send(PlayerCommand::Switch(2))?;
                    let target_c = library_item(library_id, file_uuids[2]);
                    expect_playing(&mut events, &target_c).await?;
                    let snapshot_index = 2;

                    commands_sender.send(PlayerCommand::Switch(3))?;
                    let target_d = library_item(library_id, file_uuids[3]);
                    expect_playing(&mut events, &target_d).await?;
                    assert_ne!(snapshot_index, 3);

                    commands_sender.send(PlayerCommand::EditPlaylist(PlaylistEdit {
                        tracks: library_tracks(library_id, &file_uuids[5..8]),
                        placement: PlaylistPlacement::PlayNext,
                        playback_mode: None,
                        load: Some(PlaylistLoad {
                            start_index: 1,
                            position: Duration::from_secs(1),
                            play: false,
                        }),
                    }))?;
                    let loaded = next_track_state(&mut events).await?;
                    assert!(
                        matches!(
                            loaded,
                        PlayerEvent::Loaded { ref item, index: 5, .. }
                            if *item == library_item(library_id, file_uuids[6])
                        ),
                        "{loaded:?}"
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
    async fn play_next_edit_appends_and_starts_when_stopped_with_a_stale_index() -> Result<()> {
        let file_uuids = (0..4).map(|_| Uuid::new_v4()).collect::<Vec<_>>();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&file_uuids);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    commands_sender.send(PlayerCommand::AddToPlaylist {
                        tracks: library_tracks(library_id, &file_uuids[..3]),
                        mode: AddMode::AppendToEnd,
                    })?;
                    commands_sender.send(PlayerCommand::Switch(1))?;
                    expect_playing(&mut events, &library_item(library_id, file_uuids[1])).await?;
                    commands_sender.send(PlayerCommand::Stop)?;

                    commands_sender.send(PlayerCommand::EditPlaylist(PlaylistEdit {
                        tracks: library_tracks(library_id, &file_uuids[3..]),
                        placement: PlaylistPlacement::PlayNext,
                        playback_mode: None,
                        load: None,
                    }))?;
                    let started = next_track_state(&mut events).await?;
                    assert!(
                        matches!(started, PlayerEvent::Playing { ref item, index: 3, .. }
                            if *item == library_item(library_id, file_uuids[3])),
                        "{started:?}"
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
        let resumes = internal.output_resumes.clone();
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let track = start_playing(&commands_sender, &mut events, file_uuid).await?;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::Interrupted))?;
                    let paused_at = expect_paused(&mut events, &track).await?;
                    assert!(paused_at >= Duration::from_millis(300), "{paused_at:?}");

                    // The same output starts again, so the track is not loaded again.
                    commands_sender.send(PlayerCommand::Output(
                        OutputEvent::InterruptionEnded { resume: true },
                    ))?;
                    assert_about(expect_playing(&mut events, &track).await?, paused_at);
                    assert_eq!(opens.load(Ordering::SeqCst), 1);
                    assert_eq!(resumes.load(Ordering::SeqCst), 1);

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
    async fn paused_track_starts_its_output_again_on_play_after_an_interruption() -> Result<()> {
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

                    commands_sender.send(PlayerCommand::Play)?;
                    assert_about(expect_playing(&mut events, &track).await?, paused_at);
                    assert_eq!(opens.load(Ordering::SeqCst), 1);
                    assert_eq!(resumes.load(Ordering::SeqCst), 1);

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
                    let paused_at = expect_loaded(&mut events, &track).await?;
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
    async fn stream_error_while_playing_pauses_in_place() -> Result<()> {
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
                    // A failing stream tends to say so more than once.
                    stream_errors.send("the device is no longer available".into())?;
                    stream_errors.send("the device is no longer available".into())?;
                    let paused_at = expect_paused(&mut events, &track).await?;
                    assert_about(paused_at, playing_at);
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    while let Ok(event) = events.try_recv() {
                        assert!(
                            !matches!(
                                event,
                                PlayerEvent::Playing { .. } | PlayerEvent::Paused { .. }
                            ),
                            "{event:?}"
                        );
                    }
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
                    commands_sender.send(PlayerCommand::Output(OutputEvent::DeviceLost))?;
                    expect_paused(&mut events, &track).await?;
                    commands_sender.send(PlayerCommand::Seek(2.0))?;
                    assert_about(
                        expect_paused(&mut events, &track).await?,
                        Duration::from_secs(2),
                    );

                    commands_sender.send(PlayerCommand::Play)?;
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
    async fn seek_during_an_interruption_resumes_there() -> Result<()> {
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
    async fn failed_resume_after_an_interruption_opens_a_new_output_in_place() -> Result<()> {
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
                    commands_sender.send(PlayerCommand::Output(OutputEvent::Interrupted))?;
                    let paused_at = expect_paused(&mut events, &track).await?;
                    failing_resumes.store(1, Ordering::SeqCst);

                    commands_sender.send(PlayerCommand::Output(
                        OutputEvent::InterruptionEnded { resume: true },
                    ))?;
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
    async fn interruption_during_a_paused_load_pauses_the_old_track_for_good() -> Result<()> {
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

                    // The old track plays on while the next one loads paused.
                    commands_sender.send(PlayerCommand::Load { index: 1 })?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::Interrupted))?;
                    expect_paused(&mut events, &current).await?;

                    // Nobody wanted playback, so nothing plays after the interruption.
                    commands_sender.send(PlayerCommand::Output(
                        OutputEvent::InterruptionEnded { resume: true },
                    ))?;
                    assert_nothing_plays(&mut events).await;
                    release.notify_one();
                    expect_loaded(&mut events, &expected).await?;
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
    async fn play_during_an_interruption_cancels_its_resume() -> Result<()> {
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

                    // Play before the end of the interruption came.
                    commands_sender.send(PlayerCommand::Play)?;
                    assert_about(expect_playing(&mut events, &track).await?, paused_at);
                    assert_eq!(opens.load(Ordering::SeqCst), 1);

                    // The track comes up paused again without a Pause, and a late end of the
                    // interruption must not start it.
                    commands_sender.send(PlayerCommand::Load { index: 0 })?;
                    expect_loaded(&mut events, &track).await?;
                    commands_sender.send(PlayerCommand::Output(
                        OutputEvent::InterruptionEnded { resume: true },
                    ))?;
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
    async fn a_track_that_starts_playing_cancels_the_resume_after_an_interruption() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4()];
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&file_uuids);
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
                    expect_playing(&mut events, &first).await?;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::Interrupted))?;
                    expect_paused(&mut events, &first).await?;

                    // Another track starts playing before the end of the interruption came.
                    commands_sender.send(PlayerCommand::Switch(1))?;
                    expect_playing(&mut events, &second).await?;
                    commands_sender.send(PlayerCommand::Load { index: 0 })?;
                    expect_loaded(&mut events, &first).await?;
                    commands_sender.send(PlayerCommand::Output(
                        OutputEvent::InterruptionEnded { resume: true },
                    ))?;
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

    fn restore(
        library_id: Uuid,
        file_uuids: &[Uuid],
        index: usize,
        seconds: f64,
        mode: PlaybackMode,
    ) -> PlayerCommand {
        PlayerCommand::Restore {
            tracks: library_tracks(library_id, file_uuids),
            index: Some(index),
            position: Duration::from_secs_f64(seconds),
            mode,
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
                    commands_sender.send(restore(
                        library_id,
                        &[file_uuid],
                        0,
                        1.5,
                        PlaybackMode::Sequential,
                    ))?;
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
                    commands_sender.send(restore(
                        library_id,
                        &[file_uuid],
                        0,
                        1.0,
                        PlaybackMode::Sequential,
                    ))?;
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
                    commands_sender.send(restore(
                        library_id,
                        &[file_uuid],
                        0,
                        2.0,
                        PlaybackMode::Sequential,
                    ))?;
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
                    commands_sender.send(restore(
                        library_id,
                        &file_uuids,
                        0,
                        1.0,
                        PlaybackMode::Sequential,
                    ))?;
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
                    commands_sender.send(restore(
                        library_id,
                        &file_uuids,
                        1,
                        4.0,
                        PlaybackMode::Sequential,
                    ))?;
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
    async fn mixing_with_other_apps_is_off_until_asked_and_follows_the_command() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, _events, wav_path) = long_track_engine(&[file_uuid]);
        assert!(!internal.mix_with_others);

        // The command goes through the engine's own loop, which keeps running while it waits.
        commands_sender.send(PlayerCommand::SetMixWithOthers(true))?;
        let local = LocalSet::new();
        let internal = local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move {
                    let _ = tokio::time::timeout(Duration::from_millis(200), internal.run()).await;
                    internal
                });
                runner.await
            })
            .await?;
        assert!(internal.mix_with_others);

        let mut internal = internal;
        internal.set_mix_with_others(false)?;
        assert!(!internal.mix_with_others);
        let _ = std::fs::remove_file(wav_path);
        Ok(())
    }

    #[tokio::test]
    async fn changing_mixing_while_playing_restarts_the_output_where_it_plays() -> Result<()> {
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
                    start_playing(&commands_sender, &mut events, file_uuid).await?;
                    commands_sender.send(PlayerCommand::SetMixWithOthers(true))?;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    while events.try_recv().is_ok() {}
                    let before = next_progress(&mut events).await?;
                    assert_eq!(suspends.load(Ordering::SeqCst), 1);
                    assert_eq!(resumes.load(Ordering::SeqCst), 1);

                    // The same output plays on.
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    while events.try_recv().is_ok() {}
                    let later = next_progress(&mut events).await?;
                    assert!(later >= before + Duration::from_millis(300), "{later:?}");
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

                    // The same output plays on from where the track was paused, and says so.
                    commands_sender.send(PlayerCommand::Play)?;
                    assert_about(expect_playing(&mut events, &track).await?, paused_at);
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
    async fn seek_while_paused_waits_for_play_to_run_the_output() -> Result<()> {
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
            let runtime = Builder::new_current_thread().enable_all().build().unwrap();
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
        // The stopped output stays stopped, and the track reports the place it will play from.
        assert_about(next_progress(&mut events).await?, Duration::from_secs(3));
        assert_eq!(resumes.load(Ordering::SeqCst), 0);
        assert_eq!(suspends.load(Ordering::SeqCst), 1);
        assert_nothing_plays(&mut events).await;

        commands_sender.send(PlayerCommand::Play)?;
        assert_about(
            expect_playing(&mut events, &track).await?,
            Duration::from_secs(3),
        );
        assert_about(next_progress(&mut events).await?, Duration::from_secs(3));
        assert_eq!(resumes.load(Ordering::SeqCst), 1);
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

    // A shuffled order in which every track moves one place on, so that no index maps to
    // itself and an index mapped once too often leads to another track.
    struct ShiftedOrder;

    impl PlaybackStrategy for ShiftedOrder {
        fn next(&self, current_index: usize, playlist_len: usize) -> Option<usize> {
            (current_index + 1 < playlist_len).then_some(current_index + 1)
        }

        fn previous(&self, current_index: usize, _playlist_len: usize) -> Option<usize> {
            current_index.checked_sub(1)
        }

        fn on_playlist_end(&self, _playlist_len: usize) -> Option<usize> {
            Some(0)
        }

        fn get_mapped_track_index(&self, index: usize, playlist_len: usize) -> usize {
            (index + 1) % playlist_len
        }

        fn on_playlist_updated(&mut self, _playlist_len: usize, _reason: UpdateReason) {}
    }

    // Runs the player until its next track event, and returns that event. The player keeps its
    // state, so a test can change it before running it again.
    async fn run_until_track_state(
        internal: &mut PlayerInternal,
        events: &mut mpsc::UnboundedReceiver<PlayerEvent>,
    ) -> Result<PlayerEvent> {
        tokio::select! {
            result = internal.run() => bail!("the player stopped: {result:?}"),
            event = next_track_state(events) => event,
        }
    }

    #[tokio::test]
    async fn play_after_a_shuffled_restore_plays_the_restored_track() -> Result<()> {
        let file_uuids: Vec<Uuid> = (0..4).map(|_| Uuid::new_v4()).collect();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&file_uuids);
        let library_id = Uuid::new_v4();
        let track = library_item(library_id, file_uuids[2]);
        commands_sender.send(restore(
            library_id,
            &file_uuids,
            2,
            1.0,
            PlaybackMode::Shuffle,
        ))?;
        let event = run_until_track_state(&mut internal, &mut events).await?;
        assert!(
            matches!(&event, PlayerEvent::Loaded { item, index: 2, .. } if *item == track),
            "{event:?}"
        );

        // Restore asks for a playlist position, which must not be mapped by the shuffled order.
        internal.playback_strategy = Box::new(ShiftedOrder);
        commands_sender.send(PlayerCommand::Play)?;
        let event = run_until_track_state(&mut internal, &mut events).await?;
        assert!(
            matches!(&event, PlayerEvent::Playing { item, index: 2, .. } if *item == track),
            "{event:?}"
        );
        assert_about(position_of(&event), Duration::from_secs(1));
        let _ = std::fs::remove_file(wav_path);
        Ok(())
    }

    #[tokio::test]
    async fn load_at_position_uses_the_playlist_index_in_shuffle_mode() -> Result<()> {
        let file_uuids: Vec<Uuid> = (0..4).map(|_| Uuid::new_v4()).collect();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&file_uuids);
        let library_id = Uuid::new_v4();
        let expected = library_item(library_id, file_uuids[2]);
        internal.playback_mode = PlaybackMode::Shuffle;
        internal.playback_strategy = Box::new(ShiftedOrder);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                tokio::time::timeout(Duration::from_secs(2), async {
                    while !matches!(events.recv().await, Some(PlayerEvent::PlaylistUpdated(_))) {}
                })
                .await?;

                commands_sender.send(PlayerCommand::LoadAtPosition {
                    index: 2,
                    position: Duration::from_secs(1),
                    play: false,
                })?;
                let event = next_track_state(&mut events).await?;
                assert!(
                    matches!(&event, PlayerEvent::Loaded { item, index: 2, .. } if *item == expected),
                    "{event:?}"
                );

                runner.abort();
                let _ = std::fs::remove_file(wav_path);
                Ok::<(), anyhow::Error>(())
            })
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn play_after_an_earlier_track_was_removed_plays_the_same_track() -> Result<()> {
        let file_uuids = [
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        ];
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&file_uuids);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let track = library_item(library_id, file_uuids[2]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                    commands_sender.send(PlayerCommand::Switch(2))?;
                    expect_playing(&mut events, &track).await?;
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::DeviceLost))?;
                    let paused_at = expect_paused(&mut events, &track).await?;

                    // The track moves up one place, and the next play opens a new output for it.
                    commands_sender.send(PlayerCommand::RemoveFromPlaylist { index: 0 })?;
                    commands_sender.send(PlayerCommand::Play)?;
                    let event = next_track_state(&mut events).await?;
                    assert!(
                        matches!(&event, PlayerEvent::Playing { item, index: 1, .. } if *item == track),
                        "{event:?}"
                    );
                    assert_about(position_of(&event), paused_at);

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
    async fn play_after_the_track_was_removed_stops() -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&file_uuids);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let track = library_item(library_id, file_uuids[1]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                    commands_sender.send(PlayerCommand::Switch(1))?;
                    expect_playing(&mut events, &track).await?;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::DeviceLost))?;
                    expect_paused(&mut events, &track).await?;

                    commands_sender.send(PlayerCommand::RemoveFromPlaylist { index: 1 })?;
                    commands_sender.send(PlayerCommand::Play)?;
                    tokio::time::timeout(Duration::from_secs(2), async {
                        loop {
                            match events.recv().await {
                                Some(PlayerEvent::Stopped) => return Ok(()),
                                Some(
                                    event @ (PlayerEvent::Playing { .. }
                                    | PlayerEvent::Loaded { .. }),
                                ) => bail!("{event:?}"),
                                Some(_) => {}
                                None => bail!("player events closed"),
                            }
                        }
                    })
                    .await??;
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
    async fn track_whose_first_output_fails_to_open_is_ready_to_play() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let opens = internal.output_opens.clone();
        internal.failing_opens.store(1, Ordering::SeqCst);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let track = library_item(library_id, file_uuid);
                    commands_sender.send(add_library_tracks(library_id, &[file_uuid]))?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    // `Loaded` is what marks a track ready in the player's status, which lets the
                    // UI offer Play.
                    assert_about(expect_loaded(&mut events, &track).await?, Duration::ZERO);
                    assert_nothing_plays(&mut events).await;

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

    #[tokio::test]
    async fn seek_while_a_new_output_opens_is_where_playback_goes_on() -> Result<()> {
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
                    release.notify_one();
                    expect_playing(&mut events, &track).await?;
                    commands_sender.send(PlayerCommand::Output(OutputEvent::DeviceLost))?;
                    expect_paused(&mut events, &track).await?;

                    // Play loads the track again for a new output, and the resolver holds it.
                    commands_sender.send(PlayerCommand::Play)?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::Seek(3.0))?;
                    release.notify_one();
                    assert_about(
                        expect_playing(&mut events, &track).await?,
                        Duration::from_secs(3),
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
    async fn changing_mixing_during_a_switch_stops_the_old_output_first() -> Result<()> {
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

                    // The old track keeps its output while the new one loads. The session only
                    // changes while no output runs, and the old track plays on afterwards.
                    commands_sender.send(PlayerCommand::Switch(1))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(PlayerCommand::SetMixWithOthers(true))?;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    while events.try_recv().is_ok() {}
                    let before = next_progress(&mut events).await?;
                    assert_eq!(suspends.load(Ordering::SeqCst), 1);
                    assert_eq!(resumes.load(Ordering::SeqCst), 1);
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    while events.try_recv().is_ok() {}
                    let later = next_progress(&mut events).await?;
                    assert!(later >= before + Duration::from_millis(300), "{later:?}");

                    release.notify_one();
                    expect_playing(&mut events, &expected).await?;

                    runner.abort();
                    let _ = std::fs::remove_file(wav_path);
                    Ok::<(), anyhow::Error>(())
                }
                .await
            })
            .await?;
        Ok(())
    }

    #[test]
    fn a_seek_before_the_start_goes_to_the_start_and_one_to_no_place_is_dropped() {
        assert_eq!(seek_position(2.5), Some(Duration::from_millis(2500)));
        assert_eq!(seek_position(-3.0), Some(Duration::ZERO));
        assert_eq!(seek_position(f64::NEG_INFINITY), Some(Duration::ZERO));
        assert_eq!(seek_position(f64::NAN), None);
        assert_eq!(seek_position(f64::INFINITY), None);
    }

    #[tokio::test]
    async fn seek_before_the_start_plays_from_the_start() -> Result<()> {
        let file_uuid = Uuid::new_v4();
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&[file_uuid]);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let track = start_playing(&commands_sender, &mut events, file_uuid).await?;
                    commands_sender.send(PlayerCommand::Seek(-3.0))?;
                    assert_about(expect_playing(&mut events, &track).await?, Duration::ZERO);

                    // A seek to no place is dropped, and the player still answers.
                    commands_sender.send(PlayerCommand::Seek(f64::NAN))?;
                    commands_sender.send(PlayerCommand::SetVolume(0.5))?;
                    tokio::time::timeout(Duration::from_secs(2), async {
                        loop {
                            match events.recv().await {
                                Some(PlayerEvent::VolumeUpdate(_)) => return Ok(()),
                                Some(event @ PlayerEvent::Playing { .. }) => bail!("{event:?}"),
                                Some(_) => {}
                                None => bail!("player events closed"),
                            }
                        }
                    })
                    .await??;

                    runner.abort();
                    let _ = std::fs::remove_file(wav_path);
                    Ok::<(), anyhow::Error>(())
                }
                .await
            })
            .await?;
        Ok(())
    }

    fn load_at_position(index: usize, seconds: f64, play: bool) -> PlayerCommand {
        PlayerCommand::LoadAtPosition {
            index,
            position: Duration::from_secs_f64(seconds),
            play,
        }
    }

    // Loads the second of two tracks at 2 s and checks that it comes up there, playing or
    // paused. A paused one starts playing there too, on an output that the play opens for the
    // same track.
    async fn check_load_at_position(play: bool) -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4()];
        let (mut internal, commands_sender, mut events, wav_path) = long_track_engine(&file_uuids);
        let local = LocalSet::new();
        local
            .run_until(async move {
                let runner = tokio::task::spawn_local(async move { internal.run().await });
                async {
                    let library_id = Uuid::new_v4();
                    let track = library_item(library_id, file_uuids[1]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                    commands_sender.send(load_at_position(1, 2.0, play))?;
                    let position = if play {
                        expect_playing(&mut events, &track).await?
                    } else {
                        expect_loaded(&mut events, &track).await?
                    };
                    assert_about(position, Duration::from_secs(2));

                    if !play {
                        commands_sender.send(PlayerCommand::Play)?;
                        assert_about(
                            expect_playing(&mut events, &track).await?,
                            Duration::from_secs(2),
                        );
                    }

                    runner.abort();
                    let _ = std::fs::remove_file(wav_path);
                    Ok::<(), anyhow::Error>(())
                }
                .await
            })
            .await
    }

    #[tokio::test]
    async fn load_at_a_position_comes_up_playing_there() -> Result<()> {
        check_load_at_position(true).await
    }

    #[tokio::test]
    async fn load_at_a_position_comes_up_paused_there() -> Result<()> {
        check_load_at_position(false).await
    }

    // As `check_load_at_position`, while the first track is still loading. The newer request
    // wins, with its own position.
    async fn check_load_at_position_during_a_load(play: bool) -> Result<()> {
        let file_uuids = [Uuid::new_v4(), Uuid::new_v4()];
        let ids = Arc::new(Mutex::new(Vec::new()));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (resolver, wav_path) = delayed_resolver_fixture_with_samples(
            &file_uuids,
            ids.clone(),
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
                    let track = library_item(library_id, file_uuids[1]);
                    commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
                    commands_sender.send(PlayerCommand::Switch(0))?;
                    tokio::time::timeout(Duration::from_secs(2), started.notified()).await?;
                    commands_sender.send(load_at_position(1, 2.0, play))?;
                    release.notify_one();

                    let position = if play {
                        expect_playing(&mut events, &track).await?
                    } else {
                        expect_loaded(&mut events, &track).await?
                    };
                    assert_about(position, Duration::from_secs(2));
                    assert_eq!(&*ids.lock().unwrap(), &[1, 2]);

                    runner.abort();
                    let _ = std::fs::remove_file(wav_path);
                    Ok::<(), anyhow::Error>(())
                }
                .await
            })
            .await
    }

    #[tokio::test]
    async fn load_at_a_position_during_a_load_comes_up_playing_there() -> Result<()> {
        check_load_at_position_during_a_load(true).await
    }

    #[tokio::test]
    async fn load_at_a_position_during_a_load_comes_up_paused_there() -> Result<()> {
        check_load_at_position_during_a_load(false).await
    }
}
