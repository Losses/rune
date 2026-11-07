use std::{future::Future, sync::Arc};

use axum::{
    Extension, Json,
    body::Body,
    extract::{Query, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use discovery::{
    access_policy::{AccessPolicy, PeerIdentity},
    server::UserStatus,
};
use futures::{StreamExt, stream};

use super::{
    ServerManager,
    libraries::{LibraryQuery, request_library},
};
use crate::{access_policy::Action, library_pool::LibraryHandle, node::NodeContext};

#[derive(Clone)]
pub struct AuthorizedLibrary(pub LibraryHandle);

pub(super) fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({"error": "forbidden"})),
    )
        .into_response()
}

async fn allowed(node: &NodeContext, peer: &PeerIdentity, action: &Action) -> bool {
    node.permission_manager
        .read()
        .await
        .verify_by_fingerprint(&peer.fingerprint)
        .await
        .is_some_and(|user| {
            user.status == UserStatus::Approved && user.public_key == peer.public_key
        })
        && node.access_policy.check(peer, action).is_ok()
}

/// Keep authorization active through dispatch and through the last body chunk.
pub(super) async fn guarded_response(
    node: Arc<NodeContext>,
    peer: PeerIdentity,
    action: Action,
    response: impl Future<Output = Response>,
) -> Response {
    let mut grants = node.access_policy.changes();
    let mut trust = node.permission_manager.read().await.changes();
    let valid = || async { allowed(&node, &peer, &action).await };
    if !valid().await {
        return forbidden();
    }
    let revoked = async {
        loop {
            tokio::select! {
                result = grants.changed() => if result.is_err() { return; },
                result = trust.changed() => if result.is_err() { return; },
                _ = node.token.cancelled() => return,
            }
            if !valid().await {
                return;
            }
        }
    };
    let response = tokio::select! {
        biased;
        _ = revoked => return forbidden(),
        response = response => response,
    };
    let (parts, body) = response.into_parts();
    let stream = stream::unfold(
        (body.into_data_stream(), node, peer, action, grants, trust),
        |(mut body, node, peer, action, mut grants, mut trust)| async move {
            loop {
                if !allowed(&node, &peer, &action).await {
                    return None;
                }
                tokio::select! {
                    chunk = body.next() => return chunk.map(|chunk| (chunk, (body, node, peer, action, grants, trust))),
                    result = grants.changed() => if result.is_err() { return None; },
                    result = trust.changed() => if result.is_err() { return None; },
                    _ = node.token.cancelled() => return None,
                }
            }
        },
    );
    Response::from_parts(parts, Body::from_stream(stream))
}

/// Legacy media paths select the query library, current UI library, or sole served library.
pub(super) async fn require_library_read(
    State(manager): State<Arc<ServerManager>>,
    Extension(peer): Extension<PeerIdentity>,
    mut request: Request,
    next: Next,
) -> Response {
    let query = match Query::<LibraryQuery>::try_from_uri(request.uri()) {
        Ok(Query(query)) => query,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if request.uri().path().starts_with("/files/cache/") && query.library_id.is_none() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let library = match request_library(&manager, query).await {
        Ok(library) => library,
        Err(status) => return status.into_response(),
    };
    let node = match manager.node() {
        Ok(node) => node,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    request
        .extensions_mut()
        .insert(AuthorizedLibrary(library.clone()));
    let library_id = library.descriptor.library_id;
    // Covers and metadata serve a controller's browsing; library files are the
    // audio itself and need a library grant.
    let path = request.uri().path();
    let action = if path.starts_with("/files/cache/") || path.starts_with("/media/") {
        Action::ConnectionView { library_id }
    } else {
        Action::SyncPull { library_id }
    };
    guarded_response(node, peer, action, next.run(request)).await
}
