//! Windows: render straight into DirectX 12 textures that are shared through NT handles, which
//! the app hands to Flutter without copying a pixel.
//!
//! Each target is a committed texture on a shared heap with a handle from `CreateSharedHandle`,
//! imported into wgpu as the render target. Flutter opens the handle on its own Direct3D 11 device
//! (through ANGLE), so the visualizer must render on the adapter Flutter uses: see
//! [`adapter_with_luid`]. A texture is drawn into again only once no frame holds it any more, so a
//! frame that Flutter (or the latest-frame slot) still has, or that the GPU is still drawing
//! (`crate::in_flight`), is never drawn over. A frame is handed to Flutter only once the GPU has
//! finished it.
//!
//! The textures allow simultaneous access: their state decays to `COMMON` when a command list
//! finishes, which is what Direct3D 11 expects, and promotes to a render target on the next draw
//! without a barrier. wgpu imports them as uninitialized, which on DirectX 12 is the `COMMON`
//! state they are created in.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use wgpu::hal;
use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE};
use windows::Win32::Graphics::Direct3D12::{
    D3D12_HEAP_FLAG_SHARED, D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_DEFAULT, D3D12_RESOURCE_DESC,
    D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
    D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS, D3D12_RESOURCE_STATE_COMMON,
    D3D12_TEXTURE_LAYOUT_UNKNOWN, ID3D12Resource,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::core::PCWSTR;

/// Published frames that stay referenced here after a newer one replaced them, so that a texture
/// is drawn into again only a few frames after Flutter let go of it: its compositor may still
/// sample the last frame it was given while the next one is being drawn.
const KEEP_RECENT: usize = 3;
/// Upper bound on the textures of one size: the recent ones (the latest among them), the ones the
/// GPU is drawing (`crate::in_flight`), finished ones not handed out yet, and Flutter's.
const MAX_TEXTURES: usize = 8;

/// The DirectX 12 adapter whose DXGI LUID is `luid` (as [`crate::adapter_luid`] combines it), if
/// there is one. Frames can only be shared with a device on the same adapter.
pub fn adapter_with_luid(instance: &wgpu::Instance, luid: u64) -> Option<wgpu::Adapter> {
    pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12))
        .into_iter()
        .find(|adapter| {
            // SAFETY: only reads the adapter's description; the guard is dropped before wgpu
            // uses the adapter again.
            let Some(hal_adapter) = (unsafe { adapter.as_hal::<hal::api::Dx12>() }) else {
                return false;
            };
            // SAFETY: a plain query on a valid adapter.
            unsafe { hal_adapter.raw_adapter().GetDesc1() }.is_ok_and(|desc| {
                let found = desc.AdapterLuid;
                crate::adapter_luid(found.LowPart, found.HighPart) == luid
            })
        })
}

/// A shared texture's NT handle, closed when the last reference goes.
struct Handle(HANDLE);

// SAFETY: an NT handle is a process-wide value that any thread may use and close.
unsafe impl Send for Handle {}
// SAFETY: as above; shared access only reads the value.
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from `CreateSharedHandle` and is closed only here. Flutter
        // opens its own reference to the texture, so closing this one takes nothing from it.
        if let Err(e) = unsafe { CloseHandle(self.0) } {
            log::warn!("Failed to close a shared texture handle: {e}");
        }
    }
}

struct Inner {
    handle: Handle,
    view: wgpu::TextureView,
}

/// A render target that is shared through an NT handle. Clones refer to the same texture; it is
/// free to be drawn into again once only the pool refers to it.
#[derive(Clone)]
pub struct SharedTexture(Arc<Inner>);

impl SharedTexture {
    /// The NT handle of the texture (DXGI_FORMAT_R8G8B8A8_UNORM), valid while this texture is
    /// held. The receiver opens it, for example with `ID3D11Device1::OpenSharedResource1`, and
    /// must not close it.
    pub fn handle(&self) -> *mut c_void {
        self.0.handle.0.0
    }

    /// Whether nothing but the pool refers to the texture. Only the pool hands out new
    /// references, so a free texture stays free until it does.
    fn is_free(&self) -> bool {
        Arc::strong_count(&self.0) == 1
    }
}

/// The render targets of one size.
pub struct SharedTargets {
    width: u32,
    height: u32,
    textures: Vec<SharedTexture>,
    recent: VecDeque<SharedTexture>,
}

impl SharedTargets {
    /// Creates the first texture at once, so that a device that cannot share textures fails here,
    /// before anything is drawn.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Result<Self> {
        let mut targets = Self {
            width,
            height,
            textures: Vec::new(),
            recent: VecDeque::new(),
        };
        let first = targets.create(device)?;
        targets.textures.push(first);
        Ok(targets)
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// A free texture and the view that renders into it, or `None` while every texture is still
    /// in use and the pool is full (the frame is skipped).
    pub fn next(
        &mut self,
        device: &wgpu::Device,
    ) -> Result<Option<(SharedTexture, wgpu::TextureView)>> {
        let texture = match self.textures.iter().find(|texture| texture.is_free()) {
            Some(texture) => texture.clone(),
            None if self.textures.len() < MAX_TEXTURES => {
                let texture = self.create(device)?;
                self.textures.push(texture.clone());
                texture
            }
            None => return Ok(None),
        };
        let view = texture.0.view.clone();
        Ok(Some((texture, view)))
    }

    /// Remember a frame that was just published (see [`KEEP_RECENT`]).
    pub fn keep(&mut self, texture: SharedTexture) {
        self.recent.push_back(texture);
        while self.recent.len() > KEEP_RECENT {
            self.recent.pop_front();
        }
    }

    fn create(&self, device: &wgpu::Device) -> Result<SharedTexture> {
        // SAFETY: the guard is dropped before wgpu is used again.
        let (resource, handle) = {
            let hal_device = unsafe { device.as_hal::<hal::api::Dx12>() }
                .context("The GPU device does not run on DirectX 12")?;
            let raw = hal_device.raw_device();
            let heap = D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                ..Default::default()
            };
            let desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Alignment: 0,
                Width: u64::from(self.width),
                Height: self.height,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET
                    | D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
            };
            let mut resource: Option<ID3D12Resource> = None;
            // SAFETY: valid descriptors and an out pointer to a local.
            unsafe {
                raw.CreateCommittedResource(
                    &heap,
                    D3D12_HEAP_FLAG_SHARED,
                    &desc,
                    D3D12_RESOURCE_STATE_COMMON,
                    None,
                    &mut resource,
                )
            }
            .context("Failed to create a shared texture")?;
            let resource = resource.context("The shared texture is null")?;
            // SAFETY: a resource of this device on a shared heap; `GENERIC_ALL` is the only
            // access the call accepts, and the handle has no name.
            let handle =
                unsafe { raw.CreateSharedHandle(&resource, None, GENERIC_ALL.0, PCWSTR::null()) }
                    .context("Failed to share the texture")?;
            (resource, Handle(handle))
        };
        if handle.0.is_invalid() {
            bail!("The shared texture handle is invalid");
        }
        let size = wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        };
        // SAFETY: `resource` is a 2D R8G8B8A8_UNORM texture of `size` with one layer, one mip
        // level and one sample, created on this device in the `COMMON` state (which is what
        // `UNINITIALIZED` maps to), matching the descriptor below; wgpu owns this reference to
        // it from here on.
        let texture = unsafe {
            let hal_texture = hal::dx12::Device::texture_from_raw(
                resource,
                crate::pipeline::FORMAT,
                wgpu::TextureDimension::D2,
                size,
                1,
                1,
            );
            device.create_texture_from_hal::<hal::api::Dx12>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("visualizer shared target"),
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
        Ok(SharedTexture(Arc::new(Inner {
            handle,
            view: texture.create_view(&Default::default()),
        })))
    }
}
