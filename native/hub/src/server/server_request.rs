use std::{
    collections::HashSet,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use anyhow::{Context, Error, Result};
use async_trait::async_trait;
use discovery::server::PermissionManager;
use serde::Serialize;
use sync_orm::causal_store::CommitPermit;
use tokio::sync::OwnedRwLockReadGuard;
use uuid::Uuid;

use crate::macros::{BodyLibraries, RequestAccess, RequestScope};
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
        let library = match &session {
            Some(session) if R::ACCESS == RequestAccess::ConnectionEdit => library
                .with_commit_permit(EditPermit::new(node, session, R::wire_name(), library_id)),
            _ => library,
        };
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

/// Lets an edit from another device commit only while the device may still make it.
///
/// The request's library handle carries the permit, and the library's sync store confirms it
/// once the edit's transaction holds the library's write lock, after anything the edit waited
/// for. Confirming repeats the request's authorization and keeps the permission manager
/// read-locked until the request drops the permit. Approving or blocking a device and setting
/// its grants take the permission manager for writing, so each such change comes either before
/// the confirmation, which then checks against it, or after the edit's transaction has ended.
struct EditPermit {
    node: Arc<NodeContext>,
    session: Session,
    request: &'static str,
    library_id: Uuid,
    permissions: Mutex<Option<OwnedRwLockReadGuard<PermissionManager>>>,
}

impl EditPermit {
    fn new(
        node: &Arc<NodeContext>,
        session: &Session,
        request: &'static str,
        library_id: Uuid,
    ) -> Arc<Self> {
        Arc::new(Self {
            node: node.clone(),
            session: session.clone(),
            request,
            library_id,
            permissions: Mutex::new(None),
        })
    }

    fn permissions(&self) -> MutexGuard<'_, Option<OwnedRwLockReadGuard<PermissionManager>>> {
        self.permissions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl CommitPermit for EditPermit {
    async fn confirm(&self) -> Result<()> {
        // A second write of the request checks under the lock it already holds: asking for the
        // lock again would wait behind a queued grant change, which waits for this request.
        let held = self.permissions().take();
        let permissions = match held {
            Some(permissions) => permissions,
            None => self.node.permission_manager.clone().read_owned().await,
        };
        self.node
            .authorize_request_in(
                &permissions,
                &self.session,
                self.request,
                None,
                Some(self.library_id),
            )
            .await?;
        *self.permissions() = Some(permissions);
        Ok(())
    }
}

/// The most distinct libraries one request body may name. A queue that spans hundreds of
/// libraries is not a real use, and the limit keeps the central authorization bounded before
/// any handler runs.
pub(crate) const MAX_BODY_LIBRARIES: usize = 256;

/// Authorize the libraries a request's body names, as its catalogue entry declares.
///
/// `PlayerReads` requests are scoped to the player, not to an addressed library, so each
/// named library is checked with the caller's connection view grant instead of the request's
/// node-wide Control rule. A request naming more than [`MAX_BODY_LIBRARIES`] distinct
/// libraries is refused before any per-library work.
async fn authorize_body_libraries<R: TargetRequest + NamedLibraries>(
    node: &NodeContext,
    session: &Session,
    addressed_library: Option<Uuid>,
    request: &R,
) -> Result<()> {
    match R::BODY_LIBRARIES {
        BodyLibraries::NoLibrary => return Ok(()),
        BodyLibraries::Authorized | BodyLibraries::PlayerReads => {}
    }
    let mut named = HashSet::new();
    let mut ordered = Vec::new();
    request.visit_libraries(&mut |library_id| {
        if named.len() > MAX_BODY_LIBRARIES {
            return;
        }
        if named.insert(library_id) {
            ordered.push(library_id);
        }
    });
    if ordered.len() > MAX_BODY_LIBRARIES {
        return Err(ExpectedRequestError::invalid_input(format!(
            "A request may name at most {MAX_BODY_LIBRARIES} libraries"
        ))
        .into());
    }
    let library_ids = ordered
        .into_iter()
        .map(|library_id| {
            Uuid::parse_str(library_id).map_err(|error| {
                Error::new(ExpectedRequestError::invalid_input(format!(
                    "Invalid library id: {error}"
                )))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    match R::BODY_LIBRARIES {
        BodyLibraries::Authorized => {
            for library_id in library_ids {
                node.authorize_request(session, R::wire_name(), None, Some(library_id))
                    .await?;
                let addressed_library =
                    addressed_library.ok_or(ExpectedRequestError::Unavailable {
                        what: "addressed library id",
                    })?;
                if library_id != addressed_library {
                    return Err(ExpectedRequestError::invalid_input(
                        "request library does not match the item's library",
                    )
                    .into());
                }
            }
        }
        BodyLibraries::PlayerReads => {
            node.authorize_library_views(session, library_ids).await?;
        }
        BodyLibraries::NoLibrary => {}
    }
    Ok(())
}
