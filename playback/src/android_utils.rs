use std::{
    ffi::c_void,
    ops::Deref,
    panic::{self, catch_unwind},
    string::String,
    sync::{Once, OnceLock},
};

use jni::sys;
use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JObject},
    sys::{JNI_FALSE, JNI_TRUE, JNI_VERSION_1_6, jboolean, jint},
};
use ndk_context::initialize_android_context;
use tracing::{error, info};
use tracing_logcat::{LogcatMakeWriter, LogcatTag};
use tracing_subscriber::fmt::format::Format;

/// Invalid JNI version constant, signifying JNI_OnLoad failure.
const INVALID_JNI_VERSION: jint = 0;

// Ensure 1-time initialization of JVM
static INIT: Once = Once::new();
static APPLICATION_CONTEXT: OnceLock<GlobalRef> = OnceLock::new();
static mut JVM: Option<*mut c_void> = None;

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(vm: *mut JavaVM, _: *mut c_void) -> jint {
    catch_unwind(|| {
        if let Ok(writer) =
            LogcatMakeWriter::new(LogcatTag::Fixed(env!("CARGO_PKG_NAME").to_owned()))
        {
            // init_logging() (hub::utils) installs a subscriber too; whichever
            // runs second must not panic, or the Rust main thread dies silently.
            let _ = tracing_subscriber::fmt()
                .event_format(Format::default().with_level(false).without_time())
                .with_writer(writer)
                .with_ansi(false)
                .try_init();
        }

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

        // Safely init JVM and ClassLoader
        INIT.call_once(|| unsafe {
            // Convert *mut JavaVM to *mut c_void and store it
            JVM = Some(vm as *mut c_void);

            // Initialize ClassLoader for proper class finding from non-main threads
            let java_vm = JavaVM::from_raw(vm as *mut sys::JavaVM).unwrap();
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
    env: JNIEnv,
    _activity: JObject,
    context: JObject,
) -> jboolean {
    if APPLICATION_CONTEXT.get().is_some() {
        return JNI_TRUE;
    }
    let Ok(vm) = env.get_java_vm() else {
        return JNI_FALSE;
    };
    let Ok(context) = env.new_global_ref(context) else {
        return JNI_FALSE;
    };
    if context.as_obj().is_null() {
        return JNI_FALSE;
    }
    APPLICATION_CONTEXT.get_or_init(|| {
        unsafe {
            initialize_android_context(
                vm.get_java_vm_pointer().cast(),
                context.as_obj().as_raw().cast(),
            );
        }
        context
    });
    JNI_TRUE
}

pub fn get_jvm() -> Option<*mut c_void> {
    unsafe { JVM }
}

/// The application context that `MainActivity` handed over, once it has.
pub(crate) fn application_context() -> Option<&'static GlobalRef> {
    APPLICATION_CONTEXT.get()
}
