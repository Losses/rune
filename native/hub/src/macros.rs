use std::sync::Arc;

use anyhow::{Error, Result};
use requests::define_request_types;
use uuid::Uuid;

use crate::Session;
use crate::backends::local::gui_request::{GuiRequest, listen_gui};
use crate::node::NodeContext;
use crate::server::server_request::serve_remote;
use crate::utils::{GlobalParams, RinfRustSignal};
use tokio_util::sync::CancellationToken;

use crate::messages::*;
use crate::rpc::{Slot, TargetRequest, UnsupportedReason};

define_request_types!();
