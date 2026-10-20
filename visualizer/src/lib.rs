//! A MilkDrop-style visualizer: a render thread that turns PCM into BGRA frames.
//!
//! The thread owns a headless wgpu device and a [`lactea`] engine. Each frame it drains the PCM
//! ring buffer and renders. On macOS it draws straight into an IOSurface-backed pixel buffer that
//! the app can show without a copy; elsewhere, or if that fails, it renders into an offscreen
//! texture that is copied into a readback buffer, and the mapped buffer itself is the frame.
//! Either way the result is the latest [`Frame`]. Readers ([`Visualizer::latest_frame`]) only
//! clone an `Arc` under a short lock and never wait for the GPU. A reader that keeps frames holds
//! their buffers, and the renderer skips frames while none is free.

mod pipeline;
mod presets;
mod readback;
#[cfg(target_os = "macos")]
mod surface;
mod textures;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use lactea::FrameClock;
use log::{error, info};
use rand::rngs::ThreadRng;
use rtrb::Consumer;

pub use pipeline::{DEFAULT_PRESET_NAME, Transport};
use pipeline::{Pipeline, Rendered};
use presets::{AutoSwitch, Location, Preset, PresetList};

pub use lactea::Transition;

pub const DEFAULT_FPS: u32 = 60;

/// Sample rate assumed until the first track tells the real one.
const FALLBACK_SAMPLE_RATE: u32 = 44_100;

/// The blend of a switch the user asked for (MilkDrop 2: 1.7 s).
const MANUAL_BLEND_SECONDS: f64 = 1.7;

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
    /// A mapped readback buffer: rows top to bottom, BGRA on macOS and RGBA elsewhere.
    Mapped(readback::MappedFrame),
    /// An IOSurface-backed `kCVPixelFormatType_32BGRA` pixel buffer.
    #[cfg(target_os = "macos")]
    PixelBuffer(surface::PixelBuffer),
}

impl Frame {
    /// The pixels and the bytes from one row to the next (at least `width * 4`; equal to it when
    /// the width is a multiple of 64), if the frame was read back. BGRA on macOS, RGBA elsewhere.
    pub fn pixels(&self) -> Option<(&[u8], usize)> {
        match &self.data {
            FrameData::Mapped(frame) => Some((frame.bytes(), frame.stride())),
            #[cfg(target_os = "macos")]
            FrameData::PixelBuffer(_) => None,
        }
    }

    /// A `CVPixelBufferRef` with one more reference for the receiver to release, if the frame is
    /// in a pixel buffer.
    #[cfg(target_os = "macos")]
    pub fn retained_pixel_buffer(&self) -> Option<*mut std::ffi::c_void> {
        match &self.data {
            FrameData::PixelBuffer(buffer) => Some(buffer.clone().into_retained_raw()),
            FrameData::Mapped(_) => None,
        }
    }

    /// The pixels as tightly packed rows (BGRA on macOS, RGBA elsewhere). Copies; for tests.
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
            #[cfg(target_os = "macos")]
            FrameData::PixelBuffer(buffer) => buffer.read_bgra(self.width, self.height),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub running: bool,
    pub width: u32,
    pub height: u32,
    pub target_fps: u32,
    pub measured_fps: f32,
    pub preset_name: String,
    /// Presets in the chosen folder (0 without one).
    pub preset_count: usize,
    pub adapter: String,
    /// How frames leave the GPU, once the render thread runs.
    pub transport: Option<Transport>,
    pub last_error: Option<String>,
}

enum Command {
    Resize(u32, u32),
    LoadPreset {
        bytes: Vec<u8>,
        name: String,
        transition: Transition,
    },
    /// Show the default preset (the folder was cleared).
    LoadDefaultPreset(Transition),
    /// The textures of this preset folder, none without one.
    SetTextures(Option<PathBuf>),
    Stop,
}

/// The presets of the folder the user chose, if any.
#[derive(Default)]
struct Library {
    folder: PresetList,
    folder_dir: Option<PathBuf>,
}

impl Library {
    /// The preset for an automatic switch, if there are presets to choose from.
    fn auto_next(&mut self, rng: &mut ThreadRng) -> Option<Result<Preset>> {
        (self.folder.len() > 0).then(|| self.folder.next(rng))
    }
}

struct Shared {
    latest: Mutex<Option<Arc<Frame>>>,
    status: Mutex<Status>,
    next_frame_id: AtomicU64,
    /// Whether the automatic preset switching waits (the user locked the preset or a menu is
    /// open).
    auto_switch_paused: AtomicBool,
    /// Read frames back even where pixel buffers work (tests, and comparing the two paths).
    prefer_readback: AtomicBool,
    /// Whether the music plays. While it does not, the picture and its clock stand still.
    playing: AtomicBool,
}

impl Shared {
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
        Self {
            shared: Arc::new(Shared {
                latest: Mutex::new(None),
                status: Mutex::new(Status::default()),
                next_frame_id: AtomicU64::new(1),
                auto_switch_paused: AtomicBool::new(false),
                // `RUNE_VISUALIZER_READBACK=1` picks the readback path, to compare the two.
                prefer_readback: AtomicBool::new(
                    std::env::var_os("RUNE_VISUALIZER_READBACK").is_some_and(|v| v == "1"),
                ),
                // Until the player says otherwise, draw.
                playing: AtomicBool::new(true),
            }),
            worker: Mutex::new(None),
            library: Arc::new(Mutex::new(Library::default())),
        }
    }

    /// Follow the player: while the music is paused or stopped the visualizer draws nothing and
    /// its clock stops, so presets, transitions and the automatic switching resume where they
    /// were.
    pub fn set_playing(&self, playing: bool) {
        self.shared.playing.store(playing, Ordering::Relaxed);
    }

    /// Read frames back even on macOS, where they normally go into IOSurface-backed pixel
    /// buffers. Applies from the next [`Self::start`].
    pub fn prefer_readback(&self, readback: bool) {
        self.shared
            .prefer_readback
            .store(readback, Ordering::Relaxed);
    }

    /// Start the render thread. It returns at once; a missing GPU or an engine error shows up
    /// in [`Status::last_error`] with `running` false. Fails if a thread is already there, so
    /// call [`Self::stop`] first (it gives the `PcmInput` back).
    pub fn start(&self, input: PcmInput, width: u32, height: u32, fps: u32) -> Result<()> {
        let (width, height, fps) = (width.max(1), height.max(1), fps.clamp(1, 240));
        // Before the worker lock, which `publish_folder` takes to reach the renderer.
        let textures = lock(&self.library).folder_dir.clone();
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
        let (commands, receiver) = mpsc::channel();
        let shared = Arc::clone(&self.shared);
        let library = Arc::clone(&self.library);
        let handle = thread::Builder::new()
            .name("visualizer".into())
            .spawn(move || {
                worker_main(
                    shared,
                    library,
                    input,
                    (width, height, fps),
                    textures,
                    receiver,
                )
            })
            .context("Failed to spawn the visualizer thread")?;
        *worker = Some(Worker { commands, handle });
        Ok(())
    }

    /// Stop the render thread and hand back the PCM input so that it can be reused.
    pub fn stop(&self) -> Option<PcmInput> {
        let worker = lock(&self.worker).take()?;
        let _ = worker.commands.send(Command::Stop);
        let input = worker.handle.join().ok();
        *lock(&self.shared.latest) = None;
        input
    }

    pub fn resize(&self, width: u32, height: u32) -> Result<()> {
        self.send(Command::Resize(width, height))
    }

    pub fn load_preset(&self, bytes: Vec<u8>, name: &str, transition: Transition) -> Result<()> {
        self.send(Command::LoadPreset {
            bytes,
            name: name.to_string(),
            transition,
        })
    }

    pub fn load_preset_path(&self, path: &Path, transition: Transition) -> Result<()> {
        self.load_entry(&Preset::from_path(path), transition)
    }

    fn load_entry(&self, preset: &Preset, transition: Transition) -> Result<()> {
        self.load_preset(read_preset(preset)?, &preset.name, transition)
    }

    /// Scan `dir` and its subfolders for `.milk` / `.milk2` files for next/previous/random.
    /// Returns their count. Textures come from `dir/textures`, else `dir` if it has images,
    /// else `dir/../textures`.
    pub fn set_preset_dir(&self, dir: &Path) -> Result<usize> {
        let mut library = lock(&self.library);
        let count = library.folder.scan(dir)?;
        library.folder_dir = Some(dir.to_path_buf());
        self.shared.status().preset_count = count;
        // Not running: `start` hands over the current folder.
        let _ = self.send(Command::SetTextures(Some(dir.to_path_buf())));
        Ok(count)
    }

    /// Stop using the preset folder: the list and its history are emptied, and a running
    /// visualizer softly changes to the default preset.
    pub fn clear_preset_dir(&self) {
        let mut library = lock(&self.library);
        library.folder.clear();
        library.folder_dir = None;
        self.shared.status().preset_count = 0;
        let _ = self.send(Command::SetTextures(None));
        let _ = self.send(Command::LoadDefaultPreset(Transition::Soft {
            seconds: MANUAL_BLEND_SECONDS,
        }));
    }

    /// Forward in the history if [`Self::previous_preset`] went back, else a new random preset
    /// (weighted by the presets' ratings, as in MilkDrop 2). Nothing happens without presets.
    pub fn next_preset(&self, transition: Transition) -> Result<()> {
        let mut library = lock(&self.library);
        if library.folder.len() == 0 {
            return Ok(());
        }
        let preset = library.folder.next(&mut rand::thread_rng())?;
        drop(library);
        self.load_entry(&preset, transition)
    }

    /// One step back in the history of loaded presets; nothing happens at its start or without
    /// presets.
    pub fn previous_preset(&self, transition: Transition) -> Result<()> {
        let mut library = lock(&self.library);
        if library.folder.len() == 0 {
            return Ok(());
        }
        let preset = library.folder.previous()?;
        drop(library);
        match preset {
            Some(preset) => self.load_entry(&preset, transition),
            None => Ok(()),
        }
    }

    /// A new random preset (weighted by rating); the history ahead of the current one is dropped.
    /// Nothing happens without presets.
    pub fn random_preset(&self, transition: Transition) -> Result<()> {
        let mut library = lock(&self.library);
        if library.folder.len() == 0 {
            return Ok(());
        }
        let preset = library.folder.random(&mut rand::thread_rng())?;
        drop(library);
        self.load_entry(&preset, transition)
    }

    /// Pause or resume the automatic switching of presets. A pause makes the timer start over
    /// when it ends. A new render thread starts unpaused.
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
        lock(&self.shared.latest).clone()
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

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// The bytes of a preset file.
fn read_preset(preset: &Preset) -> Result<Vec<u8>> {
    let Location::File(path) = &preset.location;
    std::fs::read(path).with_context(|| format!("Failed to read {}", path.display()))
}

fn worker_main(
    shared: Arc<Shared>,
    library: Arc<Mutex<Library>>,
    mut input: PcmInput,
    (width, height, fps): (u32, u32, u32),
    textures: Option<PathBuf>,
    commands: Receiver<Command>,
) -> PcmInput {
    let result = catch_unwind(AssertUnwindSafe(|| {
        run(
            &shared,
            &library,
            &mut input,
            (width, height, fps),
            textures,
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
    textures: Option<PathBuf>,
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
    let mut pipeline = Pipeline::new(width, height, fps, sample_rate, readback, on_gpu_error)?;
    let mut transport = pipeline.transport();
    info!(
        "Visualizer started on {} ({transport:?})",
        pipeline.adapter_name
    );
    if let Some(dir) = textures
        && let Err(e) = pipeline.set_textures(Some(&dir))
    {
        shared.set_error(format!("{e:#}"));
    }
    {
        let mut status = shared.status();
        status.adapter = pipeline.adapter_name.clone();
        status.transport = Some(transport);
        status.preset_name = pipeline.preset_name().to_string();
    }

    let interval = Duration::from_secs_f64(1.0 / f64::from(fps));
    let started = Instant::now();
    let mut deadline = started;
    let mut frame_index = 0u32;
    let mut pcm: Vec<f32> = Vec::new();
    let mut window_start = started;
    let mut window_frames = 0u32;
    let mut auto_switch = AutoSwitch::default();
    let mut rng = rand::thread_rng();
    // Time spent with the music paused, which the visualizer's clock leaves out.
    let mut frozen = Duration::ZERO;
    // While paused one frame is drawn (the first one, or after a command such as a resize) and
    // then held.
    let mut still_drawn = false;

    loop {
        loop {
            match commands.try_recv() {
                Ok(Command::Stop) | Err(TryRecvError::Disconnected) => return Ok(()),
                Ok(command) => apply(shared, &mut pipeline, &mut auto_switch, command),
                Err(TryRecvError::Empty) => break,
            }
        }

        let channels = match input.channels.load(Ordering::Relaxed) {
            0 => 2,
            n => n as usize,
        };
        drain(&mut input.consumer, &mut pcm, channels, sample_rate);

        let paused = !shared.playing.load(Ordering::Relaxed);
        if paused && still_drawn {
            // Keep the last frame on screen and wait, for commands or for the music.
            let since = Instant::now();
            match commands.recv_timeout(PAUSED_POLL) {
                Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
                Ok(command) => {
                    apply(shared, &mut pipeline, &mut auto_switch, command);
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
        let switch_paused = shared.auto_switch_paused.load(Ordering::Relaxed);
        // A scan holds the library while it reads every file; try again on the next frame
        // instead of stalling this one.
        let free = || match library.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        };
        if auto_switch.tick(time, switch_paused, &mut rng)
            && let Some(mut library) = free()
        {
            // The list is built already; only the one preset file is read here.
            let next = library.auto_next(&mut rng);
            drop(library);
            if let Some(next) = next {
                let load = next.and_then(|preset| {
                    Ok(Command::LoadPreset {
                        bytes: read_preset(&preset)?,
                        name: preset.name,
                        transition: Transition::Soft {
                            seconds: AutoSwitch::BLEND_SECONDS,
                        },
                    })
                });
                match load {
                    Ok(command) => apply(shared, &mut pipeline, &mut auto_switch, command),
                    Err(e) => shared.set_error(format!("{e:#}")),
                }
            }
            auto_switch.restart();
        }

        let clock = FrameClock {
            time,
            frame: frame_index,
        };
        let rendered = pipeline.render(clock)?;
        frame_index = frame_index.wrapping_add(1);
        let (width, height) = pipeline.size();
        match rendered {
            Rendered::Mapped(frame) => publish(shared, width, height, FrameData::Mapped(frame)),
            #[cfg(target_os = "macos")]
            Rendered::PixelBuffer(buffer) => {
                publish(shared, width, height, FrameData::PixelBuffer(buffer));
            }
            Rendered::Nothing => {}
        }
        // The output falls back to reading frames back if pixel buffers stop working.
        if pipeline.transport() != transport {
            transport = pipeline.transport();
            info!("Visualizer frames now leave the GPU by {transport:?}");
            shared.status().transport = Some(transport);
        }

        window_frames += 1;
        let elapsed = window_start.elapsed();
        if elapsed >= FPS_WINDOW {
            shared.status().measured_fps = window_frames as f32 / elapsed.as_secs_f32();
            window_start = Instant::now();
            window_frames = 0;
        }

        // Wait for the next frame, but wake up for commands.
        deadline += interval;
        let now = Instant::now();
        if deadline > now {
            match commands.recv_timeout(deadline - now) {
                Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
                Ok(command) => apply(shared, &mut pipeline, &mut auto_switch, command),
                Err(RecvTimeoutError::Timeout) => {}
            }
        } else {
            // Rendering is slower than the target: do not try to catch up.
            deadline = now;
        }
    }
}

fn apply(shared: &Shared, pipeline: &mut Pipeline, auto_switch: &mut AutoSwitch, command: Command) {
    // Any load, manual or automatic, starts the wait for the next automatic switch again.
    if matches!(
        command,
        Command::LoadPreset { .. } | Command::LoadDefaultPreset(_)
    ) {
        auto_switch.restart();
    }
    let result = match command {
        Command::Resize(width, height) => pipeline.resize(width, height).map(|()| {
            let (width, height) = pipeline.size();
            let mut status = shared.status();
            status.width = width;
            status.height = height;
        }),
        Command::LoadPreset {
            bytes,
            name,
            transition,
        } => pipeline
            .load_preset(&bytes, &name, transition)
            .map(|()| shared.status().preset_name = name),
        Command::LoadDefaultPreset(transition) => pipeline
            .load_default_preset(transition)
            .map(|()| shared.status().preset_name = DEFAULT_PRESET_NAME.to_string()),
        Command::SetTextures(dir) => pipeline.set_textures(dir.as_deref()),
        Command::Stop => Ok(()),
    };
    match result {
        Ok(()) => shared.status().last_error = None,
        Err(e) => shared.set_error(format!("{e:#}")),
    }
}

/// Move everything the ring holds into `pcm`, keeping at most the last quarter second.
fn drain(consumer: &mut Consumer<f32>, pcm: &mut Vec<f32>, channels: usize, sample_rate: u32) {
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
    let keep = (sample_rate as usize / 4) * channels;
    if pcm.len() > keep {
        let skip = pcm.len() - keep;
        pcm.drain(..skip - skip % channels);
    }
}

/// Make `data` the latest frame. The one it replaces gives its buffer back once no reader holds
/// it any more.
fn publish(shared: &Shared, width: u32, height: u32, data: FrameData) {
    let frame = Arc::new(Frame {
        id: shared.next_frame_id.fetch_add(1, Ordering::Relaxed),
        width,
        height,
        data,
    });
    let previous = lock(&shared.latest).replace(frame);
    // Dropped outside the lock: releasing a readback buffer unmaps it.
    drop(previous);
}
