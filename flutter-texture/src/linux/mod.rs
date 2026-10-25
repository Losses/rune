//! Linux: textures of the `FlEngine`'s `FlTextureRegistrar`, whose functions are looked up in
//! the process (see [`api`]).
//!
//! The registrar has no thread guarantees: register, unregister and mark run on the GLib main
//! context, where the engine wants them (marking ends in a check for the platform thread). The
//! texture object it is given ([`texture`]) is an `FlTextureGL` that takes frames in Vulkan images
//! and frames of pixels alike, so a producer that falls back from one to the other keeps its
//! texture id. Which GPU frames may come from is read from a context like Flutter's ([`gl::probe`]).
//!
//! The registrar looks textures up for the raster thread without taking a reference, so after
//! unregistering a texture its object is kept for a while ([`GRACE`]) before it is let go.

pub(crate) mod api;
mod gl;
mod logic;
mod texture;

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use log::{error, info};

use crate::thread::{self, MainContext};
use crate::{DeviceUuids, FrameNotifier, FrameSource, Kind, PlatformHandle, Result, TextureError};
use api::{Api, Gboolean, Gpointer, PRIORITY_DEFAULT, SOURCE_REMOVE};
use texture::Raster;

use api::api;

/// How long an unregistered texture's object is kept: the raster thread may still be inside its
/// populate.
const GRACE: Duration = Duration::from_secs(1);

/// Set once OpenGL failed to import a frame in a Vulkan image: from then on frames are read back
/// (no device UUIDs are given out).
static SHARING_FAILED: AtomicBool = AtomicBool::new(false);

/// The engine's texture registrar, retained.
pub(crate) struct Registry {
    api: &'static Api,
    /// The `FlTextureRegistrar*`, our reference.
    registrar: usize,
    /// The probe of Flutter's OpenGL context, once made: the GPU and driver do not change while
    /// the app runs.
    device_uuids: OnceLock<Option<DeviceUuids>>,
}

impl Registry {
    /// # Safety
    ///
    /// The handle must be a live `FlEngine*`.
    pub(crate) unsafe fn new(handle: PlatformHandle) -> Result<Self> {
        let PlatformHandle::FlEngine(engine) = handle;
        let api = api()?;
        let engine: NonNull<c_void> = engine;
        // SAFETY: the caller guarantees a live engine, whose registrar it owns; we take a
        // reference of our own.
        let registrar = unsafe { (api.engine_get_texture_registrar)(engine.as_ptr()) };
        if registrar.is_null() {
            return Err(TextureError::InvalidHandle);
        }
        // SAFETY: a live object.
        unsafe { (api.object_ref)(registrar) };
        Ok(Self {
            api,
            registrar: registrar as usize,
            device_uuids: OnceLock::new(),
        })
    }

    /// Only Windows tells which adapter the engine renders on.
    pub(crate) fn adapter_luid(&self) -> Option<(u32, i32)> {
        None
    }

    /// The GPU and driver of Flutter's OpenGL context, if frames in Vulkan images can be shown.
    /// The first call probes on the platform thread.
    pub(crate) async fn device_uuids(&self) -> Option<DeviceUuids> {
        if SHARING_FAILED.load(Ordering::Relaxed) {
            return None;
        }
        if let Some(uuids) = self.device_uuids.get() {
            return *uuids;
        }
        let api = self.api;
        match thread::call(move |context| gl::probe(api, context)).await {
            Ok(uuids) => {
                info!(
                    "Flutter texture: Flutter's OpenGL context takes Vulkan frames: {}",
                    uuids.is_some()
                );
                *self.device_uuids.get_or_init(|| uuids)
            }
            Err(e) => {
                error!("Failed to probe Flutter's OpenGL context: {e}");
                None
            }
        }
    }

    fn registrar(&self) -> Gpointer {
        self.registrar as Gpointer
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        let (api, registrar) = (self.api, self.registrar);
        // On the main context: the last reference disposes the registrar.
        thread::run(move |_| {
            // SAFETY: our reference, given up once.
            unsafe { (api.object_unref)(registrar as Gpointer) };
        });
    }
}

// SAFETY: the registrar is only used on the main context (see `thread`), apart from taking and
// giving up references, which GObject does atomically.
unsafe impl Send for Registry {}
// SAFETY: as above.
unsafe impl Sync for Registry {}

/// A texture as the registrar knows it.
pub(crate) struct Registered {
    registry: Arc<Registry>,
    id: i64,
    /// The texture object, our reference; the registrar holds its own while it is registered.
    object: usize,
    raster: Arc<Raster>,
}

impl Registered {
    pub(crate) fn id(&self) -> i64 {
        self.id
    }

    pub(crate) fn notifier(&self) -> FrameNotifier {
        let registry = Arc::clone(&self.registry);
        let object = self.object;
        FrameNotifier::new(
            |task| thread::run(move |_| task()),
            move || {
                // Posted to the main context. The object is alive (ours), registered or not.
                // SAFETY: a live registrar and texture.
                unsafe {
                    (registry.api.texture_registrar_mark_texture_frame_available)(
                        registry.registrar(),
                        object as Gpointer,
                    )
                };
            },
        )
    }

    pub(crate) async fn unregister(self) {
        if thread::call(move |context| self.unregister_now(context))
            .await
            .is_err()
        {
            error!("The platform thread did not unregister a Flutter texture");
        }
    }

    pub(crate) fn unregister_later(self) {
        thread::run(move |context| self.unregister_now(context));
    }

    fn unregister_now(self, _context: MainContext) {
        let api = self.registry.api;
        // SAFETY: a live registrar and texture, on the main context. Once the engine is gone this
        // does nothing.
        unsafe {
            (api.texture_registrar_unregister_texture)(
                self.registry.registrar(),
                self.object as Gpointer,
            )
        };
        self.raster.retire();
        release_later(api, self.object as Gpointer);
    }
}

/// Give up our reference to `object` after [`GRACE`], from the main context.
fn release_later(api: &'static Api, object: Gpointer) {
    /// The timeout's function.
    ///
    /// # Safety
    ///
    /// `object` is the reference `release_later` was given.
    unsafe extern "C" fn release(object: Gpointer) -> Gboolean {
        if let Ok(api) = api::api() {
            // SAFETY: as the caller guarantees, given up once.
            unsafe { (api.object_unref)(object) };
        }
        SOURCE_REMOVE
    }
    // SAFETY: the source runs `release` once with the object.
    unsafe {
        (api.timeout_add_full)(
            PRIORITY_DEFAULT,
            GRACE.as_millis() as u32,
            release,
            object,
            None,
        )
    };
}

pub(crate) async fn register(
    registry: Arc<Registry>,
    source: Arc<dyn FrameSource>,
    // One texture takes both kinds of frames here.
    _kind: Kind,
) -> Result<Registered> {
    thread::call(move |_| {
        let api = registry.api;
        let raster = Arc::new(Raster::new(source));
        let object = texture::new_object(api, Arc::clone(&raster)).map_err(TextureError::Frame)?;
        // SAFETY: a live registrar and texture, on the main context. The registrar takes its own
        // reference.
        let registered =
            unsafe { (api.texture_registrar_register_texture)(registry.registrar(), object) };
        if registered == 0 {
            // SAFETY: our reference, given up once; the texture was never registered.
            unsafe { (api.object_unref)(object) };
            return Err(TextureError::Frame(
                "the engine did not register the texture (it is gone)".into(),
            ));
        }
        // SAFETY: a live texture.
        let id = unsafe { (api.texture_get_id)(object) };
        Ok(Registered {
            registry,
            id,
            object: object as usize,
            raster,
        })
    })
    .await?
}
