use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{Context, Result};
use rinf::DartSignal;
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::backends::remote::RendererLink;
use crate::macros::{RequestAccess, RequestScope, RequestSlots, RequestTranslator};
use crate::messages::RequestCapability;
use crate::node::{CatalogTarget, NodeContext, SessionBinding};
use crate::request_error::ExpectedRequestError;
use crate::subsonic_binding::SubsonicBinding;
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
    const ACCESS: RequestAccess;
    const INTRODUCED_MINOR: u32;

    fn slot(slots: &RequestSlots) -> Slot;

    fn set_slot(_slots: &mut RequestSlots, _slot: Slot) {}

    fn translate<'a>(
        &'a self,
        _translator: &'a dyn RequestTranslator,
    ) -> RequestTranslationFuture<'a, Self::Response> {
        Box::pin(async {
            Err(ExpectedRequestError::Unsupported {
                request: Self::wire_name(),
                reason: UnsupportedReason::NotImplemented,
            }
            .into())
        })
    }
}

pub(crate) type RequestTranslationFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<Option<T>>> + Send + 'a>>;

/// An RPC execution target and its complete set of request slots.
pub struct RpcTarget {
    slots: RequestSlots,
    renderer: Option<Arc<RendererLink>>,
    subsonic: Option<Arc<SubsonicBinding>>,
}

/// Where a request type goes, before the renderer link is resolved.
enum TargetChoice {
    Local,
    Subsonic(Arc<SubsonicBinding>),
    /// The @RR mode: the device the UI library lives on.
    RemotePeer(Option<Arc<RendererLink>>),
    Attached(Arc<RendererLink>),
}

impl RpcTarget {
    pub fn local() -> Self {
        Self {
            slots: RequestSlots::local(),
            renderer: None,
            subsonic: None,
        }
    }

    pub fn rune(renderer: Arc<RendererLink>) -> Self {
        let mut slots = Self::rune_slots();
        slots.narrow_to_minor(renderer.peer_version().minor);
        Self {
            slots,
            renderer: Some(renderer),
            subsonic: None,
        }
    }

    /// Construct a Subsonic target from the connection-time capability probe.
    pub(crate) fn subsonic(binding: Arc<SubsonicBinding>) -> Self {
        Self {
            slots: RequestSlots::subsonic(&binding.probe),
            renderer: None,
            subsonic: Some(binding),
        }
    }

    pub(crate) fn rune_slots() -> RequestSlots {
        RequestSlots::rune()
    }

    pub fn select<R: TargetRequest>(node: &NodeContext, _request: &R) -> Result<Self> {
        Self::select_type::<R>(node)
    }

    pub(crate) fn select_type<R: TargetRequest>(node: &NodeContext) -> Result<Self> {
        Ok(match target_choice::<R>(node)? {
            TargetChoice::Local => Self::local(),
            TargetChoice::Subsonic(binding) => Self::subsonic(binding),
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
            TargetChoice::Subsonic(binding) => Self::subsonic(binding),
            TargetChoice::Attached(renderer) => Self::rune(renderer),
            TargetChoice::RemotePeer(Some(renderer)) => Self::rune(renderer),
            TargetChoice::RemotePeer(None) => Self {
                slots: Self::rune_slots(),
                renderer: None,
                subsonic: None,
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

fn target_choice<R: TargetRequest>(node: &NodeContext) -> Result<TargetChoice> {
    if R::SCOPE == RequestScope::Node {
        return Ok(TargetChoice::Local);
    }
    let catalog = node.catalog_target()?;
    if let CatalogTarget::Subsonic(binding) = &catalog {
        return Ok(match R::SCOPE {
            RequestScope::Node => TargetChoice::Local,
            RequestScope::Library | RequestScope::Session => {
                TargetChoice::Subsonic(Arc::clone(binding))
            }
        });
    }
    let remote_peer = matches!(catalog, CatalogTarget::RemotePeer);
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

/// Whether a request goes to the attached renderer rather than this node: player calls
/// do; this device's own calls are Node scope and never reach here. Mix queries resolve
/// against this device's library before casting items.
pub(crate) fn routes_to_attached_renderer<R: TargetRequest>() -> bool {
    R::SCOPE == RequestScope::Session
}

/// Dispatch a local GUI request inside the RPC layer using its declared scope.
pub(crate) async fn dispatch_gui_request<R>(
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
        Slot::Translate => {
            let binding = target
                .subsonic
                .as_ref()
                .context("Translate slot has no Subsonic binding")?;
            request.translate(binding.as_ref()).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RequestCapability, RequestSlots, RpcTarget, Slot, UnsupportedReason,
        routes_to_attached_renderer,
    };
    use crate::messages::{
        DetachRendererRequest, HandoffRequest, ListControllersRequest, ListRenderersRequest,
        LoadResolvedItemsRequest, PauseRequest, PlayRequest,
    };

    #[test]
    fn node_coordinators_stay_local_while_a_renderer_is_attached() {
        assert!(routes_to_attached_renderer::<PlayRequest>());
        assert!(routes_to_attached_renderer::<PauseRequest>());
        assert!(!routes_to_attached_renderer::<HandoffRequest>());
        assert!(!routes_to_attached_renderer::<ListRenderersRequest>());
        assert!(!routes_to_attached_renderer::<DetachRendererRequest>());
        assert!(!routes_to_attached_renderer::<ListControllersRequest>());
        assert!(routes_to_attached_renderer::<LoadResolvedItemsRequest>());
    }

    #[test]
    fn subsonic_slots_whitelist_search_and_result_fetches() {
        use crate::messages::RequestCapabilities;
        use crate::messages::{
            CollectionType, FetchAllPlaylistsRequest, FetchCollectionByIdsRequest,
            FetchMediaFileByIdsRequest, PlayRequest, SearchForRequest,
        };
        use subsonic::{SubsonicProbe, SubsonicRoles};
        let slots = RequestSlots::subsonic(&SubsonicProbe {
            api_version: "1.16.1".into(),
            open_subsonic: false,
            server_type: None,
            server_version: None,
            extensions: Vec::new(),
            roles: SubsonicRoles::default(),
            roles_known: true,
        });
        let enabled = RpcTarget {
            slots,
            renderer: None,
            subsonic: None,
        };
        assert_eq!(
            enabled.slot(&SearchForRequest {
                call_id: 1,
                query_str: String::new(),
                fields: Vec::new(),
                n: 10
            }),
            Slot::Translate
        );
        assert_eq!(
            enabled.slot(&FetchMediaFileByIdsRequest {
                ids: Vec::new(),
                bake_cover_arts: false
            }),
            Slot::Translate
        );
        assert_eq!(
            enabled.slot(&FetchCollectionByIdsRequest {
                collection_type: CollectionType::Artist,
                bake_cover_arts: false,
                ids: Vec::new()
            }),
            Slot::Translate
        );
        assert_eq!(
            enabled.slot(&PlayRequest {}),
            Slot::Unsupported {
                reason: UnsupportedReason::AccountRoleMissing
            }
        );
        assert_eq!(
            RequestCapabilities::from_target(&enabled).search_for_request,
            RequestCapability::Available
        );
        assert_eq!(
            enabled.slot(&FetchAllPlaylistsRequest {}),
            Slot::Unsupported {
                reason: UnsupportedReason::NotImplemented
            }
        );
        assert_eq!(
            RequestCapabilities::from_target(&enabled).fetch_all_playlists_request,
            RequestCapability::Hidden
        );

        let role_present = RpcTarget {
            slots: RequestSlots::subsonic(&SubsonicProbe {
                api_version: "1.16.1".into(),
                open_subsonic: false,
                server_type: None,
                server_version: None,
                extensions: Vec::new(),
                roles: SubsonicRoles {
                    jukebox_role: true,
                    ..Default::default()
                },
                roles_known: true,
            }),
            renderer: None,
            subsonic: None,
        };
        assert_eq!(
            role_present.slot(&PlayRequest {}),
            Slot::Unsupported {
                reason: UnsupportedReason::NotImplemented
            }
        );
        let roles_unknown = RpcTarget {
            slots: RequestSlots::subsonic(&SubsonicProbe {
                api_version: "1.16.1".into(),
                open_subsonic: false,
                server_type: None,
                server_version: None,
                extensions: Vec::new(),
                roles: SubsonicRoles {
                    jukebox_role: true,
                    ..Default::default()
                },
                roles_known: false,
            }),
            renderer: None,
            subsonic: None,
        };
        assert_eq!(
            roles_unknown.slot(&PlayRequest {}),
            Slot::Unsupported {
                reason: UnsupportedReason::AccountRoleMissing
            }
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
