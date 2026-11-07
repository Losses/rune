use proc_macro::TokenStream;
use quote::quote;

#[derive(Clone, Copy, PartialEq)]
enum RequestClass {
    NodeLocal,
    Control,
    LibraryRead,
    LibraryWrite,
    Administer,
}

struct RequestResponse {
    request: String,
    response: Option<String>,
    class: RequestClass,
}

#[proc_macro]
pub fn define_request_types(_input: TokenStream) -> TokenStream {
    let types = vec![
        RequestResponse {
            request: "AttachRendererRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "DetachRendererRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "ListRenderersRequest".to_string(),
            response: Some("ListRenderersResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "HandoffRequest".to_string(),
            response: Some("HandoffResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "GetDeviceGrantsRequest".to_string(),
            response: Some("GetDeviceGrantsResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "SetDeviceGrantsRequest".to_string(),
            response: Some("SetDeviceGrantsResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "ListControllersRequest".to_string(),
            response: Some("ListControllersResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "DetachControllerRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "ListLocalLibrariesRequest".to_string(),
            response: Some("ListLocalLibrariesResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "SetLibraryBackgroundSyncRequest".to_string(),
            response: Some("SetLibraryBackgroundSyncResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "ListPeerLibrariesRequest".to_string(),
            response: Some("ListPeerLibrariesResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "AddRemoteLibraryRequest".to_string(),
            response: Some("AddRemoteLibraryResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        // Library
        RequestResponse {
            request: "GetSyncSettingsRequest".to_string(),
            response: Some("GetSyncSettingsResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "SetSyncSettingsRequest".to_string(),
            response: Some("SetSyncSettingsResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "SetDownloadRuleRequest".to_string(),
            response: Some("SetDownloadRuleResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "RemoveDownloadRuleRequest".to_string(),
            response: Some("RemoveDownloadRuleResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "TestLibraryInitializedRequest".to_string(),
            response: Some("TestLibraryInitializedResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "CloseLibraryRequest".to_string(),
            response: Some("CloseLibraryResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "CancelTaskRequest".to_string(),
            response: Some("CancelTaskResponse".to_string()),
            class: RequestClass::Administer,
        },
        RequestResponse {
            request: "ScanAudioLibraryRequest".to_string(),
            response: None,
            class: RequestClass::Administer,
        },
        RequestResponse {
            request: "AnalyzeAudioLibraryRequest".to_string(),
            response: None,
            class: RequestClass::Administer,
        },
        RequestResponse {
            request: "DeduplicateAudioLibraryRequest".to_string(),
            response: None,
            class: RequestClass::Administer,
        },
        // Playback
        RequestResponse {
            request: "VolumeRequest".to_string(),
            response: Some("VolumeResponse".to_string()),
            class: RequestClass::Control,
        },
        RequestResponse {
            request: "LoadRequest".to_string(),
            response: None,
            class: RequestClass::Control,
        },
        RequestResponse {
            request: "PlayRequest".to_string(),
            response: None,
            class: RequestClass::Control,
        },
        RequestResponse {
            request: "PauseRequest".to_string(),
            response: None,
            class: RequestClass::Control,
        },
        RequestResponse {
            request: "NextRequest".to_string(),
            response: None,
            class: RequestClass::Control,
        },
        RequestResponse {
            request: "PreviousRequest".to_string(),
            response: None,
            class: RequestClass::Control,
        },
        RequestResponse {
            request: "SwitchRequest".to_string(),
            response: None,
            class: RequestClass::Control,
        },
        RequestResponse {
            request: "SeekRequest".to_string(),
            response: None,
            class: RequestClass::Control,
        },
        RequestResponse {
            request: "RemoveRequest".to_string(),
            response: None,
            class: RequestClass::Control,
        },
        RequestResponse {
            request: "SetPlaybackModeRequest".to_string(),
            response: None,
            class: RequestClass::Control,
        },
        RequestResponse {
            request: "MovePlaylistItemRequest".to_string(),
            response: None,
            class: RequestClass::Control,
        },
        RequestResponse {
            request: "SetRealtimeFFTEnabledRequest".to_string(),
            response: None,
            class: RequestClass::Control,
        },
        RequestResponse {
            request: "SetAdaptiveSwitchingEnabledRequest".to_string(),
            response: None,
            class: RequestClass::Control,
        },
        // Visualizer (the GPU and the frame buffer live in this process)
        RequestResponse {
            request: "StartVisualizerRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "StopVisualizerRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "ResizeVisualizerRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "SetVisualizerPresetDirsRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "SetVisualizerAccentRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "SetVisualizerVisibleRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "LoadVisualizerPresetRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "NextVisualizerPresetRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "PreviousVisualizerPresetRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "RandomVisualizerPresetRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "ImportVisualizerSceneRequest".to_string(),
            response: Some("ImportVisualizerSceneResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "RemoveVisualizerSceneRequest".to_string(),
            response: Some("RemoveVisualizerSceneResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "SetVisualizerScenesRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "SetVisualizerLogRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "SetVisualizerLabRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "VisualizerLabCommandRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "VisualizerLabRequest".to_string(),
            response: Some("VisualizerLabResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "VisualizerPlanRequest".to_string(),
            response: Some("VisualizerPlanResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "VisualizerSupportRequest".to_string(),
            response: Some("VisualizerSupportResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "VisualizerStatusRequest".to_string(),
            response: Some("VisualizerStatusResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "ListVisualizerPresetsRequest".to_string(),
            response: Some("ListVisualizerPresetsResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "ListVisualizerCreditsRequest".to_string(),
            response: Some("ListVisualizerCreditsResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "VisualizerTextureShownRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        // SFX
        RequestResponse {
            request: "SfxPlayRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        // Analyze
        RequestResponse {
            request: "IfAnalyzeExistsRequest".to_string(),
            response: Some("IfAnalyzeExistsResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "GetAnalyzeCountRequest".to_string(),
            response: Some("GetAnalyzeCountResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        // Media File
        RequestResponse {
            request: "FetchMediaFilesRequest".to_string(),
            response: Some("FetchMediaFilesResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "FetchMediaFileByIdsRequest".to_string(),
            response: Some("FetchMediaFileByIdsResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "FetchParsedMediaFileRequest".to_string(),
            response: Some("FetchParsedMediaFileResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "SearchMediaFileSummaryRequest".to_string(),
            response: Some("SearchMediaFileSummaryResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        // Lyric
        RequestResponse {
            request: "GetMediaFilesCountRequest".to_string(),
            response: Some("GetMediaFilesCountResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "GetLyricByTrackIdRequest".to_string(),
            response: Some("GetLyricByTrackIdResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        // Collection
        RequestResponse {
            request: "FetchCollectionGroupSummaryRequest".to_string(),
            response: Some("CollectionGroupSummaryResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "FetchCollectionGroupsRequest".to_string(),
            response: Some("FetchCollectionGroupsResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "FetchCollectionByIdsRequest".to_string(),
            response: Some("FetchCollectionByIdsResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "SearchCollectionSummaryRequest".to_string(),
            response: Some("SearchCollectionSummaryResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        // Cover Art
        RequestResponse {
            request: "GetCoverArtIdsByMixQueriesRequest".to_string(),
            response: Some("GetCoverArtIdsByMixQueriesResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "GetPrimaryColorByTrackIdRequest".to_string(),
            response: Some("GetPrimaryColorByTrackIdResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        // Playlist
        RequestResponse {
            request: "FetchAllPlaylistsRequest".to_string(),
            response: Some("FetchAllPlaylistsResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "CreatePlaylistRequest".to_string(),
            response: Some("CreatePlaylistResponse".to_string()),
            class: RequestClass::LibraryWrite,
        },
        RequestResponse {
            request: "CreateM3u8PlaylistRequest".to_string(),
            response: Some("CreateM3u8PlaylistResponse".to_string()),
            class: RequestClass::LibraryWrite,
        },
        RequestResponse {
            request: "UpdatePlaylistRequest".to_string(),
            response: Some("UpdatePlaylistResponse".to_string()),
            class: RequestClass::LibraryWrite,
        },
        RequestResponse {
            request: "RemovePlaylistRequest".to_string(),
            response: Some("RemovePlaylistResponse".to_string()),
            class: RequestClass::LibraryWrite,
        },
        RequestResponse {
            request: "AddItemToPlaylistRequest".to_string(),
            response: Some("AddItemToPlaylistResponse".to_string()),
            class: RequestClass::LibraryWrite,
        },
        RequestResponse {
            request: "ReorderPlaylistItemPositionRequest".to_string(),
            response: Some("ReorderPlaylistItemPositionResponse".to_string()),
            class: RequestClass::LibraryWrite,
        },
        RequestResponse {
            request: "FetchPlaylistOccurrencesRequest".to_string(),
            response: Some("FetchPlaylistOccurrencesResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "GetPlaylistByIdRequest".to_string(),
            response: Some("GetPlaylistByIdResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        // Mix
        RequestResponse {
            request: "FetchAllMixesRequest".to_string(),
            response: Some("FetchAllMixesResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "CreateMixRequest".to_string(),
            response: Some("CreateMixResponse".to_string()),
            class: RequestClass::LibraryWrite,
        },
        RequestResponse {
            request: "UpdateMixRequest".to_string(),
            response: Some("UpdateMixResponse".to_string()),
            class: RequestClass::LibraryWrite,
        },
        RequestResponse {
            request: "RemoveMixRequest".to_string(),
            response: Some("RemoveMixResponse".to_string()),
            class: RequestClass::LibraryWrite,
        },
        RequestResponse {
            request: "AddItemToMixRequest".to_string(),
            response: Some("AddItemToMixResponse".to_string()),
            class: RequestClass::LibraryWrite,
        },
        RequestResponse {
            request: "GetMixByIdRequest".to_string(),
            response: Some("GetMixByIdResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "MixQueryRequest".to_string(),
            response: Some("MixQueryResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "FetchMixQueriesRequest".to_string(),
            response: Some("FetchMixQueriesResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "OperatePlaybackWithMixQueryRequest".to_string(),
            response: Some("OperatePlaybackWithMixQueryResponse".to_string()),
            class: RequestClass::Control,
        },
        RequestResponse {
            request: "CastPreflightRequest".to_string(),
            response: Some("CastPreflightResponse".to_string()),
            class: RequestClass::Control,
        },
        RequestResponse {
            request: "CastLoadRequest".to_string(),
            response: Some("CastLoadResponse".to_string()),
            class: RequestClass::Control,
        },
        RequestResponse {
            request: "SessionSnapshotRequest".to_string(),
            response: Some("SessionSnapshotResponse".to_string()),
            class: RequestClass::Control,
        },
        // Like
        RequestResponse {
            request: "SetLikedRequest".to_string(),
            response: Some("SetLikedResponse".to_string()),
            class: RequestClass::LibraryWrite,
        },
        RequestResponse {
            request: "GetLikedRequest".to_string(),
            response: Some("GetLikedResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        // Query and Search
        RequestResponse {
            request: "ComplexQueryRequest".to_string(),
            response: Some("ComplexQueryResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        RequestResponse {
            request: "SearchForRequest".to_string(),
            response: Some("SearchForResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        // Directory
        RequestResponse {
            request: "FetchDirectoryTreeRequest".to_string(),
            response: Some("FetchDirectoryTreeResponse".to_string()),
            class: RequestClass::LibraryRead,
        },
        // Scrobbler
        RequestResponse {
            request: "AuthenticateSingleServiceRequest".to_string(),
            response: Some("AuthenticateSingleServiceResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "AuthenticateMultipleServiceRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "LogoutSingleServiceRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        // Log
        RequestResponse {
            request: "ListLogRequest".to_string(),
            response: Some("ListLogResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "ClearLogRequest".to_string(),
            response: Some("ClearLogResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "RemoveLogRequest".to_string(),
            response: Some("RemoveLogResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        // System
        RequestResponse {
            request: "SystemInfoRequest".to_string(),
            response: Some("SystemInfoResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        // License
        RequestResponse {
            request: "RegisterLicenseRequest".to_string(),
            response: Some("RegisterLicenseResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "ValidateLicenseRequest".to_string(),
            response: Some("ValidateLicenseResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        // Neighbors
        RequestResponse {
            request: "StartBroadcastRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "StopBroadcastRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "StartListeningRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "StopListeningRequest".to_string(),
            response: None,
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "GetDiscoveredDeviceRequest".to_string(),
            response: Some("GetDiscoveredDeviceResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "StartServerRequest".to_string(),
            response: Some("StartServerResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "StopServerRequest".to_string(),
            response: Some("StopServerResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "ListClientsRequest".to_string(),
            response: Some("ListClientsResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "GetSslCertificateFingerprintRequest".to_string(),
            response: Some("GetSslCertificateFingerprintResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "AddTrustedServerRequest".to_string(),
            response: Some("AddTrustedServerResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "RemoveTrustedClientRequest".to_string(),
            response: Some("RemoveTrustedClientResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "UpdateClientStatusRequest".to_string(),
            response: Some("UpdateClientStatusResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "EditHostsRequest".to_string(),
            response: Some("EditHostsResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "RemoveTrustedServerRequest".to_string(),
            response: Some("RemoveTrustedServerResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "ServerAvailabilityTestRequest".to_string(),
            response: Some("ServerAvailabilityTestResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "RegisterDeviceOnServerRequest".to_string(),
            response: Some("RegisterDeviceOnServerResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "CheckDeviceOnServerRequest".to_string(),
            response: Some("CheckDeviceOnServerResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "ConnectRequest".to_string(),
            response: Some("ConnectResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "FetchServerCertificateRequest".to_string(),
            response: Some("FetchServerCertificateResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "FetchRemoteFileRequest".to_string(),
            response: Some("FetchRemoteFileResponse".to_string()),
            class: RequestClass::NodeLocal,
        },
        RequestResponse {
            request: "RemoveItemFromPlaylistRequest".to_string(),
            response: Some("RemoveItemFromPlaylistResponse".to_string()),
            class: RequestClass::LibraryWrite,
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

    let non_local_requests: Vec<_> = types
        .iter()
        .filter(|t| t.class != RequestClass::NodeLocal)
        .map(|t| syn::parse_str::<syn::Ident>(&t.request).unwrap())
        .collect();

    let local_response_pairs: Vec<_> = with_response
        .iter()
        .filter(|t| t.class == RequestClass::NodeLocal)
        .map(|t| {
            let req_ident = syn::parse_str::<syn::Ident>(&t.request).unwrap();
            let resp_ident = syn::parse_str::<syn::Ident>(t.response.as_ref().unwrap()).unwrap();
            quote! { (#req_ident, #resp_ident) }
        })
        .collect();

    let local_request_only: Vec<_> = without_response
        .iter()
        .filter(|t| t.class == RequestClass::NodeLocal)
        .map(|t| {
            let ident = syn::parse_str::<syn::Ident>(&t.request).unwrap();
            quote! { #ident }
        })
        .collect();

    let remote_response_pairs: Vec<_> = with_response
        .iter()
        .filter(|t| t.class != RequestClass::NodeLocal)
        .map(|t| {
            let req_ident = syn::parse_str::<syn::Ident>(&t.request).unwrap();
            let resp_ident = syn::parse_str::<syn::Ident>(t.response.as_ref().unwrap()).unwrap();
            quote! { (#req_ident, #resp_ident) }
        })
        .collect();

    let remote_request_only: Vec<_> = without_response
        .iter()
        .filter(|t| t.class != RequestClass::NodeLocal)
        .map(|t| {
            let ident = syn::parse_str::<syn::Ident>(&t.request).unwrap();
            quote! { #ident }
        })
        .collect();

    let classifications: Vec<_> = types
        .iter()
        .map(|t| {
            let request = &t.request;
            let class = match t.class {
                RequestClass::NodeLocal => quote! { RequestClass::NodeLocal },
                RequestClass::Control => quote! { RequestClass::Control },
                RequestClass::LibraryRead => quote! { RequestClass::LibraryRead },
                RequestClass::LibraryWrite => quote! { RequestClass::LibraryWrite },
                RequestClass::Administer => quote! { RequestClass::Administer },
            };
            quote! { #request => Some(#class), }
        })
        .collect();
    let expanded = quote! {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum RequestClass { NodeLocal, Control, LibraryRead, LibraryWrite, Administer }
        pub fn request_class(name: &str) -> Option<RequestClass> {
            match name { #(#classifications)* _ => None }
        }

        #[macro_export]
        macro_rules! for_all_request_pairs {
            ($m:tt, $params:expr) => {
                $m!($params #(, #response_pairs)* #(, #request_only)*);
            }
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
        macro_rules! for_all_responses2 {
            ($m:tt, $param1:expr, $param2:expr) => {
                $m!($param1, $param2);
            };
        }

        #[macro_export]
        macro_rules! for_all_requests0 {
            ($m:tt) => {
                $m!(#(#all_requests),*);
            }
        }

        #[macro_export]
        macro_rules! for_all_requests {
            ($m:tt, $params:expr) => {
                $m!($params);
            }
        }

        #[macro_export]
        macro_rules! for_all_requests2 {
            ($m:tt, $param1:expr, $param2:expr) => {
                $m!($param1, $param2);
            };
        }

        #[macro_export]
        macro_rules! for_all_non_local_requests {
            ($m:tt, $params:expr) => {
                $m!($params, #(#non_local_requests),*);
            }
        }

        #[macro_export]
        macro_rules! for_all_non_local_requests2 {
            ($m:tt, $param1:expr, $param2:expr) => {
                $m!($param1, $param2, #(#non_local_requests),*);
            };
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
