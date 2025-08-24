use std::{
    ffi::c_void,
    ops::Deref,
    panic::{self, catch_unwind},
    string::String,
    sync::Once,
};

use jni::{
    JavaVM,
    sys::{JNI_VERSION_1_6, jint},
};
use ndk_context::{initialize_android_context, release_android_context};
use tracing::{error, info};
use tracing_logcat::{LogcatMakeWriter, LogcatTag};
use tracing_subscriber::fmt::format::Format;

/// Invalid JNI version constant, signifying JNI_OnLoad failure.
const INVALID_JNI_VERSION: jint = 0;

// Ensure 1-time initialization of JVM
static INIT: Once = Once::new();
static mut JVM: Option<*mut c_void> = None;

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(vm: *mut JavaVM, _: *mut c_void) -> jint {
    let tag = LogcatTag::Fixed(env!("CARGO_PKG_NAME").to_owned());
    let writer = LogcatMakeWriter::new(tag).expect("Failed to initialize logcat writer");

    tracing_subscriber::fmt()
        .event_format(Format::default().with_level(false).without_time())
        .with_writer(writer)
        .with_ansi(false)
        .init();
    panic::set_hook(Box::new(|panic_info| {
        let (filename, line) = panic_info
            .location()
            .map(|loc| (loc.file(), loc.line()))
            .unwrap_or(("<unknown>", 0));

        let cause = panic_info
            .payload()
            .downcast_ref::<String>()
            .map(String::deref);

        let cause = cause.unwrap_or_else(|| {
            panic_info
                .payload()
                .downcast_ref::<&str>()
                .copied()
                .unwrap_or("<cause unknown>")
        });

        error!("A panic occurred at {}:{}: {}", filename, line, cause);
    }));

    catch_unwind(|| {
        // Safely init JVM and ClassLoader
        INIT.call_once(|| unsafe {
            // Convert *mut JavaVM to *mut c_void and store it
            JVM = Some(vm as *mut c_void);

            // Initialize ClassLoader for proper class finding from non-main threads
            let java_vm = JavaVM::from_raw(vm as *mut jni::sys::JavaVM).unwrap();
            if let Ok(mut env) = java_vm.get_env() {
                if let Err(e) = ndk_saf::initialize_class_loader(vm, &mut env) {
                    error!("JNI_OnLoad: Failed to setup ClassLoader: {:?}", e);
                } else {
                    info!("JNI_OnLoad: JVM and ClassLoader initialized successfully");
                }
            } else {
                error!("JNI_OnLoad: Failed to get JNI environment");
            }
        });
        JNI_VERSION_1_6
    })
    .unwrap_or(INVALID_JNI_VERSION)
}

#[unsafe(no_mangle)]
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

#[unsafe(no_mangle)]
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
