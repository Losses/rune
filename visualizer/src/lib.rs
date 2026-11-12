//! A scene visualizer: a render thread that turns PCM into BGRA frames.
//!
//! The thread owns a headless wgpu device and the scene on screen (see [`scene`]). Each frame it
//! drains the PCM ring buffer and renders. On Apple platforms it draws straight into an IOSurface-backed pixel buffer, on
//! Windows into a DirectX 12 texture shared with Flutter's adapter, and on Linux into a Vulkan
//! image whose memory Flutter's OpenGL context imports, which the app can show without a copy.
//! On Android it presents into the native window of a Flutter texture (`Visualizer::set_window`),
//! where Flutter takes the frame itself, so no [`Frame`] is made. Elsewhere, or if that fails, it
//! renders into an offscreen texture that is copied into a readback buffer, and the mapped
//! buffer itself is the frame. Apart from Android, the result is the latest [`Frame`]. Readers ([`Visualizer::latest_frame`]) only
//! clone an `Arc` under a short lock and never wait for the GPU. A reader that keeps frames holds
//! their buffers, and the renderer skips frames while none is free.

mod adaptive;
mod bench;
pub mod bind;
mod builtin;
mod chart;
mod course;
mod covers;
mod gpu_time;
mod in_flight;
mod inspect;
mod lab;
mod loading;
mod moments;
pub mod package;
mod pipeline;
mod plan;
mod presets;
mod published;
mod readback;
mod rhythm;
mod scene;
#[cfg(target_os = "windows")]
mod shared_dx12;
#[cfg(target_os = "linux")]
mod shared_vulkan;
#[cfg(target_vendor = "apple")]
mod surface;
mod switching;
mod transition;
#[cfg(target_os = "android")]
mod window;

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use fsio::FsIo;
use log::{debug, error, info, warn};
use rtrb::Consumer;

pub use bind::{Level, RuntimeBindings, Used};

/// The runtime's decisions as the visualizer is built with them (`runtime.toml`), the text a lab
/// edits and sends with [`LabCommand::RuntimeBindings`].
pub const RUNTIME_TOML: &str = bind::RUNTIME_TOML;
pub use builtin::{Credit, credits as builtin_credits};
pub use chart::{Chart, ChartItem, ChartSource, ChartState};
use chart::{ChartFollower, Playback};
pub use inspect::{
    Inspect, InspectBinding, InspectDirector, InspectEffects, InspectInput, InspectLens,
    InspectLooks, InspectMusic, InspectSection, InspectShot, InspectSignal, InspectSwitching,
    InspectTransition, LabCommand, LabEvent, LabEventKind, LabLens, LabOverrides, LabPace,
    LabTransition,
};
pub use lab::RATE as LAB_RATE;
pub use lab::{ColumnGroup, ColumnKind, ColumnPlan, LabColumn, LabColumns, LabSample, PlanSeries};
use loading::Loads;
pub use moments::{Chances, EffectLevel};
pub use pipeline::{DEFAULT_PRESET_NAME, Transport, probe_gpu};
use pipeline::{Pipeline, Rendered, SceneBuild};
pub use plan::{PlanCut, PlanLook, PlanMoment, PlanSection, SongPlan};
pub use presets::{Location, PresetInfo, PresetLoadError, VisualizerInputs};
use presets::{Preset, PresetList};
pub use rhythm::{BeatTracker, Rhythm};
use scene::Scene;
pub use scene::SceneInputs;
#[cfg(target_os = "linux")]
pub use shared_vulkan::SharedFrame;
pub use transition::CutInStyle;
use transition::{Durations, Transition};
#[cfg(target_os = "android")]
pub use window::Window;

pub const DEFAULT_FPS: u32 = 60;

/// Analyse a track once and encode the chart for a renderer feed.
pub fn analyze_chart_file(fsio: &FsIo, path: &Path) -> anyhow::Result<Vec<u8>> {
    let cancelled = AtomicBool::new(false);
    Chart::from_analysis(chart_analyzer::analyze_file(fsio, path, &cancelled)?).encode()
}

/// Told the id of each new frame (see [`Visualizer::set_frame_listener`]).
pub type FrameListener = Arc<dyn Fn(u64) + Send + Sync>;

/// Told how frames leave the GPU (see [`Visualizer::set_transport_listener`]).
pub type TransportListener = Arc<dyn Fn(Transport) + Send + Sync>;

/// Told every error (see [`Visualizer::set_error_listener`]): what was being done, in a word or
/// two such as `load_scene`, and the error.
pub type ErrorListener = Arc<dyn Fn(&'static str, &str) + Send + Sync>;

/// Sample rate assumed until the first track tells the real one.
const FALLBACK_SAMPLE_RATE: u32 = 44_100;

/// How often the measured fps in [`Status`] is refreshed.
const FPS_WINDOW: Duration = Duration::from_millis(500);
/// How often a paused render thread looks at whether the music plays again.
const PAUSED_POLL: Duration = Duration::from_millis(50);
/// How far the paused song's place must move to count as sought and be drawn again: less is a
/// report of the same place.
const SOUGHT: Duration = Duration::from_millis(10);

/// The audio the visualizer listens to.
pub struct PcmInput {
    /// Interleaved samples, `-1..1`.
    pub consumer: Consumer<f32>,
    /// Channel count of the interleaved samples (0 while unknown).
    pub channels: Arc<AtomicU32>,
    /// Sample rate of the samples (0 while unknown).
    pub sample_rate: Arc<AtomicU32>,
}

/// One rendered picture.
pub struct Frame {
    /// Starts at 1 and only ever increases, also across restarts.
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub data: FrameData,
}

pub enum FrameData {
    /// A mapped readback buffer: rows top to bottom, BGRA on Apple and RGBA elsewhere.
    Mapped(readback::MappedFrame),
    /// An IOSurface-backed `kCVPixelFormatType_32BGRA` pixel buffer.
    #[cfg(target_vendor = "apple")]
    PixelBuffer(surface::PixelBuffer),
    /// A DirectX 12 texture (`DXGI_FORMAT_R8G8B8A8_UNORM`) shared through an NT handle.
    #[cfg(target_os = "windows")]
    Shared(shared_dx12::SharedTexture),
    /// A Vulkan image (`VK_FORMAT_R8G8B8A8_UNORM`) whose memory and semaphore can be exported as
    /// file descriptors.
    #[cfg(target_os = "linux")]
    Shared(shared_vulkan::SharedFrame),
}

impl Frame {
    /// The pixels and the bytes from one row to the next (at least `width * 4`; equal to it when
    /// the width is a multiple of 64), if the frame was read back. BGRA on Apple, RGBA elsewhere.
    pub fn pixels(&self) -> Option<(&[u8], usize)> {
        match &self.data {
            FrameData::Mapped(frame) => Some((frame.bytes(), frame.stride())),
            #[cfg(target_vendor = "apple")]
            FrameData::PixelBuffer(_) => None,
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            FrameData::Shared(_) => None,
        }
    }

    /// The IOSurface-backed `kCVPixelFormatType_32BGRA` pixel buffer the frame is in, if it is in
    /// one.
    #[cfg(target_vendor = "apple")]
    pub fn pixel_buffer(
        &self,
    ) -> Option<objc2_core_foundation::CFRetained<objc2_core_video::CVPixelBuffer>> {
        match &self.data {
            FrameData::PixelBuffer(buffer) => Some(buffer.retained()),
            FrameData::Mapped(_) => None,
        }
    }

    /// The NT handle of the shared texture, if the frame is in one. It stays valid while the frame
    /// is held; the receiver opens its own reference and must not close the handle.
    #[cfg(target_os = "windows")]
    pub fn shared_handle(&self) -> Option<*mut std::ffi::c_void> {
        match &self.data {
            FrameData::Shared(texture) => Some(texture.handle()),
            FrameData::Mapped(_) => None,
        }
    }

    /// The shared image the frame is in, if it is in one. Its memory and semaphore are exported
    /// from it while the frame is held.
    #[cfg(target_os = "linux")]
    pub fn shared_image(&self) -> Option<&shared_vulkan::SharedFrame> {
        match &self.data {
            FrameData::Shared(frame) => Some(frame),
            FrameData::Mapped(_) => None,
        }
    }

    /// The pixels as tightly packed rows (BGRA on Apple, RGBA elsewhere). Copies; for tests. A
    /// frame in a shared texture (Windows) or image (Linux) has no pixels to copy here.
    pub fn to_packed(&self) -> Result<Vec<u8>> {
        match &self.data {
            FrameData::Mapped(frame) => {
                let row = self.width as usize * 4;
                let (bytes, stride) = (frame.bytes(), frame.stride());
                if stride < row || bytes.len() < stride * (self.height as usize - 1) + row {
                    bail!("The frame is smaller than its size says");
                }
                Ok((0..self.height as usize)
                    .flat_map(|y| &bytes[y * stride..y * stride + row])
                    .copied()
                    .collect())
            }
            #[cfg(target_vendor = "apple")]
            FrameData::PixelBuffer(buffer) => buffer.read_bgra(self.width, self.height),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            FrameData::Shared(_) => bail!("The frame is in a shared texture"),
        }
    }
}

/// The device and driver UUIDs of a GPU, as Vulkan (`VkPhysicalDeviceIDProperties`) and OpenGL
/// (`GL_DEVICE_UUID_EXT`, `GL_DRIVER_UUID_EXT`) report them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceUuids {
    pub device: [u8; 16],
    pub driver: [u8; 16],
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub running: bool,
    pub width: u32,
    pub height: u32,
    pub target_fps: u32,
    pub measured_fps: f32,
    pub preset_name: String,
    /// Where the scene on screen is, as [`Location`]'s text (`builtin:<name>`, a folder's path or
    /// `app:cover-wall`): the name alone does not tell a built-in scene from a folder scene of that
    /// name. Empty until the scene the visualizer begins with is on screen.
    pub preset_path: String,
    /// Scenes to choose from: the built-in ones and those of the chosen folders.
    pub preset_count: usize,
    /// Inputs available to installed scenes; `None` also means no installed scene can use them.
    pub inputs: VisualizerInputs,
    pub adapter: String,
    /// How frames leave the GPU, once the render thread runs.
    pub transport: Option<Transport>,
    pub last_error: Option<String>,
}

enum Command {
    Resize(u32, u32),
    /// Build this scene on another thread and show it when it is done; one from a folder is then
    /// watched for changes. The scene on screen stays meanwhile.
    LoadScene(Location),
    /// Show the built-in scene that comes first (no scene folder is left), built the same way.
    LoadDefaultScene,
    /// Android: present frames of this size into this window from now on.
    #[cfg(target_os = "android")]
    ReplaceWindow(Window, u32, u32),
    /// A scene lab's request (see [`Visualizer::lab`]).
    Lab(LabCommand),
    Stop,
}

/// The built-in scenes and those of the folders the user chose, if any.
struct Library {
    folder: PresetList,
}

impl Default for Library {
    fn default() -> Self {
        let names = builtin::scenes()
            .into_iter()
            .map(|scene| {
                let inputs = scene.source.required_inputs().unwrap_or_default();
                (scene.name, inputs)
            })
            .collect();
        Self {
            folder: PresetList::with_builtins(names),
        }
    }
}

impl Library {
    /// The scene after `current` in the order, for a scene lab's switch, if there are scenes to
    /// choose from.
    fn next_after(&mut self, current: &Location) -> Option<Result<Preset>> {
        (self.folder.len() > 0).then(|| self.folder.next_other(current))
    }
}

struct Shared {
    /// The frames published for readers (see [`published`]).
    frames: Mutex<published::Published<Arc<Frame>>>,
    /// How long the frame taken last by [`Visualizer::next_frame`] waited from being published,
    /// in nanoseconds plus one; 0 once the render thread has seen it.
    taken_age: AtomicU64,
    status: Mutex<Status>,
    next_frame_id: AtomicU64,
    /// Read frames back even where pixel buffers work (tests, and comparing the two paths).
    prefer_readback: AtomicBool,
    /// Whether the music plays. While it does not, the picture and its clock stand still.
    playing: AtomicBool,
    /// Whether the app's window can be seen. While it cannot, nothing is drawn and the clock
    /// stands still, as while the music is paused.
    visible: AtomicBool,
    /// The app's accent colour (0xAARRGGBB) for scenes, or 0 before the app tells it.
    accent: AtomicU32,
    /// How often effects play: the user's setting (0 default, 1 more).
    effect_level: AtomicU8,
    /// A scene lab's chance for every moment, in percent, or [`NO_LAB_CHANCE`] for the setting's.
    lab_chance: AtomicU8,
    /// Whether the render thread fills [`Shared::inspect`] (see [`Visualizer::set_inspecting`]).
    inspecting: AtomicBool,
    /// What the render thread saw in its last frame, while inspecting.
    inspect: Mutex<Option<Inspect>>,
    /// The song on screen played through ahead of time, while inspecting and it has a chart.
    plan: Mutex<PlanSlot>,
    /// The number of the latest plan job; an older job drops its result.
    plan_job: AtomicU64,
    /// What the visualizer decided and was told in the last moments, while inspecting, oldest
    /// first (see [`Visualizer::drain_lab_events`]).
    lab_events: Mutex<VecDeque<LabEvent>>,
    /// The lab's catalogue of columns and the frames folded into buckets, while inspecting (see
    /// [`Visualizer::lab_columns`]).
    lab: Mutex<lab::LabState>,
    /// The file that plays and where, for analysing it into a chart and the time in it.
    playback: Mutex<Playback>,
    /// The chart of the song while the visualizer is stopped, for when it starts again.
    kept_chart: chart::KeptChart,
    /// Whether the scene on screen reads the atlas of album covers.
    wants_covers: AtomicBool,
    /// The latest atlas of covers and its version, which the render thread uploads.
    covers: Mutex<CoverAtlas>,
    /// The number of the latest `set_covers` job; an older job drops its result.
    covers_job: AtomicU64,
    /// The latest cell for the cover of the track playing and its version.
    playing_cover: Mutex<PlayingCover>,
    /// The bounded encoded covers from the current renderer feed.
    feed_covers: Mutex<Option<FeedCovers>>,
    /// The number of the latest `set_playing_cover` job; an older job drops its result.
    playing_cover_job: AtomicU64,
    /// Asked to provide covers (by calling [`Visualizer::set_covers`]) when a scene that reads
    /// them is loaded.
    covers_requester: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Told the id of every new frame.
    frame_listener: Mutex<Option<FrameListener>>,
    /// Told how frames leave the GPU, when the render thread knows and when that changes.
    transport_listener: Mutex<Option<TransportListener>>,
    /// Told every error.
    error_listener: Mutex<Option<ErrorListener>>,
    /// The LUID of the adapter Flutter renders on (see [`adapter_luid`]), 0 while unknown.
    adapter_luid: AtomicU64,
    /// The UUIDs of the GPU Flutter renders on, while known.
    device_uuids: Mutex<Option<DeviceUuids>>,
    /// The app could not show a shared frame: the render thread reads frames back from its next
    /// frame on.
    fall_back: AtomicBool,
    /// Latest realtime spectrum from the renderer feed, retained between display frames.
    feed_spectrum: Mutex<Option<Vec<f32>>>,
    /// Android: the window the next render thread presents into, which it takes.
    #[cfg(target_os = "android")]
    window: Mutex<Option<Window>>,
}

/// The most events the lab's log keeps: when it is not read, the oldest go.
const LAB_EVENTS: usize = 512;

/// The plan of the song made last (none: there is no chart) and how often it was replaced.
#[derive(Default)]
struct PlanSlot {
    plan: Option<Arc<SongPlan>>,
    version: u64,
}

/// The atlas of covers made last (none: there are no covers) and how often it was replaced.
#[derive(Default)]
struct CoverAtlas {
    atlas: Option<Arc<covers::Atlas>>,
    version: u64,
}

/// The cell made last for the track playing (none: not told yet) and how often it was replaced.
#[derive(Default)]
struct PlayingCover {
    cell: Option<Arc<covers::Cell>>,
    version: u64,
}

#[derive(Clone)]
struct FeedCovers {
    item: ChartItem,
    current: Option<Vec<u8>>,
    random: Vec<Vec<u8>>,
}

impl Shared {
    /// The chances the moments get an effect with: a scene lab's, else the user's setting.
    fn effect_chances(&self) -> Chances {
        match self.lab_chance.load(Ordering::Relaxed) {
            NO_LAB_CHANCE => match self.effect_level.load(Ordering::Relaxed) {
                1 => EffectLevel::More,
                _ => EffectLevel::Default,
            }
            .chances(),
            percent => Chances::every(f32::from(percent) / 100.0),
        }
    }

    /// Put `events` in the lab's log, if a lab is inspecting; the log keeps the newest
    /// [`LAB_EVENTS`].
    fn log_events(&self, events: impl IntoIterator<Item = LabEvent>) {
        if !self.inspecting.load(Ordering::Relaxed) {
            return;
        }
        let mut log = lock(&self.lab_events);
        for event in events {
            if log.len() == LAB_EVENTS {
                log.pop_front();
            }
            log.push_back(event);
        }
    }

    /// Keep `plan`, made by job `job`, unless a later job was started since.
    fn store_plan(&self, job: u64, plan: SongPlan) {
        let mut slot = lock(&self.plan);
        if self.plan_job.load(Ordering::Relaxed) != job {
            return;
        }
        slot.version += 1;
        slot.plan = Some(Arc::new(plan));
    }

    /// Drop the plan, and the result of a job still running.
    fn clear_plan(&self) {
        let mut slot = lock(&self.plan);
        self.plan_job.fetch_add(1, Ordering::Relaxed);
        if slot.plan.take().is_some() {
            slot.version += 1;
        }
    }

    fn status(&self) -> MutexGuard<'_, Status> {
        self.status.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Record an error of `operation` (see [`ErrorListener`]) for [`Status::last_error`] and the
    /// error listener.
    fn set_error(&self, operation: &'static str, message: String) {
        error!("Visualizer ({operation}): {message}");
        let listener = lock(&self.error_listener).clone();
        if let Some(listener) = listener {
            listener(operation, &message);
        }
        self.status().last_error = Some(message);
    }
}

struct Worker {
    commands: Sender<Command>,
    handle: JoinHandle<PcmInput>,
}

pub struct Visualizer {
    shared: Arc<Shared>,
    worker: Mutex<Option<Worker>>,
    /// Shared with the render thread, which picks the scene to begin with and a lab's next one.
    library: Arc<Mutex<Library>>,
}

impl Default for Visualizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Visualizer {
    pub fn new() -> Self {
        let library = Library::default();
        let status = Status {
            preset_count: library.folder.len(),
            ..Status::default()
        };
        Self {
            shared: Arc::new(Shared {
                frames: Mutex::default(),
                taken_age: AtomicU64::new(0),
                status: Mutex::new(status),
                next_frame_id: AtomicU64::new(1),
                // `RUNE_VISUALIZER_READBACK=1` picks the readback path, to compare the two.
                prefer_readback: AtomicBool::new(
                    std::env::var_os("RUNE_VISUALIZER_READBACK").is_some_and(|v| v == "1"),
                ),
                // Until the player says otherwise, draw.
                playing: AtomicBool::new(true),
                inspecting: AtomicBool::new(false),
                inspect: Mutex::new(None),
                plan: Mutex::default(),
                plan_job: AtomicU64::new(0),
                lab_events: Mutex::default(),
                lab: Mutex::default(),
                visible: AtomicBool::new(true),
                accent: AtomicU32::new(0),
                effect_level: AtomicU8::new(0),
                lab_chance: AtomicU8::new(NO_LAB_CHANCE),
                playback: Mutex::new(Playback::default()),
                kept_chart: chart::KeptChart::default(),
                wants_covers: AtomicBool::new(false),
                covers: Mutex::new(CoverAtlas::default()),
                covers_job: AtomicU64::new(0),
                playing_cover: Mutex::new(PlayingCover::default()),
                feed_covers: Mutex::new(None),
                playing_cover_job: AtomicU64::new(0),
                covers_requester: Mutex::new(None),
                frame_listener: Mutex::new(None),
                transport_listener: Mutex::new(None),
                error_listener: Mutex::new(None),
                adapter_luid: AtomicU64::new(0),
                device_uuids: Mutex::new(None),
                fall_back: AtomicBool::new(false),
                feed_spectrum: Mutex::new(None),
                #[cfg(target_os = "android")]
                window: Mutex::new(None),
            }),
            worker: Mutex::new(None),
            library: Arc::new(Mutex::new(library)),
        }
    }

    /// Whether the render thread copies what it saw in each frame for [`Self::inspect`]. Off by
    /// default: the app pays nothing for it.
    pub fn set_inspecting(&self, inspecting: bool) {
        self.shared.inspecting.store(inspecting, Ordering::Relaxed);
        if !inspecting {
            *lock(&self.shared.inspect) = None;
            lock(&self.shared.lab_events).clear();
            lock(&self.shared.lab).reset();
            self.shared.clear_plan();
        }
    }

    /// A copy of what the render thread saw in its last frame, while inspecting (see
    /// [`Self::set_inspecting`]) and once a frame was drawn; else none.
    pub fn inspect(&self) -> Option<Inspect> {
        lock(&self.shared.inspect).clone()
    }

    /// Whether a lab inspects the render thread (see [`Self::set_inspecting`]).
    pub fn is_inspecting(&self) -> bool {
        self.shared.inspecting.load(Ordering::Relaxed)
    }

    /// The song on screen played through ahead of time with the scene's decisions, while
    /// inspecting (see [`Self::set_inspecting`]) and the song has a chart; else none. Made in the
    /// background after the song, the scene or what a lab set changes.
    pub fn plan(&self) -> Option<Arc<SongPlan>> {
        let plan = lock(&self.shared.plan).plan.clone()?;
        // The plan of a song that no longer plays (it changed while the visualizer was stopped or
        // showed the cover wall, and planned nothing) is not this song's.
        let playing = match &lock(&self.shared.playback).source {
            ChartSource::LocalFile { path, .. } => Some(path.display().to_string()),
            ChartSource::None | ChartSource::Remote { .. } | ChartSource::RemoteWithoutItem => None,
        };
        (playing.as_deref() == Some(plan.path.as_str())).then_some(plan)
    }

    /// How often the plan was replaced or dropped: a reader that has seen a version needs to
    /// fetch the plan again only when this changes.
    pub fn plan_version(&self) -> u64 {
        lock(&self.shared.plan).version
    }

    /// The columns of the scene on screen that a lab draws as series (see [`crate::lab`]), while
    /// inspecting and once a frame was drawn; else none. Its version changes with the scene, the
    /// runtime's bindings, the scene's bindings and its parameters, and with each new song.
    pub fn lab_columns(&self) -> Option<Arc<LabColumns>> {
        lock(&self.shared.lab).columns()
    }

    /// The buckets of song time (a twentieth of a second) finished since the last call, oldest
    /// first, with the catalogue each follows; at most the last two minutes. Empty while not
    /// inspecting.
    pub fn drain_lab_samples(&self) -> Vec<LabSample> {
        lock(&self.shared.lab).drain()
    }

    /// What the visualizer decided (a cut, a look, a moment's effect) and what a lab told it
    /// since the last call, oldest first; at most the last 512. Empty while not inspecting.
    pub fn drain_lab_events(&self) -> Vec<LabEvent> {
        lock(&self.shared.lab_events).drain(..).collect()
    }

    /// Tell the render thread to do something now (see [`LabCommand`]), for a scene lab.
    pub fn lab(&self, command: LabCommand) -> Result<()> {
        self.send(Command::Lab(command))
    }

    /// Follow the user's setting of how often effects play at the music's moments.
    pub fn set_effect_level(&self, level: EffectLevel) {
        let level = match level {
            EffectLevel::Default => 0,
            EffectLevel::More => 1,
        };
        self.shared.effect_level.store(level, Ordering::Relaxed);
    }

    /// Follow the app's accent colour (0xAARRGGBB), which scenes may draw in.
    pub fn set_accent(&self, argb: u32) {
        self.shared.accent.store(argb, Ordering::Relaxed);
    }

    /// Whether the scene on screen reads the atlas of album covers (`covers = true`). Only then
    /// is it worth finding covers for [`Self::set_covers`].
    pub fn wants_covers(&self) -> bool {
        self.shared.wants_covers.load(Ordering::Relaxed)
    }

    /// `request` is called (from the render thread, so it must not block) whenever a scene that
    /// reads covers is loaded; it should end in [`Self::set_covers`].
    pub fn set_covers_requester(&self, request: impl Fn() + Send + Sync + 'static) {
        *lock(&self.shared.covers_requester) = Some(Arc::new(request));
    }

    /// The album covers for scenes that read them: the cover of the track playing (none if it has
    /// none) and random ones from the library, image files. The atlas is built on a background
    /// thread (a file that does not decode is skipped) and the scene picks it up with the next
    /// frame; a later call replaces an earlier one.
    pub fn set_covers(&self, current: Option<PathBuf>, random: Vec<PathBuf>) {
        if lock(&self.shared.playback).source.is_remote() {
            return;
        }
        let job = self.shared.covers_job.fetch_add(1, Ordering::Relaxed) + 1;
        let shared = Arc::clone(&self.shared);
        let spawned = thread::Builder::new()
            .name("visualizer-covers".into())
            .spawn(move || {
                let atlas = covers::build(current.as_ref(), &random).map(Arc::new);
                if shared.covers_job.load(Ordering::Relaxed) != job {
                    return;
                }
                let mut state = lock(&shared.covers);
                state.atlas = atlas;
                state.version += 1;
            });
        if let Err(e) = spawned {
            error!("Visualizer: no thread for the covers: {e}");
        }
    }

    /// The track playing changed: its cover file is `path` (none if it has none). Only cell 0 of
    /// the atlas changes, the random covers stay. The file is decoded on a background thread and
    /// the scene picks it up with the next frame; a later call replaces an earlier one.
    pub fn set_playing_cover(&self, path: Option<PathBuf>) {
        if lock(&self.shared.playback).source.is_remote() {
            return;
        }
        let job = self
            .shared
            .playing_cover_job
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        let shared = Arc::clone(&self.shared);
        let spawned = thread::Builder::new()
            .name("visualizer-cover".into())
            .spawn(move || {
                let cell = Arc::new(covers::build_cell(path.as_ref()));
                if shared.playing_cover_job.load(Ordering::Relaxed) != job {
                    return;
                }
                let mut state = lock(&shared.playing_cover);
                state.cell = Some(cell);
                state.version += 1;
            });
        if let Err(e) = spawned {
            error!("Visualizer: no thread for the playing cover: {e}");
        }
    }

    /// Follow the player: while the music is paused or stopped the visualizer draws nothing and
    /// its clock stops, so scenes resume where they were.
    pub fn set_playing(&self, playing: bool) {
        self.shared.playing.store(playing, Ordering::Relaxed);
    }

    /// Follow the player's file source and position (call it with every progress report). The file
    /// is analysed in the background; once that is done, scenes get
    /// the beats, bars and sections from the chart.
    pub fn set_playback(
        &self,
        path: Option<&Path>,
        position: Duration,
        playing: bool,
        fsio: Option<Arc<FsIo>>,
    ) {
        let mut playback = lock(&self.shared.playback);
        let leaving_feed = playback.source.is_remote();
        let previous_path = match &playback.source {
            ChartSource::LocalFile { path, .. } => Some(path.as_path()),
            ChartSource::None | ChartSource::Remote { .. } | ChartSource::RemoteWithoutItem => None,
        };
        let track_changed = previous_path != path;
        if track_changed {
            playback.generation += 1;
            *lock(&self.shared.feed_spectrum) = None;
        }
        playback.source = match path {
            Some(path) => ChartSource::LocalFile {
                path: path.to_path_buf(),
                fsio,
            },
            None => ChartSource::None,
        };
        playback.position = position;
        playback.playing = playing;
        playback.at = Some(Instant::now());
        drop(playback);
        if leaving_feed {
            *lock(&self.shared.feed_covers) = None;
            self.shared.covers_job.fetch_add(1, Ordering::Relaxed);
            self.shared
                .playing_cover_job
                .fetch_add(1, Ordering::Relaxed);
            {
                let mut covers = lock(&self.shared.covers);
                covers.atlas = None;
                covers.version += 1;
            }
            {
                let mut playing_cover = lock(&self.shared.playing_cover);
                playing_cover.cell = None;
                playing_cover.version += 1;
            }
        }
        if leaving_feed
            && self.wants_covers()
            && let Some(request) = lock(&self.shared.covers_requester).clone()
        {
            request();
        }
    }

    /// Follow playback on a remote renderer without inspecting its filesystem path.
    pub fn set_feed_playback(&self, item: Option<ChartItem>, position: Duration, playing: bool) {
        let mut playback = lock(&self.shared.playback);
        let chart = match (&playback.source, &item) {
            (
                ChartSource::Remote {
                    item: current,
                    chart,
                },
                Some(next),
            ) if current == next => chart.clone(),
            _ => None,
        };
        let source = item.map_or(ChartSource::RemoteWithoutItem, |item| ChartSource::Remote {
            item,
            chart,
        });
        let track_changed = !playback.source.same_item(&source);
        if track_changed {
            *lock(&self.shared.feed_spectrum) = None;
        }
        playback.source = source;
        playback.position = position;
        playback.playing = playing;
        playback.at = Some(Instant::now());
        let current_item = match &playback.source {
            ChartSource::Remote { item, .. } => Some(item.clone()),
            ChartSource::None | ChartSource::LocalFile { .. } | ChartSource::RemoteWithoutItem => {
                None
            }
        };
        drop(playback);
        if track_changed {
            self.shared.covers_job.fetch_add(1, Ordering::Relaxed);
            self.shared
                .playing_cover_job
                .fetch_add(1, Ordering::Relaxed);
            {
                let mut covers = lock(&self.shared.covers);
                covers.atlas = None;
                covers.version += 1;
            }
            {
                let mut playing_cover = lock(&self.shared.playing_cover);
                playing_cover.cell = None;
                playing_cover.version += 1;
            }
        }
        if lock(&self.shared.feed_covers)
            .as_ref()
            .is_some_and(|covers| Some(&covers.item) != current_item.as_ref())
        {
            *lock(&self.shared.feed_covers) = None;
        }
        self.shared.playing.store(playing, Ordering::Relaxed);
    }

    /// A realtime spectrum frame reached this visualizer from the player feed.
    pub fn set_feed_spectrum(&self, values: &[f32]) {
        if values.is_empty() || values.len() > 4096 {
            return;
        }
        *lock(&self.shared.feed_spectrum) = Some(values.to_vec());
    }

    /// Record what the current source can provide and replace an unsupported scene.
    pub fn set_inputs(&self, inputs: VisualizerInputs) {
        let mut library = lock(&self.library);
        let replacement = update_inputs(
            &self.shared,
            &mut library.folder,
            inputs,
            InputUpdateCause::SourceChanged,
        );
        if let Some(preset) = replacement {
            drop(library);
            let _ = self.load_entry(&preset);
        }
    }

    /// Give the visualizer a chart from the remote renderer for its current item.
    pub fn set_feed_chart(&self, item: ChartItem, bytes: &[u8]) -> anyhow::Result<()> {
        let chart = Arc::new(Chart::decode(bytes)?);
        let mut playback = lock(&self.shared.playback);
        if let ChartSource::Remote {
            item: current,
            chart: current_chart,
        } = &mut playback.source
            && current == &item
        {
            *current_chart = Some(chart);
        }
        Ok(())
    }

    /// Return a completed chart retained for `path`, if a previous visualizer run analysed it.
    pub fn cached_chart(&self, path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
        let kept = lock(&self.shared.kept_chart);
        let Some((cached_path, chart)) = kept.as_ref() else {
            return Ok(None);
        };
        if cached_path != path {
            return Ok(None);
        }
        chart.encode().map(Some)
    }

    /// Whether the current renderer feed has installed the chart for `identity`.
    pub fn has_feed_chart(&self, item: &ChartItem) -> bool {
        matches!(
            &lock(&self.shared.playback).source,
            ChartSource::Remote { item: current, chart: Some(_) } if current == item
        )
    }

    /// Whether playback is currently owned by a remote renderer feed.
    pub fn is_feed_playback(&self) -> bool {
        lock(&self.shared.playback).source.is_remote()
    }

    /// Whether the encoded playing and wall covers for `identity` reached this visualizer.
    pub fn feed_covers_match(
        &self,
        item: &ChartItem,
        current: Option<&[u8]>,
        random: &[Vec<u8>],
    ) -> bool {
        lock(&self.shared.feed_covers)
            .as_ref()
            .is_some_and(|covers| {
                &covers.item == item
                    && covers.current.as_deref() == current
                    && covers.random == random
            })
    }

    /// Give the cover atlas for the current remote renderer item to the visualizer.
    pub fn set_feed_covers(
        &self,
        item: &ChartItem,
        current: Option<Vec<u8>>,
        random: Vec<Vec<u8>>,
    ) -> anyhow::Result<()> {
        const MAX_COVERS: usize = 63;
        const MAX_BYTES: usize = 16 * 1024 * 1024;
        let total_bytes = current
            .iter()
            .map(Vec::len)
            .chain(random.iter().map(Vec::len))
            .sum::<usize>();
        anyhow::ensure!(random.len() <= MAX_COVERS, "too many feed covers");
        anyhow::ensure!(total_bytes <= MAX_BYTES, "feed covers exceed byte limit");
        if !matches!(
            &lock(&self.shared.playback).source,
            ChartSource::Remote { item: current, .. } if current == item
        ) {
            return Ok(());
        }
        *lock(&self.shared.feed_covers) = Some(FeedCovers {
            item: item.clone(),
            current: current.clone(),
            random: random.clone(),
        });

        let atlas_job = self.shared.covers_job.fetch_add(1, Ordering::Relaxed) + 1;
        let shared = Arc::clone(&self.shared);
        let current_atlas = current.clone();
        let random_atlas = random.clone();
        thread::Builder::new()
            .name("visualizer-feed-covers".into())
            .spawn(move || {
                let atlas =
                    covers::build_from_bytes(current_atlas.as_deref(), &random_atlas).map(Arc::new);
                if shared.covers_job.load(Ordering::Relaxed) != atlas_job {
                    return;
                }
                let mut state = lock(&shared.covers);
                state.atlas = atlas;
                state.version += 1;
            })
            .map_err(anyhow::Error::from)?;

        let cover_job = self
            .shared
            .playing_cover_job
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        let shared = Arc::clone(&self.shared);
        thread::Builder::new()
            .name("visualizer-feed-cover".into())
            .spawn(move || {
                let cell = Arc::new(covers::build_cell_from_bytes(current.as_deref()));
                if shared.playing_cover_job.load(Ordering::Relaxed) != cover_job {
                    return;
                }
                let mut state = lock(&shared.playing_cover);
                state.cell = Some(cell);
                state.version += 1;
            })
            .map_err(anyhow::Error::from)?;
        Ok(())
    }

    /// Follow the app's window: while it is minimised, hidden or covered, the visualizer stands
    /// still like it does while the music is paused, and picks up where it was when it shows
    /// again.
    pub fn set_visible(&self, visible: bool) {
        self.shared.visible.store(visible, Ordering::Relaxed);
    }

    /// Read frames back even on Apple, Windows and Linux, where they normally go into
    /// IOSurface-backed pixel buffers, shared textures or shared images. Applies from the next
    /// [`Self::start`].
    pub fn prefer_readback(&self, readback: bool) {
        self.shared
            .prefer_readback
            .store(readback, Ordering::Relaxed);
    }

    /// The adapter Flutter renders on, by the parts of its DXGI LUID (`None` if unknown). On
    /// Windows frames go to Flutter in shared textures, which only works on that adapter: with it
    /// the visualizer renders there, without it frames are read back. Elsewhere it has no
    /// effect. Applies from the next [`Self::start`].
    pub fn set_graphics_adapter(&self, luid: Option<(u32, i32)>) {
        let luid = luid.map_or(0, |(low, high)| adapter_luid(low, high));
        self.shared.adapter_luid.store(luid, Ordering::Relaxed);
    }

    /// The GPU Flutter's OpenGL context runs on, by its device and driver UUIDs (`None` if unknown
    /// or if that context cannot import Vulkan memory). On Linux frames go to Flutter in Vulkan
    /// images whose memory OpenGL imports, which only works on that GPU and driver: with it the
    /// visualizer renders there, without it frames are read back. Elsewhere it has no effect.
    /// Applies from the next [`Self::start`].
    pub fn set_graphics_device(&self, uuids: Option<DeviceUuids>) {
        *lock(&self.shared.device_uuids) = uuids;
    }

    /// Read frames back from the next frame on, until the render thread stops: the app could not
    /// show a frame it was handed in a shared texture or image (or could not register a texture
    /// for them).
    pub fn fall_back_to_readback(&self) {
        self.shared.fall_back.store(true, Ordering::Relaxed);
    }

    /// Android: the window the render thread presents frames into, from the next [`Self::start`]
    /// (`None`: no window, so frames are read back and nothing shows them). The render thread
    /// takes it and holds it until it stops or is given another one ([`Self::replace_window`]).
    #[cfg(target_os = "android")]
    pub fn set_window(&self, window: Option<Window>) {
        *lock(&self.shared.window) = window;
    }

    /// Android: present frames of `width` by `height` into `window` from now on, in place of the
    /// running render thread's window, which it lets go of first.
    #[cfg(target_os = "android")]
    pub fn replace_window(&self, window: Window, width: u32, height: u32) -> Result<()> {
        self.send(Command::ReplaceWindow(window, width, height))
    }

    /// Start the render thread. It returns at once; a missing GPU or an engine error shows up
    /// in [`Status::last_error`] with `running` false. Fails if a thread is already there, so
    /// call [`Self::stop`] first (it gives the `PcmInput` back).
    pub fn start(&self, input: PcmInput, width: u32, height: u32, fps: u32) -> Result<()> {
        let (width, height, fps) = (width.max(1), height.max(1), fps.clamp(1, 240));
        let mut worker = lock(&self.worker);
        if worker.is_some() {
            bail!("The visualizer is already started");
        }
        let preset_count = self.shared.status().preset_count;
        let inputs = lock(&self.library).folder.available_inputs();
        {
            let mut status = self.shared.status();
            *status = Status {
                running: true,
                width,
                height,
                target_fps: fps,
                preset_count,
                inputs,
                ..Status::default()
            };
        }
        self.shared.fall_back.store(false, Ordering::Relaxed);
        let (commands, receiver) = mpsc::channel();
        let shared = Arc::clone(&self.shared);
        let library = Arc::clone(&self.library);
        let handle = thread::Builder::new()
            .name("visualizer".into())
            .spawn(move || worker_main(shared, library, input, (width, height, fps), receiver))
            .context("Failed to spawn the visualizer thread")?;
        *worker = Some(Worker { commands, handle });
        Ok(())
    }

    /// Stop the render thread and hand back the PCM input so that it can be reused.
    pub fn stop(&self) -> Option<PcmInput> {
        let worker = lock(&self.worker).take()?;
        let _ = worker.commands.send(Command::Stop);
        let input = worker.handle.join().ok();
        let frames = lock(&self.shared.frames).clear();
        drop(frames);
        self.shared.taken_age.store(0, Ordering::Relaxed);
        input
    }

    pub fn resize(&self, width: u32, height: u32) -> Result<()> {
        self.send(Command::Resize(width, height))
    }

    /// Show the scene in the folder `path`, which is watched for changes. The scene is built in
    /// the background: this returns at once, the old scene stays on screen until the new one is
    /// built, and a scene that does not load shows up in [`Status::last_error`] then.
    pub fn load_preset_path(&self, path: &Path) -> Result<()> {
        self.load_entry(&Preset::from_path(path))
    }

    /// Show the scene at `location`: a folder (watched for changes) or a built-in scene, which
    /// becomes the newest of the history (what a visualizer that starts resumes). Built in the
    /// background like [`Visualizer::load_preset_path`]; a newer request replaces one that is
    /// still being built.
    pub fn load_preset(&self, location: Location) -> Result<()> {
        let mut library = lock(&self.library);
        anyhow::ensure!(
            library.folder.supports_source_inputs(&location),
            PresetLoadError::InputsUnavailable
        );
        library.folder.remember_at(&location);
        drop(library);
        self.send(Command::LoadScene(location))
    }

    /// Show a scene the history already has.
    fn load_entry(&self, preset: &Preset) -> Result<()> {
        self.send(Command::LoadScene(preset.location.clone()))
    }

    /// Scan the folders `dirs` and their subfolders for scenes (folders with a `scene.toml`) for
    /// next/previous/random, replacing the earlier folders and emptying the history. A scene
    /// folder named like a built-in scene is listed beside it. Returns the number of scenes to
    /// choose from, the built-in ones included. A folder that cannot be read is skipped and shows
    /// up in [`Status::last_error`]. A running visualizer whose scene is no longer in the folders
    /// changes to the first built-in scene.
    pub fn set_preset_dirs(&self, dirs: &[PathBuf]) -> Result<usize> {
        let mut library = lock(&self.library);
        let (count, errors) = library.folder.scan_all(dirs);
        self.shared.status().preset_count = count;
        if !errors.is_empty() {
            let message: Vec<String> = errors.iter().map(|e| format!("{e:#}")).collect();
            self.shared.set_error("scan_scenes", message.join("; "));
        }
        let inputs = library.folder.source_inputs();
        let replacement = update_inputs(
            &self.shared,
            &mut library.folder,
            inputs,
            InputUpdateCause::SceneListChanged,
        );
        // Not running: `start` begins with a scene that is listed.
        let current = Location::parse(&self.shared.status().preset_path);
        let not_running = !self.shared.status().running;
        if replacement.is_none()
            && not_running
            && matches!(current, Location::Folder(_))
            && !library.folder.contains(&current)
        {
            let _ = self.send(Command::LoadDefaultScene);
        }
        if let Some(preset) = replacement {
            drop(library);
            let _ = self.load_entry(&preset);
        }
        Ok(count)
    }

    /// Make the app's cover wall one of the scenes (see [`Location::CoverWall`]): picked like a
    /// scene, it stops the drawing until another scene comes.
    pub fn offer_cover_wall(&self) {
        let mut library = lock(&self.library);
        library.folder.set_cover_wall(true);
        self.shared.status().preset_count = library.folder.len();
    }

    /// How the scenes are picked, replacing what was set before: switches go through them in
    /// `order`, leaving out those at `disabled`, which are still listed and can be loaded by name.
    /// Scenes not in `order` come after those that are.
    pub fn set_scene_choice(&self, order: Vec<Location>, disabled: Vec<Location>) {
        let mut library = lock(&self.library);
        library.folder.set_order(order);
        library.folder.set_disabled(disabled);
    }

    /// The scene on screen when the app last ran, which a visualizer that starts resumes unless
    /// another scene was shown since the app started.
    pub fn resume_from(&self, location: Location) {
        lock(&self.library).folder.resume_from(&location);
    }

    /// Every scene, by group and name, for choosing one.
    pub fn list_presets(&self) -> Vec<PresetInfo> {
        lock(&self.library).folder.infos()
    }

    /// The next scene in the order after the one on screen. Nothing happens without scenes.
    pub fn next_preset(&self) -> Result<()> {
        let mut library = lock(&self.library);
        if library.folder.len() == 0 {
            return Ok(());
        }
        let current = Location::parse(&self.shared.status().preset_path);
        let preset = library.folder.next_other(&current)?;
        drop(library);
        self.load_entry(&preset)
    }

    /// One step back in the history of loaded scenes; nothing happens at its start or without
    /// scenes.
    pub fn previous_preset(&self) -> Result<()> {
        let mut library = lock(&self.library);
        if library.folder.len() == 0 {
            return Ok(());
        }
        let preset = library.folder.previous()?;
        drop(library);
        match preset {
            Some(preset) => self.load_entry(&preset),
            None => Ok(()),
        }
    }

    /// A new random scene; the history ahead of the current one is dropped. Nothing happens
    /// without scenes.
    pub fn random_preset(&self) -> Result<()> {
        let mut library = lock(&self.library);
        if library.folder.len() == 0 {
            return Ok(());
        }
        let preset = library.folder.random(&mut rand::thread_rng())?;
        drop(library);
        self.load_entry(&preset)
    }

    /// Record an error of `operation` that happened outside the render thread, for
    /// [`Status::last_error`] and the error listener.
    pub fn report_error(&self, operation: &'static str, message: String) {
        self.shared.set_error(operation, message);
    }

    pub fn status(&self) -> Status {
        self.shared.status().clone()
    }

    /// The most recent frame, if the visualizer is running and has rendered one.
    pub fn latest_frame(&self) -> Option<Arc<Frame>> {
        lock(&self.shared.frames).newest()
    }

    /// The frame to show next: the oldest frame that was not handed out here yet, else the one
    /// handed out last again (none before the first frame). A reader that calls this once per
    /// display refresh shows every frame even when two were finished within one refresh (see
    /// [`published`]); while another frame waits, the frame listener is told about it again.
    pub fn next_frame(&self) -> Option<Arc<Frame>> {
        let taken = lock(&self.shared.frames).take(Instant::now());
        // Dropped outside the lock: releasing a readback buffer unmaps it.
        drop(taken.replaced);
        if let Some(age) = taken.age {
            let nanos = u64::try_from(age.as_nanos()).unwrap_or(u64::MAX - 1);
            self.shared.taken_age.store(nanos + 1, Ordering::Relaxed);
        }
        if let Some(next) = taken.next {
            let listener = lock(&self.shared.frame_listener).clone();
            if let Some(listener) = listener {
                listener(next.id);
            }
        }
        taken.frame
    }

    /// `listener` is called with the id of every new frame, from the render thread right after
    /// [`Self::latest_frame`] returns it, so it must not block; `None` stops the calls. It
    /// replaces an earlier listener and stays across restarts.
    pub fn set_frame_listener(&self, listener: Option<FrameListener>) {
        *lock(&self.shared.frame_listener) = listener;
    }

    /// `listener` is called with the transport once the render thread has chosen it, and again
    /// whenever it changes (frames fall back to being read back), from the render thread: it must
    /// not block. `None` stops the calls. It replaces an earlier listener and stays across
    /// restarts; set it before [`Self::start`] to see the first call, or read
    /// [`Status::transport`] after setting it.
    pub fn set_transport_listener(&self, listener: Option<TransportListener>) {
        *lock(&self.shared.transport_listener) = listener;
    }

    /// Tell `listener` every error from now on, with what was being done (see [`ErrorListener`]).
    pub fn set_error_listener(&self, listener: Option<ErrorListener>) {
        *lock(&self.shared.error_listener) = listener;
    }

    fn send(&self, command: Command) -> Result<()> {
        let worker = lock(&self.worker);
        let worker = worker.as_ref().context("The visualizer is not running")?;
        worker
            .commands
            .send(command)
            .ok()
            .context("The visualizer is not running")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputUpdateCause {
    SourceChanged,
    SceneListChanged,
}

fn update_inputs(
    shared: &Shared,
    folder: &mut PresetList,
    inputs: VisualizerInputs,
    cause: InputUpdateCause,
) -> Option<Preset> {
    let inputs_changed = folder.set_inputs(inputs);
    let available = folder.available_inputs();
    let (status_changed, running, current) = {
        let mut status = shared.status();
        let changed = status.inputs != available;
        status.inputs = available;
        (
            changed,
            status.running,
            Location::parse(&status.preset_path),
        )
    };
    if !running
        || !(inputs_changed || status_changed || cause == InputUpdateCause::SceneListChanged)
    {
        return None;
    }
    if !folder.supports_source_inputs(&current) {
        return Some(
            folder
                .next_other(&current)
                .unwrap_or_else(|_| Preset::cover_wall()),
        );
    }
    if current == Location::CoverWall && available != VisualizerInputs::None {
        let previous = if inputs == VisualizerInputs::Full {
            folder.previous().ok().flatten()
        } else {
            None
        };
        return previous.or_else(|| folder.next_other(&current).ok());
    }
    None
}

/// A DXGI LUID as one number: the high part (as its bits) above the low part.
fn adapter_luid(low: u32, high: i32) -> u64 {
    (u64::from(high as u32) << 32) | u64::from(low)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn worker_main(
    shared: Arc<Shared>,
    library: Arc<Mutex<Library>>,
    mut input: PcmInput,
    (width, height, fps): (u32, u32, u32),
    commands: Receiver<Command>,
) -> PcmInput {
    let result = catch_unwind(AssertUnwindSafe(|| {
        run(
            &shared,
            &library,
            &mut input,
            (width, height, fps),
            &commands,
        )
    }));
    let failure = match result {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(format!("{e:#}")),
        Err(_) => Some("The visualizer thread panicked".to_string()),
    };
    if let Some(message) = failure {
        shared.set_error("run", message);
    }
    shared.status().running = false;
    input
}

fn run(
    shared: &Arc<Shared>,
    library: &Mutex<Library>,
    input: &mut PcmInput,
    (width, height, fps): (u32, u32, u32),
    commands: &Receiver<Command>,
) -> Result<()> {
    let sample_rate = match input.sample_rate.load(Ordering::Relaxed) {
        0 => FALLBACK_SAMPLE_RATE,
        rate => rate,
    };
    let on_gpu_error = {
        let shared = Arc::clone(shared);
        Arc::new(move |message: String| shared.set_error("gpu", message))
    };
    let readback = shared.prefer_readback.load(Ordering::Relaxed);
    let adapter_luid = match shared.adapter_luid.load(Ordering::Relaxed) {
        0 => None,
        luid => Some(luid),
    };
    let device_uuids = *lock(&shared.device_uuids);
    let mut pipeline = Pipeline::new(
        width,
        height,
        sample_rate,
        readback,
        adapter_luid,
        device_uuids,
        on_gpu_error,
        #[cfg(target_os = "android")]
        lock(&shared.window).take(),
    )?;
    pipeline.set_target_fps(fps);
    let mut transport = pipeline.transport();
    info!(
        "Visualizer started on {} ({transport:?})",
        pipeline.adapter_name
    );
    {
        let mut status = shared.status();
        status.adapter = pipeline.adapter_name.clone();
        status.transport = Some(transport);
    }
    notify_transport(shared, transport);

    let interval = Duration::from_secs_f64(1.0 / f64::from(fps));
    let started = Instant::now();
    let mut deadline = started;
    let mut pcm: Vec<f32> = Vec::new();
    let mut window_start = started;
    let mut window_frames = 0u32;
    // Scenes being built on other threads (see [`loading`]).
    let mut loads = Loads::new();
    // Time spent with the music paused or the window out of sight, which the visualizer's clock
    // leaves out.
    let mut frozen = Duration::ZERO;
    // While paused or out of sight one frame is drawn (the first one, or after a command such as
    // a resize) and then held.
    let mut still_drawn = false;
    // Where the player said the song was when the last frame was drawn: a seek while paused draws
    // the frame of the new place.
    let mut drawn_position: Option<Duration> = None;
    // The folder of the scene on screen, reloaded when it is saved (live editing).
    let mut watched: Option<Watched> = None;
    let mut tracker = BeatTracker::new(sample_rate);
    let mut feed_spectrum_active = false;
    let mut charts = ChartFollower::new(Arc::clone(&shared.kept_chart));
    let mut planner = Planner::default();
    // The version of the atlas of covers the pipeline has.
    let mut covers_seen: Option<u64> = None;
    // The same for the cell of the track playing.
    let mut playing_cover_seen: Option<u64> = None;
    // Benchmark mode (see [`bench`]) shows every built-in scene in turn, whatever is asked for.
    let mut bench = bench::seconds_from_env().map(bench::Bench::new);
    // Whether nothing is drawn: the cover wall is on, or the scene to begin with is being built.
    let mut hidden = false;
    // The pipeline comes with the default scene. The visualizer begins with the scene shown last,
    // or without one with the first in the order. Another scene is shown at once when it is
    // built.
    // The status names no scene until one is on screen, so that the app does not take the default
    // scene for the one it resumes.
    let begin = Location::Builtin(pipeline.preset_name().to_string());
    if bench.is_none() {
        let (first, begin_supported) = {
            let mut library = lock(library);
            let first = library.folder.resume();
            let begin_supported = library.folder.supports_source_inputs(&begin);
            (first, begin_supported)
        };
        if let Some(preset) = first.filter(|preset| preset.location != begin) {
            hidden = true;
            start_scene_load(shared, &pipeline, &mut loads, preset.location, AT_ONCE);
        } else if !begin_supported {
            hidden = true;
            start_scene_load(shared, &pipeline, &mut loads, Location::CoverWall, AT_ONCE);
        }
    }
    if !hidden {
        show_pipeline_scene(shared, &pipeline);
    }

    loop {
        loop {
            match commands.try_recv() {
                Ok(Command::Stop) | Err(TryRecvError::Disconnected) => return Ok(()),
                Ok(command) => apply(
                    shared,
                    library,
                    &mut pipeline,
                    &mut loads,
                    &mut hidden,
                    command,
                ),
                Err(TryRecvError::Empty) => break,
            }
        }

        let channels = match input.channels.load(Ordering::Relaxed) {
            0 => 2,
            n => n as usize,
        };
        drain(&mut input.consumer, &mut pcm);
        // The tracker hears every sample, also while the window is out of sight, so it keeps
        // the beat for when it is back.
        match input.sample_rate.load(Ordering::Relaxed) {
            0 => {}
            rate if rate != tracker.sample_rate() => tracker = BeatTracker::new(rate),
            _ => {}
        }
        tracker.push(&pcm, channels);
        keep_recent(&mut pcm, channels, sample_rate);
        match lock(&shared.feed_spectrum).clone() {
            Some(spectrum) => {
                pipeline.set_feed_spectrum(&spectrum);
                feed_spectrum_active = true;
            }
            None if feed_spectrum_active => {
                pipeline.clear_feed_spectrum();
                feed_spectrum_active = false;
            }
            None => {}
        }

        // A scene that finished building replaces the one on screen, also while paused: then at
        // once, without a transition.
        if let Some(loaded) = loads.take() {
            let paused = bench.is_none()
                && (!shared.playing.load(Ordering::Relaxed)
                    || !shared.visible.load(Ordering::Relaxed));
            install(
                shared,
                &mut pipeline,
                &mut watched,
                &mut hidden,
                loaded,
                paused,
            );
            still_drawn = false;
        }

        if let Some(file) = &mut watched
            && file.reload_if_saved(shared, &mut pipeline)
        {
            // Show the new version even while the music is paused.
            still_drawn = false;
        }

        shared
            .wants_covers
            .store(pipeline.wants_covers(), Ordering::Relaxed);
        {
            let state = lock(&shared.covers);
            if covers_seen != Some(state.version) {
                covers_seen = Some(state.version);
                pipeline.set_covers(state.atlas.clone());
            }
        }
        {
            let state = lock(&shared.playing_cover);
            if playing_cover_seen != Some(state.version) {
                playing_cover_seen = Some(state.version);
                pipeline.set_playing_cover(state.cell.clone());
            }
        }

        // A chart takes over the tracker's clock and musical signals when it is ready.
        let chart = charts.tick(&lock(&shared.playback), Instant::now());

        // The benchmark draws with the music paused or the window out of sight too.
        let paused = bench.is_none()
            && (!shared.playing.load(Ordering::Relaxed) || !shared.visible.load(Ordering::Relaxed));
        // A scene switch that began while the music played runs its transition to the end first,
        // so that the new scene does not wait for the music to play again.
        if paused && still_drawn && !pipeline.switching() {
            // Keep the last frame on screen and wait, for commands, the music or the window.
            let since = Instant::now();
            // The frame drawn last may still be on its way.
            show_finished(shared, &mut pipeline, since + PAUSED_POLL)?;
            match commands.recv_timeout(PAUSED_POLL) {
                Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
                Ok(command) => {
                    apply(
                        shared,
                        library,
                        &mut pipeline,
                        &mut loads,
                        &mut hidden,
                        command,
                    );
                    still_drawn = false;
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            // The song was sought while paused (in sight): draw where it is now.
            let position = lock(&shared.playback).position;
            if shared.visible.load(Ordering::Relaxed)
                && drawn_position.is_none_or(|drawn| position.abs_diff(drawn) > SOUGHT)
            {
                still_drawn = false;
            }
            frozen += since.elapsed();
            deadline = Instant::now();
            window_start = deadline;
            window_frames = 0;
            continue;
        }
        still_drawn = paused;
        drawn_position = Some(lock(&shared.playback).position);
        if !pcm.is_empty() && !hidden {
            pipeline.push_pcm(&pcm, channels);
        }

        let time = (started.elapsed() - frozen).as_secs_f64();
        if hidden {
            // Nothing to draw: wait for commands and the next song with the clock standing
            // still, as while the music is paused.
            let since = Instant::now();
            match commands.recv_timeout(PAUSED_POLL) {
                Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
                Ok(command) => apply(
                    shared,
                    library,
                    &mut pipeline,
                    &mut loads,
                    &mut hidden,
                    command,
                ),
                Err(RecvTimeoutError::Timeout) => {}
            }
            frozen += since.elapsed();
            deadline = Instant::now();
            window_start = deadline;
            window_frames = 0;
            continue;
        }

        // A lab may hold the music at a lower fallback level than the song has.
        let chart = bind::forced_chart(pipeline.lab().level, chart);
        let rhythm = chart::trusted_rhythm(tracker.rhythm(), chart.as_ref());
        pipeline.set_rhythm(&rhythm);
        pipeline.set_chart(chart.as_ref());
        // With the song's chart (and no lab holding the music at a level), scenes decide by song
        // time, with the seed the song is planned with.
        let song_seed = charts
            .path()
            .filter(|path| !path.as_os_str().is_empty())
            .map(|path| scene::song_seed(&path.display().to_string()));
        let song_chart = charts
            .chart()
            .filter(|_| pipeline.lab().level.is_none())
            .zip(song_seed)
            .map(|((_, chart, _), seed)| scene::SongChart { chart, seed });
        pipeline.set_song_chart(song_chart);
        pipeline.set_song_seed(song_seed);
        pipeline.set_song(charts.song_time(), shared.playing.load(Ordering::Relaxed));
        pipeline.set_accent(match shared.accent.load(Ordering::Relaxed) {
            0 => None,
            argb => Some(scene::argb_to_linear(argb)),
        });
        // How often the music's moments play an effect: the user's setting, or a lab's chance.
        let chances = shared.effect_chances();
        pipeline.set_effect_chances(chances);
        if shared.fall_back.swap(false, Ordering::Relaxed) {
            pipeline.fall_back_to_readback();
        }
        if let Some(bench) = &mut bench
            && bench.step(&mut pipeline, Instant::now())
        {
            show_pipeline_scene(shared, &pipeline);
        }
        pipeline.record_events(shared.inspecting.load(Ordering::Relaxed));
        let rendered = pipeline.render(time)?;
        let age = match shared.taken_age.swap(0, Ordering::Relaxed) {
            0 => None,
            nanos => Some(Duration::from_nanos(nanos - 1)),
        };
        if let Some(bench) = &mut bench {
            let taken = lock(&shared.frames).count();
            bench.frame(&mut pipeline, Instant::now(), taken, age);
        }
        show(shared, &pipeline, rendered);
        let events = pipeline.take_events();
        shared.log_events(events);
        let inspecting = shared.inspecting.load(Ordering::Relaxed);
        if inspecting {
            let song = lock(&shared.playback).generation;
            let (scene, runtime, edits) = pipeline.lab_key();
            lock(&shared.lab).follow((scene, runtime, edits, song), || pipeline.lab_scene());
        }
        planner.update(shared, &pipeline, &charts);
        if inspecting {
            let seen = inspection(shared, &pipeline, &charts, time);
            lock(&shared.lab).record(
                pipeline.song_time(),
                &seen,
                bind::Level::of(pipeline.music()),
            );
            *lock(&shared.inspect) = Some(seen);
        }
        // The output falls back to reading frames back if pixel buffers, shared textures or shared
        // images stop working, or the app could not show them.
        if pipeline.transport() != transport {
            transport = pipeline.transport();
            info!("Visualizer frames now leave the GPU by {transport:?}");
            shared.status().transport = Some(transport);
            notify_transport(shared, transport);
        }

        window_frames += 1;
        let elapsed = window_start.elapsed();
        if elapsed >= FPS_WINDOW {
            shared.status().measured_fps = window_frames as f32 / elapsed.as_secs_f32();
            window_start = Instant::now();
            window_frames = 0;
        }

        // Wait for the next frame: first for the frames still being drawn, to show each as soon as
        // the GPU has finished it, then for commands.
        deadline += interval;
        // Publish frames a little before Flutter takes them (see [`published`]).
        if let Some(age) = age {
            let step = published::phase_step(age, interval);
            let shift = Duration::from_secs_f64(step.abs());
            deadline = if step >= 0.0 {
                deadline + shift
            } else {
                deadline.checked_sub(shift).unwrap_or(deadline)
            };
        }
        show_finished(shared, &mut pipeline, deadline)?;
        let now = Instant::now();
        // A frame that is late by less than one interval starts the next one at once, so that
        // frames keep the target's rate on average. Counting again from a late frame made every
        // frame that late: a wait for the GPU ends a little after its deadline more often than
        // not, and on an iPhone that alone cost 52 fps instead of 60.
        if deadline > now {
            match commands.recv_timeout(deadline - now) {
                Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
                Ok(command) => apply(
                    shared,
                    library,
                    &mut pipeline,
                    &mut loads,
                    &mut hidden,
                    command,
                ),
                Err(RecvTimeoutError::Timeout) => {}
            }
        } else if now - deadline >= interval {
            // Rendering is slower than the target: do not try to catch up.
            deadline = now;
        }
    }
}

/// A scene built on another thread, with what the render loop does once it is on screen.
/// How a scene switch looks: the transition and how long it takes.
#[derive(Clone, Copy)]
struct Switch(Transition, Durations);

/// A scene the user chose (a command to load one): a short fade through black, 0.25 s out and
/// 0.25 s in, quick enough to feel like an answer to the click.
const MANUAL_SWITCH: Switch = Switch(
    Transition::Fade,
    Durations {
        out: 0.25,
        into: 0.25,
    },
);

/// A cut: how a scene replaces the cover wall, or nothing at the start.
const AT_ONCE: Switch = Switch(
    Transition::Cut,
    Durations {
        out: 0.0,
        into: 0.0,
    },
);

struct Loaded {
    /// How to show it.
    switch: Switch,
    /// Where the scene is from; a folder is watched for changes.
    location: Location,
    /// None for the cover wall, which has nothing to build.
    scene: Result<Option<Scene>>,
}

/// Start building the scene `build` makes, which replaces any load still running. The scene on
/// screen stays until it is built (see [`install`]). `location` is where it is from.
fn start_load(
    shared: &Shared,
    loads: &mut Loads<Loaded>,
    location: Location,
    switch: Switch,
    build: Result<SceneBuild>,
) {
    let started = build.and_then(|build| {
        loads
            .start(move || Loaded {
                switch,
                location,
                scene: build.run().map(Some),
            })
            .context("Failed to start a thread to build the scene")
    });
    if let Err(e) = started {
        shared.set_error("load_scene", format!("{e:#}"));
    }
}

/// Show a scene that finished building, or its error (the scene on screen stays then). The cover
/// wall hides the scene on screen; the scene after it shows at once, and so does a scene while the
/// music is `paused`.
fn install(
    shared: &Shared,
    pipeline: &mut Pipeline,
    watched: &mut Option<Watched>,
    hidden: &mut bool,
    loaded: Loaded,
    paused: bool,
) {
    let switched = loaded.scene.and_then(|scene| {
        let Some(scene) = scene else {
            *hidden = true;
            return Ok(());
        };
        let Switch(transition, durations) = if *hidden || paused {
            AT_ONCE
        } else {
            loaded.switch
        };
        pipeline.switch_to(scene, transition, durations)?;
        *hidden = false;
        Ok(())
    });
    match switched {
        Ok(()) if loaded.location == Location::CoverWall => {
            *watched = None;
            let mut status = shared.status();
            status.preset_name = Preset::cover_wall().name;
            status.preset_path = loaded.location.to_string();
            status.last_error = None;
        }
        Ok(()) => {
            *watched = match &loaded.location {
                Location::Folder(dir) => Some(Watched::new(dir.clone())),
                Location::Builtin(_) | Location::CoverWall => None,
            };
            {
                let mut status = shared.status();
                status.preset_name = pipeline.preset_name().to_string();
                status.preset_path = loaded.location.to_string();
            }
            let remote_feed = matches!(
                &lock(&shared.playback).source,
                ChartSource::Remote { .. } | ChartSource::RemoteWithoutItem
            );
            if pipeline.wants_covers() && !remote_feed {
                shared.wants_covers.store(true, Ordering::Relaxed);
                let request = lock(&shared.covers_requester).clone();
                if let Some(request) = request {
                    request();
                }
            }
            shared.status().last_error = None;
        }
        Err(e) => {
            // A scene that was to replace the one the visualizer began with failed: show that one
            // instead of nothing. The cover wall stays up.
            if shared.status().preset_path != Location::CoverWall.to_string() {
                if *hidden {
                    show_pipeline_scene(shared, pipeline);
                }
                *hidden = false;
            }
            shared.set_error("load_scene", format!("{e:#}"));
        }
    }
}

/// Name the built-in scene the pipeline draws in the status: the one it began with, or the
/// benchmark's.
fn show_pipeline_scene(shared: &Shared, pipeline: &Pipeline) {
    let mut status = shared.status();
    status.preset_name = pipeline.preset_name().to_string();
    status.preset_path = Location::Builtin(status.preset_name.clone()).to_string();
}

/// Start building the scene at `location`, to be shown through `switch` (see [`start_load`]).
fn start_scene_load(
    shared: &Shared,
    pipeline: &Pipeline,
    loads: &mut Loads<Loaded>,
    location: Location,
    switch: Switch,
) {
    match location {
        Location::Folder(dir) => {
            let build = Ok(pipeline.scene_build_dir(&dir));
            start_load(shared, loads, Location::Folder(dir), switch, build);
        }
        Location::Builtin(name) => {
            start_load(
                shared,
                loads,
                Location::Builtin(name.clone()),
                switch,
                pipeline.scene_build_builtin(&name),
            );
        }
        Location::CoverWall => {
            let started = loads.start(move || Loaded {
                switch,
                location: Location::CoverWall,
                scene: Ok(None),
            });
            if let Err(e) = started {
                shared.set_error(
                    "load_scene",
                    format!("Failed to start a thread for the cover wall: {e}"),
                );
            }
        }
    }
}

/// Carry out a command. A scene it loads is shown through [`MANUAL_SWITCH`].
fn apply(
    shared: &Shared,
    library: &Mutex<Library>,
    pipeline: &mut Pipeline,
    loads: &mut Loads<Loaded>,
    hidden: &mut bool,
    command: Command,
) {
    let switch = MANUAL_SWITCH;
    let result = match command {
        Command::Resize(width, height) => pipeline.resize(width, height).map(|()| {
            let (width, height) = pipeline.size();
            let mut status = shared.status();
            status.width = width;
            status.height = height;
        }),
        // Scenes are built on another thread; the loop shows them when they are done.
        Command::LoadScene(location) => {
            if location == Location::CoverWall {
                *hidden = true;
            }
            start_scene_load(shared, pipeline, loads, location, switch);
            return;
        }
        Command::LoadDefaultScene => {
            let build = pipeline.scene_build_builtin(pipeline::DEFAULT_PRESET_NAME);
            let location = Location::Builtin(pipeline::DEFAULT_PRESET_NAME.to_string());
            start_load(shared, loads, location, switch, build);
            return;
        }
        #[cfg(target_os = "android")]
        Command::ReplaceWindow(window, width, height) => {
            pipeline.replace_window(window, width, height).map(|()| {
                let (width, height) = pipeline.size();
                let mut status = shared.status();
                status.width = width;
                status.height = height;
            })
        }
        Command::Lab(command) => {
            lab(shared, library, pipeline, loads, command);
            return;
        }
        Command::Stop => Ok(()),
    };
    match result {
        Ok(()) => shared.status().last_error = None,
        Err(e) => shared.set_error("resize", format!("{e:#}")),
    }
}

/// [`Shared::lab_chance`] when no scene lab set a chance.
const NO_LAB_CHANCE: u8 = u8::MAX;

/// Carry out a scene lab's request. The scene a switch asks for is built in the background and
/// shown like one the user chose, but through the transition asked for, with the durations a
/// switch of that kind takes with the music (see [`switching::requested`]).
fn lab(
    shared: &Shared,
    library: &Mutex<Library>,
    pipeline: &mut Pipeline,
    loads: &mut Loads<Loaded>,
    command: LabCommand,
) {
    match command {
        LabCommand::Cut => pipeline.cut_now(),
        LabCommand::EffectChance(percent) => {
            let percent = percent.map_or(NO_LAB_CHANCE, |p| p.min(100));
            shared.lab_chance.store(percent, Ordering::Relaxed);
        }
        LabCommand::CutDressed(transition) => pipeline.cut_dressed(transition.into()),
        LabCommand::Flare => pipeline.start_flare(),
        LabCommand::Leak => pipeline.start_leak(),
        LabCommand::LensGlitch => pipeline.start_overlay(transition::Overlay::Glitch),
        LabCommand::LensMosaic => pipeline.start_overlay(transition::Overlay::Mosaic),
        LabCommand::Burst => pipeline.start_burst(),
        LabCommand::Shot(index) => {
            if !pipeline.hold_shot(index) {
                info!("The scene has no shot {index:?} to hold");
            }
        }
        LabCommand::Look(index) => {
            if !pipeline.hold_look(index) {
                info!("The scene has no look {index:?} to hold");
            }
        }
        LabCommand::CyclePace => pipeline.cycle_pace(),
        LabCommand::PaceWith(pace) => pipeline.set_pace(pace),
        LabCommand::SwitchWith(transition) => pipeline.set_lab_switch(transition),
        LabCommand::CutsWith(transition) => pipeline.set_lab_cuts(transition),
        LabCommand::BurstsWith(bursts) => pipeline.set_lab_bursts(bursts),
        LabCommand::LensWith(lens) => pipeline.set_lab_lens(lens),
        LabCommand::ForceLevel(level) => pipeline.set_lab_level(level),
        LabCommand::RuntimeBindings(text) => match bind::RuntimeBindings::parse(&text) {
            // Read in full before anything changes: a mistake leaves the old ones.
            Ok(runtime) => {
                pipeline.set_runtime(Arc::new(runtime));
                shared.status().last_error = None;
            }
            Err(e) => shared.set_error("lab", format!("{e:#}")),
        },
        LabCommand::SceneBindings(text) => match pipeline.set_scene_bindings(&text) {
            Ok(()) => shared.status().last_error = None,
            Err(e) => shared.set_error("lab", format!("{e:#}")),
        },
        LabCommand::NextScene(transition) => {
            let current = Location::parse(&shared.status().preset_path);
            let next = lock(library).next_after(&current);
            match next {
                Some(Ok(preset)) => {
                    let (transition, durations) = switching::replaced(
                        pipeline.lab().switch.map(Into::into),
                        switching::requested(transition.into(), pipeline.music()),
                        pipeline.music(),
                    );
                    let switch = Switch(transition, durations);
                    start_scene_load(shared, pipeline, loads, preset.location, switch);
                }
                Some(Err(e)) => shared.set_error("lab", format!("{e:#}")),
                None => info!("There is no scene to switch to"),
            }
        }
    }
}

/// What a plan of the song depends on: when it changes, the song is played through again.
#[derive(Clone, PartialEq)]
struct PlanKey {
    /// Which chart of the file (see [`ChartFollower::chart`]).
    chart: u64,
    /// The scene on screen, and how often the runtime's decisions were replaced.
    scene: u64,
    runtime: u64,
    /// How often the scene's bindings or parameters were replaced.
    edits: u64,
    /// Which catalogue the lab's series follow (`LabColumns::version`): a lab turned off and on
    /// again starts a new one, and the plan of the old one is not the plan of that.
    columns: u64,
    /// The level a lab holds the music at.
    level: Option<Level>,
    /// The chances, and what a lab holds or overrides in the scene's decisions.
    settings: scene::DeciderSettings,
}

/// Whether the plan kept for `old` must be dropped before the one for `new` is made: it is when
/// the lab's catalogue changed (another scene, song or edit, or the lab turned off and on), as
/// the old plan would then show series that no longer exist.
fn clears(old: Option<&PlanKey>, new: &PlanKey) -> bool {
    old.is_none_or(|old| old.columns != new.columns)
}

/// Keeps the plan of the song current (see [`plan`]): while a lab inspects and the song has a
/// chart, a change of what the plan depends on starts playing the song through on a thread of
/// its own, and the result is kept unless a newer change came meanwhile.
#[derive(Default)]
struct Planner {
    /// What the plan kept (or being made) is of.
    key: Option<PlanKey>,
}

impl Planner {
    fn update(&mut self, shared: &Arc<Shared>, pipeline: &Pipeline, charts: &ChartFollower) {
        // A lab holding the music at a level decides without the chart, which the plan cannot show.
        let chart = charts.chart().filter(|_| {
            shared.inspecting.load(Ordering::Relaxed) && pipeline.lab().level.is_none()
        });
        let Some((arrival, chart, path)) = chart else {
            if self.key.take().is_some() {
                shared.clear_plan();
            }
            return;
        };
        // The catalogue the series follow is made before the plan.
        let (columns, lab_scene) = {
            let lab = lock(&shared.lab);
            (lab.columns(), lab.scene())
        };
        let Some(lab) = columns
            .zip(lab_scene)
            .map(|(columns, scene)| plan::PlanLab { columns, scene })
        else {
            return;
        };
        let (scene, runtime, settings) = pipeline.plan_parts();
        let key = PlanKey {
            chart: arrival,
            scene,
            runtime,
            edits: pipeline.lab_key().2,
            columns: lab.columns.version,
            level: pipeline.lab().level,
            settings,
        };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        // The plan of another catalogue is not shown while the new one is made. A plan of the same
        // catalogue stays until the new one replaces it, so the app does not flicker.
        if clears(self.key.as_ref(), &key) {
            shared.clear_plan();
        }
        self.key = Some(key);
        let job = shared.plan_job.fetch_add(1, Ordering::Relaxed) + 1;
        let decider = pipeline.plan_decider();
        let path = path.display().to_string();
        // The scene's name, not where it is (`Status::preset_path`).
        let scene = pipeline.preset_name().to_string();
        let shared = Arc::clone(shared);
        let spawned = thread::Builder::new()
            .name("song plan".into())
            .spawn(move || {
                let start = Instant::now();
                let seed = scene::song_seed(&path);
                let wanted = || shared.plan_job.load(Ordering::Relaxed) == job;
                let Some(mut plan) = plan::plan_song(&chart, decider, seed, Some(&lab), wanted)
                else {
                    debug!("Planning {path} was superseded");
                    return;
                };
                debug!(
                    "Planned {path} in {:.2?}: {} cuts, {} looks, {} moments",
                    start.elapsed(),
                    plan.cuts.len(),
                    plan.looks.len(),
                    plan.moments.len()
                );
                plan.path = path;
                plan.scene = scene;
                shared.store_plan(job, plan);
            });
        if let Err(e) = spawned {
            warn!("Failed to start the song plan: {e}");
        }
    }
}

/// What the render thread saw in the frame it drew at `time`, for [`Visualizer::inspect`].
fn inspection(shared: &Shared, pipeline: &Pipeline, charts: &ChartFollower, time: f64) -> Inspect {
    Inspect {
        scene: pipeline.preset_name().to_string(),
        fps: shared.status().measured_fps,
        gpu_ms: pipeline.last_gpu_seconds().map(|s| (s * 1e3) as f32),
        music: InspectMusic::of(pipeline.music()),
        signals: InspectSignal::all(pipeline.music(), pipeline.bound_cut()),
        sections: charts.sections(),
        director: pipeline.director_view().map(InspectDirector::of),
        looks: pipeline.looks_view().map(InspectLooks::of),
        effects: InspectEffects::of(pipeline.effects()),
        lens: {
            let mut lens = InspectLens::from(pipeline.lens_view());
            let running = pipeline.overlay_running();
            lens.glitch_running = running == Some(transition::Overlay::Glitch);
            lens.mosaic_running = running == Some(transition::Overlay::Mosaic);
            lens
        },
        overrides: pipeline.lab(),
        bindings: pipeline
            .bindings()
            .into_iter()
            .map(InspectBinding::from)
            .collect(),
        inputs: pipeline.inputs(),
        runtime_bindings: pipeline.runtime().report(pipeline.music()),
        chance: match shared.lab_chance.load(Ordering::Relaxed) {
            NO_LAB_CHANCE => None,
            percent => Some(percent),
        },
        switching: InspectSwitching {
            transition: pipeline.transition_progress(time).map(|(kind, progress)| {
                InspectTransition {
                    kind: kind.into(),
                    progress,
                }
            }),
        },
    }
}

/// The folder of the scene on screen. Its files are looked at twice a second and the scene is
/// built again when they changed, so scenes can be written while watching them; a version that
/// does not load shows its error and the next save tries again.
struct Watched {
    dir: PathBuf,
    next_check: Instant,
}

impl Watched {
    /// How often the files are looked at: a few small files cost nothing.
    const INTERVAL: Duration = Duration::from_millis(500);

    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            next_check: Instant::now() + Self::INTERVAL,
        }
    }

    /// Whether the scene was loaded again.
    fn reload_if_saved(&mut self, shared: &Shared, pipeline: &mut Pipeline) -> bool {
        let now = Instant::now();
        if now < self.next_check {
            return false;
        }
        self.next_check = now + Self::INTERVAL;
        match pipeline.reload_scene() {
            Ok(changed) => {
                if changed {
                    info!("Reloaded {}", self.dir.display());
                    shared.status().last_error = None;
                }
                changed
            }
            Err(e) => {
                let message = format!("{e:#}");
                if shared.status().last_error.as_deref() != Some(message.as_str()) {
                    shared.set_error("reload_scene", message);
                }
                false
            }
        }
    }
}

/// Move everything the ring holds into `pcm`.
fn drain(consumer: &mut Consumer<f32>, pcm: &mut Vec<f32>) {
    pcm.clear();
    let available = consumer.slots();
    if available == 0 {
        return;
    }
    if let Ok(chunk) = consumer.read_chunk(available) {
        let (first, second) = chunk.as_slices();
        pcm.extend_from_slice(first);
        pcm.extend_from_slice(second);
        chunk.commit_all();
    }
}

/// Keep at most the last quarter second of `pcm` (after a stall, the scene only needs the
/// newest samples).
fn keep_recent(pcm: &mut Vec<f32>, channels: usize, sample_rate: u32) {
    let keep = (sample_rate as usize / 4) * channels;
    if pcm.len() > keep {
        let skip = pcm.len() - keep;
        pcm.drain(..skip - skip % channels);
    }
}

/// Make a frame that came out of the pipeline the latest one, if it is one that is handed out.
fn show(shared: &Shared, pipeline: &Pipeline, rendered: Rendered) {
    let (width, height) = pipeline.size();
    match rendered {
        Rendered::Mapped(frame) => publish(shared, width, height, FrameData::Mapped(frame)),
        #[cfg(target_vendor = "apple")]
        Rendered::PixelBuffer(buffer) => {
            publish(shared, width, height, FrameData::PixelBuffer(buffer));
        }
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        Rendered::Shared(texture) => publish(shared, width, height, FrameData::Shared(texture)),
        #[cfg(target_os = "android")]
        Rendered::Presented => {}
        Rendered::Nothing => {}
    }
}

/// Show the frames on their way as the GPU finishes them, until none is left or `until`. A
/// pipeline drops the frames on their way when its size changes, so its size is theirs.
fn show_finished(shared: &Shared, pipeline: &mut Pipeline, until: Instant) -> Result<()> {
    loop {
        match pipeline.wait_for_frame(until)? {
            Rendered::Nothing => return Ok(()),
            rendered => show(shared, pipeline, rendered),
        }
    }
}

/// Tell the transport listener how frames leave the GPU.
fn notify_transport(shared: &Shared, transport: Transport) {
    let listener = lock(&shared.transport_listener).clone();
    if let Some(listener) = listener {
        listener(transport);
    }
}

/// Make `data` the latest frame and tell the frame listener. The one it replaces gives its buffer
/// back once no reader holds it any more.
fn publish(shared: &Shared, width: u32, height: u32, data: FrameData) {
    let id = shared.next_frame_id.fetch_add(1, Ordering::Relaxed);
    let frame = Arc::new(Frame {
        id,
        width,
        height,
        data,
    });
    let dropped = lock(&shared.frames).push(frame, Instant::now());
    // Dropped outside the lock: releasing a readback buffer unmaps it.
    drop(dropped);
    let listener = lock(&shared.frame_listener).clone();
    if let Some(listener) = listener {
        listener(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectrum_only_with_only_chart_scenes_is_hidden() {
        let visualizer = Visualizer::new();
        visualizer.offer_cover_wall();
        visualizer.set_inputs(VisualizerInputs::SpectrumOnly);
        assert_eq!(visualizer.status().inputs, VisualizerInputs::None);
    }

    #[test]
    fn a_source_without_inputs_hides_the_visualizer() {
        let visualizer = Visualizer::new();
        visualizer.offer_cover_wall();
        visualizer.set_inputs(VisualizerInputs::None);
        assert_eq!(visualizer.status().inputs, VisualizerInputs::None);
        assert_eq!(
            visualizer.library.lock().unwrap().folder.available_inputs(),
            VisualizerInputs::None
        );
    }

    #[test]
    fn a_full_source_makes_chart_scenes_available_before_the_chart_arrives() {
        let visualizer = Visualizer::new();
        visualizer.set_inputs(VisualizerInputs::Full);
        assert_eq!(visualizer.status().inputs, VisualizerInputs::Full);
        assert!(
            visualizer
                .library
                .lock()
                .unwrap()
                .folder
                .supports_source_inputs(&Location::Builtin("Armillary".into()))
        );
    }

    #[test]
    fn spectrum_only_with_an_imported_scene_is_available_and_selects_only_it() {
        let visualizer = Visualizer::new();
        visualizer.offer_cover_wall();
        visualizer.set_inputs(VisualizerInputs::SpectrumOnly);
        assert_eq!(visualizer.status().inputs, VisualizerInputs::None);

        let root =
            std::env::temp_dir().join(format!("rune-visualizer-inputs-{}", std::process::id()));
        let scene = root.join("spectrum-only");
        std::fs::create_dir_all(&scene).unwrap();
        std::fs::write(
            scene.join("scene.toml"),
            r#"format = 1
name = "Spectrum Only"
inputs_required = "spectrum"
license = "MPL-2.0"

[[pass]]
shader = "view.wgsl"
target = "out"
"#,
        )
        .unwrap();

        visualizer
            .set_preset_dirs(std::slice::from_ref(&root))
            .unwrap();
        assert_eq!(visualizer.status().inputs, VisualizerInputs::SpectrumOnly);

        let mut library = lock(&visualizer.library);
        let selected = library
            .folder
            .next_other(&Location::Builtin("Armillary".into()))
            .unwrap();
        assert_eq!(selected.location, Location::Folder(scene.clone()));
        assert!(
            library
                .folder
                .supports_inputs(&selected.location, VisualizerInputs::SpectrumOnly)
        );
        assert!(!library.folder.supports_inputs(
            &Location::Builtin("Armillary".into()),
            VisualizerInputs::SpectrumOnly
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn loading_a_chart_scene_is_rejected_when_source_lacks_chart() {
        let visualizer = Visualizer::new();
        visualizer.set_inputs(VisualizerInputs::SpectrumOnly);
        let error = visualizer
            .load_preset(Location::Builtin("Armillary".into()))
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<PresetLoadError>(),
            Some(&PresetLoadError::InputsUnavailable)
        );
    }

    #[test]
    fn local_track_changes_keep_the_source_inputs_full() {
        let visualizer = Visualizer::new();
        visualizer.set_inputs(VisualizerInputs::Full);
        visualizer.set_playback(Some(Path::new("/music/a.flac")), Duration::ZERO, true, None);
        assert_eq!(visualizer.status().inputs, VisualizerInputs::Full);
        visualizer.set_playback(Some(Path::new("/music/b.flac")), Duration::ZERO, true, None);
        assert_eq!(visualizer.status().inputs, VisualizerInputs::Full);
    }

    fn plan_key(columns: u64) -> PlanKey {
        PlanKey {
            chart: 1,
            scene: 1,
            runtime: 0,
            edits: 0,
            columns,
            level: None,
            settings: scene::DeciderSettings {
                chances: moments::Chances::default(),
                pace: None,
                bursts: None,
                shot: None,
                look: None,
                lens: Default::default(),
            },
        }
    }

    #[test]
    fn a_plan_is_dropped_only_when_the_catalogue_changes() {
        let old = plan_key(3);
        assert!(clears(None, &old));
        assert!(clears(Some(&old), &plan_key(4)));
        // A held shot, another level or any other setting keeps the old plan on show.
        let mut held = plan_key(3);
        held.settings.shot = Some(2);
        held.level = Some(Level::default());
        held.edits = 1;
        held.chart = 2;
        assert!(!clears(Some(&old), &held));
    }

    fn event(i: u32) -> LabEvent {
        LabEvent {
            time: f64::from(i),
            kind: LabEventKind::Look {
                look: i.to_string(),
            },
        }
    }

    #[test]
    fn the_event_log_keeps_the_newest_and_is_read_once() {
        let visualizer = Visualizer::new();
        visualizer.set_inspecting(true);
        let over = LAB_EVENTS as u32 + 10;
        visualizer.shared.log_events((0..over).map(event));
        let events = visualizer.drain_lab_events();
        assert_eq!(events.len(), LAB_EVENTS);
        assert_eq!(events[0], event(10));
        assert_eq!(events[LAB_EVENTS - 1], event(over - 1));
        assert!(visualizer.drain_lab_events().is_empty());
    }

    #[test]
    fn nothing_is_logged_while_not_inspecting() {
        let visualizer = Visualizer::new();
        visualizer.shared.log_events([event(1)]);
        assert!(visualizer.drain_lab_events().is_empty());
        visualizer.set_inspecting(true);
        visualizer.shared.log_events([event(2)]);
        visualizer.set_inspecting(false);
        assert!(visualizer.drain_lab_events().is_empty());
    }

    #[test]
    fn a_plan_of_an_older_job_is_dropped_and_each_kept_one_has_a_new_version() {
        let visualizer = Visualizer::new();
        let shared = &visualizer.shared;
        visualizer.set_playback(Some(Path::new("/music/a.flac")), Duration::ZERO, true, None);
        let plan = || SongPlan {
            path: "/music/a.flac".to_string(),
            ..SongPlan::default()
        };
        assert_eq!((visualizer.plan_version(), visualizer.plan()), (0, None));
        shared.plan_job.store(2, Ordering::Relaxed);
        shared.store_plan(1, plan());
        assert!(visualizer.plan().is_none());
        shared.store_plan(2, plan());
        assert!(visualizer.plan().is_some());
        assert_eq!(visualizer.plan_version(), 1);
        shared.plan_job.store(3, Ordering::Relaxed);
        shared.store_plan(3, plan());
        assert_eq!(visualizer.plan_version(), 2);
        // A song without a chart has no plan, and a job still running for the one before does not
        // bring it back.
        shared.clear_plan();
        shared.store_plan(3, plan());
        assert_eq!((visualizer.plan_version(), visualizer.plan()), (3, None));
    }

    #[test]
    fn the_plan_is_of_the_song_that_plays_and_goes_with_the_lab() {
        let visualizer = Visualizer::new();
        let shared = &visualizer.shared;
        visualizer.set_inspecting(true);
        visualizer.set_playback(Some(Path::new("/music/a.flac")), Duration::ZERO, true, None);
        let job = shared.plan_job.load(Ordering::Relaxed);
        shared.store_plan(
            job,
            SongPlan {
                path: "/music/a.flac".to_string(),
                ..SongPlan::default()
            },
        );
        assert!(visualizer.plan().is_some());
        // Another song started while nothing planned it: the plan is not shown on it.
        visualizer.set_playback(Some(Path::new("/music/b.flac")), Duration::ZERO, true, None);
        assert!(visualizer.plan().is_none());
        visualizer.set_playback(Some(Path::new("/music/a.flac")), Duration::ZERO, true, None);
        assert!(visualizer.plan().is_some());
        // Turning the lab off drops it.
        visualizer.set_inspecting(false);
        assert!(lock(&shared.plan).plan.is_none());
    }

    #[test]
    fn a_new_song_is_a_new_catalogue_and_the_old_ones_last_bucket_keeps_the_old_version() {
        let visualizer = Visualizer::new();
        let shared = &visualizer.shared;
        let song = || lock(&shared.playback).generation;
        let key = |song| (1, 1, 0, song);
        let play =
            |path| visualizer.set_playback(Some(Path::new(path)), Duration::ZERO, true, None);
        play("/music/a.flac");
        let a = song();
        play("/music/a.flac");
        assert_eq!(song(), a);
        lock(&shared.lab).follow(key(a), scene::fixtures::lab_scene);
        let old = lock(&shared.lab).columns().unwrap().version;
        // The old song's bucket at 4:00 is still open when the next song's first frame comes.
        lock(&shared.lab).record(240.0, &Inspect::default(), Level::A);
        play("/music/b.flac");
        assert_ne!(song(), a);
        let mut lab = lock(&shared.lab);
        lab.follow(key(song()), scene::fixtures::lab_scene);
        lab.record(0.0, &Inspect::default(), Level::A);
        lab.record(0.5, &Inspect::default(), Level::A);
        let new = lab.columns().unwrap().version;
        assert_ne!(new, old);
        let samples = lab.drain();
        let [last, first] = &samples[..] else {
            panic!("two buckets: {}", samples.len())
        };
        assert_eq!((last.bucket, last.columns_version), (4800, old));
        assert_eq!((first.bucket, first.columns_version), (0, new));
    }
}
