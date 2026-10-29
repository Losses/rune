//! The parts of libandroid used here: `<android/looper.h>` and `<android/native_window_jni.h>`.

use std::ffi::{c_int, c_void};

/// `ALooper`, opaque.
#[repr(C)]
pub(crate) struct ALooper {
    _private: [u8; 0],
}

/// `ANativeWindow`, opaque and reference counted.
#[repr(C)]
pub(crate) struct ANativeWindow {
    _private: [u8; 0],
}

/// `ALooper_callbackFunc`: returns 1 to keep receiving events, 0 to be unregistered.
pub(crate) type ALooperCallback =
    unsafe extern "C" fn(fd: c_int, events: c_int, data: *mut c_void) -> c_int;

/// The `ident` of a file descriptor that has a callback.
pub(crate) const ALOOPER_POLL_CALLBACK: c_int = -2;
/// The file descriptor can be read.
pub(crate) const ALOOPER_EVENT_INPUT: c_int = 1;

#[link(name = "android")]
unsafe extern "C" {
    /// The looper of the calling thread, or null; not acquired.
    pub(crate) fn ALooper_forThread() -> *mut ALooper;
    pub(crate) fn ALooper_acquire(looper: *mut ALooper);
    pub(crate) fn ALooper_release(looper: *mut ALooper);
    /// Returns 1 if the file descriptor was added, -1 on error. With a callback it is called on
    /// the looper's thread while the looper polls.
    pub(crate) fn ALooper_addFd(
        looper: *mut ALooper,
        fd: c_int,
        ident: c_int,
        events: c_int,
        callback: Option<ALooperCallback>,
        data: *mut c_void,
    ) -> c_int;

    /// The native window of an `android.view.Surface`, acquired (released with
    /// `ANativeWindow_release`), or null.
    pub(crate) fn ANativeWindow_fromSurface(
        env: *mut jni::sys::JNIEnv,
        surface: jni::sys::jobject,
    ) -> *mut ANativeWindow;
    pub(crate) fn ANativeWindow_release(window: *mut ANativeWindow);
}
