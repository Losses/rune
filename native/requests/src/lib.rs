use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};

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
enum NodeReceiver {
    Loop,
    BootstrapNode,
    BootstrapSubsystem,
}

struct RequestResponse {
    request: String,
    response: Option<String>,
    scope: RequestScope,
    access: RequestAccess,
    call_id: bool,
    introduced_minor: Option<u32>,
    // Listened to for the node's whole life rather than only while a UI library is open.
    node_lifetime: bool,
    // Selects the generated node loop or one of the independently started bootstrap paths.
    node_receiver: Option<NodeReceiver>,
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
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "DetachRendererRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ListRenderersRequest".to_string(),
            response: Some("ListRenderersResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "HandoffRequest".to_string(),
            response: Some("HandoffResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "GetDeviceGrantsRequest".to_string(),
            response: Some("GetDeviceGrantsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SetDeviceGrantsRequest".to_string(),
            response: Some("SetDeviceGrantsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ListControllersRequest".to_string(),
            response: Some("ListControllersResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "DetachControllerRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ListLocalLibrariesRequest".to_string(),
            response: Some("ListLocalLibrariesResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ListLibraryRecordsRequest".to_string(),
            response: Some("ListLibraryRecordsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: true,
            node_receiver: Some(NodeReceiver::Loop),
        },
        RequestResponse {
            request: "RemoveLibraryRecordRequest".to_string(),
            response: Some("RemoveLibraryRecordResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: true,
            node_receiver: Some(NodeReceiver::Loop),
        },
        RequestResponse {
            request: "SetLibraryBackgroundSyncRequest".to_string(),
            response: Some("SetLibraryBackgroundSyncResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "RegisterLocalLibraryRequest".to_string(),
            response: Some("RegisterLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: true,
            node_receiver: Some(NodeReceiver::Loop),
        },
        RequestResponse {
            request: "RegisterRemoteControlRequest".to_string(),
            response: Some("RegisterLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: true,
            node_receiver: Some(NodeReceiver::Loop),
        },
        RequestResponse {
            request: "RegisterSubsonicLibraryRequest".to_string(),
            response: Some("RegisterLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: true,
            node_receiver: Some(NodeReceiver::Loop),
        },
        RequestResponse {
            request: "RegisterOpenDalLibraryRequest".to_string(),
            response: Some("RegisterLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: true,
            node_receiver: Some(NodeReceiver::Loop),
        },
        RequestResponse {
            request: "OpenLibraryRequest".to_string(),
            response: Some("OpenLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: true,
            introduced_minor: None,
            node_lifetime: true,
            node_receiver: Some(NodeReceiver::Loop),
        },
        RequestResponse {
            request: "InitializeNodeRequest".to_string(),
            response: Some("InitializeNodeResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: true,
            node_receiver: Some(NodeReceiver::BootstrapNode),
        },
        RequestResponse {
            request: "RunFsSelfTestRequest".to_string(),
            response: Some("RunFsSelfTestResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: true,
            node_receiver: Some(NodeReceiver::BootstrapSubsystem),
        },
        RequestResponse {
            request: "ListPeerLibrariesRequest".to_string(),
            response: Some("ListPeerLibrariesResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "AddRemoteLibraryRequest".to_string(),
            response: Some("AddRemoteLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Library
        RequestResponse {
            request: "GetSyncSettingsRequest".to_string(),
            response: Some("GetSyncSettingsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SetSyncSettingsRequest".to_string(),
            response: Some("SetSyncSettingsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SetDownloadRuleRequest".to_string(),
            response: Some("SetDownloadRuleResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "RemoveDownloadRuleRequest".to_string(),
            response: Some("RemoveDownloadRuleResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "TestLibraryInitializedRequest".to_string(),
            response: Some("TestLibraryInitializedResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "CloseLibraryRequest".to_string(),
            response: Some("CloseLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "CancelTaskRequest".to_string(),
            response: Some("CancelTaskResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::Administer,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ScanAudioLibraryRequest".to_string(),
            response: None,
            scope: RequestScope::Library,
            access: RequestAccess::Administer,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "AnalyzeAudioLibraryRequest".to_string(),
            response: None,
            scope: RequestScope::Library,
            access: RequestAccess::Administer,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "DeduplicateAudioLibraryRequest".to_string(),
            response: None,
            scope: RequestScope::Library,
            access: RequestAccess::Administer,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Playback
        RequestResponse {
            request: "VolumeRequest".to_string(),
            response: Some("VolumeResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "LoadRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "PlayRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "PauseRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "NextRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "PreviousRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SwitchRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SeekRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "RemoveRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SetPlaybackModeRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "MovePlaylistItemRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SetRealtimeFFTEnabledRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SetAdaptiveSwitchingEnabledRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // This device's own audio session, so never forwarded to a renderer.
        RequestResponse {
            request: "SetMixWithOthersRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Visualizer (the GPU and the frame buffer live in this process)
        RequestResponse {
            request: "StartVisualizerRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "StopVisualizerRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ResizeVisualizerRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SetVisualizerPresetDirsRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SetVisualizerAccentRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SetVisualizerVisibleRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "LoadVisualizerPresetRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "NextVisualizerPresetRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "PreviousVisualizerPresetRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "RandomVisualizerPresetRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ImportVisualizerSceneRequest".to_string(),
            response: Some("ImportVisualizerSceneResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "RemoveVisualizerSceneRequest".to_string(),
            response: Some("RemoveVisualizerSceneResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SetVisualizerScenesRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SetVisualizerLogRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SetVisualizerLabRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "VisualizerLabCommandRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "VisualizerLabRequest".to_string(),
            response: Some("VisualizerLabResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "VisualizerPlanRequest".to_string(),
            response: Some("VisualizerPlanResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "VisualizerSupportRequest".to_string(),
            response: Some("VisualizerSupportResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "VisualizerStatusRequest".to_string(),
            response: Some("VisualizerStatusResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ListVisualizerPresetsRequest".to_string(),
            response: Some("ListVisualizerPresetsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ListVisualizerCreditsRequest".to_string(),
            response: Some("ListVisualizerCreditsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "VisualizerTextureShownRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // SFX
        RequestResponse {
            request: "SfxPlayRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Analyze
        RequestResponse {
            request: "IfAnalyzeExistsRequest".to_string(),
            response: Some("IfAnalyzeExistsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "GetAnalyzeCountRequest".to_string(),
            response: Some("GetAnalyzeCountResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Media File
        RequestResponse {
            request: "FetchMediaFilesRequest".to_string(),
            response: Some("FetchMediaFilesResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "FetchMediaFileByIdsRequest".to_string(),
            response: Some("FetchMediaFileByIdsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "FetchParsedMediaFileRequest".to_string(),
            response: Some("FetchParsedMediaFileResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SearchMediaFileSummaryRequest".to_string(),
            response: Some("SearchMediaFileSummaryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Lyric
        RequestResponse {
            request: "GetMediaFilesCountRequest".to_string(),
            response: Some("GetMediaFilesCountResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "GetLyricByTrackIdRequest".to_string(),
            response: Some("GetLyricByTrackIdResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Collection
        RequestResponse {
            request: "FetchCollectionGroupSummaryRequest".to_string(),
            response: Some("CollectionGroupSummaryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "FetchCollectionGroupsRequest".to_string(),
            response: Some("FetchCollectionGroupsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "FetchCollectionByIdsRequest".to_string(),
            response: Some("FetchCollectionByIdsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SearchCollectionSummaryRequest".to_string(),
            response: Some("SearchCollectionSummaryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Cover Art
        RequestResponse {
            request: "GetCoverArtIdsByMixQueriesRequest".to_string(),
            response: Some("GetCoverArtIdsByMixQueriesResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "GetPrimaryColorByTrackIdRequest".to_string(),
            response: Some("GetPrimaryColorByTrackIdResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Playlist
        RequestResponse {
            request: "FetchAllPlaylistsRequest".to_string(),
            response: Some("FetchAllPlaylistsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "CreatePlaylistRequest".to_string(),
            response: Some("CreatePlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "CreateM3u8PlaylistRequest".to_string(),
            response: Some("CreateM3u8PlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "UpdatePlaylistRequest".to_string(),
            response: Some("UpdatePlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "RemovePlaylistRequest".to_string(),
            response: Some("RemovePlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "AddItemToPlaylistRequest".to_string(),
            response: Some("AddItemToPlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ReorderPlaylistItemPositionRequest".to_string(),
            response: Some("ReorderPlaylistItemPositionResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "FetchPlaylistOccurrencesRequest".to_string(),
            response: Some("FetchPlaylistOccurrencesResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "GetPlaylistByIdRequest".to_string(),
            response: Some("GetPlaylistByIdResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Mix
        RequestResponse {
            request: "FetchAllMixesRequest".to_string(),
            response: Some("FetchAllMixesResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "CreateMixRequest".to_string(),
            response: Some("CreateMixResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "UpdateMixRequest".to_string(),
            response: Some("UpdateMixResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "RemoveMixRequest".to_string(),
            response: Some("RemoveMixResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "AddItemToMixRequest".to_string(),
            response: Some("AddItemToMixResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "GetMixByIdRequest".to_string(),
            response: Some("GetMixByIdResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "MixQueryRequest".to_string(),
            response: Some("MixQueryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "FetchMixQueriesRequest".to_string(),
            response: Some("FetchMixQueriesResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ResolveMixQueryRequest".to_string(),
            response: Some("ResolveMixQueryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "LoadResolvedItemsRequest".to_string(),
            response: Some("LoadResolvedItemsResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "CastResolvedItemsRequest".to_string(),
            response: Some("CastResolvedItemsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "CastPreflightRequest".to_string(),
            response: Some("CastPreflightResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "CastLoadRequest".to_string(),
            response: Some("CastLoadResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "RefreshSessionRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SessionSnapshotRequest".to_string(),
            response: Some("SessionSnapshotResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Like
        RequestResponse {
            request: "SetLikedRequest".to_string(),
            response: Some("SetLikedResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "GetLikedRequest".to_string(),
            response: Some("GetLikedResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Query and Search
        RequestResponse {
            request: "ComplexQueryRequest".to_string(),
            response: Some("ComplexQueryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "SearchForRequest".to_string(),
            response: Some("SearchForResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: true,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Directory
        RequestResponse {
            request: "FetchDirectoryTreeRequest".to_string(),
            response: Some("FetchDirectoryTreeResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Scrobbler
        RequestResponse {
            request: "AuthenticateSingleServiceRequest".to_string(),
            response: Some("AuthenticateSingleServiceResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "AuthenticateMultipleServiceRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "LogoutSingleServiceRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Log
        RequestResponse {
            request: "ListLogRequest".to_string(),
            response: Some("ListLogResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ClearLogRequest".to_string(),
            response: Some("ClearLogResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "RemoveLogRequest".to_string(),
            response: Some("RemoveLogResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // System
        RequestResponse {
            request: "SystemInfoRequest".to_string(),
            response: Some("SystemInfoResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // License
        RequestResponse {
            request: "RegisterLicenseRequest".to_string(),
            response: Some("RegisterLicenseResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ValidateLicenseRequest".to_string(),
            response: Some("ValidateLicenseResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        // Neighbors
        RequestResponse {
            request: "StartBroadcastRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "StopBroadcastRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "StartListeningRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "StopListeningRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "GetDiscoveredDeviceRequest".to_string(),
            response: Some("GetDiscoveredDeviceResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "StartServerRequest".to_string(),
            response: Some("StartServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "StopServerRequest".to_string(),
            response: Some("StopServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ListClientsRequest".to_string(),
            response: Some("ListClientsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "GetSslCertificateFingerprintRequest".to_string(),
            response: Some("GetSslCertificateFingerprintResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "AddTrustedServerRequest".to_string(),
            response: Some("AddTrustedServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "RemoveTrustedClientRequest".to_string(),
            response: Some("RemoveTrustedClientResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "UpdateClientStatusRequest".to_string(),
            response: Some("UpdateClientStatusResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "EditHostsRequest".to_string(),
            response: Some("EditHostsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "RemoveTrustedServerRequest".to_string(),
            response: Some("RemoveTrustedServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ServerAvailabilityTestRequest".to_string(),
            response: Some("ServerAvailabilityTestResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "RegisterDeviceOnServerRequest".to_string(),
            response: Some("RegisterDeviceOnServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "CheckDeviceOnServerRequest".to_string(),
            response: Some("CheckDeviceOnServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "ConnectRequest".to_string(),
            response: Some("ConnectResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "FetchServerCertificateRequest".to_string(),
            response: Some("FetchServerCertificateResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "FetchRemoteFileRequest".to_string(),
            response: Some("FetchRemoteFileResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
        RequestResponse {
            request: "RemoveItemFromPlaylistRequest".to_string(),
            response: Some("RemoveItemFromPlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            introduced_minor: None,
            node_lifetime: false,
            node_receiver: None,
        },
    ];

    let (with_response, without_response): (Vec<_>, Vec<_>) =
        types.iter().partition(|t| t.response.is_some());

    if types
        .iter()
        .any(|entry| entry.node_lifetime != entry.node_receiver.is_some())
    {
        return syn::Error::new(
            Span::call_site(),
            "node-lifetime requests must declare exactly one receiver path",
        )
        .to_compile_error()
        .into();
    }

    let mut all_responses: Vec<_> = with_response
        .iter()
        .map(|t| syn::parse_str::<syn::Ident>(t.response.as_ref().unwrap()).unwrap())
        .collect();
    all_responses.dedup();

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
            types
                .iter()
                .any(|entry| *request == entry.request.as_str() && !entry.node_lifetime)
        })
        .collect();

    let call_id_impls: Vec<_> = all_requests
        .iter()
        .map(|request| {
            let call_id = types
                .iter()
                .any(|entry| *request == entry.request && entry.call_id);
            let method = if call_id {
                quote! { fn call_id(&self) -> Option<u64> { Some(self.call_id) } }
            } else {
                quote! {}
            };
            quote! {
                impl crate::CallIdentified for crate::messages::#request {
                    #method
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
        let emit = if entry.response.is_some() {
            quote! { if let Some(response) = _response { _emit(&response); } }
        } else {
            quote! {}
        };
        quote! {
            impl GuiRequest for #request {
                fn emit_response(
                    _response: Option<Self::Response>,
                    _emit: &impl Fn(&dyn RinfRustSignal),
                ) {
                    #emit
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
        quote! {
            impl TargetRequest for #request {
                const SCOPE: RequestScope = #scope;
                const ACCESS: RequestAccess = #access;
                const INTRODUCED_MINOR: u32 = #introduced_minor;

                fn slot(slots: &RequestSlots) -> Slot {
                    slots.#field
                }

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
    let capability_snapshot_fields = types.iter().zip(&slot_fields).map(|(entry, field)| {
        let request = format_ident!("{}", entry.request);
        quote! { #field: node.request_slot::<#request>()?.into(), }
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
        .filter(|entry| entry.node_receiver == Some(NodeReceiver::Loop))
        .map(|entry| {
            (
                syn::parse_str::<syn::Ident>(&entry.request).unwrap(),
                syn::parse_str::<syn::Ident>(entry.response.as_deref().unwrap()).unwrap(),
                format_ident!("handle_{}", snake_case(&entry.request)),
                format_ident!("{}_receiver", snake_case(&entry.request)),
            )
        })
        .collect();
    let bootstrap_requests: Vec<_> = [
        NodeReceiver::BootstrapSubsystem,
        NodeReceiver::BootstrapNode,
    ]
    .into_iter()
    .flat_map(|kind| {
        types
            .iter()
            .filter(move |entry| entry.node_receiver == Some(kind))
            .map(|entry| syn::parse_str::<syn::Ident>(&entry.request).unwrap())
    })
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
    let node_receiver_branches = node_loop_requests
        .iter()
        .map(|(request, _, method, receiver)| {
            quote! {
                Some(signal) = #receiver.recv() => {
                    let request = signal.message;
                    let result = handlers.#method(node, &request).await;
                    answer_node_request::<#request>(&request, result, &emit);
                }
            }
        });
    let bootstrap_starts = bootstrap_requests.iter().map(|request| {
        quote! { <#request as BootstrapRequestReceiver>::start(); }
    });

    let expanded = quote! {
        #(#call_id_impls)*
        #(#typed_request_impls)*
        #(#target_request_impls)*
        pub trait RequestTranslator: Send + Sync {
            #(#translator_trait_methods)*
        }
        #(#gui_request_impls)*

        pub(crate) trait NodeRequestHandlers: Send + Sync {
            #(#node_handler_methods)*
        }

        pub(crate) trait BootstrapRequestReceiver {
            fn start();
        }

        struct NodeRequestReceivers {
            #(#node_receiver_params)*
        }

        pub(crate) fn start_bootstrap_request_receivers() {
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
            E: Fn(&dyn RinfRustSignal),
        {
            let NodeRequestReceivers {
                #(#node_loop_receiver_args,)*
            } = receivers;
            loop {
                tokio::select! {
                    #(#node_receiver_branches,)*
                    else => sleep(Duration::from_secs(1)).await,
                }
            }
        }

        fn answer_node_request<T>(
            request: &T,
            result: Result<Option<T::Response>>,
            emit: &impl Fn(&dyn RinfRustSignal),
        )
        where
            T: GuiRequest + TypedRequest + CallIdentified,
        {
            match result {
                Ok(response) => T::emit_response(response, emit),
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
            let (record_id, generation) = node.capability_library_identity()?;
            Ok(RequestCapabilities { record_id, generation, #(#capability_snapshot_fields)* })
        }

        #[cfg(test)]
        impl RequestCapabilities {
            pub(crate) fn all_available() -> Self {
                Self { record_id: None, generation: 0, #(#capability_slot_fields: RequestCapability::Available,)* }
            }

            pub(crate) fn from_implementation(implementation: &Implementation) -> Self {
                let slots = implementation.slots();
                Self { record_id: None, generation: 0, #(#capability_slot_fields: RequestCapability::from(<#capability_request_idents as TargetRequest>::slot(&slots)),)* }
            }
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum RequestAccess { NodeLocal, Control, ConnectionView, ConnectionEdit, Administer }
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum RequestScope { Node, Session, Library }
        pub fn request_access(name: &str) -> Option<RequestAccess> {
            match name { #(#accesses)* _ => None }
        }
        pub fn request_scope(name: &str) -> Option<RequestScope> {
            match name { #(#scopes)* _ => None }
        }

        #[cfg(test)]
        pub(crate) fn bootstrap_request_names() -> &'static [&'static str] {
            &[#(stringify!(#bootstrap_requests)),*]
        }


        implement_rinf_rust_signal_trait!(#(#all_responses),*);

        #[cfg(test)]
        pub(crate) fn non_local_request_names() -> &'static [&'static str] {
            &[#(stringify!(#non_local_requests)),*]
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
