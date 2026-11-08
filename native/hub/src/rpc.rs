use std::sync::Arc;

use anyhow::{Context, Result};
use rinf::DartSignal;
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::backends::remote::RendererLink;
use crate::macros::{RequestAccess, RequestScope, RequestSlots, request_access};
use crate::messages::RequestCapability;
use crate::node::{CatalogTarget, NodeContext, SessionBinding};
use crate::request_error::ExpectedRequestError;
use crate::utils::{GlobalParams, ParamsExtractor};
use crate::{Signal, TypedRequest};

/// Why a target cannot execute a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum UnsupportedReason {
    #[error("not implemented")]
    NotImplemented,
    #[error("only available on the owning device")]
    OwnerOnly,
    #[error("peer needs a newer protocol version")]
    PeerTooOld,
    #[error("account role is missing")]
    AccountRoleMissing,
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
    const INTRODUCED_MINOR: u32;

    fn slot(slots: &RequestSlots) -> Slot;
}

/// An RPC execution target and its complete set of request slots.
pub struct RpcTarget {
    slots: RequestSlots,
    renderer: Option<Arc<RendererLink>>,
}

/// Where a request type goes, before the renderer link is resolved.
enum TargetChoice {
    Local,
    /// The @RR mode: the device the UI library lives on.
    RemotePeer(Option<Arc<RendererLink>>),
    Attached(Arc<RendererLink>),
}

impl RpcTarget {
    pub fn local() -> Self {
        Self {
            slots: RequestSlots::local(),
            renderer: None,
        }
    }

    pub fn rune(renderer: Arc<RendererLink>) -> Self {
        let mut slots = Self::rune_slots();
        slots.narrow_to_minor(renderer.peer_version().minor);
        Self {
            slots,
            renderer: Some(renderer),
        }
    }

    /// Construct a Subsonic target from the connection-time capability probe.
    pub fn subsonic(probe: &subsonic::SubsonicProbe) -> Self {
        Self {
            slots: RequestSlots::subsonic(probe),
            renderer: None,
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
        slots.get_sync_settings_request = owner_only;
        slots.set_sync_settings_request = owner_only;
        slots.set_download_rule_request = owner_only;
        slots.remove_download_rule_request = owner_only;
        slots.test_library_initialized_request = owner_only;
        slots.close_library_request = owner_only;
        slots
    }

    pub fn select<R: TargetRequest>(node: &NodeContext, _request: &R) -> Result<Self> {
        Self::select_type::<R>(node)
    }

    pub(crate) fn select_type<R: TargetRequest>(node: &NodeContext) -> Result<Self> {
        Ok(match target_choice::<R>(node)? {
            TargetChoice::Local => Self::local(),
            TargetChoice::RemotePeer(Some(renderer)) => Self::rune(renderer),
            TargetChoice::RemotePeer(None) => match node.session_binding()? {
                SessionBinding::RemotePeer(renderer) => Self::rune(renderer),
                _ => anyhow::bail!("Remote peer session binding changed during target selection"),
            },
            TargetChoice::Attached(renderer) => Self::rune(renderer),
        })
    }

    /// The slots a request type would use now. Unlike `select_type` it needs no live
    /// link, so a reconnecting @RR target still reports its Rune slots.
    pub(crate) fn select_capability_type<R: TargetRequest>(node: &NodeContext) -> Result<Self> {
        Ok(match target_choice::<R>(node)? {
            TargetChoice::Local => Self::local(),
            TargetChoice::Attached(renderer) => Self::rune(renderer),
            TargetChoice::RemotePeer(Some(renderer)) => Self::rune(renderer),
            TargetChoice::RemotePeer(None) => Self {
                slots: Self::rune_slots(),
                renderer: None,
            },
        })
    }

    pub(crate) fn capability<R: TargetRequest>(&self) -> RequestCapability {
        self.slot_type::<R>().into()
    }

    pub(crate) fn slot_type<R: TargetRequest>(&self) -> Slot {
        R::slot(&self.slots)
    }

    /// Replace one request's slot, for tests that exercise slot outcomes.
    #[cfg(test)]
    pub(crate) fn with_slots(mut self, edit: impl FnOnce(&mut RequestSlots)) -> Self {
        edit(&mut self.slots);
        self
    }

    pub fn slot<R: TargetRequest>(&self, _request: &R) -> Slot {
        self.slot_type::<R>()
    }
}

impl RequestSlots {
    /// Build the Subsonic whitelist. jukeboxControl covers transport commands,
    /// queue status, and queue add/remove operations; other catalogue entries stay hidden.
    pub fn subsonic(probe: &subsonic::SubsonicProbe) -> Self {
        let mut slots = Self::unimplemented();
        let slot = if probe.roles_known && probe.roles.jukebox_role {
            Slot::Translate
        } else {
            Slot::Unsupported {
                reason: UnsupportedReason::AccountRoleMissing,
            }
        };
        slots.play_request = slot;
        slots.pause_request = slot;
        slots.next_request = slot;
        slots.previous_request = slot;
        slots.seek_request = slot;
        slots.volume_request = slot;
        slots.switch_request = slot;
        slots.load_request = slot;
        slots.remove_request = slot;
        slots.session_snapshot_request = slot;
        slots.refresh_session_request = slot;
        slots
    }
}

fn target_choice<R: TargetRequest>(node: &NodeContext) -> Result<TargetChoice> {
    if request_access(R::wire_name()) == Some(RequestAccess::NodeLocal) {
        return Ok(TargetChoice::Local);
    }
    let remote_peer = matches!(node.catalog_target()?, CatalogTarget::RemotePeer);
    match R::SCOPE {
        RequestScope::Node => Ok(TargetChoice::Local),
        // Library requests follow the open record's catalog; a remote peer's link may be
        // missing while it reconnects, and selection reports that typed error itself.
        RequestScope::Library if remote_peer => Ok(TargetChoice::RemotePeer(node.renderer_link()?)),
        RequestScope::Library => Ok(TargetChoice::Local),
        RequestScope::Session if !remote_peer && !routes_to_attached_renderer::<R>() => {
            Ok(TargetChoice::Local)
        }
        RequestScope::Session => match node.session_binding() {
            Ok(SessionBinding::Local) => Ok(TargetChoice::Local),
            Ok(SessionBinding::Attached(renderer)) => Ok(TargetChoice::Attached(renderer)),
            Ok(SessionBinding::RemotePeer(renderer)) => {
                Ok(TargetChoice::RemotePeer(Some(renderer)))
            }
            Err(_) if remote_peer => Ok(TargetChoice::RemotePeer(None)),
            Err(error) => Err(error),
        },
    }
}

/// A request introduced in a protocol minor newer than the peer's cannot run there;
/// a slot that is already unsupported keeps its own reason.
pub(crate) fn narrow_slot(slot: Slot, introduced_minor: u32, peer_minor: u32) -> Slot {
    match slot {
        Slot::Unsupported { .. } => slot,
        _ if introduced_minor > peer_minor => Slot::Unsupported {
            reason: UnsupportedReason::PeerTooOld,
        },
        _ => slot,
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
    let target = RpcTarget::select_type::<R>(&params.node)?;
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
    use super::{
        RequestCapability, RpcTarget, Slot, UnsupportedReason, routes_to_attached_renderer,
    };
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

    #[test]
    fn subsonic_slots_reflect_jukebox_role_and_hide_unmapped_requests() {
        use crate::messages::CapabilityReason;
        use crate::messages::RequestCapabilities;
        use crate::messages::{
            FetchAllPlaylistsRequest, LoadRequest, NextRequest, PauseRequest, PreviousRequest,
            RefreshSessionRequest, RemoveRequest, SeekRequest, SessionSnapshotRequest,
            SwitchRequest, VolumeRequest,
        };

        let mut probe = subsonic::SubsonicProbe {
            api_version: "1.16.1".into(),
            open_subsonic: false,
            server_type: None,
            server_version: None,
            extensions: Vec::new(),
            roles: subsonic::SubsonicRoles::default(),
            roles_known: true,
        };
        probe.roles.jukebox_role = true;
        let enabled = RpcTarget::subsonic(&probe);
        assert_eq!(enabled.slot(&PauseRequest {}), Slot::Translate);
        assert_eq!(enabled.slot(&NextRequest {}), Slot::Translate);
        assert_eq!(enabled.slot(&PreviousRequest {}), Slot::Translate);
        assert_eq!(
            enabled.slot(&SeekRequest {
                position_seconds: 12.0
            }),
            Slot::Translate
        );
        assert_eq!(
            enabled.slot(&VolumeRequest { volume: 0.5 }),
            Slot::Translate
        );
        assert_eq!(
            enabled.slot(&SwitchRequest {
                index: 1,
                expected_revision: None
            }),
            Slot::Translate
        );
        assert_eq!(enabled.slot(&LoadRequest { index: 0 }), Slot::Translate);
        assert_eq!(
            enabled.slot(&RemoveRequest {
                index: 0,
                expected_revision: None
            }),
            Slot::Translate
        );
        assert_eq!(enabled.slot(&SessionSnapshotRequest {}), Slot::Translate);
        assert_eq!(enabled.slot(&RefreshSessionRequest {}), Slot::Translate);
        probe.roles.jukebox_role = false;
        let disabled = RpcTarget::subsonic(&probe);
        assert_eq!(
            disabled.slot(&PauseRequest {}),
            Slot::Unsupported {
                reason: UnsupportedReason::AccountRoleMissing
            }
        );
        assert_eq!(
            RequestCapabilities::from_target(&disabled).pause_request,
            RequestCapability::Disabled {
                reason: CapabilityReason::AccountRoleMissing
            }
        );
        assert_eq!(
            disabled.slot(&FetchAllPlaylistsRequest {}),
            Slot::Unsupported {
                reason: UnsupportedReason::NotImplemented
            }
        );
        assert_eq!(
            RequestCapabilities::from_target(&disabled).fetch_all_playlists_request,
            RequestCapability::Hidden
        );
    }
}

#[cfg(test)]
mod narrow_tests {
    use super::{Slot, UnsupportedReason, narrow_slot};

    #[test]
    fn requests_newer_than_the_peer_minor_are_unsupported() {
        let too_old = Slot::Unsupported {
            reason: UnsupportedReason::PeerTooOld,
        };
        assert_eq!(narrow_slot(Slot::Forward, 1, 0), too_old);
        assert_eq!(narrow_slot(Slot::Forward, 1, 1), Slot::Forward);
        assert_eq!(narrow_slot(Slot::Forward, 0, 3), Slot::Forward);
        let owner_only = Slot::Unsupported {
            reason: UnsupportedReason::OwnerOnly,
        };
        assert_eq!(narrow_slot(owner_only, 2, 0), owner_only);
    }
}
