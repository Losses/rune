//! What the engine's raster thread calls: one object per texture, reached through the callback's
//! `user_data`.
//!
//! The engine's rules, from its sources (Flutter 3.47.4, `external_texture_d3d.cc` and
//! `external_texture_pixelbuffer.cc`), which these objects follow:
//! - A pixel buffer is uploaded right after the callback returns, and its release callback runs
//!   then; the frame is held until that call.
//! - A GPU surface's release callback runs right after the engine bound the handle, not after
//!   it composited it, and the engine samples the texture until it is given another handle. So
//!   the frame behind the last descriptor stays held until a newer one was handed out (or the
//!   texture is unregistered), and the producer keeps a few of its latest frames besides.
//! - The engine caches only the last handle it opened, and tells handles apart by value.
//! - A null descriptor drops the engine's surface and the same handle is never opened again, so
//!   once there was a frame the last descriptor is handed out again until there is a newer one.

use std::any::Any;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use log::error;

use super::api::{GpuSurfaceDescriptor, PIXEL_FORMAT_RGBA8888, PixelBuffer};
use crate::{Frame, FrameSource, PixelFormat, TextureError, lock};

/// The texture of [`crate::Kind::Dxgi`].
pub(super) struct Gpu {
    source: Arc<dyn FrameSource>,
    state: Mutex<GpuState>,
}

struct GpuState {
    /// The frame behind `descriptor` and its id.
    held: Option<(u64, Box<dyn Any + Send>)>,
    /// What the engine reads after the callback returned: only the raster thread changes it.
    descriptor: GpuSurfaceDescriptor,
}

// SAFETY: the descriptor's pointers are an NT handle, which the held frame keeps open, and
// nothing else; they are only read by the engine's raster thread, which also writes them.
unsafe impl Send for GpuState {}

impl Gpu {
    pub(super) fn new(source: Arc<dyn FrameSource>) -> Self {
        Self {
            source,
            state: Mutex::new(GpuState {
                held: None,
                descriptor: GpuSurfaceDescriptor {
                    struct_size: size_of::<GpuSurfaceDescriptor>(),
                    handle: std::ptr::null_mut(),
                    width: 0,
                    height: 0,
                    visible_width: 0,
                    visible_height: 0,
                    format: PIXEL_FORMAT_RGBA8888,
                    release_callback: None,
                    release_context: std::ptr::null_mut(),
                },
            }),
        }
    }

    /// The descriptor of the newest frame, or of the one handed out last if there is no newer
    /// one (also if the producer falls back to pixels): null before the first frame.
    fn describe(&self) -> *const GpuSurfaceDescriptor {
        let mut state = lock(&self.state);
        let mut previous = None;
        if let Some(acquired) = self.source.acquire() {
            match acquired.frame {
                Frame::Dxgi {
                    handle,
                    width,
                    height,
                } if !handle.is_null()
                    && state.held.as_ref().map(|(id, _)| *id) != Some(acquired.id) =>
                {
                    state.descriptor.handle = handle;
                    state.descriptor.width = width as usize;
                    state.descriptor.height = height as usize;
                    state.descriptor.visible_width = width as usize;
                    state.descriptor.visible_height = height as usize;
                    // Every newer frame was acquired while the previous one was held, so the
                    // handle values differ even if releasing it closes the old handle.
                    previous = state.held.replace((acquired.id, acquired.keep));
                }
                // Already described, or a frame this texture does not show.
                _ => drop(acquired.keep),
            }
        }
        let descriptor = if state.held.is_some() {
            std::ptr::from_ref(&state.descriptor)
        } else {
            std::ptr::null()
        };
        drop(state);
        // Outside the lock: the producer's frame may do work when it goes.
        drop(previous);
        descriptor
    }

    /// The engine let go of the texture: the producer gets its last frame back.
    pub(super) fn release(&self) {
        let held = lock(&self.state).held.take();
        drop(held);
    }
}

/// `FlutterDesktopGpuSurfaceTextureCallback`.
///
/// # Safety
///
/// `user_data` is the `Arc<Gpu>` the texture was registered with, alive until it is unregistered.
pub(super) unsafe extern "C" fn gpu_surface(
    _width: usize,
    _height: usize,
    user_data: *mut c_void,
) -> *const GpuSurfaceDescriptor {
    // SAFETY: as the caller guarantees.
    let gpu = unsafe { &*user_data.cast::<Gpu>() };
    catch_unwind(AssertUnwindSafe(|| gpu.describe())).unwrap_or_else(|_| {
        error!("Flutter texture: the GPU surface callback panicked");
        std::ptr::null()
    })
}

/// The texture of [`crate::Kind::Pixels`].
pub(super) struct Pixels {
    source: Arc<dyn FrameSource>,
    /// The last frame failed; the next failure is not reported again.
    failing: AtomicBool,
}

/// A pixel buffer on its way to the engine, with what keeps its memory alive.
struct Held {
    buffer: PixelBuffer,
    /// The rows, packed, when the producer's had padding.
    _packed: Vec<u8>,
    /// The producer's frame, when `buffer` points into it.
    _keep: Option<Box<dyn Any + Send>>,
}

impl Pixels {
    pub(super) fn new(source: Arc<dyn FrameSource>) -> Self {
        Self {
            source,
            failing: AtomicBool::new(false),
        }
    }

    /// The latest frame as a pixel buffer for the engine, which frees it through its release
    /// callback. Null if there is no frame to show.
    fn copy(&self) -> *const PixelBuffer {
        let Some(acquired) = self.source.acquire() else {
            return std::ptr::null();
        };
        let Frame::Pixels(pixels) = &acquired.frame else {
            return std::ptr::null();
        };
        match Self::hold(pixels, acquired.keep) {
            Ok(held) => {
                self.failing.store(false, Ordering::Relaxed);
                let held = Box::into_raw(held);
                // SAFETY: just made; the engine owns it from here, until it calls `release_held`.
                unsafe {
                    (*held).buffer.release_context = held.cast();
                    std::ptr::addr_of!((*held).buffer)
                }
            }
            Err(e) => {
                if !self.failing.swap(true, Ordering::Relaxed) {
                    error!("Flutter texture: {e}");
                    self.source.failed(e);
                }
                std::ptr::null()
            }
        }
    }

    /// `pixels` as the engine wants them: tightly packed RGBA. Rows with padding are copied (and
    /// the frame given back at once); tight ones are handed over as they are, the frame held until
    /// the engine released the buffer.
    fn hold(pixels: &crate::Pixels, keep: Box<dyn Any + Send>) -> Result<Box<Held>, TextureError> {
        if pixels.format() != PixelFormat::Rgba8 {
            return Err(TextureError::Frame(
                "the engine takes RGBA pixels only".into(),
            ));
        }
        let bytes = pixels
            .bytes()
            .ok_or_else(|| TextureError::Frame("the pixels are empty".into()))?;
        let mut held = Box::new(Held {
            buffer: PixelBuffer {
                buffer: std::ptr::null(),
                width: pixels.width() as usize,
                height: pixels.height() as usize,
                release_callback: Some(release_held),
                release_context: std::ptr::null_mut(),
            },
            _packed: Vec::new(),
            _keep: None,
        });
        if pixels.stride() == pixels.width() as usize * 4 {
            held.buffer.buffer = bytes.as_ptr();
            held._keep = Some(keep);
        } else {
            held._packed = pixels.packed().expect("the bytes are there");
            held.buffer.buffer = held._packed.as_ptr();
            drop(keep);
        }
        Ok(held)
    }
}

/// `FlutterDesktopPixelBufferTextureCallback`.
///
/// # Safety
///
/// `user_data` is the `Arc<Pixels>` the texture was registered with, alive until it is
/// unregistered.
pub(super) unsafe extern "C" fn pixel_buffer(
    _width: usize,
    _height: usize,
    user_data: *mut c_void,
) -> *const PixelBuffer {
    // SAFETY: as the caller guarantees.
    let texture = unsafe { &*user_data.cast::<Pixels>() };
    catch_unwind(AssertUnwindSafe(|| texture.copy())).unwrap_or_else(|_| {
        error!("Flutter texture: the pixel buffer callback panicked");
        std::ptr::null()
    })
}

/// The release callback of the pixel buffers handed out.
///
/// # Safety
///
/// `context` is the `Box<Held>` the buffer is in, released once.
unsafe extern "C" fn release_held(context: *mut c_void) {
    // SAFETY: as the caller guarantees.
    let held = unsafe { Box::from_raw(context.cast::<Held>()) };
    // A panic in the producer's drop must not unwind into the engine.
    if catch_unwind(AssertUnwindSafe(|| drop(held))).is_err() {
        error!("Flutter texture: dropping a frame panicked");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;

    use super::*;
    use crate::{Acquired, Pixels as Rows};

    /// Rows of 2x2 pixels, with a pad of 8 bytes when `padded`, or a shared texture.
    struct Source {
        id: AtomicU64,
        padded: bool,
        dxgi: bool,
        rows: [u8; 24],
        /// How many frames are held.
        held: Arc<std::sync::atomic::AtomicUsize>,
        failures: Mutex<Vec<TextureError>>,
    }

    struct Hold(Arc<std::sync::atomic::AtomicUsize>);

    impl Hold {
        fn new(held: &Arc<std::sync::atomic::AtomicUsize>) -> Box<Self> {
            held.fetch_add(1, Ordering::Relaxed);
            Box::new(Self(Arc::clone(held)))
        }
    }

    impl Drop for Hold {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::Relaxed);
        }
    }

    impl FrameSource for Source {
        fn acquire(&self) -> Option<Acquired> {
            let id = self.id.load(Ordering::Relaxed);
            if id == 0 {
                return None;
            }
            let frame = if self.dxgi {
                Frame::Dxgi {
                    // Any value: the engine is not there to open it. Frames differ by it.
                    handle: (id as usize * 16) as *mut c_void,
                    width: 2,
                    height: 2,
                }
            } else {
                let stride = if self.padded { 12 } else { 8 };
                // SAFETY: `rows` lives as long as the source, which outlives the frames.
                Frame::Pixels(unsafe {
                    Rows::new(self.rows.as_ptr(), 2, 2, stride, PixelFormat::Rgba8)
                })
            };
            Some(Acquired {
                id,
                frame,
                keep: Hold::new(&self.held),
            })
        }

        fn failed(&self, error: TextureError) {
            lock(&self.failures).push(error);
        }
    }

    fn source(padded: bool, dxgi: bool) -> Arc<Source> {
        Arc::new(Source {
            id: AtomicU64::new(0),
            padded,
            dxgi,
            rows: std::array::from_fn(|i| i as u8),
            held: Arc::default(),
            failures: Mutex::default(),
        })
    }

    fn user_data<T>(texture: &Arc<T>) -> *mut c_void {
        Arc::as_ptr(texture).cast_mut().cast()
    }

    #[test]
    fn gpu_surface_holds_the_frame_until_a_newer_one_is_handed_out() {
        let source = source(false, true);
        let texture = Arc::new(Gpu::new(source.clone()));
        // SAFETY: the texture outlives the calls.
        let describe = || unsafe { gpu_surface(0, 0, user_data(&texture)) };
        assert!(describe().is_null());

        source.id.store(1, Ordering::Relaxed);
        let first = describe();
        assert!(!first.is_null());
        // SAFETY: the texture holds the descriptor.
        let first_handle = unsafe { (*first).handle };
        // SAFETY: as above.
        assert_eq!(unsafe { ((*first).visible_width, (*first).format) }, (2, 1));
        // The same frame again: still one held.
        assert_eq!(describe(), first);
        assert_eq!(source.held.load(Ordering::Relaxed), 1);

        // A newer frame: acquired while the old one was held, then the old one goes.
        source.id.store(2, Ordering::Relaxed);
        let second = describe();
        // SAFETY: as above.
        assert_ne!(unsafe { (*second).handle }, first_handle);
        assert_eq!(source.held.load(Ordering::Relaxed), 1);

        // The producer has nothing newer to show (or fell back): the descriptor is handed out
        // again, and never null after a frame.
        source.id.store(0, Ordering::Relaxed);
        assert_eq!(describe(), second);
        assert_eq!(source.held.load(Ordering::Relaxed), 1);

        texture.release();
        assert_eq!(source.held.load(Ordering::Relaxed), 0);
        assert!(describe().is_null());
    }

    #[test]
    fn gpu_surface_ignores_pixels() {
        let source = source(false, false);
        let texture = Arc::new(Gpu::new(source.clone()));
        source.id.store(1, Ordering::Relaxed);
        // SAFETY: the texture outlives the call.
        assert!(unsafe { gpu_surface(0, 0, user_data(&texture)) }.is_null());
        assert_eq!(source.held.load(Ordering::Relaxed), 0);
        assert!(lock(&source.failures).is_empty());
    }

    #[test]
    fn pixel_buffer_of_tight_rows_holds_the_frame_until_released() {
        let source = source(false, false);
        let texture = Arc::new(Pixels::new(source.clone()));
        // SAFETY: the texture outlives the call.
        let copy = || unsafe { pixel_buffer(0, 0, user_data(&texture)) };
        assert!(copy().is_null());

        source.id.store(1, Ordering::Relaxed);
        let buffer = copy();
        assert!(!buffer.is_null());
        assert_eq!(source.held.load(Ordering::Relaxed), 1);
        // SAFETY: the buffer is the one just handed out, released once below.
        unsafe {
            assert_eq!(((*buffer).width, (*buffer).height), (2, 2));
            assert_eq!((*buffer).buffer, source.rows.as_ptr());
            ((*buffer).release_callback.unwrap())((*buffer).release_context);
        }
        assert_eq!(source.held.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn pixel_buffer_of_padded_rows_is_packed() {
        let source = source(true, false);
        let texture = Arc::new(Pixels::new(source.clone()));
        source.id.store(1, Ordering::Relaxed);
        // SAFETY: the texture outlives the call.
        let buffer = unsafe { pixel_buffer(0, 0, user_data(&texture)) };
        assert!(!buffer.is_null());
        // The copy is done: the producer has its frame back already.
        assert_eq!(source.held.load(Ordering::Relaxed), 0);
        // SAFETY: the buffer is the one just handed out, released once below.
        unsafe {
            let rows = std::slice::from_raw_parts((*buffer).buffer, 16);
            assert_eq!(
                rows,
                [0, 1, 2, 3, 4, 5, 6, 7, 12, 13, 14, 15, 16, 17, 18, 19]
            );
            ((*buffer).release_callback.unwrap())((*buffer).release_context);
        }
    }
}
