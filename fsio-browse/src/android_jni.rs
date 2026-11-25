//! Calls into the Android framework, shared by places and thumbnails.

use anyhow::Result;
use jni::JNIEnv;
use jni::objects::JObject;
use ndk_saf::{clear_pending_exception, find_class, get_env};

/// Run `call` with this thread attached to the Java VM. A Java exception it
/// leaves pending is cleared: ART would abort the process with it when the
/// thread detaches.
pub(crate) fn with_env<T>(call: impl FnOnce(&mut JNIEnv) -> jni::errors::Result<T>) -> Result<T> {
    let mut guard = get_env()?;
    let env = &mut *guard;
    let result = call(env);
    if result.is_err() {
        clear_pending_exception(env);
    }
    Ok(result?)
}

/// The application's content resolver.
pub(crate) fn content_resolver<'local>(
    env: &mut JNIEnv<'local>,
) -> jni::errors::Result<JObject<'local>> {
    let thread = find_class("android/app/ActivityThread")?;
    let application = env
        .call_static_method(
            &thread,
            "currentApplication",
            "()Landroid/app/Application;",
            &[],
        )?
        .l()?;
    env.call_method(
        &application,
        "getContentResolver",
        "()Landroid/content/ContentResolver;",
        &[],
    )?
    .l()
}
