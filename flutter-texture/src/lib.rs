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
//! - Windows: the registry is the engine's plugin registrar, whose C API is looked up in
//!   `flutter_windows.dll`. A frame in a shared DXGI texture ([`Kind::Dxgi`]) is opened by the
//!   engine without a copy; pixels ([`Kind::Pixels`]) are uploaded. The two are separate
//!   textures, so a producer that falls back from one to the other registers the other kind and
//!   switches to it. The registry may be used from any thread, so there is no platform thread
//!   hop; the engine reads frames on its raster thread.
//! - Linux: the registry is the `FlEngine`'s `FlTextureRegistrar`; the engine and GLib are
//!   looked up in the process. A frame in a Vulkan image ([`Frame::VulkanImage`]) is imported
//!   into Flutter's OpenGL context without a copy; pixels are uploaded into a texture of our own.
//!   One texture takes both, so a producer may fall back from one to the other under the same
//!   texture id. The registrar is used on the GLib main context (the platform thread).
//! - Elsewhere: not yet ([`TextureError::Unsupported`]).

#[cfg(target_vendor = "apple")]
mod darwin;
#[cfg(target_os = "linux")]
mod linux;
/// The plain bookkeeping of the Linux textures, tested on every host.
#[cfg(all(test, not(target_os = "linux")))]
#[path = "linux/logic.rs"]
mod linux_logic;
mod thread;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
use self::windows as platform;
#[cfg(target_vendor = "apple")]
use darwin as platform;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(not(any(target_vendor = "apple", windows, target_os = "linux")))]
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
    /// A frame in GPU memory shared with Flutter could not be shown (Linux: Flutter's OpenGL
    /// context could not import it). The producer should hand over pixels from now on.
    SharedFrame(String),
}

impl fmt::Display for TextureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("Flutter textures are not supported on this platform"),
            Self::NotAttached => f.write_str("No Flutter texture registry is attached"),
            Self::InvalidHandle => f.write_str("The Flutter texture registry handle is not usable"),
            Self::PlatformThread => f.write_str("The platform thread did not run the call"),
            Self::Frame(message) => write!(f, "A frame could not be handed to Flutter: {message}"),
            Self::SharedFrame(message) => {
                write!(f, "A frame in GPU memory could not be shown: {message}")
            }
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
    /// A `FlutterDesktopPluginRegistrarRef` on Windows, which the runner gets from the engine
    /// (`GetRegistrarForPlugin`). It stays valid until the engine is destroyed.
    #[cfg(windows)]
    PluginRegistrar(std::ptr::NonNull<std::ffi::c_void>),
    /// An `FlEngine*` on Linux (`fl_view_get_engine`). Its texture registrar is retained.
    #[cfg(target_os = "linux")]
    FlEngine(std::ptr::NonNull<std::ffi::c_void>),
}

/// What a texture shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// [`Frame::Pixels`] (and on Apple [`Frame::PixelBuffer`], on Linux [`Frame::VulkanImage`]).
    Pixels,
    /// Windows: [`Frame::Dxgi`].
    #[cfg(windows)]
    Dxgi,
}

/// What the attached engine can do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Windows: the LUID (`DXGI_ADAPTER_DESC::AdapterLuid`: low part, high part) of the adapter
    /// Flutter renders on, which a [`Frame::Dxgi`] has to come from. `None` where it is unknown,
    /// without an engine, and on other platforms.
    pub adapter_luid: Option<(u32, i32)>,
    /// Linux: the GPU and driver of Flutter's OpenGL context, which a [`Frame::VulkanImage`] has
    /// to come from. `None` where the context cannot import Vulkan memory and semaphores (or
    /// runs on several GPUs, or in software), once an import failed, without an engine, and on
    /// other platforms.
    pub device_uuids: Option<DeviceUuids>,
}

/// The device and driver UUIDs of a GPU, as Vulkan (`VkPhysicalDeviceIDProperties`) and OpenGL
/// (`GL_DEVICE_UUID_EXT`, `GL_DRIVER_UUID_EXT`) report them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceUuids {
    pub device: [u8; 16],
    pub driver: [u8; 16],
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
    #[cfg_attr(
        not(any(target_vendor = "apple", windows, target_os = "linux")),
        allow(dead_code)
    )]
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

    /// The rows without padding (`width * 4` bytes each), for consumers that take no stride.
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    fn packed(&self) -> Option<Vec<u8>> {
        let bytes = self.bytes()?;
        let row = self.width as usize * 4;
        let mut packed = Vec::with_capacity(row * self.height as usize);
        for y in 0..self.height as usize {
            packed.extend_from_slice(&bytes[y * self.stride..][..row]);
        }
        Some(packed)
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
    /// A `DXGI_FORMAT_R8G8B8A8_UNORM` texture shared through an NT handle, which Flutter opens
    /// without a copy. The handle belongs to the producer's frame: it must stay open, and the
    /// producer must not draw into the texture again, while the [`Acquired::keep`] it comes with
    /// is alive. For [`Kind::Dxgi`].
    #[cfg(windows)]
    Dxgi {
        handle: *mut std::ffi::c_void,
        width: u32,
        height: u32,
    },
    /// Linux: a frame in a Vulkan image, which the texture imports into Flutter's OpenGL context
    /// without a copy.
    #[cfg(target_os = "linux")]
    VulkanImage(VulkanImage),
}

/// A frame in a `VK_FORMAT_R8G8B8A8_UNORM` image with optimal tiling, in a dedicated allocation
/// exported as an opaque file descriptor. The producer released the image to
/// `VK_QUEUE_FAMILY_EXTERNAL` in `VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL` and signals the
/// frame's semaphore once it is drawn. It must not draw into the image again while the
/// [`Acquired::keep`] it comes with is alive, nor for a few frames after: OpenGL never says when
/// it is done sampling.
#[cfg(target_os = "linux")]
pub struct VulkanImage {
    /// The image the frame is in. Frames in the same image have the same id, and ids are never
    /// reused: the texture imports an image's memory once and keeps it under this id.
    pub image_id: u64,
    /// The set of images the image belongs to. The producer no longer draws into images of an
    /// earlier set, so the texture lets go of them once a frame of a newer set is shown.
    pub image_set: u64,
    /// The size in bytes of the image's dedicated allocation.
    pub memory_size: u64,
    pub width: u32,
    pub height: u32,
    /// Exports the image's memory and the frame's semaphore.
    pub fds: Box<dyn VulkanFds>,
}

/// New file descriptors for what Flutter's OpenGL context imports of a [`VulkanImage`]. Each call
/// returns a new descriptor that the caller owns.
#[cfg(target_os = "linux")]
pub trait VulkanFds {
    /// The image's memory (`VK_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_FD_BIT`).
    fn export_memory(&self) -> Result<std::os::fd::OwnedFd, String>;
    /// The semaphore the frame's submission signals
    /// (`VK_EXTERNAL_SEMAPHORE_HANDLE_TYPE_OPAQUE_FD_BIT`). It is signalled once, so it is waited
    /// for at most once.
    fn export_semaphore(&self) -> Result<std::os::fd::OwnedFd, String>;
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
    /// after [`FrameNotifier::mark_frame_available`]: it must not block. A texture ignores a
    /// frame of the other [`Kind`] (a [`Frame::Dxgi`] is no use to a [`Kind::Pixels`] texture and
    /// the other way round); it keeps the last one it showed.
    fn acquire(&self) -> Option<Acquired>;

    /// A frame from [`Self::acquire`] could not be shown; the texture keeps the last one. Called
    /// on the raster thread once per run of failures. A [`TextureError::SharedFrame`] means
    /// frames on the GPU cannot be shown at all: the texture ignores them from now on and the
    /// producer should hand over pixels.
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

/// What the attached engine can do, or the defaults if none is attached. On Linux the first call
/// probes Flutter's OpenGL context on the platform thread.
pub async fn capabilities() -> Capabilities {
    let current = lock(&ATTACHMENT).current();
    let Some((_, registry)) = current else {
        return Capabilities::default();
    };
    Capabilities {
        adapter_luid: registry.adapter_luid(),
        device_uuids: registry.device_uuids().await,
    }
}

/// Register `source` as a texture of the attached registry, which shows frames of `kind`. On
/// Apple and Linux this hops to the platform thread, where the engine wants it.
pub async fn register(source: Arc<dyn FrameSource>, kind: Kind) -> Result<Texture> {
    let (generation, registry) = lock(&ATTACHMENT)
        .current()
        .ok_or(TextureError::NotAttached)?;
    let registered = platform::register(registry, source, kind).await?;
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
    #[cfg_attr(
        not(any(target_vendor = "apple", windows, target_os = "linux")),
        allow(dead_code)
    )]
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
#[cfg(not(any(target_vendor = "apple", windows, target_os = "linux")))]
mod unsupported {
    use std::sync::Arc;

    use crate::{
        DeviceUuids, FrameNotifier, FrameSource, Kind, PlatformHandle, Result, TextureError,
    };

    pub(crate) enum Registry {}

    impl Registry {
        pub(crate) unsafe fn new(handle: PlatformHandle) -> Result<Self> {
            match handle {}
        }

        pub(crate) fn adapter_luid(&self) -> Option<(u32, i32)> {
            match *self {}
        }

        pub(crate) async fn device_uuids(&self) -> Option<DeviceUuids> {
            match *self {}
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
        _kind: Kind,
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
    fn packed_drops_the_padding() {
        // Two rows of one pixel, each followed by four bytes of padding.
        let bytes = [1, 2, 3, 4, 0, 0, 0, 0, 5, 6, 7, 8, 0, 0, 0, 0];
        // SAFETY: `bytes` holds the rows and outlives `pixels`.
        let pixels = unsafe { Pixels::new(bytes.as_ptr(), 1, 2, 8, PixelFormat::Rgba8) };
        assert_eq!(pixels.packed().unwrap(), [1, 2, 3, 4, 5, 6, 7, 8]);
        // SAFETY: as above.
        let tight = unsafe { Pixels::new(bytes.as_ptr(), 2, 1, 8, PixelFormat::Rgba8) };
        assert_eq!(tight.packed().unwrap(), [1, 2, 3, 4, 0, 0, 0, 0]);
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
