use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Serialize;
use uuid::Uuid;

use crate::macros::RequestScope;
use crate::node::NodeContext;
use crate::request_error::FailedCall;
use crate::rpc::TargetRequest;
use crate::utils::ParamsExtractor;
use crate::{CallIdentified, Session, Signal, TypedRequest};

/// Serve one catalogue request, deferring decoding until after authorization.
pub(crate) async fn serve_remote<R>(
    decode: impl FnOnce() -> Result<R>,
    session: Option<Session>,
    library_id: Option<Uuid>,
    node: &Arc<NodeContext>,
) -> Result<(String, Vec<u8>)>
where
    R: TargetRequest
        + Signal<Response = <R as TypedRequest>::Response, Params = <R as ParamsExtractor>::Params>
        + ParamsExtractor
        + CallIdentified,
    <R as TypedRequest>::Response: Serialize,
{
    let addressed_library = library_id.filter(|_| R::SCOPE == RequestScope::Library);
    let params = if let Some(library_id) = addressed_library {
        if let Some(session) = &session {
            node.authorize_request(session, R::wire_name(), None, Some(library_id))
                .await?;
        }
        let library = node.library_pool.acquire_for_request(library_id).await?;
        match node.current_ui().await {
            Ok(ui_params) => Arc::new(ui_params.for_library(library)),
            Err(_) => Arc::new(node.params_for_library(library)?),
        }
    } else {
        // Frames without a library id (the @RR proxy sends these) address the UI library.
        let ui_params = node.current_ui().await?;
        if let Some(session) = &session {
            node.authorize_request(session, R::wire_name(), Some(&ui_params), None)
                .await?;
        }
        ui_params
    };
    let request = decode().context("Failed to deserialize request")?;
    let response = request
        .handle(request.extract_params(&params), session, &request)
        .await
        .map_err(|error| match request.call_id() {
            Some(call_id) => error.context(FailedCall { call_id }),
            None => error,
        })?;
    match response {
        Some(response) if !R::response_name().is_empty() => {
            Ok((R::response_name().to_owned(), rinf::serialize(&response)?))
        }
        _ => Ok((String::new(), Vec::new())),
    }
}
