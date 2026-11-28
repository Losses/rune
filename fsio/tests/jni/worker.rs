#![cfg(any(target_os = "linux", target_os = "macos", windows))]

use anyhow::{anyhow, ensure, Result};
use fsio::FileIoError;
use jni::objects::{GlobalRef, JClass, JValue};
use jni::sys::{jboolean, JNI_FALSE, JNI_TRUE};
use jni::{errors::Error::JavaException, JNIEnv, JavaVM};

#[path = "../../src/android_jni.rs"]
mod android_jni;
use android_jni::{with_document_cursor, with_document_env, Operation};

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
                for operation in [
                    Operation::Item("open document"),
                    Operation::List,
                    Operation::RootQuery,
                ] {
                    let mut java_threw = false;
                    let result = with_document_env(vm.attach_current_thread()?, operation, |_| {
                        let mut nested = vm.attach_current_thread()?;
                        let secret = nested.new_string("private-message-with-content-uri")?;
                        let exception = nested.new_object(
                            class,
                            "(Ljava/lang/String;)V",
                            &[(&secret).into()],
                        )?;
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
                    let error = result.unwrap_err();
                    let expected = match operation {
                        Operation::Item(_) => matches!(error, FileIoError::Saf(_)),
                        Operation::RootQuery if !class.ends_with("IllegalStateException") => {
                            matches!(error, FileIoError::DocumentRootUnavailable(_))
                        }
                        _ => matches!(error, FileIoError::DocumentListingIncomplete(_)),
                    };
                    ensure!(expected, "wrong scope for {class}: {error}");
                    let detail = error.to_string();
                    ensure!(
                        detail.contains(class.rsplit('/').next().unwrap()),
                        "lost the exception class: {detail}"
                    );
                    ensure!(
                        !detail.contains("private-message"),
                        "provider message escaped into error"
                    );
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
    }
    Ok(())
}

fn check_listings(vm: &JavaVM, launcher: &GlobalRef) -> Result<()> {
    let mut env = vm.attach_current_thread()?;
    for state in ["null", "empty", "loading", "error", "info", "read failure"] {
        let state_string = env.new_string(state)?;
        let cursor = env
            .call_static_method(
                launcher,
                "listing",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[(&state_string).into()],
            )?
            .l()?;
        let mut read = false;
        let result = with_document_cursor(
            &mut env,
            &cursor,
            |env, cursor| Ok(env.new_local_ref(cursor)?),
            |env, cursor| {
                read = true;
                Ok(env.call_method(cursor, "hasNext", "()Z", &[])?.z()?)
            },
        );
        if state == "empty" {
            ensure!(
                matches!(result, Ok(false)),
                "empty listing rejected: {result:?}"
            );
        } else {
            ensure!(
                matches!(result, Err(FileIoError::DocumentListingIncomplete(_))),
                "incomplete {state} listing accepted: {result:?}"
            );
        }
        ensure!(
            read == matches!(state, "empty" | "read failure"),
            "read unexpected rows from {state}"
        );
        ensure!(!env.exception_check()?, "pending exception after {state}");
        if !cursor.is_null() {
            ensure!(
                env.call_method(cursor, "isClosed", "()Z", &[])?.z()?,
                "unclosed {state} listing"
            );
        }
    }
    Ok(())
}

fn verify(env: JNIEnv, class: JClass) -> Result<()> {
    let vm = env.get_java_vm()?;
    let launcher = env.new_global_ref(class)?;
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                check_worker(&vm, &launcher)?;
                check_listings(&vm, &launcher)
            })
            .join()
    })
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
