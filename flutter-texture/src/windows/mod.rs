//! Windows: textures of the engine's texture registrar, through the C API of
//! `flutter_windows.dll` (see [`api`]).
//!
//! The registrar takes register, unregister and mark calls from any thread (the engine posts the
//! work to its own threads), so nothing hops to the platform thread. The engine reads frames on
//! its raster thread, through the objects in [`raster`]. A texture shows one [`Kind`] of frames:
//! the engine has two registration types, one for shared DXGI handles and one for pixel buffers.
//!
//! The engine can go away under us (the runner destroys it with the window). Its destruction
//! handler marks the registry as gone; after that, calls into the engine are skipped, and what
//! textures hold is released without waiting for the engine.

mod api;
mod raster;

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, Weak};

use ::windows::Win32::Graphics::Dxgi::IDXGIAdapter;
use ::windows::core::Interface;
use futures_channel::oneshot;
use log::{error, info};

use crate::{FrameNotifier, FrameSource, Kind, PlatformHandle, Result, TextureError, lock};
use api::{
    Api, GpuSurfaceConfig, PixelBufferConfig, PluginRegistrar, TextureInfo, TextureRegistrar,
};

/// The engine's functions, looked up once they are found.
fn api() -> Result<&'static Api> {
    static API: std::sync::OnceLock<Api> = std::sync::OnceLock::new();
    if let Some(api) = API.get() {
        return Ok(api);
    }
    // A failed lookup is tried again later: the engine may not be loaded yet.
    let loaded = Api::load()?;
    Ok(API.get_or_init(|| loaded))
}

/// Registrars whose destruction handler is still to run, by address.
static LIVE: Mutex<Vec<(usize, Weak<State>)>> = Mutex::new(Vec::new());

/// What textures and notifiers share with the registry.
struct State {
    api: &'static Api,
    registrar: PluginRegistrar,
    textures: TextureRegistrar,
    /// The engine exists. Held while calling into it, so that its destruction (which sets this
    /// false) does not overlap such a call. The engine does not call back into us under it
    /// except for the unregistration callback, which only drops our own state.
    alive: Mutex<bool>,
    /// The adapter Flutter renders on, once known.
    adapter_luid: Mutex<Option<(u32, i32)>>,
}

// SAFETY: the registrar pointers are only handed back to the engine, whose functions take them
// from any thread (as documented in `flutter_texture_registrar.h`); `alive` guards their use
// against the engine's destruction.
unsafe impl Send for State {}
// SAFETY: as above.
unsafe impl Sync for State {}

impl State {
    /// The adapter the engine renders on, from the engine (a new reference to it is released
    /// again here). Called with the engine alive, on the thread that attached for the first time.
    fn query_adapter(&self) -> Option<(u32, i32)> {
        let mut adapter: *mut c_void = std::ptr::null_mut();
        // SAFETY: the registrar is alive (the caller holds `alive`); `adapter` is writable.
        let found = unsafe { (self.api.get_graphics_adapter)(self.registrar, &mut adapter) };
        if !found || adapter.is_null() {
            return None;
        }
        // SAFETY: the engine returned a new reference, which we own and release on drop.
        let adapter = unsafe { IDXGIAdapter::from_raw(adapter) };
        // SAFETY: a plain call on a live adapter.
        let desc = unsafe { adapter.GetDesc() }.ok()?;
        Some((desc.AdapterLuid.LowPart, desc.AdapterLuid.HighPart))
    }

    fn mark_frame_available(&self, id: i64) {
        if *lock(&self.alive) {
            // SAFETY: the engine is alive while `alive` is held.
            unsafe { (self.api.mark_frame_available)(self.textures, id) };
        }
    }
}

/// The destruction handler: later calls into this engine are skipped.
unsafe extern "C" fn on_destroyed(registrar: PluginRegistrar) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let mut live = lock(&LIVE);
        live.retain(|(address, state)| {
            if *address != registrar as usize {
                return true;
            }
            if let Some(state) = state.upgrade() {
                *lock(&state.alive) = false;
                info!("The Flutter engine is gone; its textures are detached");
            }
            false
        });
    }));
}

/// The engine's texture registrar.
pub(crate) struct Registry(Arc<State>);

impl Registry {
    /// # Safety
    ///
    /// The handle must be a live `FlutterDesktopPluginRegistrarRef`.
    pub(crate) unsafe fn new(handle: PlatformHandle) -> Result<Self> {
        let PlatformHandle::PluginRegistrar(registrar) = handle;
        let api = api()?;
        let registrar: NonNull<c_void> = registrar;
        // SAFETY: the caller guarantees a live registrar.
        let textures = unsafe { (api.get_texture_registrar)(registrar.as_ptr()) };
        if textures.is_null() {
            return Err(TextureError::InvalidHandle);
        }
        let state = Arc::new(State {
            api,
            registrar: registrar.as_ptr(),
            textures,
            alive: Mutex::new(true),
            adapter_luid: Mutex::new(None),
        });
        *lock(&state.adapter_luid) = state.query_adapter();
        lock(&LIVE).push((registrar.as_ptr() as usize, Arc::downgrade(&state)));
        // SAFETY: the registrar is alive; the handler is a plain function.
        unsafe { (api.set_destruction_handler)(registrar.as_ptr(), Some(on_destroyed)) };
        Ok(Self(state))
    }

    /// The adapter the engine renders on. Asked again until the engine knows it (the first
    /// frame may come after the runner attached).
    pub(crate) fn adapter_luid(&self) -> Option<(u32, i32)> {
        let state = &self.0;
        let alive = lock(&state.alive);
        if !*alive {
            return None;
        }
        let mut known = lock(&state.adapter_luid);
        if known.is_none() {
            *known = state.query_adapter();
        }
        *known
    }
}

/// A texture as the registrar knows it.
pub(crate) struct Registered {
    state: Arc<State>,
    id: i64,
    /// Lets go of what the engine's callbacks used, once it can no longer call them.
    release: Box<dyn FnOnce() + Send>,
}

/// What the engine's unregistration callback gets.
struct Unregistered {
    release: Box<dyn FnOnce() + Send>,
    done: Option<oneshot::Sender<()>>,
}

impl Unregistered {
    fn finish(self) {
        (self.release)();
        if let Some(done) = self.done {
            let _ = done.send(());
        }
    }
}

/// The engine removed the texture: its callbacks will not run again.
unsafe extern "C" fn on_unregistered(context: *mut c_void) {
    // SAFETY: `context` is the box `unregister_with` leaked, called once.
    let unregistered = unsafe { Box::from_raw(context.cast::<Unregistered>()) };
    if catch_unwind(AssertUnwindSafe(|| unregistered.finish())).is_err() {
        error!("Unregistering a Flutter texture panicked");
    }
}

impl Registered {
    pub(crate) fn id(&self) -> i64 {
        self.id
    }

    pub(crate) fn notifier(&self) -> FrameNotifier {
        let state = Arc::clone(&self.state);
        let id = self.id;
        // Calls from any thread are fine here, so a notice is sent from the caller's.
        FrameNotifier::new(|task| task(), move || state.mark_frame_available(id))
    }

    /// Unregister and wait until the engine let go of the texture (its raster thread ran the
    /// callback, or the engine is gone).
    pub(crate) async fn unregister(self) {
        let (sender, receiver) = oneshot::channel();
        self.unregister_with(Some(sender));
        // An error is the sender dropped: the engine never ran the callback.
        let _ = receiver.await;
    }

    pub(crate) fn unregister_later(self) {
        self.unregister_with(None);
    }

    fn unregister_with(self, done: Option<oneshot::Sender<()>>) {
        let Self { state, id, release } = self;
        let unregistered = Unregistered { release, done };
        let alive = lock(&state.alive);
        if !*alive {
            drop(alive);
            // The engine is gone with its raster thread: nothing calls the texture any more.
            unregistered.finish();
            return;
        }
        let context = Box::into_raw(Box::new(unregistered));
        // SAFETY: the engine is alive while `alive` is held; the callback takes the box back, and
        // may run right here.
        unsafe {
            (state.api.unregister)(state.textures, id, Some(on_unregistered), context.cast());
        }
    }
}

impl State {
    /// Register a texture whose callbacks get `raster` as their user data. `info` makes the
    /// registration of the pointer to it; `release` runs once the engine can no longer call it.
    fn register_raster<T: Send + Sync + 'static>(
        self: &Arc<Self>,
        raster: Arc<T>,
        info: impl FnOnce(*mut c_void) -> TextureInfo,
        release: fn(&T),
    ) -> Result<Registered> {
        let alive = lock(&self.alive);
        if !*alive {
            return Err(TextureError::NotAttached);
        }
        // The engine's reference to the texture, taken back once it is unregistered.
        let user_data = Arc::into_raw(Arc::clone(&raster));
        let info = info(user_data.cast_mut().cast());
        // SAFETY: the engine is alive while `alive` is held; `info` is valid for the call.
        let id = unsafe { (self.api.register)(self.textures, &info) };
        drop(alive);
        let user_data = user_data as usize;
        let release = move || {
            release(&raster);
            // SAFETY: the reference `Arc::into_raw` made above, given up once.
            drop(unsafe { Arc::from_raw(user_data as *const T) });
        };
        if id < 0 {
            // Never registered: nothing calls the callbacks.
            release();
            return Err(TextureError::Frame(
                "the engine did not register the texture (it has no ANGLE, or the texture was refused)"
                    .into(),
            ));
        }
        Ok(Registered {
            state: Arc::clone(self),
            id,
            release: Box::new(release),
        })
    }
}

pub(crate) async fn register(
    registry: Arc<Registry>,
    source: Arc<dyn FrameSource>,
    kind: Kind,
) -> Result<Registered> {
    let state = &registry.0;
    match kind {
        Kind::Pixels => state.register_raster(
            Arc::new(raster::Pixels::new(source)),
            |user_data| TextureInfo {
                texture_type: api::PIXEL_BUFFER_TEXTURE,
                config: api::TextureConfig {
                    pixel_buffer: PixelBufferConfig {
                        callback: Some(raster::pixel_buffer),
                        user_data,
                    },
                },
            },
            |_| {},
        ),
        Kind::Dxgi => state.register_raster(
            Arc::new(raster::Gpu::new(source)),
            |user_data| TextureInfo {
                texture_type: api::GPU_SURFACE_TEXTURE,
                config: api::TextureConfig {
                    gpu_surface: GpuSurfaceConfig {
                        struct_size: size_of::<GpuSurfaceConfig>(),
                        surface_type: api::GPU_SURFACE_DXGI_SHARED_HANDLE,
                        callback: Some(raster::gpu_surface),
                        user_data,
                    },
                },
            },
            raster::Gpu::release,
        ),
    }
}
