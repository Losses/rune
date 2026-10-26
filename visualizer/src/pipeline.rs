//! The GPU side: a headless device, the scene being drawn and where frames go: on Apple platforms straight into
//! IOSurface-backed pixel buffers ([`crate::surface`]), on Windows into DirectX 12 textures shared
//! with Flutter's adapter (`crate::shared_dx12`), on Linux into Vulkan images that Flutter's
//! OpenGL context imports (`crate::shared_vulkan`), on Android presented into the native window
//! of a Flutter texture (`crate::window`), elsewhere (or if that fails) an offscreen target that
//! is read back into mapped buffers ([`crate::readback`]).

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

#[cfg(target_os = "android")]
use anyhow::bail;
use anyhow::{Context, Result, anyhow};
use log::{info, warn};

use crate::adaptive::{Controller, Level, Report};
use crate::chart::ChartState;
use crate::covers::{Atlas, Cell};
use crate::gpu_time::GpuTimer;
use crate::readback::{MappedFrame, Target};
use crate::rhythm::Rhythm;
use crate::scene::{FrameInput, Scene, Source};
#[cfg(target_os = "windows")]
use crate::shared_dx12::{SharedTargets, SharedTexture};
#[cfg(target_os = "linux")]
use crate::shared_vulkan::{SharedFrame, SharedTargets};
#[cfg(target_vendor = "apple")]
use crate::surface::{PixelBuffer, SurfaceTargets};
#[cfg(target_os = "android")]
use crate::window::{Window, WindowSurface};
use crate::{DEFAULT_FPS, DeviceUuids};

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
    sample_rate: u32,
    rhythm: Rhythm,
    chart: Option<ChartState>,
    song: (f64, bool),
    /// The app's accent colour (linear), once the app has told it.
    accent: Option<[f32; 3]>,
    /// The atlas of covers for scenes that read one.
    covers: Option<Arc<Atlas>>,
    /// The cell of the track playing, which stays over cell 0 of the atlas.
    playing_cover: Option<Arc<Cell>>,
    output: Output,
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
}

/// How often the GPU time of the frames is logged, in seconds.
const REPORT_INTERVAL: f64 = 10.0;

/// The name of the built-in scene shown before another is chosen.
pub const DEFAULT_PRESET_NAME: &str = "Armillary";

/// The built-in scene called `name`.
fn builtin_source(name: &str) -> Result<Source> {
    crate::builtin::find(name)
        .map(|scene| scene.source)
        .with_context(|| format!("There is no built-in scene {name}"))
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

/// Draw a frame into `view` and wait until the GPU has finished it, so that the target can be
/// handed out.
#[cfg(any(target_vendor = "apple", target_os = "windows"))]
fn render_and_wait(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    paint: impl FnOnce(&mut wgpu::CommandEncoder),
) -> Result<()> {
    let mut encoder = device.create_command_encoder(&Default::default());
    paint(&mut encoder);
    let submission = queue.submit(Some(encoder.finish()));
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .map_err(|e| anyhow!("GPU poll failed: {e:?}"))?;
    Ok(())
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
            adapter_name: adapter.get_info().name,
            device,
            queue,
            scene,
            sample_rate,
            rhythm: Rhythm::default(),
            chart: None,
            song: (0.0, false),
            accent: None,
            covers: None,
            playing_cover: None,
            output,
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
        let mut change = None;
        if let Some(recorded) = &mut self.recorded {
            recorded.extend_from_slice(&self.times);
        }
        for &seconds in &self.times {
            if let Some(summary) = self.report.frame(now, seconds) {
                let level = self.controller.level();
                let (width, height) = self.scene.internal_size();
                info!(
                    "Visualizer {}: GPU {:.1} ms on average, {:.1} ms at worst ({} frames); now resolution {:.2} ({width}x{height}), quality {:.2}",
                    self.scene.name,
                    summary.average * 1e3,
                    summary.worst * 1e3,
                    summary.frames,
                    level.factor,
                    level.quality,
                );
            }
            if self.adaptive {
                change = self.controller.frame(now, seconds).or(change);
            }
        }
        self.times.clear();
        if let Some(level) = change {
            self.scene.set_level(&self.device, &self.queue, level);
        }
    }

    /// The name of the scene on screen.
    pub fn preset_name(&self) -> &str {
        &self.scene.name
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
            self.output = Output::Readback(Target::new(&self.device, width, height));
        }
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
            return Ok(());
        }
        self.output = Output::new(&self.device, width, height, self.readback);
        Ok(())
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
            Output::Surface(surface) => surface.replace(&self.device, window, width, height),
            Output::Readback(_) => {
                warn!("Frames are read back, so the visualizer does not present into its window");
                drop(window);
                self.output = Output::Readback(Target::new(&self.device, width, height));
                Ok(())
            }
        }
    }

    /// Keep the beat tracker's findings for the scenes.
    pub fn set_rhythm(&mut self, rhythm: &Rhythm) {
        self.rhythm = *rhythm;
    }

    /// The song's chart at this frame, or none without one.
    pub fn set_chart(&mut self, chart: Option<&ChartState>) {
        self.chart = chart.copied();
    }

    /// The song time and whether it plays, for scenes.
    pub fn set_song(&mut self, time: f64, playing: bool) {
        self.song = (time, playing);
    }

    /// The app's accent colour (linear), for scenes.
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
        self.playing_cover = cell;
    }

    /// Whether the scene on screen reads the atlas of covers.
    pub fn wants_covers(&self) -> bool {
        self.scene.wants_covers()
    }

    pub fn push_pcm(&mut self, interleaved: &[f32], channels: usize) {
        self.scene.push_pcm(interleaved, channels);
    }

    /// Show the scene in the folder `dir`.
    pub fn load_scene(&mut self, dir: &Path) -> Result<()> {
        self.load_source(Source::Dir(dir.to_path_buf()))
            .with_context(|| format!("Scene {}", dir.display()))
    }

    /// Show the built-in scene `name`.
    pub fn load_builtin_scene(&mut self, name: &str) -> Result<()> {
        self.load_source(builtin_source(name)?)
            .with_context(|| format!("Built-in scene {name}"))
    }

    /// Show the built-in scene that comes first.
    pub fn load_default_scene(&mut self) -> Result<()> {
        self.load_builtin_scene(DEFAULT_PRESET_NAME)
    }

    fn load_source(&mut self, source: Source) -> Result<()> {
        let (width, height) = self.size();
        let mut scene = Scene::load_from(
            &self.device,
            &self.queue,
            source,
            self.format,
            (width, height),
            self.sample_rate,
        )?;
        scene.set_covers(
            &self.device,
            &self.queue,
            self.covers.clone(),
            self.playing_cover.clone(),
        );
        self.scene = scene;
        self.restart_adaptive();
        Ok(())
    }

    /// Read the scene on screen again if its files changed; whether it did.
    pub fn reload_scene(&mut self) -> Result<bool> {
        let source = self.scene.source().to_string();
        let changed = self
            .scene
            .reload(&self.device, &self.queue)
            .with_context(|| format!("Scene {source}"))?;
        if changed {
            self.restart_adaptive();
        }
        Ok(changed)
    }

    /// Render one frame: into the next pixel buffer, shared texture, shared image or image of the
    /// window's swapchain, or into the target that is read back. The resolution and quality of
    /// the scene follow the GPU time of the frames before (see [`crate::adaptive`]).
    pub fn render(&mut self, time: f64) -> Result<Rendered> {
        self.adapt();
        let rendered = self.render_frame(time);
        self.timer.submitted(&self.device);
        rendered
    }

    fn render_frame(&mut self, time: f64) -> Result<Rendered> {
        #[cfg(target_os = "android")]
        if let Output::Surface(surface) = &mut self.output {
            let scene = &mut self.scene;
            let (device, queue) = (&self.device, &self.queue);
            let heard = (&self.rhythm, self.chart.as_ref(), self.song, self.accent);
            let timer = &mut self.timer;
            let presented = surface.present(device, queue, |encoder, view| {
                draw(scene, timer, (device, queue), encoder, view, time, heard);
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
                    render_and_wait(&self.device, &self.queue, |encoder| {
                        draw(
                            scene,
                            timer,
                            (&self.device, &self.queue),
                            encoder,
                            &view,
                            time,
                            heard,
                        )
                    })?;
                    surfaces.keep(buffer.clone());
                    return Ok(Rendered::PixelBuffer(buffer));
                }
                Ok(None) => return Ok(Rendered::Nothing),
                Err(e) => {
                    warn!("Falling back to reading frames back: {e:#}");
                    let (width, height) = surfaces.size();
                    self.output = Output::Readback(Target::new(&self.device, width, height));
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
                    render_and_wait(&self.device, &self.queue, |encoder| {
                        draw(
                            scene,
                            timer,
                            (&self.device, &self.queue),
                            encoder,
                            &view,
                            time,
                            heard,
                        )
                    })?;
                    targets.keep(texture.clone());
                    return Ok(Rendered::Shared(texture));
                }
                Ok(None) => return Ok(Rendered::Nothing),
                Err(e) => {
                    warn!("Falling back to reading frames back: {e:#}");
                    let (width, height) = targets.size();
                    self.output = Output::Readback(Target::new(&self.device, width, height));
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
                    let frame = targets.render(device, queue, next, |encoder, view| {
                        draw(scene, timer, (device, queue), encoder, view, time, heard);
                    })?;
                    let Some(frame) = frame else {
                        return Ok(Rendered::Nothing);
                    };
                    targets.keep(&frame);
                    return Ok(Rendered::Shared(frame));
                }
                Ok(None) => return Ok(Rendered::Nothing),
                Err(e) => {
                    warn!("Falling back to reading frames back: {e:#}");
                    let (width, height) = targets.size();
                    self.output = Output::Readback(Target::new(&self.device, width, height));
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
        let frame = target.render(device, queue, |encoder, view| {
            draw(scene, timer, (device, queue), encoder, view, time, heard);
        })?;
        Ok(frame.map_or(Rendered::Nothing, Rendered::Mapped))
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

/// A frame of the scene, timed by `timer`.
fn draw(
    scene: &mut Scene,
    timer: &mut GpuTimer,
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    time: f64,
    (rhythm, chart, song, accent): Heard,
) {
    let timestamps = timer.begin();
    scene.render(
        device,
        queue,
        encoder,
        view,
        &FrameInput {
            time,
            rhythm,
            chart,
            song,
            accent,
        },
        timestamps,
    );
    timer.end(encoder);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in scene loads and draws a frame that is not black.
    #[test]
    fn default_scene_renders_a_frame() {
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

    /// Growth with a budget of 1 ms a frame: the resolution goes down to its floor, then the
    /// quality, and the frame times are logged once per interval (1 s here, 10 s in the app).
    #[test]
    fn a_heavy_scene_lowers_its_level_and_logs_its_frame_times() {
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
        let full = pipeline.scene.internal_size();
        pipeline.set_target_fps(1000);
        pipeline.report = Report::new(1.0);
        let floor = crate::adaptive::Level {
            factor: crate::adaptive::MIN_FACTOR,
            quality: 0.0,
        };
        let started = Instant::now();
        let mut levels = Vec::new();
        while started.elapsed().as_secs_f64() < 15.0 && pipeline.controller.level() != floor {
            pipeline.render(started.elapsed().as_secs_f64()).unwrap();
            let level = pipeline.controller.level();
            if levels.last() != Some(&level) {
                eprintln!("{:.2} s: {level:?}", started.elapsed().as_secs_f64());
                levels.push(level);
            }
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
        // One more second for a line at the lowest level.
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
            .filter(|l| l.starts_with("Visualizer Growth:") || l.contains("frame times come"))
            .cloned()
            .collect();
        for line in &lines {
            eprintln!("{line}");
        }
        assert_eq!(pipeline.controller.level(), floor, "{levels:?}");
        let size = pipeline.scene.internal_size();
        assert!(
            size.0 <= full.0 / 2 + 1 && size.1 <= full.1 / 2 + 1,
            "{full:?} {size:?}"
        );
        assert!(lines.iter().any(|l| l.starts_with("Visualizer Growth:")));
    }
}
