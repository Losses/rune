use prost::Message;

use crate::broadcastable;
use crate::messages::*;
use crate::utils::RinfRustSignal;

broadcastable!(ScanAudioLibraryProgress, ScanAudioLibraryResponse);
broadcastable!(SetMediaLibraryPathResponse);
broadcastable!(AnalyzeAudioLibraryProgress, AnalyzeAudioLibraryResponse);
broadcastable!(
    PlaybackStatus,
    ScrobbleServiceStatusUpdated,
    CrashResponse,
    RealtimeFft
);
broadcastable!(PlaylistUpdate);
broadcastable!(TrustListUpdated);
broadcastable!(IncommingClientPermissionNotification);
