use std::sync::Arc;

use anyhow::{Error, Result};
use requests::define_request_types;
use uuid::Uuid;

use crate::Session;
use crate::node::NodeContext;
use crate::server::server_request::serve_remote;

use crate::messages::*;
use crate::rpc::{Slot, TargetRequest, UnsupportedReason};

define_request_types!();
