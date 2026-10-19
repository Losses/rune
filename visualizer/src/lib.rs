//! A MilkDrop-style visualizer: a render thread that turns PCM into BGRA frames.
//!
//! The thread owns a headless wgpu device and a [`lactea`] engine. Each frame it drains the PCM
//! ring buffer, renders into an offscreen `Bgra8Unorm` texture, reads it back and publishes the
//! tightly packed pixels as the latest [`Frame`]. Readers ([`Visualizer::latest_frame`]) only
//! clone an `Arc` under a short lock and never wait for the GPU.

mod pipeline;
mod presets;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use lactea::FrameClock;
use log::{error, info};
use rtrb::Consumer;

use pipeline::Pipeline;
use presets::PresetList;

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
    /// `width * height * 4` bytes, BGRA, rows top to bottom with no padding.
    pub bgra: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub running: bool,
    pub width: u32,
    pub height: u32,
    pub target_fps: u32,
    pub measured_fps: f32,
    pub preset_name: String,
    pub preset_count: usize,
    pub adapter: String,
    pub last_error: Option<String>,
}

enum Command {
    Resize(u32, u32),
    LoadPreset {
        bytes: Vec<u8>,
        name: String,
        transition: Transition,
    },
    Stop,
}

struct Shared {
    latest: Mutex<Option<Arc<Frame>>>,
    status: Mutex<Status>,
    next_frame_id: AtomicU64,
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
    presets: Mutex<PresetList>,
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
            }),
            worker: Mutex::new(None),
            presets: Mutex::new(PresetList::default()),
        }
    }

    /// Start the render thread. It returns at once; a missing GPU or an engine error shows up
    /// in [`Status::last_error`] with `running` false. Fails if a thread is already there, so
    /// call [`Self::stop`] first (it gives the `PcmInput` back).
    pub fn start(&self, input: PcmInput, width: u32, height: u32, fps: u32) -> Result<()> {
        let mut worker = lock(&self.worker);
        if worker.is_some() {
            bail!("The visualizer is already started");
        }
        let (width, height, fps) = (width.max(1), height.max(1), fps.clamp(1, 240));
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
        let (commands, receiver) = mpsc::channel();
        let shared = Arc::clone(&self.shared);
        let handle = thread::Builder::new()
            .name("visualizer".into())
            .spawn(move || worker_main(shared, input, (width, height, fps), receiver))
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
        let bytes =
            std::fs::read(path).with_context(|| format!("Failed to read {}", path.display()))?;
        let name = path
            .file_stem()
            .map_or_else(|| "preset".into(), |s| s.to_string_lossy().into_owned());
        self.load_preset(bytes, &name, transition)
    }

    /// Scan `dir` for `.milk` / `.milk2` files for next/previous/random. Returns their count.
    pub fn set_preset_dir(&self, dir: &Path) -> Result<usize> {
        let count = lock(&self.presets).scan(dir)?;
        self.shared.status().preset_count = count;
        Ok(count)
    }

    pub fn next_preset(&self, transition: Transition) -> Result<()> {
        let path = lock(&self.presets).next()?;
        self.load_preset_path(&path, transition)
    }

    pub fn previous_preset(&self, transition: Transition) -> Result<()> {
        let path = lock(&self.presets).previous()?;
        self.load_preset_path(&path, transition)
    }

    pub fn random_preset(&self, transition: Transition) -> Result<()> {
        let path = lock(&self.presets).random()?;
        self.load_preset_path(&path, transition)
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

fn worker_main(
    shared: Arc<Shared>,
    mut input: PcmInput,
    (width, height, fps): (u32, u32, u32),
    commands: Receiver<Command>,
) -> PcmInput {
    let result = catch_unwind(AssertUnwindSafe(|| {
        run(&shared, &mut input, width, height, fps, &commands)
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
    input: &mut PcmInput,
    width: u32,
    height: u32,
    fps: u32,
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
    info!("Visualizer started on {}", pipeline.adapter_name);
    {
        let mut status = shared.status();
        status.adapter = pipeline.adapter_name.clone();
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

    loop {
        loop {
            match commands.try_recv() {
                Ok(Command::Stop) | Err(TryRecvError::Disconnected) => return Ok(()),
                Ok(command) => apply(shared, &mut pipeline, command),
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

        let clock = FrameClock {
            time: started.elapsed().as_secs_f64(),
            frame: frame_index,
        };
        pipeline.render(clock, &mut pixels)?;
        frame_index = frame_index.wrapping_add(1);
        let (width, height) = pipeline.size();
        pixels = publish(shared, width, height, pixels);

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
                Ok(command) => apply(shared, &mut pipeline, command),
                Err(RecvTimeoutError::Timeout) => {}
            }
        } else {
            // Rendering is slower than the target: do not try to catch up.
            deadline = now;
        }
    }
}

fn apply(shared: &Shared, pipeline: &mut Pipeline, command: Command) {
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
    let frame = Arc::new(Frame {
        id: shared.next_frame_id.fetch_add(1, Ordering::Relaxed),
        width,
        height,
        bgra: pixels,
    });
    let previous = lock(&shared.latest).replace(frame);
    previous
        .and_then(|frame| Arc::try_unwrap(frame).ok())
        .map(|frame| frame.bgra)
        .unwrap_or_default()
}
