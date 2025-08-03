use crate::implement_rinf_rust_signal_trait;
use crate::messages::*;
use rinf::RustSignal;

implement_rinf_rust_signal_trait!(ScanAudioLibraryProgress, ScanAudioLibraryResponse);
implement_rinf_rust_signal_trait!(SetMediaLibraryPathResponse);
implement_rinf_rust_signal_trait!(AnalyzeAudioLibraryProgress, AnalyzeAudioLibraryResponse);
implement_rinf_rust_signal_trait!(
    DeduplicateAudioLibraryProgress,
    DeduplicateAudioLibraryResponse
);
implement_rinf_rust_signal_trait!(
    PlaybackStatus,
    ScrobbleServiceStatusUpdated,
    CrashResponse,
    RealtimeFFT
);
implement_rinf_rust_signal_trait!(PlaylistUpdate);
implement_rinf_rust_signal_trait!(TrustListUpdated);
implement_rinf_rust_signal_trait!(IncommingClientPermissionNotification);
