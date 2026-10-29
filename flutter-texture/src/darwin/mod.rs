//! Apple: textures of a `FlutterTextureRegistry` (the `FlutterEngine` on macOS, a registrar's on iOS).
//!
//! The registry is messaged on the main thread only, as the engine requires (`registerTexture:`,
//! `unregisterTexture:`, and `textureFrameAvailable:`, which ends in a check for the platform
//! thread). The texture object it is given ([`texture::RuneTexture`]) answers `copyPixelBuffer`
//! on the raster thread.

mod pixels;
mod texture;

use std::sync::Arc;

use log::error;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, msg_send};

use crate::thread::{self, MainThreadOnly};
use crate::{DeviceUuids, FrameNotifier, FrameSource, Kind, PlatformHandle, Result, TextureError};
use texture::RuneTexture;

/// The engine's texture registry, retained.
pub(crate) struct Registry(MainThreadOnly<Retained<AnyObject>>);

impl Registry {
    /// # Safety
    ///
    /// The handle must point to a live Objective-C object that answers `FlutterTextureRegistry`'s
    /// messages.
    pub(crate) unsafe fn new(handle: PlatformHandle) -> Result<Self> {
        match handle {
            PlatformHandle::TextureRegistry(pointer) => {
                // SAFETY: the caller guarantees a live object.
                let object = unsafe { Retained::retain(pointer.as_ptr().cast::<AnyObject>()) }
                    .ok_or(TextureError::InvalidHandle)?;
                Ok(Self(MainThreadOnly::new(object)))
            }
        }
    }

    /// Only Windows tells which adapter the engine renders on.
    pub(crate) fn adapter_luid(&self) -> Option<(u32, i32)> {
        None
    }

    /// Only Linux tells which GPU the engine renders on by its UUIDs.
    pub(crate) async fn device_uuids(&self) -> Option<DeviceUuids> {
        None
    }

    fn object(&self, mtm: MainThreadMarker) -> &AnyObject {
        self.0.get(mtm)
    }
}

/// A texture as the registry knows it.
pub(crate) struct Registered {
    registry: Arc<Registry>,
    id: i64,
    /// Our reference; the engine holds its own while it may call the object.
    object: Retained<RuneTexture>,
}

impl Registered {
    pub(crate) fn id(&self) -> i64 {
        self.id
    }

    pub(crate) fn notifier(&self) -> FrameNotifier {
        let registry = Arc::clone(&self.registry);
        let id = self.id;
        FrameNotifier::new(
            |task| thread::run(move |_| task()),
            move || {
                // Posted to the main queue, so this is the main thread.
                let Some(mtm) = MainThreadMarker::new() else {
                    return;
                };
                // SAFETY: `textureFrameAvailable:` takes an `int64_t` and returns nothing.
                let _: () = unsafe { msg_send![registry.object(mtm), textureFrameAvailable: id] };
            },
        )
    }

    pub(crate) async fn unregister(self) {
        if thread::call(move |mtm| self.unregister_now(mtm))
            .await
            .is_err()
        {
            error!("The platform thread did not unregister a Flutter texture");
        }
    }

    pub(crate) fn unregister_later(self) {
        thread::run(move |mtm| self.unregister_now(mtm));
    }

    fn unregister_now(self, mtm: MainThreadMarker) {
        // SAFETY: `unregisterTexture:` takes an `int64_t` and returns nothing.
        let _: () = unsafe { msg_send![self.registry.object(mtm), unregisterTexture: self.id] };
        drop(self.object);
    }
}

pub(crate) async fn register(
    registry: Arc<Registry>,
    source: Arc<dyn FrameSource>,
    // One texture takes both kinds of frames here.
    _kind: Kind,
) -> Result<Registered> {
    thread::call(move |mtm| {
        let object = RuneTexture::new(source);
        // SAFETY: `registerTexture:` takes an object that answers `copyPixelBuffer` and returns
        // an `int64_t`. The engine retains the object.
        let id: i64 = unsafe { msg_send![registry.object(mtm), registerTexture: &*object] };
        Registered {
            registry,
            id,
            object,
        }
    })
    .await
}
