//! Running code on the platform thread, where the engine wants its texture registry used.
//!
//! - Apple: the main thread, through the main dispatch queue.
//! - Windows: none; the engine takes register, unregister and mark calls from any thread.
//! - Linux: the thread that owns GLib's default main context (GTK's main loop), through an idle
//!   source on that context.
//! - Android: the main thread, through its looper: an eventfd the looper watches wakes it, and
//!   the callback runs the queued calls.

#[cfg(target_os = "android")]
pub(crate) use android::*;
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

#[cfg(target_os = "android")]
mod android {
    use std::collections::VecDeque;
    use std::ffi::{c_int, c_void};
    use std::marker::PhantomData;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::{Mutex, OnceLock};
    use std::thread::ThreadId;

    use futures_channel::oneshot;
    use log::error;

    use crate::android::ndk;
    use crate::{Result, TextureError, lock};

    /// Proof that the code runs on the main thread, where the engine wants its registry used.
    #[derive(Clone, Copy)]
    pub(crate) struct MainThread(PhantomData<*const ()>);

    /// A call waiting for the main thread.
    type Call = Box<dyn FnOnce(MainThread) + Send>;

    /// The main thread and how to wake its looper.
    struct MainLooper {
        thread: ThreadId,
        /// An eventfd the looper watches; a write wakes it.
        wake: OwnedFd,
        calls: Mutex<VecDeque<Call>>,
    }

    /// Set once, by the first [`hook_main_looper`]; the main looper lives as long as the process.
    static MAIN: OnceLock<MainLooper> = OnceLock::new();

    /// Make the calling thread, which must be the main thread, the one [`run`] runs calls on. The
    /// first call watches an eventfd with the thread's looper; later ones (a new engine, after the
    /// activity was recreated) only check that they run on the same thread.
    pub(crate) fn hook_main_looper() -> Result<()> {
        let thread = std::thread::current().id();
        if let Some(main) = MAIN.get() {
            return if main.thread == thread {
                Ok(())
            } else {
                error!("A Flutter texture registry was attached off the main thread");
                Err(TextureError::PlatformThread)
            };
        }
        // SAFETY: a plain call; the looper is acquired below, before it is used.
        let looper = unsafe { ndk::ALooper_forThread() };
        if looper.is_null() {
            error!("The thread that attached a Flutter texture registry has no looper");
            return Err(TextureError::PlatformThread);
        }
        // SAFETY: a plain call; the descriptor is owned right away.
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            error!(
                "No eventfd for the main looper: {}",
                std::io::Error::last_os_error()
            );
            return Err(TextureError::PlatformThread);
        }
        // SAFETY: `fd` is a new descriptor nothing else owns.
        let wake = unsafe { OwnedFd::from_raw_fd(fd) };
        // SAFETY: `looper` is this thread's. It is acquired for good, so it outlives the watch;
        // the descriptor goes to `MAIN` below and lives there for the rest of the process.
        // `drain` takes no data.
        let added = unsafe {
            ndk::ALooper_acquire(looper);
            ndk::ALooper_addFd(
                looper,
                wake.as_raw_fd(),
                ndk::ALOOPER_POLL_CALLBACK,
                ndk::ALOOPER_EVENT_INPUT,
                Some(drain),
                std::ptr::null_mut(),
            )
        };
        if added != 1 {
            // SAFETY: undoes the acquire above; `wake` closes when it drops here.
            unsafe { ndk::ALooper_release(looper) };
            error!("The main looper does not watch the eventfd");
            return Err(TextureError::PlatformThread);
        }
        // Set only once the watch works: a later attach must not find a main looper that never
        // wakes, or every call for the main thread would wait forever. Only the main thread gets
        // here, so nothing else sets it in between.
        let _ = MAIN.set(MainLooper {
            thread,
            wake,
            calls: Mutex::new(VecDeque::new()),
        });
        Ok(())
    }

    /// Run `f` on the main thread: right away if this is it, else later from its looper. Before
    /// a registry was attached, `f` is dropped without running.
    pub(crate) fn run(f: impl FnOnce(MainThread) + Send + 'static) {
        let Some(main) = MAIN.get() else {
            error!("A call for the platform thread is dropped: no registry was attached");
            return;
        };
        if std::thread::current().id() == main.thread {
            guarded(Box::new(f));
            return;
        }
        lock(&main.calls).push_back(Box::new(f));
        let one: u64 = 1;
        // SAFETY: writes the 8 bytes of `one` to the eventfd. It only fails once the counter is
        // about to overflow, and then the looper has a wake-up pending anyway.
        unsafe { libc::write(main.wake.as_raw_fd(), (&raw const one).cast(), 8) };
    }

    /// Run `f` on the main thread (see [`run`]) and wait for its result.
    pub(crate) async fn call<R: Send + 'static>(
        f: impl FnOnce(MainThread) -> R + Send + 'static,
    ) -> Result<R> {
        let (sender, receiver) = oneshot::channel();
        run(move |main| {
            let _ = sender.send(f(main));
        });
        receiver.await.map_err(|_| TextureError::PlatformThread)
    }

    /// Run `call` on the main thread, which this is.
    fn guarded(call: Call) {
        // A panic must not unwind into the looper (or the caller on the main thread).
        if catch_unwind(AssertUnwindSafe(|| call(MainThread(PhantomData)))).is_err() {
            error!("A call on the platform thread panicked");
        }
    }

    /// The looper's callback for the eventfd, on the main thread: run the calls queued so far.
    unsafe extern "C" fn drain(fd: c_int, _events: c_int, _data: *mut c_void) -> c_int {
        let drained = catch_unwind(|| {
            let mut count: u64 = 0;
            // SAFETY: reads the eventfd's counter into `count`, which resets it.
            unsafe { libc::read(fd, (&raw mut count).cast(), 8) };
            let Some(main) = MAIN.get() else { return };
            // Taken one at a time, without holding the queue: a call may queue another.
            loop {
                let call = lock(&main.calls).pop_front();
                let Some(call) = call else { break };
                guarded(call);
            }
        });
        if drained.is_err() {
            error!("Running the calls for the platform thread panicked");
        }
        // Keep watching.
        1
    }
}
