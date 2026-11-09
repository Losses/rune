use crate::messages::*;
use rinf::RustSignal;
use std::any::TypeId;

implement_rinf_rust_signal_trait!(ControllersUpdated);
implement_rinf_rust_signal_trait!(
    DeduplicateAudioLibraryProgress,
    DeduplicateAudioLibraryResponse
);
implement_rinf_rust_signal_trait!(
    PlaybackStatus,
    PlaylistUpdate,
    RealtimeFFT,
    LibraryContentChanged,
    ScanAudioLibraryProgress,
    ScanAudioLibraryResponse,
    AnalyzeAudioLibraryProgress,
    AnalyzeAudioLibraryResponse,
    ScrobbleServiceStatusUpdated,
    CrashResponse
);
implement_rinf_rust_signal_trait!(TrustListUpdated);
implement_rinf_rust_signal_trait!(IncommingClientPermissionNotification);

implement_rinf_rust_signal_trait!(TrustedClientsChanged);

implement_rinf_rust_signal_trait!(RequestFailedResponse);

crate::declare_remote_events!(
    PlaybackStatus => Session,
    PlaylistUpdate => Session,
    RealtimeFFT => Session,
    LibraryContentChanged => Library,
    ScanAudioLibraryProgress => Library,
    ScanAudioLibraryResponse => Library,
    AnalyzeAudioLibraryProgress => Library,
    AnalyzeAudioLibraryResponse => Library,
);
