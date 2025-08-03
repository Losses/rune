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
    ($($t:ty),*) => {
        $(
            impl $crate::utils::RinfRustSignal for $t {
                fn name(&self) -> String {
                    stringify!($t).to_string()
                }

                fn encode_to_vec(&self) -> anyhow::Result<Vec<u8>> {
                    rinf::serialize(self).map_err(anyhow::Error::from)
                }

                fn send_to_dart(&self) {
                    self.send_signal_to_dart();
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
    ($bridge:expr, $cancel_token:expr, $write:expr, $($request:expr), *) => {
        $(
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
                                let payload = match rinf::serialize(&dart_signal.message) {
                                    Ok(x) => x,
                                    Err(e) => {
                                        CrashResponse {
                                            detail: format!("Failed to serialize message: {e}"),
                                        }.send_signal_to_dart();
                                        continue;
                                    }
                                };

                                let type_name = dart_signal.message.name();
                                let encoded_message = encode_message(&type_name, &payload, Some(Uuid::new_v4()));

                                // Send the message
                                let result = write_clone.lock().await
                                    .send(TungsteniteMessage::Binary(encoded_message.into()))
                                    .await;

                                if let Err(e) = result {
                                    CrashResponse {
                                        detail: format!("Failed to send message: {e}"),
                                    }.send_signal_to_dart();
                                }
                            }
                        }
                    }
                };

                tokio::spawn([<handle_event_ $request:snake>]());
            }
        )*
    };
}
