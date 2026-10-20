//! A MilkDrop-style visualizer: a render thread that turns PCM into BGRA frames.
//!
//! The thread owns a headless wgpu device and a [`lactea`] engine. Each frame it drains the PCM
//! ring buffer and renders. On macOS it draws straight into an IOSurface-backed pixel buffer that
//! the app can show without a copy; elsewhere, or if that fails, it renders into an offscreen
//! `Bgra8Unorm` texture and reads it back as tightly packed pixels. Either way the result is the
//! latest [`Frame`]. Readers ([`Visualizer::latest_frame`]) only clone an `Arc` under a short lock
//! and never wait for the GPU.

mod pack;
mod pipeline;
mod presets;
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

use pack::Zip;
pub use pipeline::Transport;
use pipeline::{Pipeline, Rendered};
use presets::{AutoSwitch, Location, Preset, PresetList};
use textures::TextureSource;

pub use pack::{BUILTIN_NAME, builtin_pack};

pub use lactea::Transition;

pub const DEFAULT_FPS: u32 = 60;

/// Sample rate assumed until the first track tells the real one.
const FALLBACK_SAMPLE_RATE: u32 = 44_100;

/// How often the measured fps in [`Status`] is refreshed.
const FPS_WINDOW: Duration = Duration::from_millis(500);

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
    /// `width * height * 4` bytes, BGRA, rows top to bottom with no padding.
    Bgra(Vec<u8>),
    /// An IOSurface-backed `kCVPixelFormatType_32BGRA` pixel buffer.
    #[cfg(target_os = "macos")]
    PixelBuffer(surface::PixelBuffer),
}

impl Frame {
    /// The packed pixels, if the frame was read back.
    pub fn bgra(&self) -> Option<&[u8]> {
        match &self.data {
            FrameData::Bgra(pixels) => Some(pixels),
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
            FrameData::Bgra(_) => None,
        }
    }

    /// The pixels as packed BGRA, copied out of a pixel buffer if need be. For tests.
    pub fn to_bgra(&self) -> Result<Vec<u8>> {
        match &self.data {
            FrameData::Bgra(pixels) => Ok(pixels.clone()),
            #[cfg(target_os = "macos")]
            FrameData::PixelBuffer(buffer) => buffer.read_bgra(self.width, self.height),
        }
    }
}

/// Where next, previous and random take presets (and textures) from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Source {
    /// Nothing selected yet.
    #[default]
    None,
    /// The pack embedded at build time, if there is one ([`builtin_pack`]).
    Builtin,
    /// The folder given to [`Visualizer::set_preset_dir`].
    Folder,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub running: bool,
    pub width: u32,
    pub height: u32,
    pub target_fps: u32,
    pub measured_fps: f32,
    pub preset_name: String,
    /// Presets in the selected [`Source`].
    pub preset_count: usize,
    pub source: Source,
    /// Whether this build embeds a preset pack.
    pub builtin_available: bool,
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
    SetTextures(TextureSource),
    Stop,
}

/// The preset lists of both sources and which one is selected.
#[derive(Default)]
struct Library {
    source: Source,
    builtin: PresetList,
    folder: PresetList,
    folder_dir: Option<PathBuf>,
}

impl Library {
    fn list(&mut self) -> Result<&mut PresetList> {
        match self.source {
            Source::Builtin => Ok(&mut self.builtin),
            Source::Folder => Ok(&mut self.folder),
            Source::None => bail!("No preset source is selected"),
        }
    }

    /// The preset for an automatic switch, if there are presets to choose from.
    fn auto_next(&mut self, rng: &mut ThreadRng) -> Option<Result<Preset>> {
        let list = self.list().ok().filter(|list| list.len() > 0)?;
        Some(list.next(rng))
    }

    fn texture_source(&self) -> Option<TextureSource> {
        match self.source {
            Source::Builtin => Some(TextureSource::Builtin),
            Source::Folder => self.folder_dir.clone().map(TextureSource::Folder),
            Source::None => None,
        }
    }
}

struct Shared {
    latest: Mutex<Option<Arc<Frame>>>,
    status: Mutex<Status>,
    next_frame_id: AtomicU64,
    /// Whether the automatic preset switching waits (the user locked the preset or a menu is
    /// open).
    auto_switch_paused: AtomicBool,
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
            }),
            worker: Mutex::new(None),
            library: Arc::new(Mutex::new(Library::default())),
        }
    }

    /// Start the render thread. It returns at once; a missing GPU or an engine error shows up
    /// in [`Status::last_error`] with `running` false. Fails if a thread is already there, so
    /// call [`Self::stop`] first (it gives the `PcmInput` back).
    pub fn start(&self, input: PcmInput, width: u32, height: u32, fps: u32) -> Result<()> {
        let (width, height, fps) = (width.max(1), height.max(1), fps.clamp(1, 240));
        // Without a folder the built-in pack, if there is one, is the default source. This
        // comes before the worker lock, which `publish_source` takes to reach the renderer.
        let textures = {
            let mut library = lock(&self.library);
            if library.source == Source::None && builtin_pack().is_some() {
                self.select_builtin(&mut library)?;
            }
            library.texture_source()
        };
        let mut worker = lock(&self.worker);
        if worker.is_some() {
            bail!("The visualizer is already started");
        }
        {
            let mut status = self.shared.status();
            let (preset_count, source) = (status.preset_count, status.source);
            *status = Status {
                running: true,
                width,
                height,
                target_fps: fps,
                preset_count,
                source,
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

    /// Scan `dir` for `.milk` / `.milk2` files for next/previous/random. Returns their count.
    /// The folder becomes the source unless one was chosen already (see [`Self::set_source`]);
    /// its `textures` subfolder, or the folder itself, supplies the textures.
    pub fn set_preset_dir(&self, dir: &Path) -> Result<usize> {
        let mut library = lock(&self.library);
        let count = library.folder.scan(dir)?;
        library.folder_dir = Some(dir.to_path_buf());
        if library.source == Source::None {
            library.source = Source::Folder;
        }
        if library.source == Source::Folder {
            self.publish_source(&library);
        }
        Ok(count)
    }

    /// Choose where presets and textures come from. [`Source::Builtin`] needs a build with the
    /// pack, [`Source::Folder`] a folder from [`Self::set_preset_dir`].
    pub fn set_source(&self, source: Source) -> Result<()> {
        let mut library = lock(&self.library);
        match source {
            Source::None => bail!("A preset source must be Builtin or Folder"),
            Source::Builtin => self.select_builtin(&mut library)?,
            Source::Folder => {
                if library.folder_dir.is_none() {
                    bail!("No preset folder was chosen");
                }
                library.source = Source::Folder;
            }
        }
        self.publish_source(&library);
        Ok(())
    }

    fn select_builtin(&self, library: &mut Library) -> Result<()> {
        if library.builtin.len() == 0 {
            let count = library.builtin.scan_pack(&mut Zip::builtin()?);
            if count == 0 {
                bail!("The built-in preset pack is empty");
            }
        }
        library.source = Source::Builtin;
        self.publish_source(library);
        Ok(())
    }

    /// Show the selection in the status and give the renderer the matching textures.
    fn publish_source(&self, library: &Library) {
        let count = match library.source {
            Source::Builtin => library.builtin.len(),
            Source::Folder => library.folder.len(),
            Source::None => 0,
        };
        {
            let mut status = self.shared.status();
            status.source = library.source;
            status.preset_count = count;
        }
        if let Some(textures) = library.texture_source() {
            // Not running: `start` hands over the current source.
            let _ = self.send(Command::SetTextures(textures));
        }
    }

    /// Forward in the history if [`Self::previous_preset`] went back, else a new random preset
    /// (weighted by the presets' ratings, as in MilkDrop 2).
    pub fn next_preset(&self, transition: Transition) -> Result<()> {
        let preset = lock(&self.library).list()?.next(&mut rand::thread_rng())?;
        self.load_entry(&preset, transition)
    }

    /// One step back in the history of loaded presets; nothing happens at its start.
    pub fn previous_preset(&self, transition: Transition) -> Result<()> {
        let preset = lock(&self.library).list()?.previous()?;
        match preset {
            Some(preset) => self.load_entry(&preset, transition),
            None => Ok(()),
        }
    }

    /// A new random preset (weighted by rating); the history ahead of the current one is dropped.
    pub fn random_preset(&self, transition: Transition) -> Result<()> {
        let preset = lock(&self.library)
            .list()?
            .random(&mut rand::thread_rng())?;
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
        Status {
            builtin_available: builtin_pack().is_some(),
            ..self.shared.status().clone()
        }
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
    match &preset.location {
        Location::File(path) => {
            std::fs::read(path).with_context(|| format!("Failed to read {}", path.display()))
        }
        Location::Pack(path) => Zip::builtin()?.read(path),
    }
}

fn worker_main(
    shared: Arc<Shared>,
    library: Arc<Mutex<Library>>,
    mut input: PcmInput,
    (width, height, fps): (u32, u32, u32),
    textures: Option<TextureSource>,
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
    textures: Option<TextureSource>,
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
    let mut pipeline = Pipeline::new(width, height, fps, sample_rate, on_gpu_error)?;
    let mut transport = pipeline.transport();
    info!(
        "Visualizer started on {} ({transport:?})",
        pipeline.adapter_name
    );
    if let Some(source) = textures
        && let Err(e) = pipeline.set_textures(&source)
    {
        shared.set_error(format!("{e:#}"));
    }
    {
        let mut status = shared.status();
        status.adapter = pipeline.adapter_name.clone();
        status.transport = Some(transport);
        status.preset_name = lactea::IDLE_PRESET_NAME.to_string();
    }

    let interval = Duration::from_secs_f64(1.0 / f64::from(fps));
    let started = Instant::now();
    let mut deadline = started;
    let mut frame_index = 0u32;
    let mut pcm: Vec<f32> = Vec::new();
    let mut pixels: Vec<u8> = Vec::new();
    let mut window_start = started;
    let mut window_frames = 0u32;
    let mut auto_switch = AutoSwitch::default();
    let mut rng = rand::thread_rng();

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
        if !pcm.is_empty() {
            pipeline.push_pcm(&pcm, channels);
        }

        let time = started.elapsed().as_secs_f64();
        let paused = shared.auto_switch_paused.load(Ordering::Relaxed);
        // A scan holds the library while it reads every file; try again on the next frame
        // instead of stalling this one.
        let free = || match library.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        };
        if auto_switch.tick(time, paused, &mut rng)
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
        let rendered = pipeline.render(clock, &mut pixels)?;
        frame_index = frame_index.wrapping_add(1);
        let (width, height) = pipeline.size();
        match rendered {
            Rendered::Pixels => pixels = publish(shared, width, height, pixels),
            #[cfg(target_os = "macos")]
            Rendered::PixelBuffer(buffer) => {
                publish_frame(shared, width, height, FrameData::PixelBuffer(buffer));
            }
            Rendered::Skipped => {}
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
    if matches!(command, Command::LoadPreset { .. }) {
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
        Command::SetTextures(source) => pipeline.set_textures(&source),
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

/// Make `pixels` the latest frame. Returns a buffer to render the next one into: the previous
/// frame's, unless a reader still holds it.
fn publish(shared: &Shared, width: u32, height: u32, pixels: Vec<u8>) -> Vec<u8> {
    let previous = publish_frame(shared, width, height, FrameData::Bgra(pixels));
    match previous.and_then(|frame| Arc::try_unwrap(frame).ok()) {
        Some(Frame {
            data: FrameData::Bgra(pixels),
            ..
        }) => pixels,
        _ => Vec::new(),
    }
}

/// Make `data` the latest frame and return the one it replaces.
fn publish_frame(shared: &Shared, width: u32, height: u32, data: FrameData) -> Option<Arc<Frame>> {
    let frame = Arc::new(Frame {
        id: shared.next_frame_id.fetch_add(1, Ordering::Relaxed),
        width,
        height,
        data,
    });
    lock(&shared.latest).replace(frame)
}
