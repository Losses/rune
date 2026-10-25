//! Running code on the platform thread, where the engine wants its texture registry used.
//!
//! - Apple: the main thread, through the main dispatch queue.
//! - Windows: none; the engine takes register, unregister and mark calls from any thread.

#[cfg(target_vendor = "apple")]
pub(crate) use apple::*;

#[cfg(target_vendor = "apple")]
mod apple {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use dispatch2::DispatchQueue;
    use futures_channel::oneshot;
    use log::error;
    use objc2::MainThreadMarker;

    use crate::{Result, TextureError};

    /// Run `f` on the main thread: right away if this is it, else later from the main queue.
    pub(crate) fn run(f: impl FnOnce(MainThreadMarker) + Send + 'static) {
        let guarded = move |mtm| {
            // A panic must not unwind into the dispatch queue (or the caller on the main thread).
            if catch_unwind(AssertUnwindSafe(|| f(mtm))).is_err() {
                error!("A call on the platform thread panicked");
            }
        };
        match MainThreadMarker::new() {
            Some(mtm) => guarded(mtm),
            None => DispatchQueue::main().exec_async(move || {
                // SAFETY: the main queue runs its work on the main thread.
                guarded(unsafe { MainThreadMarker::new_unchecked() })
            }),
        }
    }

    /// Run `f` on the main thread (see [`run`]) and wait for its result.
    pub(crate) async fn call<R: Send + 'static>(
        f: impl FnOnce(MainThreadMarker) -> R + Send + 'static,
    ) -> Result<R> {
        let (sender, receiver) = oneshot::channel();
        run(move |mtm| {
            let _ = sender.send(f(mtm));
        });
        receiver.await.map_err(|_| TextureError::PlatformThread)
    }

    /// A value only used on the main thread, which may be moved anywhere: it is dropped on the
    /// main thread as well.
    pub(crate) struct MainThreadOnly<T: 'static>(Option<T>);

    // SAFETY: the value is only reached with a `MainThreadMarker`, and dropped on the main thread.
    unsafe impl<T: 'static> Send for MainThreadOnly<T> {}
    // SAFETY: as above.
    unsafe impl<T: 'static> Sync for MainThreadOnly<T> {}

    impl<T: 'static> MainThreadOnly<T> {
        pub(crate) fn new(value: T) -> Self {
            Self(Some(value))
        }

        pub(crate) fn get(&self, _mtm: MainThreadMarker) -> &T {
            self.0.as_ref().expect("present until dropped")
        }
    }

    impl<T: 'static> Drop for MainThreadOnly<T> {
        fn drop(&mut self) {
            let Some(value) = self.0.take() else { return };
            if MainThreadMarker::new().is_some() {
                drop(value);
                return;
            }
            struct AssertSend<T>(T);
            // SAFETY: the value only moves to the main thread, where it is dropped.
            unsafe impl<T> Send for AssertSend<T> {}
            let value = AssertSend(value);
            // Not `exec_sync`: the main thread may be waiting for this one.
            DispatchQueue::main().exec_async(move || {
                let value = value;
                drop(value.0);
            });
        }
    }
}
