#![cfg(any(target_os = "linux", target_os = "macos", windows))]

use anyhow::{anyhow, ensure, Result};
use fsio::FileIoError;
use jni::objects::{GlobalRef, JClass, JValue};
use jni::sys::{jboolean, JNI_FALSE, JNI_TRUE};
use jni::{errors::Error::JavaException, JNIEnv, JavaVM};

#[path = "../../src/android_jni.rs"]
mod android_jni;
use android_jni::with_document_env;

fn check_worker(vm: &JavaVM, launcher: &GlobalRef) -> Result<()> {
    for already_attached in [false, true] {
        let _attachment = already_attached
            .then(|| vm.attach_current_thread())
            .transpose()?;
        for class in [
            "java/lang/IllegalStateException",
            "java/io/FileNotFoundException",
            "java/lang/SecurityException",
        ] {
            for reports_error in [false, true] {
                let mut java_threw = false;
                let result = with_document_env(vm.attach_current_thread()?, |_| {
                    let mut nested = vm.attach_current_thread()?;
                    let exception = nested.new_object(class, "()V", &[])?;
                    let call = nested.call_static_method(
                        launcher,
                        "fail",
                        "(Ljava/lang/Exception;)V",
                        &[(&exception).into()],
                    );
                    java_threw = matches!(call, Err(JavaException));
                    if reports_error {
                        call?;
                    }
                    Ok(())
                });
                ensure!(java_threw, "the fixture did not throw {class}");
                ensure!(matches!(result, Err(FileIoError::Saf(_))), "{class} did not become a provider error (reports_error={reports_error}, already_attached={already_attached})");
                let mut env = vm.attach_current_thread()?;
                ensure!(!env.exception_check()?, "{class} remained pending");
                let value = env.call_static_method(
                    "java/lang/Integer",
                    "signum",
                    "(I)I",
                    &[JValue::Int(17)],
                )?;
                ensure!(value.i()? == 1, "Java call after {class} failed");
            }
        }
    }
    Ok(())
}

fn verify(env: JNIEnv, class: JClass) -> Result<()> {
    let vm = env.get_java_vm()?;
    let launcher = env.new_global_ref(class)?;
    std::thread::scope(|scope| scope.spawn(|| check_worker(&vm, &launcher)).join())
        .map_err(|_| anyhow!("JNI worker panicked"))?
}

#[no_mangle]
pub extern "system" fn Java_DocumentProviderJni_verify(env: JNIEnv, class: JClass) -> jboolean {
    match verify(env, class) {
        Ok(()) => JNI_TRUE,
        Err(error) => {
            eprintln!("{error:#}");
            JNI_FALSE
        }
    }
}
