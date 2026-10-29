//! The object the engine is given as a texture.
//!
//! The engine only sends it `copyPixelBuffer` (on the raster thread) and never checks that it
//! conforms to `FlutterTexture`, a protocol FlutterMacOS has no run-time metadata for; so it is a
//! plain `NSObject` subclass with that method.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use log::error;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_core_foundation::CFRetained;
use objc2_core_video::CVPixelBuffer;

use super::pixels::Pool;
use crate::{Frame, FrameSource, lock};

/// What the raster thread works with.
pub(super) struct Raster {
    source: Arc<dyn FrameSource>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// The frame handed over last and its id, handed over again until there is a newer one.
    last: Option<(u64, CFRetained<CVPixelBuffer>)>,
    /// Buffers for frames that come as pixels.
    pool: Option<Pool>,
    /// The last frame failed; the next failure is not reported again.
    failing: bool,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `RuneTexture` does not implement
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "RuneFlutterTexture"]
    #[ivars = Raster]
    pub(super) struct RuneTexture;

    impl RuneTexture {
        /// `-[FlutterTexture copyPixelBuffer]`: the latest frame with a reference that the engine
        /// takes over, or null before the first frame. Runs on the raster thread.
        #[unsafe(method(copyPixelBuffer))]
        fn copy_pixel_buffer(&self) -> *mut CVPixelBuffer {
            match catch_unwind(AssertUnwindSafe(|| self.ivars().latest())) {
                // A raw pointer return is passed through as it is (no autorelease), so the
                // reference goes to the engine.
                Ok(Some(buffer)) => CFRetained::into_raw(buffer).as_ptr(),
                Ok(None) => std::ptr::null_mut(),
                Err(_) => {
                    error!("Flutter texture: copyPixelBuffer panicked");
                    std::ptr::null_mut()
                }
            }
        }
    }

    unsafe impl NSObjectProtocol for RuneTexture {}
);

// SAFETY: the instance variables are thread-safe: the source is `Send + Sync`, and the state is
// only reached under its mutex. CoreFoundation and CoreVideo count references atomically, and a
// buffer's pixels are only written under `CVPixelBufferLockBaseAddress`, before the buffer is
// handed over.
unsafe impl Send for RuneTexture {}
// SAFETY: as above.
unsafe impl Sync for RuneTexture {}

impl RuneTexture {
    pub(super) fn new(source: Arc<dyn FrameSource>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Raster {
            source,
            state: Mutex::new(State::default()),
        });
        // SAFETY: `-[NSObject init]` on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}

impl Raster {
    /// The latest frame as a pixel buffer, retained for the caller: a newer one from the source if
    /// there is one, else the last one again.
    fn latest(&self) -> Option<CFRetained<CVPixelBuffer>> {
        let mut state = lock(&self.state);
        if let Some(acquired) = self.source.acquire()
            && state.last.as_ref().map(|(id, _)| *id) != Some(acquired.id)
        {
            let buffer = match &acquired.frame {
                Frame::PixelBuffer(buffer) => {
                    // The producer renders into its own buffers now: no copies needed.
                    state.pool = None;
                    Ok(buffer.clone())
                }
                Frame::Pixels(pixels) => Pool::copy(&mut state.pool, pixels),
            };
            // The copy is done (or the buffer retained): the producer may have its frame back.
            drop(acquired.keep);
            match buffer {
                Ok(buffer) => {
                    state.last = Some((acquired.id, buffer));
                    state.failing = false;
                }
                Err(e) => {
                    if !std::mem::replace(&mut state.failing, true) {
                        error!("Flutter texture: {e}");
                        self.source.failed(e);
                    }
                }
            }
        }
        state.last.as_ref().map(|(_, buffer)| buffer.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use objc2_core_foundation::CFGetRetainCount;

    use super::*;
    use crate::{Acquired, PixelFormat, Pixels, TextureError};

    /// Frames of 2x1 RGBA pixels whose id the test sets.
    struct Source {
        id: AtomicU64,
        pixels: [u8; 8],
        failures: Mutex<Vec<TextureError>>,
    }

    impl FrameSource for Source {
        fn acquire(&self) -> Option<Acquired> {
            let id = self.id.load(Ordering::Relaxed);
            (id != 0).then(|| Acquired {
                id,
                // SAFETY: `pixels` lives as long as the source, which outlives the call.
                frame: Frame::Pixels(unsafe {
                    Pixels::new(self.pixels.as_ptr(), 2, 1, 8, PixelFormat::Rgba8)
                }),
                keep: Box::new(()),
            })
        }

        fn failed(&self, error: TextureError) {
            lock(&self.failures).push(error);
        }
    }

    fn copy(texture: &RuneTexture) -> *mut CVPixelBuffer {
        // SAFETY: the method takes nothing and returns a pixel buffer or null.
        unsafe { msg_send![texture, copyPixelBuffer] }
    }

    #[test]
    fn copy_pixel_buffer_hands_over_one_reference() {
        let source = Arc::new(Source {
            id: AtomicU64::new(0),
            pixels: [1, 2, 3, 4, 5, 6, 7, 8],
            failures: Mutex::default(),
        });
        let texture = RuneTexture::new(source.clone());
        assert!(copy(&texture).is_null());

        source.id.store(1, Ordering::Relaxed);
        let first = copy(&texture);
        assert!(!first.is_null());
        // SAFETY: the engine's side of the contract: it owns the returned reference.
        let first = unsafe { CFRetained::from_raw(std::ptr::NonNull::new(first).unwrap()) };
        // Ours and the texture's (the frame it hands over again); the pool does not count it while
        // it is in use.
        assert_eq!(CFGetRetainCount(Some(&first)), 2);

        // No new frame: the same buffer again, with another reference for the caller.
        let again = copy(&texture);
        assert_eq!(again, CFRetained::as_ptr(&first).as_ptr());
        assert_eq!(CFGetRetainCount(Some(&first)), 3);
        // SAFETY: as above.
        drop(unsafe { CFRetained::from_raw(std::ptr::NonNull::new(again).unwrap()) });
        assert_eq!(CFGetRetainCount(Some(&first)), 2);

        // A new frame replaces the texture's reference to the old one.
        source.id.store(2, Ordering::Relaxed);
        let second = copy(&texture);
        assert_ne!(second, CFRetained::as_ptr(&first).as_ptr());
        assert_eq!(CFGetRetainCount(Some(&first)), 1);
        // SAFETY: as above.
        drop(unsafe { CFRetained::from_raw(std::ptr::NonNull::new(second).unwrap()) });
        assert!(lock(&source.failures).is_empty());
    }
}
