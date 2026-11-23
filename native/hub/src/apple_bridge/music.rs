//! Calls into the Swift MusicKit bridge. Apple Music can only be read and played through
//! Apple's own framework, so the hub asks Swift and gets JSON back.

use std::ffi::{CStr, CString, c_char, c_void};

use anyhow::{Context, Result, anyhow};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::oneshot;

unsafe extern "C" {
    fn rune_apple_music_request(
        request: *const c_char,
        context: *mut c_void,
        reply: unsafe extern "C" fn(*mut c_void, *const c_char),
    );
}

/// Why an Apple Music request was not answered with data.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppleMusicError {
    /// The user has not allowed this app to use Apple Music.
    #[error("Apple Music access is not allowed: {0}")]
    NotAuthorized(String),
    /// The item no longer exists in the library.
    #[error("Apple Music item not found: {0}")]
    NotFound(String),
    /// This system is too old for the MusicKit calls the bridge uses.
    #[error("Apple Music is not available on this system: {0}")]
    Unsupported(String),
    /// MusicKit or the service reported a failure.
    #[error("Apple Music request failed: {0}")]
    Service(String),
}

/// Completes the request's channel. Swift calls it exactly once, on any thread.
unsafe extern "C" fn deliver(context: *mut c_void, reply: *const c_char) {
    // SAFETY: `context` is the box leaked by `request_json` for this call, and Swift hands it
    // back once. `reply` is a NUL-terminated string that lives until this function returns.
    let (sender, text) = unsafe {
        (
            Box::from_raw(context.cast::<oneshot::Sender<String>>()),
            CStr::from_ptr(reply).to_string_lossy().into_owned(),
        )
    };
    // The caller may have stopped waiting; its answer is then dropped.
    let _ = sender.send(text);
}

async fn request_json(request: &Value) -> Result<Value> {
    let text = CString::new(request.to_string()).context("Apple Music request contains NUL")?;
    let (sender, receiver) = oneshot::channel::<String>();
    let context = Box::into_raw(Box::new(sender)).cast::<c_void>();
    // SAFETY: `text` outlives the call, which copies it before returning. Swift calls `deliver`
    // once with `context`, which frees the box.
    unsafe { rune_apple_music_request(text.as_ptr(), context, deliver) };
    let reply = receiver
        .await
        .context("the Apple Music bridge dropped the request")?;
    let mut reply: Value = serde_json::from_str(&reply).context("Apple Music reply is not JSON")?;
    if let Some(error) = reply.get("error") {
        let detail = error["detail"].as_str().unwrap_or_default().to_owned();
        return Err(match error["kind"].as_str() {
            Some("notAuthorized") => AppleMusicError::NotAuthorized(detail),
            Some("notFound") => AppleMusicError::NotFound(detail),
            Some("unsupported") => AppleMusicError::Unsupported(detail),
            _ => AppleMusicError::Service(detail),
        }
        .into());
    }
    reply
        .get_mut("ok")
        .map(Value::take)
        .ok_or_else(|| anyhow!("Apple Music reply has neither ok nor error"))
}

/// Sends `request` to MusicKit and decodes its answer.
pub async fn request<T: DeserializeOwned>(request: Value) -> Result<T> {
    serde_json::from_value(request_json(&request).await?)
        .context("unexpected Apple Music reply shape")
}
