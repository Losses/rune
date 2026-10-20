//! The GPU side: a headless device, the lactea engine and where frames go: on macOS straight into
//! IOSurface-backed pixel buffers ([`crate::surface`]), elsewhere (or if that fails) an offscreen
//! target that is read back into mapped buffers ([`crate::readback`]).

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use lactea::{Engine, EngineConfig, FrameClock, Transition};
use log::warn;

use crate::readback::{MappedFrame, Target};
#[cfg(target_os = "macos")]
use crate::surface::{PixelBuffer, SurfaceTargets};
use crate::textures::{PackTextures, TextureSource};

/// `kCVPixelFormatType_32BGRA` on macOS. Windows' `FlutterDesktopPixelBuffer` and Linux's
/// `FlPixelBufferTexture` take RGBA. The engine only compensates for `*Srgb` targets, so with a
/// linear format the bytes are exactly what the preset drew.
#[cfg(target_os = "macos")]
pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
#[cfg(not(target_os = "macos"))]
pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub struct Pipeline {
    device: wgpu::Device,
    queue: wgpu::Queue,
    engine: Engine,
    output: Output,
    readback: bool,
    pub adapter_name: String,
}

/// What a rendered frame became.
pub enum Rendered {
    /// A read-back frame (on the readback path a frame comes out a call or two after it was
    /// drawn).
    Mapped(MappedFrame),
    #[cfg(target_os = "macos")]
    PixelBuffer(PixelBuffer),
    /// Nothing new: every buffer is still in use, or the GPU has not finished one yet.
    Nothing,
}

/// How frames leave the GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Readback,
    IoSurface,
}

enum Output {
    Readback(Target),
    #[cfg(target_os = "macos")]
    Surfaces(SurfaceTargets),
}

impl Output {
    /// Pixel buffers on macOS unless `readback`, else (or if they fail) the readback target.
    fn new(device: &wgpu::Device, width: u32, height: u32, readback: bool) -> Self {
        #[cfg(target_os = "macos")]
        if !readback {
            match SurfaceTargets::new(device, width, height) {
                Ok(surfaces) => return Self::Surfaces(surfaces),
                Err(e) => warn!("Falling back to reading frames back: {e:#}"),
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = readback;
        Self::Readback(Target::new(device, width, height))
    }

    fn size(&self) -> (u32, u32) {
        match self {
            Self::Readback(target) => (target.width, target.height),
            #[cfg(target_os = "macos")]
            Self::Surfaces(surfaces) => surfaces.size(),
        }
    }
}

impl Pipeline {
    pub fn new(
        width: u32,
        height: u32,
        fps: u32,
        sample_rate: u32,
        readback: bool,
        on_gpu_error: Arc<dyn Fn(String) + Send + Sync>,
    ) -> Result<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| anyhow!("No GPU adapter: {e}"))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .context("Failed to create the GPU device")?;
        // Without a handler wgpu panics on validation errors.
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| on_gpu_error(e.to_string())));

        let (width, height) = (width.max(1), height.max(1));
        let mut engine = Engine::new(
            &device,
            &queue,
            EngineConfig {
                width,
                height,
                output_format: FORMAT,
                sample_rate,
                fps,
                // Pick projectM or MilkDrop 3 semantics per preset (and allow `.milk2`).
                auto_compat: true,
                ..Default::default()
            },
        )
        .map_err(|e| anyhow!("Failed to create the engine: {e}"))?;
        engine
            .load_idle_preset()
            .map_err(|e| anyhow!("Failed to load the idle preset: {e}"))?;
        let output = Output::new(&device, width, height, readback);
        Ok(Self {
            adapter_name: adapter.get_info().name,
            device,
            queue,
            engine,
            output,
            readback,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.output.size()
    }

    pub fn transport(&self) -> Transport {
        match self.output {
            Output::Readback(_) => Transport::Readback,
            #[cfg(target_os = "macos")]
            Output::Surfaces(_) => Transport::IoSurface,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let (width, height) = (width.max(1), height.max(1));
        self.engine
            .resize(width, height)
            .map_err(|e| anyhow!("Failed to resize the engine: {e}"))?;
        self.output = Output::new(&self.device, width, height, self.readback);
        Ok(())
    }

    pub fn load_preset(&mut self, bytes: &[u8], name: &str, transition: Transition) -> Result<()> {
        self.engine
            .load_preset(bytes, name, transition)
            .map(|_| ())
            .map_err(|e| anyhow!("Failed to load preset {name}: {e}"))
    }

    pub fn set_textures(&mut self, source: &TextureSource) -> Result<()> {
        self.engine
            .set_texture_provider(Box::new(PackTextures::new(source)?));
        Ok(())
    }

    pub fn push_pcm(&mut self, interleaved: &[f32], channels: usize) {
        self.engine.push_pcm_f32(interleaved, channels);
    }

    /// Render one frame: into the next pixel buffer, or into the target that is read back.
    pub fn render(&mut self, clock: FrameClock) -> Result<Rendered> {
        #[cfg(target_os = "macos")]
        if let Output::Surfaces(surfaces) = &mut self.output {
            match surfaces.next(&self.device) {
                Ok(Some((buffer, view))) => {
                    let mut encoder = self.device.create_command_encoder(&Default::default());
                    self.engine.render(&mut encoder, &view, clock);
                    let submission = self.queue.submit(Some(encoder.finish()));
                    // Hand the buffer out only once the GPU has finished drawing into it.
                    self.device
                        .poll(wgpu::PollType::Wait {
                            submission_index: Some(submission),
                            timeout: None,
                        })
                        .map_err(|e| anyhow!("GPU poll failed: {e:?}"))?;
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
        let Output::Readback(target) = &mut self.output else {
            unreachable!("only macOS has other outputs")
        };
        let engine = &mut self.engine;
        let frame = target.render(&self.device, &self.queue, |encoder, view| {
            engine.render(encoder, view, clock);
        })?;
        Ok(frame.map_or(Rendered::Nothing, Rendered::Mapped))
    }
}
