#[macro_export]
macro_rules! listen_server_event {
    ($connection:expr, $($req:tt)*) => {
        process_server_requests!(@internal $connection, $($req)*);
    };
}

#[macro_export]
macro_rules! process_server_requests {
    (@internal $connection:expr, ($request:ty, $response:ty) $(, $rest:tt)*) => {
        handle_single_server_event!($connection, $request, with_response);
        process_server_requests!(@internal $connection $(, $rest)*);
    };
    (@internal $connection:expr, $request:ty $(, $rest:tt)*) => {
        handle_single_server_event!($connection, $request, without_response);
        process_server_requests!(@internal $connection $(, $rest)*);
    };
    (@internal $connection:expr $(,)?) => {};
}

#[macro_export]
macro_rules! handle_single_server_event {
    ($connection:expr, $request:ty, $response_type:tt) => {
        paste::paste! {
            async fn [<handle_event_ $request:snake>](server: Arc<GlobalParams>) {
                let receiver = <$request>::get_dart_signal_receiver();
                while let Some(dart_signal) = receiver.recv().await {
                    let event = dart_signal.message;
                    let params = event.extract_params(&server);
                    match event.handle(params, &event).await {
                        Ok(_response) => {
                            handle_response!(_response, $response_type);
                        }
                        Err(e) => {
                            error!("{:?}", e);
                            CrashResponse {
                                detail: format!("{:#?}", e),
                            }
                            .send_signal_to_dart();
                        }
                    }
                }
            }
            tokio::spawn([<handle_event_ $request:snake>]($connection.clone()));
        }
    };
}
