use proc_macro::TokenStream;
use quote::quote;

#[proc_macro]
pub fn define_request_types(_input: TokenStream) -> TokenStream {
    let types = vec![
        // Library
        "TestLibraryInitializedRequest",
        "CloseLibraryRequest",
        "CancelTaskRequest",
        "ScanAudioLibraryRequest",
        "AnalyzeAudioLibraryRequest",
        // Playback
        "VolumeRequest",
        "LoadRequest",
        "PlayRequest",
        "PauseRequest",
        "NextRequest",
        "PreviousRequest",
        "SwitchRequest",
        "SeekRequest",
        "RemoveRequest",
        "SetPlaybackModeRequest",
        "MovePlaylistItemRequest",
        "SetRealtimeFftEnabledRequest",
        "SetAdaptiveSwitchingEnabledRequest",
        // SFX
        "SfxPlayRequest",
        // Analyze
        "IfAnalyzeExistsRequest",
        "GetAnalyzeCountRequest",
        // Media File
        "FetchMediaFilesRequest",
        "FetchMediaFileByIdsRequest",
        "FetchParsedMediaFileRequest",
        "SearchMediaFileSummaryRequest",
        // Lyric
        "GetLyricByTrackIdRequest",
        // Collection
        "FetchCollectionGroupSummaryRequest",
        "FetchCollectionGroupsRequest",
        "FetchCollectionByIdsRequest",
        "SearchCollectionSummaryRequest",
        // Cover Art
        "GetCoverArtIdsByMixQueriesRequest",
        "GetPrimaryColorByTrackIdRequest",
        // Playlist
        "FetchAllPlaylistsRequest",
        "CreatePlaylistRequest",
        "CreateM3u8PlaylistRequest",
        "UpdatePlaylistRequest",
        "RemovePlaylistRequest",
        "AddItemToPlaylistRequest",
        "ReorderPlaylistItemPositionRequest",
        "GetPlaylistByIdRequest",
        // Mix
        "FetchAllMixesRequest",
        "CreateMixRequest",
        "UpdateMixRequest",
        "RemoveMixRequest",
        "AddItemToMixRequest",
        "GetMixByIdRequest",
        "MixQueryRequest",
        "FetchMixQueriesRequest",
        "OperatePlaybackWithMixQueryRequest",
        // Like
        "SetLikedRequest",
        "GetLikedRequest",
        // Query and Search
        "ComplexQueryRequest",
        "SearchForRequest",
        // Directory
        "FetchDirectoryTreeRequest",
        // Scrobbler
        "AuthenticateSingleServiceRequest",
        "AuthenticateMultipleServiceRequest",
        "LogoutSingleServiceRequest",
        // Log
        "ListLogRequest",
        "ClearLogRequest",
        "RemoveLogRequest",
        // System
        "SystemInfoRequest",
        // License
        "RegisterLicenseRequest",
        "ValidateLicenseRequest",
    ];

    let types = types.iter().map(|t| {
        let ident = syn::parse_str::<syn::Ident>(t).unwrap();
        quote! { #ident }
    });

    let expanded = quote! {
        macro_rules! __private_request_iterator {
            ($m:tt) => {
                $m!(#(#types),*);
            }
        }
    };

    TokenStream::from(expanded)
}
