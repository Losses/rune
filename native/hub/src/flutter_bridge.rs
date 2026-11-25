//! The visualizer's frames in a Flutter texture of our own texture layer (`flutter_texture`).
//!
//! The runner hands the engine over at launch (`rune_flutter_attach`, on Android
//! `MainActivity.attachFlutter`); when the visualizer runs,
//! a texture showing its latest frame is registered and its id is sent to Dart
//! (`VisualizerTextureResponse`). A texture that replaces another one is sent the same way;
//! the old one is unregistered once Dart says it shows the new one
//! (`VisualizerTextureShownRequest`).
//!
//! - Apple and Linux: one texture takes the frames on the GPU (pixel buffers, Vulkan images) and
//!   the read-back frames alike, so it is made when the visualizer starts. On Linux, if Flutter's
//!   OpenGL context cannot import a Vulkan image, the visualizer reads frames back from then on,
//!   into the same texture.
//! - Windows: the engine has one texture type for shared DirectX textures and another for pixels,
//!   so the texture is made once the visualizer says how its frames leave the GPU, and replaced
//!   when that changes (a fall back to reading frames back). If the GPU texture cannot be
//!   registered the visualizer falls back to reading frames back, which ends in a pixel texture.
//! - Android: the texture is a `SurfaceProducer`, made before the visualizer starts, whose native
//!   window the visualizer presents frames into ([`prepare_visualizer`]); nothing is handed over
//!   frame by frame. A resize gives the producer the new size and the visualizer the producer's
//!   new window ([`resize_visualizer`]). Frames read back would show nowhere, so there is no
//!   fallback: a window the visualizer cannot present into shows up in its status.

#[cfg(target_os = "linux")]
use std::os::fd::OwnedFd;
use std::ptr::NonNull;
use std::sync::{Arc, LazyLock, Weak};
use tokio::runtime::Handle;

use log::{error, info, warn};
use rinf::RustSignal;
use tokio::sync::Mutex;

use ::flutter_texture::{
    Acquired, Frame, FrameSource, Kind, PixelFormat, Pixels, Texture, TextureError,
};
use ::visualizer::{DeviceUuids, PcmInput, Transport};

use crate::messages::VisualizerTextureResponse;
use flutter_texture::PlatformHandle;
#[cfg(target_os = "windows")]
use std::env;
use std::ffi;
use std::mem;
#[cfg(target_os = "android")]
use std::panic;
#[cfg(target_os = "linux")]
use visualizer::SharedFrame;

/// Whether the visualizer is shown through `flutter_texture` here (else not at all).
const TEXTURE_LAYER: bool = cfg!(any(
    target_vendor = "apple",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
));

/// Hand the engine's texture registry over to `flutter_texture`. Called by the runner once the
/// engine exists; returns whether it worked.
///
/// # Safety
///
/// `registry` must be null or, on macOS, point to the `FlutterEngine`, on iOS to the registrar's `FlutterTextureRegistry` (`FlutterPluginRegistrar.textures()`); on Windows, be the
/// engine's `FlutterDesktopPluginRegistrarRef` (`FlutterEngine::GetRegistrarForPlugin`); on Linux,
/// be the `FlEngine*` (`fl_view_get_engine`), called on the GTK main thread.
#[cfg(any(target_vendor = "apple", target_os = "windows", target_os = "linux"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rune_flutter_attach(registry: *mut ffi::c_void) -> bool {
    let Some(registry) = NonNull::new(registry) else {
        return false;
    };
    #[cfg(target_vendor = "apple")]
    let handle = PlatformHandle::TextureRegistry(registry);
    #[cfg(target_os = "windows")]
    let handle = PlatformHandle::PluginRegistrar(registry);
    #[cfg(target_os = "linux")]
    let handle = PlatformHandle::FlEngine(registry);
    // SAFETY: the caller guarantees the object the handle stands for.
    match unsafe { flutter_texture::attach(handle) } {
        Ok(()) => true,
        Err(e) => {
            error!("Failed to attach the Flutter texture registry: {e}");
            false
        }
    }
}

/// Android: hand the engine's texture registry (`flutterEngine.renderer`) over to
/// `flutter_texture`. Called by `MainActivity` on the main thread once the engine is configured;
/// returns whether it worked.
///
/// # Safety
///
/// Called by the JVM, on the main thread, with that thread's `JNIEnv*` and a live reference to an
/// `io.flutter.view.TextureRegistry`.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_ci_not_rune_MainActivity_attachFlutter(
    env: *mut ffi::c_void,
    _activity: *mut ffi::c_void,
    registry: *mut ffi::c_void,
) -> u8 {
    let (Some(env), Some(registry)) = (NonNull::new(env), NonNull::new(registry)) else {
        return 0;
    };
    let handle = PlatformHandle::JniTextureRegistry { env, registry };
    // A panic must not unwind into the JVM.
    // SAFETY: the JVM passes this thread's `JNIEnv` and the registry the activity hands over.
    match panic::catch_unwind(|| unsafe { flutter_texture::attach(handle) }) {
        Ok(Ok(())) => 1,
        Ok(Err(e)) => {
            error!("Failed to attach the Flutter texture registry: {e}");
            0
        }
        Err(_) => {
            error!("Attaching the Flutter texture registry panicked");
            0
        }
    }
}

/// Tell the visualizer what it needs to know about the engine before it starts: on Windows the
/// adapter Flutter renders on, on Linux the GPU and driver of Flutter's OpenGL context (probed on
/// the platform thread the first time), so that frames are shared with it (else they are read
/// back). On Android, the window of the texture (of `width` by `height`) to present into, which
/// is registered now. Applies from the visualizer's next start.
pub async fn prepare_visualizer(
    visualizer: Arc<::visualizer::Visualizer>,
    width: u32,
    height: u32,
) {
    #[cfg(target_os = "android")]
    prepare_window(Arc::clone(&visualizer), width, height).await;
    #[cfg(not(target_os = "android"))]
    let _ = (width, height);
    if TEXTURE_LAYER {
        #[cfg(target_os = "windows")]
        {
            let readback = env::var_os("RUNE_VISUALIZER_READBACK");
            let shared = env::var_os("RUNE_WINDOWS_DXGI_SHARED");
            // Registration can succeed even when Flutter later fails to import
            // the shared surface. Prefer pixels until that path is verified.
            let prefer_readback = readback.as_deref().is_some_and(|value| value == "1")
                || !shared.as_deref().is_some_and(|value| value == "1");
            visualizer.prefer_readback(prefer_readback);
            info!(
                "Windows Flutter visualizer transport preference: {}",
                if prefer_readback {
                    "readback"
                } else {
                    "experimental DXGI shared"
                }
            );
        }
        let capabilities = flutter_texture::capabilities().await;
        visualizer.set_graphics_adapter(capabilities.adapter_luid);
        visualizer.set_graphics_device(capabilities.device_uuids.map(|uuids| DeviceUuids {
            device: uuids.device,
            driver: uuids.driver,
        }));
    }
}

/// Android: make sure there is a texture of `width` by `height` to show the visualizer (the one
/// of a run before, if the visualizer restarts), and give the visualizer its window. Without one,
/// the visualizer has nothing to present into, which shows up in its status.
#[cfg(target_os = "android")]
async fn prepare_window(visualizer: Arc<::visualizer::Visualizer>, width: u32, height: u32) {
    visualizer.set_window(None);
    let visualizer_weak = Arc::downgrade(&visualizer);
    let mut textures = TEXTURES.lock().await;
    if textures.slots.current.is_none() {
        match flutter_texture::register_surface(width, height).await {
            Ok(texture) => install(
                &mut textures,
                Kind::Surface,
                texture,
                visualizer_weak.clone(),
            ),
            Err(e) => {
                visualizer.report_error(
                    "texture",
                    format!("Failed to register the visualizer texture: {e}"),
                );
                return;
            }
        }
    } else if let Some((_, texture, _)) = &textures.slots.current {
        if let Err(e) = texture.set_size(width, height).await {
            visualizer.report_error(
                "texture",
                format!("Failed to resize the visualizer texture: {e}"),
            );
            return;
        }
    }
    // After registering or resizing, set the window from the current texture for both cases.
    // On Android, this is essential: the first start needs its window to present into.
    let Some((_, texture, _)) = &textures.slots.current else {
        return;
    };
    match texture.window().await {
        Ok(window) => visualizer.set_window(Some(Box::new(window))),
        Err(e) => visualizer.report_error(
            "texture",
            format!("The visualizer texture has no window: {e}"),
        ),
    }
}

/// Frames of `width` by `height` from now on. On Android the texture's producer is told the new
/// size, and the visualizer presents into the window it hands out for it.
pub async fn resize_visualizer(
    visualizer: &::visualizer::Visualizer,
    width: u32,
    height: u32,
) -> anyhow::Result<()> {
    #[cfg(target_os = "android")]
    {
        let textures = TEXTURES.lock().await;
        if let Some((_, texture, owner)) = &textures.slots.current
            && owner
                .upgrade()
                .is_some_and(|v| std::ptr::eq(Arc::as_ptr(&v), visualizer))
        {
            texture.set_size(width, height).await?;
            let window = texture.window().await?;
            return visualizer.replace_window(Box::new(window), width, height);
        }
    }
    visualizer.resize(width, height)
}

/// The visualizer's frames, as they are: on the GPU (a pixel buffer, a shared texture or image)
/// or read back. Each acquire takes the next frame in order, so that frames finished within one
/// display refresh are shown on consecutive ones (see [`visualizer::Visualizer::next_frame`]).
struct VisualizerFrames {
    visualizer: Arc<::visualizer::Visualizer>,
}

impl VisualizerFrames {
    fn new(visualizer: Arc<::visualizer::Visualizer>) -> Self {
        Self { visualizer }
    }
}

impl FrameSource for VisualizerFrames {
    fn acquire(&self) -> Option<Acquired> {
        let frame = self.visualizer.next_frame()?;
        #[cfg(target_vendor = "apple")]
        if let Some(buffer) = frame.pixel_buffer() {
            return Some(Acquired {
                id: frame.id,
                frame: Frame::PixelBuffer(buffer),
                keep: Box::new(frame),
            });
        }
        #[cfg(target_os = "windows")]
        if let Some(handle) = frame.shared_handle() {
            return Some(Acquired {
                id: frame.id,
                frame: Frame::Dxgi {
                    handle,
                    width: frame.width,
                    height: frame.height,
                },
                keep: Box::new(frame),
            });
        }
        #[cfg(target_os = "linux")]
        if let Some(image) = frame.shared_image() {
            return Some(Acquired {
                id: frame.id,
                frame: Frame::VulkanImage(flutter_texture::VulkanImage {
                    image_id: image.image_id(),
                    image_set: image.image_set(),
                    memory_size: image.memory_size(),
                    width: frame.width,
                    height: frame.height,
                    fds: Box::new(SharedImage(Arc::clone(&frame))),
                }),
                keep: Box::new(frame),
            });
        }
        let (bytes, stride) = frame.pixels()?;
        // Read back frames are BGRA on Apple and RGBA elsewhere.
        let format = if cfg!(target_vendor = "apple") {
            PixelFormat::Bgra8
        } else {
            PixelFormat::Rgba8
        };
        // SAFETY: the mapped buffer holds the rows and stays mapped and unchanged while the frame
        // is held, which `keep` does.
        let pixels =
            unsafe { Pixels::new(bytes.as_ptr(), frame.width, frame.height, stride, format) };
        Some(Acquired {
            id: frame.id,
            frame: Frame::Pixels(pixels),
            keep: Box::new(frame),
        })
    }

    fn failed(&self, error: TextureError) {
        match error {
            // Linux: the frame in a Vulkan image could not be imported. The same texture shows
            // read-back frames.
            TextureError::SharedFrame(_) => {
                warn!("{error}; the visualizer reads frames back");
                crate::visualizer_log::diagnostic(
                    "transport",
                    format!("{error}; the visualizer reads frames back"),
                );
                self.visualizer.fall_back_to_readback();
            }
            error => self.visualizer.report_error("texture", error.to_string()),
        }
    }
}

/// Exports what OpenGL imports of a frame in a Vulkan image, while the frame is held.
#[cfg(target_os = "linux")]
struct SharedImage(Arc<::visualizer::Frame>);

#[cfg(target_os = "linux")]
impl SharedImage {
    fn export(
        &self,
        export: impl FnOnce(&SharedFrame) -> anyhow::Result<OwnedFd>,
    ) -> Result<OwnedFd, String> {
        let image = self
            .0
            .shared_image()
            .ok_or("the frame is not in a shared image")?;
        export(image).map_err(|e| format!("{e:#}"))
    }
}

#[cfg(target_os = "linux")]
impl flutter_texture::VulkanFds for SharedImage {
    fn export_memory(&self) -> Result<OwnedFd, String> {
        self.export(SharedFrame::export_memory)
    }

    fn export_semaphore(&self) -> Result<OwnedFd, String> {
        self.export(SharedFrame::export_semaphore)
    }
}

/// The texture slots `take_stopping` works on, generic over the texture type so the ownership
/// logic can be tested without a Flutter engine (a `Texture` exists only once the engine
/// registered it).
struct TextureSlots<T> {
    /// The texture Dart is told to show and the kind of frames it takes.
    current: Option<(Kind, T, Weak<::visualizer::Visualizer>)>,
    /// Each with the id of the texture that replaced it and its owning visualizer.
    retiring: Vec<(i64, T, Weak<::visualizer::Visualizer>)>,
}

impl<T> Default for TextureSlots<T> {
    fn default() -> Self {
        Self {
            current: None,
            retiring: Vec::new(),
        }
    }
}

/// The texture Dart is told to show, and the ones it replaced, until Dart shows their
/// replacement.
#[derive(Default)]
struct Textures {
    /// The visualizer runs and may have a texture.
    running: bool,
    /// The texture slots.
    slots: TextureSlots<Texture>,
    /// The id of `current` as last sent to Dart.
    announced: Option<i64>,
}

static TEXTURES: LazyLock<Mutex<Textures>> = LazyLock::new(Mutex::default);

/// The kind of texture that shows frames leaving the GPU by `transport`, or `None` while that is
/// not known and the texture has to wait for it.
fn wanted_kind(transport: Option<Transport>) -> Option<Kind> {
    #[cfg(target_os = "windows")]
    return match transport {
        Some(Transport::DxgiShared) => Some(Kind::Dxgi),
        Some(_) => Some(Kind::Pixels),
        None => None,
    };
    // The texture's window, made before the visualizer started (see `prepare_window`).
    #[cfg(target_os = "android")]
    {
        let _ = transport;
        Some(Kind::Surface)
    }
    // One texture takes all the frames there (on the GPU and read back).
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = transport;
        Some(Kind::Pixels)
    }
}

/// The visualizer started: make sure a texture shows it, as soon as it is known which kind, and
/// tell Dart which one. A texture that cannot be registered shows up in the visualizer's status.
pub async fn visualizer_started(visualizer: Arc<::visualizer::Visualizer>) {
    if !TEXTURE_LAYER {
        return;
    }
    {
        let mut textures = TEXTURES.lock().await;
        textures.running = true;
        // Dart may not have been told about the texture of an earlier run.
        textures.announced = None;
    }
    // The render thread tells how its frames leave the GPU once it is up, and when that changes.
    let runtime = Handle::current();
    let visualizer_weak = Arc::downgrade(&visualizer);
    visualizer.set_transport_listener(Some(Arc::new(move |transport| {
        let Some(visualizer) = visualizer_weak.upgrade() else {
            return;
        };
        let status = visualizer.status();
        crate::visualizer_log::diagnostic(
            "transport",
            format!("Frames leave {} by {transport:?}", status.adapter),
        );
        if let Some(kind) = wanted_kind(Some(transport)) {
            runtime.spawn(async move { show(visualizer, kind).await });
        }
    })));
    // It may know already.
    if let Some(kind) = wanted_kind(visualizer.status().transport) {
        show(visualizer, kind).await;
    }
}

/// Show the visualizer in a texture of `kind`, replacing the one of another kind, and tell Dart.
async fn show(visualizer: Arc<::visualizer::Visualizer>, kind: Kind) {
    let mut textures = TEXTURES.lock().await;
    if !textures.running {
        return;
    }
    // The current texture serves this visualizer only when it is of the kind asked for and this
    // visualizer owns it: reusing another visualizer's texture would attach this one's frame
    // listener to a texture whose frames come from the other one.
    let serves_visualizer =
        textures
            .slots
            .current
            .as_ref()
            .is_some_and(|(current_kind, _, owner)| {
                *current_kind == kind
                    && owner
                        .upgrade()
                        .is_some_and(|v| Arc::ptr_eq(&v, &visualizer))
            });
    if !serves_visualizer {
        // Android: the texture is made with its window before the visualizer starts; without
        // one, the error is in the status already.
        if cfg!(target_os = "android") {
            return;
        }
        let visualizer_weak = Arc::downgrade(&visualizer);
        match flutter_texture::register(
            Arc::new(VisualizerFrames::new(Arc::clone(&visualizer))),
            kind,
        )
        .await
        {
            Ok(texture) => install(&mut textures, kind, texture, visualizer_weak),
            Err(e) if kind != Kind::Pixels => {
                // Not what the visualizer is told to avoid: it reads frames back from now on, and
                // the pixel texture is made when it says so.
                warn!("Failed to register the visualizer's GPU texture, reading frames back: {e}");
                visualizer.fall_back_to_readback();
                return;
            }
            Err(e) => {
                visualizer.report_error(
                    "texture",
                    format!("Failed to register the visualizer texture: {e}"),
                );
                return;
            }
        }
    }
    let Some((_, texture, _)) = &textures.slots.current else {
        return;
    };
    let notifier = texture.notifier();
    visualizer.set_frame_listener(Some(Arc::new(move |_| notifier.mark_frame_available())));
    let id = texture.id();
    if textures.announced.replace(id) != Some(id) {
        VisualizerTextureResponse { texture_id: id }.send_signal_to_dart();
    }
}

/// Make `texture` the one Dart shows, returning the one it replaces. The replaced texture keeps
/// the owner it had: `stop_visualizer` unregisters it when that visualizer stops, even before
/// Dart shows its replacement.
fn replace_current<T>(
    current: &mut Option<(Kind, T, Weak<::visualizer::Visualizer>)>,
    retiring: &mut Vec<(i64, T, Weak<::visualizer::Visualizer>)>,
    kind: Kind,
    id: i64,
    texture: T,
    visualizer_weak: Weak<::visualizer::Visualizer>,
) {
    if let Some((_, old, old_owner)) = current.replace((kind, texture, visualizer_weak)) {
        retiring.push((id, old, old_owner));
    }
}

/// Make `texture` the one Dart shows; the one it replaces goes once Dart shows it.
fn install(
    textures: &mut Textures,
    kind: Kind,
    texture: Texture,
    visualizer_weak: Weak<::visualizer::Visualizer>,
) {
    let id = texture.id();
    if textures.slots.current.is_some() {
        info!("Visualizer texture {id} replaces the one before");
    }
    replace_current(
        &mut textures.slots.current,
        &mut textures.slots.retiring,
        kind,
        id,
        texture,
        visualizer_weak,
    );
}

/// Dart shows the texture `texture_id`: the textures it replaced can go.
pub async fn texture_shown(texture_id: i64) {
    let retired = {
        let mut textures = TEXTURES.lock().await;
        let current = textures
            .slots
            .current
            .as_ref()
            .map(|(_, texture, _)| texture.id());
        // The current texture shown: every earlier one can go, also any that were replaced before
        // Dart showed them.
        let (retired, kept) = mem::take(&mut textures.slots.retiring)
            .into_iter()
            .partition(|(replacement, _, _)| {
                current == Some(texture_id) || *replacement == texture_id
            });
        textures.slots.retiring = kept;
        retired
    };
    for (_, texture, _) in retired {
        texture.unregister().await;
    }
}

/// The textures of the visualizer that stops, taken out of the shared state; the state keeps
/// everything else. The owner check comes before anything is taken: a visualizer that owns no
/// texture leaves the state untouched, so stopping one node's visualizer never takes away
/// another node's texture.
fn take_stopping<T>(
    textures: &mut TextureSlots<T>,
    visualizer: &::visualizer::Visualizer,
) -> Vec<T> {
    let owns = |owner: &Weak<::visualizer::Visualizer>| {
        owner
            .upgrade()
            .is_some_and(|v| std::ptr::eq(Arc::as_ptr(&v), visualizer))
    };
    let mut taken: Vec<T> = Vec::new();
    if textures
        .current
        .as_ref()
        .is_some_and(|(_, _, owner)| owns(owner))
        && let Some((_, texture, _)) = textures.current.take()
    {
        taken.push(texture);
    }
    let (stopping, kept): (Vec<_>, Vec<_>) = mem::take(&mut textures.retiring)
        .into_iter()
        .partition(|(_, _, owner)| owns(owner));
    taken.extend(stopping.into_iter().map(|(_, texture, _)| texture));
    textures.retiring = kept;
    taken
}

/// Stop the visualizer: stop telling the texture about frames, stop the render thread, then
/// unregister its textures. Returns the PCM input, as [`visualizer::Visualizer::stop`] does.
pub async fn stop_visualizer(visualizer: Arc<::visualizer::Visualizer>) -> Option<PcmInput> {
    visualizer.set_transport_listener(None);
    visualizer.set_frame_listener(None);
    let input = visualizer.stop();
    // The window of a texture whose visualizer did not start.
    #[cfg(target_os = "android")]
    visualizer.set_window(None);
    let textures_to_unregister = {
        let mut textures = TEXTURES.lock().await;
        take_stopping(&mut textures.slots, &visualizer)
    };
    for texture in textures_to_unregister {
        texture.unregister().await;
    }
    input
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::visualizer::Visualizer;

    // `Texture` cannot be built in a test: it only exists once it is registered with a running
    // Flutter engine. `take_stopping` is generic over the texture type for that reason, and the
    // tests below use ids in its place.
    fn owned_textures(owners: &[Option<&Arc<Visualizer>>]) -> Vec<(i64, i64, Weak<Visualizer>)> {
        owners
            .iter()
            .enumerate()
            .map(|(index, owner)| {
                (
                    index as i64,
                    index as i64,
                    owner.map_or_else(Weak::default, Arc::downgrade),
                )
            })
            .collect()
    }

    /// Stopping a visualizer that owns nothing touches nothing: another node's texture stays
    /// the current one (the isolation issue #497 is about).
    #[test]
    fn stopping_a_visualizer_keeps_another_ones_textures() {
        let visualizer_a = Arc::new(Visualizer::new());
        let visualizer_b = Arc::new(Visualizer::new());
        let mut slots = TextureSlots {
            current: Some((Kind::Pixels, 1, Weak::clone(&Arc::downgrade(&visualizer_a)))),
            retiring: owned_textures(&[Some(&visualizer_a)]),
        };
        let taken = take_stopping(&mut slots, &visualizer_b);
        assert!(taken.is_empty());
        assert_eq!(
            slots.current.as_ref().map(|(_, texture, _)| *texture),
            Some(1)
        );
        assert_eq!(slots.retiring.len(), 1);
    }

    /// Stopping a visualizer takes its own textures, current and retired, and nothing else.
    #[test]
    fn stopping_a_visualizer_takes_only_its_own_textures() {
        let visualizer_a = Arc::new(Visualizer::new());
        let visualizer_b = Arc::new(Visualizer::new());
        // Retired textures of both owners; a texture that outlived its visualizer (`None`)
        // belongs to no one and is not taken either.
        let mut slots = TextureSlots {
            current: Some((
                Kind::Pixels,
                10,
                Weak::clone(&Arc::downgrade(&visualizer_b)),
            )),
            retiring: vec![
                (10, 8, Arc::downgrade(&visualizer_b)),
                (10, 9, Arc::downgrade(&visualizer_a)),
                (10, 7, Weak::default()),
            ],
        };
        let taken = take_stopping(&mut slots, &visualizer_b);
        assert_eq!(taken, vec![10, 8]);
        assert!(slots.current.is_none());
        assert_eq!(
            slots
                .retiring
                .iter()
                .map(|(_, texture, _)| *texture)
                .collect::<Vec<_>>(),
            vec![9, 7]
        );
    }

    /// A retired texture keeps the owner it had when it was replaced, so stopping that owner
    /// unregisters it even before Dart shows its replacement.
    #[test]
    fn a_replaced_texture_keeps_its_owner() {
        let visualizer_a = Arc::new(Visualizer::new());
        let visualizer_b = Arc::new(Visualizer::new());
        let mut slots: TextureSlots<i64> = TextureSlots::default();
        replace_current(
            &mut slots.current,
            &mut slots.retiring,
            Kind::Pixels,
            1,
            1,
            Arc::downgrade(&visualizer_a),
        );
        replace_current(
            &mut slots.current,
            &mut slots.retiring,
            Kind::Pixels,
            2,
            2,
            Arc::downgrade(&visualizer_b),
        );
        let Some((_, _, retired_owner)) = slots.retiring.first() else {
            panic!("the replaced texture must retire");
        };
        assert!(
            retired_owner
                .upgrade()
                .is_some_and(|v| Arc::ptr_eq(&v, &visualizer_a))
        );
    }

    #[test]
    fn the_transport_decides_the_kind_of_texture_on_windows_only() {
        #[cfg(target_os = "windows")]
        {
            assert_eq!(wanted_kind(Some(Transport::Readback)), Some(Kind::Pixels));
            assert_eq!(wanted_kind(Some(Transport::DxgiShared)), Some(Kind::Dxgi));
            assert_eq!(wanted_kind(None), None);
        }
        #[cfg(target_os = "android")]
        {
            // Frames read back would show nowhere: the texture's window takes them all.
            assert_eq!(wanted_kind(Some(Transport::Readback)), Some(Kind::Surface));
            assert_eq!(wanted_kind(None), Some(Kind::Surface));
        }
        #[cfg(not(any(target_os = "windows", target_os = "android")))]
        {
            assert_eq!(wanted_kind(Some(Transport::Readback)), Some(Kind::Pixels));
            assert_eq!(wanted_kind(None), Some(Kind::Pixels));
        }
    }
}
