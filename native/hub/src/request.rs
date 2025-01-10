use requests::define_request_types;

#[macro_export]
macro_rules! listen_local_gui_event {
    ($global_params:expr, $(($request:ty, $response:ty)),*) => {
        $(
            paste::paste! {
                async fn [<handle_event_ $request:snake>](global_params: Arc<GlobalParams>) {
                    let receiver = <$request>::get_dart_signal_receiver();

                    while let Some(dart_signal) = receiver.recv().await {
                        let event = dart_signal.message;
                        let params = event.extract_params(&global_params);

                        match event.handle(params, &event).await {
                            Ok(response) => {
                                if let Some(response) = response {
                                    response.send_signal_to_dart();
                                }
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
        )*
    };
    ($global_params:expr, $($request:ty),*) => {
        $(
            paste::paste! {
                async fn [<handle_event_ $request:snake>](global_params: Arc<GlobalParams>) {
                    let receiver = <$request>::get_dart_signal_receiver();

                    while let Some(dart_signal) = receiver.recv().await {
                        let event = dart_signal.message;
                        let params = event.extract_params(&global_params);

                        match event.handle(params, &event).await {
                            Ok(_) => (),
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
        )*
    };
}

define_request_types!();
