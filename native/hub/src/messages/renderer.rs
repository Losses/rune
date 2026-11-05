use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, DartSignal)]
pub struct AttachRendererRequest {
    pub fingerprint: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct DetachRendererRequest {}

#[derive(Serialize, Deserialize, SignalPiece, Clone)]
pub struct AttachedRenderer {
    pub fingerprint: String,
    pub alias: String,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct RendererAttachmentChanged {
    pub renderer: Option<AttachedRenderer>,
}

crate::implement_rinf_rust_signal_trait!(RendererAttachmentChanged);
