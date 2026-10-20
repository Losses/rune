//! The process-wide visualizer and its C interface.
//!
//! The native side of the app pulls finished frames through these functions, so that the pixels
//! never travel through Dart:
//! - macOS: an IOSurface-backed pixel buffer ([`rune_visualizer_copy_pixel_buffer`]), or a copy of
//!   a read-back frame ([`rune_visualizer_copy_frame`]) if pixel buffers failed;
//! - Windows and Linux: the read-back frame itself, held until released
//!   ([`rune_visualizer_acquire_frame`], [`rune_visualizer_release_frame`]).
//!
//! The functions are exported from `rinf.framework` on macOS and from the hub library
//! (`hub.dll`, `libhub.so`) elsewhere.

use std::ffi::c_void;
use std::sync::{Arc, LazyLock};

use visualizer::{Frame, Visualizer};

pub static VISUALIZER: LazyLock<Visualizer> = LazyLock::new(Visualizer::new);

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

/// Give back a frame from [`rune_visualizer_acquire_frame`]. Null is ignored.
///
/// # Safety
///
/// `handle` must be null or a handle from [`rune_visualizer_acquire_frame`] that was not released
/// yet. Any thread may release it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rune_visualizer_release_frame(handle: *const c_void) {
    if !handle.is_null() {
        // SAFETY: the handle is an `Arc<Frame>` from `acquire`, released exactly once.
        drop(unsafe { Arc::from_raw(handle.cast::<Frame>()) });
    }
}
