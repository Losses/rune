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

implement_rinf_rust_signal_trait!(VisualizerScenesChanged);

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
);

#[cfg(test)]
mod tests {
    use super::REMOTE_EVENT_DECLARATIONS;
    use std::collections::HashSet;

    /// A repeated declaration is silently dead, because lookup takes the first.
    #[test]
    fn every_remote_event_is_declared_once() {
        let mut seen = HashSet::new();
        let repeated: Vec<&str> = REMOTE_EVENT_DECLARATIONS
            .iter()
            .filter(|declaration| !seen.insert(declaration.type_id))
            .map(|declaration| declaration.name)
            .collect();
        assert!(repeated.is_empty(), "declared more than once: {repeated:?}");
    }
}
