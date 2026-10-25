//! The visualizer's frames in a Flutter texture of our own texture layer (`flutter_texture`).
//!
//! The runner hands the engine over at launch ([`rune_flutter_attach`]); when the visualizer runs,
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

use std::sync::{Arc, LazyLock};

use log::{error, info, warn};
use rinf::RustSignal;
use tokio::sync::Mutex;

use ::flutter_texture::{
    Acquired, Frame, FrameSource, Kind, PixelFormat, Pixels, Texture, TextureError,
};
use ::visualizer::{DeviceUuids, PcmInput, Transport};

use crate::messages::VisualizerTextureResponse;
use crate::visualizer::VISUALIZER;

/// Whether the visualizer is shown through `flutter_texture` here (else not at all).
const TEXTURE_LAYER: bool = cfg!(any(
    target_vendor = "apple",
    target_os = "windows",
    target_os = "linux"
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
pub unsafe extern "C" fn rune_flutter_attach(registry: *mut std::ffi::c_void) -> bool {
    let Some(registry) = std::ptr::NonNull::new(registry) else {
        return false;
    };
    #[cfg(target_vendor = "apple")]
    let handle = flutter_texture::PlatformHandle::TextureRegistry(registry);
    #[cfg(target_os = "windows")]
    let handle = flutter_texture::PlatformHandle::PluginRegistrar(registry);
    #[cfg(target_os = "linux")]
    let handle = flutter_texture::PlatformHandle::FlEngine(registry);
    // SAFETY: the caller guarantees the object the handle stands for.
    match unsafe { flutter_texture::attach(handle) } {
        Ok(()) => true,
        Err(e) => {
            error!("Failed to attach the Flutter texture registry: {e}");
            false
        }
    }
}

/// Tell the visualizer what it needs to know about the engine before it starts: on Windows the
/// adapter Flutter renders on, on Linux the GPU and driver of Flutter's OpenGL context (probed on
/// the platform thread the first time), so that frames are shared with it (else they are read
/// back). Applies from the visualizer's next start.
pub async fn prepare_visualizer() {
    if TEXTURE_LAYER {
        let capabilities = flutter_texture::capabilities().await;
        VISUALIZER.set_graphics_adapter(capabilities.adapter_luid);
        VISUALIZER.set_graphics_device(capabilities.device_uuids.map(|uuids| DeviceUuids {
            device: uuids.device,
            driver: uuids.driver,
        }));
    }
}

/// The visualizer's latest frame, as it is: on the GPU (a pixel buffer, a shared texture or image)
/// or read back.
struct VisualizerFrames;

impl FrameSource for VisualizerFrames {
    fn acquire(&self) -> Option<Acquired> {
        let frame = VISUALIZER.latest_frame()?;
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
                VISUALIZER.fall_back_to_readback();
            }
            error => VISUALIZER.report_error(error.to_string()),
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
        export: impl FnOnce(&::visualizer::SharedFrame) -> anyhow::Result<std::os::fd::OwnedFd>,
    ) -> Result<std::os::fd::OwnedFd, String> {
        let image = self
            .0
            .shared_image()
            .ok_or("the frame is not in a shared image")?;
        export(image).map_err(|e| format!("{e:#}"))
    }
}

#[cfg(target_os = "linux")]
impl flutter_texture::VulkanFds for SharedImage {
    fn export_memory(&self) -> Result<std::os::fd::OwnedFd, String> {
        self.export(::visualizer::SharedFrame::export_memory)
    }

    fn export_semaphore(&self) -> Result<std::os::fd::OwnedFd, String> {
        self.export(::visualizer::SharedFrame::export_semaphore)
    }
}

/// The texture Dart is told to show, and the ones it replaced, until Dart shows their
/// replacement.
#[derive(Default)]
struct Textures {
    /// The visualizer runs and may have a texture.
    running: bool,
    /// The texture Dart is told to show and the kind of frames it takes.
    current: Option<(Kind, Texture)>,
    /// The id of `current` as last sent to Dart.
    announced: Option<i64>,
    /// Each with the id of the texture that replaced it.
    retiring: Vec<(i64, Texture)>,
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
    // One texture takes all the frames there (on the GPU and read back).
    #[cfg(not(target_os = "windows"))]
    {
        let _ = transport;
        Some(Kind::Pixels)
    }
}

/// The visualizer started: make sure a texture shows it, as soon as it is known which kind, and
/// tell Dart which one. A texture that cannot be registered shows up in the visualizer's status.
pub async fn visualizer_started() {
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
    let runtime = tokio::runtime::Handle::current();
    VISUALIZER.set_transport_listener(Some(Arc::new(move |transport| {
        if let Some(kind) = wanted_kind(Some(transport)) {
            runtime.spawn(show(kind));
        }
    })));
    // It may know already.
    if let Some(kind) = wanted_kind(VISUALIZER.status().transport) {
        show(kind).await;
    }
}

/// Show the visualizer in a texture of `kind`, replacing the one of another kind, and tell Dart.
async fn show(kind: Kind) {
    let mut textures = TEXTURES.lock().await;
    if !textures.running {
        return;
    }
    if textures.current.as_ref().map(|(current, _)| *current) != Some(kind) {
        match flutter_texture::register(Arc::new(VisualizerFrames), kind).await {
            Ok(texture) => install(&mut textures, kind, texture),
            Err(e) if kind != Kind::Pixels => {
                // Not what the visualizer is told to avoid: it reads frames back from now on, and
                // the pixel texture is made when it says so.
                warn!("Failed to register the visualizer's GPU texture, reading frames back: {e}");
                VISUALIZER.fall_back_to_readback();
                return;
            }
            Err(e) => {
                VISUALIZER.report_error(format!("Failed to register the visualizer texture: {e}"));
                return;
            }
        }
    }
    let Some((_, texture)) = &textures.current else {
        return;
    };
    let notifier = texture.notifier();
    VISUALIZER.set_frame_listener(Some(Arc::new(move |_| notifier.mark_frame_available())));
    let id = texture.id();
    if textures.announced.replace(id) != Some(id) {
        VisualizerTextureResponse { texture_id: id }.send_signal_to_dart();
    }
}

/// Make `texture` the one Dart shows; the one it replaces goes once Dart shows it.
fn install(textures: &mut Textures, kind: Kind, texture: Texture) {
    let id = texture.id();
    if let Some((_, old)) = textures.current.replace((kind, texture)) {
        info!("Visualizer texture {} replaces {}", id, old.id());
        textures.retiring.push((id, old));
    }
}

/// Dart shows the texture `texture_id`: the textures it replaced can go.
pub async fn texture_shown(texture_id: i64) {
    let retired = {
        let mut textures = TEXTURES.lock().await;
        let current = textures.current.as_ref().map(|(_, texture)| texture.id());
        // The current texture shown: every earlier one can go, also any that were replaced before
        // Dart showed them.
        let (retired, kept) = std::mem::take(&mut textures.retiring)
            .into_iter()
            .partition(|(replacement, _)| {
                current == Some(texture_id) || *replacement == texture_id
            });
        textures.retiring = kept;
        retired
    };
    for (_, texture) in retired {
        texture.unregister().await;
    }
}

/// Stop the visualizer: stop telling the texture about frames, stop the render thread, then
/// unregister the textures. Returns the PCM input, as [`visualizer::Visualizer::stop`] does.
pub async fn stop_visualizer() -> Option<PcmInput> {
    VISUALIZER.set_transport_listener(None);
    VISUALIZER.set_frame_listener(None);
    let input = VISUALIZER.stop();
    let textures = std::mem::take(&mut *TEXTURES.lock().await);
    for texture in textures
        .current
        .into_iter()
        .map(|(_, texture)| texture)
        .chain(textures.retiring.into_iter().map(|(_, texture)| texture))
    {
        texture.unregister().await;
    }
    input
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_transport_decides_the_kind_of_texture_on_windows_only() {
        assert_eq!(wanted_kind(Some(Transport::Readback)), Some(Kind::Pixels));
        #[cfg(target_os = "windows")]
        {
            assert_eq!(wanted_kind(Some(Transport::DxgiShared)), Some(Kind::Dxgi));
            assert_eq!(wanted_kind(None), None);
        }
        #[cfg(not(target_os = "windows"))]
        assert_eq!(wanted_kind(None), Some(Kind::Pixels));
    }
}
