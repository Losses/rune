use super::playback::CastLibraryState;
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

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListRenderersRequest {
    pub library_ids: Vec<String>,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererAvailability {
    Available,
    Offline,
    NotApproved,
    NoControl,
    CantPlay,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone)]
pub struct RendererCandidate {
    pub fingerprint: String,
    pub alias: String,
    pub attached: bool,
    pub availability: RendererAvailability,
    pub blocked_library_id: Option<String>,
    pub library_state: Option<CastLibraryState>,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct ListRenderersResponse {
    pub renderers: Vec<RendererCandidate>,
}

crate::implement_rinf_rust_signal_trait!(RendererAttachmentChanged);
