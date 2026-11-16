use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Serialize;
use uuid::Uuid;

use crate::macros::{BodyLibraries, RequestScope};
use crate::node::NodeContext;
use crate::request_error::{ExpectedRequestError, FailedCall};
use crate::rpc::{NamedLibraries, TargetRequest};
use crate::utils::ParamsExtractor;
use crate::{RequestCallId, ResponseCallId, Session, Signal, TypedRequest};

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
        + NamedLibraries
        + RequestCallId,
    <R as TypedRequest>::Response: Serialize,
    <R as TypedRequest>::Response: ResponseCallId,
{
    if R::SCOPE == RequestScope::Library && library_id.is_none() {
        return Err(ExpectedRequestError::Unavailable { what: "library id" }.into());
    }
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
        // Unaddressed library requests need the UI library; session-only requests do not.
        let ui_params = match node.current_ui().await {
            Ok(params) => params,
            Err(_) if matches!(R::SCOPE, RequestScope::Session | RequestScope::Node) => {
                Arc::new(node.params_for_ui()?)
            }
            Err(error) => return Err(error),
        };
        if let Some(session) = &session {
            node.authorize_request(session, R::wire_name(), Some(&ui_params), None)
                .await?;
        }
        ui_params
    };
    let request = decode().context("Failed to deserialize request")?;
    if let Some(session) = &session {
        authorize_body_libraries(node, session, addressed_library, &request).await?;
    }
    let response = request
        .handle(request.extract_params(&params), session, &request)
        .await
        .map_err(|error| match request.call_id() {
            Some(call_id) => error.context(FailedCall { call_id }),
            None => error,
        })?;
    match response {
        Some(mut response) if !R::response_name().is_empty() => {
            response.set_call_id(request.call_id());
            Ok((R::response_name().to_owned(), rinf::serialize(&response)?))
        }
        _ => Ok((String::new(), Vec::new())),
    }
}

/// Authorize the libraries a request's body names, as its catalogue entry declares.
async fn authorize_body_libraries<R: TargetRequest + NamedLibraries>(
    node: &NodeContext,
    session: &Session,
    addressed_library: Option<Uuid>,
    request: &R,
) -> Result<()> {
    match R::BODY_LIBRARIES {
        BodyLibraries::NoLibrary | BodyLibraries::PlayerReads => return Ok(()),
        BodyLibraries::Authorized => {}
    }
    let mut named = Vec::new();
    request.visit_libraries(&mut |library_id| {
        if !named.contains(&library_id) {
            named.push(library_id);
        }
    });
    for library_id in named {
        let library_id = Uuid::parse_str(library_id).map_err(|error| {
            ExpectedRequestError::invalid_input(format!("Invalid library id: {error}"))
        })?;
        node.authorize_request(session, R::wire_name(), None, Some(library_id))
            .await?;
        let addressed_library = addressed_library.ok_or(ExpectedRequestError::Unavailable {
            what: "addressed library id",
        })?;
        if library_id != addressed_library {
            return Err(ExpectedRequestError::invalid_input(
                "request library does not match the item's library",
            )
            .into());
        }
    }
    Ok(())
}
