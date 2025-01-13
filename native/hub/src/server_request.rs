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
