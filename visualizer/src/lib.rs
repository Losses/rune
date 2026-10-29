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
mod loading;
mod moments;
mod pipeline;
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

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use fsio::FsIo;
use log::{error, info};
use rand::rngs::ThreadRng;
use rtrb::Consumer;

pub use bind::{Level, RuntimeBindings, Used};

/// The runtime's decisions as the visualizer is built with them (`runtime.toml`), the text a lab
/// edits and sends with [`LabCommand::RuntimeBindings`].
pub const RUNTIME_TOML: &str = bind::RUNTIME_TOML;
pub use builtin::{Credit, credits as builtin_credits};
pub use chart::{Chart, ChartState};
use chart::{ChartFollower, Playback};
pub use inspect::{
    Inspect, InspectBinding, InspectDirector, InspectEffects, InspectInput, InspectLens,
    InspectLooks, InspectMusic, InspectSection, InspectShot, InspectSignal, InspectSwitching,
    InspectTransition, LabCommand, LabLens, LabOverrides, LabPace, LabTransition,
};
use loading::Loads;
pub use moments::{Chances, EffectLevel};
pub use pipeline::{DEFAULT_PRESET_NAME, Transport};
use pipeline::{Pipeline, Rendered, SceneBuild};
pub use presets::{Location, PresetInfo};
use presets::{Preset, PresetList};
pub use rhythm::{BeatTracker, Rhythm};
use scene::Scene;
#[cfg(target_os = "linux")]
pub use shared_vulkan::SharedFrame;
use switching::{Decision, SceneSwitch};
pub use transition::CutInStyle;
use transition::{Durations, Transition};
#[cfg(target_os = "android")]
pub use window::Window;

pub const DEFAULT_FPS: u32 = 60;

/// Told the id of each new frame (see [`Visualizer::set_frame_listener`]).
pub type FrameListener = Arc<dyn Fn(u64) + Send + Sync>;

/// Told how frames leave the GPU (see [`Visualizer::set_transport_listener`]).
pub type TransportListener = Arc<dyn Fn(Transport) + Send + Sync>;

/// Sample rate assumed until the first track tells the real one.
const FALLBACK_SAMPLE_RATE: u32 = 44_100;

/// How often the measured fps in [`Status`] is refreshed.
const FPS_WINDOW: Duration = Duration::from_millis(500);
/// How often a paused render thread looks at whether the music plays again.
const PAUSED_POLL: Duration = Duration::from_millis(50);

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
    /// Where the scene on screen is, as [`Location`]'s text (`builtin:<name>` or a folder's
    /// path): the name alone does not tell a built-in scene from a folder scene of that name.
    pub preset_path: String,
    /// Scenes to choose from: the built-in ones and those of the chosen folders.
    pub preset_count: usize,
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
        let names = builtin::scenes().into_iter().map(|s| s.name).collect();
        Self {
            folder: PresetList::with_builtins(names),
        }
    }
}

impl Library {
    /// The scene for an automatic switch while `current` is on screen, if there are scenes to
    /// choose from.
    fn auto_next(&mut self, rng: &mut ThreadRng, current: &Location) -> Option<Result<Preset>> {
        (self.folder.len() > 0).then(|| self.folder.next_other(rng, current))
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
    /// Whether the automatic preset switching waits (the user locked the preset or a menu is
    /// open).
    auto_switch_paused: AtomicBool,
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
    /// The file that plays and where, for analysing it into a chart and the time in it.
    playback: Mutex<Playback>,
    /// Where the player's files are opened from, set once; without it files are not analysed.
    fsio: OnceLock<Arc<FsIo>>,
    /// Whether the scene on screen reads the atlas of album covers.
    wants_covers: AtomicBool,
    /// The latest atlas of covers and its version, which the render thread uploads.
    covers: Mutex<CoverAtlas>,
    /// The number of the latest `set_covers` job; an older job drops its result.
    covers_job: AtomicU64,
    /// The latest cell for the cover of the track playing and its version.
    playing_cover: Mutex<PlayingCover>,
    /// The number of the latest `set_playing_cover` job; an older job drops its result.
    playing_cover_job: AtomicU64,
    /// Asked to provide covers (by calling [`Visualizer::set_covers`]) when a scene that reads
    /// them is loaded.
    covers_requester: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Told the id of every new frame.
    frame_listener: Mutex<Option<FrameListener>>,
    /// Told how frames leave the GPU, when the render thread knows and when that changes.
    transport_listener: Mutex<Option<TransportListener>>,
    /// The LUID of the adapter Flutter renders on (see [`adapter_luid`]), 0 while unknown.
    adapter_luid: AtomicU64,
    /// The UUIDs of the GPU Flutter renders on, while known.
    device_uuids: Mutex<Option<DeviceUuids>>,
    /// The app could not show a shared frame: the render thread reads frames back from its next
    /// frame on.
    fall_back: AtomicBool,
    /// Android: the window the next render thread presents into, which it takes.
    #[cfg(target_os = "android")]
    window: Mutex<Option<Window>>,
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

    fn status(&self) -> MutexGuard<'_, Status> {
        self.status.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_error(&self, message: String) {
        error!("Visualizer: {message}");
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
    /// Shared with the render thread, which picks the presets of the automatic switching.
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
                auto_switch_paused: AtomicBool::new(false),
                // `RUNE_VISUALIZER_READBACK=1` picks the readback path, to compare the two.
                prefer_readback: AtomicBool::new(
                    std::env::var_os("RUNE_VISUALIZER_READBACK").is_some_and(|v| v == "1"),
                ),
                // Until the player says otherwise, draw.
                playing: AtomicBool::new(true),
                inspecting: AtomicBool::new(false),
                inspect: Mutex::new(None),
                visible: AtomicBool::new(true),
                accent: AtomicU32::new(0),
                effect_level: AtomicU8::new(0),
                lab_chance: AtomicU8::new(NO_LAB_CHANCE),
                playback: Mutex::new(Playback::default()),
                fsio: OnceLock::new(),
                wants_covers: AtomicBool::new(false),
                covers: Mutex::new(CoverAtlas::default()),
                covers_job: AtomicU64::new(0),
                playing_cover: Mutex::new(PlayingCover::default()),
                playing_cover_job: AtomicU64::new(0),
                covers_requester: Mutex::new(None),
                frame_listener: Mutex::new(None),
                transport_listener: Mutex::new(None),
                adapter_luid: AtomicU64::new(0),
                device_uuids: Mutex::new(None),
                fall_back: AtomicBool::new(false),
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
        }
    }

    /// A copy of what the render thread saw in its last frame, while inspecting (see
    /// [`Self::set_inspecting`]) and once a frame was drawn; else none.
    pub fn inspect(&self) -> Option<Inspect> {
        lock(&self.shared.inspect).clone()
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

    /// Open the player's files through `fsio` (once, before [`Self::start`]), so that the
    /// visualizer can analyse the file that plays. Without it scenes stay on the live beat
    /// tracker.
    pub fn set_fsio(&self, fsio: Arc<FsIo>) {
        let _ = self.shared.fsio.set(fsio);
    }

    /// Follow the player's file and position (call it with every progress report). The file
    /// is analysed in the background (see [`Self::set_fsio`]); once that is done, scenes get
    /// the beats, bars and sections from the chart.
    pub fn set_playback(&self, path: Option<&Path>, position: Duration, playing: bool) {
        let mut playback = lock(&self.shared.playback);
        if playback.path.as_deref() != path {
            playback.path = path.map(Path::to_path_buf);
        }
        playback.position = position;
        playback.playing = playing;
        playback.at = Some(Instant::now());
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
        {
            let mut status = self.shared.status();
            let preset_count = status.preset_count;
            *status = Status {
                running: true,
                width,
                height,
                target_fps: fps,
                preset_count,
                ..Status::default()
            };
        }
        self.shared
            .auto_switch_paused
            .store(false, Ordering::Relaxed);
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

    /// Show the scene at `location`: a folder (watched for changes) or a built-in scene. Built
    /// in the background like [`Visualizer::load_preset_path`]; a newer request replaces one that
    /// is still being built.
    pub fn load_preset(&self, location: Location) -> Result<()> {
        self.send(Command::LoadScene(location))
    }

    fn load_entry(&self, preset: &Preset) -> Result<()> {
        self.load_preset(preset.location.clone())
    }

    /// Scan the folders `dirs` and their subfolders for scenes (folders with a `scene.toml`) for
    /// next/previous/random, replacing the earlier folders and emptying the history. A scene
    /// folder named like a built-in scene is listed beside it. Returns the number of scenes to
    /// choose from, the built-in ones included. A folder that cannot be read is skipped and shows
    /// up in [`Status::last_error`]. Without scenes in the folders a running visualizer changes to
    /// the first built-in scene; otherwise the caller loads one.
    pub fn set_preset_dirs(&self, dirs: &[PathBuf]) -> Result<usize> {
        let mut library = lock(&self.library);
        let (count, errors) = library.folder.scan_all(dirs);
        self.shared.status().preset_count = count;
        if !errors.is_empty() {
            let message: Vec<String> = errors.iter().map(|e| format!("{e:#}")).collect();
            self.shared.set_error(message.join("; "));
        }
        // Not running: `start` begins with the built-in scene anyway.
        if library.folder.folder_len() == 0 {
            let _ = self.send(Command::LoadDefaultScene);
        }
        Ok(count)
    }

    /// Every scene, by group and name, for choosing one.
    pub fn list_presets(&self) -> Vec<PresetInfo> {
        lock(&self.library).folder.infos()
    }

    /// Forward in the history if [`Self::previous_preset`] went back, else a new random scene.
    /// Nothing happens without scenes.
    pub fn next_preset(&self) -> Result<()> {
        let mut library = lock(&self.library);
        if library.folder.len() == 0 {
            return Ok(());
        }
        let preset = library.folder.next(&mut rand::thread_rng())?;
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

    /// Pause or resume the automatic switching of scenes. While paused a song that begins is no
    /// change: the scene counts as on with the song that plays when the pause ends. A new render
    /// thread starts unpaused.
    pub fn set_auto_switch_paused(&self, paused: bool) {
        self.shared
            .auto_switch_paused
            .store(paused, Ordering::Relaxed);
    }

    /// Record an error that happened outside the render thread, for [`Status::last_error`].
    pub fn report_error(&self, message: String) {
        self.shared.set_error(message);
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
        shared.set_error(message);
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
        Arc::new(move |message: String| shared.set_error(message))
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
        status.preset_name = pipeline.preset_name().to_string();
        status.preset_path = Location::Builtin(status.preset_name.clone()).to_string();
    }
    notify_transport(shared, transport);

    let interval = Duration::from_secs_f64(1.0 / f64::from(fps));
    let started = Instant::now();
    let mut deadline = started;
    let mut pcm: Vec<f32> = Vec::new();
    let mut window_start = started;
    let mut window_frames = 0u32;
    let mut auto = AutoScene::new();
    // Scenes being built on other threads (see [`loading`]).
    let mut loads = Loads::new();
    let mut rng = rand::thread_rng();
    // Time spent with the music paused or the window out of sight, which the visualizer's clock
    // leaves out.
    let mut frozen = Duration::ZERO;
    // While paused or out of sight one frame is drawn (the first one, or after a command such as
    // a resize) and then held.
    let mut still_drawn = false;
    // The folder of the scene on screen, reloaded when it is saved (live editing).
    let mut watched: Option<Watched> = None;
    let mut tracker = BeatTracker::new(sample_rate);
    let mut charts = ChartFollower::new(shared.fsio.get().cloned());
    // The version of the atlas of covers the pipeline has.
    let mut covers_seen: Option<u64> = None;
    // The same for the cell of the track playing.
    let mut playing_cover_seen: Option<u64> = None;
    // Benchmark mode (see [`bench`]) shows every built-in scene in turn, whatever is asked for.
    let mut bench = bench::seconds_from_env().map(bench::Bench::new);

    loop {
        loop {
            match commands.try_recv() {
                Ok(Command::Stop) | Err(TryRecvError::Disconnected) => return Ok(()),
                Ok(command) => apply(
                    shared,
                    library,
                    &mut pipeline,
                    &mut auto,
                    &mut loads,
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

        // A scene that finished building replaces the one on screen, also while paused.
        if let Some(loaded) = loads.take() {
            install(shared, &mut pipeline, &mut watched, loaded);
            still_drawn = false;
        }

        // A scene built ahead is held until the switch; one that failed to build shows its error
        // now and the wait starts again.
        if let Some(loaded) = auto.loads.take() {
            if let Ok(scene) = &loaded.scene {
                // The scene coming decides how the switch to it looks.
                auto.decider
                    .allow(switching::Allowed::of(scene.switch_transitions()));
                auto.held = Some(loaded);
            } else {
                install(shared, &mut pipeline, &mut watched, loaded);
                auto.restart();
            }
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

        // The benchmark draws with the music paused or the window out of sight too.
        let paused = bench.is_none()
            && (!shared.playing.load(Ordering::Relaxed) || !shared.visible.load(Ordering::Relaxed));
        if paused && still_drawn {
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
                        &mut auto,
                        &mut loads,
                        command,
                    );
                    still_drawn = false;
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            frozen += since.elapsed();
            deadline = Instant::now();
            window_start = deadline;
            window_frames = 0;
            continue;
        }
        still_drawn = paused;
        if !pcm.is_empty() {
            pipeline.push_pcm(&pcm, channels);
        }

        let time = (started.elapsed() - frozen).as_secs_f64();
        // The benchmark picks the scenes itself.
        let switch_paused = bench.is_some() || shared.auto_switch_paused.load(Ordering::Relaxed);
        // A scan holds the library while it reads every file; try again on the next frame
        // instead of stalling this one.
        let free = || match library.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        };
        // The switch follows the songs (see [`switching`]): it comes when another file starts. It
        // only happens with the next scene built and no scene loaded by hand still on its way.
        let ready = auto.held.is_some() && !loads.is_pending();
        let song = lock(&shared.playback).path.clone();
        let decision = auto.decider.tick(
            time,
            song.as_deref(),
            pipeline.music(),
            switch_paused,
            ready,
        );
        if switch_paused {
            // Nothing is built ahead while the switch waits.
            auto.restart();
        }
        match decision {
            Decision::Wait => {}
            Decision::Prepare => {
                if let Some(mut library) = free() {
                    let current = Location::parse(&shared.status().preset_path);
                    let next = library.auto_next(&mut rng, &current);
                    drop(library);
                    match next {
                        Some(Ok(preset)) => {
                            start_scene_load(
                                shared,
                                &pipeline,
                                &mut auto.loads,
                                preset.location,
                                PREPARED_SWITCH,
                            );
                            auto.decider.prepare_started();
                        }
                        Some(Err(e)) => {
                            shared.set_error(format!("{e:#}"));
                            auto.restart();
                        }
                        None => auto.restart(),
                    }
                }
            }
            Decision::Switch {
                transition,
                durations,
            } => {
                if let Some(mut loaded) = auto.held.take() {
                    // A lab's override replaces what the music chose, last of all.
                    let (transition, durations) = switching::replaced(
                        pipeline.lab().switch.map(Into::into),
                        (transition, durations),
                        pipeline.music(),
                    );
                    loaded.switch = Switch(transition, durations);
                    install(shared, &mut pipeline, &mut watched, loaded);
                    still_drawn = false;
                }
                auto.restart();
            }
        }

        // A chart knows the beats of the whole song, or that it has none: it replaces the
        // tracker's guess either way.
        let chart = charts.tick(&lock(&shared.playback), Instant::now());
        // A lab may hold the music at a lower fallback level than the song has.
        let chart = bind::forced_chart(pipeline.lab().level, chart);
        let rhythm = chart::trusted_rhythm(tracker.rhythm(), chart.as_ref());
        pipeline.set_rhythm(&rhythm);
        pipeline.set_chart(chart.as_ref());
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
            let mut status = shared.status();
            status.preset_name = pipeline.preset_name().to_string();
            status.preset_path = Location::Builtin(status.preset_name.clone()).to_string();
        }
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
        if shared.inspecting.load(Ordering::Relaxed) {
            let seen = inspection(shared, &pipeline, &auto, &charts, time);
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
                    &mut auto,
                    &mut loads,
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

/// How a scene built ahead for the automatic switch is shown until the switch picks the real
/// one (see [`switching`]), when the scene is held and nothing is shown.
const PREPARED_SWITCH: Switch = Switch(
    Transition::Cut,
    Durations {
        out: 0.0,
        into: 0.0,
    },
);

/// The automatic scene switch: its decision ([`SceneSwitch`]) and the scene it has built ahead.
struct AutoScene {
    decider: SceneSwitch,
    /// The scene being built ahead, on its own so that a load the user asks for neither replaces
    /// it nor is replaced by it.
    loads: Loads<Loaded>,
    /// The scene built ahead, waiting for the moment to switch.
    held: Option<Loaded>,
}

impl AutoScene {
    fn new() -> Self {
        Self {
            decider: SceneSwitch::default(),
            loads: Loads::new(),
            held: None,
        }
    }

    /// Start over: the wait counts again and what was built ahead is dropped.
    fn restart(&mut self) {
        self.decider.restart();
        self.loads.discard();
        self.held = None;
    }
}

struct Loaded {
    /// How to show it.
    switch: Switch,
    /// Where the scene is from; a folder is watched for changes.
    location: Location,
    scene: Result<Scene>,
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
                scene: build.run(),
            })
            .context("Failed to start a thread to build the scene")
    });
    if let Err(e) = started {
        shared.set_error(format!("{e:#}"));
    }
}

/// Show a scene that finished building, or its error (the scene on screen stays then).
fn install(
    shared: &Shared,
    pipeline: &mut Pipeline,
    watched: &mut Option<Watched>,
    loaded: Loaded,
) {
    match loaded.scene.and_then(|scene| {
        let Switch(transition, durations) = loaded.switch;
        pipeline.switch_to(scene, transition, durations)
    }) {
        Ok(()) => {
            *watched = match &loaded.location {
                Location::Folder(dir) => Some(Watched::new(dir.clone())),
                Location::Builtin(_) => None,
            };
            {
                let mut status = shared.status();
                status.preset_name = pipeline.preset_name().to_string();
                status.preset_path = loaded.location.to_string();
            }
            if pipeline.wants_covers() {
                shared.wants_covers.store(true, Ordering::Relaxed);
                let request = lock(&shared.covers_requester).clone();
                if let Some(request) = request {
                    request();
                }
            }
            shared.status().last_error = None;
        }
        Err(e) => shared.set_error(format!("{e:#}")),
    }
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
    }
}

/// Carry out a command. A scene it loads is shown through [`MANUAL_SWITCH`].
fn apply(
    shared: &Shared,
    library: &Mutex<Library>,
    pipeline: &mut Pipeline,
    auto: &mut AutoScene,
    loads: &mut Loads<Loaded>,
    command: Command,
) {
    // A load the user asked for replaces the scene built ahead and starts the wait for the next
    // automatic switch again.
    if matches!(
        command,
        Command::LoadScene(_) | Command::LoadDefaultScene | Command::Lab(LabCommand::NextScene(_))
    ) {
        auto.restart();
    }
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
        Err(e) => shared.set_error(format!("{e:#}")),
    }
}

/// [`Shared::lab_chance`] when no scene lab set a chance.
const NO_LAB_CHANCE: u8 = u8::MAX;

/// Carry out a scene lab's request. The scene a switch asks for is built in the background and
/// shown like one the user chose, but through the transition asked for, with the durations an
/// automatic switch of that kind takes (see [`switching::requested`]).
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
            Err(e) => shared.set_error(format!("{e:#}")),
        },
        LabCommand::SceneBindings(text) => match pipeline.set_scene_bindings(&text) {
            Ok(()) => shared.status().last_error = None,
            Err(e) => shared.set_error(format!("{e:#}")),
        },
        LabCommand::NextScene(transition) => {
            let current = Location::parse(&shared.status().preset_path);
            let next = lock(library).auto_next(&mut rand::thread_rng(), &current);
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
                Some(Err(e)) => shared.set_error(format!("{e:#}")),
                None => info!("There is no scene to switch to"),
            }
        }
    }
}

/// What the render thread saw in the frame it drew at `time`, for [`Visualizer::inspect`].
fn inspection(
    shared: &Shared,
    pipeline: &Pipeline,
    auto: &AutoScene,
    charts: &ChartFollower,
    time: f64,
) -> Inspect {
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
            preparing: auto.loads.is_pending(),
            held: auto.held.is_some(),
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
                    shared.set_error(message);
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
