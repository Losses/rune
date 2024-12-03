use std::ffi::c_void;
use std::panic::catch_unwind;
use std::sync::Once;

use log::info;

use jni::{
    sys::{jint, JNI_VERSION_1_6},
    JavaVM,
};
use ndk_context::{initialize_android_context, release_android_context};

/// Invalid JNI version constant, signifying JNI_OnLoad failure.
const INVALID_JNI_VERSION: jint = 0;

// Ensure 1-time initialization of JVM
static INIT: Once = Once::new();
static mut JVM: Option<*mut c_void> = None;

#[allow(non_snake_case)]
#[no_mangle]
pub extern "system" fn JNI_OnLoad(vm: *mut JavaVM, _: *mut c_void) -> jint {
    catch_unwind(|| {
        // Safely init JVM
        INIT.call_once(|| {
            unsafe {
                // Convert *mut JavaVM to *mut c_void and store it
                JVM = Some(vm as *mut c_void);
            }
            info!("JNI_OnLoad called and JVM initialized");
        });
        JNI_VERSION_1_6
    })
    .unwrap_or(INVALID_JNI_VERSION)
}

#[no_mangle]
pub extern "system" fn Java_ci_not_rune_MainActivity_initializeContext(
    _env: *mut jni::JNIEnv,
    _class: jni::objects::JClass,
    context: jni::objects::JObject,
) {
    unsafe {
        // Convert JObject Context to c_void pointer and initialize Context
        if let Some(jvm) = JVM {
            // Converting context to raw pointer
            let context_ptr = context.into_raw() as *mut c_void;

            initialize_android_context(jvm, context_ptr);
        }
    }
    info!("JNI Context initialized");
}

#[no_mangle]
pub extern "system" fn Java_ci_not_rune_MainActivity_releaseContext(
    _env: *mut jni::JNIEnv,
    _class: jni::objects::JClass,
) {
    unsafe {
        release_android_context();
    }
    info!("JNI Context released");
}

pub fn get_jvm() -> Option<*mut c_void> {
    unsafe { JVM }
}
