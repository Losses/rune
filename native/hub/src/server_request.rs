#[macro_export]
macro_rules! register_server_handlers {
    ($bridge:expr) => {
        process_server_handlers!(@internal $bridge);
    };
}

#[macro_export]
macro_rules! process_server_handlers {
    (@internal $bridge:expr, $response:ty) => {
        paste::paste! {
            $bridge.register_handler::<$response>(stringify!($response)).await;
        }
    };
    (@internal $bridge:expr $(,)?) => {};
}

#[macro_export]
macro_rules! implement_rinf_dart_signal_trait {
    ($($request:ty),*) => {
        $(
            impl RinfDartSignal for $request {
                fn name(&self) -> String {
                    stringify!($request).to_string()
                }
            }
        )*
    };
}
