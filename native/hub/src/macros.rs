#[macro_export]
macro_rules! implement_rinf_rust_signal_trait {
    ($($t:ty),*) => {
        $(
            impl $crate::RinfRustSignal for $t {
                fn name(&self) -> String {
                    stringify!($t).to_string()
                }

                fn encode_to_vec(&self) -> anyhow::Result<Vec<u8>> {
                    rinf::serialize(self).map_err(anyhow::Error::from)
                }

                fn send_to_dart(&self) {
                    self.send_signal_to_dart();
                }
            }
        )*
    };
}

#[macro_export]
macro_rules! declare_remote_events {
    ($($t:ty => $scope:ident),+ $(,)?) => {
        pub const REMOTE_EVENT_DECLARATIONS: &[$crate::RemoteEventDeclaration] = &[
            $(
                $crate::RemoteEventDeclaration {
                    name: stringify!($t),
                    type_id: TypeId::of::<$t>(),
                    scope: $crate::RemoteEventScope::$scope,
                    decode: |payload| {
                        Ok(Box::new(rinf::deserialize::<$t>(payload)?) as Box<dyn $crate::RinfRustSignal>)
                    },
                },
            )+
        ];
    };
}

use std::sync::Arc;

use anyhow::{Error, Result};
use requests::define_request_types;
use rinf::RustSignal;
use subsonic::SubsonicProbe;
use uuid::Uuid;

use crate::backends::local::gui_request::{GuiRequest, listen_gui};
use crate::node::NodeContext;
use crate::request_error::ExpectedRequestError;
use crate::rpc::{
    RpcTarget, Slot, SubsonicTranslate, SubsonicTranslationFuture, TargetRequest,
    UnsupportedReason, narrow_slot,
};
use crate::server::server_request::serve_remote;
use crate::subsonic_binding::SubsonicBinding;
use crate::utils::{GlobalParams, RinfRustSignal};
use crate::{Session, TypedRequest};
use tokio_util::sync::CancellationToken;

use crate::messages::*;
define_request_types!();
