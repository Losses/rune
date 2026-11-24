//! Linux: textures of the `FlEngine`'s `FlTextureRegistrar`, whose functions are looked up in
//! the process (see [`api`]).
//!
//! The registrar has no thread guarantees: register, unregister and mark run on the GLib main
//! context, where the engine wants them (marking ends in a check for the platform thread). The
//! texture object it is given ([`texture`]) is an `FlTextureGL` that takes frames in Vulkan images
//! and frames of pixels alike, so a producer that falls back from one to the other keeps its
//! texture id. Which GPU frames may come from is read from a context like Flutter's ([`gl::probe`]).
//!
//! The registrar looks textures up for the raster thread without taking a reference. An
//! unregistered texture's object is therefore kept in a retired list until a populate has run on
//! the raster thread: that populate is ordered after every callback that could still hold the
//! object. The next populate releases the retired objects, and the `Raster` it is using stays
//! alive through an `Arc` clone until the callback returns. If no populate runs again, the
//! objects stay until the process exits: they are small, and the raster thread is the only place
//! that can prove it is done with them.

pub(crate) mod api;
mod gl;
mod logic;
mod texture;

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use log::{error, info};

use crate::thread::{self, MainContext};
use crate::{
    DeviceUuids, FrameNotifier, FrameSource, Kind, PlatformHandle, Result, TextureError, lock,
};
use api::{Api, Gpointer};
use texture::Raster;

use api::api;

/// Set once OpenGL failed to import a frame in a Vulkan image: from then on frames are read back
/// (no device UUIDs are given out).
static SHARING_FAILED: AtomicBool = AtomicBool::new(false);

/// The objects of unregistered textures that the raster thread may still use. A populate takes
/// them out and lets them go once it has finished: the raster thread has then passed every
/// callback that could have looked one up.
#[derive(Default)]
pub(super) struct Retired {
    objects: Mutex<Vec<ObjectRef>>,
}

impl Retired {
    fn add(&self, object: ObjectRef) {
        lock(&self.objects).push(object);
    }

    /// Let go of the objects: the raster thread has finished the populate that was using them.
    /// Called on the raster thread.
    pub(super) fn release(&self) {
        let objects = std::mem::take(&mut *lock(&self.objects));
        drop(objects);
    }
}

/// Our reference to a texture object that the raster thread may still use.
struct ObjectRef {
    object: usize,
    /// `g_object_unref`, the reference `new_object` made.
    release: unsafe extern "C" fn(Gpointer),
}

impl ObjectRef {
    fn new(api: &'static Api, object: Gpointer) -> Self {
        Self {
            object: object as usize,
            release: api.object_unref,
        }
    }

    fn as_usize(&self) -> usize {
        self.object
    }

    fn as_ptr(&self) -> Gpointer {
        self.object as Gpointer
    }

    #[cfg(test)]
    fn for_test(object: usize, release: unsafe extern "C" fn(Gpointer)) -> Self {
        Self { object, release }
    }
}

impl Drop for ObjectRef {
    fn drop(&mut self) {
        // SAFETY: the reference `new_object` made, given up once.
        unsafe { (self.release)(self.as_ptr()) }
    }
}

/// The engine's texture registrar, retained.
pub(crate) struct Registry {
    api: &'static Api,
    /// The `FlTextureRegistrar*`, our reference.
    registrar: usize,
    /// The probe of Flutter's OpenGL context, once made: the GPU and driver do not change while
    /// the app runs.
    device_uuids: OnceLock<Option<DeviceUuids>>,
    /// The objects of unregistered textures of this engine, until its raster thread has finished
    /// the callback that may use them.
    retired: Arc<Retired>,
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
            retired: Arc::new(Retired::default()),
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
    object: ObjectRef,
    raster: Arc<Raster>,
}

impl Registered {
    pub(crate) fn id(&self) -> i64 {
        self.id
    }

    pub(crate) fn notifier(&self) -> FrameNotifier {
        let registry = Arc::clone(&self.registry);
        let object = self.object.as_usize();
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
        let object = self.object;
        // SAFETY: a live registrar and texture, on the main context. Once the engine is gone this
        // does nothing.
        unsafe {
            (api.texture_registrar_unregister_texture)(self.registry.registrar(), object.as_ptr())
        };
        self.raster.retire();
        // The raster thread may still be inside a callback that looked the object up. Keep it
        // until a populate has finished there.
        self.registry.retired.add(object);
    }
}

pub(crate) async fn register(
    registry: Arc<Registry>,
    source: Arc<dyn FrameSource>,
    // One texture takes both kinds of frames here.
    _kind: Kind,
) -> Result<Registered> {
    thread::call(move |_| {
        let api = registry.api;
        let raster = Arc::new(Raster::new(source, Arc::clone(&registry.retired)));
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
            object: ObjectRef::new(api, object),
            raster,
        })
    })
    .await?
}

#[cfg(test)]
mod tests {
    use std::sync::Weak;

    use super::*;
    use crate::Acquired;

    /// A texture source that never has a frame; the test only needs its `Raster`.
    struct NoFrames;

    impl FrameSource for NoFrames {
        fn acquire(&self) -> Option<Acquired> {
            None
        }
    }

    /// A stand-in for the native texture object, released the way `g_object_unref` would.
    struct Fake {
        value: u32,
    }

    /// `g_object_unref` for a fake object: the test put an `Arc<Fake>` in the pointer.
    unsafe extern "C" fn release_fake(object: Gpointer) {
        // SAFETY: the test gives the pointer `Arc::into_raw` made, once.
        drop(unsafe { Arc::from_raw(object.cast::<Fake>()) });
    }

    /// Retire a fake texture in `retired` and return what can tell whether it is still alive.
    fn retire_fake(retired: &Retired, value: u32) -> Weak<Fake> {
        let fake = Arc::new(Fake { value });
        let weak = Arc::downgrade(&fake);
        retired.add(ObjectRef::for_test(
            Arc::into_raw(fake) as usize,
            release_fake,
        ));
        weak
    }

    #[test]
    fn retired_textures_live_until_a_populate_finishes() {
        let retired = Arc::new(Retired::default());
        let raster = Raster::new(Arc::new(NoFrames), Arc::clone(&retired));

        let first = retire_fake(&retired, 1);
        let second = retire_fake(&retired, 2);

        // The raster thread may still use either object: both are alive.
        assert_eq!(
            first
                .upgrade()
                .expect("first texture released before raster use")
                .value,
            1
        );
        assert_eq!(
            second
                .upgrade()
                .expect("second texture released before raster use")
                .value,
            2
        );

        // The next populate has finished: both can go, exactly once.
        raster.release_retired();
        assert!(first.upgrade().is_none());
        assert!(second.upgrade().is_none());

        // A later populate has nothing left to release.
        raster.release_retired();
    }
}
