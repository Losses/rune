//! Android: frames presented into the native window of a Flutter texture (a `SurfaceProducer` of
//! `flutter-texture`) through a Vulkan swapchain. The swapchain's images are the buffers of the
//! producer's `ImageReader` (an `AHardwareBuffer` each), which Impeller samples without a copy,
//! so nothing is read back or handed over: a frame is on its way to Flutter once it is presented.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use log::{info, warn};

use crate::pipeline::FORMAT;

/// A native window to present frames into (Android: a Flutter texture's `ANativeWindow`). wgpu
/// holds it as long as the surface made from it, and lets go of it after the surface.
pub type Window = Box<dyn wgpu::DisplayAndWindowHandle>;

/// A wgpu surface of a [`Window`] and how it is configured.
pub(crate) struct WindowSurface {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    /// `None` once the surface was lost, or a new window could not be made into one, until the
    /// next window.
    surface: Option<wgpu::Surface<'static>>,
    config: wgpu::SurfaceConfiguration,
    /// Errors for the status.
    report: Arc<dyn Fn(String) + Send + Sync>,
}

/// A surface of `window`, which the surface holds from now on.
pub(crate) fn create_surface(
    instance: &wgpu::Instance,
    window: Window,
) -> Result<wgpu::Surface<'static>> {
    // wgpu-hal's Vulkan backend panics when the driver cannot make the surface.
    catch_unwind(AssertUnwindSafe(|| {
        instance.create_surface(wgpu::SurfaceTarget::DisplayAndWindow(window))
    }))
    .map_err(|_| anyhow!("Vulkan could not make a surface of the window"))?
    .context("Failed to make a surface of the window")
}

/// What became of an attempt to get the next image of the swapchain.
enum Next {
    Image(wgpu::SurfaceTexture),
    /// No image this time (every image is in use, or the window does not show): skip the frame.
    Skip,
    /// The swapchain no longer matches the window: configure it again and retry.
    Reconfigure,
}

impl WindowSurface {
    /// Configure `surface` for frames of `width` by `height`, presented in turn as the window
    /// takes them. `adapter` and `device` must be compatible with the surface.
    pub(crate) fn new(
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        surface: wgpu::Surface<'static>,
        (width, height): (u32, u32),
        report: Arc<dyn Fn(String) + Send + Sync>,
    ) -> Result<Self> {
        let caps = surface.get_capabilities(adapter);
        let format = choose_format(&caps.formats).context("The window takes no frames")?;
        // Mailbox replaces a frame the window has not taken yet, so the render thread keeps its
        // own pace; Fifo, which every window has, waits for the window instead.
        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            wgpu::CompositeAlphaMode::Auto
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width,
            height,
            desired_maximum_frame_latency: 2,
            present_mode,
            alpha_mode,
            view_formats: Vec::new(),
        };
        info!("Presenting {format:?} frames ({present_mode:?}, {alpha_mode:?})");
        surface.configure(device, &config);
        Ok(Self {
            instance: instance.clone(),
            adapter: adapter.clone(),
            surface: Some(surface),
            config,
            report,
        })
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// The format of the swapchain's images, which scenes draw in.
    pub(crate) fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// Frames of `width` by `height` from now on, into the same window.
    pub(crate) fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.config.width = width;
        self.config.height = height;
        if let Some(surface) = &self.surface {
            surface.configure(device, &self.config);
        }
    }

    /// Present frames of `width` by `height` into `window` from now on, in the same format. The
    /// surface of the window before goes first (and with it that window), also if the new one
    /// fails: a window takes frames from one swapchain at a time.
    pub(crate) fn replace(
        &mut self,
        device: &wgpu::Device,
        window: Window,
        width: u32,
        height: u32,
    ) -> Result<()> {
        self.surface = None;
        self.config.width = width;
        self.config.height = height;
        let surface = create_surface(&self.instance, window)?;
        if !self.adapter.is_surface_supported(&surface) {
            bail!("The GPU cannot present into the new window");
        }
        let caps = surface.get_capabilities(&self.adapter);
        if !caps.formats.contains(&self.config.format) {
            bail!("The new window takes no {:?} frames", self.config.format);
        }
        surface.configure(device, &self.config);
        self.surface = Some(surface);
        Ok(())
    }

    /// Draw a frame with `paint` into the next image of the swapchain and present it; whether
    /// there was an image to draw into.
    pub(crate) fn present(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        paint: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView),
    ) -> bool {
        let Some(image) = self.next_image(device) else {
            return false;
        };
        let view = image.texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        paint(&mut encoder, &view);
        queue.submit(Some(encoder.finish()));
        queue.present(image);
        true
    }

    /// The next image of the swapchain, configuring it again once if it no longer matches the
    /// window. A surface that is lost even then is let go of until the next window.
    fn next_image(&mut self, device: &wgpu::Device) -> Option<wgpu::SurfaceTexture> {
        let surface = self.surface.as_ref()?;
        for retry in [false, true] {
            match Self::acquire(surface) {
                Next::Image(image) => return Some(image),
                Next::Skip => return None,
                Next::Reconfigure if !retry => surface.configure(device, &self.config),
                Next::Reconfigure => break,
            }
        }
        warn!("The visualizer's window was lost");
        (self.report)("The window the visualizer draws into was lost".to_string());
        self.surface = None;
        None
    }

    fn acquire(surface: &wgpu::Surface) -> Next {
        match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(image) => Next::Image(image),
            // Still shows right, only less efficiently; a resize configures the swapchain again.
            wgpu::CurrentSurfaceTexture::Suboptimal(image) => Next::Image(image),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                Next::Reconfigure
            }
            // A validation error went to the device's error handler (the status).
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => Next::Skip,
        }
    }
}

/// [`FORMAT`] if the window takes it, else another linear 8-bit format, else any linear one, else
/// the first: scenes write the bytes they mean, so a format that encodes sRGB would show them
/// too bright.
fn choose_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    [FORMAT, wgpu::TextureFormat::Bgra8Unorm]
        .into_iter()
        .find(|format| formats.contains(format))
        .or_else(|| formats.iter().copied().find(|format| !format.is_srgb()))
        .or_else(|| formats.first().copied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::TextureFormat::*;

    #[test]
    fn the_scene_format_comes_first_then_a_linear_one() {
        assert_eq!(
            choose_format(&[Rgba8UnormSrgb, Bgra8Unorm, Rgba8Unorm]),
            Some(Rgba8Unorm)
        );
        assert_eq!(
            choose_format(&[Rgba8UnormSrgb, Bgra8Unorm]),
            Some(Bgra8Unorm)
        );
        assert_eq!(
            choose_format(&[Rgba8UnormSrgb, Rgb10a2Unorm]),
            Some(Rgb10a2Unorm)
        );
        assert_eq!(choose_format(&[Rgba8UnormSrgb]), Some(Rgba8UnormSrgb));
        assert_eq!(choose_format(&[]), None);
    }
}
