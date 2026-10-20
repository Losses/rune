//! The GPU side: a headless device, the lactea engine and where frames go: on macOS straight into
//! IOSurface-backed pixel buffers ([`crate::surface`]), elsewhere (or if that fails) an offscreen
//! target that is read back.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use lactea::{Engine, EngineConfig, FrameClock, Transition};
use log::warn;

#[cfg(target_os = "macos")]
use crate::surface::{PixelBuffer, SurfaceTargets};
use crate::textures::{PackTextures, TextureSource};

/// `kCVPixelFormatType_32BGRA`. The engine only compensates for `*Srgb` targets, so with this
/// linear format the bytes are exactly what the preset drew.
pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

pub struct Pipeline {
    device: wgpu::Device,
    queue: wgpu::Queue,
    engine: Engine,
    output: Output,
    pub adapter_name: String,
}

/// What a rendered frame became.
pub enum Rendered {
    /// Tightly packed BGRA in the `pixels` passed to [`Pipeline::render`].
    Pixels,
    #[cfg(target_os = "macos")]
    PixelBuffer(PixelBuffer),
    /// Every pixel buffer is still in use; nothing was drawn.
    Skipped,
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
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        #[cfg(target_os = "macos")]
        match SurfaceTargets::new(device, width, height) {
            Ok(surfaces) => return Self::Surfaces(surfaces),
            Err(e) => warn!("Falling back to reading frames back: {e:#}"),
        }
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

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    width: u32,
    height: u32,
    /// Bytes per row in the readback buffer, padded to `COPY_BYTES_PER_ROW_ALIGNMENT`.
    padded_row: u32,
}

impl Target {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("visualizer target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let padded_row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("visualizer readback"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            texture,
            view,
            readback,
            width,
            height,
            padded_row,
        }
    }
}

impl Pipeline {
    pub fn new(
        width: u32,
        height: u32,
        fps: u32,
        sample_rate: u32,
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
        let output = Output::new(&device, width, height);
        Ok(Self {
            adapter_name: adapter.get_info().name,
            device,
            queue,
            engine,
            output,
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
        self.output = Output::new(&self.device, width, height);
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

    /// Render one frame: into the next pixel buffer, or read back into `pixels`.
    pub fn render(&mut self, clock: FrameClock, pixels: &mut Vec<u8>) -> Result<Rendered> {
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
                Ok(None) => return Ok(Rendered::Skipped),
                Err(e) => {
                    warn!("Falling back to reading frames back: {e:#}");
                    let (width, height) = surfaces.size();
                    self.output = Output::Readback(Target::new(&self.device, width, height));
                }
            }
        }
        let Output::Readback(target) = &self.output else {
            unreachable!("only macOS has other outputs")
        };
        read_back(
            &self.device,
            &self.queue,
            &mut self.engine,
            target,
            clock,
            pixels,
        )?;
        Ok(Rendered::Pixels)
    }
}

/// Render into the offscreen target and write the frame, tightly packed BGRA, into `out`.
fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    engine: &mut Engine,
    target: &Target,
    clock: FrameClock,
    out: &mut Vec<u8>,
) -> Result<()> {
    let mut encoder = device.create_command_encoder(&Default::default());
    engine.render(&mut encoder, &target.view, clock);
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &target.readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(target.padded_row),
                rows_per_image: Some(target.height),
            },
        },
        wgpu::Extent3d {
            width: target.width,
            height: target.height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = target.readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| anyhow!("GPU poll failed: {e:?}"))?;
    rx.recv()
        .context("GPU readback callback was dropped")?
        .context("Failed to map the readback buffer")?;
    {
        let mapped = slice
            .get_mapped_range()
            .map_err(|e| anyhow!("Failed to read the mapped buffer: {e}"))?;
        let row = target.width as usize * 4;
        let padded_row = target.padded_row as usize;
        out.clear();
        out.reserve(row * target.height as usize);
        for y in 0..target.height as usize {
            out.extend_from_slice(&mapped[y * padded_row..y * padded_row + row]);
        }
    }
    target.readback.unmap();
    Ok(())
}
