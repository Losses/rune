//! Frames made in Rust shown in Flutter textures, without linking Flutter.
//!
//! The app hands over its engine's texture registry once ([`attach`]). After that a
//! [`FrameSource`] can be registered as a texture ([`register`]), whose id a `Texture` widget
//! shows. The producer says when a new frame is there ([`FrameNotifier::mark_frame_available`]),
//! and Flutter pulls it from the source when it draws next. Everything Flutter offers is looked up
//! at run time, so the crate builds without Flutter's headers or libraries.
//!
//! - Apple: the registry is the `FlutterEngine` (macOS). A frame in an IOSurface-backed
//!   `CVPixelBuffer` is handed over without a copy; pixels are copied into one. One texture
//!   takes both, so a producer may fall back from one to the other under the same texture id.
//! - Elsewhere: not yet ([`TextureError::Unsupported`]).

#[cfg(target_vendor = "apple")]
mod darwin;
mod thread;

#[cfg(target_vendor = "apple")]
use darwin as platform;
#[cfg(not(target_vendor = "apple"))]
use unsupported as platform;

use std::any::Any;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use log::info;

#[cfg(target_vendor = "apple")]
use objc2_core_foundation::CFRetained;
#[cfg(target_vendor = "apple")]
use objc2_core_video::CVPixelBuffer;

pub type Result<T, E = TextureError> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextureError {
    /// This platform has no texture layer yet.
    Unsupported,
    /// No registry was attached (see [`attach`]).
    NotAttached,
    /// The handle given to [`attach`] is not usable.
    InvalidHandle,
    /// The platform thread did not run the call (it panicked or the app is quitting).
    PlatformThread,
    /// A frame could not be handed to Flutter.
    Frame(String),
}

impl fmt::Display for TextureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("Flutter textures are not supported on this platform"),
            Self::NotAttached => f.write_str("No Flutter texture registry is attached"),
            Self::InvalidHandle => f.write_str("The Flutter texture registry handle is not usable"),
            Self::PlatformThread => f.write_str("The platform thread did not run the call"),
            Self::Frame(message) => write!(f, "A frame could not be handed to Flutter: {message}"),
        }
    }
}

impl std::error::Error for TextureError {}

/// What the app hands over in [`attach`].
#[non_exhaustive]
pub enum PlatformHandle {
    /// An object that answers `FlutterTextureRegistry`'s messages: the `FlutterEngine` on macOS.
    #[cfg(target_vendor = "apple")]
    TextureRegistry(std::ptr::NonNull<std::ffi::c_void>),
}

/// The order of the four bytes of a pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba8,
    Bgra8,
}

/// Rows of pixels in memory the producer owns, top to bottom.
#[derive(Debug)]
pub struct Pixels {
    ptr: *const u8,
    width: u32,
    height: u32,
    stride: usize,
    format: PixelFormat,
}

impl Pixels {
    /// `height` rows of `stride` bytes at `ptr`, each starting with `width` pixels.
    ///
    /// # Safety
    ///
    /// `ptr` must be valid for reading `stride * (height - 1) + width * 4` bytes, which nothing
    /// writes, while the [`Acquired::keep`] it comes with is alive.
    pub unsafe fn new(
        ptr: *const u8,
        width: u32,
        height: u32,
        stride: usize,
        format: PixelFormat,
    ) -> Self {
        Self {
            ptr,
            width,
            height,
            stride,
            format,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Bytes from one row to the next.
    pub fn stride(&self) -> usize {
        self.stride
    }

    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// The bytes of the rows, or `None` if the size is empty or the rows overlap.
    #[cfg_attr(not(target_vendor = "apple"), allow(dead_code))]
    fn bytes(&self) -> Option<&[u8]> {
        let row = self.width as usize * 4;
        if self.ptr.is_null() || row == 0 || self.height == 0 || self.stride < row {
            return None;
        }
        let len = self.stride * (self.height as usize - 1) + row;
        // SAFETY: `new`'s caller guarantees `len` readable bytes while the frame is held, which
        // outlives this borrow.
        Some(unsafe { std::slice::from_raw_parts(self.ptr, len) })
    }
}

/// One picture for Flutter.
pub enum Frame {
    /// Pixels in memory, which the texture copies.
    Pixels(Pixels),
    /// An IOSurface-backed `kCVPixelFormatType_32BGRA` pixel buffer, which Flutter samples
    /// without a copy. The producer must not draw into it again while Flutter may still sample
    /// it (in practice: until a few newer frames were handed over).
    #[cfg(target_vendor = "apple")]
    PixelBuffer(CFRetained<CVPixelBuffer>),
}

/// The latest frame of a [`FrameSource`].
pub struct Acquired {
    /// Changes with every new frame; the texture skips a frame with the id of the one it has.
    pub id: u64,
    pub frame: Frame,
    /// Whatever keeps the frame's memory alive and unchanged, dropped once Flutter no longer
    /// reads it.
    pub keep: Box<dyn Any + Send>,
}

/// Where a texture gets its frames.
pub trait FrameSource: Send + Sync + 'static {
    /// The latest frame, if there is one. Called on Flutter's raster thread when it draws a frame
    /// after [`FrameNotifier::mark_frame_available`]: it must not block.
    fn acquire(&self) -> Option<Acquired>;

    /// A frame from [`Self::acquire`] could not be shown; the texture keeps the last one. Called
    /// on the raster thread once per run of failures.
    fn failed(&self, _error: TextureError) {}
}

/// Hand over the engine's texture registry, which [`register`] uses from then on. A second call
/// replaces the registry; textures registered before stay with the one they were registered on.
///
/// # Safety
///
/// The handle must be what [`PlatformHandle`] says it is, valid for the duration of the call
/// (the registry is retained here).
pub unsafe fn attach(handle: PlatformHandle) -> Result<()> {
    // SAFETY: the caller guarantees a valid handle.
    let registry = unsafe { platform::Registry::new(handle) }?;
    let mut attachment = lock(&ATTACHMENT);
    let previous = attachment.replace(registry);
    info!(
        "Attached Flutter texture registry {}",
        attachment.generation
    );
    drop(attachment);
    drop(previous);
    Ok(())
}

/// Register `source` as a texture of the attached registry. Hops to the platform thread, where
/// the engine wants it.
pub async fn register(source: Arc<dyn FrameSource>) -> Result<Texture> {
    let (generation, registry) = lock(&ATTACHMENT)
        .current()
        .ok_or(TextureError::NotAttached)?;
    let registered = platform::register(registry, source).await?;
    let id = registered.id();
    info!("Registered Flutter texture {id} (registry {generation})");
    Ok(Texture {
        id,
        notifier: registered.notifier(),
        registered: Some(registered),
    })
}

/// A registered texture. Dropping it unregisters it too, without waiting.
pub struct Texture {
    id: i64,
    notifier: FrameNotifier,
    /// `None` once unregistered.
    registered: Option<platform::Registered>,
}

impl Texture {
    /// The id for Flutter's `Texture` widget.
    pub fn id(&self) -> i64 {
        self.id
    }

    /// Tells Flutter about new frames.
    pub fn notifier(&self) -> FrameNotifier {
        self.notifier.clone()
    }

    /// Unregister the texture and wait until the engine let go of it. Its notifier does nothing
    /// from now on.
    pub async fn unregister(mut self) {
        self.notifier.close();
        if let Some(registered) = self.registered.take() {
            registered.unregister().await;
            info!("Unregistered Flutter texture {}", self.id);
        }
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        self.notifier.close();
        if let Some(registered) = self.registered.take() {
            registered.unregister_later();
        }
    }
}

/// Tells Flutter that a texture has a new frame. Calls from any thread; while one notice waits for
/// the platform thread, more calls add nothing. Does nothing once the texture is unregistered.
#[derive(Clone)]
pub struct FrameNotifier(Arc<Notice>);

/// Work for the platform thread.
type Task = Box<dyn FnOnce() + Send>;

struct Notice {
    /// A notice waits for the platform thread.
    pending: AtomicBool,
    /// The texture is registered.
    open: AtomicBool,
    /// Runs a task on the platform thread.
    post: Box<dyn Fn(Task) + Send + Sync>,
    /// Tells the engine; called on the platform thread.
    fire: Box<dyn Fn() + Send + Sync>,
}

impl FrameNotifier {
    #[cfg_attr(not(target_vendor = "apple"), allow(dead_code))]
    fn new(
        post: impl Fn(Task) + Send + Sync + 'static,
        fire: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(Notice {
            pending: AtomicBool::new(false),
            open: AtomicBool::new(true),
            post: Box::new(post),
            fire: Box::new(fire),
        }))
    }

    pub fn mark_frame_available(&self) {
        let notice = &self.0;
        if !notice.open.load(Ordering::Acquire) || notice.pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let task = Arc::clone(notice);
        (notice.post)(Box::new(move || {
            // Cleared first: a frame that comes while the engine is told gets its own notice.
            task.pending.store(false, Ordering::Release);
            if task.open.load(Ordering::Acquire) {
                (task.fire)();
            }
        }));
    }

    fn close(&self) {
        self.0.open.store(false, Ordering::Release);
    }
}

/// The registry handed over last and how many were handed over.
struct Attachment<R> {
    generation: u64,
    registry: Option<Arc<R>>,
}

impl<R> Attachment<R> {
    const fn new() -> Self {
        Self {
            generation: 0,
            registry: None,
        }
    }

    /// Make `registry` the current one; returns the one it replaces.
    fn replace(&mut self, registry: R) -> Option<Arc<R>> {
        self.generation += 1;
        self.registry.replace(Arc::new(registry))
    }

    fn current(&self) -> Option<(u64, Arc<R>)> {
        self.registry
            .as_ref()
            .map(|registry| (self.generation, Arc::clone(registry)))
    }
}

static ATTACHMENT: Mutex<Attachment<platform::Registry>> = Mutex::new(Attachment::new());

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Platforms without a texture layer yet: nothing can be attached, so nothing is registered.
#[cfg(not(target_vendor = "apple"))]
mod unsupported {
    use std::sync::Arc;

    use crate::{FrameNotifier, FrameSource, PlatformHandle, Result, TextureError};

    pub(crate) enum Registry {}

    impl Registry {
        pub(crate) unsafe fn new(handle: PlatformHandle) -> Result<Self> {
            match handle {}
        }
    }

    pub(crate) enum Registered {}

    impl Registered {
        pub(crate) fn id(&self) -> i64 {
            match *self {}
        }

        pub(crate) fn notifier(&self) -> FrameNotifier {
            match *self {}
        }

        pub(crate) async fn unregister(self) {
            match self {}
        }

        pub(crate) fn unregister_later(self) {
            match self {}
        }
    }

    pub(crate) async fn register(
        registry: Arc<Registry>,
        _source: Arc<dyn FrameSource>,
    ) -> Result<Registered> {
        match *registry {}
        #[allow(unreachable_code)]
        Err(TextureError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    type Queue = Arc<Mutex<Vec<Task>>>;

    /// A notifier whose platform thread is a queue that the test runs, and how often it fired.
    fn notifier() -> (FrameNotifier, Queue, Arc<AtomicUsize>) {
        let queue: Queue = Arc::default();
        let fired = Arc::new(AtomicUsize::new(0));
        let notifier = FrameNotifier::new(
            {
                let queue = Arc::clone(&queue);
                move |task| lock(&queue).push(task)
            },
            {
                let fired = Arc::clone(&fired);
                move || {
                    fired.fetch_add(1, Ordering::Relaxed);
                }
            },
        );
        (notifier, queue, fired)
    }

    fn run_all(queue: &Queue) {
        let tasks: Vec<_> = lock(queue).drain(..).collect();
        for task in tasks {
            task();
        }
    }

    #[test]
    fn notices_coalesce_while_one_waits() {
        let (notifier, queue, fired) = notifier();
        notifier.mark_frame_available();
        notifier.clone().mark_frame_available();
        notifier.mark_frame_available();
        assert_eq!(lock(&queue).len(), 1);
        run_all(&queue);
        assert_eq!(fired.load(Ordering::Relaxed), 1);

        // Once the engine was told, the next frame gets a notice of its own.
        notifier.mark_frame_available();
        assert_eq!(lock(&queue).len(), 1);
        run_all(&queue);
        assert_eq!(fired.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn closed_notifier_does_nothing() {
        let (notifier, queue, fired) = notifier();
        notifier.mark_frame_available();
        // Unregistered while a notice waits: it must not reach the engine.
        notifier.close();
        run_all(&queue);
        assert_eq!(fired.load(Ordering::Relaxed), 0);
        notifier.mark_frame_available();
        assert!(lock(&queue).is_empty());
    }

    #[test]
    fn attach_replaces_the_registry() {
        struct Registry(&'static str, Arc<AtomicUsize>);
        impl Drop for Registry {
            fn drop(&mut self) {
                self.1.fetch_add(1, Ordering::Relaxed);
            }
        }

        let dropped = Arc::new(AtomicUsize::new(0));
        let mut attachment = Attachment::new();
        assert!(attachment.current().is_none());

        assert!(
            attachment
                .replace(Registry("first", Arc::clone(&dropped)))
                .is_none()
        );
        let (generation, first) = attachment.current().unwrap();
        assert_eq!((generation, first.0), (1, "first"));

        let previous = attachment.replace(Registry("second", Arc::clone(&dropped)));
        let (generation, second) = attachment.current().unwrap();
        assert_eq!((generation, second.0), (2, "second"));
        // A texture registered on the first registry still holds it.
        drop(previous);
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
        drop(first);
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
    }
}
