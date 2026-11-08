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

use std::sync::Arc;

use anyhow::{Error, Result};
use requests::define_request_types;
use rinf::RustSignal;
use uuid::Uuid;

use crate::Session;
use crate::backends::local::gui_request::{GuiRequest, listen_gui};
use crate::node::NodeContext;
use crate::rpc::RpcTarget;
use crate::server::server_request::serve_remote;
use crate::utils::{GlobalParams, RinfRustSignal};
use tokio_util::sync::CancellationToken;

use crate::messages::*;
use crate::rpc::{Slot, TargetRequest, UnsupportedReason};

define_request_types!();
