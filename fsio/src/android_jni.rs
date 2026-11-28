use jni::objects::{JObject, JString, JThrowable};
use jni::{errors::Error as JniError, AttachGuard, JNIEnv};

use super::FileIoError;

#[derive(Clone, Copy)]
pub(super) enum Operation {
    Item(&'static str),
    List,
    RootQuery,
}

#[derive(Clone, Copy)]
enum Cause {
    AccessDenied,
    Missing,
    Runtime(&'static str),
    Other(&'static str),
}

impl Cause {
    fn name(self) -> &'static str {
        match self {
            Self::AccessDenied => "SecurityException",
            Self::Missing => "FileNotFoundException",
            Self::Runtime(name) | Self::Other(name) => name,
        }
    }
}

impl Operation {
    fn error(self, cause: Cause) -> FileIoError {
        let description = cause.name();
        if matches!(cause, Cause::Runtime(_)) {
            return FileIoError::DocumentRuntime(description.into());
        }
        match self {
            Self::Item(name) => FileIoError::Saf(format!("{name}: {description}")),
            Self::List => {
                FileIoError::DocumentListingIncomplete(format!("list documents: {description}"))
            }
            Self::RootQuery => {
                let detail = format!("query root document: {description}");
                match cause {
                    Cause::AccessDenied | Cause::Missing => {
                        FileIoError::DocumentRootUnavailable(detail)
                    }
                    _ => FileIoError::DocumentListingIncomplete(detail),
                }
            }
        }
    }
}

fn runtime_error(_: JniError) -> FileIoError {
    FileIoError::DocumentRuntime("JNI call failed".into())
}

/// The provider controls its exceptions, including getMessage and toString. Only
/// report known platform class names; a message can contain the full granted URI.
fn exception_kind(env: &mut JNIEnv, exception: &JThrowable) -> jni::errors::Result<Cause> {
    let class = env.get_object_class(exception)?;
    let name = JString::from(
        env.call_method(class, "getName", "()Ljava/lang/String;", &[])?
            .l()?,
    );
    let name = env.get_string(&name)?;
    Ok(match name.to_bytes() {
        b"java.lang.SecurityException" => Cause::AccessDenied,
        b"java.io.FileNotFoundException" => Cause::Missing,
        b"java.lang.IllegalStateException" => Cause::Other("IllegalStateException"),
        b"java.io.IOException" => Cause::Other("IOException"),
        b"android.app.AuthenticationRequiredException" => {
            Cause::Other("AuthenticationRequiredException")
        }
        b"android.app.RecoverableSecurityException" => Cause::Other("RecoverableSecurityException"),
        b"java.lang.OutOfMemoryError" => Cause::Runtime("OutOfMemoryError"),
        b"java.lang.NoClassDefFoundError" => Cause::Runtime("NoClassDefFoundError"),
        b"java.lang.NoSuchMethodError" => Cause::Runtime("NoSuchMethodError"),
        _ => Cause::Other("Java exception"),
    })
}

/// Convert pending exceptions before making another Java call (including
/// Cursor.close) or dropping the final thread attachment. Some SAF helpers
/// return Ok with a pending exception, so check both success and failure.
pub(super) fn checked_result<T>(
    env: &mut JNIEnv,
    operation: Operation,
    result: anyhow::Result<T>,
) -> Result<T, FileIoError> {
    if env.exception_check().map_err(runtime_error)? {
        let exception = env.exception_occurred();
        env.exception_clear().map_err(runtime_error)?;
        let exception = env.auto_local(exception.map_err(runtime_error)?);
        let kind = env.with_local_frame(4, |env| exception_kind(env, &exception));
        if kind.is_err() {
            // Class inspection can itself throw, for example under memory pressure.
            env.exception_clear().map_err(runtime_error)?;
        }
        return Err(
            operation.error(kind.unwrap_or(Cause::Runtime("cannot inspect Java exception")))
        );
    }
    result.map_err(|error| match error.downcast::<FileIoError>() {
        Ok(error) => error,
        Err(error) => match error.downcast_ref::<JniError>() {
            Some(JniError::NullPtr(_) | JniError::JavaException) | None => {
                operation.error(Cause::Other("document provider call failed"))
            }
            Some(_) => FileIoError::DocumentRuntime("JNI call failed".into()),
        },
    })
}

pub(super) fn with_document_env<T>(
    mut env: AttachGuard<'_>,
    operation: Operation,
    call: impl FnOnce(&mut JNIEnv) -> anyhow::Result<T>,
) -> Result<T, FileIoError> {
    let result = env.with_local_frame(32, call);
    checked_result(&mut env, operation, result)
}

/// Only complete provider listings can establish that an entry is absent.
/// Close on all paths, including when reading the cursor raised an exception.
pub(super) fn with_document_cursor<'local, T>(
    env: &mut JNIEnv<'local>,
    cursor: &JObject,
    extras: impl FnOnce(&mut JNIEnv<'local>, &JObject) -> anyhow::Result<JObject<'local>>,
    read: impl FnOnce(&mut JNIEnv<'local>, &JObject) -> anyhow::Result<T>,
) -> Result<T, FileIoError> {
    if cursor.is_null() {
        return Err(FileIoError::DocumentListingIncomplete(
            "provider returned no cursor".into(),
        ));
    }
    let rows = (|| {
        let extras = extras(env, cursor)?;
        if !extras.is_null() {
            let loading = env.new_string("loading")?;
            let loading = env
                .call_method(
                    &extras,
                    "getBoolean",
                    "(Ljava/lang/String;)Z",
                    &[(&loading).into()],
                )?
                .z()?;
            // DocumentsContract.EXTRA_INFO may also indicate that some entries
            // are unavailable. A partial listing cannot establish a deletion.
            let has_notice = |env: &mut JNIEnv, key: &str| -> anyhow::Result<bool> {
                let key = env.new_string(key)?;
                let notice = env
                    .call_method(
                        &extras,
                        "getString",
                        "(Ljava/lang/String;)Ljava/lang/String;",
                        &[(&key).into()],
                    )?
                    .l()?;
                Ok(!notice.is_null())
            };
            if loading || has_notice(env, "error")? || has_notice(env, "info")? {
                return Err(FileIoError::DocumentListingIncomplete(
                    "provider reported a partial result".into(),
                )
                .into());
            }
        }

        read(env, cursor)
    })();
    let rows = checked_result(env, Operation::List, rows);
    let close = env
        .call_method(cursor, "close", "()V", &[])
        .and_then(|v| v.v());
    let closed = checked_result(env, Operation::List, close.map_err(Into::into));
    let rows = rows?;
    closed?;
    Ok(rows)
}
