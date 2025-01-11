use proc_macro::TokenStream;
use quote::quote;

struct RequestResponse {
    request: String,
    response: Option<String>,
}

#[proc_macro]
pub fn define_request_types(_input: TokenStream) -> TokenStream {
    let types = vec![
        // Library
        RequestResponse {
            request: "TestLibraryInitializedRequest".to_string(),
            response: Some("TestLibraryInitializedResponse".to_string()),
        },
        RequestResponse {
            request: "CloseLibraryRequest".to_string(),
            response: Some("CloseLibraryResponse".to_string()),
        },
        RequestResponse {
            request: "CancelTaskRequest".to_string(),
            response: Some("CancelTaskResponse".to_string()),
        },
        RequestResponse {
            request: "ScanAudioLibraryRequest".to_string(),
            response: None,
        },
        RequestResponse {
            request: "AnalyzeAudioLibraryRequest".to_string(),
            response: None,
        },
        // Playback
        RequestResponse {
            request: "VolumeRequest".to_string(),
            response: Some("VolumeResponse".to_string()),
        },
        RequestResponse {
            request: "LoadRequest".to_string(),
            response: None,
        },
        RequestResponse {
            request: "PlayRequest".to_string(),
            response: None,
        },
        RequestResponse {
            request: "PauseRequest".to_string(),
            response: None,
        },
        RequestResponse {
            request: "NextRequest".to_string(),
            response: None,
        },
        RequestResponse {
            request: "PreviousRequest".to_string(),
            response: None,
        },
        RequestResponse {
            request: "SwitchRequest".to_string(),
            response: None,
        },
        RequestResponse {
            request: "SeekRequest".to_string(),
            response: None,
        },
        RequestResponse {
            request: "RemoveRequest".to_string(),
            response: None,
        },
        RequestResponse {
            request: "SetPlaybackModeRequest".to_string(),
            response: None,
        },
        RequestResponse {
            request: "MovePlaylistItemRequest".to_string(),
            response: None,
        },
        RequestResponse {
            request: "SetRealtimeFftEnabledRequest".to_string(),
            response: None,
        },
        RequestResponse {
            request: "SetAdaptiveSwitchingEnabledRequest".to_string(),
            response: None,
        },
        // SFX
        RequestResponse {
            request: "SfxPlayRequest".to_string(),
            response: None,
        },
        // Analyze
        RequestResponse {
            request: "IfAnalyzeExistsRequest".to_string(),
            response: Some("IfAnalyzeExistsResponse".to_string()),
        },
        RequestResponse {
            request: "GetAnalyzeCountRequest".to_string(),
            response: Some("GetAnalyzeCountResponse".to_string()),
        },
        // Media File
        RequestResponse {
            request: "FetchMediaFilesRequest".to_string(),
            response: Some("FetchMediaFilesResponse".to_string()),
        },
        RequestResponse {
            request: "FetchMediaFileByIdsRequest".to_string(),
            response: Some("FetchMediaFileByIdsResponse".to_string()),
        },
        RequestResponse {
            request: "FetchParsedMediaFileRequest".to_string(),
            response: Some("FetchParsedMediaFileResponse".to_string()),
        },
        RequestResponse {
            request: "SearchMediaFileSummaryRequest".to_string(),
            response: Some("SearchMediaFileSummaryResponse".to_string()),
        },
        // Lyric
        RequestResponse {
            request: "GetLyricByTrackIdRequest".to_string(),
            response: Some("GetLyricByTrackIdResponse".to_string()),
        },
        // Collection
        RequestResponse {
            request: "FetchCollectionGroupSummaryRequest".to_string(),
            response: Some("CollectionGroupSummaryResponse".to_string()),
        },
        RequestResponse {
            request: "FetchCollectionGroupsRequest".to_string(),
            response: Some("CollectionGroups".to_string()),
        },
        RequestResponse {
            request: "FetchCollectionByIdsRequest".to_string(),
            response: Some("FetchCollectionByIdsResponse".to_string()),
        },
        RequestResponse {
            request: "SearchCollectionSummaryRequest".to_string(),
            response: Some("SearchCollectionSummaryResponse".to_string()),
        },
        // Cover Art
        RequestResponse {
            request: "GetCoverArtIdsByMixQueriesRequest".to_string(),
            response: Some("GetCoverArtIdsByMixQueriesResponse".to_string()),
        },
        RequestResponse {
            request: "GetPrimaryColorByTrackIdRequest".to_string(),
            response: Some("GetPrimaryColorByTrackIdResponse".to_string()),
        },
        // Playlist
        RequestResponse {
            request: "FetchAllPlaylistsRequest".to_string(),
            response: Some("FetchAllPlaylistsResponse".to_string()),
        },
        RequestResponse {
            request: "CreatePlaylistRequest".to_string(),
            response: Some("CreatePlaylistResponse".to_string()),
        },
        RequestResponse {
            request: "CreateM3u8PlaylistRequest".to_string(),
            response: Some("CreateM3u8PlaylistResponse".to_string()),
        },
        RequestResponse {
            request: "UpdatePlaylistRequest".to_string(),
            response: Some("UpdatePlaylistResponse".to_string()),
        },
        RequestResponse {
            request: "RemovePlaylistRequest".to_string(),
            response: Some("RemovePlaylistResponse".to_string()),
        },
        RequestResponse {
            request: "AddItemToPlaylistRequest".to_string(),
            response: Some("AddItemToPlaylistResponse".to_string()),
        },
        RequestResponse {
            request: "ReorderPlaylistItemPositionRequest".to_string(),
            response: Some("ReorderPlaylistItemPositionResponse".to_string()),
        },
        RequestResponse {
            request: "GetPlaylistByIdRequest".to_string(),
            response: Some("GetPlaylistByIdResponse".to_string()),
        },
        // Mix
        RequestResponse {
            request: "FetchAllMixesRequest".to_string(),
            response: Some("FetchAllMixesResponse".to_string()),
        },
        RequestResponse {
            request: "CreateMixRequest".to_string(),
            response: Some("CreateMixResponse".to_string()),
        },
        RequestResponse {
            request: "UpdateMixRequest".to_string(),
            response: Some("UpdateMixResponse".to_string()),
        },
        RequestResponse {
            request: "RemoveMixRequest".to_string(),
            response: Some("RemoveMixResponse".to_string()),
        },
        RequestResponse {
            request: "AddItemToMixRequest".to_string(),
            response: Some("AddItemToMixResponse".to_string()),
        },
        RequestResponse {
            request: "GetMixByIdRequest".to_string(),
            response: Some("GetMixByIdResponse".to_string()),
        },
        RequestResponse {
            request: "MixQueryRequest".to_string(),
            response: Some("MixQueryResponse".to_string()),
        },
        RequestResponse {
            request: "FetchMixQueriesRequest".to_string(),
            response: Some("FetchMixQueriesResponse".to_string()),
        },
        RequestResponse {
            request: "OperatePlaybackWithMixQueryRequest".to_string(),
            response: Some("OperatePlaybackWithMixQueryResponse".to_string()),
        },
        // Like
        RequestResponse {
            request: "SetLikedRequest".to_string(),
            response: Some("SetLikedResponse".to_string()),
        },
        RequestResponse {
            request: "GetLikedRequest".to_string(),
            response: Some("GetLikedResponse".to_string()),
        },
        // Query and Search
        RequestResponse {
            request: "ComplexQueryRequest".to_string(),
            response: Some("ComplexQueryResponse".to_string()),
        },
        RequestResponse {
            request: "SearchForRequest".to_string(),
            response: Some("SearchForResponse".to_string()),
        },
        // Directory
        RequestResponse {
            request: "FetchDirectoryTreeRequest".to_string(),
            response: Some("FetchDirectoryTreeResponse".to_string()),
        },
        // Scrobbler
        RequestResponse {
            request: "AuthenticateSingleServiceRequest".to_string(),
            response: Some("AuthenticateSingleServiceResponse".to_string()),
        },
        RequestResponse {
            request: "AuthenticateMultipleServiceRequest".to_string(),
            response: None,
        },
        RequestResponse {
            request: "LogoutSingleServiceRequest".to_string(),
            response: None,
        },
        // Log
        RequestResponse {
            request: "ListLogRequest".to_string(),
            response: Some("ListLogResponse".to_string()),
        },
        RequestResponse {
            request: "ClearLogRequest".to_string(),
            response: Some("ClearLogResponse".to_string()),
        },
        RequestResponse {
            request: "RemoveLogRequest".to_string(),
            response: Some("RemoveLogResponse".to_string()),
        },
        // System
        RequestResponse {
            request: "SystemInfoRequest".to_string(),
            response: Some("SystemInfoResponse".to_string()),
        },
        // License
        RequestResponse {
            request: "RegisterLicenseRequest".to_string(),
            response: Some("RegisterLicenseResponse".to_string()),
        },
        RequestResponse {
            request: "ValidateLicenseRequest".to_string(),
            response: Some("ValidateLicenseResponse".to_string()),
        },
    ];

    let (with_response, without_response): (Vec<_>, Vec<_>) =
        types.into_iter().partition(|t| t.response.is_some());

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

    let response_only: Vec<_> = with_response
        .iter()
        .map(|t| {
            let resp_ident = syn::parse_str::<syn::Ident>(t.response.as_ref().unwrap()).unwrap();
            quote! { #resp_ident }
        })
        .collect();

    let expanded = quote! {
        #[macro_export]
        macro_rules! for_all_requests {
            ($m:tt, $params:expr) => {
                $m!($params #(, #response_pairs)* #(, #request_only)*);
            }
        }

        #[macro_export]
        macro_rules! for_all_requests2 {
            ($m:tt, $param1:expr, $param2:expr) => {
                $m!($param1, $param2 #(, #response_pairs)* #(, #request_only)*);
            };
        }

        #[macro_export]
        macro_rules! for_all_responses {
            ($m:tt, $params:expr) => {
                $m!($params #(, #response_only)*);
            }
        }
    };

    TokenStream::from(expanded)
}
