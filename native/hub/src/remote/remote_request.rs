#[macro_export]
macro_rules! register_remote_handlers {
    ($bridge:expr, $($response:ty),*) => {
        $(
            $bridge
                .register_handler::<$response>(stringify!($response))
                .await;
        )*
    };
}

#[macro_export]
macro_rules! implement_rinf_rust_signal_trait {
    ($($request:ty),*) => {
        $(
            impl RinfRustSignal for $request {
                fn send(&self) {
                    self.send_signal_to_dart()
                }

                fn name(&self) -> String {
                    stringify!($request).to_string()
                }

                fn encode_message(&self) -> Vec<u8> {
                    self.encode_to_vec()
                }
            }
        )*
    };
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
    ($bridge:expr, $cancel_token:expr, $write:expr, $($request:ty),* $(,)?) => {
        $(
            process_forward_event_to_remote_handlers!(@internal $bridge, $cancel_token, $write, $request);
        )*
    };
}

#[macro_export]
macro_rules! process_forward_event_to_remote_handlers {
    (@internal $bridge:expr, $cancel_token:expr, $write:expr, $request:ty) => {
        paste::paste! {
            handle_single_to_remote_event!($bridge, $cancel_token, $write, $request);
        }
    };
    (@internal $bridge:expr, $cancel_token:expr, $write:expr $(,)?) => {};
}

#[macro_export]
macro_rules! handle_single_to_remote_event {
    ($bridge:expr, $cancel_token:expr, $write:expr, $request:ty) => {
        paste::paste! {
            let [<cancel_token_ $request:snake>] = Arc::clone(&$cancel_token);
            let write_clone = Arc::clone(&$write);
            let [<handle_event_ $request:snake>] = || async move {
                let receiver = <$request>::get_dart_signal_receiver();
                loop {
                    tokio::select! {
                        _ = [<cancel_token_ $request:snake>].cancelled() => {
                            break;
                        }
                        Some(dart_signal) = receiver.recv() => {
                            // Encode the message
                            let payload = dart_signal.binary;

                            let type_name = dart_signal.message.name();
                            let encoded_message = encode_message(&type_name, &payload, Some(Uuid::new_v4()));

                            // Send the message
                            let result = write_clone.lock().await
                                .send(TungsteniteMessage::Binary(encoded_message.into()))
                                .await;

                            if let Err(e) = result {
                                CrashResponse {
                                    detail: format!("Failed to send message: {}", e),
                                }.send();
                            }
                        }
                    }
                }
            };

            tokio::spawn([<handle_event_ $request:snake>]());
        }
    };
    (@internal $bridge:expr $(,)?) => {};
}
