//! The engine's C API on Windows, looked up in `flutter_windows.dll` at run time.
//!
//! The declarations and layouts mirror Flutter 3.47.4's
//! `shell/platform/common/public/flutter_texture_registrar.h`, `.../flutter_plugin_registrar.h`
//! and `shell/platform/windows/public/flutter_windows.h`.

use std::ffi::{CStr, c_int, c_void};

use log::error;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::core::{PCSTR, w};

use crate::{Result, TextureError};

/// `FlutterDesktopPluginRegistrarRef`.
pub(super) type PluginRegistrar = *mut c_void;
/// `FlutterDesktopTextureRegistrarRef`.
pub(super) type TextureRegistrar = *mut c_void;

/// `FlutterDesktopTextureType`.
pub(super) const PIXEL_BUFFER_TEXTURE: c_int = 0;
pub(super) const GPU_SURFACE_TEXTURE: c_int = 1;
/// `FlutterDesktopGpuSurfaceType::kFlutterDesktopGpuSurfaceTypeDxgiSharedHandle`.
pub(super) const GPU_SURFACE_DXGI_SHARED_HANDLE: c_int = 1;
/// `FlutterDesktopPixelFormat::kFlutterDesktopPixelFormatRGBA8888`.
pub(super) const PIXEL_FORMAT_RGBA8888: c_int = 1;

pub(super) type ReleaseCallback = Option<unsafe extern "C" fn(release_context: *mut c_void)>;

/// `FlutterDesktopPixelBuffer`: tightly packed RGBA rows (there is no stride).
#[repr(C)]
pub(super) struct PixelBuffer {
    pub buffer: *const u8,
    pub width: usize,
    pub height: usize,
    pub release_callback: ReleaseCallback,
    pub release_context: *mut c_void,
}

/// `FlutterDesktopGpuSurfaceDescriptor`.
#[repr(C)]
pub(super) struct GpuSurfaceDescriptor {
    pub struct_size: usize,
    pub handle: *mut c_void,
    pub width: usize,
    pub height: usize,
    pub visible_width: usize,
    pub visible_height: usize,
    pub format: c_int,
    pub release_callback: ReleaseCallback,
    pub release_context: *mut c_void,
}

/// `FlutterDesktopPixelBufferTextureCallback`.
pub(super) type PixelBufferCallback =
    Option<unsafe extern "C" fn(usize, usize, *mut c_void) -> *const PixelBuffer>;
/// `FlutterDesktopGpuSurfaceTextureCallback`.
pub(super) type GpuSurfaceCallback =
    Option<unsafe extern "C" fn(usize, usize, *mut c_void) -> *const GpuSurfaceDescriptor>;

/// `FlutterDesktopPixelBufferTextureConfig`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct PixelBufferConfig {
    pub callback: PixelBufferCallback,
    pub user_data: *mut c_void,
}

/// `FlutterDesktopGpuSurfaceTextureConfig`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct GpuSurfaceConfig {
    pub struct_size: usize,
    pub surface_type: c_int,
    pub callback: GpuSurfaceCallback,
    pub user_data: *mut c_void,
}

/// The anonymous union in `FlutterDesktopTextureInfo`.
#[repr(C)]
pub(super) union TextureConfig {
    pub pixel_buffer: PixelBufferConfig,
    pub gpu_surface: GpuSurfaceConfig,
}

/// `FlutterDesktopTextureInfo`.
#[repr(C)]
pub(super) struct TextureInfo {
    pub texture_type: c_int,
    pub config: TextureConfig,
}

/// The functions of `flutter_windows.dll` the texture layer calls.
pub(super) struct Api {
    /// `FlutterDesktopRegistrarGetTextureRegistrar`.
    pub get_texture_registrar: unsafe extern "C" fn(PluginRegistrar) -> TextureRegistrar,
    /// `FlutterDesktopTextureRegistrarRegisterExternalTexture`: the id, or -1 (no ANGLE, or an
    /// invalid texture).
    pub register: unsafe extern "C" fn(TextureRegistrar, *const TextureInfo) -> i64,
    /// `FlutterDesktopTextureRegistrarUnregisterExternalTexture`: calls back on the raster
    /// thread once the engine let go of the texture, or at once if it cannot post to it.
    pub unregister: unsafe extern "C" fn(
        TextureRegistrar,
        i64,
        Option<unsafe extern "C" fn(*mut c_void)>,
        *mut c_void,
    ),
    /// `FlutterDesktopTextureRegistrarMarkExternalTextureFrameAvailable`.
    pub mark_frame_available: unsafe extern "C" fn(TextureRegistrar, i64) -> bool,
    /// `FlutterDesktopPluginRegistrarGetGraphicsAdapter`: the `IDXGIAdapter*` (a new reference)
    /// Flutter renders on.
    pub get_graphics_adapter: unsafe extern "C" fn(PluginRegistrar, *mut *mut c_void) -> bool,
    /// `FlutterDesktopPluginRegistrarSetDestructionHandler`: adds a handler, called when the
    /// engine is destroyed.
    pub set_destruction_handler:
        unsafe extern "C" fn(PluginRegistrar, Option<unsafe extern "C" fn(PluginRegistrar)>),
}

impl Api {
    /// Look the functions up in the `flutter_windows.dll` the process has loaded. Without it
    /// (the hub in a server) nothing can be attached.
    pub(super) fn load() -> Result<Self> {
        // SAFETY: a plain lookup of a module by name; the module is the engine, which stays
        // loaded for the life of the process once the app has started it.
        let module = unsafe { GetModuleHandleW(w!("flutter_windows.dll")) }.map_err(|e| {
            error!("flutter_windows.dll is not loaded: {e}");
            TextureError::Unsupported
        })?;
        Ok(Self {
            get_texture_registrar: lookup(module, c"FlutterDesktopRegistrarGetTextureRegistrar")?,
            register: lookup(
                module,
                c"FlutterDesktopTextureRegistrarRegisterExternalTexture",
            )?,
            unregister: lookup(
                module,
                c"FlutterDesktopTextureRegistrarUnregisterExternalTexture",
            )?,
            mark_frame_available: lookup(
                module,
                c"FlutterDesktopTextureRegistrarMarkExternalTextureFrameAvailable",
            )?,
            get_graphics_adapter: lookup(
                module,
                c"FlutterDesktopPluginRegistrarGetGraphicsAdapter",
            )?,
            set_destruction_handler: lookup(
                module,
                c"FlutterDesktopPluginRegistrarSetDestructionHandler",
            )?,
        })
    }
}

/// The function `name` of `module`, as the function pointer type `F`.
///
/// `F` must be the signature the engine declares for `name` (see the header of this module).
fn lookup<F: Copy>(module: HMODULE, name: &'static CStr) -> Result<F> {
    const { assert!(size_of::<F>() == size_of::<usize>()) };
    // SAFETY: `name` is a NUL-terminated string.
    let Some(address) = (unsafe { GetProcAddress(module, PCSTR(name.as_ptr().cast())) }) else {
        error!("flutter_windows.dll has no {}", name.to_string_lossy());
        return Err(TextureError::Unsupported);
    };
    // SAFETY: a function pointer of the size of `F`, which the caller says is the function's
    // signature.
    Ok(unsafe { std::mem::transmute_copy::<_, F>(&address) })
}
