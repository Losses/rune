use proc_macro::TokenStream;
use quote::quote;

struct RequestResponse {
    request: String,
    response: Option<String>,
    local_only: bool,
}

#[proc_macro]
pub fn define_request_types(_input: TokenStream) -> TokenStream {
    let types = vec![
        // Library
        RequestResponse {
            request: "TestLibraryInitializedRequest".to_string(),
            response: Some("TestLibraryInitializedResponse".to_string()),
            local_only: true,
        },
        RequestResponse {
            request: "CloseLibraryRequest".to_string(),
            response: Some("CloseLibraryResponse".to_string()),
            local_only: true,
        },
        RequestResponse {
            request: "CancelTaskRequest".to_string(),
            response: Some("CancelTaskResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "ScanAudioLibraryRequest".to_string(),
            response: None,
            local_only: false,
        },
        RequestResponse {
            request: "AnalyzeAudioLibraryRequest".to_string(),
            response: None,
            local_only: false,
        },
        RequestResponse {
            request: "DeduplicateAudioLibraryRequest".to_string(),
            response: None,
            local_only: false,
        },
        // Playback
        RequestResponse {
            request: "VolumeRequest".to_string(),
            response: Some("VolumeResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "LoadRequest".to_string(),
            response: None,
            local_only: false,
        },
        RequestResponse {
            request: "PlayRequest".to_string(),
            response: None,
            local_only: false,
        },
        RequestResponse {
            request: "PauseRequest".to_string(),
            response: None,
            local_only: false,
        },
        RequestResponse {
            request: "NextRequest".to_string(),
            response: None,
            local_only: false,
        },
        RequestResponse {
            request: "PreviousRequest".to_string(),
            response: None,
            local_only: false,
        },
        RequestResponse {
            request: "SwitchRequest".to_string(),
            response: None,
            local_only: false,
        },
        RequestResponse {
            request: "SeekRequest".to_string(),
            response: None,
            local_only: false,
        },
        RequestResponse {
            request: "RemoveRequest".to_string(),
            response: None,
            local_only: false,
        },
        RequestResponse {
            request: "SetPlaybackModeRequest".to_string(),
            response: None,
            local_only: false,
        },
        RequestResponse {
            request: "MovePlaylistItemRequest".to_string(),
            response: None,
            local_only: false,
        },
        RequestResponse {
            request: "SetRealtimeFFTEnabledRequest".to_string(),
            response: None,
            local_only: false,
        },
        RequestResponse {
            request: "SetAdaptiveSwitchingEnabledRequest".to_string(),
            response: None,
            local_only: false,
        },
        // SFX
        RequestResponse {
            request: "SfxPlayRequest".to_string(),
            response: None,
            local_only: true,
        },
        // Analyze
        RequestResponse {
            request: "IfAnalyzeExistsRequest".to_string(),
            response: Some("IfAnalyzeExistsResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "GetAnalyzeCountRequest".to_string(),
            response: Some("GetAnalyzeCountResponse".to_string()),
            local_only: false,
        },
        // Media File
        RequestResponse {
            request: "FetchMediaFilesRequest".to_string(),
            response: Some("FetchMediaFilesResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "FetchMediaFileByIdsRequest".to_string(),
            response: Some("FetchMediaFileByIdsResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "FetchParsedMediaFileRequest".to_string(),
            response: Some("FetchParsedMediaFileResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "SearchMediaFileSummaryRequest".to_string(),
            response: Some("SearchMediaFileSummaryResponse".to_string()),
            local_only: false,
        },
        // Lyric
        RequestResponse {
            request: "GetMediaFilesCountRequest".to_string(),
            response: Some("GetMediaFilesCountResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "GetLyricByTrackIdRequest".to_string(),
            response: Some("GetLyricByTrackIdResponse".to_string()),
            local_only: false,
        },
        // Collection
        RequestResponse {
            request: "FetchCollectionGroupSummaryRequest".to_string(),
            response: Some("CollectionGroupSummaryResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "FetchCollectionGroupsRequest".to_string(),
            response: Some("FetchCollectionGroupsResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "FetchCollectionByIdsRequest".to_string(),
            response: Some("FetchCollectionByIdsResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "SearchCollectionSummaryRequest".to_string(),
            response: Some("SearchCollectionSummaryResponse".to_string()),
            local_only: false,
        },
        // Cover Art
        RequestResponse {
            request: "GetCoverArtIdsByMixQueriesRequest".to_string(),
            response: Some("GetCoverArtIdsByMixQueriesResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "GetPrimaryColorByTrackIdRequest".to_string(),
            response: Some("GetPrimaryColorByTrackIdResponse".to_string()),
            local_only: false,
        },
        // Playlist
        RequestResponse {
            request: "FetchAllPlaylistsRequest".to_string(),
            response: Some("FetchAllPlaylistsResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "CreatePlaylistRequest".to_string(),
            response: Some("CreatePlaylistResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "CreateM3u8PlaylistRequest".to_string(),
            response: Some("CreateM3u8PlaylistResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "UpdatePlaylistRequest".to_string(),
            response: Some("UpdatePlaylistResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "RemovePlaylistRequest".to_string(),
            response: Some("RemovePlaylistResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "AddItemToPlaylistRequest".to_string(),
            response: Some("AddItemToPlaylistResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "ReorderPlaylistItemPositionRequest".to_string(),
            response: Some("ReorderPlaylistItemPositionResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "GetPlaylistByIdRequest".to_string(),
            response: Some("GetPlaylistByIdResponse".to_string()),
            local_only: false,
        },
        // Mix
        RequestResponse {
            request: "FetchAllMixesRequest".to_string(),
            response: Some("FetchAllMixesResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "CreateMixRequest".to_string(),
            response: Some("CreateMixResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "UpdateMixRequest".to_string(),
            response: Some("UpdateMixResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "RemoveMixRequest".to_string(),
            response: Some("RemoveMixResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "AddItemToMixRequest".to_string(),
            response: Some("AddItemToMixResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "GetMixByIdRequest".to_string(),
            response: Some("GetMixByIdResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "MixQueryRequest".to_string(),
            response: Some("MixQueryResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "FetchMixQueriesRequest".to_string(),
            response: Some("FetchMixQueriesResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "OperatePlaybackWithMixQueryRequest".to_string(),
            response: Some("OperatePlaybackWithMixQueryResponse".to_string()),
            local_only: false,
        },
        // Like
        RequestResponse {
            request: "SetLikedRequest".to_string(),
            response: Some("SetLikedResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "GetLikedRequest".to_string(),
            response: Some("GetLikedResponse".to_string()),
            local_only: false,
        },
        // Query and Search
        RequestResponse {
            request: "ComplexQueryRequest".to_string(),
            response: Some("ComplexQueryResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "SearchForRequest".to_string(),
            response: Some("SearchForResponse".to_string()),
            local_only: false,
        },
        // Directory
        RequestResponse {
            request: "FetchDirectoryTreeRequest".to_string(),
            response: Some("FetchDirectoryTreeResponse".to_string()),
            local_only: false,
        },
        // Scrobbler
        RequestResponse {
            request: "AuthenticateSingleServiceRequest".to_string(),
            response: Some("AuthenticateSingleServiceResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "AuthenticateMultipleServiceRequest".to_string(),
            response: None,
            local_only: false,
        },
        RequestResponse {
            request: "LogoutSingleServiceRequest".to_string(),
            response: None,
            local_only: false,
        },
        // Log
        RequestResponse {
            request: "ListLogRequest".to_string(),
            response: Some("ListLogResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "ClearLogRequest".to_string(),
            response: Some("ClearLogResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "RemoveLogRequest".to_string(),
            response: Some("RemoveLogResponse".to_string()),
            local_only: false,
        },
        // System
        RequestResponse {
            request: "SystemInfoRequest".to_string(),
            response: Some("SystemInfoResponse".to_string()),
            local_only: false,
        },
        // License
        RequestResponse {
            request: "RegisterLicenseRequest".to_string(),
            response: Some("RegisterLicenseResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "ValidateLicenseRequest".to_string(),
            response: Some("ValidateLicenseResponse".to_string()),
            local_only: false,
        },
        // Neighbors
        RequestResponse {
            request: "StartBroadcastRequest".to_string(),
            response: None,
            local_only: true,
        },
        RequestResponse {
            request: "StopBroadcastRequest".to_string(),
            response: None,
            local_only: true,
        },
        RequestResponse {
            request: "StartListeningRequest".to_string(),
            response: None,
            local_only: true,
        },
        RequestResponse {
            request: "StopListeningRequest".to_string(),
            response: None,
            local_only: true,
        },
        RequestResponse {
            request: "GetDiscoveredDeviceRequest".to_string(),
            response: Some("GetDiscoveredDeviceResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "StartServerRequest".to_string(),
            response: Some("StartServerResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "StopServerRequest".to_string(),
            response: Some("StopServerResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "ListClientsRequest".to_string(),
            response: Some("ListClientsResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "GetSslCertificateFingerprintRequest".to_string(),
            response: Some("GetSslCertificateFingerprintResponse".to_string()),
            local_only: true,
        },
        RequestResponse {
            request: "AddTrustedServerRequest".to_string(),
            response: Some("AddTrustedServerResponse".to_string()),
            local_only: true,
        },
        RequestResponse {
            request: "RemoveTrustedClientRequest".to_string(),
            response: Some("RemoveTrustedClientResponse".to_string()),
            local_only: true,
        },
        RequestResponse {
            request: "UpdateClientStatusRequest".to_string(),
            response: Some("UpdateClientStatusResponse".to_string()),
            local_only: false,
        },
        RequestResponse {
            request: "EditHostsRequest".to_string(),
            response: Some("EditHostsResponse".to_string()),
            local_only: true,
        },
        RequestResponse {
            request: "RemoveTrustedServerRequest".to_string(),
            response: Some("RemoveTrustedServerResponse".to_string()),
            local_only: true,
        },
        RequestResponse {
            request: "ServerAvailabilityTestRequest".to_string(),
            response: Some("ServerAvailabilityTestResponse".to_string()),
            local_only: true,
        },
        RequestResponse {
            request: "RegisterDeviceOnServerRequest".to_string(),
            response: Some("RegisterDeviceOnServerResponse".to_string()),
            local_only: true,
        },
        RequestResponse {
            request: "CheckDeviceOnServerRequest".to_string(),
            response: Some("CheckDeviceOnServerResponse".to_string()),
            local_only: true,
        },
        RequestResponse {
            request: "ConnectRequest".to_string(),
            response: Some("ConnectResponse".to_string()),
            local_only: true,
        },
        RequestResponse {
            request: "FetchServerCertificateRequest".to_string(),
            response: Some("FetchServerCertificateResponse".to_string()),
            local_only: true,
        },
        RequestResponse {
            request: "FetchRemoteFileRequest".to_string(),
            response: Some("FetchRemoteFileResponse".to_string()),
            local_only: true,
        },
        RequestResponse {
            request: "RemoveItemFromPlaylistRequest".to_string(),
            response: Some("RemoveItemFromPlaylistResponse".to_string()),
            local_only: true,
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
        .filter(|t| !t.local_only)
        .map(|t| syn::parse_str::<syn::Ident>(&t.request).unwrap())
        .collect();

    let local_response_pairs: Vec<_> = with_response
        .iter()
        .filter(|t| t.local_only)
        .map(|t| {
            let req_ident = syn::parse_str::<syn::Ident>(&t.request).unwrap();
            let resp_ident = syn::parse_str::<syn::Ident>(t.response.as_ref().unwrap()).unwrap();
            quote! { (#req_ident, #resp_ident) }
        })
        .collect();

    let local_request_only: Vec<_> = without_response
        .iter()
        .filter(|t| t.local_only)
        .map(|t| {
            let ident = syn::parse_str::<syn::Ident>(&t.request).unwrap();
            quote! { #ident }
        })
        .collect();

    let expanded = quote! {
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
