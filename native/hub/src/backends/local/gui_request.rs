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
