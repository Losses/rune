//! Android: `SurfaceProducer`s of the engine's `TextureRegistry` (the `FlutterRenderer`).
//!
//! A texture is a `SurfaceProducer` (`TextureRegistry.createSurfaceProducer()`). On API 29 and
//! later it is backed by an `ImageReader` whose images Impeller samples as they are; before, by
//! a `SurfaceTexture`. Either way the producer hands out an `android.view.Surface`, whose native
//! window ([`NativeWindow`]) the visualizer presents frames into with a Vulkan swapchain: there
//! are no frames to pull, and the engine schedules a frame itself when an image arrives, so the
//! [`FrameNotifier`] does nothing here.
//!
//! The producer is created and released on the main thread, as the engine requires
//! (`FlutterJNI.registerImageTexture` and `unregisterTexture` check it), through the main
//! thread's looper ([`crate::thread`]). `setSize` and `getSurface` would take any thread, but go
//! there too, so that one thread does all the JNI calls. Only methods of objects we hold are
//! looked up (`GetObjectClass`), never classes by name, which a thread without the app's class
//! loader would not find. A Java exception a call raises is logged and cleared, as when the
//! engine was detached from its activity.

pub(crate) mod ndk;

use std::ffi::c_void;
use std::future::Future;
use std::ptr::NonNull;
use std::sync::Arc;

use jni::objects::{GlobalRef, JObject, JValue};
use jni::{JNIEnv, JavaVM};
use log::{error, warn};
use raw_window_handle::{
    AndroidNdkWindowHandle, DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle,
    RawWindowHandle, WindowHandle,
};

use crate::thread::{self, MainThread};
use crate::{DeviceUuids, FrameNotifier, FrameSource, Kind, PlatformHandle, Result, TextureError};

/// `TextureRegistry.createSurfaceProducer()`, which makes a producer with the manual surface
/// lifecycle: the engine never asks for the surface back, so no callback is needed.
const CREATE_SURFACE_PRODUCER: (&str, &str) = (
    "createSurfaceProducer",
    "()Lio/flutter/view/TextureRegistry$SurfaceProducer;",
);

/// The engine's texture registry, by a global reference.
pub(crate) struct Registry {
    vm: JavaVM,
    registry: GlobalRef,
}

impl Registry {
    /// # Safety
    ///
    /// Called on the main thread, with the `JNIEnv` of this thread and a live reference to an
    /// object that implements `io.flutter.view.TextureRegistry`.
    pub(crate) unsafe fn new(handle: PlatformHandle) -> Result<Self> {
        match handle {
            PlatformHandle::JniTextureRegistry { env, registry } => {
                // SAFETY: the caller guarantees this thread's `JNIEnv`.
                let env = unsafe { JNIEnv::from_raw(env.as_ptr().cast()) }
                    .map_err(|_| TextureError::InvalidHandle)?;
                let vm = env.get_java_vm().map_err(|_| TextureError::InvalidHandle)?;
                // SAFETY: the caller guarantees a live reference. `JObject` does not delete it.
                let object = unsafe { JObject::from_raw(registry.as_ptr().cast()) };
                let registry = env
                    .new_global_ref(&object)
                    .map_err(|_| TextureError::InvalidHandle)?;
                thread::hook_main_looper()?;
                Ok(Self { vm, registry })
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

    /// The `JNIEnv` of the main thread, which is a Java thread and so always attached.
    fn env(&self, _main: MainThread) -> Result<JNIEnv<'_>> {
        self.vm
            .get_env()
            .map_err(|e| TextureError::Engine(format!("No JNIEnv on the main thread: {e}")))
    }
}

/// Run `calls` in a local reference frame of their own; a Java exception they raise is logged
/// (with its stack trace, in logcat) and cleared.
fn jni_calls<T>(
    env: &mut JNIEnv,
    what: &str,
    calls: impl FnOnce(&mut JNIEnv) -> jni::errors::Result<T>,
) -> Result<T> {
    let result = env.with_local_frame(8, calls);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    result.map_err(|e| TextureError::Engine(format!("{what}: {e}")))
}

/// A `SurfaceProducer` as the registry knows it.
pub(crate) struct Registered {
    registry: Arc<Registry>,
    id: i64,
    producer: GlobalRef,
}

impl Registered {
    pub(crate) fn id(&self) -> i64 {
        self.id
    }

    /// Does nothing: the engine schedules a frame when an image arrives in the producer.
    pub(crate) fn notifier(&self) -> FrameNotifier {
        FrameNotifier::new(|task| task(), || {})
    }

    pub(crate) async fn unregister(self) {
        if thread::call(move |main| self.release(main)).await.is_err() {
            error!("The platform thread did not unregister a Flutter texture");
        }
    }

    pub(crate) fn unregister_later(self) {
        thread::run(move |main| self.release(main));
    }

    /// `SurfaceProducer.release()`. Native windows taken from the producer stay valid (they hold
    /// their own reference), but nothing they show reaches the engine any more.
    fn release(self, main: MainThread) {
        let released = self.registry.env(main).and_then(|mut env| {
            jni_calls(&mut env, "SurfaceProducer.release", |env| {
                env.call_method(&self.producer, "release", "()V", &[])?.v()
            })
        });
        if let Err(e) = released {
            warn!("Failed to release Flutter texture {}: {e}", self.id);
        }
    }

    /// `SurfaceProducer.setSize(width, height)`: the size of the surfaces from the next
    /// [`Self::window`] on.
    pub(crate) fn set_size(
        &self,
        width: u32,
        height: u32,
    ) -> impl Future<Output = Result<()>> + Send + 'static {
        let (registry, producer) = (Arc::clone(&self.registry), self.producer.clone());
        async move {
            thread::call(move |main| {
                let mut env = registry.env(main)?;
                set_size(&mut env, &producer, width, height)
            })
            .await?
        }
    }

    /// The native window of the producer's current surface (`SurfaceProducer.getSurface()`), which
    /// is a new one after a [`Self::set_size`].
    pub(crate) fn window(&self) -> impl Future<Output = Result<NativeWindow>> + Send + 'static {
        let (registry, producer) = (Arc::clone(&self.registry), self.producer.clone());
        async move {
            thread::call(move |main| {
                let mut env = registry.env(main)?;
                jni_calls(&mut env, "SurfaceProducer.getSurface", |env| {
                    let surface = env
                        .call_method(&producer, "getSurface", "()Landroid/view/Surface;", &[])?
                        .l()?;
                    if surface.is_null() {
                        return Ok(None);
                    }
                    // SAFETY: `env` is this thread's and `surface` a live `android.view.Surface`.
                    let window =
                        unsafe { ndk::ANativeWindow_fromSurface(env.get_raw(), surface.as_raw()) };
                    Ok(NonNull::new(window).map(NativeWindow))
                })?
                .ok_or_else(|| TextureError::Engine("The texture has no surface".to_string()))
            })
            .await?
        }
    }
}

fn set_size(env: &mut JNIEnv, producer: &GlobalRef, width: u32, height: u32) -> Result<()> {
    let (width, height) = (dimension(width), dimension(height));
    jni_calls(env, "SurfaceProducer.setSize", |env| {
        env.call_method(
            producer,
            "setSize",
            "(II)V",
            &[JValue::Int(width), JValue::Int(height)],
        )?
        .v()
    })
}

/// A width or height as Java's `int`, at least 1.
fn dimension(value: u32) -> i32 {
    i32::try_from(value.max(1)).unwrap_or(i32::MAX)
}

/// Only textures whose frames are presented into their window ([`register_surface`]) exist here.
pub(crate) async fn register(
    _registry: Arc<Registry>,
    _source: Arc<dyn FrameSource>,
    _kind: Kind,
) -> Result<Registered> {
    Err(TextureError::Unsupported)
}

/// Make a `SurfaceProducer` of `width` by `height` on the main thread.
pub(crate) async fn register_surface(
    registry: Arc<Registry>,
    width: u32,
    height: u32,
) -> Result<Registered> {
    thread::call(move |main| {
        let mut env = registry.env(main)?;
        let (create, signature) = CREATE_SURFACE_PRODUCER;
        let producer = jni_calls(&mut env, "TextureRegistry.createSurfaceProducer", |env| {
            let producer = env
                .call_method(registry.registry.as_obj(), create, signature, &[])?
                .l()?;
            if producer.is_null() {
                return Ok(None);
            }
            env.new_global_ref(producer).map(Some)
        })?
        .ok_or_else(|| TextureError::Engine("The engine made no surface producer".to_string()))?;
        let id = jni_calls(&mut env, "SurfaceProducer.id", |env| {
            env.call_method(&producer, "id", "()J", &[])?.j()
        })
        .and_then(|id| set_size(&mut env, &producer, width, height).map(|()| id));
        match id {
            Ok(id) => Ok(Registered {
                registry,
                id,
                producer,
            }),
            Err(e) => {
                Registered {
                    registry,
                    id: -1,
                    producer,
                }
                .release(main);
                Err(e)
            }
        }
    })
    .await?
}

/// An `ANativeWindow` of a texture's surface, which a producer presents frames into (for wgpu,
/// through [`HasWindowHandle`]). Holds a reference to the window, released when it is dropped.
pub struct NativeWindow(NonNull<ndk::ANativeWindow>);

// SAFETY: `ANativeWindow` is reference counted with atomics, and its functions may be called
// from any thread; the producer (the Vulkan swapchain) serializes its use of the window.
unsafe impl Send for NativeWindow {}
// SAFETY: as above; `&NativeWindow` only hands out the pointer.
unsafe impl Sync for NativeWindow {}

impl NativeWindow {
    /// The `ANativeWindow*`, valid while `self` is alive.
    pub fn as_ptr(&self) -> NonNull<c_void> {
        self.0.cast()
    }
}

impl Drop for NativeWindow {
    fn drop(&mut self) {
        // SAFETY: the reference `ANativeWindow_fromSurface` acquired, released once.
        unsafe { ndk::ANativeWindow_release(self.0.as_ptr()) };
    }
}

impl HasWindowHandle for NativeWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(self.as_ptr()));
        // SAFETY: the window stays valid while `self`, which the handle borrows, is alive.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for NativeWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::android())
    }
}
