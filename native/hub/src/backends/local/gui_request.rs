use std::sync::Arc;

use log::{error, warn};
use rinf::{DartSignal, DartSignalPack, SignalReceiver};
use serde::{Serialize, de::DeserializeOwned};
use tokio_util::sync::CancellationToken;

use crate::messages::{CrashResponse, RequestErrorKind, RequestFailedResponse};
use crate::request_error::{classify, request_failure_cause};
use crate::rpc::{TargetRequest, dispatch_gui_request};
use crate::utils::{GlobalParams, ParamsExtractor, RinfRustSignal};
use crate::{CallIdentified, Signal, TypedRequest};

/// Catalogue-generated response emission, including requests with no response.
pub(crate) trait GuiRequest: TypedRequest {
    fn emit_response(response: Option<Self::Response>, emit: &impl Fn(&dyn RinfRustSignal));
}

/// Listen sequentially for one request type; other types have their own tasks.
pub(crate) async fn listen_gui<R>(params: Arc<GlobalParams>, cancel: CancellationToken)
where
    R: GuiRequest
        + TargetRequest
        + Signal<Response = <R as TypedRequest>::Response, Params = <R as ParamsExtractor>::Params>
        + ParamsExtractor
        + CallIdentified
        + DartSignal
        + Serialize,
    <R as TypedRequest>::Response: DeserializeOwned,
{
    listen_gui_with_io::<R>(params, cancel, R::get_dart_signal_receiver(), |signal| {
        signal.send_to_dart();
    })
    .await;
}

// Keep the receiver and signal sink explicit so tests can use isolated rinf channels.
pub(crate) async fn listen_gui_with_io<R>(
    params: Arc<GlobalParams>,
    cancel: CancellationToken,
    receiver: SignalReceiver<DartSignalPack<R>>,
    emit: impl Fn(&dyn RinfRustSignal),
) where
    R: GuiRequest
        + TargetRequest
        + Signal<Response = <R as TypedRequest>::Response, Params = <R as ParamsExtractor>::Params>
        + ParamsExtractor
        + CallIdentified
        + DartSignal
        + Serialize,
    <R as TypedRequest>::Response: DeserializeOwned,
{
    tokio::select! {
        biased;
        _ = cancel.cancelled() => {}
        _ = async {
            while let Some(dart_signal) = receiver.recv().await {
                let event = dart_signal.message;
                match dispatch_gui_request(&params, &event).await {
                    Ok(response) => R::emit_response(response, &emit),
                    Err(e) => {
                        let kind = classify(&e);
                        emit(&RequestFailedResponse::for_request(
                            &event,
                            R::wire_name(),
                            kind,
                            format!("{e:#}"),
                            request_failure_cause(&e),
                        ));
                        if kind == RequestErrorKind::Internal {
                            error!("Request {} failed: {:?}", R::wire_name(), e);
                            error!("Full error chain for {}: {:#}", R::wire_name(), e);
                            error!("Backtrace for {}: {:?}", R::wire_name(), e.backtrace());
                            emit(&CrashResponse { detail: format!("{e:#?}") });
                        } else {
                            warn!("Request {} failed: {e:#}", R::wire_name());
                        }
                    }
                }
                if cancel.is_cancelled() {
                    break;
                }
            }
        } => {}
    }
}

// R5b: remove this legacy macro chain and its paste identifiers when the @RR
// proxy replaces server_player_loop's local-only listeners.
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
                    biased;
                    _ = [<cancel_token_ $request:snake>].cancelled() => {
                        return;
                    }
                    _ = async {
                        while let Some(dart_signal) = receiver.recv().await {
                            let event = dart_signal.message;
                            match dispatch_gui_request(&global_params, &event).await {
                                Ok(_response) => {
                                    handle_response!(_response, $response_type);
                                }
                                Err(e) => {
                                    let kind = classify(&e);
                                    RequestFailedResponse::for_request(
                                        &event,
                                        stringify!($request),
                                        kind,
                                        format!("{e:#}"),
                                        $crate::request_failure_cause(&e),
                                    )
                                    .send_signal_to_dart();
                                    if kind == RequestErrorKind::Internal {
                                        error!("Request {} failed: {:?}", stringify!($request), e);
                                        error!("Full error chain for {}: {:#}", stringify!($request), e);
                                        let backtrace = e.backtrace();
                                        error!("Backtrace for {}: {:?}", stringify!($request), backtrace);
                                        CrashResponse {
                                            detail: format!("{e:#?}"),
                                        }
                                        .send_signal_to_dart();
                                    } else {
                                        warn!("Request {} failed: {e:#}", stringify!($request));
                                    }
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

#[cfg(test)]
mod tests {
    use crate::messages::{RequestErrorKind, RequestFailedResponse, SearchForRequest};

    #[test]
    fn search_request_failure_in_local_dispatch_keeps_call_id() {
        let request = SearchForRequest {
            call_id: 73,
            query_str: "missing".to_owned(),
            fields: Vec::new(),
            n: 30,
        };
        let failure = RequestFailedResponse::for_request(
            &request,
            "SearchForRequest",
            RequestErrorKind::NotFound,
            "failed".to_owned(),
            None,
        );

        assert_eq!(failure.call_id, Some(73));
    }
}
