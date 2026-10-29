//! Reading frames back without stalling the render thread or copying them on the CPU
//! (the fallback where frames cannot stay on the GPU).
//!
//! One render target and a few readback buffers. Each frame is copied into a free buffer and
//! mapped asynchronously; once the mapping is done the mapped buffer itself becomes the frame, so
//! the platform side reads the pixels straight out of it. Dropping the frame unmaps the buffer
//! and returns it. With no free buffer (readers hold them all) the frame is skipped.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};

use anyhow::{Result, anyhow};

use crate::pipeline::FORMAT;

/// Readback buffers per target: one being written, one being mapped, one held by the screen.
const BUFFERS: usize = 3;

const PENDING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

type Pool = Arc<Mutex<Vec<wgpu::Buffer>>>;

/// A frame in a mapped readback buffer: `height` rows of `stride` bytes, the first `width * 4`
/// of each in the target's format.
pub struct MappedFrame {
    /// `None` only while dropping.
    view: Option<wgpu::BufferView>,
    buffer: wgpu::Buffer,
    stride: usize,
    /// The target's free buffers; gone once the target was replaced (for example by a resize).
    home: Weak<Mutex<Vec<wgpu::Buffer>>>,
}

// SAFETY: the view is a read-only mapping that stays mapped until `drop`, which is the only place
// that unmaps; wgpu's `Buffer` itself is `Send + Sync`.
unsafe impl Send for MappedFrame {}
// SAFETY: as above; shared access only reads.
unsafe impl Sync for MappedFrame {}

impl MappedFrame {
    pub fn bytes(&self) -> &[u8] {
        self.view.as_deref().unwrap_or_default()
    }

    /// Bytes from one row to the next.
    pub fn stride(&self) -> usize {
        self.stride
    }
}

impl Drop for MappedFrame {
    fn drop(&mut self) {
        // The view must go before the buffer can be unmapped.
        self.view = None;
        self.buffer.unmap();
        if let Some(home) = self.home.upgrade() {
            home.lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(self.buffer.clone());
        }
    }
}

struct InFlight {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

pub struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    /// Bytes per row in the readback buffers, padded to `COPY_BYTES_PER_ROW_ALIGNMENT`. Equal to
    /// `width * 4` when the width is a multiple of 64.
    padded_row: u32,
    free: Pool,
    /// Copies that were submitted and are being mapped, oldest first.
    in_flight: VecDeque<InFlight>,
}

impl Target {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
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
        let buffers = (0..BUFFERS)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("visualizer readback"),
                    size: u64::from(padded_row) * u64::from(height),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })
            })
            .collect();
        Self {
            texture,
            view,
            width,
            height,
            padded_row,
            free: Arc::new(Mutex::new(buffers)),
            in_flight: VecDeque::new(),
        }
    }

    /// Render a frame (unless every buffer is taken) and return the newest frame whose mapping
    /// has finished since the last call, if any. Never waits for the GPU.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        draw: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView),
    ) -> Result<Option<MappedFrame>> {
        let buffer = self.free.lock().unwrap_or_else(|e| e.into_inner()).pop();
        if let Some(buffer) = buffer {
            let mut encoder = device.create_command_encoder(&Default::default());
            draw(&mut encoder, &self.view);
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(self.padded_row),
                        rows_per_image: Some(self.height),
                    },
                },
                wgpu::Extent3d {
                    width: self.width,
                    height: self.height,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit(Some(encoder.finish()));
            let state = Arc::new(AtomicU8::new(PENDING));
            let done = Arc::clone(&state);
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    done.store(
                        if result.is_ok() { MAPPED } else { FAILED },
                        Ordering::Release,
                    );
                });
            self.in_flight.push_back(InFlight { buffer, state });
        }

        // Runs the callbacks of finished mappings without blocking.
        device
            .poll(wgpu::PollType::Poll)
            .map_err(|e| anyhow!("GPU poll failed: {e:?}"))?;

        let mut newest = None;
        while let Some(front) = self.in_flight.front() {
            match front.state.load(Ordering::Acquire) {
                PENDING => break,
                state => {
                    let InFlight { buffer, .. } = self.in_flight.pop_front().expect("front exists");
                    if state == FAILED {
                        self.free
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(buffer);
                        return Err(anyhow!("Failed to map a readback buffer"));
                    }
                    let view = buffer
                        .slice(..)
                        .get_mapped_range()
                        .map_err(|e| anyhow!("Failed to read a mapped buffer: {e}"))?;
                    // An older frame that finished in the same call is dropped, which returns
                    // its buffer.
                    newest = Some(MappedFrame {
                        view: Some(view),
                        buffer,
                        stride: self.padded_row as usize,
                        home: Arc::downgrade(&self.free),
                    });
                }
            }
        }
        Ok(newest)
    }
}
