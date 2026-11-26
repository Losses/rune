//! Registering a texture with a registry that refuses it, as the macOS engine does by answering
//! 0 to `registerTexture:`.
//!
//! The registry is messaged on the main thread, so this test is its own `main` instead of a
//! harness test, which runs elsewhere and leaves nobody to run the main queue.

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    macos::refused_registration_is_an_error_and_the_next_one_registers();
    println!("test refused_registration_is_an_error_and_the_next_one_registers ... ok");
}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::{Cell, RefCell};
    use std::ptr::NonNull;
    use std::sync::Arc;

    use flutter_texture::{
        Acquired, FrameSource, Kind, PlatformHandle, TextureError, attach, register,
    };
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};

    /// What the registry answers and what it was sent.
    #[derive(Default)]
    struct Calls {
        /// The answer to the next `registerTexture:`.
        answer: Cell<i64>,
        registered: Cell<usize>,
        unregistered: RefCell<Vec<i64>>,
    }

    define_class!(
        // SAFETY: `NSObject` has no subclassing requirements, and `Registry` does not implement
        // `Drop`.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "RuneTestTextureRegistry"]
        #[ivars = Calls]
        struct Registry;

        impl Registry {
            #[unsafe(method(registerTexture:))]
            fn register_texture(&self, _texture: &AnyObject) -> i64 {
                let calls = self.ivars();
                calls.registered.set(calls.registered.get() + 1);
                calls.answer.get()
            }

            #[unsafe(method(unregisterTexture:))]
            fn unregister_texture(&self, id: i64) {
                self.ivars().unregistered.borrow_mut().push(id);
            }

            #[unsafe(method(textureFrameAvailable:))]
            fn texture_frame_available(&self, _id: i64) {}
        }

        unsafe impl NSObjectProtocol for Registry {}
    );

    impl Registry {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(Calls::default());
            // SAFETY: `init` of `NSObject` takes no arguments and returns the object.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// A producer that never has a frame: the registry here asks for none.
    struct NoFrames;

    impl FrameSource for NoFrames {
        fn acquire(&self) -> Option<Acquired> {
            None
        }
    }

    pub(super) fn refused_registration_is_an_error_and_the_next_one_registers() {
        let mtm = MainThreadMarker::new().expect("the test's `main` runs on the main thread");
        let registry = Registry::new(mtm);
        let handle = NonNull::from(&*registry).cast();
        // SAFETY: `registry` is a live object that answers the three messages of a texture
        // registry, and it outlives every call below.
        unsafe { attach(PlatformHandle::TextureRegistry(handle)) }.expect("attach the registry");
        let calls = registry.ivars();

        // The macOS engine's answer for a texture it did not register.
        calls.answer.set(0);
        let refused = pollster::block_on(register(Arc::new(NoFrames), Kind::Pixels));
        match refused {
            Err(TextureError::Engine(_)) => {}
            Err(other) => panic!("a refused registration reported {other:?}"),
            Ok(texture) => panic!(
                "a refused registration returned texture {} as if it were usable",
                texture.id()
            ),
        }
        assert_eq!(calls.registered.get(), 1);
        assert!(
            calls.unregistered.borrow().is_empty(),
            "nothing was registered, so nothing is unregistered"
        );

        // The next registration is a new attempt, and what the engine registers is a texture.
        calls.answer.set(7);
        let texture = pollster::block_on(register(Arc::new(NoFrames), Kind::Pixels))
            .expect("the engine registered the texture");
        assert_eq!(texture.id(), 7);
        assert_eq!(calls.registered.get(), 2);
        pollster::block_on(texture.unregister());
        assert_eq!(*calls.unregistered.borrow(), [7]);
    }
}
