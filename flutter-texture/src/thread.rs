//! Running code on the platform thread, where the engine wants its texture registry used.
//!
//! - Apple: the main thread, through the main dispatch queue.
//! - Windows: none; the engine takes register, unregister and mark calls from any thread.
//! - Linux: the thread that owns GLib's default main context (GTK's main loop), through an idle
//!   source on that context.

#[cfg(target_vendor = "apple")]
pub(crate) use apple::*;
#[cfg(target_os = "linux")]
pub(crate) use glib::*;

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

#[cfg(target_os = "linux")]
mod glib {
    use std::ffi::c_void;
    use std::marker::PhantomData;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use futures_channel::oneshot;
    use log::error;

    use crate::linux::api::{self, Gboolean, PRIORITY_DEFAULT, SOURCE_REMOVE};
    use crate::{Result, TextureError};

    /// Proof that the code runs on the thread that owns GLib's default main context, where GTK
    /// and the engine's registrar are used.
    #[derive(Clone, Copy)]
    pub(crate) struct MainContext(PhantomData<*const ()>);

    /// A call waiting for the main context.
    type Call = Box<dyn FnOnce(MainContext) + Send>;

    /// Run `f` on the main context: right away if this thread owns it, else later from an idle
    /// source. If GLib is not loaded `f` is dropped without running.
    ///
    /// Not `g_main_context_invoke_full`: from a thread without a context of its own, it runs the
    /// call right there whenever it can acquire the default context, as before GTK's main loop
    /// runs or after it quit.
    pub(crate) fn run(f: impl FnOnce(MainContext) + Send + 'static) {
        let api = match api::api() {
            Ok(api) => api,
            Err(e) => {
                error!("A call for the platform thread is dropped: {e}");
                return;
            }
        };
        // SAFETY: plain calls; the default context exists for the life of the process.
        let owner = unsafe { (api.main_context_is_owner)((api.main_context_default)()) } != 0;
        if owner {
            guarded(Box::new(f));
            return;
        }
        let call: Box<Option<Call>> = Box::new(Some(Box::new(f)));
        // SAFETY: the source takes the box, which `invoke` runs and `free` frees once.
        unsafe {
            (api.idle_add_full)(
                PRIORITY_DEFAULT,
                invoke,
                Box::into_raw(call).cast(),
                Some(free),
            );
        }
    }

    /// Run `f` on the main context (see [`run`]) and wait for its result.
    pub(crate) async fn call<R: Send + 'static>(
        f: impl FnOnce(MainContext) -> R + Send + 'static,
    ) -> Result<R> {
        let (sender, receiver) = oneshot::channel();
        run(move |context| {
            let _ = sender.send(f(context));
        });
        receiver.await.map_err(|_| TextureError::PlatformThread)
    }

    /// Run `call` on the main context, which this thread owns.
    fn guarded(call: Call) {
        // A panic must not unwind into GLib (or the caller on the main thread).
        if catch_unwind(AssertUnwindSafe(|| call(MainContext(PhantomData)))).is_err() {
            error!("A call on the platform thread panicked");
        }
    }

    /// The idle source's function.
    ///
    /// # Safety
    ///
    /// `data` is the `Box<Option<Call>>` that `run` leaked, not yet freed.
    unsafe extern "C" fn invoke(data: *mut c_void) -> Gboolean {
        // SAFETY: as the caller guarantees; the source runs on the thread that owns the context.
        let call = unsafe { &mut *data.cast::<Option<Call>>() };
        if let Some(call) = call.take() {
            guarded(call);
        }
        SOURCE_REMOVE
    }

    /// The idle source's destroy notify: frees the call, dropping it if it never ran.
    ///
    /// # Safety
    ///
    /// `data` is the `Box<Option<Call>>` that `run` leaked, freed once.
    unsafe extern "C" fn free(data: *mut c_void) {
        // SAFETY: as the caller guarantees.
        let call = unsafe { Box::from_raw(data.cast::<Option<Call>>()) };
        if catch_unwind(AssertUnwindSafe(|| drop(call))).is_err() {
            error!("Dropping a call for the platform thread panicked");
        }
    }
}
