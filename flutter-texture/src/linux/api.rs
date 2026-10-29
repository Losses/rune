//! What the texture layer calls of GLib, GObject and the engine, looked up at run time in what
//! the process has loaded (`dlsym(RTLD_DEFAULT)`): the runner links `libflutter_linux_gtk.so`,
//! which brings GTK, GDK, GObject and GLib. Nothing links against them, so the hub's other
//! binaries do not depend on them.
//!
//! The declarations and layouts mirror GLib 2.x's `gobject/gtype.h`, `gobject/gobject.h` and
//! `glib/gmain.h`, and Flutter 3.47.4's `shell/platform/linux/public/flutter_linux/fl_engine.h`,
//! `fl_texture.h`, `fl_texture_gl.h` and `fl_texture_registrar.h`.

use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::sync::OnceLock;

use log::error;

use crate::{Result, TextureError};

/// `GType`.
pub(crate) type GType = usize;
/// `gboolean`.
pub(crate) type Gboolean = c_int;
/// `gpointer`.
pub(crate) type Gpointer = *mut c_void;
/// `GSourceFunc`.
pub(crate) type SourceFunc = unsafe extern "C" fn(Gpointer) -> Gboolean;
/// `GDestroyNotify`.
pub(crate) type DestroyNotify = Option<unsafe extern "C" fn(Gpointer)>;
/// `GClassInitFunc`.
pub(crate) type ClassInitFunc = unsafe extern "C" fn(Gpointer, Gpointer);
/// `GInstanceInitFunc`.
pub(crate) type InstanceInitFunc = unsafe extern "C" fn(Gpointer, Gpointer);
/// `GObjectClass::dispose` and `finalize`.
pub(crate) type ObjectFunc = unsafe extern "C" fn(Gpointer);
/// `FlTextureGLClass::populate`.
pub(crate) type PopulateFunc = unsafe extern "C" fn(
    texture: Gpointer,
    target: *mut u32,
    name: *mut u32,
    width: *mut u32,
    height: *mut u32,
    error: *mut *mut c_void,
) -> Gboolean;

pub(crate) const FALSE: Gboolean = 0;
pub(crate) const TRUE: Gboolean = 1;
/// `G_SOURCE_REMOVE`.
pub(crate) const SOURCE_REMOVE: Gboolean = 0;
/// `G_PRIORITY_DEFAULT`.
pub(crate) const PRIORITY_DEFAULT: c_int = 0;

/// `GTypeQuery`.
#[repr(C)]
pub(crate) struct TypeQuery {
    pub type_: GType,
    pub type_name: *const c_char,
    pub class_size: c_uint,
    pub instance_size: c_uint,
}

/// `GTypeInfo`.
#[repr(C)]
pub(crate) struct TypeInfo {
    pub class_size: u16,
    pub base_init: Option<unsafe extern "C" fn(Gpointer)>,
    pub base_finalize: Option<unsafe extern "C" fn(Gpointer)>,
    pub class_init: Option<ClassInitFunc>,
    pub class_finalize: Option<ClassInitFunc>,
    pub class_data: *const c_void,
    pub instance_size: u16,
    pub n_preallocs: u16,
    pub instance_init: Option<InstanceInitFunc>,
    pub value_table: *const c_void,
}

/// `GObjectClass`: the head of every GObject class structure. `FlTextureGLClass` adds `populate`
/// right after it.
#[repr(C)]
pub(crate) struct ObjectClass {
    pub g_type: GType,
    pub construct_properties: Gpointer,
    pub constructor: Gpointer,
    pub set_property: Gpointer,
    pub get_property: Gpointer,
    pub dispose: Option<ObjectFunc>,
    pub finalize: Option<ObjectFunc>,
    pub dispatch_properties_changed: Gpointer,
    pub notify: Gpointer,
    pub constructed: Gpointer,
    pub flags: usize,
    pub n_construct_properties: usize,
    pub pspecs: Gpointer,
    pub n_pspecs: usize,
    pub pdummy: [Gpointer; 3],
}

/// The functions the texture layer calls.
pub(crate) struct Api {
    /// `g_type_query`.
    pub type_query: unsafe extern "C" fn(GType, *mut TypeQuery),
    /// `g_type_register_static`.
    pub type_register_static:
        unsafe extern "C" fn(GType, *const c_char, *const TypeInfo, c_uint) -> GType,
    /// `g_type_class_peek_parent`.
    pub type_class_peek_parent: unsafe extern "C" fn(Gpointer) -> Gpointer,
    /// `g_type_check_instance_is_a`.
    pub type_check_instance_is_a: unsafe extern "C" fn(Gpointer, GType) -> Gboolean,
    /// `g_object_new_with_properties` (GLib 2.54), the one constructor without varargs.
    pub object_new_with_properties:
        unsafe extern "C" fn(GType, c_uint, *const *const c_char, *const c_void) -> Gpointer,
    /// `g_object_ref`.
    pub object_ref: unsafe extern "C" fn(Gpointer) -> Gpointer,
    /// `g_object_unref`.
    pub object_unref: unsafe extern "C" fn(Gpointer),
    /// `g_main_context_default`.
    pub main_context_default: unsafe extern "C" fn() -> Gpointer,
    /// `g_main_context_is_owner`.
    pub main_context_is_owner: unsafe extern "C" fn(Gpointer) -> Gboolean,
    /// `g_idle_add_full`: attaches to the default main context.
    pub idle_add_full: unsafe extern "C" fn(c_int, SourceFunc, Gpointer, DestroyNotify) -> c_uint,
    /// `g_timeout_add_full`: attaches to the default main context.
    pub timeout_add_full:
        unsafe extern "C" fn(c_int, c_uint, SourceFunc, Gpointer, DestroyNotify) -> c_uint,
    /// `g_quark_from_static_string`.
    pub quark_from_static_string: unsafe extern "C" fn(*const c_char) -> u32,
    /// `g_set_error_literal`.
    pub set_error_literal: unsafe extern "C" fn(*mut *mut c_void, u32, c_int, *const c_char),
    /// `fl_engine_get_texture_registrar`: borrowed, owned by the engine.
    pub engine_get_texture_registrar: unsafe extern "C" fn(Gpointer) -> Gpointer,
    /// `fl_texture_registrar_register_texture`: false unless the texture is an `FlTextureGL` or
    /// an `FlPixelBufferTexture`, or once the engine is gone. The registrar holds a reference to
    /// the texture while it is registered.
    pub texture_registrar_register_texture: unsafe extern "C" fn(Gpointer, Gpointer) -> Gboolean,
    /// `fl_texture_registrar_mark_texture_frame_available`.
    pub texture_registrar_mark_texture_frame_available:
        unsafe extern "C" fn(Gpointer, Gpointer) -> Gboolean,
    /// `fl_texture_registrar_unregister_texture`.
    pub texture_registrar_unregister_texture: unsafe extern "C" fn(Gpointer, Gpointer) -> Gboolean,
    /// `fl_texture_get_id`.
    pub texture_get_id: unsafe extern "C" fn(Gpointer) -> i64,
    /// `fl_texture_gl_get_type`.
    pub texture_gl_get_type: unsafe extern "C" fn() -> GType,
}

/// The functions, looked up once they are found.
pub(crate) fn api() -> Result<&'static Api> {
    static API: OnceLock<Api> = OnceLock::new();
    if let Some(api) = API.get() {
        return Ok(api);
    }
    // A failed lookup is tried again later: the engine may not be loaded yet.
    let loaded = Api::load()?;
    Ok(API.get_or_init(|| loaded))
}

impl Api {
    fn load() -> Result<Self> {
        Ok(Self {
            type_query: lookup(c"g_type_query")?,
            type_register_static: lookup(c"g_type_register_static")?,
            type_class_peek_parent: lookup(c"g_type_class_peek_parent")?,
            type_check_instance_is_a: lookup(c"g_type_check_instance_is_a")?,
            object_new_with_properties: lookup(c"g_object_new_with_properties")?,
            object_ref: lookup(c"g_object_ref")?,
            object_unref: lookup(c"g_object_unref")?,
            main_context_default: lookup(c"g_main_context_default")?,
            main_context_is_owner: lookup(c"g_main_context_is_owner")?,
            idle_add_full: lookup(c"g_idle_add_full")?,
            timeout_add_full: lookup(c"g_timeout_add_full")?,
            quark_from_static_string: lookup(c"g_quark_from_static_string")?,
            set_error_literal: lookup(c"g_set_error_literal")?,
            engine_get_texture_registrar: lookup(c"fl_engine_get_texture_registrar")?,
            texture_registrar_register_texture: lookup(c"fl_texture_registrar_register_texture")?,
            texture_registrar_mark_texture_frame_available: lookup(
                c"fl_texture_registrar_mark_texture_frame_available",
            )?,
            texture_registrar_unregister_texture: lookup(
                c"fl_texture_registrar_unregister_texture",
            )?,
            texture_get_id: lookup(c"fl_texture_get_id")?,
            texture_gl_get_type: lookup(c"fl_texture_gl_get_type")?,
        })
    }
}

/// The function `name` from what the process has loaded, as the function pointer type `F`.
///
/// `F` must be the signature the library declares for `name` (see the header of this module).
fn lookup<F: Copy>(name: &'static CStr) -> Result<F> {
    lookup_optional(name).ok_or_else(|| {
        error!(
            "{} is not loaded (no Flutter engine in this process)",
            name.to_string_lossy()
        );
        TextureError::Unsupported
    })
}

/// The function `name` from what the process has loaded, if it is there, as the function pointer
/// type `F`.
///
/// `F` must be the signature the library declares for `name`.
pub(crate) fn lookup_optional<F: Copy>(name: &CStr) -> Option<F> {
    // SAFETY: `RTLD_DEFAULT` searches the global scope; `name` is NUL-terminated.
    let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    // SAFETY: a function's address, which the caller says has the signature `F`.
    (!address.is_null()).then(|| unsafe { function(address) })
}

/// `address` as the function pointer type `F`.
///
/// # Safety
///
/// `address` must be a function with the signature `F`.
pub(crate) unsafe fn function<F: Copy>(address: *mut c_void) -> F {
    const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
    // SAFETY: a function pointer of the size of `F`, which the caller says is its signature.
    unsafe { std::mem::transmute_copy::<*mut c_void, F>(&address) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_glib() {
        // 64-bit Linux, as GLib lays them out.
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(size_of::<ObjectClass>(), 136);
            assert_eq!(std::mem::offset_of!(ObjectClass, dispose), 40);
            assert_eq!(std::mem::offset_of!(ObjectClass, finalize), 48);
            assert_eq!(size_of::<TypeInfo>(), 72);
            assert_eq!(std::mem::offset_of!(TypeInfo, instance_size), 48);
            assert_eq!(size_of::<TypeQuery>(), 24);
        }
    }
}
