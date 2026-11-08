use proc_macro::TokenStream;
use quote::quote;

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

struct RequestResponse {
    request: String,
    response: Option<String>,
    scope: RequestScope,
    access: RequestAccess,
    call_id: bool,
    // This query resolves against the sender library before controlling its session.
    uses_controller_library: bool,
}

#[proc_macro]
pub fn define_request_types(_input: TokenStream) -> TokenStream {
    let types = vec![
        RequestResponse {
            request: "AttachRendererRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "DetachRendererRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "ListRenderersRequest".to_string(),
            response: Some("ListRenderersResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "HandoffRequest".to_string(),
            response: Some("HandoffResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "GetDeviceGrantsRequest".to_string(),
            response: Some("GetDeviceGrantsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SetDeviceGrantsRequest".to_string(),
            response: Some("SetDeviceGrantsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "ListControllersRequest".to_string(),
            response: Some("ListControllersResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "DetachControllerRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "ListLocalLibrariesRequest".to_string(),
            response: Some("ListLocalLibrariesResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SetLibraryBackgroundSyncRequest".to_string(),
            response: Some("SetLibraryBackgroundSyncResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "ListPeerLibrariesRequest".to_string(),
            response: Some("ListPeerLibrariesResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "AddRemoteLibraryRequest".to_string(),
            response: Some("AddRemoteLibraryResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        // Library
        RequestResponse {
            request: "GetSyncSettingsRequest".to_string(),
            response: Some("GetSyncSettingsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SetSyncSettingsRequest".to_string(),
            response: Some("SetSyncSettingsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SetDownloadRuleRequest".to_string(),
            response: Some("SetDownloadRuleResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "RemoveDownloadRuleRequest".to_string(),
            response: Some("RemoveDownloadRuleResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "TestLibraryInitializedRequest".to_string(),
            response: Some("TestLibraryInitializedResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "CloseLibraryRequest".to_string(),
            response: Some("CloseLibraryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "CancelTaskRequest".to_string(),
            response: Some("CancelTaskResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::Administer,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "ScanAudioLibraryRequest".to_string(),
            response: None,
            scope: RequestScope::Library,
            access: RequestAccess::Administer,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "AnalyzeAudioLibraryRequest".to_string(),
            response: None,
            scope: RequestScope::Library,
            access: RequestAccess::Administer,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "DeduplicateAudioLibraryRequest".to_string(),
            response: None,
            scope: RequestScope::Library,
            access: RequestAccess::Administer,
            call_id: false,
            uses_controller_library: false,
        },
        // Playback
        RequestResponse {
            request: "VolumeRequest".to_string(),
            response: Some("VolumeResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "LoadRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "PlayRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "PauseRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "NextRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "PreviousRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SwitchRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SeekRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "RemoveRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SetPlaybackModeRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "MovePlaylistItemRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SetRealtimeFFTEnabledRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SetAdaptiveSwitchingEnabledRequest".to_string(),
            response: None,
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        // This device's own audio session, so never forwarded to a renderer.
        RequestResponse {
            request: "SetMixWithOthersRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        // Visualizer (the GPU and the frame buffer live in this process)
        RequestResponse {
            request: "StartVisualizerRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "StopVisualizerRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "ResizeVisualizerRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SetVisualizerPresetDirsRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SetVisualizerAccentRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SetVisualizerVisibleRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "LoadVisualizerPresetRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "NextVisualizerPresetRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "PreviousVisualizerPresetRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "RandomVisualizerPresetRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "ImportVisualizerSceneRequest".to_string(),
            response: Some("ImportVisualizerSceneResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "RemoveVisualizerSceneRequest".to_string(),
            response: Some("RemoveVisualizerSceneResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SetVisualizerScenesRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SetVisualizerLogRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SetVisualizerLabRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "VisualizerLabCommandRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "VisualizerLabRequest".to_string(),
            response: Some("VisualizerLabResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "VisualizerPlanRequest".to_string(),
            response: Some("VisualizerPlanResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "VisualizerSupportRequest".to_string(),
            response: Some("VisualizerSupportResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "VisualizerStatusRequest".to_string(),
            response: Some("VisualizerStatusResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "ListVisualizerPresetsRequest".to_string(),
            response: Some("ListVisualizerPresetsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "ListVisualizerCreditsRequest".to_string(),
            response: Some("ListVisualizerCreditsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "VisualizerTextureShownRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        // SFX
        RequestResponse {
            request: "SfxPlayRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        // Analyze
        RequestResponse {
            request: "IfAnalyzeExistsRequest".to_string(),
            response: Some("IfAnalyzeExistsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "GetAnalyzeCountRequest".to_string(),
            response: Some("GetAnalyzeCountResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        // Media File
        RequestResponse {
            request: "FetchMediaFilesRequest".to_string(),
            response: Some("FetchMediaFilesResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "FetchMediaFileByIdsRequest".to_string(),
            response: Some("FetchMediaFileByIdsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "FetchParsedMediaFileRequest".to_string(),
            response: Some("FetchParsedMediaFileResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SearchMediaFileSummaryRequest".to_string(),
            response: Some("SearchMediaFileSummaryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        // Lyric
        RequestResponse {
            request: "GetMediaFilesCountRequest".to_string(),
            response: Some("GetMediaFilesCountResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "GetLyricByTrackIdRequest".to_string(),
            response: Some("GetLyricByTrackIdResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        // Collection
        RequestResponse {
            request: "FetchCollectionGroupSummaryRequest".to_string(),
            response: Some("CollectionGroupSummaryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "FetchCollectionGroupsRequest".to_string(),
            response: Some("FetchCollectionGroupsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "FetchCollectionByIdsRequest".to_string(),
            response: Some("FetchCollectionByIdsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SearchCollectionSummaryRequest".to_string(),
            response: Some("SearchCollectionSummaryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        // Cover Art
        RequestResponse {
            request: "GetCoverArtIdsByMixQueriesRequest".to_string(),
            response: Some("GetCoverArtIdsByMixQueriesResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "GetPrimaryColorByTrackIdRequest".to_string(),
            response: Some("GetPrimaryColorByTrackIdResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        // Playlist
        RequestResponse {
            request: "FetchAllPlaylistsRequest".to_string(),
            response: Some("FetchAllPlaylistsResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "CreatePlaylistRequest".to_string(),
            response: Some("CreatePlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "CreateM3u8PlaylistRequest".to_string(),
            response: Some("CreateM3u8PlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "UpdatePlaylistRequest".to_string(),
            response: Some("UpdatePlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "RemovePlaylistRequest".to_string(),
            response: Some("RemovePlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "AddItemToPlaylistRequest".to_string(),
            response: Some("AddItemToPlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "ReorderPlaylistItemPositionRequest".to_string(),
            response: Some("ReorderPlaylistItemPositionResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "FetchPlaylistOccurrencesRequest".to_string(),
            response: Some("FetchPlaylistOccurrencesResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "GetPlaylistByIdRequest".to_string(),
            response: Some("GetPlaylistByIdResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        // Mix
        RequestResponse {
            request: "FetchAllMixesRequest".to_string(),
            response: Some("FetchAllMixesResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "CreateMixRequest".to_string(),
            response: Some("CreateMixResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "UpdateMixRequest".to_string(),
            response: Some("UpdateMixResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "RemoveMixRequest".to_string(),
            response: Some("RemoveMixResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "AddItemToMixRequest".to_string(),
            response: Some("AddItemToMixResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "GetMixByIdRequest".to_string(),
            response: Some("GetMixByIdResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "MixQueryRequest".to_string(),
            response: Some("MixQueryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "FetchMixQueriesRequest".to_string(),
            response: Some("FetchMixQueriesResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "OperatePlaybackWithMixQueryRequest".to_string(),
            response: Some("OperatePlaybackWithMixQueryResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: true,
        },
        RequestResponse {
            request: "CastPreflightRequest".to_string(),
            response: Some("CastPreflightResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "CastLoadRequest".to_string(),
            response: Some("CastLoadResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SessionSnapshotRequest".to_string(),
            response: Some("SessionSnapshotResponse".to_string()),
            scope: RequestScope::Session,
            access: RequestAccess::Control,
            call_id: false,
            uses_controller_library: false,
        },
        // Like
        RequestResponse {
            request: "SetLikedRequest".to_string(),
            response: Some("SetLikedResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "GetLikedRequest".to_string(),
            response: Some("GetLikedResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        // Query and Search
        RequestResponse {
            request: "ComplexQueryRequest".to_string(),
            response: Some("ComplexQueryResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "SearchForRequest".to_string(),
            response: Some("SearchForResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: true,
            uses_controller_library: false,
        },
        // Directory
        RequestResponse {
            request: "FetchDirectoryTreeRequest".to_string(),
            response: Some("FetchDirectoryTreeResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionView,
            call_id: false,
            uses_controller_library: false,
        },
        // Scrobbler
        RequestResponse {
            request: "AuthenticateSingleServiceRequest".to_string(),
            response: Some("AuthenticateSingleServiceResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "AuthenticateMultipleServiceRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "LogoutSingleServiceRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        // Log
        RequestResponse {
            request: "ListLogRequest".to_string(),
            response: Some("ListLogResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "ClearLogRequest".to_string(),
            response: Some("ClearLogResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "RemoveLogRequest".to_string(),
            response: Some("RemoveLogResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        // System
        RequestResponse {
            request: "SystemInfoRequest".to_string(),
            response: Some("SystemInfoResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        // License
        RequestResponse {
            request: "RegisterLicenseRequest".to_string(),
            response: Some("RegisterLicenseResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "ValidateLicenseRequest".to_string(),
            response: Some("ValidateLicenseResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        // Neighbors
        RequestResponse {
            request: "StartBroadcastRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "StopBroadcastRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "StartListeningRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "StopListeningRequest".to_string(),
            response: None,
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "GetDiscoveredDeviceRequest".to_string(),
            response: Some("GetDiscoveredDeviceResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "StartServerRequest".to_string(),
            response: Some("StartServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "StopServerRequest".to_string(),
            response: Some("StopServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "ListClientsRequest".to_string(),
            response: Some("ListClientsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "GetSslCertificateFingerprintRequest".to_string(),
            response: Some("GetSslCertificateFingerprintResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "AddTrustedServerRequest".to_string(),
            response: Some("AddTrustedServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "RemoveTrustedClientRequest".to_string(),
            response: Some("RemoveTrustedClientResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "UpdateClientStatusRequest".to_string(),
            response: Some("UpdateClientStatusResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "EditHostsRequest".to_string(),
            response: Some("EditHostsResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "RemoveTrustedServerRequest".to_string(),
            response: Some("RemoveTrustedServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "ServerAvailabilityTestRequest".to_string(),
            response: Some("ServerAvailabilityTestResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "RegisterDeviceOnServerRequest".to_string(),
            response: Some("RegisterDeviceOnServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "CheckDeviceOnServerRequest".to_string(),
            response: Some("CheckDeviceOnServerResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "ConnectRequest".to_string(),
            response: Some("ConnectResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "FetchServerCertificateRequest".to_string(),
            response: Some("FetchServerCertificateResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "FetchRemoteFileRequest".to_string(),
            response: Some("FetchRemoteFileResponse".to_string()),
            scope: RequestScope::Node,
            access: RequestAccess::NodeLocal,
            call_id: false,
            uses_controller_library: false,
        },
        RequestResponse {
            request: "RemoveItemFromPlaylistRequest".to_string(),
            response: Some("RemoveItemFromPlaylistResponse".to_string()),
            scope: RequestScope::Library,
            access: RequestAccess::ConnectionEdit,
            call_id: false,
            uses_controller_library: false,
        },
    ];

    let (with_response, without_response): (Vec<_>, Vec<_>) =
        types.iter().partition(|t| t.response.is_some());

    let response_pairs: Vec<_> = with_response
        .iter()
        .map(|t| {
            let req_ident = syn::parse_str::<syn::Ident>(&t.request).unwrap();
            let resp_ident = syn::parse_str::<syn::Ident>(t.response.as_ref().unwrap()).unwrap();
            quote! { (#req_ident, #resp_ident) }
        })
        .collect();

    let request_only: Vec<_> = without_response
        .iter()
        .map(|t| {
            let ident = syn::parse_str::<syn::Ident>(&t.request).unwrap();
            quote! { #ident }
        })
        .collect();

    let all_responses: Vec<_> = with_response
        .iter()
        .map(|t| syn::parse_str::<syn::Ident>(t.response.as_ref().unwrap()).unwrap())
        .collect();

    let all_requests: Vec<_> = with_response
        .iter()
        .map(|t| syn::parse_str::<syn::Ident>(&t.request).unwrap())
        .chain(
            without_response
                .iter()
                .map(|t| syn::parse_str::<syn::Ident>(&t.request).unwrap()),
        )
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

    let non_local_requests: Vec<_> = types
        .iter()
        .filter(|t| t.access != RequestAccess::NodeLocal)
        .map(|t| syn::parse_str::<syn::Ident>(&t.request).unwrap())
        .collect();

    let local_response_pairs: Vec<_> = with_response
        .iter()
        .filter(|t| t.access == RequestAccess::NodeLocal)
        .map(|t| {
            let req_ident = syn::parse_str::<syn::Ident>(&t.request).unwrap();
            let resp_ident = syn::parse_str::<syn::Ident>(t.response.as_ref().unwrap()).unwrap();
            quote! { (#req_ident, #resp_ident) }
        })
        .collect();

    let local_request_only: Vec<_> = without_response
        .iter()
        .filter(|t| t.access == RequestAccess::NodeLocal)
        .map(|t| {
            let ident = syn::parse_str::<syn::Ident>(&t.request).unwrap();
            quote! { #ident }
        })
        .collect();

    let remote_response_pairs: Vec<_> = with_response
        .iter()
        .filter(|t| t.access != RequestAccess::NodeLocal)
        .map(|t| {
            let req_ident = syn::parse_str::<syn::Ident>(&t.request).unwrap();
            let resp_ident = syn::parse_str::<syn::Ident>(t.response.as_ref().unwrap()).unwrap();
            quote! { (#req_ident, #resp_ident) }
        })
        .collect();

    let remote_request_only: Vec<_> = without_response
        .iter()
        .filter(|t| t.access != RequestAccess::NodeLocal)
        .map(|t| {
            let ident = syn::parse_str::<syn::Ident>(&t.request).unwrap();
            quote! { #ident }
        })
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
    let controller_library_queries: Vec<_> = types
        .iter()
        .filter(|t| t.uses_controller_library)
        .map(|t| syn::parse_str::<syn::Ident>(&t.request).unwrap())
        .collect();

    let expanded = quote! {
        #(#call_id_impls)*
        #(#typed_request_impls)*

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
        pub fn request_uses_controller_library(name: &str) -> bool {
            matches!(name, #(stringify!(#controller_library_queries))|*)
        }


        #[macro_export]
        macro_rules! for_all_request_pairs2 {
            ($m:tt, $param1:expr, $param2:expr) => {
                $m!($param1, $param2 #(, #response_pairs)* #(, #request_only)*);
            };
        }

        #[macro_export]
        macro_rules! for_all_remote_request_pairs2 {
            ($m:tt, $param1:expr, $param2:expr) => {
                $m!($param1, $param2 #(, #remote_response_pairs)* #(, #remote_request_only)*);
            };
        }

        #[macro_export]
        macro_rules! for_all_local_only_request_pairs2 {
            ($m:tt, $param1:expr, $param2:expr) => {
                $m!($param1, $param2 #(, #local_response_pairs)* #(, #local_request_only)*);
            };
        }

        #[macro_export]
        macro_rules! for_all_responses0 {
            ($m:tt) => {
                $m!(#(#all_responses),*);
            }
        }

        #[macro_export]
        macro_rules! for_all_responses {
            ($m:tt, $params:expr) => {
                $m!($params, #(#all_responses),*);
            }
        }


        #[macro_export]
        macro_rules! for_all_requests0 {
            ($m:tt) => {
                $m!(#(#all_requests),*);
            }
        }



        #[macro_export]
        macro_rules! for_all_non_local_requests {
            ($m:tt, $params:expr) => {
                $m!($params, #(#non_local_requests),*);
            }
        }


        #[macro_export]
        macro_rules! for_all_non_local_requests3 {
            ($m:tt, $param1:expr, $param2:expr, $param3:expr) => {
                $m!($param1, $param2, $param3, #(#non_local_requests),*);
            };
        }
    };

    TokenStream::from(expanded)
}
