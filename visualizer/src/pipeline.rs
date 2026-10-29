//! The GPU side: a headless device, the scene being drawn and where frames go: on Apple platforms straight into
//! IOSurface-backed pixel buffers ([`crate::surface`]), on Windows into DirectX 12 textures shared
//! with Flutter's adapter (`crate::shared_dx12`), on Linux into Vulkan images that Flutter's
//! OpenGL context imports (`crate::shared_vulkan`), on Android presented into the native window
//! of a Flutter texture (`crate::window`), elsewhere (or if that fails) an offscreen target that
//! is read back into mapped buffers ([`crate::readback`]).
//!
//! Frames handed to Flutter without a copy are submitted without waiting for the GPU and handed
//! out once it has finished them (see [`crate::in_flight`]).

use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

#[cfg(target_os = "android")]
use anyhow::bail;
use anyhow::{Context, Result, anyhow};
use log::{info, warn};

use crate::adaptive::{Controller, Level, Report};
use crate::bind::RuntimeBindings;
use crate::chart::ChartState;
use crate::covers::{Atlas, Cell};
use crate::gpu_time::GpuTimer;
use crate::in_flight::InFlight;
use crate::inspect::{LabLens, LabOverrides, LabPace, LabTransition};
use crate::moments::Chances;
use crate::readback::{MappedFrame, Target};
use crate::rhythm::Rhythm;
use crate::scene::{DirectorView, FrameInput, LooksView, MusicValues, Scene, Source};
#[cfg(target_os = "windows")]
use crate::shared_dx12::{SharedTargets, SharedTexture};
#[cfg(target_os = "linux")]
use crate::shared_vulkan::{SharedFrame, SharedTargets};
#[cfg(target_vendor = "apple")]
use crate::surface::{PixelBuffer, SurfaceTargets};
use crate::transition::{
    Dressed, Durations, FadePhase, Forced, Overlay, OverlayRun, Plan, Punch, Running, Transition,
    Transitions, cutin_frame, dissolve_progress, flood_color, seed_of,
};
#[cfg(target_os = "android")]
use crate::window::{Window, WindowSurface};
use crate::{DEFAULT_FPS, DeviceUuids, switching};

/// `kCVPixelFormatType_32BGRA` on Apple. Windows' `FlutterDesktopPixelBuffer` and Linux's
/// `FlPixelBufferTexture` take RGBA, and so do ANGLE for a shared texture and Linux's `FlTextureGL`
/// (`GL_RGBA8`). Scenes draw the target as it is, so with a linear format the bytes are exactly
/// what the scene drew. Android's swapchains take RGBA too (see `crate::window`).
#[cfg(target_vendor = "apple")]
pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
#[cfg(not(target_vendor = "apple"))]
pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub struct Pipeline {
    device: wgpu::Device,
    queue: wgpu::Queue,
    scene: Scene,
    /// The scene switches that are not cuts, and the textures they need while they run.
    transitions: Transitions,
    /// What a scene lab replaces of the choices made by themselves.
    lab: LabOverrides,
    sample_rate: u32,
    rhythm: Rhythm,
    chart: Option<ChartState>,
    song: (f64, bool),
    /// The app's accent colour (linear), once the app has told it.
    accent: Option<[f32; 3]>,
    /// The chances the music's moments play an effect with (see [`crate::moments`]).
    chances: Chances,
    /// The runtime's decisions (`runtime.toml`), which every scene reads.
    runtime: Arc<RuntimeBindings>,
    /// The atlas of covers for scenes that read one.
    covers: Option<Arc<Atlas>>,
    /// The cell of the track playing, which stays over cell 0 of the atlas.
    playing_cover: Option<Arc<Cell>>,
    output: Output,
    /// Frames drawn into the output's targets that the GPU may not have finished yet.
    in_flight: InFlight<Rendered>,
    /// Why the device was lost, once it was. Nothing waits for a particular frame any more, so
    /// without this a lost device would go on taking frames that never draw anything.
    lost: Arc<OnceLock<String>>,
    /// The format scenes draw in: [`FORMAT`], or what the window takes (Android).
    format: wgpu::TextureFormat,
    readback: bool,
    pub adapter_name: String,
    /// The GPU time of each frame, which the adaptive quality follows.
    timer: GpuTimer,
    controller: Controller,
    report: Report,
    /// The clock of the controller and the report.
    clock: Instant,
    /// Seconds per frame at the target frame rate.
    budget: f64,
    /// Frame times collected this frame (kept to save allocating).
    times: Vec<f64>,
    /// Whether the controller may change the scene's level (see [`Pipeline::set_adaptive`]).
    adaptive: bool,
    /// The GPU time of every frame since the last [`Pipeline::take_frame_times`], when the
    /// benchmark asked for them.
    recorded: Option<Vec<f64>>,
    /// Frames not drawn because every target was still in use, so far and at the last report.
    skipped: (u64, u64),
    /// The GPU time of the frame whose time arrived last, in seconds.
    last_gpu: Option<f64>,
}

/// How often the GPU time of the frames is logged, in seconds.
const REPORT_INTERVAL: f64 = 10.0;
/// How long the frames on their way are waited for when they are let go of (a resize, the end).
/// Longer than this, the GPU is stuck or lost.
const FINISH_WAIT: Duration = Duration::from_secs(1);
/// How often a wait for a frame that is not the newest submission looks whether the GPU finished
/// it (see [`Pipeline::wait_for_oldest`]).
const WAIT_STEP: Duration = Duration::from_millis(1);

/// The name of the built-in scene shown before another is chosen.
pub const DEFAULT_PRESET_NAME: &str = "Armillary";

/// The built-in scene called `name`.
fn builtin_source(name: &str) -> Result<Source> {
    crate::builtin::find(name)
        .map(|scene| scene.source)
        .with_context(|| format!("There is no built-in scene {name}"))
}

/// A scene to build away from the render thread: everything [`Scene::load_from`] needs, taken
/// from the pipeline when the load was asked for.
pub struct SceneBuild {
    device: wgpu::Device,
    queue: wgpu::Queue,
    source: Source,
    format: wgpu::TextureFormat,
    size: (u32, u32),
    sample_rate: u32,
    /// What the error says the scene was, when it fails to build.
    context: String,
}

impl SceneBuild {
    /// Read and build the scene, which blocks for as long as the driver takes to compile it.
    pub fn run(self) -> Result<Scene> {
        let context = self.context.clone();
        self.run_bare().context(context)
    }

    fn run_bare(self) -> Result<Scene> {
        // wgpu's Vulkan backend compiles shaders with naga, which panics on the odd one (a struct
        // field's address as a call argument, for one). That happens on this thread; without the
        // guard the scene would never come in and the status would say nothing.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Scene::load_from(
                &self.device,
                &self.queue,
                self.source,
                self.format,
                self.size,
                self.sample_rate,
            )
        }))
        .unwrap_or_else(|payload| {
            let said = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned());
            Err(anyhow!(
                "The graphics driver panicked while building the scene{}",
                said.map(|s| format!(": {s}")).unwrap_or_default()
            ))
        })
    }
}

/// What a rendered frame became.
pub enum Rendered {
    /// A read-back frame (on the readback path a frame comes out a call or two after it was
    /// drawn).
    Mapped(MappedFrame),
    #[cfg(target_vendor = "apple")]
    PixelBuffer(PixelBuffer),
    #[cfg(target_os = "windows")]
    Shared(SharedTexture),
    #[cfg(target_os = "linux")]
    Shared(SharedFrame),
    /// Android: presented into the window, where Flutter takes it.
    #[cfg(target_os = "android")]
    Presented,
    /// Nothing new: every buffer is still in use, or the GPU has not finished one yet.
    Nothing,
}

/// How frames leave the GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Readback,
    /// Apple: IOSurface-backed pixel buffers.
    IoSurface,
    /// Windows: DirectX 12 textures shared through NT handles.
    DxgiShared,
    /// Linux: Vulkan images whose memory Flutter's OpenGL context imports.
    VulkanShared,
    /// Android: presented into the native window of a Flutter texture.
    Surface,
}

enum Output {
    Readback(Target),
    #[cfg(target_vendor = "apple")]
    Surfaces(SurfaceTargets),
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    Shared(SharedTargets),
    #[cfg(target_os = "android")]
    Surface(WindowSurface),
}

impl Output {
    /// Pixel buffers on Apple platforms, shared textures on Windows and shared images on Linux unless
    /// `readback`, else (or if they fail) the readback target.
    fn new(device: &wgpu::Device, width: u32, height: u32, readback: bool) -> Self {
        #[cfg(target_vendor = "apple")]
        if !readback {
            match SurfaceTargets::new(device, width, height) {
                Ok(surfaces) => return Self::Surfaces(surfaces),
                Err(e) => warn!("Falling back to reading frames back: {e:#}"),
            }
        }
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        if !readback {
            match SharedTargets::new(device, width, height) {
                Ok(targets) => return Self::Shared(targets),
                Err(e) => warn!("Falling back to reading frames back: {e:#}"),
            }
        }
        #[cfg(not(any(target_vendor = "apple", target_os = "windows", target_os = "linux")))]
        let _ = readback;
        Self::Readback(Target::new(device, width, height))
    }

    fn size(&self) -> (u32, u32) {
        match self {
            Self::Readback(target) => (target.width, target.height),
            #[cfg(target_vendor = "apple")]
            Self::Surfaces(surfaces) => surfaces.size(),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            Self::Shared(targets) => targets.size(),
            #[cfg(target_os = "android")]
            Self::Surface(surface) => surface.size(),
        }
    }
}

/// The adapter to render on, and whether frames may leave it without being read back. On Windows
/// that is only so on the adapter Flutter renders on (`adapter_luid`, see
/// [`crate::Visualizer::set_graphics_adapter`]), because a shared texture cannot be opened on
/// another one; on Linux likewise only on the GPU and driver of Flutter's OpenGL context
/// (`device_uuids`, see [`crate::Visualizer::set_graphics_device`]). Without it, the adapter is
/// chosen as elsewhere and frames are read back. Elsewhere the adapter can present into
/// `compatible_surface`, if there is one.
fn choose_adapter(
    instance: &wgpu::Instance,
    compatible_surface: Option<&wgpu::Surface<'_>>,
    adapter_luid: Option<u64>,
    device_uuids: Option<DeviceUuids>,
) -> Result<(wgpu::Adapter, bool)> {
    #[cfg(target_os = "windows")]
    match adapter_luid {
        Some(luid) => match crate::shared_dx12::adapter_with_luid(instance, luid) {
            Some(adapter) => return Ok((adapter, true)),
            None => {
                warn!("No DirectX 12 adapter has Flutter's LUID {luid:#x}; reading frames back")
            }
        },
        None => log::info!("Flutter's adapter is unknown; reading frames back"),
    }
    #[cfg(not(target_os = "windows"))]
    let _ = adapter_luid;
    #[cfg(target_os = "linux")]
    match device_uuids {
        Some(uuids) => match crate::shared_vulkan::adapter_with_uuids(instance, &uuids) {
            Some(adapter) => return Ok((adapter, true)),
            None => warn!("No Vulkan adapter is the GPU Flutter renders on; reading frames back"),
        },
        None => log::info!("Flutter's GPU is unknown; reading frames back"),
    }
    #[cfg(not(target_os = "linux"))]
    let _ = device_uuids;
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface,
        ..Default::default()
    }))
    .map_err(|e| anyhow!("No GPU adapter: {e}"))?;
    // Without a DirectX 12 GPU wgpu falls back to WARP, Windows' software renderer, whose shader
    // compiler crashes the whole process on most scenes (its control-flow analysis overruns its
    // own stack), so the visualizer does not start there.
    #[cfg(target_os = "windows")]
    if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
        return Err(anyhow!(
            "The visualizer needs a DirectX 12 GPU; only Windows' software renderer is available"
        ));
    }
    Ok((
        adapter,
        cfg!(not(any(target_os = "windows", target_os = "linux"))),
    ))
}

/// The device and queue on `adapter`, and whether frames may leave it without being read back,
/// which is `share` except on Linux when the device cannot export semaphores (see
/// [`crate::shared_vulkan::request_device`]).
fn request_device(
    adapter: &wgpu::Adapter,
    share: bool,
) -> Result<(wgpu::Device, wgpu::Queue, bool)> {
    #[cfg(target_os = "linux")]
    if share {
        match crate::shared_vulkan::request_device(adapter) {
            Ok((device, queue)) => return Ok((device, queue, true)),
            Err(e) => warn!("Falling back to reading frames back: {e:#}"),
        }
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        // For the GPU time of a frame (see `crate::gpu_time`), where there are timestamps.
        required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))
    .context("Failed to create the GPU device")?;
    Ok((device, queue, share && cfg!(not(target_os = "linux"))))
}

/// Draw a frame with `paint` and submit it, without waiting for the GPU.
#[cfg(any(target_vendor = "apple", target_os = "windows"))]
fn render_and_submit(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    paint: impl FnOnce(&mut wgpu::CommandEncoder),
) {
    let mut encoder = device.create_command_encoder(&Default::default());
    paint(&mut encoder);
    queue.submit(Some(encoder.finish()));
}

/// Hand `frame` out once the GPU has finished the submission just made on `queue`.
#[cfg(any(target_vendor = "apple", target_os = "windows", target_os = "linux"))]
fn track(queue: &wgpu::Queue, in_flight: &mut InFlight<Rendered>, frame: Rendered) {
    let submitted = Instant::now();
    let done = in_flight.push(frame, submitted);
    // Runs inside a later poll or submission on this thread.
    queue.on_submitted_work_done(move || {
        let _ = done.set(Instant::now());
    });
}

impl Pipeline {
    /// `adapter_luid` and `device_uuids`: the GPU Flutter renders on, which only Windows and
    /// Linux use (see [`choose_adapter`]). Android: frames are presented into `window` (unless
    /// `readback`, or without a window: then they are read back, and nothing shows them); a
    /// window the GPU cannot present into is an error, since nothing else would show the frames.
    #[cfg_attr(target_os = "android", allow(clippy::too_many_arguments))]
    pub fn new(
        width: u32,
        height: u32,
        sample_rate: u32,
        readback: bool,
        adapter_luid: Option<u64>,
        device_uuids: Option<DeviceUuids>,
        on_gpu_error: Arc<dyn Fn(String) + Send + Sync>,
        #[cfg(target_os = "android")] window: Option<Window>,
    ) -> Result<Self> {
        let instance = wgpu::Instance::default();
        // The surface comes first, so that the adapter is one that can present into it.
        #[cfg(target_os = "android")]
        let surface = match window {
            Some(window) if !readback => Some(crate::window::create_surface(&instance, window)?),
            _ => None,
        };
        #[cfg(target_os = "android")]
        let compatible_surface = surface.as_ref();
        #[cfg(not(target_os = "android"))]
        let compatible_surface = None;
        let (adapter, shareable) =
            choose_adapter(&instance, compatible_surface, adapter_luid, device_uuids)?;
        let (device, queue, shareable) = request_device(&adapter, shareable && !readback)?;
        let readback = !shareable;
        // Without a handler wgpu panics on validation errors.
        #[cfg(target_os = "android")]
        let report = Arc::clone(&on_gpu_error);
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| on_gpu_error(e.to_string())));
        let lost = Arc::new(OnceLock::new());
        device.set_device_lost_callback({
            let lost = Arc::clone(&lost);
            move |reason, message| {
                let _ = lost.set(format!("{reason:?}: {message}"));
            }
        });

        let (width, height) = (width.max(1), height.max(1));
        #[cfg(target_os = "android")]
        let output = match surface {
            Some(surface) => {
                if !adapter.is_surface_supported(&surface) {
                    bail!("The GPU cannot present into the visualizer's window");
                }
                Output::Surface(WindowSurface::new(
                    &instance,
                    &adapter,
                    &device,
                    surface,
                    (width, height),
                    report,
                )?)
            }
            None => Output::new(&device, width, height, readback),
        };
        #[cfg(not(target_os = "android"))]
        let output = Output::new(&device, width, height, readback);
        let format = match &output {
            #[cfg(target_os = "android")]
            Output::Surface(surface) => surface.format(),
            _ => FORMAT,
        };
        let scene = Scene::load_from(
            &device,
            &queue,
            builtin_source(DEFAULT_PRESET_NAME)?,
            format,
            (width, height),
            sample_rate,
        )
        .context("The default scene")?;
        let timer = GpuTimer::new(&device, &queue);
        info!(
            "Visualizer frame times come from {}",
            if timer.has_timestamps() {
                "GPU timestamps"
            } else {
                "waiting for each frame"
            }
        );
        let transitions = Transitions::new(&device, format);
        let clock = Instant::now();
        let budget = 1.0 / f64::from(DEFAULT_FPS);
        Ok(Self {
            timer,
            controller: Controller::new(budget, 0.0),
            report: Report::new(REPORT_INTERVAL),
            clock,
            budget,
            times: Vec::new(),
            adaptive: true,
            recorded: None,
            skipped: (0, 0),
            last_gpu: None,
            adapter_name: adapter.get_info().name,
            device,
            transitions,
            lab: LabOverrides::default(),
            queue,
            scene,
            sample_rate,
            rhythm: Rhythm::default(),
            chart: None,
            song: (0.0, false),
            accent: None,
            chances: Chances::default(),
            runtime: RuntimeBindings::embedded(),
            covers: None,
            playing_cover: None,
            output,
            in_flight: InFlight::new(),
            lost,
            format,
            readback,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.output.size()
    }

    /// The frame rate the adaptive quality keeps frames within.
    pub fn set_target_fps(&mut self, fps: u32) {
        self.budget = 1.0 / f64::from(fps.max(1));
        self.controller.set_budget(self.budget);
    }

    /// Whether the scene's resolution and quality follow the GPU time. Off, the scene stays at
    /// full level; on, the controller starts again from full (as on a new scene).
    pub fn set_adaptive(&mut self, adaptive: bool) {
        self.adaptive = adaptive;
        self.restart_adaptive();
        self.scene.set_level(&self.device, &self.queue, Level::FULL);
    }

    /// The scene's level and its internal size at the moment.
    pub fn level(&self) -> (Level, (u32, u32)) {
        (self.controller.level(), self.scene.internal_size())
    }

    /// Whether GPU times come from timestamp queries (else from waiting for each frame).
    pub fn has_timestamps(&self) -> bool {
        self.timer.has_timestamps()
    }

    /// Start (or stop) keeping the GPU time of every frame for [`Pipeline::take_frame_times`].
    pub fn record_frame_times(&mut self, record: bool) {
        self.recorded = record.then(Vec::new);
    }

    /// The GPU time of the frames (seconds) that finished since the last call.
    pub fn take_frame_times(&mut self) -> Vec<f64> {
        self.recorded
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// How many frames were not drawn so far because every target was still in use.
    pub fn skipped(&self) -> u64 {
        self.skipped.0
    }

    fn now(&self) -> f64 {
        self.clock.elapsed().as_secs_f64()
    }

    /// A new scene (or a new version of it) starts at full quality, and frames still in flight
    /// do not count for it.
    fn restart_adaptive(&mut self) {
        self.times.clear();
        self.timer
            .collect(&self.device, &self.queue, &mut self.times);
        self.times.clear();
        self.controller = Controller::new(self.budget, self.now());
        self.report.clear();
        if let Some(recorded) = &mut self.recorded {
            recorded.clear();
        }
    }

    /// Follow the GPU time of the frames that finished: log it now and then, and change the
    /// scene's resolution and quality when the controller says so.
    fn adapt(&mut self) {
        self.timer
            .collect(&self.device, &self.queue, &mut self.times);
        let now = self.now();
        self.report.tick(now);
        let mut change = None;
        if let Some(recorded) = &mut self.recorded {
            recorded.extend_from_slice(&self.times);
        }
        for &seconds in &self.times {
            if let Some(summary) = self.report.frame(now, seconds) {
                let level = self.controller.level();
                let (width, height) = self.scene.internal_size();
                let (skipped, reported) = &mut self.skipped;
                info!(
                    "Visualizer {}: {:.1} fps, GPU {:.1} ms on average, {:.1} ms at worst ({} frames, {} skipped without a free target); now resolution {:.2} ({width}x{height}), quality {:.2}",
                    self.scene.name,
                    summary.fps,
                    summary.average * 1e3,
                    summary.worst * 1e3,
                    summary.frames,
                    *skipped - *reported,
                    level.factor,
                    level.quality,
                );
                *reported = *skipped;
            }
            if self.adaptive {
                self.controller.sample(now, seconds);
            }
        }
        if let Some(&seconds) = self.times.last() {
            self.last_gpu = Some(seconds);
        }
        self.times.clear();
        if self.adaptive {
            change = self.controller.tick(now);
        }
        if let Some(change) = change {
            if let Some(undone) = change.undone {
                info!(
                    "Visualizer {}: {} did not help ({:.1} fps before, {:.1} after); back to resolution {:.2}, quality {:.2}",
                    self.scene.name,
                    if undone.raise {
                        "the raise"
                    } else {
                        "the lowering"
                    },
                    undone.fps_before,
                    undone.fps_after,
                    change.level.factor,
                    change.level.quality,
                );
            }
            self.scene
                .set_level(&self.device, &self.queue, change.level);
        }
    }

    /// The name of the scene on screen.
    ///
    /// A scene waiting for a transition to reach it counts as the one on screen: this is what
    /// the app shows as the name of the scene.
    pub fn preset_name(&self) -> &str {
        &self.latest().name
    }

    /// The music's values of the frame drawn last, for the switching that follows the music. They
    /// are the drawn scene's, which a switch that is still fading out has not replaced yet.
    pub fn music(&self) -> &MusicValues {
        self.scene.music()
    }

    /// The GPU time in seconds of the newest frame whose time has arrived, if frames are timed.
    pub fn last_gpu_seconds(&self) -> Option<f64> {
        self.last_gpu
    }

    /// Whether the bindings of the scene being drawn heard a cut in the last frame.
    pub fn bound_cut(&self) -> bool {
        self.scene.bound_cut()
    }

    /// What a readout shows of the camera director of the scene being drawn.
    pub fn director_view(&self) -> Option<DirectorView> {
        self.scene.director_view()
    }

    /// What a readout shows of the looks of the scene being drawn.
    pub fn looks_view(&self) -> Option<LooksView> {
        self.scene.looks_view()
    }

    /// The transition under way at `time` (the render loop's) and how far it is, 0 to 1 over
    /// the whole of it (a fade's out and in together).
    pub fn transition_progress(&self, time: f64) -> Option<(Transition, f32)> {
        let Some(running) = self.transitions.running.as_ref() else {
            // A transition that dresses a camera cut.
            let dressed = self.transitions.dressed?;
            return Some((dressed.kind, dressed.progress(time - dressed.start)));
        };
        let seconds = running.start.map_or(0.0, |start| time - start).max(0.0);
        let Durations { out, into } = running.durations;
        let progress = match running.kind {
            Transition::Cut => 1.0,
            Transition::Fade | Transition::Flash => {
                (seconds / (out + into).max(f64::EPSILON)).clamp(0.0, 1.0)
            }
            // The new scene goes in with the second frame, which sets the start.
            Transition::Dissolve | Transition::Glitch | Transition::Mosaic
                if running.pending.is_some() =>
            {
                0.0
            }
            Transition::Dissolve | Transition::Glitch | Transition::Mosaic => {
                f64::from(dissolve_progress(seconds, into))
            }
            Transition::CutIn(_) => (seconds / (out + into).max(f64::EPSILON)).clamp(0.0, 1.0),
        };
        Some((running.kind, progress as f32))
    }

    /// The camera director of the newest scene cuts now (see [`Scene::cut_now`]).
    pub fn cut_now(&mut self) {
        self.latest_mut().cut_now();
    }

    /// The camera director cuts now, dressed with `transition` even if the scene does not list
    /// it and the cue is weak. The cut happens a frame later, once a frame is held for it to
    /// leave from; a scene without shots has no cut to dress, and a switch under way takes the
    /// place of it.
    pub fn cut_dressed(&mut self, transition: Transition) {
        if transition == Transition::Cut {
            return self.cut_now();
        }
        self.transitions.forced = Forced::Wanted(transition);
    }

    /// The camera director of the newest scene starts a burst of cuts now (see
    /// [`Scene::start_burst`]); each cut gets its punch.
    pub fn start_burst(&mut self) {
        self.latest_mut().start_burst();
    }

    /// The newest scene starts a lens flare now (see [`Scene::start_flare`]).
    pub fn start_flare(&mut self) {
        self.latest_mut().start_flare();
    }

    /// The newest scene starts a light leak now (see [`Scene::start_leak`]).
    pub fn start_leak(&mut self) {
        self.latest_mut().start_leak();
    }

    /// The newest scene plays a glitch or a mosaic over its picture from the next frame (see
    /// [`Scene::start_overlay`]).
    pub fn start_overlay(&mut self, kind: Overlay) {
        self.latest_mut().start_overlay(kind);
    }

    /// Which lens effects the scene on screen has on.
    pub fn lens_view(&self) -> crate::scene::LensView {
        self.scene.lens_view()
    }

    /// The glitch or mosaic playing over the picture, if one is.
    pub fn overlay_running(&self) -> Option<Overlay> {
        self.transitions.overlay.map(|run| run.kind)
    }

    /// What a scene lab replaces of the choices made by themselves.
    pub fn lab(&self) -> LabOverrides {
        self.lab
    }

    /// What each binding of the scene being drawn said in its last frame.
    pub fn bindings(&self) -> Vec<crate::bind::Report> {
        self.scene.bindings()
    }

    /// The music is held at this fallback level from the next frame on, or with none left as it
    /// is (see [`crate::bind::forced_chart`]; the render loop applies it).
    pub fn set_lab_level(&mut self, level: Option<crate::bind::Level>) {
        self.lab.level = level;
    }

    /// Every scene switch takes `transition` from now on, or with none what was chosen (see
    /// [`switching::replaced`]; the render loop asks [`Pipeline::lab`]).
    pub fn set_lab_switch(&mut self, transition: Option<LabTransition>) {
        self.lab.switch = transition;
    }

    /// Every camera cut is dressed with `transition` from now on (a cut for a plain one), or with
    /// none as the scene and the moment say (see [`switching::cut_dressing`]). A cut-in is not
    /// drawn over a camera cut and counts as none.
    pub fn set_lab_cuts(&mut self, transition: Option<LabTransition>) {
        self.lab.cuts = transition;
        self.transitions.lab_cuts = transition
            .filter(|t| !matches!(t, LabTransition::CutIn(_)))
            .map(Transition::from);
    }

    /// Every lifting section start bursts (`Some(true)`), none does (`Some(false)`), or the
    /// runtime's chance decides (none), in the scene on screen, the one waiting for a transition
    /// and every scene that comes (see [`Scene::set_bursts`]).
    pub fn set_lab_bursts(&mut self, bursts: Option<bool>) {
        self.lab.bursts = bursts;
        self.scene.set_bursts(bursts);
        if let Some(pending) = self
            .transitions
            .running
            .as_mut()
            .and_then(|running| running.pending.as_mut())
        {
            pending.set_bursts(bursts);
        }
    }

    /// These lens effects start at every moment, in the scene on screen, the one waiting for a
    /// transition and every scene that comes (see [`Scene::set_lens_forced`]).
    pub fn set_lab_lens(&mut self, lens: LabLens) {
        self.lab.lens = lens;
        self.scene.set_lens_forced(lens);
        if let Some(pending) = self
            .transitions
            .running
            .as_mut()
            .and_then(|running| running.pending.as_mut())
        {
            pending.set_lens_forced(lens);
        }
    }

    /// The newest scene's director uses `pace` in place of the scene's own, or with none its own
    /// again (see [`Scene::set_pace`]).
    pub fn set_pace(&mut self, pace: Option<LabPace>) {
        self.latest_mut().set_pace(pace.map(Into::into));
    }

    /// The newest scene's director holds shot `index`, or with none lets go of a held shot (see
    /// [`Scene::hold_shot`]); whether it could.
    pub fn hold_shot(&mut self, index: Option<u32>) -> bool {
        self.latest_mut().hold_shot(index)
    }

    /// The newest scene holds look `index`, or with none hands its look back to the runtime (see
    /// [`Scene::hold_look`]); whether it could.
    pub fn hold_look(&mut self, index: Option<u32>) -> bool {
        self.latest_mut().hold_look(index)
    }

    /// The newest scene's director steps to the next pace (see [`Scene::cycle_pace`]).
    pub fn cycle_pace(&mut self) {
        self.latest_mut().cycle_pace();
    }

    /// The transitions and lens effects the scene on screen declares.
    pub fn effects(&self) -> &crate::scene::Effects {
        self.scene.effects()
    }

    /// The newest scene: the one waiting to be installed when a transition runs, else the one
    /// being drawn.
    fn latest(&self) -> &Scene {
        self.transitions
            .running
            .as_ref()
            .and_then(|running| running.pending.as_ref())
            .unwrap_or(&self.scene)
    }

    /// The same, to change.
    fn latest_mut(&mut self) -> &mut Scene {
        self.transitions
            .running
            .as_mut()
            .and_then(|running| running.pending.as_mut())
            .unwrap_or(&mut self.scene)
    }

    pub fn transport(&self) -> Transport {
        match self.output {
            Output::Readback(_) => Transport::Readback,
            #[cfg(target_vendor = "apple")]
            Output::Surfaces(_) => Transport::IoSurface,
            #[cfg(target_os = "windows")]
            Output::Shared(_) => Transport::DxgiShared,
            #[cfg(target_os = "linux")]
            Output::Shared(_) => Transport::VulkanShared,
            #[cfg(target_os = "android")]
            Output::Surface(_) => Transport::Surface,
        }
    }

    /// Read frames back from the next frame on, also after a resize: the app could not show the
    /// frames this output hands over. Frames presented into a window (Android) hand nothing over,
    /// and read-back frames would show nowhere there, so they stay as they are.
    pub fn fall_back_to_readback(&mut self) {
        #[cfg(target_os = "android")]
        if matches!(self.output, Output::Surface(_)) {
            return;
        }
        self.readback = true;
        if !matches!(self.output, Output::Readback(_)) {
            let (width, height) = self.output.size();
            self.read_back(width, height);
        }
    }

    /// Read frames of `width` by `height` back from now on. Frames on their way in the targets
    /// before are let go of.
    fn read_back(&mut self, width: u32, height: u32) {
        self.finish_in_flight();
        self.output = Output::Readback(Target::new(&self.device, width, height));
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let (width, height) = (width.max(1), height.max(1));
        self.scene
            .resize(&self.device, &self.queue, width, height)?;
        let now = self.now();
        self.controller.settle(now);
        #[cfg(target_os = "android")]
        if let Output::Surface(surface) = &mut self.output {
            surface.resize(&self.device, width, height);
            return self.end_transition();
        }
        // They are of the old size, and the targets they are in go with the old output.
        self.finish_in_flight();
        self.output = Output::new(&self.device, width, height, self.readback);
        self.end_transition()
    }

    /// Android: present frames of `width` by `height` into `window` from now on, in place of the
    /// window before (a Flutter texture hands out a new window when its size changes). Frames
    /// that are read back stay so, and `window` is let go of.
    #[cfg(target_os = "android")]
    pub fn replace_window(&mut self, window: Window, width: u32, height: u32) -> Result<()> {
        let (width, height) = (width.max(1), height.max(1));
        self.scene
            .resize(&self.device, &self.queue, width, height)?;
        let now = self.now();
        self.controller.settle(now);
        match &mut self.output {
            Output::Surface(surface) => {
                surface.replace(&self.device, window, width, height)?;
                self.end_transition()
            }
            Output::Readback(_) => {
                warn!("Frames are read back, so the visualizer does not present into its window");
                drop(window);
                self.output = Output::Readback(Target::new(&self.device, width, height));
                self.end_transition()
            }
        }
    }

    /// Keep the beat tracker's findings for the scenes.
    pub fn set_rhythm(&mut self, rhythm: &Rhythm) {
        self.rhythm = *rhythm;
    }

    /// The song's chart at this frame, or none without one.
    pub fn set_chart(&mut self, chart: Option<&ChartState>) {
        self.chart = chart.cloned();
    }

    /// The song time and whether it plays, for scenes.
    pub fn set_song(&mut self, time: f64, playing: bool) {
        self.song = (time, playing);
    }

    /// The chances the music's moments play an effect with, in the scene on screen, the one
    /// waiting for a transition and every scene that comes.
    pub fn set_effect_chances(&mut self, chances: Chances) {
        self.chances = chances;
        self.scene.set_effect_chances(chances);
        if let Some(pending) = self
            .transitions
            .running
            .as_mut()
            .and_then(|running| running.pending.as_mut())
        {
            pending.set_effect_chances(chances);
        }
    }

    /// The runtime's decisions, read from now on by the scene on screen, the one waiting for a
    /// transition and every scene that comes (a scene lab replaced `runtime.toml`).
    pub fn set_runtime(&mut self, runtime: Arc<RuntimeBindings>) {
        self.runtime = runtime;
        self.scene.set_runtime(&self.runtime);
        if let Some(pending) = self
            .transitions
            .running
            .as_mut()
            .and_then(|running| running.pending.as_mut())
        {
            pending.set_runtime(&self.runtime);
        }
    }

    /// The runtime's decisions in use.
    pub fn runtime(&self) -> &Arc<RuntimeBindings> {
        &self.runtime
    }

    /// The app's accent colour (linear), which scenes and transitions draw in.
    pub fn set_accent(&mut self, accent: Option<[f32; 3]>) {
        self.accent = accent;
    }

    /// The atlas of album covers for scenes with `covers = true` (none: no covers).
    pub fn set_covers(&mut self, covers: Option<Arc<Atlas>>) {
        self.scene.set_covers(
            &self.device,
            &self.queue,
            covers.clone(),
            self.playing_cover.clone(),
        );
        self.covers = covers;
    }

    /// The cell of the cover of the track playing, for cell 0 of the atlas (none: not known yet).
    pub fn set_playing_cover(&mut self, cell: Option<Arc<Cell>>) {
        self.scene
            .set_playing_cover(&self.device, &self.queue, cell.clone());
        self.transitions
            .set_cover(&self.device, &self.queue, cell.as_deref());
        self.playing_cover = cell;
    }

    /// Whether the scene on screen reads the atlas of covers.
    pub fn wants_covers(&self) -> bool {
        self.latest().wants_covers()
    }

    pub fn push_pcm(&mut self, interleaved: &[f32], channels: usize) {
        self.scene.push_pcm(interleaved, channels);
    }

    /// Show the scene in the folder `dir`, waiting until it is built. The render loop builds
    /// scenes on another thread instead (see [`Pipeline::scene_build_dir`]); tests wait.
    #[cfg(test)]
    pub fn load_scene(&mut self, dir: &Path) -> Result<()> {
        self.load_source(Source::Dir(dir.to_path_buf()))
            .with_context(|| format!("Scene {}", dir.display()))
    }

    /// Show the built-in scene `name`, waiting until it is built.
    pub fn load_builtin_scene(&mut self, name: &str) -> Result<()> {
        self.load_source(builtin_source(name)?)
            .with_context(|| format!("Built-in scene {name}"))
    }

    fn load_source(&mut self, source: Source) -> Result<()> {
        let scene = self.scene_build(source, String::new()).run_bare()?;
        self.install(scene)
    }

    /// What a thread needs to build the scene in the folder `dir` for the size the output has
    /// now, to be shown with [`Pipeline::install`]. Errors carry the same context as
    /// [`Pipeline::load_scene`].
    pub fn scene_build_dir(&self, dir: &Path) -> SceneBuild {
        self.scene_build(
            Source::Dir(dir.to_path_buf()),
            format!("Scene {}", dir.display()),
        )
    }

    /// The same for the built-in scene `name`; it fails at once if there is none.
    pub fn scene_build_builtin(&self, name: &str) -> Result<SceneBuild> {
        Ok(self.scene_build(builtin_source(name)?, format!("Built-in scene {name}")))
    }

    fn scene_build(&self, source: Source, context: String) -> SceneBuild {
        SceneBuild {
            device: self.device.clone(),
            queue: self.queue.clone(),
            source,
            format: self.format,
            size: self.size(),
            sample_rate: self.sample_rate,
            context,
        }
    }

    /// Show a scene that was built with [`Pipeline::scene_build_dir`] or the like at once (a cut),
    /// possibly while frames of the scene before went on. If the output changed its size meanwhile
    /// the scene is resized first. Whatever the old scene heard of the music goes on in the new
    /// one, and a transition under way ends (see [`Pipeline::switch_to`] for the ones that fade).
    pub fn install(&mut self, scene: Scene) -> Result<()> {
        // A scene waiting for the transition is dropped for this one, which cuts.
        self.transitions.end();
        self.place(scene)
    }

    fn place(&mut self, mut scene: Scene) -> Result<()> {
        let (width, height) = self.size();
        if scene.output_size() != (width, height) {
            scene.resize(&self.device, &self.queue, width, height)?;
        }
        scene.set_covers(
            &self.device,
            &self.queue,
            self.covers.clone(),
            self.playing_cover.clone(),
        );
        scene.take_music_from(&mut self.scene);
        scene.set_lens_forced(self.lab.lens);
        scene.set_bursts(self.lab.bursts);
        scene.set_effect_chances(self.chances);
        scene.set_runtime(&self.runtime);
        self.scene = scene;
        self.restart_adaptive();
        Ok(())
    }

    /// Show a scene that was built with [`Pipeline::scene_build_dir`] or the like, through
    /// `transition` of `durations`. The pipeline holds the scene until the transition gets to it:
    /// a fade draws the old scene live while it fades out and installs the new one at the end of
    /// that; a dissolve keeps the old scene's last picture and installs the new one in the next
    /// frame; a cut-in keeps it too and installs the new scene in the next frame, which then draws
    /// under the band. A transition that shows nothing (a cut, or no time) is [`Pipeline::install`].
    ///
    /// A switch while another transition runs: if the scene of that one is still waiting (a fade
    /// that is fading out, a dissolve that has not started), the newer scene takes its place and
    /// the transition goes on (so the picture does not jump); if it is already in, the transition
    /// ends and the newer scene is cut in.
    pub fn switch_to(
        &mut self,
        mut scene: Scene,
        transition: Transition,
        durations: Durations,
    ) -> Result<()> {
        let (width, height) = self.size();
        if scene.output_size() != (width, height) {
            scene.resize(&self.device, &self.queue, width, height)?;
        }
        scene.set_runtime(&self.runtime);
        if let Some(running) = &mut self.transitions.running {
            if running.pending.is_some() {
                running.pending = Some(scene);
                return Ok(());
            }
            return self.install(scene);
        }
        if durations.is_instant(transition) {
            return self.install(scene);
        }
        self.transitions.running = Some(Running {
            kind: transition,
            durations,
            start: None,
            pending: Some(scene),
        });
        Ok(())
    }

    /// What the transition does in the frame at `time`, installing the scene it was waiting to
    /// when its time has come.
    fn plan_transition(&mut self, time: f64) -> Plan {
        let captured = self.transitions.captured();
        let Some(running) = &mut self.transitions.running else {
            return Plan::None;
        };
        let start = *running.start.get_or_insert(time);
        let seconds = time - start;
        let Durations { out, into } = running.durations;
        let mut install = None;
        let kind = running.kind;
        let plan = match kind {
            // A cut never runs (see `switch_to`).
            Transition::Cut => Plan::None,
            Transition::Fade | Transition::Flash => {
                // The new scene goes in at the black, or at the peak of the light.
                let (plan, phase) = Plan::out_and_in(kind, seconds, out, into, false);
                if !matches!(phase, FadePhase::Out(_)) {
                    install = running.pending.take();
                }
                plan
            }
            Transition::Dissolve | Transition::Glitch | Transition::Mosaic => {
                if running.pending.is_some() {
                    if captured {
                        install = running.pending.take();
                        running.start = Some(time);
                        Plan::over_held(kind, 0.0, seed_of(time))
                    } else {
                        Plan::Capture
                    }
                } else {
                    match dissolve_progress(seconds, into) {
                        progress if progress >= 1.0 => Plan::None,
                        progress => Plan::over_held(kind, progress, seed_of(start)),
                    }
                }
            }
            // The old scene plays on behind the band, as the picture behind a cut-in does, and
            // nothing is held; the new scene goes in with the first frame of the tear, like a
            // fade's at the black. It is the cheaper way too: one scene draws at a time.
            Transition::CutIn(style) => {
                let frame = cutin_frame(seconds, out, into, style);
                // Also when a long frame skipped the whole of it.
                if frame.is_none_or(|frame| frame.tearing()) {
                    install = running.pending.take();
                }
                frame.map_or(Plan::None, Plan::CutIn)
            }
        };
        if let Some(scene) = install
            && let Err(e) = self.place(scene)
        {
            warn!("The scene after a transition did not fit the output: {e:#}");
            self.transitions.end();
            return Plan::None;
        }
        if plan == Plan::None {
            self.transitions.end();
        }
        plan
    }

    /// End a transition under way before the output changes its size: the scene it waits for is
    /// installed at once.
    fn end_transition(&mut self) -> Result<()> {
        match self.transitions.end() {
            Some(scene) => self.place(scene),
            None => Ok(()),
        }
    }

    /// Read the scene on screen again if its files changed; whether it did.
    pub fn reload_scene(&mut self) -> Result<bool> {
        let source = self.latest().source().to_string();
        let (device, queue) = (self.device.clone(), self.queue.clone());
        let changed = self
            .latest_mut()
            .reload(&device, &queue)
            .with_context(|| format!("Scene {source}"))?;
        if changed {
            // A scene built anew reads the runtime's decisions as they were set.
            let runtime = self.runtime.clone();
            self.latest_mut().set_runtime(&runtime);
            self.restart_adaptive();
        }
        Ok(changed)
    }

    /// Replace the inputs and binds of the scene on screen with those of `text` (see
    /// [`crate::LabCommand::SceneBindings`]); on an error nothing changes.
    pub fn set_scene_bindings(&mut self, text: &str) -> Result<()> {
        let (device, queue) = (self.device.clone(), self.queue.clone());
        let rebuilt = self
            .scene
            .set_bindings(&device, &queue, text)
            .with_context(|| format!("Scene {}", self.scene.source()))?;
        if rebuilt {
            let runtime = self.runtime.clone();
            self.scene.set_runtime(&runtime);
            self.restart_adaptive();
        }
        Ok(())
    }

    /// The inputs the scene on screen declares.
    pub fn inputs(&self) -> Vec<crate::inspect::InspectInput> {
        self.scene.inputs()
    }

    /// Render one frame: into the next pixel buffer, shared texture, shared image or image of the
    /// window's swapchain, or into the target that is read back. The resolution and quality of
    /// the scene follow the GPU time of the frames before (see [`crate::adaptive`]).
    ///
    /// Pixel buffers, shared textures and shared images are submitted without waiting for the GPU,
    /// unless [`crate::in_flight::MAX_IN_FLIGHT`] frames are still being drawn: then for the
    /// oldest of them. What comes back is the newest frame the GPU has finished since, if any; the
    /// frame just submitted usually comes out of [`Pipeline::wait_for_frame`] or the next call.
    pub fn render(&mut self, time: f64) -> Result<Rendered> {
        if let Some(reason) = self.lost.get() {
            return Err(anyhow!("The GPU device was lost ({reason})"));
        }
        self.adapt();
        if self.in_flight.is_full() {
            self.wait_for_oldest(None)?;
        }
        let rendered = self.render_frame(time);
        self.timer.submitted(&self.device);
        match rendered? {
            Rendered::Nothing => self.finished(),
            rendered => Ok(rendered),
        }
    }

    /// Wait until the GPU has finished the oldest frame on its way and it is not held back any
    /// more (see [`crate::in_flight`]), but not past `until`, and return the newest such frame
    /// (nothing if there is none yet, or none is on its way).
    pub fn wait_for_frame(&mut self, until: Instant) -> Result<Rendered> {
        // A frame held back is handed out at its time, also while a newer one is being drawn.
        let release = |in_flight: &InFlight<Rendered>| {
            in_flight
                .release_at()
                .map_or(until, |release| release.min(until))
        };
        self.wait_for_oldest(Some(release(&self.in_flight)))?;
        let wake = release(&self.in_flight);
        let now = Instant::now();
        if self.in_flight.release_at().is_some() && wake > now {
            std::thread::sleep(wake - now);
        }
        Ok(self.take_finished())
    }

    /// Wait until the GPU has finished the oldest frame on its way, if one is, or `until` came.
    ///
    /// wgpu waits for one submission only when it is the newest: on Metal, wgpu-hal 30 forgets a
    /// finished command buffer on the next submission, possibly before its completion handler has
    /// recorded it, and a wait for that submission then reports the device lost. So the newest
    /// frame is waited for by waiting for everything submitted (at most a little work comes after
    /// it), and an older one by looking every [`WAIT_STEP`].
    fn wait_for_oldest(&self, until: Option<Instant>) -> Result<()> {
        let unfinished = self.in_flight.unfinished();
        if unfinished == 0 {
            return Ok(());
        }
        if unfinished == 1 {
            let timeout = until.map(|until| until.saturating_duration_since(Instant::now()));
            return match self.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout,
            }) {
                Ok(_) | Err(wgpu::PollError::Timeout) => Ok(()),
                Err(e) => Err(anyhow!("GPU poll failed: {e:?}")),
            };
        }
        loop {
            self.poll()?;
            let now = Instant::now();
            if self.in_flight.unfinished() < unfinished || until.is_some_and(|until| now >= until) {
                return Ok(());
            }
            let step = until.map_or(WAIT_STEP, |until| WAIT_STEP.min(until - now));
            std::thread::sleep(step);
        }
    }

    /// Run the callbacks of the work the GPU has finished, without waiting.
    fn poll(&self) -> Result<()> {
        self.device
            .poll(wgpu::PollType::Poll)
            .map(|_| ())
            .map_err(|e| anyhow!("GPU poll failed: {e:?}"))
    }

    /// The newest frame the GPU has finished, looking without waiting.
    fn finished(&mut self) -> Result<Rendered> {
        if self.in_flight.is_empty() {
            return Ok(Rendered::Nothing);
        }
        self.poll()?;
        Ok(self.take_finished())
    }

    /// The newest finished frame out of the frames on their way, remembered by the output as
    /// published.
    fn take_finished(&mut self) -> Rendered {
        let Some(frame) = self.in_flight.take_finished(Instant::now()) else {
            return Rendered::Nothing;
        };
        match (&mut self.output, &frame) {
            #[cfg(target_vendor = "apple")]
            (Output::Surfaces(surfaces), Rendered::PixelBuffer(buffer)) => {
                surfaces.keep(buffer.clone());
            }
            #[cfg(target_os = "windows")]
            (Output::Shared(targets), Rendered::Shared(texture)) => targets.keep(texture.clone()),
            #[cfg(target_os = "linux")]
            (Output::Shared(targets), Rendered::Shared(shared)) => targets.keep(shared),
            _ => {}
        }
        frame
    }

    /// Wait until the GPU has finished the frames on their way, and drop them. If that fails
    /// they are leaked rather than dropped: their targets (and on Linux their semaphores) must
    /// not go while the GPU may still use them.
    fn finish_in_flight(&mut self) {
        if self.in_flight.is_empty() {
            return;
        }
        // Everything: see `wait_for_oldest` for why not their own submissions.
        let waited = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(FINISH_WAIT),
        });
        let frames = self.in_flight.take_all();
        match waited {
            Ok(_) => drop(frames),
            Err(e) => {
                warn!("Letting go of frames the GPU has not finished: {e:?}");
                std::mem::forget(frames);
            }
        }
    }

    /// What the pass does in the frame at `time` for a scene whose cuts may be dressed (see
    /// [`Plan::Keep`]) and for the course of a cut that was: nothing for a scene that has no use
    /// for it.
    fn plan_dressed(&mut self, time: f64) -> Plan {
        // A glitch or a mosaic over the picture follows the music of every frame.
        let music = *self.scene.music();
        let transitions = &mut self.transitions;
        if let Some(overlay) = &mut transitions.overlay {
            overlay.tick(time, &music);
        }
        // The punch on a cut of a burst plays over the live frame, whatever the scene lists.
        if transitions.dressed.is_none()
            && let Some(punch) = transitions.punch
        {
            match punch.plan(time - punch.start) {
                Plan::None => transitions.punch = None,
                plan => return plan,
            }
        }
        // So does a glitch or a mosaic over the picture, in place of the plain frame (it is long:
        // the dressing of a cut and the lab's cut go on under it).
        let overlay = match transitions.overlay {
            Some(overlay) if transitions.dressed.is_none() => {
                let plan = overlay.plan();
                if plan == Plan::None {
                    transitions.overlay = None;
                }
                plan
            }
            _ => Plan::None,
        };
        // A lab's override decides which cuts are dressed in place of the scene's list.
        let dresses = match transitions.lab_cuts {
            Some(kind) => kind != Transition::Cut,
            None => !self.scene.cut_transitions().is_empty(),
        };
        if !dresses && transitions.forced == Forced::None && transitions.dressed.is_none() {
            return overlay;
        }
        // The frame is kept for a cut, under an overlay too.
        let keep = match overlay {
            Plan::None => Plan::Keep,
            overlay => overlay.keeping(),
        };
        // A cut the lab asked for waits for a frame to be held, then the director is told to.
        if let Forced::Wanted(kind) = transitions.forced
            && transitions.dressed.is_none()
            && transitions.has_kept()
        {
            self.scene.cut_now();
            transitions.forced = Forced::Armed(kind);
        }
        let Some(dressed) = transitions.dressed else {
            return keep;
        };
        match dressed.plan(time - dressed.start) {
            Plan::None => {
                // What is held is the picture before the cut until a frame is kept again.
                transitions.dressed = None;
                transitions.forget_kept();
                Plan::Keep
            }
            plan => plan,
        }
    }

    fn render_frame(&mut self, time: f64) -> Result<Rendered> {
        let mut plan = self.plan_transition(time);
        if plan == Plan::None {
            plan = self.plan_dressed(time);
        } else {
            // A switch takes the place of the dressing of a cut, of a punch and of a glitch or
            // mosaic over the picture.
            self.transitions.dressed = None;
            self.transitions.punch = None;
            self.transitions.overlay = None;
        }
        let size = self.size();
        self.transitions.prepare(&self.device, size, plan);
        #[cfg(target_os = "android")]
        if let Output::Surface(surface) = &mut self.output {
            let scene = &mut self.scene;
            let (device, queue) = (&self.device, &self.queue);
            let heard = (&self.rhythm, self.chart.as_ref(), self.song, self.accent);
            let timer = &mut self.timer;
            let fx = (&mut self.transitions, plan);
            let presented = surface.present(device, queue, |encoder, view| {
                draw(
                    scene,
                    timer,
                    fx,
                    (device, queue),
                    encoder,
                    view,
                    time,
                    heard,
                );
            });
            return Ok(if presented {
                Rendered::Presented
            } else {
                Rendered::Nothing
            });
        }
        #[cfg(target_vendor = "apple")]
        if let Output::Surfaces(surfaces) = &mut self.output {
            match surfaces.next(&self.device) {
                Ok(Some((buffer, view))) => {
                    let scene = &mut self.scene;
                    let heard = (&self.rhythm, self.chart.as_ref(), self.song, self.accent);
                    let timer = &mut self.timer;
                    let fx = (&mut self.transitions, plan);
                    render_and_submit(&self.device, &self.queue, |encoder| {
                        draw(
                            scene,
                            timer,
                            fx,
                            (&self.device, &self.queue),
                            encoder,
                            &view,
                            time,
                            heard,
                        )
                    });
                    let frame = Rendered::PixelBuffer(buffer);
                    track(&self.queue, &mut self.in_flight, frame);
                    return Ok(Rendered::Nothing);
                }
                Ok(None) => {
                    self.skipped.0 += 1;
                    return Ok(Rendered::Nothing);
                }
                Err(e) => {
                    warn!("Falling back to reading frames back: {e:#}");
                    let (width, height) = surfaces.size();
                    self.read_back(width, height);
                }
            }
        }
        #[cfg(target_os = "windows")]
        if let Output::Shared(targets) = &mut self.output {
            match targets.next(&self.device) {
                Ok(Some((texture, view))) => {
                    let scene = &mut self.scene;
                    let heard = (&self.rhythm, self.chart.as_ref(), self.song, self.accent);
                    let timer = &mut self.timer;
                    let fx = (&mut self.transitions, plan);
                    render_and_submit(&self.device, &self.queue, |encoder| {
                        draw(
                            scene,
                            timer,
                            fx,
                            (&self.device, &self.queue),
                            encoder,
                            &view,
                            time,
                            heard,
                        )
                    });
                    let frame = Rendered::Shared(texture);
                    track(&self.queue, &mut self.in_flight, frame);
                    return Ok(Rendered::Nothing);
                }
                Ok(None) => {
                    self.skipped.0 += 1;
                    return Ok(Rendered::Nothing);
                }
                Err(e) => {
                    warn!("Falling back to reading frames back: {e:#}");
                    let (width, height) = targets.size();
                    self.read_back(width, height);
                }
            }
        }
        #[cfg(target_os = "linux")]
        if let Output::Shared(targets) = &mut self.output {
            match targets.next(&self.device) {
                Ok(Some(next)) => {
                    let scene = &mut self.scene;
                    let (device, queue) = (&self.device, &self.queue);
                    let heard = (&self.rhythm, self.chart.as_ref(), self.song, self.accent);
                    let timer = &mut self.timer;
                    let fx = (&mut self.transitions, plan);
                    let frame = targets.render(device, queue, next, |encoder, view| {
                        draw(
                            scene,
                            timer,
                            fx,
                            (device, queue),
                            encoder,
                            view,
                            time,
                            heard,
                        );
                    })?;
                    if let Some(frame) = frame {
                        track(queue, &mut self.in_flight, Rendered::Shared(frame));
                    }
                    return Ok(Rendered::Nothing);
                }
                Ok(None) => {
                    self.skipped.0 += 1;
                    return Ok(Rendered::Nothing);
                }
                Err(e) => {
                    warn!("Falling back to reading frames back: {e:#}");
                    let (width, height) = targets.size();
                    self.read_back(width, height);
                }
            }
        }
        let Output::Readback(target) = &mut self.output else {
            unreachable!("the other outputs returned or were replaced above")
        };
        let scene = &mut self.scene;
        let (device, queue) = (&self.device, &self.queue);
        let heard = (&self.rhythm, self.chart.as_ref(), self.song, self.accent);
        let timer = &mut self.timer;
        let fx = (&mut self.transitions, plan);
        let frame = target.render(device, queue, |encoder, view| {
            draw(
                scene,
                timer,
                fx,
                (device, queue),
                encoder,
                view,
                time,
                heard,
            );
        })?;
        Ok(frame.map_or(Rendered::Nothing, Rendered::Mapped))
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.finish_in_flight();
    }
}

/// What a scene hears and sees of the player: the rhythm, the chart, the song time and whether
/// it plays, and the app's accent colour.
type Heard<'a> = (
    &'a Rhythm,
    Option<&'a ChartState>,
    (f64, bool),
    Option<[f32; 3]>,
);

/// A frame of the scene, timed by `timer`. With a transition (`plan`, see
/// [`crate::transition`]) the scene draws into the transition's texture and its pass writes
/// `view`; that pass then writes the frame's end timestamp, so its cost is part of the frame's.
#[allow(clippy::too_many_arguments)]
fn draw(
    scene: &mut Scene,
    timer: &mut GpuTimer,
    (transitions, plan): (&mut Transitions, Plan),
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    time: f64,
    (rhythm, chart, song, accent): Heard,
) {
    let timestamps = timer.begin();
    scene.render_timed(
        device,
        queue,
        encoder,
        transitions.scene_target(plan, view),
        &FrameInput {
            time,
            rhythm,
            chart,
            song,
            accent,
        },
        timestamps,
        plan == Plan::None,
    );
    let plan = dress_cut(transitions, plan, scene, time);
    let plan = start_overlay(transitions, plan, scene, time);
    let tint = flood_color(accent);
    transitions.composite(queue, encoder, view, (plan, tint), accent, timestamps);
    transitions.keep(encoder, plan);
    timer.end(encoder);
}

/// The plan of a frame in which the scene's director may have cut: a frame of [`Plan::Keep`] (or
/// of a punch) with a cut that the scene lets be dressed (see [`switching::dress`]), or that the
/// lab asked to be, starts the course of its transition with this frame; the held frame is the
/// one before it. A cut of a burst that is not dressed gets its punch instead (see [`Punch`]):
/// at once where the scene draws into the texture, else from the next frame, since the scene has
/// drawn into the output already.
fn dress_cut(transitions: &mut Transitions, plan: Plan, scene: &Scene, time: f64) -> Plan {
    let live = plan.is_live();
    if !live && plan != Plan::None {
        return plan;
    }
    let forced = match transitions.forced {
        Forced::Armed(kind) if live => {
            transitions.forced = Forced::None;
            Some(kind)
        }
        _ => None,
    };
    let Some(cut) = scene.cut_this_frame() else {
        return plan;
    };
    if live && transitions.has_kept() {
        let music = scene.music();
        let chosen = match forced {
            Some(kind) => Some((kind, switching::dressed_durations(kind, music))),
            // The moment of the cut chose its dressing, if it plays one (see `crate::moments`).
            None => cut
                .dress
                .map(|kind| (kind, switching::dressed_durations(kind, music))),
        };
        // A lab's override replaces whatever was chosen, last of all.
        if let Some((kind, durations)) =
            switching::cut_dressing(transitions.lab_cuts, chosen, music)
        {
            let dressed = Dressed {
                kind,
                durations,
                start: time,
            };
            match dressed.plan(0.0) {
                Plan::None => {}
                first => {
                    transitions.dressed = Some(dressed);
                    transitions.punch = None;
                    return first;
                }
            }
        }
    }
    if cut.fill == 0 {
        return plan;
    }
    let punch = Punch {
        start: time,
        fill: cut.fill,
    };
    transitions.punch = Some(punch);
    if live { punch.plan(0.0) } else { plan }
}

/// The plan of a frame in which the scene started a glitch or a mosaic over its picture (see
/// [`Scene::overlay_this_frame`]): it plays over the live frame for its time (see
/// `crate::course`), from this frame where the scene draws into the texture, else
/// from the next; one under way of the same kind goes on (see [`Transitions::start_overlay`]). A
/// switch under way takes its place; a dressed cut's course or a punch plays first, and it goes on
/// after them.
fn start_overlay(transitions: &mut Transitions, plan: Plan, scene: &Scene, time: f64) -> Plan {
    let Some(kind) = scene.overlay_this_frame() else {
        return plan;
    };
    if transitions.running.is_some() {
        return plan;
    }
    transitions.start_overlay(OverlayRun::with(
        scene.course_binds(),
        kind,
        time,
        &scene.phrase(),
        scene.music(),
    ));
    let Some(run) = transitions.overlay else {
        return plan;
    };
    match plan {
        Plan::Keep => run.plan().keeping(),
        Plan::Overlay(frame) if frame.keep => run.plan().keeping(),
        Plan::Overlay(_) => run.plan(),
        plan => plan,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transition::{CUTIN_BAND_SECONDS, CutInStyle};

    /// Pipelines build the default scene, and the driver loses the device (an unexplained
    /// error) when several do so at once, so the tests that make one take turns.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
        TURN.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The built-in scene loads and draws a frame that is not black.
    #[test]
    fn default_scene_renders_a_frame() {
        let _turn = serial();
        let Ok(mut pipeline) = Pipeline::new(
            64,
            36,
            44_100,
            true,
            None,
            None,
            Arc::new(|_| {}),
            #[cfg(target_os = "android")]
            None,
        ) else {
            eprintln!("SKIPPED: no GPU adapter");
            return;
        };
        assert_eq!(pipeline.preset_name(), DEFAULT_PRESET_NAME);
        // A frame on the readback path comes out a few calls after it was drawn.
        let mut frame = None;
        for i in 0..500 {
            if let Rendered::Mapped(mapped) = pipeline.render(f64::from(i) / 60.0).unwrap() {
                frame = Some(mapped);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let frame = frame.expect("no frame came out");
        assert!(frame.bytes().iter().any(|&b| b != 0), "the frame is black");
        assert!(!pipeline.reload_scene().unwrap());
    }

    /// A scene built on another thread is installed, resized to the output if that changed
    /// meanwhile, and one that does not build is an error that leaves the scene on screen.
    #[test]
    fn a_scene_built_on_another_thread_is_installed_at_the_current_size() {
        let _turn = serial();
        let Ok(mut pipeline) = Pipeline::new(
            64,
            32,
            44_100,
            true,
            None,
            None,
            Arc::new(|e| panic!("GPU error: {e}")),
            #[cfg(target_os = "android")]
            None,
        ) else {
            eprintln!("SKIPPED: no GPU adapter");
            return;
        };
        let build = pipeline.scene_build_builtin("Growth").unwrap();
        let scene = std::thread::spawn(move || build.run())
            .join()
            .unwrap()
            .unwrap();
        assert_eq!(scene.output_size(), (64, 32));
        // The output changes while the scene is being built.
        pipeline.resize(96, 48).unwrap();
        pipeline.install(scene).unwrap();
        assert_eq!(pipeline.preset_name(), "Growth");
        assert_eq!(pipeline.scene.output_size(), (96, 48));
        pipeline.render(0.0).unwrap();

        let missing = std::env::temp_dir().join("rune-no-such-scene-folder");
        let error = pipeline.scene_build_dir(&missing).run().err().unwrap();
        assert!(format!("{error:#}").starts_with("Scene "), "{error:#}");
        assert_eq!(pipeline.preset_name(), "Growth");
        assert!(pipeline.scene_build_builtin("No such scene").is_err());
    }

    /// A scene folder drawing one plain colour (`value`, linear, all three channels), with no
    /// post-processing, so that a pixel is the sRGB of it. (It has a parameter because a scene
    /// without any makes an empty buffer, which the GPU reports as an error.)
    fn plain_scene(name: &str, value: f32) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rune-pipeline-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = format!(
            "format = 1\nname = \"{name}\"\nlicense = \"MPL-2.0\"\n[[param]]\nid = \"unused\"\ndefault = 0.0\n[[pass]]\nshader = \"a.wgsl\"\ntarget = \"out\"\n[post]\ntonemap = \"none\"\ngrain = 0.0\nchromatic = 0.0\nvignette = 0.0\nbloom = {{ intensity = 0.0 }}\n"
        );
        std::fs::write(dir.join("scene.toml"), manifest).unwrap();
        let shader =
            format!("fn shade(px: Pixel) -> vec4f {{\n  return vec4f(vec3f({value}), 1.0);\n}}\n");
        std::fs::write(dir.join("a.wgsl"), shader).unwrap();
        dir
    }

    /// The frames drawn at `times`, as the mean of their bytes (0 to 255) in the middle of the
    /// picture, in order.
    fn means(pipeline: &mut Pipeline, times: &[f64]) -> Vec<f32> {
        let mut means = Vec::new();
        let render = |pipeline: &mut Pipeline, time: f64| {
            let mean = if let Rendered::Mapped(frame) = pipeline.render(time).unwrap() {
                let (w, h) = pipeline.size();
                let at = (h as usize / 2) * frame.stride() + (w as usize / 2) * 4;
                let px = &frame.bytes()[at..at + 3];
                Some(px.iter().map(|&b| f32::from(b)).sum::<f32>() / 3.0)
            } else {
                None
            };
            // Finish the frame, so that at most one comes out per call and none is passed over.
            pipeline
                .device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .unwrap();
            mean
        };
        for &time in times {
            means.extend(render(pipeline, time));
        }
        // The frames come out some calls after they were drawn.
        for _ in 0..50 {
            if means.len() >= times.len() {
                break;
            }
            means.extend(render(pipeline, times[times.len() - 1]));
        }
        means.truncate(times.len());
        assert_eq!(means.len(), times.len(), "frames did not come out");
        means
    }

    fn test_pipeline() -> Option<Pipeline> {
        let pipeline = Pipeline::new(
            64,
            32,
            44_100,
            true,
            None,
            None,
            Arc::new(|e| panic!("GPU error: {e}")),
            #[cfg(target_os = "android")]
            None,
        );
        if pipeline.is_err() {
            eprintln!("SKIPPED: no GPU adapter");
        }
        pipeline.ok()
    }

    fn load_plain(pipeline: &mut Pipeline, name: &str, value: f32) -> Scene {
        let dir = plain_scene(name, value);
        pipeline
            .scene_build_dir(&dir)
            .run()
            .unwrap_or_else(|e| panic!("{e:#}"))
    }

    /// A fade takes the old scene to black while it draws live, installs the new one at the
    /// black, and brings it in.
    #[test]
    fn a_fade_goes_through_black() {
        let _turn = serial();
        let Some(mut pipeline) = test_pipeline() else {
            return;
        };
        let white = load_plain(&mut pipeline, "fade-white", 1.0);
        pipeline.install(white).unwrap();
        let grey = load_plain(&mut pipeline, "fade-grey", 0.2);
        let durations = Durations {
            out: 1.0,
            into: 1.0,
        };
        pipeline
            .switch_to(grey, Transition::Fade, durations)
            .unwrap();
        // The name is the new scene's at once; the old one still draws.
        assert_eq!(pipeline.preset_name(), "fade-grey");
        assert_eq!(pipeline.scene.name, "fade-white");
        let times: Vec<f64> = (0..=10).map(|i| f64::from(i) * 0.25).collect();
        let m = means(&mut pipeline, &times);
        assert!(m[0] > 250.0, "{m:?}");
        assert!((m[2] - 127.0).abs() < 6.0, "half way out: {m:?}");
        assert!(m[4] < 6.0, "black in the middle: {m:?}");
        assert!((m[6] - 62.0).abs() < 6.0, "half way in: {m:?}");
        assert!((m[8] - 124.0).abs() < 4.0, "the new scene: {m:?}");
        assert_eq!(pipeline.scene.name, "fade-grey");
        assert!(pipeline.transitions.running.is_none());
        // Nothing is held once it is over.
        assert!(!pipeline.transitions.holds_textures());
    }

    /// A dissolve holds the old scene's picture and mixes it with the new scene's frames.
    #[test]
    fn a_dissolve_mixes_the_old_picture_with_the_new_scene() {
        let _turn = serial();
        let Some(mut pipeline) = test_pipeline() else {
            return;
        };
        let white = load_plain(&mut pipeline, "dissolve-white", 1.0);
        pipeline.install(white).unwrap();
        let grey = load_plain(&mut pipeline, "dissolve-grey", 0.2);
        let durations = Durations {
            out: 0.0,
            into: 2.0,
        };
        pipeline
            .switch_to(grey, Transition::Dissolve, durations)
            .unwrap();
        let times: Vec<f64> = (0..=10).map(|i| f64::from(i) * 0.25).collect();
        let m = means(&mut pipeline, &times);
        // The held picture first, then half and half, then the new scene.
        assert!(m[0] > 250.0 && m[1] > 250.0, "{m:?}");
        assert!((m[5] - 189.0).abs() < 6.0, "half way: {m:?}");
        assert!((m[10] - 124.0).abs() < 4.0, "the new scene: {m:?}");
        assert!(pipeline.transitions.running.is_none());
        assert!(!pipeline.transitions.holds_textures());
    }

    /// A scene of one grey that is the input `shade`, which follows the music's `meter` (4 with
    /// no chart: half the range of 0 to 8).
    fn bound_scene(name: &str) -> std::path::PathBuf {
        let dir = plain_scene(name, 0.0);
        let manifest = std::fs::read_to_string(dir.join("scene.toml")).unwrap();
        let manifest = format!(
            "{manifest}[[input]]\nid = \"shade\"\n[[bind]]\ntarget = \"shade\"\nsource = \"meter\"\nhow = \"follow\"\nwhen = {{ from = [0, 8] }}\n"
        );
        std::fs::write(dir.join("scene.toml"), manifest).unwrap();
        std::fs::write(
            dir.join("a.wgsl"),
            "fn shade(px: Pixel) -> vec4f {\n  return vec4f(vec3f(input.shade), 1.0);\n}\n",
        )
        .unwrap();
        dir
    }

    /// The inputs a scene's binds drive reach its shaders through the pipeline, and what a lab
    /// reads says which level the music was at.
    #[test]
    fn a_scene_draws_with_its_bound_inputs_and_tells_the_level() {
        let _turn = serial();
        let Some(mut pipeline) = test_pipeline() else {
            return;
        };
        let scene = pipeline
            .scene_build_dir(&bound_scene("bound"))
            .run()
            .unwrap_or_else(|e| panic!("{e:#}"));
        pipeline.install(scene).unwrap();
        // Half of the range, as sRGB.
        let m = means(&mut pipeline, &[0.0, 0.1, 0.2]);
        assert!((m[2] - 188.0).abs() < 4.0, "{m:?}");
        let report = pipeline.bindings();
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].target, "shade");
        assert_eq!(report[0].tick.level, crate::bind::Level::C);
        assert_eq!(report[0].tick.used, crate::bind::Used::Source);
        // A chart without a beat makes it level B.
        pipeline.set_chart(Some(&ChartState::default()));
        means(&mut pipeline, &[0.3, 0.4]);
        assert_eq!(pipeline.bindings()[0].tick.level, crate::bind::Level::B);
        // What a lab forces is kept for the render loop, which applies it to the chart.
        assert_eq!(pipeline.lab().level, None);
        pipeline.set_lab_level(Some(crate::bind::Level::C));
        assert_eq!(pipeline.lab().level, Some(crate::bind::Level::C));
    }

    /// A cover of one colour, for the band of a cut-in.
    fn solid_cover(rgb: [u8; 3]) -> Arc<Cell> {
        Arc::new(Cell::new(Some(&image::RgbaImage::from_pixel(
            8,
            8,
            image::Rgba([rgb[0], rgb[1], rgb[2], 255]),
        ))))
    }

    /// The middle pixel of the frame drawn at `time` and one near the corner, as RGB. Frames come
    /// out some calls after they were drawn, so the frame is drawn again until the ones before
    /// it are out, and the last one counts.
    fn probes(pipeline: &mut Pipeline, time: f64) -> ([u8; 3], [u8; 3]) {
        let (w, h) = pipeline.size();
        let mut last = None;
        for _ in 0..8 {
            let rendered = pipeline.render(time).unwrap();
            pipeline
                .device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .unwrap();
            if let Rendered::Mapped(frame) = rendered {
                let at = |x: usize, y: usize| {
                    let i = y * frame.stride() + x * 4;
                    let px = &frame.bytes()[i..i + 3];
                    if cfg!(target_vendor = "apple") {
                        [px[2], px[1], px[0]]
                    } else {
                        [px[0], px[1], px[2]]
                    }
                };
                last = Some((at(w as usize / 2, h as usize / 2), at(2, 2)));
            }
        }
        last.expect("no frame came out")
    }

    /// A cut-in draws its band over the old scene, which plays on darkened, shows the cover in
    /// the band over its middle, puts the new scene in with the tear, and holds nothing.
    #[test]
    fn a_cutin_shows_the_cover_in_a_band_over_the_old_scene_then_the_new_scene() {
        let _turn = serial();
        let Some(mut pipeline) = test_pipeline() else {
            return;
        };
        let white = load_plain(&mut pipeline, "cutin-white", 1.0);
        pipeline.install(white).unwrap();
        pipeline.set_playing_cover(Some(solid_cover([255, 0, 0])));
        let grey = load_plain(&mut pipeline, "cutin-grey", 0.2);
        let durations = Durations {
            out: 0.5,
            into: 0.1,
        };
        let style = CutInStyle::Halftone;
        pipeline
            .switch_to(grey, Transition::CutIn(style), durations)
            .unwrap();
        // The name is the new scene's at once; the old one draws until the band tears.
        assert_eq!(pipeline.preset_name(), "cutin-grey");
        assert_eq!(pipeline.scene.name, "cutin-white");
        // The first frame is the old scene, whole away from the slash.
        let (_, corner) = probes(&mut pipeline, 0.0);
        assert!(corner.iter().all(|&c| c > 250), "{corner:?}");
        // In the hold the old scene still draws, dark around the band, which shows the cover
        // over the middle; nothing is held.
        let (middle, corner) = probes(&mut pipeline, 0.3);
        assert_eq!(pipeline.scene.name, "cutin-white");
        assert!(
            middle[0] > 200 && middle[1] < 60 && middle[2] < 60,
            "{middle:?}"
        );
        assert!(
            corner.iter().all(|&c| (120..160).contains(&c)),
            "{corner:?}"
        );
        assert!(!pipeline.transitions.has_held());
        // The tear: the new scene is in.
        probes(&mut pipeline, 0.52);
        assert_eq!(pipeline.scene.name, "cutin-grey");
        // Torn away: the new scene, and nothing held.
        let (middle, corner) = probes(&mut pipeline, 0.7);
        assert!(
            middle.iter().all(|&c| (115..135).contains(&c)),
            "{middle:?}"
        );
        assert!(
            corner.iter().all(|&c| (115..135).contains(&c)),
            "{corner:?}"
        );
        assert!(pipeline.transitions.running.is_none());
        assert!(!pipeline.transitions.holds_textures());
    }

    /// A frame that skips past the whole of a cut-in still puts the new scene in.
    #[test]
    fn a_cutin_skipped_by_a_long_frame_still_installs_the_new_scene() {
        let _turn = serial();
        let Some(mut pipeline) = test_pipeline() else {
            return;
        };
        let white = load_plain(&mut pipeline, "skip-white", 1.0);
        pipeline.install(white).unwrap();
        let grey = load_plain(&mut pipeline, "skip-grey", 0.2);
        let durations = Durations {
            out: 0.5,
            into: 0.1,
        };
        pipeline
            .switch_to(grey, Transition::CutIn(CutInStyle::default()), durations)
            .unwrap();
        probes(&mut pipeline, 0.0);
        assert_eq!(pipeline.scene.name, "skip-white");
        probes(&mut pipeline, 5.0);
        assert_eq!(pipeline.scene.name, "skip-grey");
        assert!(pipeline.transitions.running.is_none());
    }

    /// The frames of a cut-in over two built-in scenes, as a contact sheet per style (with a
    /// cover and without one) in `$CUTIN_FRAMES` (default: the temp dir), to look at once.
    #[test]
    #[ignore = "writes images, run by hand"]
    fn dump_cutin_frames() {
        let _turn = serial();
        let dir = std::env::var_os("CUTIN_FRAMES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let (w, h) = (1280u32, 720u32);
        let cover = image::RgbaImage::from_fn(256, 256, |x, y| {
            let (fx, fy) = (x as f32 / 255.0, y as f32 / 255.0);
            let ring = ((fx - 0.5).hypot(fy - 0.5) * 18.0).sin() * 0.5 + 0.5;
            image::Rgba([
                (255.0 * (0.25 + 0.7 * fx)) as u8,
                (255.0 * (0.15 + 0.5 * ring * (1.0 - fy))) as u8,
                (255.0 * (0.3 + 0.6 * fy)) as u8,
                255,
            ])
        });
        let runs = [
            ("streaks", CutInStyle::Streaks, true),
            ("halftone", CutInStyle::Halftone, true),
            ("echo", CutInStyle::Echo, true),
            ("streaks-nocover", CutInStyle::Streaks, false),
        ];
        // The cut-in starts with the first frame, at 1 s, and is drawn frame by frame at 60 fps
        // through the slash and the opening, twice in the hold (the second with the glint),
        // and frame by frame through the tear.
        let start = 1.0;
        let hold = 0.5;
        let frame = 1.0 / 60.0;
        let tear = CUTIN_BAND_SECONDS + hold;
        let offsets: Vec<f64> = (0..11)
            .map(|i| f64::from(i) * frame)
            .chain([0.35, CUTIN_BAND_SECONDS + 0.7 * hold])
            .chain((0..11).map(|i| tear + f64::from(i) * frame))
            .collect();
        for (label, style, with_cover) in runs {
            let Ok(mut pipeline) = Pipeline::new(
                w,
                h,
                44_100,
                true,
                None,
                None,
                Arc::new(|e| panic!("GPU error: {e}")),
                #[cfg(target_os = "android")]
                None,
            ) else {
                eprintln!("SKIPPED: no GPU adapter");
                return;
            };
            pipeline.set_accent(Some([0.05, 0.35, 0.9]));
            if with_cover {
                pipeline.set_playing_cover(Some(Arc::new(Cell::new(Some(&cover)))));
            }
            pipeline.load_builtin_scene("Armillary").unwrap();
            let mut next = Some(
                pipeline
                    .scene_build_builtin("Growth")
                    .unwrap()
                    .run()
                    .unwrap(),
            );
            let durations = Durations {
                out: CUTIN_BAND_SECONDS + hold,
                into: crate::switching::CUTIN_TEAR_SECONDS,
            };
            // A frame of the old scene before, then the cut-in.
            let mut times = vec![start - 0.1];
            times.extend(offsets.iter().map(|o| start + o));
            let mut frames: Vec<Vec<u8>> = Vec::new();
            let render = |pipeline: &mut Pipeline, time: f64| -> Option<Vec<u8>> {
                let rgba = if let Rendered::Mapped(frame) = pipeline.render(time).unwrap() {
                    let mut rgba = Vec::new();
                    for y in 0..h as usize {
                        for px in frame.bytes()[y * frame.stride()..][..w as usize * 4].chunks(4) {
                            let (r, b) = if cfg!(target_vendor = "apple") {
                                (px[2], px[0])
                            } else {
                                (px[0], px[2])
                            };
                            rgba.extend_from_slice(&[r, px[1], b, 255]);
                        }
                    }
                    Some(rgba)
                } else {
                    None
                };
                pipeline
                    .device
                    .poll(wgpu::PollType::Wait {
                        submission_index: None,
                        timeout: None,
                    })
                    .unwrap();
                rgba
            };
            for (i, &time) in times.iter().enumerate() {
                if i == 1 {
                    pipeline
                        .switch_to(next.take().unwrap(), Transition::CutIn(style), durations)
                        .unwrap();
                }
                frames.extend(render(&mut pipeline, time));
            }
            for _ in 0..50 {
                frames.extend(render(&mut pipeline, *times.last().unwrap()));
                if frames.len() >= times.len() {
                    break;
                }
            }
            assert!(frames.len() >= times.len(), "frames did not come out");
            let (tw, th) = (w / 2, h / 2);
            let columns = 6;
            let rows = times.len().div_ceil(columns) as u32;
            let mut sheet = image::RgbaImage::new(tw * columns as u32, th * rows);
            for (i, rgba) in frames.into_iter().take(times.len()).enumerate() {
                let tile = image::imageops::resize(
                    &image::RgbaImage::from_raw(w, h, rgba).unwrap(),
                    tw,
                    th,
                    image::imageops::FilterType::Triangle,
                );
                image::imageops::replace(
                    &mut sheet,
                    &tile,
                    i64::from(tw) * (i % columns) as i64,
                    i64::from(th) * (i / columns) as i64,
                );
            }
            let path = dir.join(format!("cutin-{label}.png"));
            sheet.save(&path).unwrap();
            println!("{}", path.display());
        }
    }

    /// Every frame at 60 fps of a mosaic and a glitch switching two built-in scenes in a beat
    /// (120 bpm); each played over the picture, sampled across its two bars and frame by frame at
    /// its peak. A contact sheet each (and the frames at full size in a folder) in `$FX_FRAMES`
    /// (default: the temp dir), to look at once.
    #[test]
    #[ignore = "writes images, run by hand"]
    fn dump_glitch_and_mosaic_frames() {
        let _turn = serial();
        let dir = std::env::var_os("FX_FRAMES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let (w, h) = (1280u32, 720u32);
        let beat: f64 = 0.5;
        let frame = 1.0 / 60.0;
        // The times of the frames from the start: a frame before, then the start.
        let by_frame = |seconds: f64| -> Vec<f64> {
            (0..(seconds / frame).ceil() as usize + 3)
                .map(|i| (i as f64 - 1.0) * frame)
                .collect()
        };
        // A phrase of four bars at 120 bpm (see `crate::course`).
        let overlay = 16.0 * beat;
        let across: Vec<f64> = [-frame]
            .into_iter()
            .chain((0..36).map(|k| f64::from(k) * overlay / 35.0))
            .collect();
        let peak: Vec<f64> = [-frame, 0.0]
            .into_iter()
            .chain((0..22).map(|k| overlay / 3.0 + f64::from(k) * frame))
            .collect();
        let runs = [
            ("mosaic", Some(Transition::Mosaic), None, by_frame(beat)),
            ("glitch", Some(Transition::Glitch), None, by_frame(beat)),
            (
                "overlay-mosaic",
                None,
                Some(Overlay::Mosaic),
                across.clone(),
            ),
            ("overlay-glitch", None, Some(Overlay::Glitch), across),
            (
                "overlay-mosaic-peak",
                None,
                Some(Overlay::Mosaic),
                peak.clone(),
            ),
            ("overlay-glitch-peak", None, Some(Overlay::Glitch), peak),
        ];
        for (label, switch, overlay, offsets) in runs {
            let Ok(mut pipeline) = Pipeline::new(
                w,
                h,
                44_100,
                true,
                None,
                None,
                Arc::new(|e| panic!("GPU error: {e}")),
                #[cfg(target_os = "android")]
                None,
            ) else {
                eprintln!("SKIPPED: no GPU adapter");
                return;
            };
            pipeline.set_accent(Some([0.05, 0.35, 0.9]));
            pipeline.load_builtin_scene("Armillary").unwrap();
            let mut next = switch.map(|_| {
                pipeline
                    .scene_build_builtin("Growth")
                    .unwrap()
                    .run()
                    .unwrap()
            });
            let start = 1.0;
            let times: Vec<f64> = offsets.iter().map(|o| start + o).collect();
            let render = |pipeline: &mut Pipeline, time: f64| -> Option<Vec<u8>> {
                let rgba = if let Rendered::Mapped(frame) = pipeline.render(time).unwrap() {
                    let mut rgba = Vec::new();
                    for y in 0..h as usize {
                        for px in frame.bytes()[y * frame.stride()..][..w as usize * 4].chunks(4) {
                            let (r, b) = if cfg!(target_vendor = "apple") {
                                (px[2], px[0])
                            } else {
                                (px[0], px[2])
                            };
                            rgba.extend_from_slice(&[r, px[1], b, 255]);
                        }
                    }
                    Some(rgba)
                } else {
                    None
                };
                pipeline
                    .device
                    .poll(wgpu::PollType::Wait {
                        submission_index: None,
                        timeout: None,
                    })
                    .unwrap();
                rgba
            };
            let mut frames: Vec<Vec<u8>> = Vec::new();
            for (i, &time) in times.iter().enumerate() {
                if i == 1 {
                    if let (Some(kind), Some(scene)) = (switch, next.take()) {
                        let durations = Durations {
                            out: 0.0,
                            into: beat,
                        };
                        pipeline.switch_to(scene, kind, durations).unwrap();
                    }
                    if let Some(kind) = overlay {
                        pipeline.start_overlay(kind);
                    }
                }
                frames.extend(render(&mut pipeline, time));
            }
            for _ in 0..50 {
                frames.extend(render(&mut pipeline, *times.last().unwrap()));
                if frames.len() >= times.len() {
                    break;
                }
            }
            assert!(frames.len() >= times.len(), "frames did not come out");
            let folder = dir.join(label);
            std::fs::create_dir_all(&folder).unwrap();
            let (tw, th) = (w / 4, h / 4);
            let columns = 6;
            let rows = times.len().div_ceil(columns) as u32;
            let mut sheet = image::RgbaImage::new(tw * columns as u32, th * rows);
            for (i, rgba) in frames.into_iter().take(times.len()).enumerate() {
                let full = image::RgbaImage::from_raw(w, h, rgba).unwrap();
                full.save(folder.join(format!("{i:02}.png"))).unwrap();
                let tile =
                    image::imageops::resize(&full, tw, th, image::imageops::FilterType::Triangle);
                image::imageops::replace(
                    &mut sheet,
                    &tile,
                    i64::from(tw) * (i % columns) as i64,
                    i64::from(th) * (i / columns) as i64,
                );
            }
            let path = dir.join(format!("{label}.png"));
            sheet.save(&path).unwrap();
            println!("{}", path.display());
        }
    }

    /// What the pass of a glitch and a mosaic costs at 1080p, at their heaviest steps and as lens
    /// effects, against a pass that only copies the frame, by the wall time of many passes in
    /// one submission, printed.
    #[test]
    #[ignore = "measures, run by hand"]
    fn time_glitch_and_mosaic_pass() {
        let _turn = serial();
        let Some(mut pipeline) = test_pipeline() else {
            return;
        };
        let size = (1920, 1080);
        let output = pipeline.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glitch and mosaic timing"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: pipeline.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = output.create_view(&Default::default());
        let wait = |device: &wgpu::Device| {
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .unwrap();
        };
        // An overlay of 4 s at its peak, a third of the way.
        let overlay_at_peak = |kind| {
            let music = MusicValues {
                tempo: 120.0,
                locked: 1.0,
                meter: 4.0,
                intensity: 1.0,
                ..MusicValues::default()
            };
            let mut run = OverlayRun::new(kind, 0.0, &crate::course::Phrase::default(), &music);
            run.course = run.course.at_peak();
            // A step of the beat grid at a time, to the first that is not blank.
            for i in 1..12 {
                let plan = run.plan();
                if matches!(plan, Plan::Overlay(frame) if frame.amount > 0.0) {
                    return plan;
                }
                let clock = i as f32 * 0.5;
                run.tick(f64::from(i) / 60.0, &MusicValues { clock, ..music });
            }
            panic!("every step blank");
        };
        // The middle of each pose or stage (see `mosaic_step` and `glitch_step`).
        let mosaic = |pose: f32| (pose + 0.5) / 8.0;
        let glitch = |stage: f32| (stage + 0.5) / 6.0;
        for (label, plan) in [
            ("nothing (a fade of 0)", Plan::Fade(0.0)),
            ("mosaic, scattered blocks", Plan::Mosaic(mosaic(1.0), 7)),
            ("mosaic, densest blocks", Plan::Mosaic(mosaic(3.0), 7)),
            ("mosaic, veil of dots", Plan::Mosaic(mosaic(4.0), 7)),
            ("mosaic, picture in dots", Plan::Mosaic(mosaic(5.0), 7)),
            ("glitch, heaviest stage", Plan::Glitch(glitch(3.0), 7)),
            (
                "mosaic overlay, at its most",
                overlay_at_peak(Overlay::Mosaic),
            ),
            (
                "glitch overlay, at its most",
                overlay_at_peak(Overlay::Glitch),
            ),
            // Again, now that the GPU is busy: the first is often slow.
            ("nothing, again", Plan::Fade(0.0)),
        ] {
            pipeline.transitions.prepare(&pipeline.device, size, plan);
            let passes = 200;
            let run = |passes: u32| {
                let mut encoder = pipeline.device.create_command_encoder(&Default::default());
                for _ in 0..passes {
                    pipeline.transitions.composite(
                        &pipeline.queue,
                        &mut encoder,
                        &view,
                        (plan, [0.8; 3]),
                        Some([0.05, 0.35, 0.9]),
                        None,
                    );
                }
                let started = std::time::Instant::now();
                pipeline.queue.submit([encoder.finish()]);
                wait(&pipeline.device);
                started.elapsed()
            };
            // Once to warm up.
            run(10);
            let took = run(passes);
            println!(
                "{label}: {:.3} ms a pass",
                took.as_secs_f64() * 1000.0 / f64::from(passes)
            );
        }
    }

    /// What the pass of a cut-in costs at 1080p, in the hold and in the tear (the frame of the
    /// tear with the most splinters), by the wall time of many passes in one submission, printed.
    #[test]
    #[ignore = "measures, run by hand"]
    fn time_cutin_pass() {
        let _turn = serial();
        let Some(mut pipeline) = test_pipeline() else {
            return;
        };
        let cover = image::RgbaImage::from_pixel(64, 64, image::Rgba([200, 60, 90, 255]));
        pipeline.set_playing_cover(Some(Arc::new(Cell::new(Some(&cover)))));
        let size = (1920, 1080);
        let output = pipeline.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cut-in timing"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: pipeline.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = output.create_view(&Default::default());
        let wait = |device: &wgpu::Device| {
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .unwrap();
        };
        let hold = 0.5;
        let tear = CUTIN_BAND_SECONDS + hold;
        for (label, style, seconds) in [
            ("nothing (a fade of 0)", None, 0.0),
            ("hold, speed lines", Some(CutInStyle::Streaks), 0.2),
            ("hold, echo", Some(CutInStyle::Echo), 0.2),
            (
                "tear, strips flying",
                Some(CutInStyle::Streaks),
                tear + 3.0 / 60.0,
            ),
        ] {
            let plan = match style {
                Some(style) => Plan::CutIn(
                    cutin_frame(seconds, tear, switching::CUTIN_TEAR_SECONDS, style).unwrap(),
                ),
                None => Plan::Fade(0.0),
            };
            pipeline.transitions.prepare(&pipeline.device, size, plan);
            let passes = 200;
            let run = |passes: u32| {
                let mut encoder = pipeline.device.create_command_encoder(&Default::default());
                for _ in 0..passes {
                    pipeline.transitions.composite(
                        &pipeline.queue,
                        &mut encoder,
                        &view,
                        (plan, [0.8; 3]),
                        Some([0.05, 0.35, 0.9]),
                        None,
                    );
                }
                let started = std::time::Instant::now();
                pipeline.queue.submit([encoder.finish()]);
                wait(&pipeline.device);
                started.elapsed()
            };
            // Once to warm up.
            run(10);
            let took = run(passes);
            println!(
                "{label}: {:.3} ms a pass",
                took.as_secs_f64() * 1000.0 / f64::from(passes)
            );
        }
    }

    /// A scene of one colour with two shots, which lets `cut` dress its cuts.
    fn dressed_scene(name: &str, value: f32, cut: &str) -> std::path::PathBuf {
        let dir = plain_scene(name, value);
        let manifest = std::fs::read_to_string(dir.join("scene.toml")).unwrap();
        let manifest = format!(
            "{manifest}[effects]\ntransitions = [\"{cut}\"]\n[[shot]]\nsize = \"wide\"\n[[shot]]\nsize = \"close\"\n"
        );
        std::fs::write(dir.join("scene.toml"), manifest).unwrap();
        dir
    }

    /// The glitch, the mosaic and the flash switch scenes like the ones before them: the new
    /// scene goes in (at the peak of the light for the flash) and nothing is held at the end.
    #[test]
    fn the_strong_transitions_switch_the_scene_and_let_go() {
        let _turn = serial();
        let Some(mut pipeline) = test_pipeline() else {
            return;
        };
        pipeline.set_accent(Some([0.1, 0.4, 1.0]));
        for (kind, durations) in [
            (
                Transition::Flash,
                Durations {
                    out: 0.5,
                    into: 1.0,
                },
            ),
            (
                Transition::Glitch,
                Durations {
                    out: 0.0,
                    into: 0.15,
                },
            ),
            (
                Transition::Mosaic,
                Durations {
                    out: 0.0,
                    into: 0.5,
                },
            ),
        ] {
            let old = load_plain(&mut pipeline, "strong-old", 0.8);
            pipeline.install(old).unwrap();
            let new = load_plain(&mut pipeline, "strong-new", 0.2);
            pipeline.switch_to(new, kind, durations).unwrap();
            assert_eq!(pipeline.preset_name(), "strong-new");
            let times: Vec<f64> = (0..=40).map(|i| f64::from(i) * 0.05).collect();
            // The first frame out can be one of the call before.
            let m = &means(&mut pipeline, &times)[1..];
            if kind == Transition::Flash {
                // A flash starts by washing the old picture in the theme colour, under the flood.
                let flood = flood_color(Some([0.1, 0.4, 1.0]));
                let flood_mean = flood.iter().sum::<f32>() / 3.0 * 255.0;
                assert!(
                    m[0] < 190.0 && m[0] <= flood_mean + 3.0,
                    "a flash starts washed: {m:?}"
                );
            } else {
                assert!(m[0] > 190.0, "{kind:?} starts on the old picture: {m:?}");
            }
            assert!(
                (m[39] - 124.0).abs() < 4.0,
                "{kind:?} ends on the new scene: {m:?}"
            );
            assert_eq!(pipeline.scene.name, "strong-new");
            assert!(pipeline.transitions.running.is_none());
            assert!(!pipeline.transitions.holds_textures());
        }
    }

    /// A flash floods the screen with a light tint of the accent that is not white, and the new
    /// scene comes out of it.
    #[test]
    fn a_flash_peaks_in_the_tint_and_the_new_scene_comes_out_of_it() {
        let _turn = serial();
        let Some(mut pipeline) = test_pipeline() else {
            return;
        };
        pipeline.set_accent(Some([0.8, 0.1, 0.05]));
        let old = load_plain(&mut pipeline, "flash-old", 0.2);
        pipeline.install(old).unwrap();
        let new = load_plain(&mut pipeline, "flash-new", 0.1);
        let durations = Durations {
            out: 1.0,
            into: 2.0,
        };
        pipeline
            .switch_to(new, Transition::Flash, durations)
            .unwrap();
        let times: Vec<f64> = (0..=12).map(|i| f64::from(i) * 0.25).collect();
        let m = means(&mut pipeline, &times);
        let flood = flood_color(Some([0.8, 0.1, 0.05]));
        let flood_mean = flood.iter().sum::<f32>() / 3.0 * 255.0;
        // The old scene (0.2 is byte 124), rising, the peak at one beat, falling, the new scene.
        assert!((m[0] - 124.0).abs() < 4.0, "{m:?}");
        assert!(m[2] > m[0] + 10.0 && m[3] > m[2], "rising: {m:?}");
        assert!(
            (m[4] - flood_mean).abs() < 3.0,
            "the flood at the peak: {m:?}"
        );
        assert!(m[4] < 0.83 * 255.0, "not white: {m:?}");
        assert!(m[6] < m[4] && m[8] < m[6], "falling: {m:?}");
        assert!((m[12] - 89.0).abs() < 4.0, "the new scene: {m:?}");
        assert!(
            m.iter().all(|mean| *mean < 0.83 * 255.0),
            "never white: {m:?}"
        );
    }

    /// A scene that lets its cuts be dressed holds a frame; one that does not holds nothing. A
    /// cut that is asked for is dressed whether the scene lists the transition or not, with a
    /// course of its own that ends in the scene as it was drawn.
    #[test]
    fn a_dressed_cut_flashes_over_the_picture_before_it() {
        let _turn = serial();
        let Some(mut pipeline) = test_pipeline() else {
            return;
        };
        // No list and nothing asked for: nothing is held.
        let plain = load_plain(&mut pipeline, "dress-plain", 0.2);
        pipeline.install(plain).unwrap();
        let times: Vec<f64> = (0..4).map(|i| f64::from(i) * 0.1).collect();
        means(&mut pipeline, &times);
        assert!(!pipeline.transitions.holds_textures());

        let dir = dressed_scene("dress-flash", 0.2, "glitch");
        let scene = pipeline.scene_build_dir(&dir).run().unwrap();
        pipeline.install(scene).unwrap();
        pipeline.set_accent(Some([0.8, 0.1, 0.05]));
        // The scene lists a glitch, the lab asks for a flash: a cut now, 0.2 s up, 0.4 s down.
        pipeline.cut_dressed(Transition::Flash);
        let times: Vec<f64> = (0..=14).map(|i| 1.0 + f64::from(i) * 0.1).collect();
        // The first frame out is one of the call before.
        let m = &means(&mut pipeline, &times)[1..];
        assert!(pipeline.transitions.holds_textures(), "a frame is kept");
        // The frame before the cut is kept, then the cut (at 1.1) starts the flash.
        assert!(
            (m[0] - 124.0).abs() < 4.0 && (m[1] - 124.0).abs() < 4.0,
            "{m:?}"
        );
        assert!(m[2] > m[1] + 10.0, "rising on the held picture: {m:?}");
        assert!(m[3] > m[2], "the peak at 0.2 s: {m:?}");
        assert!(m[4] < m[3] && m[5] < m[4], "falling: {m:?}");
        assert!((m[8] - 124.0).abs() < 4.0, "over after 0.6 s: {m:?}");
        assert!((m[13] - 124.0).abs() < 4.0, "{m:?}");
        assert!(pipeline.transitions.dressed.is_none());
        assert_eq!(pipeline.transitions.forced, Forced::None);
    }

    /// A burst of cuts (the scene declares four shots for it) gives each cut a punch over the live frame, in a scene that lists nothing
    /// to dress its cuts with and so holds no frame: the pass runs only while the punch plays.
    #[test]
    fn a_burst_punches_each_cut_and_lets_go() {
        let _turn = serial();
        let Some(mut pipeline) = test_pipeline() else {
            return;
        };
        let dir = plain_scene("punch", 0.2);
        let mut manifest = std::fs::read_to_string(dir.join("scene.toml")).unwrap();
        manifest.push_str("[[shot]]\nsize = \"wide\"\n[[shot]]\nsize = \"close\"\n");
        for size in ["medium", "close", "detail", "top"] {
            manifest.push_str(&format!("[[burst]]\nsize = \"{size}\"\n"));
        }
        std::fs::write(dir.join("scene.toml"), manifest).unwrap();
        let scene = pipeline.scene_build_dir(&dir).run().unwrap();
        pipeline.install(scene).unwrap();
        // The dithering moves a byte up or down.
        let rest = means(&mut pipeline, &[0.9, 0.95, 1.0])
            .into_iter()
            .fold(0.0, f32::max);
        assert!(!pipeline.transitions.holds_textures());
        pipeline.start_burst();
        let mut fills = Vec::new();
        let mut raised = 0;
        for i in 0..120 {
            let time = 1.0 + f64::from(i) / 60.0;
            let mean = means(&mut pipeline, &[time])[0];
            let fill = pipeline.transitions.punch.map(|punch| punch.fill);
            if let Some(fill) = fill
                && fills.last() != Some(&fill)
            {
                fills.push(fill);
            }
            if mean > rest + 1.5 {
                raised += 1;
            }
            assert!(fill.is_some() || !pipeline.transitions.holds_textures() || i < 2);
        }
        assert_eq!(fills, [1, 2, 3, 4]);
        // The flash is faint but there, for a few frames after each cut, not all the time.
        assert!((4..60).contains(&raised), "{raised}");
        assert!(pipeline.transitions.punch.is_none());
        assert!(
            !pipeline.transitions.holds_textures(),
            "nothing is held after it"
        );
    }

    /// A glitch or a mosaic over the picture plays over the live frame to the end of its phrase
    /// (4 s here, without a followed beat) and lets go, one at a time: in a scene that holds no frame the pass runs only while it plays; in one
    /// that dresses its cuts the frame is still kept under it and a cut is dressed; a switch takes
    /// its place.
    #[test]
    fn a_lens_glitch_or_mosaic_plays_over_the_live_frame_and_lets_go() {
        let _turn = serial();
        let Some(mut pipeline) = test_pipeline() else {
            return;
        };
        let scene = load_plain(&mut pipeline, "overlay", 0.2);
        pipeline.install(scene).unwrap();
        means(&mut pipeline, &[0.9, 0.95]);
        assert!(!pipeline.transitions.holds_textures());
        // Without a followed beat, 4 s: from 1.0 to 5.0.
        pipeline.start_overlay(Overlay::Mosaic);
        for time in [1.0, 1.1, 2.5, 4.9] {
            means(&mut pipeline, &[time]);
            assert_eq!(pipeline.overlay_running(), Some(Overlay::Mosaic), "{time}");
            assert!(!pipeline.transitions.has_held());
            // Started again while it plays, it goes on.
            if time < 4.0 {
                pipeline.start_overlay(Overlay::Mosaic);
            }
        }
        assert_eq!(pipeline.transitions.overlay.unwrap().start, 1.0);
        assert!(pipeline.transitions.holds_textures());
        means(&mut pipeline, &[5.02, 5.04]);
        assert_eq!(pipeline.overlay_running(), None);
        assert!(
            !pipeline.transitions.holds_textures(),
            "nothing is held after it"
        );

        // A scene that dresses its cuts keeps its frame under it.
        let dir = dressed_scene("overlay-dressed", 0.2, "glitch");
        let scene = pipeline.scene_build_dir(&dir).run().unwrap();
        pipeline.install(scene).unwrap();
        means(&mut pipeline, &[6.0, 6.02]);
        pipeline.start_overlay(Overlay::Glitch);
        means(&mut pipeline, &[6.04, 6.1]);
        assert_eq!(pipeline.overlay_running(), Some(Overlay::Glitch));
        assert!(pipeline.transitions.has_kept());
        // A cut the lab asks for is dressed under it, without waiting for it to end.
        pipeline.cut_dressed(Transition::Fade);
        let mut dressed = false;
        for i in 0..6 {
            means(&mut pipeline, &[6.12 + f64::from(i) / 60.0]);
            dressed |= pipeline.transitions.dressed.is_some();
        }
        assert!(dressed);
        assert_eq!(pipeline.overlay_running(), Some(Overlay::Glitch));
        // The other kind takes its place.
        pipeline.start_overlay(Overlay::Mosaic);
        means(&mut pipeline, &[6.3, 6.32]);
        assert_eq!(pipeline.overlay_running(), Some(Overlay::Mosaic));
        // A switch takes its place.
        let next = load_plain(&mut pipeline, "overlay-next", 0.5);
        let durations = Durations {
            out: 0.0,
            into: 0.2,
        };
        pipeline
            .switch_to(next, Transition::Dissolve, durations)
            .unwrap();
        means(&mut pipeline, &[6.4, 6.42]);
        assert_eq!(pipeline.overlay_running(), None);
    }

    /// A newer switch replaces a scene that still waits, and cuts in over one that is in.
    #[test]
    fn a_switch_during_a_transition_replaces_the_waiting_scene_or_cuts() {
        let _turn = serial();
        let Some(mut pipeline) = test_pipeline() else {
            return;
        };
        let durations = Durations {
            out: 1.0,
            into: 1.0,
        };
        let a = load_plain(&mut pipeline, "during-a", 1.0);
        pipeline.switch_to(a, Transition::Fade, durations).unwrap();
        let b = load_plain(&mut pipeline, "during-b", 0.2);
        pipeline.switch_to(b, Transition::Fade, durations).unwrap();
        assert_eq!(pipeline.preset_name(), "during-b");
        // Past the fade-out: "during-b" is in, and a third scene cuts.
        pipeline.render(0.0).unwrap();
        pipeline.render(1.25).unwrap();
        assert_eq!(pipeline.scene.name, "during-b");
        let c = load_plain(&mut pipeline, "during-c", 0.5);
        pipeline.switch_to(c, Transition::Fade, durations).unwrap();
        assert_eq!(pipeline.scene.name, "during-c");
        assert!(pipeline.transitions.running.is_none());
    }

    /// A resize ends a transition with the new scene installed, at the new size.
    #[test]
    fn a_resize_ends_a_transition() {
        let _turn = serial();
        let Some(mut pipeline) = test_pipeline() else {
            return;
        };
        let a = load_plain(&mut pipeline, "resize-a", 1.0);
        let durations = Durations {
            out: 1.0,
            into: 1.0,
        };
        pipeline.switch_to(a, Transition::Fade, durations).unwrap();
        pipeline.render(0.0).unwrap();
        pipeline.resize(96, 48).unwrap();
        assert_eq!(pipeline.scene.name, "resize-a");
        assert_eq!(pipeline.scene.output_size(), (96, 48));
        assert!(pipeline.transitions.running.is_none());
        pipeline.render(0.1).unwrap();
    }

    /// The lines logged at info level while the tests run.
    struct Lines(std::sync::Mutex<Vec<String>>);

    impl log::Log for Lines {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= log::Level::Info
        }

        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                let mut lines = self.0.lock().unwrap_or_else(|e| e.into_inner());
                lines.push(record.args().to_string());
            }
        }

        fn flush(&self) {}
    }

    static LINES: Lines = Lines(std::sync::Mutex::new(Vec::new()));

    /// Growth with a budget of 1 ms a frame, drawn at about 200 fps by the sleep between frames,
    /// whatever the resolution: the GPU time is over the budget and the frame rate under the
    /// target, so the controller lowers the level, sees that the frame rate does not follow and
    /// puts it back (and says so), and the frame rate is logged once per interval (1 s here,
    /// 10 s in the app).
    ///
    /// It judges the frame rate by the wall clock, so other tests drawing on the GPU at the same
    /// time can make a lowering look as if it helped: run it alone (`-- --ignored`). The rules
    /// themselves are pinned by the tests in `crate::adaptive`, which need no GPU.
    #[test]
    #[ignore]
    fn a_lowering_that_does_not_raise_the_frame_rate_is_undone_and_logged() {
        let Ok(mut pipeline) = Pipeline::new(
            1920,
            1080,
            44_100,
            true,
            None,
            None,
            Arc::new(|_| {}),
            #[cfg(target_os = "android")]
            None,
        ) else {
            eprintln!("SKIPPED: no GPU adapter");
            return;
        };
        if log::set_logger(&LINES).is_ok() {
            log::set_max_level(log::LevelFilter::Info);
        }
        pipeline.load_builtin_scene("Growth").unwrap();
        pipeline.set_target_fps(1000);
        pipeline.report = Report::new(1.0);
        let started = Instant::now();
        let mut levels = Vec::new();
        let undone = |lines: &std::sync::Mutex<Vec<String>>| {
            lines
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.starts_with("Visualizer Growth:") && l.contains("did not help"))
        };
        while started.elapsed().as_secs_f64() < 15.0 && !undone(&LINES.0) {
            pipeline.render(started.elapsed().as_secs_f64()).unwrap();
            let level = pipeline.controller.level();
            if levels.last() != Some(&level) {
                eprintln!("{:.2} s: {level:?}", started.elapsed().as_secs_f64());
                levels.push(level);
            }
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
        // One more second for a line with the frame rate.
        let last = Instant::now();
        while last.elapsed().as_secs_f64() < 1.2 {
            pipeline.render(started.elapsed().as_secs_f64()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
        let lines: Vec<String> = LINES
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with("Visualizer Growth:"))
            .cloned()
            .collect();
        for line in &lines {
            eprintln!("{line}");
        }
        assert!(undone(&LINES.0), "{levels:?}");
        assert_eq!(pipeline.controller.level(), Level::FULL, "{levels:?}");
        assert!(levels.len() >= 2, "it never lowered: {levels:?}");
        assert!(lines.iter().any(|l| l.contains(" fps, GPU ")), "{lines:?}");
    }
}

/// Probes for making a scene cheaper without changing its picture: ignored tests, run by hand on
/// a machine with a GPU, one at a time (`cargo test -p visualizer --lib probe_scene -- --ignored
/// --nocapture --test-threads 1`).
///
/// Every probe draws at the phone's size on the same timeline: 60 frames a second, a steady
/// 120 bpm in bars of four (so a scene grows, changes its view and its colours as it would with
/// music), and each frame's seed fixed, so a scene draws the same frames on every run.
/// The music's signals move on by wall time, not by the timeline's (`EventCounter::wall_dt`), and
/// a scene takes the music over from the one before it (`Scene::take_music_from`). Left alone,
/// that made the first frames differ between runs, and between two draws of the same scene in one
/// run (the second started from where the first ended): a mean difference of 1 to 3.5 in the first
/// 20 frames that vanished on a repeat run, since only a cold run, with its shaders compiling, took
/// long enough for the wall time of those frames to differ much. `load` therefore starts the
/// music over with every frame counted as 1/60 s of wall time.
///
/// - `PROBE`: the scenes, built-in names or absolute scene folders, comma-separated (default
///   Growth). `probe_scene_compare` takes one, and the scene to compare it with in `PROBE_B`.
/// - `PROBE_LEVELS`: the levels, `factor:quality` pairs, comma-separated; `PROBE_LEVEL_A` the
///   one level `probe_scene_compare` draws `PROBE` at (default 1:1).
/// - `PROBE_FRAMES`: the frames to save or compare, by number from the scene's start.
/// - `PROBE_OUT`: the folder PNGs are saved in.
/// - `PROBE_SEED`: the seed of the scene's choices (default 1); another one is another timeline,
///   with other views and colours.
/// - `PROBE_COST_FRAMES`, `PROBE_REPEATS`: see `probe_scene_cost`.
///
/// For example, whether a changed copy of a scene still draws the same picture at full level,
/// and how it looks at a lower quality:
///
/// ```text
/// PROBE=/path/to/original PROBE_B=/path/to/changed PROBE_LEVELS=1:1,1:0.5 PROBE_OUT=/tmp/cmp \
///   cargo test -p visualizer --lib probe_scene_compare -- --ignored --nocapture
/// ```
#[cfg(test)]
mod probe {
    use std::fmt::Write as _;
    use std::path::PathBuf;

    use super::*;
    use crate::scene::argb_to_linear;

    /// The visualizer's size on the phone (an iPhone 13 mini's cover wall).
    const SIZE: (u32, u32) = (448, 970);
    const FPS: f64 = 60.0;
    const BPM: f64 = 120.0;
    /// The visualizer's clock at frame 0 (a scene's own clock starts at 0 there).
    const START: f64 = 30.0;
    const COST_LEVELS: &str = "1:1,0.75:1,0.5:1,0.5:0,0.25:0";
    /// Early and late in the tower's growth, and after each of the first two changes of light
    /// (bar 16, frame 1920, and bar 32, frame 3840).
    const FRAMES: &str = "120,600,1500,2100,2700,4000";

    fn env_or(name: &str, default: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| default.to_string())
    }

    fn levels(default: &str) -> Vec<Level> {
        levels_from("PROBE_LEVELS", default)
    }

    fn levels_from(name: &str, default: &str) -> Vec<Level> {
        env_or(name, default)
            .split(',')
            .map(|pair| {
                let (factor, quality) = pair.split_once(':').expect("levels are factor:quality");
                Level {
                    factor: factor.trim().parse().expect("a factor"),
                    quality: quality.trim().parse().expect("a quality"),
                }
            })
            .collect()
    }

    fn frame_numbers() -> Vec<u32> {
        let mut frames: Vec<u32> = env_or("PROBE_FRAMES", FRAMES)
            .split(',')
            .map(|n| n.trim().parse().expect("a frame number"))
            .collect();
        frames.sort_unstable();
        frames
    }

    fn out_dir() -> Option<PathBuf> {
        let dir = PathBuf::from(std::env::var("PROBE_OUT").ok()?);
        std::fs::create_dir_all(&dir).expect("PROBE_OUT can be made");
        Some(dir)
    }

    /// A short name for file names: a built-in scene's name, or a folder's last two parts.
    fn label(scene: &str) -> String {
        let path = Path::new(scene);
        let parts: Vec<_> = path
            .components()
            .rev()
            .take(if path.is_absolute() { 2 } else { 1 })
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        parts
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("-")
            .replace(' ', "_")
    }

    fn level_label(level: Level) -> String {
        format!("f{}-q{}", level.factor, level.quality)
    }

    fn pipeline() -> Pipeline {
        // PROBE_SIZE=<width>x<height> in place of the reference phone's.
        let size = std::env::var("PROBE_SIZE").ok().map_or(SIZE, |s| {
            let (w, h) = s.split_once('x').expect("PROBE_SIZE is WxH");
            (w.parse().unwrap(), h.parse().unwrap())
        });
        let mut pipeline = Pipeline::new(
            size.0,
            size.1,
            44_100,
            true,
            None,
            None,
            Arc::new(|e| panic!("GPU error: {e}")),
            #[cfg(target_os = "android")]
            None,
        )
        .expect("a GPU");
        assert!(pipeline.has_timestamps(), "the probe needs GPU timestamps");
        pipeline.record_frame_times(true);
        pipeline
    }

    /// Start `scene` from its beginning at `level`, with its seeds fixed (`PROBE_SEED`).
    fn load(pipeline: &mut Pipeline, scene: &str, level: Level) {
        if scene.starts_with('/') {
            pipeline.load_scene(Path::new(scene)).unwrap();
        } else {
            pipeline.load_builtin_scene(scene).unwrap();
        }
        pipeline.set_adaptive(false);
        pipeline
            .scene
            .set_level(&pipeline.device, &pipeline.queue, level);
        let seed = env_or("PROBE_SEED", "1").parse().expect("a seed");
        pipeline.scene.fix_seed(Some(seed));
        pipeline.scene.fix_wall_dt(1.0 / FPS);
        pipeline.take_frame_times();
    }

    /// What the tracker would hear at frame `i`: a steady beat, sure of itself, a kick on every
    /// beat.
    fn rhythm_at(i: u32) -> Rhythm {
        let beat = f64::from(i) / FPS * BPM / 60.0;
        Rhythm {
            bpm: BPM,
            beat,
            confidence: 0.9,
            kick: (-beat.fract() * 8.0).exp(),
            snare: 0.0,
            hat: 0.0,
            energy: 1.0,
        }
    }

    /// The pixels of a frame, RGBA, row after row.
    fn rgba(frame: &MappedFrame, (width, height): (u32, u32)) -> Vec<u8> {
        let row = width as usize * 4;
        let mut pixels = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            let at = y * frame.stride();
            pixels.extend_from_slice(&frame.bytes()[at..at + row]);
        }
        if FORMAT == wgpu::TextureFormat::Bgra8Unorm {
            for px in pixels.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
        }
        pixels
    }

    /// Draw frames 0 to `last` of the timeline, waiting for each, and hand `each` every frame
    /// number with its GPU time and, for the numbers in `keep`, its pixels.
    fn run(
        pipeline: &mut Pipeline,
        last: u32,
        keep: &[u32],
        each: impl FnMut(u32, &[f64], Option<Vec<u8>>),
    ) {
        run_with(pipeline, last, keep, |_, _| {}, each);
    }

    /// The same, with `before` called on the pipeline ahead of drawing each frame number.
    fn run_with(
        pipeline: &mut Pipeline,
        last: u32,
        keep: &[u32],
        mut before: impl FnMut(&mut Pipeline, u32),
        mut each: impl FnMut(u32, &[f64], Option<Vec<u8>>),
    ) {
        let size = pipeline.size();
        // A read-back frame comes out of the call after the one that drew it (the probe waits
        // for each), so one call more than there are frames.
        for i in 0..=last + 1 {
            before(pipeline, i);
            pipeline.set_rhythm(&rhythm_at(i));
            let rendered = pipeline.render(START + f64::from(i) / FPS).unwrap();
            pipeline
                .device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let times = pipeline.take_frame_times();
            if i == 0 {
                continue;
            }
            let drawn = i - 1;
            let pixels = keep.contains(&drawn).then(|| {
                let Rendered::Mapped(frame) = &rendered else {
                    panic!("frame {drawn} did not come out on the next call");
                };
                rgba(frame, size)
            });
            // The times collected now are of the frame before (`adapt` runs before drawing).
            each(drawn, &times, pixels);
        }
    }

    fn save_png(path: &Path, pixels: Vec<u8>, (width, height): (u32, u32)) {
        image::RgbaImage::from_raw(width, height, pixels)
            .expect("the size matches")
            .save(path)
            .unwrap();
    }

    /// The GPU time of each scene at each level, in milliseconds: of each frame the least of
    /// `PROBE_REPEATS` runs of the timeline (default 3; other work on the GPU only ever adds to a
    /// frame's time), then the median and the 90th percentile of those over `PROBE_COST_FRAMES`
    /// frames (default 600) after the first 60. The runs take turns between the scenes and
    /// levels, so that a GPU clocking up or down, or busy with something else for a while, weighs
    /// on all of them alike.
    #[test]
    #[ignore = "a probe, run by hand"]
    fn probe_scene_cost() {
        let mut pipeline = pipeline();
        let frames: u32 = env_or("PROBE_COST_FRAMES", "600").parse().unwrap();
        let repeats: usize = env_or("PROBE_REPEATS", "3").parse().unwrap();
        let scenes = env_or("PROBE", "Growth");
        let runs: Vec<(&str, Level)> = scenes
            .split(',')
            .flat_map(|scene| {
                levels(COST_LEVELS)
                    .into_iter()
                    .map(move |level| (scene, level))
            })
            .collect();
        let mut least = vec![vec![f64::INFINITY; frames as usize + 1]; runs.len()];
        let mut sizes = vec![(0, 0); runs.len()];
        for _ in 0..repeats {
            for (k, &(scene, level)) in runs.iter().enumerate() {
                load(&mut pipeline, scene, level);
                sizes[k] = pipeline.scene.internal_size();
                let least = &mut least[k];
                // With PROBE_HOLD=<shot index>, the director holds that shot, cut to again every
                // PROBE_HOLD_EVERY frames (default 60).
                let hold: Option<u32> =
                    std::env::var("PROBE_HOLD").ok().map(|s| s.parse().unwrap());
                let every: u32 = env_or("PROBE_HOLD_EVERY", "60").parse().unwrap();
                let before = |pipeline: &mut Pipeline, i: u32| {
                    if let Some(index) = hold
                        && i % every == 1
                    {
                        pipeline.hold_shot(Some(index));
                    }
                };
                run_with(&mut pipeline, frames, &[], before, |i, time, _| {
                    if let Some(&time) = time.last() {
                        least[i as usize] = least[i as usize].min(time);
                    }
                });
            }
        }
        for (k, &(scene, level)) in runs.iter().enumerate() {
            let mut times: Vec<f64> = least[k][60..]
                .iter()
                .copied()
                .filter(|t| t.is_finite())
                .collect();
            times.sort_by(f64::total_cmp);
            let at = |q: usize| times.get(times.len() * q / 100).copied().unwrap_or(0.0) * 1e3;
            eprintln!(
                "PROBE {scene} factor={} quality={} size={:?} median={:.3} ms p90={:.3} ms n={}",
                level.factor,
                level.quality,
                sizes[k],
                at(50),
                at(90),
                times.len()
            );
        }
    }

    /// The lens effects of scene `PROBE` (default Armillary) started from the lab's commands, at
    /// the app's accent, and a flash (a switch to Torii Sea, and one dressing a cut of the
    /// director): frames over each one's course as PNGs in `PROBE_OUT`
    /// (`lens-<scene>-<effect>-<frames since it started>.png`, at 60 fps and 120 bpm, so 30
    /// frames are a beat) and one with no effect; then the GPU time of the scene with each lens
    /// effect kept running against none, as the median over `PROBE_COST_FRAMES` frames (default
    /// 600) after the first 60, in three rounds.
    #[test]
    #[ignore = "a probe, run by hand"]
    fn probe_lens() {
        let mut pipeline = pipeline();
        pipeline.set_accent(Some(argb_to_linear(0xffe8602c)));
        let size = pipeline.size();
        let scene = env_or("PROBE", "Armillary");
        let out = out_dir();
        /// The frame an effect starts at.
        const AT: u32 = 100;
        /// What starts an effect.
        type Start = fn(&mut Pipeline);
        // The effect and the frames after its start that are looked at.
        // Across the phrase of the long effects (the rest of four bars from the start: 14 beats
        // here, about 420 frames).
        const PHRASE: &[u32] = &[0, 40, 80, 120, 160, 200, 240, 280, 320, 360, 400];
        // Around the peak, a half-beat (15 frames) apart: a mosaic's blocks live on.
        const STEPS: &[u32] = &[120, 135, 150, 165, 180, 195, 210];
        // Around a beat near the peak (frame 330, 11 beats in): the frame before it, then 0, 2, 5,
        // 10 and 25 frames after, the push and the settle.
        const PUSH: &[u32] = &[229, 230, 232, 235, 240, 255];
        let looks: [(&str, Start, &[u32]); 11] = [
            ("none", |_| {}, &[0]),
            ("flare", Pipeline::start_flare, PHRASE),
            ("leak", Pipeline::start_leak, PHRASE),
            (
                "mosaic-overlay",
                |pipeline| pipeline.start_overlay(Overlay::Mosaic),
                PHRASE,
            ),
            (
                "mosaic-steps",
                |pipeline| pipeline.start_overlay(Overlay::Mosaic),
                STEPS,
            ),
            (
                "glitch-overlay",
                |pipeline| pipeline.start_overlay(Overlay::Glitch),
                PHRASE,
            ),
            (
                "mosaic-push",
                |pipeline| pipeline.start_overlay(Overlay::Mosaic),
                PUSH,
            ),
            (
                "glitch-push",
                |pipeline| pipeline.start_overlay(Overlay::Glitch),
                PUSH,
            ),
            // A beat up (30 frames) and two down.
            (
                "flash",
                |pipeline| {
                    let next = pipeline
                        .scene_build_builtin("Torii Sea")
                        .unwrap()
                        .run()
                        .unwrap();
                    let (kind, durations) =
                        switching::requested(Transition::Flash, pipeline.music());
                    pipeline.switch_to(next, kind, durations).unwrap();
                },
                &[0, 3, 6, 9, 12, 16, 20, 24, 28, 31, 40, 60, 80],
            ),
            // A beat (30 frames), the new scene in at half way.
            (
                "mosaic-switch",
                |pipeline| {
                    let next = pipeline
                        .scene_build_builtin("Torii Sea")
                        .unwrap()
                        .run()
                        .unwrap();
                    let (kind, durations) =
                        switching::requested(Transition::Mosaic, pipeline.music());
                    pipeline.switch_to(next, kind, durations).unwrap();
                },
                &[0, 3, 6, 9, 12, 14, 16, 18, 21, 24, 27, 30],
            ),
            // A quarter of a beat up and half a beat down, from the frame after the start.
            (
                "flash-cut",
                |pipeline| pipeline.cut_dressed(Transition::Flash),
                &[0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 14, 18, 24],
            ),
        ];
        for (name, start, after) in looks {
            load(&mut pipeline, &scene, Level::FULL);
            let keep: Vec<u32> = after.iter().map(|a| AT + a).collect();
            run_with(
                &mut pipeline,
                *keep.last().unwrap(),
                &keep,
                |pipeline, i| {
                    if i == AT {
                        start(pipeline);
                    }
                },
                |i, _, pixels| {
                    if let (Some(pixels), Some(out)) = (pixels, &out) {
                        let file = format!("lens-{}-{name}-{:03}.png", label(&scene), i - AT);
                        save_png(&out.join(&file), pixels, size);
                        eprintln!("PROBE saved {file}");
                    }
                },
            );
        }
        let frames: u32 = env_or("PROBE_COST_FRAMES", "600").parse().unwrap();
        // Each effect started again before it has gone, so that one is always under way.
        let kept: [(&str, Start, u32); 3] = [
            ("none", |_| {}, 1),
            ("flare", Pipeline::start_flare, 120),
            ("leak", Pipeline::start_leak, 120),
        ];
        let mut cost = |start: Start, every: u32| {
            load(&mut pipeline, &scene, Level::FULL);
            let mut times = Vec::new();
            run_with(
                &mut pipeline,
                frames,
                &[],
                |pipeline, i| {
                    if i % every == 0 {
                        start(pipeline);
                    }
                },
                |i, time, _| {
                    if let (true, Some(&time)) = (i >= 60, time.last()) {
                        times.push(time);
                    }
                },
            );
            times.sort_by(f64::total_cmp);
            times[times.len() / 2] * 1e3
        };
        for round in 0..3 {
            let costs: Vec<f64> = kept
                .iter()
                .map(|&(_, start, every)| cost(start, every))
                .collect();
            let mut line = format!("PROBE round {round}: none {:.3} ms", costs[0]);
            for ((name, _, _), ms) in kept.iter().zip(&costs).skip(1) {
                line += &format!(", {name} {ms:.3} ms (+{:.3})", ms - costs[0]);
            }
            eprintln!("{line}");
        }
    }

    /// Where light leaks come from: scene `PROBE` (default Neon Street) with a leak started a
    /// bar apart (at frames 100, 220, 340, ... at 120 bpm, so at other places of the song), each
    /// at its peak (160 frames on), drawn with and without it; the frame and the leak's own layer
    /// (the difference, three times as strong) as PNGs in `PROBE_OUT` (`leak-place-<start>.png`,
    /// `leak-layer-<start>.png`).
    #[test]
    #[ignore = "a probe, run by hand"]
    fn probe_leak_places() {
        let mut pipeline = pipeline();
        pipeline.set_accent(Some(argb_to_linear(0xffe8602c)));
        let size = pipeline.size();
        let scene = env_or("PROBE", "Neon Street");
        let out = out_dir().expect("PROBE_OUT names a folder");
        for start in [100u32, 220, 340, 460, 580, 700] {
            let look = start + 160;
            let mut frames = Vec::new();
            for leak in [false, true] {
                load(&mut pipeline, &scene, Level::FULL);
                run_with(
                    &mut pipeline,
                    look,
                    &[look],
                    |pipeline, i| {
                        if leak && i == start {
                            pipeline.start_leak();
                        }
                    },
                    |_, _, pixels| frames.extend(pixels),
                );
            }
            let layer: Vec<u8> = frames[0]
                .iter()
                .zip(&frames[1])
                .enumerate()
                .map(|(k, (a, b))| {
                    if k % 4 == 3 {
                        255
                    } else {
                        (b.abs_diff(*a) as u32 * 3).min(255) as u8
                    }
                })
                .collect();
            save_png(&out.join(format!("leak-layer-{start:03}.png")), layer, size);
            let frame = frames.pop().unwrap();
            save_png(&out.join(format!("leak-place-{start:03}.png")), frame, size);
            eprintln!("PROBE saved leak-place-{start:03}");
        }
    }

    /// How much a glitch or a mosaic over the picture (`PROBE_OVERLAY`, default mosaic) shows:
    /// scene `PROBE` (default Neon Street) drawn with and without one started at frame 100; for
    /// the frames `PROBE_LAYER_FRAMES` after its start (default a half-beat apart over its
    /// phrase), the share of the pixels it changes by more than 3 of 255, the largest change, and
    /// the share of the changed pixels that were changed in the frame looked at before too. With
    /// `PROBE_OUT`, the layer (the difference, four times as strong) of each as
    /// `<overlay>-layer-<frames since its start>.png`.
    #[test]
    #[ignore = "a probe, run by hand"]
    fn probe_overlay_layer() {
        let mut pipeline = pipeline();
        pipeline.set_accent(Some(argb_to_linear(0xffe8602c)));
        let size = pipeline.size();
        let out = out_dir();
        let scene = env_or("PROBE", "Neon Street");
        let (name, kind) = match env_or("PROBE_OVERLAY", "mosaic").as_str() {
            "glitch" => ("glitch", Overlay::Glitch),
            _ => ("mosaic", Overlay::Mosaic),
        };
        const AT: u32 = 100;
        let half_beats: Vec<String> = (0..26).map(|k| (15 * k).to_string()).collect();
        let keep: Vec<u32> = env_or("PROBE_LAYER_FRAMES", &half_beats.join(","))
            .split(',')
            .map(|n| AT + n.trim().parse::<u32>().expect("a frame number"))
            .collect();
        let last = *keep.last().unwrap();
        let mut runs = [Vec::new(), Vec::new()];
        for (k, frames) in runs.iter_mut().enumerate() {
            load(&mut pipeline, &scene, Level::FULL);
            run_with(
                &mut pipeline,
                last,
                &keep,
                |pipeline, i| {
                    if k == 1 && i == AT {
                        pipeline.start_overlay(kind);
                    }
                },
                |_, _, pixels| frames.extend(pixels),
            );
        }
        let mut before: Option<Vec<bool>> = None;
        for (n, (a, b)) in runs[0].iter().zip(&runs[1]).enumerate() {
            let changes: Vec<u8> = a
                .as_chunks::<4>()
                .0
                .iter()
                .zip(b.as_chunks::<4>().0)
                .map(|(p, q)| (0..3).map(|c| p[c].abs_diff(q[c])).max().unwrap())
                .collect();
            let changed: Vec<bool> = changes.iter().map(|&d| d > 3).collect();
            if let Some(out) = &out {
                let layer: Vec<u8> = changes
                    .iter()
                    .flat_map(|&d| {
                        let v = (u32::from(d) * 4).min(255) as u8;
                        [v, v, v, 255]
                    })
                    .collect();
                let file = format!("{name}-layer-{:03}.png", keep[n] - AT);
                save_png(&out.join(file), layer, size);
            }
            let count = changed.iter().filter(|&&c| c).count();
            let most = changes.iter().copied().max().unwrap_or(0);
            let kept = before.as_ref().map_or(0.0, |before| {
                let both = changed
                    .iter()
                    .zip(before)
                    .filter(|(a, b)| **a && **b)
                    .count();
                both as f64 / count.max(1) as f64
            });
            eprintln!(
                "PROBE frame +{:3}: changed {:.2} %, most {most}, kept from the frame before {:.0} %",
                keep[n] - AT,
                count as f64 / changed.len() as f64 * 100.0,
                kept * 100.0
            );
            before = Some(changed);
        }
    }

    /// How steady the lens flare is: scene `PROBE` (default Neon Street) is drawn twice over the
    /// same `PROBE_STEADY_FRAMES` frames in a row (default 20, from frame 100), without a flare
    /// and with one held at its full swell; the flare's layer is the difference. For each pair of
    /// frames in a row it prints how much the layer changed (the mean, the 99th percentile and
    /// the largest change of a colour channel, of 255) beside how much the layer is there and how
    /// much the picture itself changed. The scene is drawn without a flare twice, and how far the
    /// two differ is printed first: a scene that does not draw the same frames every time cannot
    /// be measured so. With `PROBE_OUT`, the first frames with the flare and its layer (four
    /// times as strong) are saved there.
    #[test]
    #[ignore = "a probe, run by hand"]
    fn probe_flare_steady() {
        let mut pipeline = pipeline();
        pipeline.set_accent(Some(argb_to_linear(0xffe8602c)));
        let size = pipeline.size();
        let scene = env_or("PROBE", "Neon Street");
        let count: u32 = env_or("PROBE_STEADY_FRAMES", "20").parse().unwrap();
        const AT: u32 = 100;
        let keep: Vec<u32> = (AT..AT + count).collect();
        let last = *keep.last().unwrap();
        // The first scene loaded after the pipeline is made can draw a different shot.
        load(&mut pipeline, &scene, Level::FULL);
        run(&mut pipeline, 2, &[], |_, _, _| {});
        let mut plain = Vec::new();
        load(&mut pipeline, &scene, Level::FULL);
        run(&mut pipeline, last, &keep, |_, _, pixels| {
            plain.extend(pixels)
        });
        let mut again = Vec::new();
        load(&mut pipeline, &scene, Level::FULL);
        run(&mut pipeline, last, &keep, |_, _, pixels| {
            again.extend(pixels)
        });
        let differ: u64 = plain
            .iter()
            .zip(&again)
            .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| u64::from(a.abs_diff(*b))))
            .sum();
        eprintln!(
            "PROBE the plain runs differ by {:.4} on average",
            differ as f64 / (plain.len() * plain[0].len()) as f64
        );
        let mut flared = Vec::new();
        load(&mut pipeline, &scene, Level::FULL);
        run_with(
            &mut pipeline,
            last,
            &keep,
            |pipeline, i| {
                if i + 30 >= AT {
                    pipeline.scene.hold_flare();
                }
            },
            |_, _, pixels| flared.extend(pixels),
        );
        let layers: Vec<Vec<i16>> = plain
            .iter()
            .zip(&flared)
            .map(|(a, b)| {
                a.iter()
                    .zip(b)
                    .enumerate()
                    .filter(|(k, _)| k % 4 != 3)
                    .map(|(_, (&a, &b))| i16::from(b) - i16::from(a))
                    .collect()
            })
            .collect();
        let out = out_dir();
        for (k, layer) in layers.iter().enumerate().take(3) {
            if let Some(out) = &out {
                let pixels: Vec<u8> = layer
                    .chunks(3)
                    .flat_map(|p| {
                        let c = |v: i16| (v.unsigned_abs() * 4).min(255) as u8;
                        [c(p[0]), c(p[1]), c(p[2]), 255]
                    })
                    .collect();
                let name = label(&scene);
                save_png(
                    &out.join(format!("steady-{name}-layer-{k:02}.png")),
                    pixels,
                    size,
                );
                let frame = flared[k].clone();
                save_png(
                    &out.join(format!("steady-{name}-flare-{k:02}.png")),
                    frame,
                    size,
                );
            }
        }
        let mean_abs = |v: &mut dyn Iterator<Item = u16>| {
            let (sum, n) = v.fold((0u64, 0u64), |(s, n), x| (s + u64::from(x), n + 1));
            sum as f64 / n as f64
        };
        let mut worst_mean = 0.0f64;
        let mut worst_energy = 0.0f64;
        for k in 1..layers.len() {
            let mut changes: Vec<u16> = layers[k]
                .iter()
                .zip(&layers[k - 1])
                .map(|(a, b)| (a - b).unsigned_abs())
                .collect();
            let mean = mean_abs(&mut changes.iter().copied());
            changes.sort_unstable();
            let p99 = changes[changes.len() * 99 / 100];
            let max = changes[changes.len() - 1];
            let there = mean_abs(&mut layers[k].iter().map(|v| v.unsigned_abs()));
            let picture = mean_abs(
                &mut plain[k]
                    .iter()
                    .zip(&plain[k - 1])
                    .map(|(a, b)| u16::from(a.abs_diff(*b))),
            );
            // How much the layer's light as a whole changed: what a flicker or a pumping shows
            // as, which the picture's motion alone does not.
            let before = mean_abs(&mut layers[k - 1].iter().map(|v| v.unsigned_abs()));
            let energy = (there - before).abs() / there.max(1e-6);
            worst_mean = worst_mean.max(mean);
            worst_energy = worst_energy.max(energy);
            eprintln!(
                "PROBE frame {}: layer change mean {mean:.3} p99 {p99} max {max}; layer {there:.2} \
                 ({:+.1} %); picture change {picture:.2}",
                keep[k],
                (there - before) / before.max(1e-6) * 100.0
            );
        }
        eprintln!(
            "PROBE worst: mean change of the layer {worst_mean:.3}, change of its light {:.1} %",
            worst_energy * 100.0
        );
    }

    /// The frames of `PROBE_FRAMES` of each scene at each level (default full), as PNGs in
    /// `PROBE_OUT`.
    #[test]
    #[ignore = "a probe, run by hand"]
    fn probe_scene_frames() {
        let out = out_dir().expect("PROBE_OUT names a folder");
        let frames = frame_numbers();
        let mut pipeline = pipeline();
        let size = pipeline.size();
        // With PROBE_HOLD=<shot index>, the director cuts to that shot and holds it, again every
        // PROBE_HOLD_EVERY frames (default 60), each time a new cut (a new shot.seed).
        let hold: Option<u32> = std::env::var("PROBE_HOLD").ok().map(|s| s.parse().unwrap());
        let every: u32 = env_or("PROBE_HOLD_EVERY", "60").parse().unwrap();
        for scene in env_or("PROBE", "Growth").split(',') {
            for level in levels("1:1") {
                load(&mut pipeline, scene, level);
                run_with(
                    &mut pipeline,
                    *frames.last().unwrap(),
                    &frames,
                    |pipeline, i| {
                        if let Some(index) = hold
                            && i % every == 1
                        {
                            pipeline.hold_shot(Some(index));
                        }
                    },
                    |i, _, pixels| {
                        if let Some(pixels) = pixels {
                            let name =
                                format!("{}-{}-{i:05}.png", label(scene), level_label(level));
                            save_png(&out.join(&name), pixels, size);
                            eprintln!("PROBE saved {name}");
                        }
                    },
                );
            }
        }
    }

    /// How far scene `PROBE_B` at each level (default full) is from scene `PROBE` at level
    /// `PROBE_LEVEL_A` (default full), frame by frame: the largest and mean difference of a
    /// colour channel (of 255) and the PSNR.
    /// With `PROBE_OUT`, the pair of frames that differ most at each level and their difference
    /// (16 times as strong) are saved there.
    #[test]
    #[ignore = "a probe, run by hand"]
    fn probe_scene_compare() {
        let a = env_or("PROBE", "Growth");
        let b = std::env::var("PROBE_B").expect("PROBE_B names the scene to compare with");
        let frames = frame_numbers();
        let last = *frames.last().unwrap();
        let mut pipeline = pipeline();
        let size = pipeline.size();
        let mut reference = Vec::new();
        let level_a = levels_from("PROBE_LEVEL_A", "1:1")[0];
        load(&mut pipeline, &a, level_a);
        run(&mut pipeline, last, &frames, |_, _, pixels| {
            reference.extend(pixels);
        });
        for level in levels("1:1") {
            load(&mut pipeline, &b, level);
            let mut drawn = Vec::new();
            run(&mut pipeline, last, &frames, |_, _, pixels| {
                drawn.extend(pixels)
            });
            let mut report = String::new();
            let mut worst: Option<(f64, usize)> = None;
            for (k, (x, y)) in reference.iter().zip(&drawn).enumerate() {
                let (mut max, mut sum, mut squares, mut n) = (0u8, 0u64, 0u64, 0u64);
                for (p, q) in x.as_chunks::<4>().0.iter().zip(y.as_chunks::<4>().0) {
                    for c in 0..3 {
                        let d = p[c].abs_diff(q[c]);
                        max = max.max(d);
                        sum += u64::from(d);
                        squares += u64::from(d) * u64::from(d);
                        n += 1;
                    }
                }
                let mse = squares as f64 / n as f64;
                let psnr = if mse == 0.0 {
                    f64::INFINITY
                } else {
                    10.0 * (255.0 * 255.0 / mse).log10()
                };
                writeln!(
                    report,
                    "PROBE frame {:5}: max {max:3} mean {:.4} psnr {psnr:.2} dB",
                    frames[k],
                    sum as f64 / n as f64
                )
                .unwrap();
                if worst.is_none_or(|(p, _)| psnr < p) {
                    worst = Some((psnr, k));
                }
            }
            eprintln!(
                "PROBE {b} at {} against {a} at {}:\n{report}",
                level_label(level),
                level_label(level_a)
            );
            if let (Some(out), Some((_, k))) = (out_dir(), worst) {
                let stem = format!(
                    "compare-{}-{}-{:05}",
                    label(&b),
                    level_label(level),
                    frames[k]
                );
                let diff = reference[k]
                    .iter()
                    .zip(&drawn[k])
                    .enumerate()
                    .map(|(i, (p, q))| {
                        if i % 4 == 3 {
                            255
                        } else {
                            p.abs_diff(*q).saturating_mul(16)
                        }
                    })
                    .collect();
                save_png(
                    &out.join(format!("{stem}-a.png")),
                    reference[k].clone(),
                    size,
                );
                save_png(&out.join(format!("{stem}-b.png")), drawn[k].clone(), size);
                save_png(&out.join(format!("{stem}-diff.png")), diff, size);
                eprintln!("PROBE saved {stem}-{{a,b,diff}}.png");
            }
        }
    }
}
