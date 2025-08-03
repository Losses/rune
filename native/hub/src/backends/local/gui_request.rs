#[macro_export]
macro_rules! listen_local_gui_event {
    ($global_params:expr, $cancel_token:expr, $($req:tt)*) => {
        process_gui_requests!(@internal $global_params, $cancel_token, $($req)*);
    };
}

#[macro_export]
macro_rules! process_gui_requests {
    (@internal $global_params:expr, $cancel_token:expr, ($request:ty, $response:ty) $(, $rest:tt)*) => {
        handle_single_gui_event!($global_params, $cancel_token, $request, with_response);
        process_gui_requests!(@internal $global_params, $cancel_token $(, $rest)*);
    };
    (@internal $global_params:expr, $cancel_token:expr, $request:ty $(, $rest:tt)*) => {
        handle_single_gui_event!($global_params, $cancel_token, $request, without_response);
        process_gui_requests!(@internal $global_params, $cancel_token $(, $rest)*);
    };
    (@internal $global_params:expr, $cancel_token:expr $(,)?) => {};
}

#[macro_export]
macro_rules! handle_single_gui_event {
    ($global_params:expr, $cancel_token:expr, $request:ty, $response_type:tt) => {
        paste::paste! {
            let [<cancel_token_ $request:snake>]  = $cancel_token.clone();
            let [<handle_event_ $request:snake>] = |global_params: Arc<GlobalParams>| async move {
                let receiver = <$request>::get_dart_signal_receiver();

                tokio::select! {
                    _ = [<cancel_token_ $request:snake>].cancelled() => {
                        return;
                    }
                    _ = async {
                        while let Some(dart_signal) = receiver.recv().await {
                            let event = dart_signal.message;
                            let params = event.extract_params(&global_params);

                            match event.handle(params, None, &event).await {
                                Ok(_response) => {
                                    handle_response!(_response, $response_type);
                                }
                                Err(e) => {
                                    error!("{e:?}");
                                    CrashResponse {
                                        detail: format!("{e:#?}"),
                                    }
                                    .send_signal_to_dart();
                                }
                            }

                            if [<cancel_token_ $request:snake>].is_cancelled() {
                                break;
                            }
                        }
                    } => {}
                }
            };
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
