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

#[macro_export]
macro_rules! forward_event_to_remote {
    ($bridge:expr, $cancel_token: expr) => {
        process_forward_event_to_remote_handlers!(@internal $bridge, $cancel_token);
    };
}

#[macro_export]
macro_rules! process_forward_event_to_remote_handlers {
    (@internal $bridge:expr, $cancel_token: expr, $response:ty) => {
        paste::paste! {
            handle_single_to_remote_event!($bridge, $response).await;
        }
    };
    (@internal $bridge:expr, $cancel_token: expr $(,)?) => {};
}

#[macro_export]
macro_rules! handle_single_to_remote_event {
    (@internal $bridge:expr, $cancel_token:expr, $response:ty) => {
        paste::paste! {
            async fn [<handle_event_ $request:snake>](global_params: Arc<GlobalParams>) {
                let receiver = <$request>::get_dart_signal_receiver();
                loop {
                    tokio::select! {
                        // Check if cancel token is triggered
                        _ = $cancel_token.cancelled() => {
                            break;
                        }
                        // Try to receive message
                        Some(dart_signal) = receiver.recv() => {
                            $bridge.send_message(dart_signal);
                        }
                        // If channel is closed
                        else => break
                    }
                }
            }
            tokio::spawn([<handle_event_ $request:snake>]($global_params.clone()));
        }
    };
    (@internal $bridge:expr $(,)?) => {};
}
