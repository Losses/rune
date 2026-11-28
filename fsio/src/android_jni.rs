use std::fmt::Display;

use anyhow::anyhow;
use jni::{AttachGuard, JNIEnv};

use super::FileIoError;

pub(super) fn saf_error(error: impl Display) -> FileIoError {
    FileIoError::Saf(error.to_string())
}

/// Keep the thread attached across nested provider calls. A pending exception
/// must be cleared before the outermost guard detaches the native thread.
pub(super) fn with_document_env<T>(
    mut env: AttachGuard<'_>,
    call: impl FnOnce(&mut JNIEnv) -> anyhow::Result<T>,
) -> Result<T, FileIoError> {
    let result = call(&mut env);
    if env.exception_check().map_err(saf_error)? {
        let _ = env.exception_describe();
        env.exception_clear().map_err(saf_error)?;
        let error = result
            .err()
            .unwrap_or_else(|| anyhow!("the Android document provider threw an exception"));
        return Err(saf_error(error));
    }
    result.map_err(saf_error)
}
