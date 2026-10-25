//! EGL and OpenGL ES, looked up at run time: the engine renders through libepoxy, which has loaded
//! `libEGL.so.1` (and `libGLESv2.so.2`) already. Functions come from `eglGetProcAddress`, core
//! OpenGL ones from `libGLESv2.so.2` where it gives none.
//!
//! - [`probe`] reads, on the platform thread, which GPU and driver a context like Flutter's runs
//!   on, and whether it can import Vulkan memory and semaphores (a port of the probe in the
//!   runner's former `visualizer_texture.cc`).
//! - [`Gl`] imports Vulkan images, waits for their semaphores and uploads pixels, inside a
//!   populate, with Flutter's context current.
//!
//! The constants and signatures are those of Khronos' `EGL/egl.h`, `EGL/eglext.h`, `GLES2/gl2.h`,
//! `GLES3/gl3.h` and `GLES2/gl2ext.h` (`GL_EXT_memory_object`, `GL_EXT_memory_object_fd`,
//! `GL_EXT_semaphore`, `GL_EXT_semaphore_fd`).

use std::cell::OnceCell;
use std::ffi::{CStr, c_char, c_void};
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};
use std::sync::OnceLock;

use log::{info, warn};

use super::api::{self, Api};
use super::logic::{self, Garbage, Imported, Rows};
use crate::thread::MainContext;
use crate::{DeviceUuids, PixelFormat, Pixels, VulkanImage};

type GLenum = u32;
type GLuint = u32;
type GLint = i32;
type GLsizei = i32;

const GL_NO_ERROR: GLenum = 0;
const GL_TRUE: GLint = 1;
const GL_EXTENSIONS: GLenum = 0x1F03;
const GL_RENDERER: GLenum = 0x1F01;
const GL_VERSION: GLenum = 0x1F02;
pub(super) const GL_TEXTURE_2D: GLenum = 0x0DE1;
const GL_TEXTURE_BINDING_2D: GLenum = 0x8069;
const GL_TEXTURE_MIN_FILTER: GLenum = 0x2801;
const GL_TEXTURE_MAG_FILTER: GLenum = 0x2800;
const GL_TEXTURE_WRAP_S: GLenum = 0x2802;
const GL_TEXTURE_WRAP_T: GLenum = 0x2803;
const GL_LINEAR: GLint = 0x2601;
const GL_CLAMP_TO_EDGE: GLint = 0x812F;
const GL_RGBA: GLenum = 0x1908;
const GL_RGBA8: GLenum = 0x8058;
const GL_UNSIGNED_BYTE: GLenum = 0x1401;
const GL_UNPACK_ROW_LENGTH: GLenum = 0x0CF2;
const GL_TEXTURE_TILING_EXT: GLenum = 0x9580;
const GL_DEDICATED_MEMORY_OBJECT_EXT: GLenum = 0x9581;
const GL_OPTIMAL_TILING_EXT: GLint = 0x9584;
const GL_HANDLE_TYPE_OPAQUE_FD_EXT: GLenum = 0x9586;
const GL_LAYOUT_SHADER_READ_ONLY_EXT: GLenum = 0x9591;
const GL_NUM_DEVICE_UUIDS_EXT: GLenum = 0x9596;
const GL_DEVICE_UUID_EXT: GLenum = 0x9597;
const GL_DRIVER_UUID_EXT: GLenum = 0x9598;

type EGLDisplay = *mut c_void;
type EGLContext = *mut c_void;
type EGLSurface = *mut c_void;
type EGLConfig = *mut c_void;
type EGLint = i32;
type EGLBoolean = u32;
type EGLenum = u32;

const EGL_TRUE: EGLBoolean = 1;
const EGL_NONE: EGLint = 0x3038;
const EGL_DRAW: EGLint = 0x3059;
const EGL_READ: EGLint = 0x305A;
const EGL_RENDERABLE_TYPE: EGLint = 0x3040;
const EGL_OPENGL_ES2_BIT: EGLint = 0x0004;
const EGL_CONTEXT_CLIENT_VERSION: EGLint = 0x3098;
const EGL_OPENGL_ES_API: EGLenum = 0x30A0;
const EGL_PLATFORM_X11_EXT: EGLenum = 0x31D5;
const EGL_PLATFORM_WAYLAND_EXT: EGLenum = 0x31D8;

/// What OpenGL needs to import Vulkan memory and semaphores from file descriptors.
const IMPORT_EXTENSIONS: [&str; 4] = [
    "GL_EXT_memory_object",
    "GL_EXT_memory_object_fd",
    "GL_EXT_semaphore",
    "GL_EXT_semaphore_fd",
];

/// A library the engine loaded (else loaded here), kept for the life of the process.
fn library(name: &CStr) -> Option<*mut c_void> {
    // SAFETY: plain calls with a NUL-terminated name; the handle is never closed.
    let handle = unsafe {
        let loaded = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_NOLOAD);
        if loaded.is_null() {
            libc::dlopen(name.as_ptr(), libc::RTLD_NOW)
        } else {
            loaded
        }
    };
    (!handle.is_null()).then_some(handle)
}

/// The function `name` of the library `handle`, as the function pointer type `F`.
///
/// `F` must be the signature the library declares for `name`.
fn symbol<F: Copy>(handle: *mut c_void, name: &CStr) -> Option<F> {
    // SAFETY: a live handle and a NUL-terminated name.
    let address = unsafe { libc::dlsym(handle, name.as_ptr()) };
    // SAFETY: a function's address, which the caller says has the signature `F`.
    (!address.is_null()).then(|| unsafe { api::function(address) })
}

/// The EGL functions.
struct Egl {
    get_proc_address: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    initialize: unsafe extern "C" fn(EGLDisplay, *mut EGLint, *mut EGLint) -> EGLBoolean,
    query_api: unsafe extern "C" fn() -> EGLenum,
    bind_api: unsafe extern "C" fn(EGLenum) -> EGLBoolean,
    get_current_display: unsafe extern "C" fn() -> EGLDisplay,
    get_current_context: unsafe extern "C" fn() -> EGLContext,
    get_current_surface: unsafe extern "C" fn(EGLint) -> EGLSurface,
    choose_config: unsafe extern "C" fn(
        EGLDisplay,
        *const EGLint,
        *mut EGLConfig,
        EGLint,
        *mut EGLint,
    ) -> EGLBoolean,
    create_context:
        unsafe extern "C" fn(EGLDisplay, EGLConfig, EGLContext, *const EGLint) -> EGLContext,
    make_current:
        unsafe extern "C" fn(EGLDisplay, EGLSurface, EGLSurface, EGLContext) -> EGLBoolean,
    destroy_context: unsafe extern "C" fn(EGLDisplay, EGLContext) -> EGLBoolean,
    get_error: unsafe extern "C" fn() -> EGLint,
    /// `eglGetPlatformDisplayEXT`, as the engine gets its display (`fl_opengl_manager.cc`).
    get_platform_display:
        Option<unsafe extern "C" fn(EGLenum, *mut c_void, *const EGLint) -> EGLDisplay>,
}

// SAFETY: plain function pointers.
unsafe impl Send for Egl {}
// SAFETY: as above.
unsafe impl Sync for Egl {}

impl Egl {
    fn load() -> Result<Self, String> {
        let handle = library(c"libEGL.so.1").ok_or("libEGL.so.1 cannot be loaded")?;
        let get_proc_address: unsafe extern "C" fn(*const c_char) -> *mut c_void =
            egl_symbol(handle, c"eglGetProcAddress")?;
        Ok(Self {
            get_proc_address,
            initialize: egl_symbol(handle, c"eglInitialize")?,
            query_api: egl_symbol(handle, c"eglQueryAPI")?,
            bind_api: egl_symbol(handle, c"eglBindAPI")?,
            get_current_display: egl_symbol(handle, c"eglGetCurrentDisplay")?,
            get_current_context: egl_symbol(handle, c"eglGetCurrentContext")?,
            get_current_surface: egl_symbol(handle, c"eglGetCurrentSurface")?,
            choose_config: egl_symbol(handle, c"eglChooseConfig")?,
            create_context: egl_symbol(handle, c"eglCreateContext")?,
            make_current: egl_symbol(handle, c"eglMakeCurrent")?,
            destroy_context: egl_symbol(handle, c"eglDestroyContext")?,
            get_error: egl_symbol(handle, c"eglGetError")?,
            get_platform_display: proc_address(get_proc_address, c"eglGetPlatformDisplayEXT"),
        })
    }
}

/// The function `name` of libEGL, as the function pointer type `F`.
///
/// `F` must be the signature Khronos declares for `name`.
fn egl_symbol<F: Copy>(handle: *mut c_void, name: &CStr) -> Result<F, String> {
    symbol(handle, name).ok_or_else(|| format!("libEGL has no {}", name.to_string_lossy()))
}

/// The function `name` from `eglGetProcAddress`, as the function pointer type `F`.
///
/// `F` must be the signature Khronos declares for `name`.
fn proc_address<F: Copy>(
    get_proc_address: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    name: &CStr,
) -> Option<F> {
    // SAFETY: a NUL-terminated name.
    let address = unsafe { get_proc_address(name.as_ptr()) };
    // SAFETY: a function's address, which the caller says has the signature `F`.
    (!address.is_null()).then(|| unsafe { api::function(address) })
}

fn egl() -> Result<&'static Egl, String> {
    static EGL: OnceLock<Result<Egl, String>> = OnceLock::new();
    EGL.get_or_init(Egl::load).as_ref().map_err(Clone::clone)
}

/// The OpenGL ES functions. The ones of the import extensions may be there although the context
/// lacks the extensions (`eglGetProcAddress` may return a stub for any name), so they are only
/// called after [`Gl::imports_vulkan`].
pub(super) struct Gl {
    get_string: unsafe extern "C" fn(GLenum) -> *const u8,
    get_error: unsafe extern "C" fn() -> GLenum,
    get_integerv: unsafe extern "C" fn(GLenum, *mut GLint),
    gen_textures: unsafe extern "C" fn(GLsizei, *mut GLuint),
    delete_textures: unsafe extern "C" fn(GLsizei, *const GLuint),
    bind_texture: unsafe extern "C" fn(GLenum, GLuint),
    tex_parameteri: unsafe extern "C" fn(GLenum, GLenum, GLint),
    tex_image_2d: unsafe extern "C" fn(
        GLenum,
        GLint,
        GLint,
        GLsizei,
        GLsizei,
        GLint,
        GLenum,
        GLenum,
        *const c_void,
    ),
    tex_sub_image_2d: unsafe extern "C" fn(
        GLenum,
        GLint,
        GLint,
        GLint,
        GLsizei,
        GLsizei,
        GLenum,
        GLenum,
        *const c_void,
    ),
    pixel_storei: unsafe extern "C" fn(GLenum, GLint),
    ext: Option<Ext>,
}

/// The functions of the import extensions.
struct Ext {
    get_unsigned_bytev: unsafe extern "C" fn(GLenum, *mut u8),
    get_unsigned_bytei_v: unsafe extern "C" fn(GLenum, GLuint, *mut u8),
    create_memory_objects: unsafe extern "C" fn(GLsizei, *mut GLuint),
    delete_memory_objects: unsafe extern "C" fn(GLsizei, *const GLuint),
    memory_object_parameteriv: unsafe extern "C" fn(GLuint, GLenum, *const GLint),
    import_memory_fd: unsafe extern "C" fn(GLuint, u64, GLenum, GLint),
    tex_storage_mem_2d:
        unsafe extern "C" fn(GLenum, GLsizei, GLenum, GLsizei, GLsizei, GLuint, u64),
    gen_semaphores: unsafe extern "C" fn(GLsizei, *mut GLuint),
    delete_semaphores: unsafe extern "C" fn(GLsizei, *const GLuint),
    import_semaphore_fd: unsafe extern "C" fn(GLuint, GLenum, GLint),
    wait_semaphore:
        unsafe extern "C" fn(GLuint, GLuint, *const GLuint, GLuint, *const GLuint, *const GLenum),
}

// SAFETY: plain function pointers.
unsafe impl Send for Gl {}
// SAFETY: as above.
unsafe impl Sync for Gl {}

impl Gl {
    fn load() -> Result<Self, String> {
        let loader = Loader {
            get_proc_address: egl()?.get_proc_address,
            gles: OnceCell::new(),
        };
        let ext = (|| {
            Some(Ext {
                get_unsigned_bytev: loader.extension(c"glGetUnsignedBytevEXT")?,
                get_unsigned_bytei_v: loader.extension(c"glGetUnsignedBytei_vEXT")?,
                create_memory_objects: loader.extension(c"glCreateMemoryObjectsEXT")?,
                delete_memory_objects: loader.extension(c"glDeleteMemoryObjectsEXT")?,
                memory_object_parameteriv: loader.extension(c"glMemoryObjectParameterivEXT")?,
                import_memory_fd: loader.extension(c"glImportMemoryFdEXT")?,
                tex_storage_mem_2d: loader.extension(c"glTexStorageMem2DEXT")?,
                gen_semaphores: loader.extension(c"glGenSemaphoresEXT")?,
                delete_semaphores: loader.extension(c"glDeleteSemaphoresEXT")?,
                import_semaphore_fd: loader.extension(c"glImportSemaphoreFdEXT")?,
                wait_semaphore: loader.extension(c"glWaitSemaphoreEXT")?,
            })
        })();
        Ok(Self {
            get_string: loader.core(c"glGetString")?,
            get_error: loader.core(c"glGetError")?,
            get_integerv: loader.core(c"glGetIntegerv")?,
            gen_textures: loader.core(c"glGenTextures")?,
            delete_textures: loader.core(c"glDeleteTextures")?,
            bind_texture: loader.core(c"glBindTexture")?,
            tex_parameteri: loader.core(c"glTexParameteri")?,
            tex_image_2d: loader.core(c"glTexImage2D")?,
            tex_sub_image_2d: loader.core(c"glTexSubImage2D")?,
            pixel_storei: loader.core(c"glPixelStorei")?,
            ext,
        })
    }

    /// The string `name` of the current context, empty if there is none.
    fn string(&self, name: GLenum) -> String {
        // SAFETY: a plain query; the result is null or a static NUL-terminated string.
        let string = unsafe { (self.get_string)(name) };
        if string.is_null() {
            return String::new();
        }
        // SAFETY: as above.
        unsafe { CStr::from_ptr(string.cast()) }
            .to_string_lossy()
            .into_owned()
    }

    fn error(&self) -> GLenum {
        // SAFETY: a plain query of the current context.
        unsafe { (self.get_error)() }
    }

    /// Discard errors raised before, so that the next check sees only its own. Bounded: a lost
    /// context can report errors forever.
    fn clear_errors(&self) {
        for _ in 0..16 {
            if self.error() == GL_NO_ERROR {
                break;
            }
        }
    }

    /// The texture bound to `GL_TEXTURE_2D`, to bind again after ours.
    fn bound_texture(&self) -> GLuint {
        let mut bound = 0;
        // SAFETY: a plain query into a local.
        unsafe { (self.get_integerv)(GL_TEXTURE_BINDING_2D, &mut bound) };
        bound as GLuint
    }

    /// The functions of the import extensions, if the current context has the extensions.
    /// Logs what it lacks.
    fn import_functions(&self) -> Option<&Ext> {
        let extensions = self.string(GL_EXTENSIONS);
        for extension in IMPORT_EXTENSIONS {
            if !logic::has_extension(&extensions, extension) {
                info!("Flutter texture: OpenGL lacks {extension}; frames are read back");
                return None;
            }
        }
        self.ext.as_ref()
    }

    /// Whether the current context imports Vulkan memory and semaphores from file descriptors.
    pub(super) fn imports_vulkan(&self) -> bool {
        self.import_functions().is_some()
    }

    fn ext(&self) -> Result<&Ext, String> {
        self.ext
            .as_ref()
            .ok_or_else(|| "OpenGL has no functions to import Vulkan memory".into())
    }

    /// Delete what textures left behind. Flutter's context must be current.
    pub(super) fn delete(&self, garbage: Garbage) {
        if garbage.is_empty() {
            return;
        }
        // SAFETY: names of this context, each deleted once; textures before the memory they are
        // stored in. Memory objects and semaphores exist only if the extension functions do.
        unsafe {
            if !garbage.textures.is_empty() {
                (self.delete_textures)(len(&garbage.textures), garbage.textures.as_ptr());
            }
            if let Some(ext) = &self.ext {
                if !garbage.memory_objects.is_empty() {
                    (ext.delete_memory_objects)(
                        len(&garbage.memory_objects),
                        garbage.memory_objects.as_ptr(),
                    );
                }
                if !garbage.semaphores.is_empty() {
                    (ext.delete_semaphores)(len(&garbage.semaphores), garbage.semaphores.as_ptr());
                }
            }
        }
    }

    /// Import the memory of `image` and make a texture on it. Flutter's context must be current,
    /// and have the import extensions.
    pub(super) fn import_image(&self, image: &VulkanImage) -> Result<Imported, String> {
        let ext = self.ext()?;
        let (width, height) = size(image.width, image.height)?;
        let fd = image.fds.export_memory()?;
        self.clear_errors();
        let mut memory = 0;
        // SAFETY: plain calls on the current context with valid pointers. Only a successful import
        // takes the descriptor over; otherwise it is closed here.
        unsafe {
            (ext.create_memory_objects)(1, &mut memory);
            // Vulkan allocated the memory for the image alone.
            (ext.memory_object_parameteriv)(memory, GL_DEDICATED_MEMORY_OBJECT_EXT, &GL_TRUE);
            let raw = fd.into_raw_fd();
            (ext.import_memory_fd)(memory, image.memory_size, GL_HANDLE_TYPE_OPAQUE_FD_EXT, raw);
            let error = self.error();
            if error != GL_NO_ERROR {
                drop(OwnedFd::from_raw_fd(raw));
                (ext.delete_memory_objects)(1, &memory);
                return Err(format!(
                    "OpenGL cannot import the memory of image {} (error 0x{error:x})",
                    image.image_id
                ));
            }
        }

        let bound = self.bound_texture();
        let mut texture = 0;
        // SAFETY: plain calls on the current context; the memory holds the image at offset 0.
        let error = unsafe {
            (self.gen_textures)(1, &mut texture);
            (self.bind_texture)(GL_TEXTURE_2D, texture);
            // Matches the image's VK_IMAGE_TILING_OPTIMAL (also the default).
            (self.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_TILING_EXT, GL_OPTIMAL_TILING_EXT);
            self.set_sampling();
            (ext.tex_storage_mem_2d)(GL_TEXTURE_2D, 1, GL_RGBA8, width, height, memory, 0);
            let error = self.error();
            (self.bind_texture)(GL_TEXTURE_2D, bound);
            error
        };
        if error != GL_NO_ERROR {
            // SAFETY: the names just made, deleted once.
            unsafe {
                (self.delete_textures)(1, &texture);
                (ext.delete_memory_objects)(1, &memory);
            }
            return Err(format!(
                "OpenGL cannot make a texture on image {} (error 0x{error:x})",
                image.image_id
            ));
        }
        Ok(Imported {
            image_id: image.image_id,
            image_set: image.image_set,
            memory,
            texture,
        })
    }

    /// Wait for the semaphore `fd` before `texture` is sampled, which makes Vulkan's writes
    /// visible and tells OpenGL the layout Vulkan left the image in
    /// (`VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL`). Returns the semaphore, to delete after the
    /// next frame. Flutter's context must be current, and have the import extensions.
    pub(super) fn wait(&self, fd: OwnedFd, texture: GLuint) -> Result<GLuint, String> {
        let ext = self.ext()?;
        self.clear_errors();
        let mut semaphore = 0;
        // SAFETY: plain calls on the current context with valid pointers. Only a successful import
        // takes the descriptor over; otherwise it is closed here.
        unsafe {
            (ext.gen_semaphores)(1, &mut semaphore);
            let raw = fd.into_raw_fd();
            (ext.import_semaphore_fd)(semaphore, GL_HANDLE_TYPE_OPAQUE_FD_EXT, raw);
            if self.error() != GL_NO_ERROR {
                drop(OwnedFd::from_raw_fd(raw));
                (ext.delete_semaphores)(1, &semaphore);
                return Err("OpenGL cannot import a semaphore".into());
            }
            let layout = GL_LAYOUT_SHADER_READ_ONLY_EXT;
            (ext.wait_semaphore)(semaphore, 0, std::ptr::null(), 1, &texture, &layout);
            if self.error() != GL_NO_ERROR {
                (ext.delete_semaphores)(1, &semaphore);
                return Err("OpenGL cannot wait for a semaphore".into());
            }
        }
        Ok(semaphore)
    }

    /// Delete a semaphore whose wait was issued. Flutter's context must be current.
    pub(super) fn delete_semaphore(&self, semaphore: GLuint) {
        if let (Some(ext), true) = (&self.ext, semaphore != 0) {
            // SAFETY: a name of this context, deleted once.
            unsafe { (ext.delete_semaphores)(1, &semaphore) };
        }
    }

    /// Delete images no longer shown. Flutter's context must be current.
    pub(super) fn delete_images(&self, images: Vec<Imported>) {
        let mut garbage = Garbage::default();
        garbage.add_images(images);
        self.delete(garbage);
    }

    /// Upload `pixels` into `texture` (made, or reallocated for a new size, as needed). Returns
    /// the texture's name. Flutter's context must be current.
    pub(super) fn upload(
        &self,
        texture: &mut Option<PixelTexture>,
        pixels: &Pixels,
    ) -> Result<GLuint, String> {
        if pixels.format() != PixelFormat::Rgba8 {
            return Err("the engine takes RGBA pixels only".into());
        }
        let bytes = pixels.bytes().ok_or("the pixels are empty")?;
        let (width, height) = size(pixels.width(), pixels.height())?;
        self.clear_errors();
        let bound = self.bound_texture();
        let current = texture.get_or_insert_with(|| {
            let mut name = 0;
            // SAFETY: plain calls on the current context.
            unsafe {
                (self.gen_textures)(1, &mut name);
                (self.bind_texture)(GL_TEXTURE_2D, name);
                self.set_sampling();
            }
            PixelTexture { name, size: (0, 0) }
        });
        // SAFETY: plain calls on the current context. `bytes` holds `height` rows of `stride`
        // bytes, the last one at least `width * 4`, which is what each upload reads; OpenGL copies
        // them before the call returns.
        let error = unsafe {
            (self.bind_texture)(GL_TEXTURE_2D, current.name);
            if current.size != (width, height) {
                // Unsized RGBA of bytes: RGBA8 in OpenGL ES 2 and 3 alike.
                (self.tex_image_2d)(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA as GLint,
                    width,
                    height,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    std::ptr::null(),
                );
                current.size = (width, height);
            }
            let upload = |y: GLint, rows: GLsizei, data: *const u8| {
                (self.tex_sub_image_2d)(
                    GL_TEXTURE_2D,
                    0,
                    0,
                    y,
                    width,
                    rows,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    data.cast(),
                )
            };
            let each_row = || {
                for y in 0..height {
                    upload(y, 1, bytes[y as usize * pixels.stride()..].as_ptr());
                }
            };
            match logic::rows(pixels.width(), pixels.stride()) {
                Rows::Tight => upload(0, height, bytes.as_ptr()),
                Rows::RowLength(length) => {
                    (self.pixel_storei)(GL_UNPACK_ROW_LENGTH, length);
                    if self.error() == GL_NO_ERROR {
                        upload(0, height, bytes.as_ptr());
                        (self.pixel_storei)(GL_UNPACK_ROW_LENGTH, 0);
                    } else {
                        // OpenGL ES 2 without GL_EXT_unpack_subimage.
                        each_row();
                    }
                }
                Rows::EachRow => each_row(),
            }
            let error = self.error();
            (self.bind_texture)(GL_TEXTURE_2D, bound);
            error
        };
        if error != GL_NO_ERROR {
            // Allocated again next time.
            current.size = (0, 0);
            return Err(format!("OpenGL cannot upload pixels (error 0x{error:x})"));
        }
        Ok(current.name)
    }

    /// Linear sampling, clamped, of the texture bound to `GL_TEXTURE_2D`.
    ///
    /// # Safety
    ///
    /// A context must be current.
    unsafe fn set_sampling(&self) {
        // SAFETY: plain calls on the current context.
        unsafe {
            (self.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            (self.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            (self.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (self.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
        }
    }
}

/// The texture that frames as pixels are uploaded into, and the size it is allocated for.
pub(super) struct PixelTexture {
    pub name: GLuint,
    pub size: (GLsizei, GLsizei),
}

fn size(width: u32, height: u32) -> Result<(GLsizei, GLsizei), String> {
    match (GLsizei::try_from(width), GLsizei::try_from(height)) {
        (Ok(width), Ok(height)) if width > 0 && height > 0 => Ok((width, height)),
        _ => Err(format!("a frame of {width}x{height} cannot be a texture")),
    }
}

fn len(names: &[GLuint]) -> GLsizei {
    GLsizei::try_from(names.len()).unwrap_or(GLsizei::MAX)
}

/// Looks OpenGL functions up.
struct Loader {
    get_proc_address: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    /// `libGLESv2.so.2`, once it was needed.
    gles: OnceCell<Option<*mut c_void>>,
}

impl Loader {
    /// The core function `name`, as the function pointer type `F`: from `eglGetProcAddress`, which
    /// gives core functions only from EGL 1.5 (or `EGL_KHR_get_all_proc_addresses`) on, else
    /// from `libGLESv2.so.2`.
    ///
    /// `F` must be the signature Khronos declares for `name`.
    fn core<F: Copy>(&self, name: &CStr) -> Result<F, String> {
        proc_address(self.get_proc_address, name)
            .or_else(|| {
                symbol(
                    (*self.gles.get_or_init(|| library(c"libGLESv2.so.2")))?,
                    name,
                )
            })
            .ok_or_else(|| format!("OpenGL ES has no {}", name.to_string_lossy()))
    }

    /// The extension function `name`, as the function pointer type `F`.
    ///
    /// `F` must be the signature Khronos declares for `name`.
    fn extension<F: Copy>(&self, name: &CStr) -> Option<F> {
        proc_address(self.get_proc_address, name)
    }
}

/// The OpenGL functions, loaded once.
pub(super) fn gl() -> Result<&'static Gl, String> {
    static GL: OnceLock<Result<Gl, String>> = OnceLock::new();
    GL.get_or_init(Gl::load).as_ref().map_err(Clone::clone)
}

/// The GDK functions that tell the display GTK uses, if GDK is loaded.
struct Gdk {
    display_get_default: unsafe extern "C" fn() -> *mut c_void,
    /// Each backend is there only if GDK was built with it.
    wayland: Option<(
        unsafe extern "C" fn() -> api::GType,
        unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    )>,
    x11: Option<(
        unsafe extern "C" fn() -> api::GType,
        unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    )>,
}

impl Gdk {
    fn load() -> Option<Self> {
        let backend = |get_type: &CStr, native: &CStr| {
            Some((
                api::lookup_optional(get_type)?,
                api::lookup_optional(native)?,
            ))
        };
        Some(Self {
            display_get_default: api::lookup_optional(c"gdk_display_get_default")?,
            wayland: backend(
                c"gdk_wayland_display_get_type",
                c"gdk_wayland_display_get_wl_display",
            ),
            x11: backend(c"gdk_x11_display_get_type", c"gdk_x11_display_get_xdisplay"),
        })
    }
}

/// The EGL display the engine renders on (as `fl_opengl_manager.cc` gets it): EGL hands out one
/// display per platform and native display.
fn flutter_display(api: &Api, egl: &Egl) -> Option<EGLDisplay> {
    let get_platform_display = egl.get_platform_display?;
    let gdk = Gdk::load()?;
    // SAFETY: plain calls on the platform thread; the display is GTK's, alive while it runs.
    unsafe {
        let display = (gdk.display_get_default)();
        if display.is_null() {
            return None;
        }
        let is = |get_type: unsafe extern "C" fn() -> api::GType| {
            (api.type_check_instance_is_a)(display, get_type()) != 0
        };
        let (platform, native) = match (gdk.wayland, gdk.x11) {
            (Some((get_type, native)), _) if is(get_type) => (EGL_PLATFORM_WAYLAND_EXT, native),
            (_, Some((get_type, native))) if is(get_type) => (EGL_PLATFORM_X11_EXT, native),
            _ => return None,
        };
        let display = get_platform_display(platform, native(display), std::ptr::null());
        (!display.is_null()).then_some(display)
    }
}

/// The GPU and driver of a context made like Flutter's on Flutter's EGL display (OpenGL ES 2
/// requested, no surface), if it imports Vulkan memory and semaphores from file descriptors and
/// runs on one GPU. Restores whatever context this thread had current.
pub(super) fn probe(api: &Api, _context: MainContext) -> Option<DeviceUuids> {
    if std::env::var_os("FLUTTER_LINUX_RENDERER").is_some_and(|renderer| renderer == "software") {
        info!("Flutter texture: Flutter renders in software; frames are read back");
        return None;
    }
    let (egl, gl) = match egl().and_then(|egl| Ok((egl, gl()?))) {
        Ok(loaded) => loaded,
        Err(e) => {
            warn!("Flutter texture: {e}; frames are read back");
            return None;
        }
    };
    // SAFETY: plain EGL calls on the platform thread with valid pointers. The display is never
    // terminated here (the engine uses it); initialising it again does nothing. The context made
    // here is destroyed, and the one current before is made current again.
    unsafe {
        let Some(display) = flutter_display(api, egl).filter(|&display| {
            (egl.initialize)(display, std::ptr::null_mut(), std::ptr::null_mut()) == EGL_TRUE
        }) else {
            info!("Flutter texture: no EGL display; frames are read back");
            return None;
        };
        // The current context is tracked per client API, so the API is restored first.
        let previous_api = (egl.query_api)();
        let previous_display = (egl.get_current_display)();
        let previous_context = (egl.get_current_context)();
        let previous_draw = (egl.get_current_surface)(EGL_DRAW);
        let previous_read = (egl.get_current_surface)(EGL_READ);

        (egl.bind_api)(EGL_OPENGL_ES_API);
        let config_attributes = [EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE];
        let context_attributes = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
        let mut config = std::ptr::null_mut();
        let mut configs = 0;
        let mut context = std::ptr::null_mut();
        if (egl.choose_config)(
            display,
            config_attributes.as_ptr(),
            &mut config,
            1,
            &mut configs,
        ) == EGL_TRUE
            && configs > 0
        {
            context = (egl.create_context)(
                display,
                config,
                std::ptr::null_mut(),
                context_attributes.as_ptr(),
            );
        }
        let none = std::ptr::null_mut();
        let mut uuids = None;
        // Without a surface, as the engine does (EGL_KHR_surfaceless_context).
        if !context.is_null() && (egl.make_current)(display, none, none, context) == EGL_TRUE {
            uuids = probe_current(gl);
            (egl.make_current)(display, none, none, none);
        } else {
            info!(
                "Flutter texture: cannot probe OpenGL (EGL error 0x{:x}); frames are read back",
                (egl.get_error)()
            );
        }
        if !context.is_null() {
            (egl.destroy_context)(display, context);
        }

        (egl.bind_api)(previous_api);
        if !previous_context.is_null() {
            (egl.make_current)(
                previous_display,
                previous_draw,
                previous_read,
                previous_context,
            );
        }
        uuids
    }
}

/// What the context current on this thread supports.
fn probe_current(gl: &Gl) -> Option<DeviceUuids> {
    info!(
        "Flutter texture: OpenGL renderer {}, version {}",
        gl.string(GL_RENDERER),
        gl.string(GL_VERSION)
    );
    let ext = gl.import_functions()?;
    gl.clear_errors();
    let mut devices = 0;
    let mut uuids = DeviceUuids {
        device: [0; 16],
        driver: [0; 16],
    };
    // SAFETY: plain queries of the current context, which has the extensions, into locals of the
    // size `GL_UUID_SIZE_EXT` (16).
    unsafe {
        (gl.get_integerv)(GL_NUM_DEVICE_UUIDS_EXT, &mut devices);
        // A context on several GPUs (a device group) is not supported.
        if devices != 1 {
            info!("Flutter texture: OpenGL runs on {devices} GPUs; frames are read back");
            return None;
        }
        (ext.get_unsigned_bytei_v)(GL_DEVICE_UUID_EXT, 0, uuids.device.as_mut_ptr());
        (ext.get_unsigned_bytev)(GL_DRIVER_UUID_EXT, uuids.driver.as_mut_ptr());
    }
    (gl.error() == GL_NO_ERROR).then_some(uuids)
}
