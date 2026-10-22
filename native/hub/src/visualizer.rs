//! The process-wide visualizer and its C interface.
//!
//! The native side of the app pulls finished frames through these functions, so that the pixels
//! never travel through Dart:
//! - macOS: an IOSurface-backed pixel buffer ([`rune_visualizer_copy_pixel_buffer`]), or a copy of
//!   a read-back frame ([`rune_visualizer_copy_frame`]) if pixel buffers failed;
//! - Windows: a DirectX 12 texture shared through an NT handle, held until released
//!   (`rune_visualizer_acquire_shared_frame`), once the app told which adapter Flutter renders on
//!   (`rune_visualizer_set_graphics_adapter`); else, or if sharing failed, read back;
//! - Linux: a Vulkan image whose memory and a semaphore OpenGL imports from file descriptors, held
//!   until released (`rune_visualizer_acquire_shared_frame`), once the app told which GPU its
//!   OpenGL context runs on (`rune_visualizer_set_graphics_device`); else, or if sharing failed,
//!   read back;
//! - Windows and Linux, read back: the read-back frame itself, held until released
//!   ([`rune_visualizer_acquire_frame`], [`rune_visualizer_release_frame`]).
//!
//! The functions are exported from `rinf.framework` on macOS and from the hub library
//! (`hub.dll`, `libhub.so`) elsewhere.

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use ::database::{
    actions::{cover_art::bake_cover_art_by_media_files, mixes::query_mix_media_files},
    connection::{MainDbConnection, RecommendationDbConnection},
};
use ::fsio::FsIo;
use anyhow::Result;
use log::warn;
#[cfg(target_os = "linux")]
use std::ffi::c_int;
#[cfg(target_os = "linux")]
use std::os::fd::{IntoRawFd, OwnedFd};

#[cfg(target_os = "linux")]
use log::error;
#[cfg(target_os = "linux")]
use visualizer::{DeviceUuids, SharedFrame};
use visualizer::{Frame, Visualizer};

pub static VISUALIZER: LazyLock<Visualizer> = LazyLock::new(Visualizer::new);

/// How many random covers a scene that reads covers gets besides the one of the track playing.
const RANDOM_COVERS: usize = 63;

/// What finding the covers for scenes needs.
struct CoverSource {
    runtime: tokio::runtime::Handle,
    fsio: Arc<FsIo>,
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
}

static COVER_SOURCE: Mutex<Option<Arc<CoverSource>>> = Mutex::new(None);
/// The cover file of the track playing, once its cover is baked.
static PLAYING_COVER: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Let the visualizer's scenes read covers from this library: whenever a scene that asks for them
/// loads they are found and handed over, and whenever the playing track changes while one is on
/// screen, so is its cover. Call it within the runtime.
pub fn provide_covers(
    fsio: Arc<FsIo>,
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
) {
    *COVER_SOURCE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(CoverSource {
        runtime: tokio::runtime::Handle::current(),
        fsio,
        main_db,
        recommend_db,
    }));
    VISUALIZER.set_covers_requester(refresh_covers);
}

/// The player moved to another track, whose cover file is `cover` (none if it has none): a scene
/// that reads covers gets it in cell 0, the random covers stay.
pub fn playing_cover_changed(cover: Option<&str>) {
    let cover = cover.filter(|path| !path.is_empty()).map(PathBuf::from);
    *PLAYING_COVER.lock().unwrap_or_else(|e| e.into_inner()) = cover.clone();
    if VISUALIZER.wants_covers() {
        VISUALIZER.set_playing_cover(cover);
    }
}

/// Find 63 random covers in the background and give them to the visualizer with the playing
/// track's.
fn refresh_covers() {
    // The cell of the track playing too: it may have changed while no such scene was on screen,
    // and the one the visualizer kept would cover the new atlas's.
    VISUALIZER.set_playing_cover(
        PLAYING_COVER
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
    );
    let source = COVER_SOURCE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(source) = source else {
        return;
    };
    let runtime = source.runtime.clone();
    runtime.spawn(async move {
        match random_covers(&source).await {
            Ok(random) => {
                let current = PLAYING_COVER
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                VISUALIZER.set_covers(current, random);
            }
            Err(e) => warn!("Failed to find covers for the visualizer: {e:#}"),
        }
    });
}

/// The same query as the cover wall's, for the cover files of random tracks.
async fn random_covers(source: &CoverSource) -> Result<Vec<PathBuf>> {
    let files = query_mix_media_files(
        &source.main_db,
        &source.recommend_db,
        vec![
            ("lib::random".to_string(), RANDOM_COVERS.to_string()),
            ("filter::with_cover_art".to_string(), "true".to_string()),
        ],
        0,
        RANDOM_COVERS,
    )
    .await?;
    let ids: Vec<i32> = files.iter().map(|file| file.id).collect();
    let paths = bake_cover_art_by_media_files(&source.fsio, &source.main_db, files).await?;
    Ok(ids
        .iter()
        .filter_map(|id| paths.get(id))
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect())
}

/// Id of the latest frame, or 0 if there is none (the visualizer is stopped or has not rendered
/// yet). If there is one and the pointers are not null, its size is written to them.
///
/// # Safety
///
/// `out_width` and `out_height` must each be null or valid for writing a `u32`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rune_visualizer_frame_info(
    out_width: *mut u32,
    out_height: *mut u32,
) -> u64 {
    let Some(frame) = VISUALIZER.latest_frame() else {
        return 0;
    };
    // SAFETY: the caller guarantees that the pointers are null or writable.
    unsafe {
        if !out_width.is_null() {
            *out_width = frame.width;
        }
        if !out_height.is_null() {
            *out_height = frame.height;
        }
    }
    frame.id
}

/// Copy the latest frame (BGRA, rows top to bottom) into `dst`, `dst_stride` bytes per row, if
/// its size is exactly `width` x `height`. Returns the frame id, or 0 if nothing was copied
/// (no frame, a frame in a pixel buffer, a different size, a null `dst`, a zero width, or
/// `dst_stride` below `width * 4`). Only the
/// first `width * 4` bytes of each row are written; padding is left alone.
///
/// # Safety
///
/// `dst` must be null or valid for writing `dst_stride * (height - 1) + width * 4` bytes, and
/// must not overlap memory that another thread uses during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rune_visualizer_copy_frame(
    dst: *mut u8,
    dst_stride: usize,
    width: u32,
    height: u32,
) -> u64 {
    let row = width as usize * 4;
    // A zero-sized row would make `chunks_exact` panic, and a panic must not unwind into C.
    if dst.is_null() || row == 0 || dst_stride < row {
        return 0;
    }
    let Some(frame) = VISUALIZER.latest_frame() else {
        return 0;
    };
    let Some((pixels, stride)) = frame.pixels() else {
        return 0;
    };
    let rows = height as usize;
    if frame.width != width
        || frame.height != height
        || stride < row
        || pixels.len() < stride * (rows - 1) + row
    {
        return 0;
    }
    for y in 0..rows {
        // SAFETY: the source row is in bounds (checked above); the destination row ends at
        // `dst_stride * y + row`, within the range the caller guarantees.
        unsafe {
            std::ptr::copy_nonoverlapping(
                pixels.as_ptr().add(stride * y),
                dst.add(dst_stride * y),
                row,
            );
        }
    }
    frame.id
}

/// The latest frame as a `CVPixelBufferRef` (IOSurface-backed `kCVPixelFormatType_32BGRA`) with
/// one reference that the caller owns and must release, or null if there is no frame or it was
/// read back (use [`rune_visualizer_copy_frame`] then). If `out_id` is not null, the frame id is
/// written to it, or 0 when null is returned.
///
/// # Safety
///
/// `out_id` must be null or valid for writing a `u64`.
#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rune_visualizer_copy_pixel_buffer(
    out_id: *mut u64,
) -> *mut std::ffi::c_void {
    let frame = VISUALIZER.latest_frame();
    let buffer = frame
        .as_ref()
        .and_then(|frame| frame.retained_pixel_buffer());
    if !out_id.is_null() {
        let id = match (&frame, buffer) {
            (Some(frame), Some(_)) => frame.id,
            _ => 0,
        };
        // SAFETY: the caller guarantees that the pointer is null or writable.
        unsafe { *out_id = id };
    }
    buffer.unwrap_or(std::ptr::null_mut())
}

/// A read-back frame, as filled in by [`rune_visualizer_acquire_frame`].
#[repr(C)]
pub struct RuneVisualizerFrame {
    /// `height` rows of `stride` bytes; the first `width * 4` of each are the pixels, RGBA on
    /// Windows and Linux (BGRA on macOS). Valid until the frame is released.
    pub pixels: *const u8,
    pub width: u32,
    pub height: u32,
    /// Equal to `width * 4` when the width is a multiple of 64.
    pub stride: usize,
    pub id: u64,
}

/// Hold the latest frame if it was read back, and describe it in `out`. Returns a handle for
/// [`rune_visualizer_release_frame`], or null (nothing written) if there is no such frame. While
/// held, the frame's buffer is not reused, so release it as soon as it has been shown: the
/// renderer skips frames while every buffer is held.
///
/// # Safety
///
/// `out` must be valid for writing a [`RuneVisualizerFrame`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rune_visualizer_acquire_frame(
    out: *mut RuneVisualizerFrame,
) -> *const c_void {
    if out.is_null() {
        return std::ptr::null();
    }
    let Some(frame) = VISUALIZER.latest_frame() else {
        return std::ptr::null();
    };
    let Some((pixels, stride)) = frame.pixels() else {
        return std::ptr::null();
    };
    // SAFETY: the caller guarantees that `out` is writable.
    unsafe {
        out.write(RuneVisualizerFrame {
            pixels: pixels.as_ptr(),
            width: frame.width,
            height: frame.height,
            stride,
            id: frame.id,
        });
    }
    Arc::into_raw(frame).cast()
}

/// Tell the visualizer which adapter Flutter renders on, by the parts of its DXGI LUID
/// (`DXGI_ADAPTER_DESC::AdapterLuid`); both 0 if it is unknown. Frames are shared with Flutter only
/// when the visualizer renders on that adapter, otherwise they are read back. Applies from the
/// next start of the visualizer, so call it before the app asks for one.
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub extern "C" fn rune_visualizer_set_graphics_adapter(luid_low: u32, luid_high: i32) {
    let luid = (luid_low != 0 || luid_high != 0).then_some((luid_low, luid_high));
    VISUALIZER.set_graphics_adapter(luid);
}

/// A frame in a shared texture, as filled in by `rune_visualizer_acquire_shared_frame`.
#[cfg(target_os = "windows")]
#[repr(C)]
pub struct RuneVisualizerSharedFrame {
    /// The NT handle of a `DXGI_FORMAT_R8G8B8A8_UNORM` texture of `width` x `height`, for
    /// `ID3D11Device1::OpenSharedResource1` or Flutter's
    /// `kFlutterDesktopGpuSurfaceTypeDxgiSharedHandle`. Valid until the frame is released; do not
    /// close it.
    pub handle: *mut c_void,
    pub width: u32,
    pub height: u32,
    pub id: u64,
}

/// Hold the latest frame if it is in a shared texture, and describe it in `out`. Returns a
/// handle for [`rune_visualizer_release_frame`], or null (nothing written) if there is no such
/// frame. The GPU has finished drawing it, and the visualizer does not draw into the texture again
/// while it is held, nor for a few frames after. Release it once a newer frame is shown: the
/// renderer skips frames while every texture is held.
///
/// # Safety
///
/// `out` must be null or valid for writing a [`RuneVisualizerSharedFrame`].
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rune_visualizer_acquire_shared_frame(
    out: *mut RuneVisualizerSharedFrame,
) -> *const c_void {
    if out.is_null() {
        return std::ptr::null();
    }
    let Some(frame) = VISUALIZER.latest_frame() else {
        return std::ptr::null();
    };
    let Some(handle) = frame.shared_handle() else {
        return std::ptr::null();
    };
    // SAFETY: the caller guarantees that `out` is writable.
    unsafe {
        out.write(RuneVisualizerSharedFrame {
            handle,
            width: frame.width,
            height: frame.height,
            id: frame.id,
        });
    }
    Arc::into_raw(frame).cast()
}

/// Tell the visualizer which GPU Flutter's OpenGL context runs on, by its device and driver UUIDs
/// (`GL_DEVICE_UUID_EXT`, `GL_DRIVER_UUID_EXT`); either null if it is unknown or if the context
/// cannot import Vulkan memory and semaphores (`GL_EXT_memory_object_fd`, `GL_EXT_semaphore_fd`).
/// Frames are shared with Flutter only when the visualizer renders on that GPU and driver,
/// otherwise they are read back. Applies from the next start of the visualizer, so call it before
/// the app asks for one.
///
/// # Safety
///
/// `device_uuid` and `driver_uuid` must each be null or valid for reading 16 bytes.
#[cfg(target_os = "linux")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rune_visualizer_set_graphics_device(
    device_uuid: *const u8,
    driver_uuid: *const u8,
) {
    let uuids = (!device_uuid.is_null() && !driver_uuid.is_null()).then(|| {
        // SAFETY: the caller guarantees 16 readable bytes behind each pointer, which are not null.
        unsafe {
            DeviceUuids {
                device: device_uuid.cast::<[u8; 16]>().read_unaligned(),
                driver: driver_uuid.cast::<[u8; 16]>().read_unaligned(),
            }
        }
    });
    VISUALIZER.set_graphics_device(uuids);
}

/// A frame in a shared image, as filled in by `rune_visualizer_acquire_shared_frame`.
#[cfg(target_os = "linux")]
#[repr(C)]
pub struct RuneVisualizerSharedFrame {
    /// The `VK_FORMAT_R8G8B8A8_UNORM` image with optimal tiling the frame is in, to be imported
    /// as a `GL_RGBA8` texture with `GL_OPTIMAL_TILING_EXT`. Frames in the same image have the
    /// same id, and ids are never reused: import an image's memory once
    /// (`rune_visualizer_export_frame_memory`) and keep the texture under this id.
    pub image_id: u64,
    /// The set of images the frame's image belongs to, one per size and start. The visualizer no
    /// longer draws into images of an earlier set, so their textures can be deleted once Flutter
    /// no longer samples them.
    pub image_set: u64,
    /// The size in bytes of the image's dedicated allocation, for `glImportMemoryFdEXT` (whose
    /// memory object is `GL_DEDICATED_MEMORY_OBJECT_EXT`, the texture at offset 0).
    pub memory_size: u64,
    pub width: u32,
    pub height: u32,
    pub id: u64,
}

/// Hold the latest frame if it is in a shared image, and describe it in `out`. Returns a handle for
/// [`rune_visualizer_release_frame`], or null (nothing written) if there is no such frame. Before
/// sampling the frame, wait once for its semaphore (`rune_visualizer_export_frame_semaphore`) with
/// `glWaitSemaphoreEXT`, naming the texture with the layout `GL_LAYOUT_SHADER_READ_ONLY_EXT`. The
/// visualizer does not draw into the image again while the frame is held, nor for a few frames
/// after. Release it once a newer frame is shown: the renderer skips frames while every image is
/// held.
///
/// # Safety
///
/// `out` must be null or valid for writing a [`RuneVisualizerSharedFrame`].
#[cfg(target_os = "linux")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rune_visualizer_acquire_shared_frame(
    out: *mut RuneVisualizerSharedFrame,
) -> *const c_void {
    if out.is_null() {
        return std::ptr::null();
    }
    let Some(frame) = VISUALIZER.latest_frame() else {
        return std::ptr::null();
    };
    let Some(shared) = frame.shared_image() else {
        return std::ptr::null();
    };
    // SAFETY: the caller guarantees that `out` is writable.
    unsafe {
        out.write(RuneVisualizerSharedFrame {
            image_id: shared.image_id(),
            image_set: shared.image_set(),
            memory_size: shared.memory_size(),
            width: frame.width,
            height: frame.height,
            id: frame.id,
        });
    }
    Arc::into_raw(frame).cast()
}

/// A new file descriptor for the memory of the image a held shared frame is in
/// (`GL_HANDLE_TYPE_OPAQUE_FD_EXT`), or -1 if it cannot be exported. The caller owns it: hand it
/// to `glImportMemoryFdEXT`, which takes it over when it succeeds, or close it.
///
/// # Safety
///
/// `handle` must be null (which gives -1) or a handle from `rune_visualizer_acquire_shared_frame`
/// that was not released yet.
#[cfg(target_os = "linux")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rune_visualizer_export_frame_memory(handle: *const c_void) -> c_int {
    // SAFETY: the caller guarantees a held frame or null.
    unsafe { export_fd(handle, SharedFrame::export_memory) }
}

/// A new file descriptor for the semaphore signalled once a held shared frame was drawn
/// (`GL_HANDLE_TYPE_OPAQUE_FD_EXT`), or -1 if it cannot be exported. The caller owns it: hand it
/// to `glImportSemaphoreFdEXT`, which takes it over when it succeeds, or close it. Each frame has
/// a semaphore of its own, signalled once: wait for it at most once.
///
/// # Safety
///
/// `handle` must be null (which gives -1) or a handle from `rune_visualizer_acquire_shared_frame`
/// that was not released yet.
#[cfg(target_os = "linux")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rune_visualizer_export_frame_semaphore(handle: *const c_void) -> c_int {
    // SAFETY: the caller guarantees a held frame or null.
    unsafe { export_fd(handle, SharedFrame::export_semaphore) }
}

/// The file descriptor `export` makes of the shared frame `handle` holds, or -1 (logged) if it
/// fails, `handle` is null or the frame is not in a shared image.
///
/// # Safety
///
/// `handle` must be null or a handle from `rune_visualizer_acquire_shared_frame` that was not
/// released yet.
#[cfg(target_os = "linux")]
unsafe fn export_fd(
    handle: *const c_void,
    export: impl FnOnce(&SharedFrame) -> anyhow::Result<OwnedFd>,
) -> c_int {
    if handle.is_null() {
        return -1;
    }
    // SAFETY: the caller guarantees an `Arc<Frame>` from `acquire`, held during the call.
    let frame = unsafe { &*handle.cast::<Frame>() };
    match frame.shared_image().map(export) {
        Some(Ok(fd)) => fd.into_raw_fd(),
        Some(Err(e)) => {
            error!("Visualizer: {e:#}");
            -1
        }
        None => -1,
    }
}

/// The app could not show a shared frame (it failed to import it into OpenGL): the visualizer
/// reads frames back from its next frame on, until it stops. Its transport then becomes
/// "readback", which tells the app to show the texture fed with read-back frames.
#[cfg(target_os = "linux")]
#[unsafe(no_mangle)]
pub extern "C" fn rune_visualizer_fall_back_to_readback() {
    VISUALIZER.fall_back_to_readback();
}

/// Give back a frame from [`rune_visualizer_acquire_frame`] or
/// `rune_visualizer_acquire_shared_frame`. Null is ignored.
///
/// # Safety
///
/// `handle` must be null or a handle from [`rune_visualizer_acquire_frame`] or
/// `rune_visualizer_acquire_shared_frame` that was not released yet. Any thread may release it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rune_visualizer_release_frame(handle: *const c_void) {
    if !handle.is_null() {
        // SAFETY: the handle is an `Arc<Frame>` from one of the acquire functions, released
        // exactly once.
        drop(unsafe { Arc::from_raw(handle.cast::<Frame>()) });
    }
}
