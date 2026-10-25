//! The object the engine is given as a texture: a GObject subclass of `FlTextureGL`, registered
//! from Rust, whose instances point to a [`Raster`].
//!
//! The engine calls `populate` on its raster thread with its OpenGL context current, and samples
//! the texture it is given (a `GL_RGBA8` `GL_TEXTURE_2D`) until populate gives another one. A
//! frame in a Vulkan image is imported and its texture handed out; a frame of pixels is uploaded
//! into a texture of our own. Either way the texture keeps the id it was registered with.
//!
//! OpenGL objects can only be deleted inside a populate, since only there is Flutter's context
//! current: an unregistered texture leaves its objects to the next populate of any texture
//! ([`GARBAGE`]).

use std::any::Any;
use std::ffi::{CString, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use log::{error, info, warn};

use super::SHARING_FAILED;
use super::api::{
    self, Api, FALSE, GType, Gboolean, Gpointer, ObjectClass, PopulateFunc, TRUE, TypeInfo,
    TypeQuery,
};
use super::gl::{self, Gl, PixelTexture};
use super::logic::{self, Garbage, Imported};
use crate::{Acquired, Frame, FrameSource, Pixels, TextureError, VulkanImage, lock};

/// What textures left for a populate to delete.
static GARBAGE: Mutex<Garbage> = Mutex::new(Garbage {
    textures: Vec::new(),
    memory_objects: Vec::new(),
    semaphores: Vec::new(),
});

/// The registered subclass and where its instances keep the pointer to their [`Raster`].
struct TextureType {
    gtype: GType,
    offset: usize,
}

static TEXTURE_TYPE: OnceLock<Result<TextureType, String>> = OnceLock::new();
/// `FlTextureGLClass`, for chaining up; set by `class_init`.
static PARENT_CLASS: AtomicPtr<ObjectClass> = AtomicPtr::new(std::ptr::null_mut());

/// The subclass of `FlTextureGL`, registered on first use. Its instances are the parent's plus a
/// pointer; its class is the parent's (`FlTextureGLClass`) with our functions.
fn texture_type(api: &'static Api) -> Result<&'static TextureType, String> {
    TEXTURE_TYPE
        .get_or_init(|| {
            let mut query = TypeQuery {
                type_: 0,
                type_name: std::ptr::null(),
                class_size: 0,
                instance_size: 0,
            };
            // SAFETY: plain calls; `query` is writable.
            let parent = unsafe { (api.texture_gl_get_type)() };
            // SAFETY: as above.
            unsafe { (api.type_query)(parent, &mut query) };
            if query.type_ == 0 {
                return Err("FlTextureGL is not a registered type".into());
            }
            let offset =
                (query.instance_size as usize).next_multiple_of(align_of::<*const Raster>());
            let instance_size = u16::try_from(offset + size_of::<*const Raster>());
            let class_size = u16::try_from(query.class_size);
            let (Ok(instance_size), Ok(class_size)) = (instance_size, class_size) else {
                return Err("FlTextureGL is too large to derive from".into());
            };
            if usize::from(class_size) < size_of::<ObjectClass>() + size_of::<PopulateFunc>() {
                return Err("FlTextureGLClass has no populate".into());
            }
            let info = TypeInfo {
                class_size,
                base_init: None,
                base_finalize: None,
                class_init: Some(class_init),
                class_finalize: None,
                class_data: std::ptr::null(),
                instance_size,
                n_preallocs: 0,
                instance_init: None,
                value_table: std::ptr::null(),
            };
            // SAFETY: a valid type info with functions that live for the program; the name is
            // NUL-terminated. Instances are zero-filled, so the pointer starts null.
            let gtype = unsafe {
                (api.type_register_static)(parent, c"RuneFlutterTextureGL".as_ptr(), &info, 0)
            };
            if gtype == 0 {
                return Err("GObject did not register RuneFlutterTextureGL".into());
            }
            Ok(TextureType { gtype, offset })
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// `GClassInitFunc`: overrides `dispose`, `finalize` and `populate`.
///
/// # Safety
///
/// `class` is the class structure of [`texture_type`], being initialised.
unsafe extern "C" fn class_init(class: Gpointer, _data: Gpointer) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let Ok(api) = api::api() else { return };
        // SAFETY: GObject passes the class being initialised, whose head is a `GObjectClass`
        // followed by `FlTextureGLClass::populate` (its size was checked on registration).
        unsafe {
            PARENT_CLASS.store(
                (api.type_class_peek_parent)(class).cast(),
                Ordering::Release,
            );
            let object_class = class.cast::<ObjectClass>();
            (*object_class).dispose = Some(dispose);
            (*object_class).finalize = Some(finalize);
            class
                .cast::<u8>()
                .add(size_of::<ObjectClass>())
                .cast::<Option<PopulateFunc>>()
                .write(Some(populate));
        }
    }));
    if result.is_err() {
        error!("Flutter texture: initialising the texture class panicked");
    }
}

/// Where `object` keeps the pointer to its [`Raster`].
///
/// # Safety
///
/// `object` is a live instance of [`texture_type`].
unsafe fn slot(object: Gpointer) -> *mut *const Raster {
    let offset = TEXTURE_TYPE
        .get()
        .and_then(|registered| registered.as_ref().ok())
        .map_or(0, |registered| registered.offset);
    debug_assert_ne!(offset, 0);
    // SAFETY: the instance is `offset` bytes plus the pointer, aligned for it.
    unsafe { object.cast::<u8>().add(offset).cast() }
}

/// The raster state of `object`, if it has one.
///
/// # Safety
///
/// `object` is a live instance of [`texture_type`]; the result is valid while it is alive.
unsafe fn raster<'a>(object: Gpointer) -> Option<&'a Raster> {
    // SAFETY: as the caller guarantees; the pointer is null or the `Arc` that `new_object` put
    // there, given up only by `finalize`.
    unsafe { slot(object).read().as_ref() }
}

/// A new texture object with `raster`, holding one reference for the caller.
pub(super) fn new_object(api: &'static Api, raster: Arc<Raster>) -> Result<Gpointer, String> {
    let texture_type = texture_type(api)?;
    // SAFETY: a registered instantiatable type, no properties.
    let object = unsafe {
        (api.object_new_with_properties)(texture_type.gtype, 0, std::ptr::null(), std::ptr::null())
    };
    if object.is_null() {
        return Err("GObject did not make a texture".into());
    }
    // SAFETY: a fresh instance of the type; `finalize` takes the reference back.
    unsafe { slot(object).write(Arc::into_raw(raster)) };
    Ok(object)
}

/// The parent's `dispose` or `finalize`.
fn parent(pick: impl FnOnce(&ObjectClass) -> Option<api::ObjectFunc>) -> Option<api::ObjectFunc> {
    // SAFETY: the parent class lives as long as the type system, and is only read.
    unsafe { PARENT_CLASS.load(Ordering::Acquire).as_ref() }.and_then(pick)
}

/// `GObjectClass::dispose`: stops handing out frames (see [`Raster::retire`]).
///
/// # Safety
///
/// `object` is a live instance of [`texture_type`].
unsafe extern "C" fn dispose(object: Gpointer) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as the caller guarantees.
        if let Some(raster) = unsafe { raster(object) } {
            raster.retire();
        }
    }));
    if result.is_err() {
        error!("Flutter texture: dispose panicked");
    }
    if let Some(dispose) = parent(|class| class.dispose) {
        // SAFETY: chaining up with the same object.
        unsafe { dispose(object) };
    }
}

/// `GObjectClass::finalize`: drops the raster state.
///
/// # Safety
///
/// `object` is a live instance of [`texture_type`], finalised once.
unsafe extern "C" fn finalize(object: Gpointer) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as the caller guarantees; the pointer is the `Arc` from `new_object`, taken back
        // once.
        unsafe {
            let raster = slot(object).replace(std::ptr::null());
            if !raster.is_null() {
                drop(Arc::from_raw(raster));
            }
        }
    }));
    if result.is_err() {
        error!("Flutter texture: finalize panicked");
    }
    if let Some(finalize) = parent(|class| class.finalize) {
        // SAFETY: chaining up with the same object.
        unsafe { finalize(object) };
    }
}

/// `FlTextureGLClass::populate`, on the raster thread with Flutter's context current. On failure
/// it must set `error`: the engine logs its message without checking it.
///
/// # Safety
///
/// `texture` is a live instance of [`texture_type`]; the other pointers are writable or (`error`)
/// null.
unsafe extern "C" fn populate(
    texture: Gpointer,
    target: *mut u32,
    name: *mut u32,
    width: *mut u32,
    height: *mut u32,
    error: *mut *mut c_void,
) -> Gboolean {
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as the caller guarantees; the engine borrows the object during the call.
        let raster = unsafe { raster(texture) }.ok_or("the texture has no frames")?;
        raster.populate()
    }));
    let message = match result {
        Ok(Ok(shown)) => {
            // SAFETY: the engine passes writable outputs.
            unsafe {
                *target = gl::GL_TEXTURE_2D;
                *name = shown.texture;
                *width = shown.width;
                *height = shown.height;
            }
            return TRUE;
        }
        Ok(Err(message)) => message,
        Err(_) => {
            error!("Flutter texture: populate panicked");
            "populate panicked".into()
        }
    };
    if !error.is_null()
        && let Ok(api) = api::api()
    {
        let message = CString::new(message).unwrap_or_default();
        // SAFETY: `error` points to a null or an earlier `GError*`, which GLib handles; the
        // strings are NUL-terminated and copied.
        unsafe {
            let domain = (api.quark_from_static_string)(c"rune-flutter-texture".as_ptr());
            (api.set_error_literal)(error, domain, 0, message.as_ptr());
        }
    }
    FALSE
}

/// What a texture works with on the raster thread.
pub(super) struct Raster {
    source: Arc<dyn FrameSource>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Unregistered: nothing is handed out any more.
    retired: bool,
    /// What the engine was given last.
    shown: Option<Shown>,
    /// The Vulkan images imported so far that may still be shown.
    images: Vec<Imported>,
    /// The semaphore waited for before the frame shown was handed over, deleted once a newer one
    /// is.
    semaphore: u32,
    /// The texture frames of pixels go into.
    pixels: Option<PixelTexture>,
    /// Whether Flutter's own context has the import extensions, once checked (the probe found
    /// them on a context like it).
    importable: Option<bool>,
    /// A frame in a Vulkan image could not be shown: frames in images are no longer imported, and
    /// the producer was told.
    shared_failed: bool,
    /// The last frame of pixels failed; the next failure is not reported again.
    pixels_failing: bool,
    /// Frames handed over, for the log.
    handed_over: u64,
}

/// A frame handed to the engine.
struct Shown {
    frame_id: u64,
    texture: u32,
    width: u32,
    height: u32,
    /// The Vulkan image the frame is in, if it is in one.
    image_id: Option<u64>,
    /// The producer's frame, held while the engine may sample its image.
    keep: Option<Box<dyn Any + Send>>,
}

/// The texture populate gives the engine.
struct Output {
    texture: u32,
    width: u32,
    height: u32,
}

impl Raster {
    pub(super) fn new(source: Arc<dyn FrameSource>) -> Self {
        Self {
            source,
            state: Mutex::default(),
        }
    }

    /// Hand out the newest frame, or the one handed out last if there is no newer one.
    fn populate(&self) -> Result<Output, String> {
        let gl = gl::gl()?;
        // Only here is Flutter's context current.
        let garbage = std::mem::take(&mut *lock(&GARBAGE));
        gl.delete(garbage);

        // Frames given back once the lock is released: the producer's may do work when they go.
        let mut released: Vec<Box<dyn Any + Send>> = Vec::new();
        let mut state = lock(&self.state);
        if state.retired {
            return Err("the texture is unregistered".into());
        }
        if let Some(Acquired { id, frame, keep }) = self.source.acquire() {
            if state.shown.as_ref().map(|shown| shown.frame_id) == Some(id) {
                released.push(keep);
            } else {
                match frame {
                    Frame::VulkanImage(image) => {
                        self.show_image(&mut state, gl, id, image, keep, &mut released);
                    }
                    Frame::Pixels(pixels) => {
                        self.show_pixels(&mut state, gl, id, &pixels, &mut released);
                        // Uploaded (or not): the producer may have its frame back.
                        released.push(keep);
                    }
                }
            }
        }
        let output = state.shown.as_ref().map(|shown| Output {
            texture: shown.texture,
            width: shown.width,
            height: shown.height,
        });
        drop(state);
        drop(released);
        output.ok_or_else(|| "no frame yet".into())
    }

    /// Make the frame `id` in `image` the one handed out: import the image unless it was before,
    /// and wait for the frame's semaphore. If OpenGL cannot, frames in images are no longer
    /// imported and the producer is told; the last frame stays.
    fn show_image(
        &self,
        state: &mut State,
        gl: &Gl,
        id: u64,
        image: VulkanImage,
        keep: Box<dyn Any + Send>,
        released: &mut Vec<Box<dyn Any + Send>>,
    ) {
        if state.shared_failed {
            released.push(keep);
            return;
        }
        let importable = *state.importable.get_or_insert_with(|| gl.imports_vulkan());
        let result = if importable {
            import(state, gl, &image)
        } else {
            Err("Flutter's OpenGL context cannot import Vulkan memory and semaphores".into())
        };
        let (texture, semaphore) = match result {
            Ok(imported) => imported,
            Err(e) => {
                released.push(keep);
                state.shared_failed = true;
                // Later textures read frames back from the start.
                SHARING_FAILED.store(true, Ordering::Relaxed);
                warn!("Flutter texture: {e}; reading frames back");
                self.source.failed(TextureError::SharedFrame(e));
                return;
            }
        };

        // The wait for the previous frame was issued long ago.
        gl.delete_semaphore(std::mem::replace(&mut state.semaphore, semaphore));
        let previous = state.shown.replace(Shown {
            frame_id: id,
            texture,
            width: image.width,
            height: image.height,
            image_id: Some(image.image_id),
            keep: Some(keep),
        });
        let previous_image = previous.as_ref().and_then(|shown| shown.image_id);
        released.extend(previous.and_then(|shown| shown.keep));
        // Images of an earlier set are no longer drawn into; their textures go once the engine
        // stopped sampling them, which it may still do with the frame handed over before.
        gl.delete_images(logic::prune(
            &mut state.images,
            Some(image.image_set),
            [Some(image.image_id), previous_image],
        ));

        state.handed_over += 1;
        if logic::should_log(state.handed_over) {
            info!(
                "Flutter texture: frame {id} #{} shared (image {}, {}x{})",
                state.handed_over, image.image_id, image.width, image.height
            );
        }
    }

    /// Upload the frame `id` of `pixels` and make it the one handed out. If that fails the
    /// producer is told (once per run of failures); the last frame stays.
    fn show_pixels(
        &self,
        state: &mut State,
        gl: &Gl,
        id: u64,
        pixels: &Pixels,
        released: &mut Vec<Box<dyn Any + Send>>,
    ) {
        let texture = match gl.upload(&mut state.pixels, pixels) {
            Ok(texture) => texture,
            Err(e) => {
                if !std::mem::replace(&mut state.pixels_failing, true) {
                    error!("Flutter texture: {e}");
                    self.source.failed(TextureError::Frame(e));
                }
                return;
            }
        };
        state.pixels_failing = false;
        let previous = state.shown.replace(Shown {
            frame_id: id,
            texture,
            width: pixels.width(),
            height: pixels.height(),
            image_id: None,
            keep: None,
        });
        let previous_image = previous.as_ref().and_then(|shown| shown.image_id);
        released.extend(previous.and_then(|shown| shown.keep));
        // The producer reads frames back now: the images go, the one shown last once the engine
        // stopped sampling it.
        gl.delete_images(logic::prune(
            &mut state.images,
            None,
            [previous_image, None],
        ));
        if previous_image.is_some() {
            gl.delete_semaphore(std::mem::take(&mut state.semaphore));
        }
    }

    /// Stop handing out frames, give the frame held back and leave the OpenGL objects to the next
    /// populate. Called once the texture is unregistered, and when it is disposed.
    pub(super) fn retire(&self) {
        let mut state = lock(&self.state);
        if std::mem::replace(&mut state.retired, true) {
            return;
        }
        {
            let mut garbage = lock(&GARBAGE);
            garbage.add_images(state.images.drain(..));
            if let Some(pixels) = state.pixels.take() {
                garbage.textures.push(pixels.name);
            }
            if state.semaphore != 0 {
                garbage
                    .semaphores
                    .push(std::mem::take(&mut state.semaphore));
            }
        }
        let shown = state.shown.take();
        let handed_over = state.handed_over;
        drop(state);
        drop(shown);
        info!("Flutter texture retired after {handed_over} shared frames");
    }
}

/// The texture of `image`'s memory (imported now unless it was before) after a wait for the
/// frame's semaphore, and the semaphore.
fn import(state: &mut State, gl: &Gl, image: &VulkanImage) -> Result<(u32, u32), String> {
    let texture = match state
        .images
        .iter()
        .find(|imported| imported.image_id == image.image_id)
    {
        Some(imported) => imported.texture,
        None => {
            let imported = gl.import_image(image)?;
            state.images.push(imported);
            imported.texture
        }
    };
    let fd = image.fds.export_semaphore()?;
    let semaphore = gl.wait(fd, texture)?;
    Ok((texture, semaphore))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    struct NoFrames;

    impl FrameSource for NoFrames {
        fn acquire(&self) -> Option<Acquired> {
            None
        }
    }

    /// Counts how often it was dropped.
    struct Held(Arc<AtomicUsize>);

    impl Drop for Held {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn retire_gives_the_frame_back_and_leaves_the_objects_to_a_populate() {
        let raster = Raster::new(Arc::new(NoFrames));
        let dropped = Arc::new(AtomicUsize::new(0));
        {
            let mut state = lock(&raster.state);
            state.images.push(Imported {
                image_id: 1,
                image_set: 1,
                memory: 10,
                texture: 11,
            });
            state.pixels = Some(PixelTexture {
                name: 20,
                size: (64, 32),
            });
            state.semaphore = 30;
            state.shown = Some(Shown {
                frame_id: 5,
                texture: 11,
                width: 64,
                height: 32,
                image_id: Some(1),
                keep: Some(Box::new(Held(Arc::clone(&dropped)))),
            });
        }
        drop(std::mem::take(&mut *lock(&GARBAGE)));

        raster.retire();
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
        assert_eq!(
            std::mem::take(&mut *lock(&GARBAGE)),
            Garbage {
                textures: vec![11, 20],
                memory_objects: vec![10],
                semaphores: vec![30],
            }
        );
        let state = lock(&raster.state);
        assert!(state.retired && state.shown.is_none() && state.images.is_empty());
        drop(state);

        // Disposing after unregistering retires again: nothing more to give back.
        raster.retire();
        assert!(lock(&GARBAGE).is_empty());
    }
}
