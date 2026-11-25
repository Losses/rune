use proc_macro::TokenStream;
use quote::{format_ident, quote};
use std::collections::HashSet;
use syn::Data;
use syn::Error;

#[derive(Clone, Copy, PartialEq)]
enum RequestScope {
    Node,
    Session,
    Library,
}

#[derive(Clone, Copy, PartialEq)]
enum RequestAccess {
    NodeLocal,
    Control,
    ConnectionView,
    ConnectionEdit,
    Administer,
}

#[derive(Clone, Copy, PartialEq)]
enum BodyLibraries {
    NoLibrary,
    Authorized,
    PlayerReads,
}

#[derive(Clone, Copy, PartialEq)]
enum RequestReceiver {
    Library,
    NodeLoop,
    Standalone,
}

/// Which library a device answers a request by.
#[derive(Clone, Copy, PartialEq)]
enum RequestRouting {
    /// The library its UI has open.
    OpenLibrary,
    /// The library of the one item the request names, whichever library its UI has open.
    Item,
}

struct RequestResponse {
    request: String,
    response: Option<String>,
    scope: RequestScope,
    access: RequestAccess,
    body_libraries: BodyLibraries,
    introduced_minor: Option<u32>,
    receiver: RequestReceiver,
    routing: RequestRouting,
}

fn body_libraries_tokens(body_libraries: BodyLibraries) -> proc_macro2::TokenStream {
    match body_libraries {
        BodyLibraries::NoLibrary => quote! { BodyLibraries::NoLibrary },
        BodyLibraries::Authorized => quote! { BodyLibraries::Authorized },
        BodyLibraries::PlayerReads => quote! { BodyLibraries::PlayerReads },
    }
}

fn snake_case(value: &str) -> String {
    let mut name = String::new();
    for (index, ch) in value.chars().enumerate() {
        if ch.is_uppercase() && index != 0 {
            name.push('_');
        }
        name.extend(ch.to_lowercase());
    }
    name
}

#[proc_macro]
pub fn define_request_types(_input: TokenStream) -> TokenStream {
    let types = vec![
        RequestResponse {
            request: "AttachRendererRequest".to_string(),
            response: Some("AttachRendererResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "DetachRendererRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ListRenderersRequest".to_string(),
            response: Some("ListRenderersResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "HandoffRequest".to_string(),
            response: Some("HandoffResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "GetDeviceGrantsRequest".to_string(),
            response: Some("GetDeviceGrantsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SetDeviceGrantsRequest".to_string(),
            response: Some("SetDeviceGrantsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ListControllersRequest".to_string(),
            response: Some("ListControllersResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "DetachControllerRequest".to_string(),
            response: Some("DetachControllerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ListLocalLibrariesRequest".to_string(),
            response: Some("ListLocalLibrariesResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ListLibraryRecordsRequest".to_string(),
            response: Some("ListLibraryRecordsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RemoveLibraryRecordRequest".to_string(),
            response: Some("RemoveLibraryRecordResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SetLibraryBackgroundSyncRequest".to_string(),
            response: Some("SetLibraryBackgroundSyncResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RegisterLocalLibraryRequest".to_string(),
            response: Some("RegisterLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RegisterRemoteControlRequest".to_string(),
            response: Some("RegisterLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RegisterSubsonicLibraryRequest".to_string(),
            response: Some("RegisterLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RegisterOpenDalLibraryRequest".to_string(),
            response: Some("RegisterLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "DetectOpenDalShareDataRequest".to_string(),
            response: Some("DetectOpenDalShareDataResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "OpenLibraryRequest".to_string(),
            response: Some("OpenLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ReplaceSubsonicCredentialRequest".to_string(),
            response: Some("ReplaceSubsonicCredentialResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "InitializeNodeRequest".to_string(),
            response: Some("InitializeNodeResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Standalone,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RunFsSelfTestRequest".to_string(),
            response: Some("RunFsSelfTestResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Standalone,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ListFileDialogPlacesRequest".to_string(),
            response: Some("ListFileDialogPlacesResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Standalone,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ResolveFileDialogPathRequest".to_string(),
            response: Some("ResolveFileDialogPathResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Standalone,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ListFileDialogFolderRequest".to_string(),
            response: Some("ListFileDialogFolderResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Standalone,
            routing: RequestRouting::OpenLibrary,
        },
        // The reply carries pixels as binary, which the catalogue's
        // responses cannot, so the receiver sends it itself.
        RequestResponse {
            request: "FetchFileDialogThumbnailRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Standalone,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "CreateFileDialogFolderRequest".to_string(),
            response: Some("CreateFileDialogFolderResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Standalone,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RenameFileDialogEntryRequest".to_string(),
            response: Some("RenameFileDialogEntryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Standalone,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ListPeerLibrariesRequest".to_string(),
            response: Some("ListPeerLibrariesResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "AddRemoteLibraryRequest".to_string(),
            response: Some("AddRemoteLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        // Library
        RequestResponse {
            request: "GetSyncSettingsRequest".to_string(),
            response: Some("GetSyncSettingsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SetSyncSettingsRequest".to_string(),
            response: Some("SetSyncSettingsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SetDownloadRuleRequest".to_string(),
            response: Some("SetDownloadRuleResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RemoveDownloadRuleRequest".to_string(),
            response: Some("RemoveDownloadRuleResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "TestLibraryInitializedRequest".to_string(),
            response: Some("TestLibraryInitializedResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "CloseLibraryRequest".to_string(),
            response: Some("CloseLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "CancelTaskRequest".to_string(),
            response: Some("CancelTaskResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::Administer,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ScanAudioLibraryRequest".to_string(),
            response: None,
            scope: RequestScope::Library,
            access: RequestAccess::Administer,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "AnalyzeAudioLibraryRequest".to_string(),
            response: None,
            scope: RequestScope::Library,
            access: RequestAccess::Administer,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "DeduplicateAudioLibraryRequest".to_string(),
            response: None,
            scope: RequestScope::Library,
            access: RequestAccess::Administer,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        // Playback
        RequestResponse {
            request: "VolumeRequest".to_string(),
            response: Some("VolumeResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "LoadRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "PlayRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "PauseRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "PauseReportingRequest".to_string(),
            response: Some("PauseReportingResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: Some(1),
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "NextRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "PreviousRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SwitchRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SeekRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RemoveRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SetPlaybackModeRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "MovePlaylistItemRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SetRealtimeFFTEnabledRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SetAdaptiveSwitchingEnabledRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        // This device's own audio session, so never forwarded to a renderer.
        RequestResponse {
            request: "SetMixWithOthersRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        // Visualizer (the GPU and the frame buffer live in this process)
        RequestResponse {
            request: "StartVisualizerRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "StopVisualizerRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ResizeVisualizerRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SetVisualizerPresetDirsRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SetVisualizerAccentRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SetVisualizerVisibleRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "LoadVisualizerPresetRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "NextVisualizerPresetRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "PreviousVisualizerPresetRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RandomVisualizerPresetRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ImportVisualizerSceneRequest".to_string(),
            response: Some("ImportVisualizerSceneResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RemoveVisualizerSceneRequest".to_string(),
            response: Some("RemoveVisualizerSceneResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SetVisualizerScenesRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SetVisualizerLogRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SetVisualizerLabRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "VisualizerLabCommandRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "VisualizerLabRequest".to_string(),
            response: Some("VisualizerLabResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "VisualizerPlanRequest".to_string(),
            response: Some("VisualizerPlanResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "VisualizerSupportRequest".to_string(),
            response: Some("VisualizerSupportResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "VisualizerStatusRequest".to_string(),
            response: Some("VisualizerStatusResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ListVisualizerPresetsRequest".to_string(),
            response: Some("ListVisualizerPresetsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ListVisualizerCreditsRequest".to_string(),
            response: Some("ListVisualizerCreditsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "VisualizerTextureShownRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        // SFX
        RequestResponse {
            request: "SfxPlayRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        // Analyze
        RequestResponse {
            request: "IfAnalyzeExistsRequest".to_string(),
            response: Some("IfAnalyzeExistsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "GetAnalyzeCountRequest".to_string(),
            response: Some("GetAnalyzeCountResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        // Media File
        RequestResponse {
            request: "FetchMediaFilesRequest".to_string(),
            response: Some("FetchMediaFilesResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "FetchMediaFileByIdsRequest".to_string(),
            response: Some("FetchMediaFileByIdsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "FetchParsedMediaFileRequest".to_string(),
            response: Some("FetchParsedMediaFileResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SearchMediaFileSummaryRequest".to_string(),
            response: Some("SearchMediaFileSummaryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        // Lyric
        RequestResponse {
            request: "GetMediaFilesCountRequest".to_string(),
            response: Some("GetMediaFilesCountResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "GetLyricByTrackIdRequest".to_string(),
            response: Some("GetLyricByTrackIdResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::Authorized,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::Item,
        },
        // Collection
        RequestResponse {
            request: "FetchCollectionGroupSummaryRequest".to_string(),
            response: Some("CollectionGroupSummaryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "FetchCollectionGroupsRequest".to_string(),
            response: Some("FetchCollectionGroupsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "FetchCollectionByIdsRequest".to_string(),
            response: Some("FetchCollectionByIdsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SearchCollectionSummaryRequest".to_string(),
            response: Some("SearchCollectionSummaryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        // Cover Art
        RequestResponse {
            request: "GetCoverArtIdsByMixQueriesRequest".to_string(),
            response: Some("GetCoverArtIdsByMixQueriesResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "GetPrimaryColorByTrackIdRequest".to_string(),
            response: Some("GetPrimaryColorByTrackIdResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::Authorized,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::Item,
        },
        // Playlist
        RequestResponse {
            request: "FetchAllPlaylistsRequest".to_string(),
            response: Some("FetchAllPlaylistsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "CreatePlaylistRequest".to_string(),
            response: Some("CreatePlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "CreateM3u8PlaylistRequest".to_string(),
            response: Some("CreateM3u8PlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "UpdatePlaylistRequest".to_string(),
            response: Some("UpdatePlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RemovePlaylistRequest".to_string(),
            response: Some("RemovePlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "AddItemToPlaylistRequest".to_string(),
            response: Some("AddItemToPlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ReorderPlaylistItemPositionRequest".to_string(),
            response: Some("ReorderPlaylistItemPositionResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "FetchPlaylistOccurrencesRequest".to_string(),
            response: Some("FetchPlaylistOccurrencesResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "GetPlaylistByIdRequest".to_string(),
            response: Some("GetPlaylistByIdResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        // Mix
        RequestResponse {
            request: "FetchAllMixesRequest".to_string(),
            response: Some("FetchAllMixesResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "CreateMixRequest".to_string(),
            response: Some("CreateMixResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "UpdateMixRequest".to_string(),
            response: Some("UpdateMixResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RemoveMixRequest".to_string(),
            response: Some("RemoveMixResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "AddItemToMixRequest".to_string(),
            response: Some("AddItemToMixResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "GetMixByIdRequest".to_string(),
            response: Some("GetMixByIdResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "MixQueryRequest".to_string(),
            response: Some("MixQueryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "FetchMixQueriesRequest".to_string(),
            response: Some("FetchMixQueriesResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ResolveMixQueryRequest".to_string(),
            response: Some("ResolveMixQueryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::Authorized,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "CastResolvedItemsRequest".to_string(),
            response: Some("CastResolvedItemsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::PlayerReads,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RemotePlaylistRevisionRequest".to_string(),
            response: Some("RemotePlaylistRevisionResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "LoadResolvedItemsRequest".to_string(),
            response: Some("LoadResolvedItemsResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::PlayerReads,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "HandoffLoadRequest".to_string(),
            response: Some("HandoffLoadResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::PlayerReads,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "TakeBackHandoffBlockRequest".to_string(),
            response: Some("SessionSnapshotResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "CastPreflightRequest".to_string(),
            response: Some("CastPreflightResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RefreshSessionRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SessionSnapshotRequest".to_string(),
            response: Some("SessionSnapshotResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "PlaylistRevisionRequest".to_string(),
            response: Some("PlaylistRevisionResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        // Like
        RequestResponse {
            request: "SetLikedRequest".to_string(),
            response: Some("SetLikedResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            body_libraries: BodyLibraries::Authorized,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::Item,
        },
        RequestResponse {
            request: "GetLikedRequest".to_string(),
            response: Some("GetLikedResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::Authorized,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::Item,
        },
        // Query and Search
        RequestResponse {
            request: "ComplexQueryRequest".to_string(),
            response: Some("ComplexQueryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SearchForRequest".to_string(),
            response: Some("SearchForResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        // Artist
        RequestResponse {
            request: "FetchArtistPageRequest".to_string(),
            response: Some("FetchArtistPageResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: Some(1),
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "FetchArtistSectionAlbumsRequest".to_string(),
            response: Some("FetchArtistSectionAlbumsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: Some(1),
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        // Directory
        RequestResponse {
            request: "FetchDirectoryTreeRequest".to_string(),
            response: Some("FetchDirectoryTreeResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        // Scrobbler
        RequestResponse {
            request: "AuthenticateSingleServiceRequest".to_string(),
            response: Some("AuthenticateSingleServiceResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "AuthenticateMultipleServiceRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "LogoutSingleServiceRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        // Log
        RequestResponse {
            request: "ListLogRequest".to_string(),
            response: Some("ListLogResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ClearLogRequest".to_string(),
            response: Some("ClearLogResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RemoveLogRequest".to_string(),
            response: Some("RemoveLogResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        // System
        RequestResponse {
            request: "SystemInfoRequest".to_string(),
            response: Some("SystemInfoResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        // License
        RequestResponse {
            request: "RegisterLicenseRequest".to_string(),
            response: Some("RegisterLicenseResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ValidateLicenseRequest".to_string(),
            response: Some("ValidateLicenseResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        // Neighbors
        RequestResponse {
            request: "StartBroadcastRequest".to_string(),
            response: Some("StartBroadcastResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "StopBroadcastRequest".to_string(),
            response: Some("StopBroadcastResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "StartListeningRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "StopListeningRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "GetDiscoveredDeviceRequest".to_string(),
            response: Some("GetDiscoveredDeviceResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "StartServerRequest".to_string(),
            response: Some("StartServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "StopServerRequest".to_string(),
            response: Some("StopServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ListClientsRequest".to_string(),
            response: Some("ListClientsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ListPairedServersRequest".to_string(),
            response: Some("ListPairedServersResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "GetOpenLibraryRequest".to_string(),
            response: Some("GetOpenLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::Control,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "GetSslCertificateFingerprintRequest".to_string(),
            response: Some("GetSslCertificateFingerprintResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "AddTrustedServerRequest".to_string(),
            response: Some("AddTrustedServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RemoveTrustedClientRequest".to_string(),
            response: Some("RemoveTrustedClientResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "UpdateClientStatusRequest".to_string(),
            response: Some("UpdateClientStatusResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "EditHostsRequest".to_string(),
            response: Some("EditHostsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RemoveTrustedServerRequest".to_string(),
            response: Some("RemoveTrustedServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ServerAvailabilityTestRequest".to_string(),
            response: Some("ServerAvailabilityTestResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RegisterDeviceOnServerRequest".to_string(),
            response: Some("RegisterDeviceOnServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "CheckDeviceOnServerRequest".to_string(),
            response: Some("CheckDeviceOnServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "ConnectRequest".to_string(),
            response: Some("ConnectResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "FetchServerCertificateRequest".to_string(),
            response: Some("FetchServerCertificateResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "FetchRemoteFileRequest".to_string(),
            response: Some("FetchRemoteFileResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RemoveItemFromPlaylistRequest".to_string(),
            response: Some("RemoveItemFromPlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::Library,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "RegisterServiceLibraryRequest".to_string(),
            response: Some("RegisterLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SignInServiceLibraryRequest".to_string(),
            response: Some("SignInServiceLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SignOutServiceLibraryRequest".to_string(),
            response: Some("SignOutServiceLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
        RequestResponse {
            request: "SetOnlineLyricsRequest".to_string(),
            response: Some("SetOnlineLyricsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            body_libraries: BodyLibraries::NoLibrary,
            introduced_minor: None,
            receiver: RequestReceiver::NodeLoop,
            routing: RequestRouting::OpenLibrary,
        },
    ];

    let (with_response, without_response): (Vec<_>, Vec<_>) =
        types.iter().partition(|t| t.response.is_some());

    let mut all_responses: Vec<_> = with_response
        .iter()
        .map(|t| syn::parse_str::<syn::Ident>(t.response.as_ref().unwrap()).unwrap())
        .collect();
    let mut seen_responses = HashSet::new();
    all_responses.retain(|response| seen_responses.insert(response.to_string()));

    let all_requests: Vec<_> = with_response
        .iter()
        .map(|t| syn::parse_str::<syn::Ident>(&t.request).unwrap())
        .chain(
            without_response
                .iter()
                .map(|t| syn::parse_str::<syn::Ident>(&t.request).unwrap()),
        )
        .collect();
    let local_gui_requests: Vec<_> = all_requests
        .iter()
        .filter(|request| {
            types.iter().any(|entry| {
                *request == entry.request.as_str() && entry.receiver == RequestReceiver::Library
            })
        })
        .collect();

    let request_call_id_impls: Vec<_> = all_requests
        .iter()
        .map(|request| {
            quote! {
                impl crate::RequestCallId for crate::messages::#request {
                    fn call_id(&self) -> Option<u64> { self.call_id }
                }
            }
        })
        .collect();
    let response_call_id_impls: Vec<_> = all_responses
        .iter()
        .map(|response| {
            quote! {
                impl crate::ResponseCallId for crate::messages::#response {
                    fn set_call_id(&mut self, call_id: Option<u64>) { self.call_id = call_id; }
                }
            }
        })
        .collect();

    let typed_request_impls: Vec<_> = types
        .iter()
        .map(|entry| {
            let request = syn::parse_str::<syn::Ident>(&entry.request).unwrap();
            let response = entry
                .response
                .as_deref()
                .and_then(|name| syn::parse_str::<syn::Ident>(name).ok());
            let (response_type, response_name) = match response {
                Some(response) => (
                    quote! { crate::messages::#response },
                    quote! { stringify!(#response) },
                ),
                None => (quote! { () }, quote! { "" }),
            };
            quote! {
                impl crate::TypedRequest for crate::messages::#request {
                    type Response = #response_type;

                    fn wire_name() -> &'static str {
                        stringify!(#request)
                    }

                    fn response_name() -> &'static str {
                        #response_name
                    }
                }
            }
        })
        .collect();

    let gui_request_impls = types.iter().map(|entry| {
        let request = format_ident!("{}", entry.request);
        let emit_response = if entry.response.is_some() {
            quote! {
                if let Some(mut response) = response {
                    response.set_call_id(request.call_id());
                    emit(&response);
                }
            }
        } else {
            quote! { let _ = (request, response, emit); }
        };
        quote! {
            impl GuiRequest for #request {
                fn emit_response(
                    request: &Self,
                    response: Option<Self::Response>,
                    emit: &impl Fn(&dyn RinfRustSignal),
                ) {
                    #emit_response
                }
            }
        }
    });

    let slot_fields: Vec<_> = types
        .iter()
        .map(|entry| {
            let mut name = String::new();
            for (index, ch) in entry.request.chars().enumerate() {
                if ch.is_uppercase() && index != 0 {
                    name.push('_');
                }
                name.extend(ch.to_lowercase());
            }
            format_ident!("{}", name)
        })
        .collect();
    let target_request_impls = types.iter().zip(&slot_fields).map(|(entry, field)| {
        let request = format_ident!("{}", entry.request);
        let translate_method = format_ident!("translate_{}", snake_case(&entry.request));
        let scope = match entry.scope {
            RequestScope::Node => quote! { RequestScope::Node },
            RequestScope::Session => quote! { RequestScope::Session },
            RequestScope::Library => quote! { RequestScope::Library },
        };
        let introduced_minor = entry.introduced_minor.unwrap_or(0);
        let access = match entry.access {
            RequestAccess::NodeLocal => quote! { RequestAccess::NodeLocal },
            RequestAccess::Control => quote! { RequestAccess::Control },
            RequestAccess::ConnectionView => quote! { RequestAccess::ConnectionView },
            RequestAccess::ConnectionEdit => quote! { RequestAccess::ConnectionEdit },
            RequestAccess::Administer => quote! { RequestAccess::Administer },
        };
        let body_libraries = body_libraries_tokens(entry.body_libraries);
        let routed_by_item = entry.routing == RequestRouting::Item;
        // Only a request another device serves can be forwarded there.
        let forwarded_libraries = (entry.access != RequestAccess::NodeLocal).then(|| {
            quote! {
                fn visit_forwarded_libraries<'a>(&'a self, visit: &mut dyn FnMut(&'a str)) {
                    crate::rpc::NamedLibraries::visit_libraries(self, visit)
                }

                fn forwarded_lists_kept(&self, kept: &dyn Fn(&str) -> bool) -> bool {
                    crate::rpc::NamedLibraries::lists_kept(self, kept)
                }

                fn leave_kept_out_of_forwarded(&mut self, kept: &dyn Fn(&str) -> bool) {
                    crate::rpc::NamedLibraries::leave_out_kept(self, kept)
                }
            }
        });
        quote! {
            impl TargetRequest for #request {
                const SCOPE: RequestScope = #scope;
                const ACCESS: RequestAccess = #access;
                const BODY_LIBRARIES: BodyLibraries = #body_libraries;
                const INTRODUCED_MINOR: u32 = #introduced_minor;
                const ROUTED_BY_ITEM: bool = #routed_by_item;

                fn slot(slots: &RequestSlots) -> Slot {
                    slots.#field
                }

                #forwarded_libraries

                fn set_slot(slots: &mut RequestSlots, slot: Slot) {
                    slots.#field = slot;
                }

                fn translate<'a>(
                    &'a self,
                    translator: &'a dyn RequestTranslator,
                ) -> RequestTranslationFuture<'a, Self::Response> {
                    translator.#translate_method(self)
                }
            }
        }
    });
    let translator_trait_methods = types.iter().map(|entry| {
        let request = format_ident!("{}", entry.request);
        let method = format_ident!("translate_{}", snake_case(&entry.request));
        let response = entry.response.as_deref().map_or_else(
            || quote! { () },
            |name| {
                let response = format_ident!("{}", name);
                quote! { #response }
            },
        );
        quote! {
            fn #method<'a>(
                &'a self,
                    _request: &'a #request,
            ) -> RequestTranslationFuture<'a, #response> {
                Box::pin(async {
                    Err(ExpectedRequestError::Unsupported {
                            request: <#request as TypedRequest>::wire_name(),
                        reason: UnsupportedReason::NotImplemented,
                    }.into())
                })
            }
        }
    });
    let capability_request_idents: Vec<_> = types
        .iter()
        .map(|entry| format_ident!("{}", entry.request))
        .collect();
    let capability_slot_fields: Vec<_> = slot_fields.iter().collect();
    let capability_access_fields =
        types
            .iter()
            .zip(&capability_slot_fields)
            .map(|(entry, field)| {
                let access = match entry.access {
                    RequestAccess::NodeLocal => quote! { RequestAccess::NodeLocal },
                    RequestAccess::Control => quote! { RequestAccess::Control },
                    RequestAccess::ConnectionView => quote! { RequestAccess::ConnectionView },
                    RequestAccess::ConnectionEdit => quote! { RequestAccess::ConnectionEdit },
                    RequestAccess::Administer => quote! { RequestAccess::Administer },
                };
                quote! { visit(#access, &mut self.#field); }
            });
    let capability_snapshot_fields = types.iter().zip(&slot_fields).map(|(entry, field)| {
        let request = format_ident!("{}", entry.request);
        quote! { #field: node.request_slot::<#request>()?.into(), }
    });
    let item_capability_fields = types
        .iter()
        .zip(&slot_fields)
        .filter(|(entry, _)| entry.routing == RequestRouting::Item)
        .map(|(entry, field)| {
            let request = format_ident!("{}", entry.request);
            quote! { #field: <#request as TargetRequest>::slot(slots).into(), }
        });
    let scoped_slots = types
        .iter()
        .zip(&slot_fields)
        .filter(|(entry, _)| matches!(entry.scope, RequestScope::Session | RequestScope::Library))
        .map(|(entry, field)| {
            let name = &entry.request;
            quote! { (#name, self.#field) }
        });

    let non_local_requests: Vec<_> = types
        .iter()
        .filter(|t| t.access != RequestAccess::NodeLocal)
        .map(|t| syn::parse_str::<syn::Ident>(&t.request).unwrap())
        .collect();

    let accesses: Vec<_> = types
        .iter()
        .map(|t| {
            let request = &t.request;
            let access = match t.access {
                RequestAccess::NodeLocal => quote! { RequestAccess::NodeLocal },
                RequestAccess::Control => quote! { RequestAccess::Control },
                RequestAccess::ConnectionView => quote! { RequestAccess::ConnectionView },
                RequestAccess::ConnectionEdit => quote! { RequestAccess::ConnectionEdit },
                RequestAccess::Administer => quote! { RequestAccess::Administer },
            };
            quote! { #request => Some(#access), }
        })
        .collect();
    let introduced_minors: Vec<_> = types
        .iter()
        .map(|entry| entry.introduced_minor.unwrap_or(0))
        .collect();
    let scopes: Vec<_> = types
        .iter()
        .map(|t| {
            let request = &t.request;
            let scope = match t.scope {
                RequestScope::Node => quote! { RequestScope::Node },
                RequestScope::Session => quote! { RequestScope::Session },
                RequestScope::Library => quote! { RequestScope::Library },
            };
            quote! { #request => Some(#scope), }
        })
        .collect();
    let node_loop_requests: Vec<_> = types
        .iter()
        .filter(|entry| entry.receiver == RequestReceiver::NodeLoop)
        .map(|entry| {
            (
                syn::parse_str::<syn::Ident>(&entry.request).unwrap(),
                syn::parse_str::<syn::Type>(entry.response.as_deref().unwrap_or("()")).unwrap(),
                format_ident!("handle_{}", snake_case(&entry.request)),
                format_ident!("{}_receiver", snake_case(&entry.request)),
            )
        })
        .collect();
    let bootstrap_requests: Vec<_> = types
        .iter()
        .filter(|entry| entry.receiver == RequestReceiver::Standalone)
        .map(|entry| syn::parse_str::<syn::Ident>(&entry.request).unwrap())
        .collect();
    let node_handler_methods = node_loop_requests
        .iter()
        .map(|(request, response, method, _)| {
            quote! {
                fn #method(
                    &self,
                    node: &NodeSlot,
                    request: &#request,
                ) -> impl Future<Output = Result<Option<#response>>> + Send;
            }
        });
    let node_receivers = node_loop_requests.iter().map(|(request, _, _, receiver)| {
        quote! {
            let #receiver = #request::get_dart_signal_receiver();
        }
    });
    let node_receiver_params = node_loop_requests.iter().map(|(request, _, _, receiver)| {
        quote! {
            #receiver: SignalReceiver<DartSignalPack<#request>>,
        }
    });
    let node_loop_receiver_args: Vec<_> = node_loop_requests
        .iter()
        .map(|(_, _, _, receiver)| receiver)
        .collect();
    let node_receiver_tasks = node_loop_requests
        .iter()
        .map(|(request, _, method, receiver)| {
            quote! {
                let receiver = #receiver;
                request_tasks.push(Box::pin(async move {
                    let mut receiver = receiver;
                    while let Some(signal) = receiver.recv().await {
                        let request = signal.message;
                        let result = handlers.#method(node, &request).await;
                        answer_node_request::<#request>(&request, result, emit);
                    }
                }));
            }
        });
    let bootstrap_starts = bootstrap_requests.iter().map(|request| {
        quote! { <#request as StandaloneRequestReceiver>::start(); }
    });

    let expanded = quote! {
        #(#request_call_id_impls)*
        #(#response_call_id_impls)*
        #(#typed_request_impls)*
        #(#target_request_impls)*
        pub trait RequestTranslator: Send + Sync {
            #(#translator_trait_methods)*
        }
        #(#gui_request_impls)*

        pub(crate) trait NodeRequestHandlers: Send + Sync {
            #(#node_handler_methods)*
        }

        pub(crate) trait StandaloneRequestReceiver {
            fn start();
        }

        struct NodeRequestReceivers {
            #(#node_receiver_params)*
        }

        pub(crate) fn start_standalone_request_receivers() {
            #(#bootstrap_starts)*
        }

        pub(crate) async fn receive_node_requests<H>(
            node: &NodeSlot,
            handlers: H,
        )
        where
            H: NodeRequestHandlers,
        {
            #(#node_receivers)*
            let emit = |signal: &dyn RinfRustSignal| signal.send_to_dart();
            node_request_loop_with_receivers(
                node,
                handlers,
                emit,
                NodeRequestReceivers {
                    #(#node_loop_receiver_args,)*
                },
            ).await
        }

        async fn node_request_loop_with_receivers<H, E>(
            node: &NodeSlot,
            handlers: H,
            emit: E,
            receivers: NodeRequestReceivers,
        )
        where
            H: NodeRequestHandlers,
            E: Fn(&dyn RinfRustSignal) + Sync,
        {
            let NodeRequestReceivers {
                #(#node_loop_receiver_args,)*
            } = receivers;
            let handlers = &handlers;
            let emit = &emit;
            let mut request_tasks: FuturesUnordered<BoxFuture<'_, ()>> =
                FuturesUnordered::new();
            #(#node_receiver_tasks)*
            loop {
                if request_tasks.next().await.is_none() {
                    sleep(Duration::from_secs(1)).await;
                }
            }
        }

        /// Answer one request outside the GUI listeners: its reply or its typed failure,
        /// tagged with its call id.
        pub(crate) fn answer_node_request<T>(
            request: &T,
            result: Result<Option<T::Response>>,
            emit: &impl Fn(&dyn RinfRustSignal),
        )
        where
            T: GuiRequest + TypedRequest + crate::RequestCallId,
        {
            match result {
                Ok(response) => T::emit_response(request, response, emit),
                Err(error) => {
                    log::warn!("Request {} failed: {error:#}", T::wire_name());
                    emit(&RequestFailedResponse::for_request(
                        request,
                        T::wire_name(),
                        classify(&error),
                        format!("{error:#}"),
                        request_failure_cause(&error),
                    ));
                }
            }
        }

        /// Start one sequential GUI listener per catalogue request type.
        pub(crate) fn listen_local_gui(params: &Arc<GlobalParams>, cancel: &CancellationToken) {
            #(tokio::spawn(listen_gui::<#local_gui_requests>(params.clone(), cancel.clone()));)*
        }


        /// Dispatch a network request through the catalogue's non-local entries.
        pub(crate) async fn dispatch_remote(
            name: &str,
            payload: &[u8],
            session: Option<Session>,
            library_id: Option<Uuid>,
            node: &Arc<NodeContext>,
        ) -> Option<(String, Result<Vec<u8>>)> {
            let result = match name {
                #(stringify!(#non_local_requests) => serve_remote::<#non_local_requests>(
                    || rinf::deserialize::<#non_local_requests>(payload).map_err(Error::new),
                    session,
                    library_id,
                    node,
                ).await,)*
                _ => return None,
            };
            Some(match result {
                Ok((name, payload)) => (name, Ok(payload)),
                Err(error) => (String::new(), Err(error)),
            })
        }

        /// Catalogue-generated slots shared by every RPC target.
        #[derive(Clone, Copy)]
        pub struct RequestSlots {
            #(pub(crate) #slot_fields: Slot,)*
        }

        impl RequestSlots {
            pub fn unimplemented() -> Self {
                Self { #(#slot_fields: Slot::Unsupported { reason: UnsupportedReason::NotImplemented },)* }
            }

            pub fn local() -> Self {
                let mut slots = Self::unimplemented();
                #(slots.#slot_fields = Slot::Native;)*
                slots
            }

            pub fn rune() -> Self {
                let mut slots = Self::unimplemented();
                #(slots.#slot_fields = Slot::Forward;)*
                slots
            }

            pub(crate) fn set<R: TargetRequest>(&mut self, slot: Slot) {
                R::set_slot(self, slot);
            }

            /// Mark every request newer than the peer's protocol minor unsupported.
            pub fn narrow_to_minor(&mut self, peer_minor: u32) {
                #(self.#slot_fields = narrow_slot(self.#slot_fields, #introduced_minors, peer_minor);)*
            }

            #[cfg(test)]
            pub(crate) fn scoped_slots(&self) -> Vec<(&'static str, Slot)> {
                vec![#(#scoped_slots),*]
            }
        }

        pub(crate) fn capability_snapshot(node: &NodeContext) -> Result<RequestCapabilities> {
            let (record_id, generation, library_id) = node.capability_library_identity()?;
            Ok(RequestCapabilities { record_id, generation, library_id: library_id.map(|id| id.to_string()), #(#capability_snapshot_fields)* })
        }

        /// What `item` supports of the requests routed by item, read from `slots`: the item
        /// slots of the implementation of the library it belongs to.
        pub(crate) fn item_capability_snapshot(item: Option<PlayingItemRef>, slots: &RequestSlots) -> PlayingItemCapabilities {
            PlayingItemCapabilities { item, #(#item_capability_fields)* }
        }

        impl RequestCapabilities {
            pub(crate) fn for_each_capability_mut(
                &mut self,
                mut visit: impl FnMut(RequestAccess, &mut RequestCapability),
            ) {
                #(#capability_access_fields)*
            }
        }

        #[cfg(test)]
        impl RequestCapabilities {
            pub(crate) fn all_available() -> Self {
                Self { record_id: None, generation: 0, library_id: None, #(#capability_slot_fields: RequestCapability::Available,)* }
            }

            pub(crate) fn from_implementation(implementation: &Implementation) -> Self {
                let slots = implementation.slots();
                Self { record_id: None, generation: 0, library_id: None, #(#capability_slot_fields: RequestCapability::from(<#capability_request_idents as TargetRequest>::slot(&slots)),)* }
            }
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum RequestAccess { NodeLocal, Control, ConnectionView, ConnectionEdit, Administer }
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum RequestScope { Node, Session, Library }
        /// What the libraries named in a request's body mean for a call from another device.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum BodyLibraries {
            /// The body names no library.
            NoLibrary,
            /// The request acts on every library its body names: each one is authorized with
            /// the request's access rule and must be the library the frame names.
            Authorized,
            /// The body names items for this device's player, which reads them under its own
            /// standing grants on their libraries; each named library is also checked for the
            /// calling session before the player reads it.
            PlayerReads,
        }
        pub fn request_access(name: &str) -> Option<RequestAccess> {
            match name { #(#accesses)* _ => None }
        }
        pub fn request_scope(name: &str) -> Option<RequestScope> {
            match name { #(#scopes)* _ => None }
        }

        #[cfg(test)]
        pub(crate) fn standalone_request_names() -> &'static [&'static str] {
            &[#(stringify!(#bootstrap_requests)),*]
        }


        implement_rinf_rust_signal_trait!(#(#all_responses),*);

        #[cfg(test)]
        pub(crate) fn non_local_request_names() -> &'static [&'static str] {
            &[#(stringify!(#non_local_requests)),*]
        }

        /// Each non-local request's declared body rule, and whether its body type names a library.
        #[cfg(test)]
        pub(crate) fn body_library_declarations() -> &'static [(&'static str, BodyLibraries, bool)] {
            &[#((
                stringify!(#non_local_requests),
                <#non_local_requests as TargetRequest>::BODY_LIBRARIES,
                <#non_local_requests as crate::rpc::NamedLibraries>::NAMES_LIBRARY,
            )),*]
        }

        #[macro_export]
        macro_rules! for_all_requests0 {
            ($m:tt) => {
                $m!(#(#all_requests),*);
            }
        }




    };

    TokenStream::from(expanded)
}

/// Derive `NamedLibraries` from a message's fields: the message names a library when one
/// of its fields does, and visiting it visits every field. The items it lists are those its
/// fields list.
#[proc_macro_derive(NamedLibraries)]
pub fn derive_named_libraries(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    let name = &input.ident;
    let named_libraries = quote! { crate::rpc::NamedLibraries };
    // What each field, or each variant's fields, does for `visit`, `lists_kept` and
    // `leave_out_kept`.
    let each_field =
        |mutable: bool, call: &dyn Fn(&proc_macro2::TokenStream) -> proc_macro2::TokenStream| {
            match &input.data {
                Data::Struct(data) => {
                    let calls = data.fields.members().map(|member| {
                        call(&if mutable {
                            quote! { &mut self.#member }
                        } else {
                            quote! { &self.#member }
                        })
                    });
                    quote! { #(#calls)* }
                }
                Data::Enum(data) => {
                    let arms = data.variants.iter().map(|variant| {
                        let ident = &variant.ident;
                        let members = variant.fields.members();
                        let bindings: Vec<_> = (0..variant.fields.len())
                            .map(|index| format_ident!("field_{index}"))
                            .collect();
                        let calls = bindings.iter().map(|binding| call(&quote! { #binding }));
                        quote! { Self::#ident { #(#members: #bindings),* } => { #(#calls)* } }
                    });
                    quote! { match self { #(#arms)* } }
                }
                Data::Union(_) => quote! {},
            }
        };
    let visit = each_field(false, &|field| {
        quote! { #named_libraries::visit_libraries(#field, visit); }
    });
    let lists_kept = each_field(false, &|field| {
        quote! { if #named_libraries::lists_kept(#field, kept) { return true; } }
    });
    let leave_out_kept = each_field(true, &|field| {
        quote! { #named_libraries::leave_out_kept(#field, kept); }
    });
    let field_types = match &input.data {
        Data::Struct(data) => data
            .fields
            .iter()
            .map(|field| &field.ty)
            .collect::<Vec<_>>(),
        Data::Enum(data) => data
            .variants
            .iter()
            .flat_map(|variant| variant.fields.iter().map(|field| &field.ty))
            .collect(),
        Data::Union(_) => {
            return Error::new_spanned(name, "NamedLibraries cannot be derived for a union")
                .to_compile_error()
                .into();
        }
    };
    let mut seen = HashSet::new();
    let field_types: Vec<_> = field_types
        .into_iter()
        .filter(|ty| seen.insert(quote! { #ty }.to_string()))
        .collect();
    let names_library = match field_types.split_first() {
        None => quote! { false },
        Some((first, rest)) => quote! {
            <#first as #named_libraries>::NAMES_LIBRARY
                #(|| <#rest as #named_libraries>::NAMES_LIBRARY)*
        },
    };
    let unused_visit = seen.is_empty().then(|| quote! { let _ = visit; });
    let unused_kept = seen.is_empty().then(|| quote! { let _ = kept; });
    quote! {
        impl #named_libraries for #name {
            const NAMES_LIBRARY: bool = #names_library;

            fn visit_libraries<'a>(&'a self, visit: &mut dyn FnMut(&'a str)) {
                #unused_visit
                #visit
            }

            fn lists_kept(&self, kept: &dyn Fn(&str) -> bool) -> bool {
                #unused_kept
                #lists_kept
                false
            }

            fn leave_out_kept(&mut self, kept: &dyn Fn(&str) -> bool) {
                #unused_kept
                #leave_out_kept
            }
        }
    }
    .into()
}
