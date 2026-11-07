#[macro_export]
macro_rules! listen_server_event {
    ($server:expr, $global_params:expr, $($req:tt)*) => {
        process_server_handlers!(@internal $server, $global_params, $($req)*);
    };
}

#[macro_export]
macro_rules! process_server_handlers {
    (@internal $server:expr, $global_params:expr, ($request:ty, $response:ty) $(, $rest:tt)*) => {
        register_single_handler!($server, $global_params, $request, with_response);
        process_server_handlers!(@internal $server, $global_params $(, $rest)*);
    };
    (@internal $server:expr, $global_params:expr, $request:ty $(, $rest:tt)*) => {
        register_single_handler!($server, $global_params, $request, without_response);
        process_server_handlers!(@internal $server, $global_params $(, $rest)*);
    };
    (@internal $server:expr, $global_params:expr $(,)?) => {};
}

#[macro_export]
macro_rules! register_single_handler {
    ($server:expr, $global_params:expr, $request:ty, $with_response:tt) => {
        paste::paste! {
            let global_params = Arc::downgrade(&$global_params);
            $server.register_handler(stringify!($request), move |payload, session, library_id| {
                let global_params = global_params.clone();
                async move {
                    use $crate::macros::{RequestClass, request_class};
                    use $crate::request_error::ExpectedRequestError;

                    let Some(global_params) = global_params.upgrade() else {
                        return (String::new(), Err(anyhow::anyhow!("Node has shut down")));
                    };
                    let request_class = request_class(stringify!($request));
                    let addressed_library = library_id.filter(|_| matches!(
                        request_class,
                        Some(RequestClass::LibraryRead | RequestClass::LibraryWrite)
                    ));
                    if addressed_library.is_none()
                        && matches!(
                            request_class,
                            Some(RequestClass::LibraryRead | RequestClass::LibraryWrite)
                        )
                    {
                        return (
                            String::new(),
                            Err(ExpectedRequestError::invalid_input(
                                "library-scoped request is missing library_id",
                            )
                            .into()),
                        );
                    }
                    let params = if let Some(library_id) = addressed_library {
                        if let Some(session) = &session
                            && let Err(error) = global_params.authorize_request(
                                session,
                                stringify!($request),
                                None,
                                Some(library_id),
                            ).await
                        {
                            return (String::new(), Err(error));
                        }
                        let library = match global_params.library_pool.acquire_for_request(library_id).await {
                            Ok(library) => library,
                            Err(error) => return (String::new(), Err(error)),
                        };
                        match global_params.current_ui().await {
                            Ok(ui_params) => Arc::new(ui_params.for_library(library)),
                            Err(_) => match global_params.params_for_library(library) {
                                Ok(params) => Arc::new(params),
                                Err(error) => return (String::new(), Err(error)),
                            },
                        }
                    } else {
                        let ui_params = match global_params.current_ui().await {
                            Ok(params) => params,
                            Err(error) => return (String::new(), Err(error)),
                        };
                        if let Some(session) = &session
                            && let Err(error) = global_params.authorize_request(
                                session,
                                stringify!($request),
                                Some(&ui_params),
                                None,
                            ).await
                        {
                            return (String::new(), Err(error));
                        }
                        ui_params
                    };
                    let buf = payload.as_slice();
                    let request = match rinf::deserialize::<$request>(buf) {
                        Ok(req) => req,
                        Err(e) => {
                            return (String::new(), Err(anyhow::Error::new(e).context("Failed to deserialize request")));
                        }
                    };

                    let handler_params = request.extract_params(&params);
                    match request.handle(handler_params, session, &request).await {
                        Ok(_response) => {
                            handle_server_response!(_response, $with_response)
                        }
                        Err(e) => {
                            (String::new(), Err(e))
                        }
                    }
                }
            }).await;
        }
    };
}

#[macro_export]
macro_rules! handle_server_response {
    ($response:expr, with_response) => {
        if let Some(response) = $response {
            (
                response.name(),
                rinf::serialize(&response).map_err(|e| anyhow::Error::new(e)),
            )
        } else {
            ("".to_owned(), Ok::<Vec<u8>, anyhow::Error>(Vec::new()))
        }
    };
    ($response:expr, without_response) => {
        ("".to_owned(), Ok::<Vec<u8>, anyhow::Error>(Vec::new()))
    };
}
