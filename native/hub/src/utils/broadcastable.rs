use crate::messages::*;
use rinf::RustSignal;
use std::any::TypeId;

implement_rinf_rust_signal_trait!(ControllersUpdated);

implement_rinf_rust_signal_trait!(
    PlaybackStatus,
    PlaylistUpdate,
    RealtimeFFT,
    VisualizerFeedChart,
    VisualizerFeedCovers,
    LibraryContentChanged,
    LibraryTaskStateChanged,
    ScrobbleServiceStatusUpdated,
    CrashResponse
);
implement_rinf_rust_signal_trait!(TrustListUpdated);
implement_rinf_rust_signal_trait!(IncommingClientPermissionNotification);

implement_rinf_rust_signal_trait!(TrustedClientsChanged);

implement_rinf_rust_signal_trait!(RequestFailedResponse);

crate::declare_remote_events!(
    RequestCapabilities => Library,
    PlaybackStatus => Session,
    PlaylistUpdate => Session,
    RealtimeFFT => Session,
    VisualizerFeedChart => Session,
    VisualizerFeedCovers => Session,
    LibraryContentChanged => Library,
    LibraryTaskStateChanged => Library,
    RequestCapabilities => Session,
);
