// Generated from the request catalogue by native/requests/generate_capabilities.py.
use rinf::{RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::rpc::RpcTarget;
use crate::rpc::{Slot, UnsupportedReason};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, SignalPiece)]
pub enum RequestCapability {
    Available,
    Hidden,
    Disabled { reason: CapabilityReason },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, SignalPiece)]
pub enum CapabilityReason {
    NotImplemented,
    OwnerOnly,
    PeerTooOld,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RustSignal)]
pub struct RequestCapabilities {
    pub attach_renderer_request: RequestCapability,
    pub detach_renderer_request: RequestCapability,
    pub list_renderers_request: RequestCapability,
    pub handoff_request: RequestCapability,
    pub get_device_grants_request: RequestCapability,
    pub set_device_grants_request: RequestCapability,
    pub list_controllers_request: RequestCapability,
    pub detach_controller_request: RequestCapability,
    pub list_local_libraries_request: RequestCapability,
    pub set_library_background_sync_request: RequestCapability,
    pub list_peer_libraries_request: RequestCapability,
    pub add_remote_library_request: RequestCapability,
    pub get_sync_settings_request: RequestCapability,
    pub set_sync_settings_request: RequestCapability,
    pub set_download_rule_request: RequestCapability,
    pub remove_download_rule_request: RequestCapability,
    pub test_library_initialized_request: RequestCapability,
    pub close_library_request: RequestCapability,
    pub cancel_task_request: RequestCapability,
    pub scan_audio_library_request: RequestCapability,
    pub analyze_audio_library_request: RequestCapability,
    pub deduplicate_audio_library_request: RequestCapability,
    pub volume_request: RequestCapability,
    pub load_request: RequestCapability,
    pub play_request: RequestCapability,
    pub pause_request: RequestCapability,
    pub next_request: RequestCapability,
    pub previous_request: RequestCapability,
    pub switch_request: RequestCapability,
    pub seek_request: RequestCapability,
    pub remove_request: RequestCapability,
    pub set_playback_mode_request: RequestCapability,
    pub move_playlist_item_request: RequestCapability,
    pub set_realtime_f_f_t_enabled_request: RequestCapability,
    pub set_adaptive_switching_enabled_request: RequestCapability,
    pub set_mix_with_others_request: RequestCapability,
    pub start_visualizer_request: RequestCapability,
    pub stop_visualizer_request: RequestCapability,
    pub resize_visualizer_request: RequestCapability,
    pub set_visualizer_preset_dirs_request: RequestCapability,
    pub set_visualizer_accent_request: RequestCapability,
    pub set_visualizer_visible_request: RequestCapability,
    pub load_visualizer_preset_request: RequestCapability,
    pub next_visualizer_preset_request: RequestCapability,
    pub previous_visualizer_preset_request: RequestCapability,
    pub random_visualizer_preset_request: RequestCapability,
    pub import_visualizer_scene_request: RequestCapability,
    pub remove_visualizer_scene_request: RequestCapability,
    pub set_visualizer_scenes_request: RequestCapability,
    pub set_visualizer_log_request: RequestCapability,
    pub set_visualizer_lab_request: RequestCapability,
    pub visualizer_lab_command_request: RequestCapability,
    pub visualizer_lab_request: RequestCapability,
    pub visualizer_plan_request: RequestCapability,
    pub visualizer_support_request: RequestCapability,
    pub visualizer_status_request: RequestCapability,
    pub list_visualizer_presets_request: RequestCapability,
    pub list_visualizer_credits_request: RequestCapability,
    pub visualizer_texture_shown_request: RequestCapability,
    pub sfx_play_request: RequestCapability,
    pub if_analyze_exists_request: RequestCapability,
    pub get_analyze_count_request: RequestCapability,
    pub fetch_media_files_request: RequestCapability,
    pub fetch_media_file_by_ids_request: RequestCapability,
    pub fetch_parsed_media_file_request: RequestCapability,
    pub search_media_file_summary_request: RequestCapability,
    pub get_media_files_count_request: RequestCapability,
    pub get_lyric_by_track_id_request: RequestCapability,
    pub fetch_collection_group_summary_request: RequestCapability,
    pub fetch_collection_groups_request: RequestCapability,
    pub fetch_collection_by_ids_request: RequestCapability,
    pub search_collection_summary_request: RequestCapability,
    pub get_cover_art_ids_by_mix_queries_request: RequestCapability,
    pub get_primary_color_by_track_id_request: RequestCapability,
    pub fetch_all_playlists_request: RequestCapability,
    pub create_playlist_request: RequestCapability,
    pub create_m3u8_playlist_request: RequestCapability,
    pub update_playlist_request: RequestCapability,
    pub remove_playlist_request: RequestCapability,
    pub add_item_to_playlist_request: RequestCapability,
    pub reorder_playlist_item_position_request: RequestCapability,
    pub fetch_playlist_occurrences_request: RequestCapability,
    pub get_playlist_by_id_request: RequestCapability,
    pub fetch_all_mixes_request: RequestCapability,
    pub create_mix_request: RequestCapability,
    pub update_mix_request: RequestCapability,
    pub remove_mix_request: RequestCapability,
    pub add_item_to_mix_request: RequestCapability,
    pub get_mix_by_id_request: RequestCapability,
    pub mix_query_request: RequestCapability,
    pub fetch_mix_queries_request: RequestCapability,
    pub operate_playback_with_mix_query_request: RequestCapability,
    pub cast_preflight_request: RequestCapability,
    pub cast_load_request: RequestCapability,
    pub refresh_session_request: RequestCapability,
    pub session_snapshot_request: RequestCapability,
    pub set_liked_request: RequestCapability,
    pub get_liked_request: RequestCapability,
    pub complex_query_request: RequestCapability,
    pub search_for_request: RequestCapability,
    pub fetch_directory_tree_request: RequestCapability,
    pub authenticate_single_service_request: RequestCapability,
    pub authenticate_multiple_service_request: RequestCapability,
    pub logout_single_service_request: RequestCapability,
    pub list_log_request: RequestCapability,
    pub clear_log_request: RequestCapability,
    pub remove_log_request: RequestCapability,
    pub system_info_request: RequestCapability,
    pub register_license_request: RequestCapability,
    pub validate_license_request: RequestCapability,
    pub start_broadcast_request: RequestCapability,
    pub stop_broadcast_request: RequestCapability,
    pub start_listening_request: RequestCapability,
    pub stop_listening_request: RequestCapability,
    pub get_discovered_device_request: RequestCapability,
    pub start_server_request: RequestCapability,
    pub stop_server_request: RequestCapability,
    pub list_clients_request: RequestCapability,
    pub get_ssl_certificate_fingerprint_request: RequestCapability,
    pub add_trusted_server_request: RequestCapability,
    pub remove_trusted_client_request: RequestCapability,
    pub update_client_status_request: RequestCapability,
    pub edit_hosts_request: RequestCapability,
    pub remove_trusted_server_request: RequestCapability,
    pub server_availability_test_request: RequestCapability,
    pub register_device_on_server_request: RequestCapability,
    pub check_device_on_server_request: RequestCapability,
    pub connect_request: RequestCapability,
    pub fetch_server_certificate_request: RequestCapability,
    pub fetch_remote_file_request: RequestCapability,
    pub remove_item_from_playlist_request: RequestCapability,
}

impl RequestCapabilities {
    #[cfg(test)]
    pub(crate) fn all_available() -> Self {
        Self {
            attach_renderer_request: RequestCapability::Available,
            detach_renderer_request: RequestCapability::Available,
            list_renderers_request: RequestCapability::Available,
            handoff_request: RequestCapability::Available,
            get_device_grants_request: RequestCapability::Available,
            set_device_grants_request: RequestCapability::Available,
            list_controllers_request: RequestCapability::Available,
            detach_controller_request: RequestCapability::Available,
            list_local_libraries_request: RequestCapability::Available,
            set_library_background_sync_request: RequestCapability::Available,
            list_peer_libraries_request: RequestCapability::Available,
            add_remote_library_request: RequestCapability::Available,
            get_sync_settings_request: RequestCapability::Available,
            set_sync_settings_request: RequestCapability::Available,
            set_download_rule_request: RequestCapability::Available,
            remove_download_rule_request: RequestCapability::Available,
            test_library_initialized_request: RequestCapability::Available,
            close_library_request: RequestCapability::Available,
            cancel_task_request: RequestCapability::Available,
            scan_audio_library_request: RequestCapability::Available,
            analyze_audio_library_request: RequestCapability::Available,
            deduplicate_audio_library_request: RequestCapability::Available,
            volume_request: RequestCapability::Available,
            load_request: RequestCapability::Available,
            play_request: RequestCapability::Available,
            pause_request: RequestCapability::Available,
            next_request: RequestCapability::Available,
            previous_request: RequestCapability::Available,
            switch_request: RequestCapability::Available,
            seek_request: RequestCapability::Available,
            remove_request: RequestCapability::Available,
            set_playback_mode_request: RequestCapability::Available,
            move_playlist_item_request: RequestCapability::Available,
            set_realtime_f_f_t_enabled_request: RequestCapability::Available,
            set_adaptive_switching_enabled_request: RequestCapability::Available,
            set_mix_with_others_request: RequestCapability::Available,
            start_visualizer_request: RequestCapability::Available,
            stop_visualizer_request: RequestCapability::Available,
            resize_visualizer_request: RequestCapability::Available,
            set_visualizer_preset_dirs_request: RequestCapability::Available,
            set_visualizer_accent_request: RequestCapability::Available,
            set_visualizer_visible_request: RequestCapability::Available,
            load_visualizer_preset_request: RequestCapability::Available,
            next_visualizer_preset_request: RequestCapability::Available,
            previous_visualizer_preset_request: RequestCapability::Available,
            random_visualizer_preset_request: RequestCapability::Available,
            import_visualizer_scene_request: RequestCapability::Available,
            remove_visualizer_scene_request: RequestCapability::Available,
            set_visualizer_scenes_request: RequestCapability::Available,
            set_visualizer_log_request: RequestCapability::Available,
            set_visualizer_lab_request: RequestCapability::Available,
            visualizer_lab_command_request: RequestCapability::Available,
            visualizer_lab_request: RequestCapability::Available,
            visualizer_plan_request: RequestCapability::Available,
            visualizer_support_request: RequestCapability::Available,
            visualizer_status_request: RequestCapability::Available,
            list_visualizer_presets_request: RequestCapability::Available,
            list_visualizer_credits_request: RequestCapability::Available,
            visualizer_texture_shown_request: RequestCapability::Available,
            sfx_play_request: RequestCapability::Available,
            if_analyze_exists_request: RequestCapability::Available,
            get_analyze_count_request: RequestCapability::Available,
            fetch_media_files_request: RequestCapability::Available,
            fetch_media_file_by_ids_request: RequestCapability::Available,
            fetch_parsed_media_file_request: RequestCapability::Available,
            search_media_file_summary_request: RequestCapability::Available,
            get_media_files_count_request: RequestCapability::Available,
            get_lyric_by_track_id_request: RequestCapability::Available,
            fetch_collection_group_summary_request: RequestCapability::Available,
            fetch_collection_groups_request: RequestCapability::Available,
            fetch_collection_by_ids_request: RequestCapability::Available,
            search_collection_summary_request: RequestCapability::Available,
            get_cover_art_ids_by_mix_queries_request: RequestCapability::Available,
            get_primary_color_by_track_id_request: RequestCapability::Available,
            fetch_all_playlists_request: RequestCapability::Available,
            create_playlist_request: RequestCapability::Available,
            create_m3u8_playlist_request: RequestCapability::Available,
            update_playlist_request: RequestCapability::Available,
            remove_playlist_request: RequestCapability::Available,
            add_item_to_playlist_request: RequestCapability::Available,
            reorder_playlist_item_position_request: RequestCapability::Available,
            fetch_playlist_occurrences_request: RequestCapability::Available,
            get_playlist_by_id_request: RequestCapability::Available,
            fetch_all_mixes_request: RequestCapability::Available,
            create_mix_request: RequestCapability::Available,
            update_mix_request: RequestCapability::Available,
            remove_mix_request: RequestCapability::Available,
            add_item_to_mix_request: RequestCapability::Available,
            get_mix_by_id_request: RequestCapability::Available,
            mix_query_request: RequestCapability::Available,
            fetch_mix_queries_request: RequestCapability::Available,
            operate_playback_with_mix_query_request: RequestCapability::Available,
            cast_preflight_request: RequestCapability::Available,
            cast_load_request: RequestCapability::Available,
            refresh_session_request: RequestCapability::Available,
            session_snapshot_request: RequestCapability::Available,
            set_liked_request: RequestCapability::Available,
            get_liked_request: RequestCapability::Available,
            complex_query_request: RequestCapability::Available,
            search_for_request: RequestCapability::Available,
            fetch_directory_tree_request: RequestCapability::Available,
            authenticate_single_service_request: RequestCapability::Available,
            authenticate_multiple_service_request: RequestCapability::Available,
            logout_single_service_request: RequestCapability::Available,
            list_log_request: RequestCapability::Available,
            clear_log_request: RequestCapability::Available,
            remove_log_request: RequestCapability::Available,
            system_info_request: RequestCapability::Available,
            register_license_request: RequestCapability::Available,
            validate_license_request: RequestCapability::Available,
            start_broadcast_request: RequestCapability::Available,
            stop_broadcast_request: RequestCapability::Available,
            start_listening_request: RequestCapability::Available,
            stop_listening_request: RequestCapability::Available,
            get_discovered_device_request: RequestCapability::Available,
            start_server_request: RequestCapability::Available,
            stop_server_request: RequestCapability::Available,
            list_clients_request: RequestCapability::Available,
            get_ssl_certificate_fingerprint_request: RequestCapability::Available,
            add_trusted_server_request: RequestCapability::Available,
            remove_trusted_client_request: RequestCapability::Available,
            update_client_status_request: RequestCapability::Available,
            edit_hosts_request: RequestCapability::Available,
            remove_trusted_server_request: RequestCapability::Available,
            server_availability_test_request: RequestCapability::Available,
            register_device_on_server_request: RequestCapability::Available,
            check_device_on_server_request: RequestCapability::Available,
            connect_request: RequestCapability::Available,
            fetch_server_certificate_request: RequestCapability::Available,
            fetch_remote_file_request: RequestCapability::Available,
            remove_item_from_playlist_request: RequestCapability::Available,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_target(target: &RpcTarget) -> Self {
        Self {
            attach_renderer_request: target.capability::<crate::messages::AttachRendererRequest>(),
            detach_renderer_request: target.capability::<crate::messages::DetachRendererRequest>(),
            list_renderers_request: target.capability::<crate::messages::ListRenderersRequest>(),
            handoff_request: target.capability::<crate::messages::HandoffRequest>(),
            get_device_grants_request: target
                .capability::<crate::messages::GetDeviceGrantsRequest>(),
            set_device_grants_request: target
                .capability::<crate::messages::SetDeviceGrantsRequest>(),
            list_controllers_request: target
                .capability::<crate::messages::ListControllersRequest>(),
            detach_controller_request: target
                .capability::<crate::messages::DetachControllerRequest>(),
            list_local_libraries_request: target
                .capability::<crate::messages::ListLocalLibrariesRequest>(),
            set_library_background_sync_request: target
                .capability::<crate::messages::SetLibraryBackgroundSyncRequest>(
            ),
            list_peer_libraries_request: target
                .capability::<crate::messages::ListPeerLibrariesRequest>(),
            add_remote_library_request: target
                .capability::<crate::messages::AddRemoteLibraryRequest>(),
            get_sync_settings_request: target
                .capability::<crate::messages::GetSyncSettingsRequest>(),
            set_sync_settings_request: target
                .capability::<crate::messages::SetSyncSettingsRequest>(),
            set_download_rule_request: target
                .capability::<crate::messages::SetDownloadRuleRequest>(),
            remove_download_rule_request: target
                .capability::<crate::messages::RemoveDownloadRuleRequest>(),
            test_library_initialized_request: target
                .capability::<crate::messages::TestLibraryInitializedRequest>(),
            close_library_request: target.capability::<crate::messages::CloseLibraryRequest>(),
            cancel_task_request: target.capability::<crate::messages::CancelTaskRequest>(),
            scan_audio_library_request: target
                .capability::<crate::messages::ScanAudioLibraryRequest>(),
            analyze_audio_library_request: target
                .capability::<crate::messages::AnalyzeAudioLibraryRequest>(),
            deduplicate_audio_library_request: target
                .capability::<crate::messages::DeduplicateAudioLibraryRequest>(
            ),
            volume_request: target.capability::<crate::messages::VolumeRequest>(),
            load_request: target.capability::<crate::messages::LoadRequest>(),
            play_request: target.capability::<crate::messages::PlayRequest>(),
            pause_request: target.capability::<crate::messages::PauseRequest>(),
            next_request: target.capability::<crate::messages::NextRequest>(),
            previous_request: target.capability::<crate::messages::PreviousRequest>(),
            switch_request: target.capability::<crate::messages::SwitchRequest>(),
            seek_request: target.capability::<crate::messages::SeekRequest>(),
            remove_request: target.capability::<crate::messages::RemoveRequest>(),
            set_playback_mode_request: target
                .capability::<crate::messages::SetPlaybackModeRequest>(),
            move_playlist_item_request: target
                .capability::<crate::messages::MovePlaylistItemRequest>(),
            set_realtime_f_f_t_enabled_request: target
                .capability::<crate::messages::SetRealtimeFFTEnabledRequest>(),
            set_adaptive_switching_enabled_request:
                target.capability::<crate::messages::SetAdaptiveSwitchingEnabledRequest>(),
            set_mix_with_others_request: target
                .capability::<crate::messages::SetMixWithOthersRequest>(),
            start_visualizer_request: target
                .capability::<crate::messages::StartVisualizerRequest>(),
            stop_visualizer_request: target.capability::<crate::messages::StopVisualizerRequest>(),
            resize_visualizer_request: target
                .capability::<crate::messages::ResizeVisualizerRequest>(),
            set_visualizer_preset_dirs_request: target
                .capability::<crate::messages::SetVisualizerPresetDirsRequest>(
            ),
            set_visualizer_accent_request: target
                .capability::<crate::messages::SetVisualizerAccentRequest>(),
            set_visualizer_visible_request: target
                .capability::<crate::messages::SetVisualizerVisibleRequest>(),
            load_visualizer_preset_request: target
                .capability::<crate::messages::LoadVisualizerPresetRequest>(),
            next_visualizer_preset_request: target
                .capability::<crate::messages::NextVisualizerPresetRequest>(),
            previous_visualizer_preset_request: target
                .capability::<crate::messages::PreviousVisualizerPresetRequest>(
            ),
            random_visualizer_preset_request: target
                .capability::<crate::messages::RandomVisualizerPresetRequest>(),
            import_visualizer_scene_request: target
                .capability::<crate::messages::ImportVisualizerSceneRequest>(),
            remove_visualizer_scene_request: target
                .capability::<crate::messages::RemoveVisualizerSceneRequest>(),
            set_visualizer_scenes_request: target
                .capability::<crate::messages::SetVisualizerScenesRequest>(),
            set_visualizer_log_request: target
                .capability::<crate::messages::SetVisualizerLogRequest>(),
            set_visualizer_lab_request: target
                .capability::<crate::messages::SetVisualizerLabRequest>(),
            visualizer_lab_command_request: target
                .capability::<crate::messages::VisualizerLabCommandRequest>(),
            visualizer_lab_request: target.capability::<crate::messages::VisualizerLabRequest>(),
            visualizer_plan_request: target.capability::<crate::messages::VisualizerPlanRequest>(),
            visualizer_support_request: target
                .capability::<crate::messages::VisualizerSupportRequest>(),
            visualizer_status_request: target
                .capability::<crate::messages::VisualizerStatusRequest>(),
            list_visualizer_presets_request: target
                .capability::<crate::messages::ListVisualizerPresetsRequest>(),
            list_visualizer_credits_request: target
                .capability::<crate::messages::ListVisualizerCreditsRequest>(),
            visualizer_texture_shown_request: target
                .capability::<crate::messages::VisualizerTextureShownRequest>(),
            sfx_play_request: target.capability::<crate::messages::SfxPlayRequest>(),
            if_analyze_exists_request: target
                .capability::<crate::messages::IfAnalyzeExistsRequest>(),
            get_analyze_count_request: target
                .capability::<crate::messages::GetAnalyzeCountRequest>(),
            fetch_media_files_request: target
                .capability::<crate::messages::FetchMediaFilesRequest>(),
            fetch_media_file_by_ids_request: target
                .capability::<crate::messages::FetchMediaFileByIdsRequest>(),
            fetch_parsed_media_file_request: target
                .capability::<crate::messages::FetchParsedMediaFileRequest>(),
            search_media_file_summary_request: target
                .capability::<crate::messages::SearchMediaFileSummaryRequest>(),
            get_media_files_count_request: target
                .capability::<crate::messages::GetMediaFilesCountRequest>(),
            get_lyric_by_track_id_request: target
                .capability::<crate::messages::GetLyricByTrackIdRequest>(),
            fetch_collection_group_summary_request:
                target.capability::<crate::messages::FetchCollectionGroupSummaryRequest>(),
            fetch_collection_groups_request: target
                .capability::<crate::messages::FetchCollectionGroupsRequest>(),
            fetch_collection_by_ids_request: target
                .capability::<crate::messages::FetchCollectionByIdsRequest>(),
            search_collection_summary_request: target
                .capability::<crate::messages::SearchCollectionSummaryRequest>(
            ),
            get_cover_art_ids_by_mix_queries_request:
                target.capability::<crate::messages::GetCoverArtIdsByMixQueriesRequest>(),
            get_primary_color_by_track_id_request: target
                .capability::<crate::messages::GetPrimaryColorByTrackIdRequest>(
            ),
            fetch_all_playlists_request: target
                .capability::<crate::messages::FetchAllPlaylistsRequest>(),
            create_playlist_request: target.capability::<crate::messages::CreatePlaylistRequest>(),
            create_m3u8_playlist_request: target
                .capability::<crate::messages::CreateM3u8PlaylistRequest>(),
            update_playlist_request: target.capability::<crate::messages::UpdatePlaylistRequest>(),
            remove_playlist_request: target.capability::<crate::messages::RemovePlaylistRequest>(),
            add_item_to_playlist_request: target
                .capability::<crate::messages::AddItemToPlaylistRequest>(),
            reorder_playlist_item_position_request:
                target.capability::<crate::messages::ReorderPlaylistItemPositionRequest>(),
            fetch_playlist_occurrences_request: target
                .capability::<crate::messages::FetchPlaylistOccurrencesRequest>(
            ),
            get_playlist_by_id_request: target
                .capability::<crate::messages::GetPlaylistByIdRequest>(),
            fetch_all_mixes_request: target.capability::<crate::messages::FetchAllMixesRequest>(),
            create_mix_request: target.capability::<crate::messages::CreateMixRequest>(),
            update_mix_request: target.capability::<crate::messages::UpdateMixRequest>(),
            remove_mix_request: target.capability::<crate::messages::RemoveMixRequest>(),
            add_item_to_mix_request: target.capability::<crate::messages::AddItemToMixRequest>(),
            get_mix_by_id_request: target.capability::<crate::messages::GetMixByIdRequest>(),
            mix_query_request: target.capability::<crate::messages::MixQueryRequest>(),
            fetch_mix_queries_request: target
                .capability::<crate::messages::FetchMixQueriesRequest>(),
            operate_playback_with_mix_query_request:
                target.capability::<crate::messages::OperatePlaybackWithMixQueryRequest>(),
            cast_preflight_request: target.capability::<crate::messages::CastPreflightRequest>(),
            cast_load_request: target.capability::<crate::messages::CastLoadRequest>(),
            refresh_session_request: target.capability::<crate::messages::RefreshSessionRequest>(),
            session_snapshot_request: target
                .capability::<crate::messages::SessionSnapshotRequest>(),
            set_liked_request: target.capability::<crate::messages::SetLikedRequest>(),
            get_liked_request: target.capability::<crate::messages::GetLikedRequest>(),
            complex_query_request: target.capability::<crate::messages::ComplexQueryRequest>(),
            search_for_request: target.capability::<crate::messages::SearchForRequest>(),
            fetch_directory_tree_request: target
                .capability::<crate::messages::FetchDirectoryTreeRequest>(),
            authenticate_single_service_request: target
                .capability::<crate::messages::AuthenticateSingleServiceRequest>(
            ),
            authenticate_multiple_service_request:
                target.capability::<crate::messages::AuthenticateMultipleServiceRequest>(),
            logout_single_service_request: target
                .capability::<crate::messages::LogoutSingleServiceRequest>(),
            list_log_request: target.capability::<crate::messages::ListLogRequest>(),
            clear_log_request: target.capability::<crate::messages::ClearLogRequest>(),
            remove_log_request: target.capability::<crate::messages::RemoveLogRequest>(),
            system_info_request: target.capability::<crate::messages::SystemInfoRequest>(),
            register_license_request: target
                .capability::<crate::messages::RegisterLicenseRequest>(),
            validate_license_request: target
                .capability::<crate::messages::ValidateLicenseRequest>(),
            start_broadcast_request: target.capability::<crate::messages::StartBroadcastRequest>(),
            stop_broadcast_request: target.capability::<crate::messages::StopBroadcastRequest>(),
            start_listening_request: target.capability::<crate::messages::StartListeningRequest>(),
            stop_listening_request: target.capability::<crate::messages::StopListeningRequest>(),
            get_discovered_device_request: target
                .capability::<crate::messages::GetDiscoveredDeviceRequest>(),
            start_server_request: target.capability::<crate::messages::StartServerRequest>(),
            stop_server_request: target.capability::<crate::messages::StopServerRequest>(),
            list_clients_request: target.capability::<crate::messages::ListClientsRequest>(),
            get_ssl_certificate_fingerprint_request:
                target.capability::<crate::messages::GetSslCertificateFingerprintRequest>(),
            add_trusted_server_request: target
                .capability::<crate::messages::AddTrustedServerRequest>(),
            remove_trusted_client_request: target
                .capability::<crate::messages::RemoveTrustedClientRequest>(),
            update_client_status_request: target
                .capability::<crate::messages::UpdateClientStatusRequest>(),
            edit_hosts_request: target.capability::<crate::messages::EditHostsRequest>(),
            remove_trusted_server_request: target
                .capability::<crate::messages::RemoveTrustedServerRequest>(),
            server_availability_test_request: target
                .capability::<crate::messages::ServerAvailabilityTestRequest>(),
            register_device_on_server_request: target
                .capability::<crate::messages::RegisterDeviceOnServerRequest>(),
            check_device_on_server_request: target
                .capability::<crate::messages::CheckDeviceOnServerRequest>(),
            connect_request: target.capability::<crate::messages::ConnectRequest>(),
            fetch_server_certificate_request: target
                .capability::<crate::messages::FetchServerCertificateRequest>(),
            fetch_remote_file_request: target
                .capability::<crate::messages::FetchRemoteFileRequest>(),
            remove_item_from_playlist_request: target
                .capability::<crate::messages::RemoveItemFromPlaylistRequest>(),
        }
    }
}

impl From<Slot> for RequestCapability {
    fn from(slot: Slot) -> Self {
        match slot {
            Slot::Native | Slot::Forward | Slot::Translate => Self::Available,
            Slot::Unsupported {
                reason: UnsupportedReason::NotImplemented,
            } => Self::Hidden,
            Slot::Unsupported {
                reason: UnsupportedReason::OwnerOnly,
            } => Self::Disabled {
                reason: CapabilityReason::OwnerOnly,
            },
            Slot::Unsupported {
                reason: UnsupportedReason::PeerTooOld,
            } => Self::Disabled {
                reason: CapabilityReason::PeerTooOld,
            },
        }
    }
}

crate::implement_rinf_rust_signal_trait!(RequestCapabilities);
