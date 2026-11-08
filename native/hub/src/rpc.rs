use std::sync::Arc;

use anyhow::{Context, Result};
use rinf::DartSignal;
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::backends::remote::RendererLink;
use crate::macros::{RequestAccess, RequestScope, RequestSlots, request_access};
use crate::node::NodeContext;
use crate::request_error::ExpectedRequestError;
use crate::utils::{GlobalParams, ParamsExtractor, RunningMode};
use crate::{Signal, TypedRequest};

/// Why a target cannot execute a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum UnsupportedReason {
    #[error("not implemented")]
    NotImplemented,
    #[error("only available on the owning device")]
    OwnerOnly,
}

/// Execution assigned to one catalogue request on a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Native,
    Forward,
    Translate,
    Unsupported { reason: UnsupportedReason },
}

/// Scope and typed slot access generated from the request catalogue.
pub trait TargetRequest: TypedRequest {
    const SCOPE: RequestScope;
    const USES_CONTROLLER_LIBRARY: bool;

    fn slot(slots: &RequestSlots) -> Slot;
}

/// An RPC execution target and its complete set of request slots.
pub struct RpcTarget {
    slots: RequestSlots,
    renderer: Option<Arc<RendererLink>>,
}

impl RpcTarget {
    pub fn local() -> Self {
        Self {
            slots: RequestSlots::local(),
            renderer: None,
        }
    }

    pub fn rune(renderer: Arc<RendererLink>) -> Self {
        Self {
            slots: Self::rune_slots(),
            renderer: Some(renderer),
        }
    }

    pub(crate) fn rune_slots() -> RequestSlots {
        let mut slots = RequestSlots::rune();
        // These owner operations have no registered remote handler.
        let owner_only = Slot::Unsupported {
            reason: UnsupportedReason::OwnerOnly,
        };
        slots.attach_renderer_request = owner_only;
        slots.detach_renderer_request = owner_only;
        slots.list_renderers_request = owner_only;
        slots.handoff_request = owner_only;
        slots.list_controllers_request = owner_only;
        slots.detach_controller_request = owner_only;
        slots
    }

    pub fn select<R: TargetRequest>(node: &NodeContext, _request: &R) -> Result<Self> {
        if routes_to_attached_renderer::<R>()
            && let Some(renderer) = node.renderer_link()?
        {
            return Ok(Self::rune(renderer));
        }
        Ok(Self::local())
    }

    /// Replace one request's slot, for tests that exercise slot outcomes.
    #[cfg(test)]
    pub(crate) fn with_slots(mut self, edit: impl FnOnce(&mut RequestSlots)) -> Self {
        edit(&mut self.slots);
        self
    }

    pub fn slot<R: TargetRequest>(&self, _request: &R) -> Slot {
        R::slot(&self.slots)
    }
}

/// Whether a request goes to the attached renderer rather than this node. Session
/// requests a peer may run do; this node's own coordinators (attach, detach, handoff,
/// renderer and controller lists) are node-local and stay here even while attached, and
/// mix queries resolve against this device's library before casting items.
pub(crate) fn routes_to_attached_renderer<R: TargetRequest>() -> bool {
    R::SCOPE == RequestScope::Session
        && !R::USES_CONTROLLER_LIBRARY
        && !matches!(
            request_access(R::wire_name()),
            None | Some(RequestAccess::NodeLocal)
        )
}

/// Dispatch a local GUI request inside the RPC layer using its declared scope.
pub async fn dispatch_gui_request<R>(
    params: &GlobalParams,
    request: &R,
) -> Result<Option<<R as TypedRequest>::Response>>
where
    R: TargetRequest
        + Signal<Response = <R as TypedRequest>::Response, Params = <R as ParamsExtractor>::Params>
        + ParamsExtractor
        + DartSignal
        + Serialize,
    <R as TypedRequest>::Response: DeserializeOwned,
{
    // The @RR proxy's node-local listener retains its native dispatch.
    let target = if matches!(params.running_mode, RunningMode::Server) {
        RpcTarget::local()
    } else {
        RpcTarget::select(&params.node, request)?
    };
    run_on_target(&target, params, request).await
}

/// Run a request on an already selected target according to its slot.
pub(crate) async fn run_on_target<R>(
    target: &RpcTarget,
    params: &GlobalParams,
    request: &R,
) -> Result<Option<<R as TypedRequest>::Response>>
where
    R: TargetRequest
        + Signal<Response = <R as TypedRequest>::Response, Params = <R as ParamsExtractor>::Params>
        + ParamsExtractor
        + DartSignal
        + Serialize,
    <R as TypedRequest>::Response: DeserializeOwned,
{
    match target.slot(request) {
        Slot::Native => {
            request
                .handle(request.extract_params(params), None, request)
                .await
        }
        Slot::Forward => {
            let renderer = target
                .renderer
                .as_ref()
                .context("Forward slot has no renderer link")?;
            Ok(Some(renderer.call(request).await?))
        }
        Slot::Unsupported { reason } => Err(ExpectedRequestError::Unsupported {
            request: R::wire_name(),
            reason,
        }
        .into()),
        Slot::Translate => Err(ExpectedRequestError::Unsupported {
            request: R::wire_name(),
            reason: UnsupportedReason::NotImplemented,
        }
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::routes_to_attached_renderer;
    use crate::messages::{
        DetachRendererRequest, HandoffRequest, ListControllersRequest, ListRenderersRequest,
        OperatePlaybackWithMixQueryRequest, PauseRequest, PlayRequest,
    };

    #[test]
    fn node_coordinators_stay_local_while_a_renderer_is_attached() {
        assert!(routes_to_attached_renderer::<PlayRequest>());
        assert!(routes_to_attached_renderer::<PauseRequest>());
        assert!(!routes_to_attached_renderer::<HandoffRequest>());
        assert!(!routes_to_attached_renderer::<ListRenderersRequest>());
        assert!(!routes_to_attached_renderer::<DetachRendererRequest>());
        assert!(!routes_to_attached_renderer::<ListControllersRequest>());
        assert!(!routes_to_attached_renderer::<
            OperatePlaybackWithMixQueryRequest,
        >());
    }
}
