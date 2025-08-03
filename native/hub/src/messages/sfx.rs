use rinf::DartSignal;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SfxPlayRequest {
    pub path: String,
}
