//! The visualizer's frames in a Flutter texture of our own texture layer (`flutter_texture`).
//!
//! On macOS the runner hands the engine over at launch ([`rune_flutter_attach`]); when the
//! visualizer starts, a texture showing its latest frame is registered and its id is sent to Dart
//! (`VisualizerTextureResponse`). A texture that replaces another one is sent the same way; the
//! old one is unregistered once Dart says it shows the new one (`VisualizerTextureShownRequest`).
//!
//! Windows and Linux still have their own texture bridges in the runner, which pull frames through
//! the C interface in [`crate::visualizer`].

use std::sync::{Arc, LazyLock};

use log::{error, info};
use rinf::RustSignal;
use tokio::sync::Mutex;

use ::flutter_texture::{Acquired, Frame, FrameSource, PixelFormat, Pixels, Texture, TextureError};
use ::visualizer::PcmInput;

use crate::messages::VisualizerTextureResponse;
use crate::visualizer::VISUALIZER;

/// Whether the visualizer is shown through `flutter_texture` here (else through the runner's own
/// bridge).
const TEXTURE_LAYER: bool = cfg!(target_os = "macos");

/// Hand the engine's texture registry over to `flutter_texture`. Called by the runner once the
/// engine exists; returns whether it worked.
///
/// # Safety
///
/// `registry` must be null or point to the `FlutterEngine`.
#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rune_flutter_attach(registry: *mut std::ffi::c_void) -> bool {
    let Some(registry) = std::ptr::NonNull::new(registry) else {
        return false;
    };
    // SAFETY: the caller guarantees the engine, which answers `FlutterTextureRegistry`.
    let attached = unsafe {
        flutter_texture::attach(flutter_texture::PlatformHandle::TextureRegistry(registry))
    };
    match attached {
        Ok(()) => true,
        Err(e) => {
            error!("Failed to attach the Flutter texture registry: {e}");
            false
        }
    }
}

/// The visualizer's latest frame, as it is: in a pixel buffer or read back.
struct VisualizerFrames;

impl FrameSource for VisualizerFrames {
    fn acquire(&self) -> Option<Acquired> {
        let frame = VISUALIZER.latest_frame()?;
        #[cfg(target_os = "macos")]
        if let Some(buffer) = frame.pixel_buffer() {
            return Some(Acquired {
                id: frame.id,
                frame: Frame::PixelBuffer(buffer),
                keep: Box::new(frame),
            });
        }
        let (bytes, stride) = frame.pixels()?;
        // Read back frames are BGRA on macOS and RGBA elsewhere.
        let format = if cfg!(target_os = "macos") {
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
        VISUALIZER.report_error(error.to_string());
    }
}

/// The texture Dart is told to show, and the ones it replaced, until Dart shows their
/// replacement.
#[derive(Default)]
struct Textures {
    current: Option<Texture>,
    /// Each with the id of the texture that replaced it.
    retiring: Vec<(i64, Texture)>,
}

static TEXTURES: LazyLock<Mutex<Textures>> = LazyLock::new(Mutex::default);

/// The visualizer started: make sure a texture shows it and tell Dart which one. A texture that
/// cannot be registered shows up in the visualizer's status.
pub async fn visualizer_started() {
    if !TEXTURE_LAYER {
        return;
    }
    let mut textures = TEXTURES.lock().await;
    if textures.current.is_none() {
        match flutter_texture::register(Arc::new(VisualizerFrames)).await {
            Ok(texture) => install(&mut textures, texture),
            Err(e) => {
                VISUALIZER.report_error(format!("Failed to register the visualizer texture: {e}"));
                return;
            }
        }
    }
    let Some(texture) = &textures.current else {
        return;
    };
    let notifier = texture.notifier();
    VISUALIZER.set_frame_listener(Some(Arc::new(move |_| notifier.mark_frame_available())));
    VisualizerTextureResponse {
        texture_id: texture.id(),
    }
    .send_signal_to_dart();
}

/// Make `texture` the one Dart shows; the one it replaces goes once Dart shows it.
fn install(textures: &mut Textures, texture: Texture) {
    let id = texture.id();
    if let Some(old) = textures.current.replace(texture) {
        info!("Visualizer texture {} replaces {}", id, old.id());
        textures.retiring.push((id, old));
    }
}

/// Dart shows the texture `texture_id`: the textures it replaced can go.
pub async fn texture_shown(texture_id: i64) {
    let retired = {
        let mut textures = TEXTURES.lock().await;
        let current = textures.current.as_ref().map(Texture::id);
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
    VISUALIZER.set_frame_listener(None);
    let input = VISUALIZER.stop();
    let textures = std::mem::take(&mut *TEXTURES.lock().await);
    for texture in textures
        .current
        .into_iter()
        .chain(textures.retiring.into_iter().map(|(_, texture)| texture))
    {
        texture.unregister().await;
    }
    input
}
