use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use rinf::DartSignal;
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

pub use crate::macros::RequestScope;
use crate::macros::{RequestAccess, RequestSlots, RequestTranslator};
use crate::request_error::ExpectedRequestError;
use crate::utils::{GlobalParams, ParamsExtractor};
use crate::{Signal, TypedRequest};

/// Why an implementation cannot execute a request.
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

/// Execution assigned to one catalogue request on an implementation.
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
    params
        .node
        .implementation_for_scope::<R>()?
        .run(params, request)
        .await
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

#[cfg(test)]
mod tests {
    use super::{RequestSlots, Slot, TargetRequest, UnsupportedReason};
    use crate::messages::{
        FetchAllPlaylistsRequest, FetchCollectionByIdsRequest, FetchMediaFileByIdsRequest,
        GetLikedRequest, PlayRequest, SearchForRequest, SetLikedRequest, SetPlaybackModeRequest,
    };
    use subsonic::{SubsonicProbe, SubsonicRoles};

    #[test]
    fn subsonic_slots_whitelist_search_and_result_fetches() {
        let slots = RequestSlots::subsonic(&SubsonicProbe {
            api_version: "1.16.1".into(),
            open_subsonic: false,
            server_type: None,
            server_version: None,
            extensions: Vec::new(),
            roles: SubsonicRoles::default(),
            roles_known: true,
        });
        assert_eq!(SearchForRequest::slot(&slots), Slot::Translate);
        assert_eq!(FetchMediaFileByIdsRequest::slot(&slots), Slot::Translate);
        assert_eq!(FetchCollectionByIdsRequest::slot(&slots), Slot::Translate);
        assert_eq!(SetLikedRequest::slot(&slots), Slot::Native);
        assert_eq!(GetLikedRequest::slot(&slots), Slot::Native);
        assert_eq!(
            PlayRequest::slot(&slots),
            Slot::Unsupported {
                reason: UnsupportedReason::AccountRoleMissing
            }
        );
        assert_eq!(FetchAllPlaylistsRequest::slot(&slots), Slot::Translate);

        let jukebox = RequestSlots::subsonic(&SubsonicProbe {
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
        });
        assert_eq!(PlayRequest::slot(&jukebox), Slot::Translate);
        assert_eq!(SetPlaybackModeRequest::slot(&jukebox), Slot::Translate);
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
