//! Apple (macOS, iOS): render straight into IOSurface-backed pixel buffers, which the app hands to Flutter
//! without copying a pixel.
//!
//! A CoreVideo pool hands out BGRA buffers. Each buffer's IOSurface is wrapped in a Metal texture
//! and imported into wgpu as the render target. The pool only reuses a buffer once nobody holds
//! it any more, so a frame that Flutter (or the latest-frame slot) still has, or that the GPU is
//! still drawing (`crate::in_flight`), is never drawn over.

use std::collections::{HashMap, VecDeque};
use std::ptr::NonNull;

use anyhow::{Context, Result, bail};
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetIOSurface, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferPool, CVPixelBufferUnlockBaseAddress, kCVPixelBufferHeightKey,
    kCVPixelBufferIOSurfacePropertiesKey, kCVPixelBufferMetalCompatibilityKey,
    kCVPixelBufferPixelFormatTypeKey, kCVPixelBufferPoolAllocationThresholdKey,
    kCVPixelBufferPoolMinimumBufferCountKey, kCVPixelBufferWidthKey, kCVPixelFormatType_32BGRA,
    kCVReturnSuccess, kCVReturnWouldExceedAllocationThreshold,
};
use objc2_io_surface::IOSurfaceID;
use objc2_metal::{
    MTLDevice, MTLPixelFormat, MTLTextureDescriptor, MTLTextureType, MTLTextureUsage,
};
use wgpu::hal;

use crate::in_flight::MAX_IN_FLIGHT;

/// Published frames that stay referenced here after a newer one replaced them, so that a buffer
/// goes back to the pool only a few frames after Flutter let go of it: its compositor may still
/// sample the last frame it was given while the next one is being drawn.
const KEEP_RECENT: usize = 3;
/// Upper bound on the buffers of a pool: the recent ones (the latest among them), the ones the GPU
/// is drawing ([`MAX_IN_FLIGHT`]), finished ones not handed out yet, and Flutter's.
const MAX_BUFFERS: i32 = 8;
/// Imported textures, one per IOSurface of the pool; more than this means the pool replaced its
/// buffers (for example after aging them out), so the cache starts over.
const MAX_CACHED_TEXTURES: usize = 16;

/// A CoreVideo pixel buffer backed by an IOSurface. CoreVideo buffers are reference counted
/// thread-safely and this type never touches the pixels except under `CVPixelBufferLockBaseAddress`,
/// so it can move to the threads that read the latest frame.
#[derive(Clone)]
pub struct PixelBuffer(CFRetained<CVPixelBuffer>);

// SAFETY: see the type's documentation.
unsafe impl Send for PixelBuffer {}
// SAFETY: see the type's documentation.
unsafe impl Sync for PixelBuffer {}

impl PixelBuffer {
    /// The buffer, with one more reference.
    pub fn retained(&self) -> CFRetained<CVPixelBuffer> {
        self.0.clone()
    }

    /// The pixels as tightly packed BGRA rows. Copies; meant for tests and diagnostics.
    pub fn read_bgra(&self, width: u32, height: u32) -> Result<Vec<u8>> {
        let buffer = &*self.0;
        // SAFETY: a read-only lock on a valid buffer, released below.
        if unsafe { CVPixelBufferLockBaseAddress(buffer, CVPixelBufferLockFlags::ReadOnly) }
            != kCVReturnSuccess
        {
            bail!("Failed to lock the pixel buffer");
        }
        let base = CVPixelBufferGetBaseAddress(buffer).cast::<u8>();
        let stride = CVPixelBufferGetBytesPerRow(buffer);
        let row = width as usize * 4;
        let mut out = Vec::with_capacity(row * height as usize);
        if !base.is_null() && stride >= row {
            for y in 0..height as usize {
                // SAFETY: the buffer is locked and has `height` rows of `stride` bytes.
                out.extend_from_slice(unsafe {
                    std::slice::from_raw_parts(base.add(y * stride), row)
                });
            }
        }
        // SAFETY: matches the lock above.
        unsafe { CVPixelBufferUnlockBaseAddress(buffer, CVPixelBufferLockFlags::ReadOnly) };
        if out.len() != row * height as usize {
            bail!("The pixel buffer has no readable pixels");
        }
        Ok(out)
    }
}

fn opaque(dictionary: &CFDictionary<CFString, CFType>) -> &CFDictionary {
    dictionary.as_ref()
}

/// The render targets of one size.
pub struct SurfaceTargets {
    pool: CFRetained<CVPixelBufferPool>,
    aux: CFRetained<CFDictionary<CFString, CFType>>,
    width: u32,
    height: u32,
    textures: HashMap<IOSurfaceID, wgpu::TextureView>,
    recent: VecDeque<PixelBuffer>,
}

impl SurfaceTargets {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Result<Self> {
        // Fail early, before anything is drawn, if this is not the Metal backend.
        // SAFETY: only checks which backend the device runs on.
        if unsafe { device.as_hal::<hal::api::Metal>() }.is_none() {
            bail!("The GPU device does not run on Metal");
        }
        let number = |value: i32| CFNumber::new_i32(value);
        // SAFETY: CoreVideo's constant keys are initialized once the framework is loaded.
        let (buffer_keys, pool_keys, aux_keys) = unsafe {
            (
                [
                    kCVPixelBufferPixelFormatTypeKey,
                    kCVPixelBufferWidthKey,
                    kCVPixelBufferHeightKey,
                    kCVPixelBufferIOSurfacePropertiesKey,
                    kCVPixelBufferMetalCompatibilityKey,
                ],
                [kCVPixelBufferPoolMinimumBufferCountKey],
                [kCVPixelBufferPoolAllocationThresholdKey],
            )
        };
        let buffer_attributes = CFDictionary::<CFString, CFType>::from_slices(
            &buffer_keys,
            &[
                &number(kCVPixelFormatType_32BGRA as i32),
                &number(width as i32),
                &number(height as i32),
                &CFDictionary::<CFString, CFType>::empty(),
                objc2_core_foundation::CFBoolean::new(true),
            ],
        );
        let pool_attributes = CFDictionary::<CFString, CFType>::from_slices(
            &pool_keys,
            &[&number((KEEP_RECENT + MAX_IN_FLIGHT + 1) as i32)],
        );
        let aux = CFDictionary::<CFString, CFType>::from_slices(&aux_keys, &[&number(MAX_BUFFERS)]);
        let mut pool: *mut CVPixelBufferPool = std::ptr::null_mut();
        // SAFETY: valid dictionaries and an out pointer to a local.
        let status = unsafe {
            CVPixelBufferPool::create(
                None,
                Some(opaque(&pool_attributes)),
                Some(opaque(&buffer_attributes)),
                NonNull::from(&mut pool),
            )
        };
        if status != kCVReturnSuccess {
            bail!("Failed to create the pixel buffer pool: CVReturn {status}");
        }
        let pool = NonNull::new(pool).context("The pixel buffer pool is null")?;
        Ok(Self {
            // SAFETY: `create` returns the pool with a +1 reference, which this takes over.
            pool: unsafe { CFRetained::from_raw(pool) },
            aux,
            width,
            height,
            textures: HashMap::new(),
            recent: VecDeque::new(),
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// A free buffer and the view that renders into it, or `None` while every buffer is still
    /// in use (the frame is skipped).
    pub fn next(
        &mut self,
        device: &wgpu::Device,
    ) -> Result<Option<(PixelBuffer, wgpu::TextureView)>> {
        let mut raw: *mut CVPixelBuffer = std::ptr::null_mut();
        // SAFETY: a valid pool and dictionary, and an out pointer to a local.
        let status = unsafe {
            CVPixelBufferPool::create_pixel_buffer_with_aux_attributes(
                None,
                &self.pool,
                Some(opaque(&self.aux)),
                NonNull::from(&mut raw),
            )
        };
        if status == kCVReturnWouldExceedAllocationThreshold {
            return Ok(None);
        }
        if status != kCVReturnSuccess {
            bail!("Failed to get a pixel buffer: CVReturn {status}");
        }
        let raw = NonNull::new(raw).context("The pixel buffer is null")?;
        // SAFETY: the pool returns the buffer with a +1 reference, which this takes over.
        let buffer = PixelBuffer(unsafe { CFRetained::from_raw(raw) });
        let surface = CVPixelBufferGetIOSurface(Some(&buffer.0))
            .context("The pixel buffer has no IOSurface")?;
        let id = surface.id();
        if !self.textures.contains_key(&id) {
            if self.textures.len() >= MAX_CACHED_TEXTURES {
                self.textures.clear();
            }
            let view = self.import(device, &surface)?;
            self.textures.insert(id, view);
        }
        Ok(Some((buffer, self.textures[&id].clone())))
    }

    /// Remember a frame that was just published (see [`KEEP_RECENT`]).
    pub fn keep(&mut self, buffer: PixelBuffer) {
        self.recent.push_back(buffer);
        while self.recent.len() > KEEP_RECENT {
            self.recent.pop_front();
        }
    }

    fn import(
        &self,
        device: &wgpu::Device,
        surface: &objc2_io_surface::IOSurfaceRef,
    ) -> Result<wgpu::TextureView> {
        // SAFETY: `new` checked that this is the Metal backend; the guard is dropped before
        // wgpu is used again.
        let raw = {
            let hal_device = unsafe { device.as_hal::<hal::api::Metal>() }
                .context("The GPU device does not run on Metal")?;
            // SAFETY: a plain descriptor for a 2D texture of the pool's size and format.
            let descriptor = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    MTLPixelFormat::BGRA8Unorm,
                    self.width as usize,
                    self.height as usize,
                    false,
                )
            };
            descriptor.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
            hal_device
                .raw_device()
                .newTextureWithDescriptor_iosurface_plane(&descriptor, surface, 0)
                .context("Failed to wrap the IOSurface in a Metal texture")?
        };
        let size = wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        };
        // SAFETY: `raw` is a 2D BGRA8Unorm texture of `size` with one layer and one mip level,
        // matching the descriptor below; wgpu owns the reference from here on.
        let texture = unsafe {
            let hal_texture = hal::metal::Device::texture_from_raw(
                raw,
                crate::pipeline::FORMAT,
                MTLTextureType::Type2D,
                1,
                1,
                hal::CopyExtent {
                    width: self.width,
                    height: self.height,
                    depth: 1,
                },
                None,
            );
            device.create_texture_from_hal::<hal::api::Metal>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("visualizer IOSurface target"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: crate::pipeline::FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::TextureUses::UNINITIALIZED,
            )
        };
        Ok(texture.create_view(&Default::default()))
    }
}
