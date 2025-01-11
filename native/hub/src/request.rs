use requests::define_request_types;

#[macro_export]
macro_rules! listen_local_gui_event {
    ($global_params:expr, $($req:tt)*) => {
        process_requests!(@internal $global_params, $($req)*);
    };
}

#[macro_export]
macro_rules! process_requests {
    (@internal $global_params:expr, ($request:ty, $response:ty) $(, $rest:tt)*) => {
        handle_single_event!($global_params, $request, with_response);
        process_requests!(@internal $global_params $(, $rest)*);
    };
    (@internal $global_params:expr, $request:ty $(, $rest:tt)*) => {
        handle_single_event!($global_params, $request, without_response);
        process_requests!(@internal $global_params $(, $rest)*);
    };
    (@internal $global_params:expr $(,)?) => {};
}

#[macro_export]
macro_rules! handle_single_event {
    ($global_params:expr, $request:ty, $response_type:tt) => {
        paste::paste! {
            async fn [<handle_event_ $request:snake>](global_params: Arc<GlobalParams>) {
                let receiver = <$request>::get_dart_signal_receiver();
                while let Some(dart_signal) = receiver.recv().await {
                    let event = dart_signal.message;
                    let params = event.extract_params(&global_params);
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
            tokio::spawn([<handle_event_ $request:snake>]($global_params.clone()));
        }
    };
}

#[macro_export]
macro_rules! handle_response {
    ($response:expr, with_response) => {
        if let Some(response) = $response {
            response.send_signal_to_dart();
        }
    };
    ($response:expr, without_response) => {};
}

define_request_types!();
