#[macro_export]
macro_rules! implement_rinf_rust_signal_trait {
    ($($t:ty),*) => {
        $(
            impl $crate::RinfRustSignal for $t {
                fn as_any(&self) -> &dyn std::any::Any {
                    self
                }

                fn name(&self) -> String {
                    stringify!($t).to_string()
                }

                fn encode_to_vec(&self) -> anyhow::Result<Vec<u8>> {
                    rinf::serialize(self).map_err(anyhow::Error::from)
                }

                fn send_to_dart(&self) {
                    self.send_signal_to_dart();
                }
            }
        )*
    };
}

#[macro_export]
macro_rules! declare_remote_events {
    ($($t:ty => $scope:ident),+ $(,)?) => {
        pub const REMOTE_EVENT_DECLARATIONS: &[$crate::RemoteEventDeclaration] = &[
            $(
                $crate::RemoteEventDeclaration {
                    name: stringify!($t),
                    type_id: TypeId::of::<$t>(),
                    scope: $crate::RemoteEventScope::$scope,
                    decode: |payload| {
                        Ok(Box::new(rinf::deserialize::<$t>(payload)?) as Box<dyn $crate::RinfRustSignal>)
                    },
                },
            )+
        ];
    };
}

use std::{future::Future, sync::Arc, time::Duration};

use anyhow::{Error, Result};
use futures::{StreamExt, future::BoxFuture, stream::FuturesUnordered};
use requests::define_request_types;
use rinf::{DartSignal, DartSignalPack, RustSignal, SignalReceiver};
use uuid::Uuid;

use crate::backends::local::gui_request::{GuiRequest, listen_gui};
use crate::node::{NodeContext, NodeSlot};
use crate::request_error::{ExpectedRequestError, classify, request_failure_cause};
#[cfg(test)]
use crate::request_owner::Implementation;
use crate::rpc::{RequestTranslationFuture, Slot, TargetRequest, UnsupportedReason, narrow_slot};
use crate::server::server_request::serve_remote;
use crate::utils::{GlobalParams, RinfRustSignal};
use crate::{RequestCallId, ResponseCallId, Session, TypedRequest};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use crate::messages::*;
define_request_types!();

#[cfg(test)]
mod node_request_loop_tests {
    use std::{future::Future, pin::Pin, sync::Arc};

    use anyhow::{Result, anyhow};
    use rinf::{DartSignalPack, signal_channel};
    use tokio::{
        sync::{Notify, mpsc},
        time::{Duration, timeout},
    };

    use super::{
        NodeRequestHandlers, NodeRequestReceivers, node_request_loop_with_receivers,
        standalone_request_names,
    };
    use crate::messages::*;
    use crate::node::NodeSlot;
    use crate::utils::RinfRustSignal;

    struct TestHandlers {
        list_started: Arc<Notify>,
        release_list: Arc<Notify>,
    }

    macro_rules! error_handler {
        ($method:ident, $request:ty, $response:ty) => {
            fn $method(
                &self,
                _: &NodeSlot,
                _: &$request,
            ) -> impl Future<Output = Result<Option<$response>>> + Send {
                async { Err(anyhow!("test route")) }
            }
        };
    }

    #[allow(clippy::manual_async_fn)]
    impl NodeRequestHandlers for TestHandlers {
        error_handler!(
            handle_get_device_grants_request,
            GetDeviceGrantsRequest,
            GetDeviceGrantsResponse
        );
        error_handler!(
            handle_set_device_grants_request,
            SetDeviceGrantsRequest,
            SetDeviceGrantsResponse
        );
        error_handler!(
            handle_set_library_background_sync_request,
            SetLibraryBackgroundSyncRequest,
            SetLibraryBackgroundSyncResponse
        );
        error_handler!(
            handle_list_peer_libraries_request,
            ListPeerLibrariesRequest,
            ListPeerLibrariesResponse
        );
        error_handler!(
            handle_add_remote_library_request,
            AddRemoteLibraryRequest,
            AddRemoteLibraryResponse
        );
        error_handler!(
            handle_register_license_request,
            RegisterLicenseRequest,
            RegisterLicenseResponse
        );
        error_handler!(
            handle_validate_license_request,
            ValidateLicenseRequest,
            ValidateLicenseResponse
        );
        error_handler!(
            handle_get_discovered_device_request,
            GetDiscoveredDeviceRequest,
            GetDiscoveredDeviceResponse
        );
        error_handler!(
            handle_list_clients_request,
            ListClientsRequest,
            ListClientsResponse
        );
        error_handler!(
            handle_get_ssl_certificate_fingerprint_request,
            GetSslCertificateFingerprintRequest,
            GetSslCertificateFingerprintResponse
        );
        error_handler!(
            handle_visualizer_support_request,
            VisualizerSupportRequest,
            VisualizerSupportResponse
        );
        error_handler!(
            handle_list_visualizer_presets_request,
            ListVisualizerPresetsRequest,
            ListVisualizerPresetsResponse
        );
        error_handler!(
            handle_test_library_initialized_request,
            TestLibraryInitializedRequest,
            TestLibraryInitializedResponse
        );
        error_handler!(
            handle_set_mix_with_others_request,
            SetMixWithOthersRequest,
            ()
        );
        error_handler!(
            handle_get_sync_settings_request,
            GetSyncSettingsRequest,
            GetSyncSettingsResponse
        );
        error_handler!(
            handle_set_sync_settings_request,
            SetSyncSettingsRequest,
            SetSyncSettingsResponse
        );
        error_handler!(
            handle_system_info_request,
            SystemInfoRequest,
            SystemInfoResponse
        );
        error_handler!(
            handle_add_trusted_server_request,
            AddTrustedServerRequest,
            AddTrustedServerResponse
        );
        error_handler!(
            handle_remove_trusted_client_request,
            RemoveTrustedClientRequest,
            RemoveTrustedClientResponse
        );
        error_handler!(
            handle_update_client_status_request,
            UpdateClientStatusRequest,
            UpdateClientStatusResponse
        );
        error_handler!(
            handle_remove_trusted_server_request,
            RemoveTrustedServerRequest,
            RemoveTrustedServerResponse
        );
        error_handler!(
            handle_server_availability_test_request,
            ServerAvailabilityTestRequest,
            ServerAvailabilityTestResponse
        );
        error_handler!(
            handle_register_device_on_server_request,
            RegisterDeviceOnServerRequest,
            RegisterDeviceOnServerResponse
        );
        error_handler!(
            handle_check_device_on_server_request,
            CheckDeviceOnServerRequest,
            CheckDeviceOnServerResponse
        );
        error_handler!(handle_connect_request, ConnectRequest, ConnectResponse);
        error_handler!(
            handle_fetch_server_certificate_request,
            FetchServerCertificateRequest,
            FetchServerCertificateResponse
        );
        error_handler!(
            handle_start_server_request,
            StartServerRequest,
            StartServerResponse
        );
        error_handler!(
            handle_stop_server_request,
            StopServerRequest,
            StopServerResponse
        );
        error_handler!(handle_start_broadcast_request, StartBroadcastRequest, ());
        error_handler!(handle_stop_broadcast_request, StopBroadcastRequest, ());
        error_handler!(handle_start_listening_request, StartListeningRequest, ());
        error_handler!(handle_stop_listening_request, StopListeningRequest, ());
        error_handler!(
            handle_set_visualizer_preset_dirs_request,
            SetVisualizerPresetDirsRequest,
            ()
        );
        error_handler!(
            handle_set_visualizer_scenes_request,
            SetVisualizerScenesRequest,
            ()
        );

        fn handle_list_controllers_request(
            &self,
            _: &NodeSlot,
            _: &ListControllersRequest,
        ) -> impl Future<Output = Result<Option<ListControllersResponse>>> + Send {
            async {
                Ok(Some(ListControllersResponse {
                    controllers: Vec::new(),
                    call_id: None,
                }))
            }
        }

        fn handle_list_local_libraries_request(
            &self,
            _: &NodeSlot,
            _: &ListLocalLibrariesRequest,
        ) -> impl Future<Output = Result<Option<ListLocalLibrariesResponse>>> + Send {
            async {
                Ok(Some(ListLocalLibrariesResponse {
                    libraries: Vec::new(),
                    call_id: None,
                }))
            }
        }

        fn handle_list_library_records_request(
            &self,
            _: &NodeSlot,
            _: &ListLibraryRecordsRequest,
        ) -> impl Future<Output = Result<Option<ListLibraryRecordsResponse>>> + Send {
            let list_started = Arc::clone(&self.list_started);
            let release_list = Arc::clone(&self.release_list);
            async move {
                let release = release_list.notified();
                list_started.notify_one();
                release.await;
                Ok(Some(ListLibraryRecordsResponse {
                    call_id: None,
                    records: Vec::new(),
                    invalid_records: Vec::new(),
                    open: None,
                    last_open_record: None,
                }))
            }
        }

        fn handle_remove_library_record_request(
            &self,
            _: &NodeSlot,
            _: &RemoveLibraryRecordRequest,
        ) -> impl Future<Output = Result<Option<RemoveLibraryRecordResponse>>> + Send {
            async {
                Ok(Some(RemoveLibraryRecordResponse {
                    call_id: None,
                    outcome: RemoveLibraryRecordOutcome::Removed,
                }))
            }
        }

        fn handle_register_local_library_request(
            &self,
            _: &NodeSlot,
            _: &RegisterLocalLibraryRequest,
        ) -> impl Future<Output = Result<Option<RegisterLibraryResponse>>> + Send {
            async { Ok(Some(register_response())) }
        }

        fn handle_register_remote_control_request(
            &self,
            _: &NodeSlot,
            _: &RegisterRemoteControlRequest,
        ) -> impl Future<Output = Result<Option<RegisterLibraryResponse>>> + Send {
            async { Ok(Some(register_response())) }
        }

        fn handle_register_subsonic_library_request(
            &self,
            _: &NodeSlot,
            _: &RegisterSubsonicLibraryRequest,
        ) -> impl Future<Output = Result<Option<RegisterLibraryResponse>>> + Send {
            async { Ok(Some(register_response())) }
        }
        fn handle_replace_subsonic_credential_request(
            &self,
            _: &NodeSlot,
            request: &ReplaceSubsonicCredentialRequest,
        ) -> impl Future<Output = Result<Option<ReplaceSubsonicCredentialResponse>>> + Send
        {
            let response = ReplaceSubsonicCredentialResponse {
                result: ReplaceSubsonicCredentialResult::Updated,
                call_id: request.call_id,
            };
            async move { Ok(Some(response)) }
        }
        fn handle_register_open_dal_library_request(
            &self,
            _: &NodeSlot,
            _: &RegisterOpenDalLibraryRequest,
        ) -> impl Future<Output = Result<Option<RegisterLibraryResponse>>> + Send {
            async { Ok(Some(register_response())) }
        }

        fn handle_open_library_request(
            &self,
            _: &NodeSlot,
            request: &OpenLibraryRequest,
        ) -> impl Future<Output = Result<Option<OpenLibraryResponse>>> + Send {
            let response = OpenLibraryResponse {
                call_id: request.call_id,
                record_id: request.record_id.clone(),
                generation: 1,
                result: OpenLibraryResult::Opened,
            };
            async move { Ok(Some(response)) }
        }
    }

    fn register_response() -> RegisterLibraryResponse {
        RegisterLibraryResponse {
            call_id: None,
            record: None,
            outcome: RegisterLibraryOutcome::Registered,
            detail: None,
        }
    }

    async fn response_or_loop_exit<F>(
        loop_future: &mut Pin<Box<F>>,
        output: &mut mpsc::UnboundedReceiver<(String, Vec<u8>)>,
    ) -> (String, Vec<u8>)
    where
        F: Future<Output = ()>,
    {
        timeout(Duration::from_secs(1), async {
            tokio::select! {
                response = output.recv() => response.expect("receiver loop emitted no response"),
                () = loop_future => panic!("node request receiver loop exited"),
            }
        })
        .await
        .expect("node request receiver did not respond before timeout")
    }

    #[tokio::test]
    async fn generated_node_loop_receives_every_catalogue_request_concurrently() {
        let (controllers_tx, controllers_rx) = signal_channel();
        let (device_grants_tx, device_grants_rx) = signal_channel();
        let (set_device_grants_tx, set_device_grants_rx) = signal_channel();
        let (list_tx, list_rx) = signal_channel();
        let (background_sync_tx, background_sync_rx) = signal_channel();
        let (local_libraries_tx, local_libraries_rx) = signal_channel();
        let (peer_libraries_tx, peer_libraries_rx) = signal_channel();
        let (add_remote_tx, add_remote_rx) = signal_channel();
        let (register_license_tx, register_license_rx) = signal_channel();
        let (validate_license_tx, validate_license_rx) = signal_channel();
        let (discovered_tx, discovered_rx) = signal_channel();
        let (clients_tx, clients_rx) = signal_channel();
        let (start_server_tx, start_server_rx) = signal_channel();
        let (stop_server_tx, stop_server_rx) = signal_channel();
        let (library_test_tx, library_test_rx) = signal_channel();
        let (mix_with_others_tx, mix_with_others_rx) = signal_channel();
        let (sync_settings_tx, sync_settings_rx) = signal_channel();
        let (set_sync_settings_tx, set_sync_settings_rx) = signal_channel();
        let (system_info_tx, system_info_rx) = signal_channel();
        let (add_trusted_server_tx, add_trusted_server_rx) = signal_channel();
        let (remove_trusted_client_tx, remove_trusted_client_rx) = signal_channel();
        let (update_client_status_tx, update_client_status_rx) = signal_channel();
        let (remove_trusted_server_tx, remove_trusted_server_rx) = signal_channel();
        let (availability_tx, availability_rx) = signal_channel();
        let (register_device_tx, register_device_rx) = signal_channel();
        let (check_device_tx, check_device_rx) = signal_channel();
        let (connect_tx, connect_rx) = signal_channel();
        let (fetch_certificate_tx, fetch_certificate_rx) = signal_channel();
        let (fingerprint_tx, fingerprint_rx) = signal_channel();
        let (visualizer_support_tx, visualizer_support_rx) = signal_channel();
        let (visualizer_presets_tx, visualizer_presets_rx) = signal_channel();
        let (broadcast_tx, broadcast_rx) = signal_channel();
        let (stop_broadcast_tx, stop_broadcast_rx) = signal_channel();
        let (listening_tx, listening_rx) = signal_channel();
        let (stop_listening_tx, stop_listening_rx) = signal_channel();
        let (visualizer_dirs_tx, visualizer_dirs_rx) = signal_channel();
        let (visualizer_scenes_tx, visualizer_scenes_rx) = signal_channel();
        let (remove_tx, remove_rx) = signal_channel();
        let (local_tx, local_rx) = signal_channel();
        let (remote_tx, remote_rx) = signal_channel();
        let (subsonic_tx, subsonic_rx) = signal_channel();
        let (_replace_subsonic_tx, replace_subsonic_rx) = signal_channel();
        let (opendal_tx, opendal_rx) = signal_channel();
        let (open_tx, open_rx) = signal_channel();
        let (output_tx, mut output_rx) = mpsc::unbounded_channel();
        let emit = move |signal: &dyn RinfRustSignal| {
            output_tx
                .send((signal.name(), signal.encode_to_vec().unwrap()))
                .unwrap();
        };
        let node = NodeSlot::new();
        let list_started = Arc::new(Notify::new());
        let release_list = Arc::new(Notify::new());
        let mut loop_future = Box::pin(node_request_loop_with_receivers(
            &node,
            TestHandlers {
                list_started: Arc::clone(&list_started),
                release_list: Arc::clone(&release_list),
            },
            emit,
            NodeRequestReceivers {
                list_controllers_request_receiver: controllers_rx,
                get_device_grants_request_receiver: device_grants_rx,
                set_device_grants_request_receiver: set_device_grants_rx,
                list_library_records_request_receiver: list_rx,
                set_library_background_sync_request_receiver: background_sync_rx,
                list_local_libraries_request_receiver: local_libraries_rx,
                list_peer_libraries_request_receiver: peer_libraries_rx,
                add_remote_library_request_receiver: add_remote_rx,
                register_license_request_receiver: register_license_rx,
                validate_license_request_receiver: validate_license_rx,
                get_discovered_device_request_receiver: discovered_rx,
                list_clients_request_receiver: clients_rx,
                start_server_request_receiver: start_server_rx,
                stop_server_request_receiver: stop_server_rx,
                test_library_initialized_request_receiver: library_test_rx,
                set_mix_with_others_request_receiver: mix_with_others_rx,
                get_sync_settings_request_receiver: sync_settings_rx,
                set_sync_settings_request_receiver: set_sync_settings_rx,
                system_info_request_receiver: system_info_rx,
                add_trusted_server_request_receiver: add_trusted_server_rx,
                remove_trusted_client_request_receiver: remove_trusted_client_rx,
                update_client_status_request_receiver: update_client_status_rx,
                remove_trusted_server_request_receiver: remove_trusted_server_rx,
                server_availability_test_request_receiver: availability_rx,
                register_device_on_server_request_receiver: register_device_rx,
                check_device_on_server_request_receiver: check_device_rx,
                connect_request_receiver: connect_rx,
                fetch_server_certificate_request_receiver: fetch_certificate_rx,
                get_ssl_certificate_fingerprint_request_receiver: fingerprint_rx,
                visualizer_support_request_receiver: visualizer_support_rx,
                list_visualizer_presets_request_receiver: visualizer_presets_rx,
                start_broadcast_request_receiver: broadcast_rx,
                stop_broadcast_request_receiver: stop_broadcast_rx,
                start_listening_request_receiver: listening_rx,
                stop_listening_request_receiver: stop_listening_rx,
                set_visualizer_preset_dirs_request_receiver: visualizer_dirs_rx,
                set_visualizer_scenes_request_receiver: visualizer_scenes_rx,
                remove_library_record_request_receiver: remove_rx,
                register_local_library_request_receiver: local_rx,
                register_remote_control_request_receiver: remote_rx,
                register_subsonic_library_request_receiver: subsonic_rx,
                replace_subsonic_credential_request_receiver: replace_subsonic_rx,
                register_open_dal_library_request_receiver: opendal_rx,
                open_library_request_receiver: open_rx,
            },
        ));

        local_libraries_tx.send(DartSignalPack {
            message: ListLocalLibrariesRequest { call_id: None },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "ListLocalLibrariesResponse"
        );

        device_grants_tx.send(DartSignalPack {
            message: GetDeviceGrantsRequest {
                fingerprint: String::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        start_server_tx.send(DartSignalPack {
            message: StartServerRequest {
                interface: String::new(),
                alias: String::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        stop_server_tx.send(DartSignalPack {
            message: StopServerRequest { call_id: None },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        set_device_grants_tx.send(DartSignalPack {
            message: SetDeviceGrantsRequest {
                fingerprint: String::new(),
                grants: DeviceGrantsChange::Preset(DevicePreset::Guest),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        background_sync_tx.send(DartSignalPack {
            message: SetLibraryBackgroundSyncRequest {
                library_id: String::new(),
                enabled: false,
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        peer_libraries_tx.send(DartSignalPack {
            message: ListPeerLibrariesRequest {
                fingerprint: String::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        add_remote_tx.send(DartSignalPack {
            message: AddRemoteLibraryRequest {
                fingerprint: String::new(),
                library_id: String::new(),
                alias: String::new(),
                connection: DeviceConnectionLevel::None,
                sync: DeviceSyncLevel::None,
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        register_license_tx.send(DartSignalPack {
            message: RegisterLicenseRequest {
                path: String::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        validate_license_tx.send(DartSignalPack {
            message: ValidateLicenseRequest {
                license: None,
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        discovered_tx.send(DartSignalPack {
            message: GetDiscoveredDeviceRequest { call_id: None },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        clients_tx.send(DartSignalPack {
            message: ListClientsRequest { call_id: None },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        fingerprint_tx.send(DartSignalPack {
            message: GetSslCertificateFingerprintRequest { call_id: None },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        visualizer_support_tx.send(DartSignalPack {
            message: VisualizerSupportRequest { call_id: None },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        visualizer_presets_tx.send(DartSignalPack {
            message: ListVisualizerPresetsRequest { call_id: None },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );

        broadcast_tx.send(DartSignalPack {
            message: StartBroadcastRequest {
                duration_seconds: 1,
                alias: String::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        stop_broadcast_tx.send(DartSignalPack {
            message: StopBroadcastRequest { call_id: None },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        listening_tx.send(DartSignalPack {
            message: StartListeningRequest {
                alias: String::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        stop_listening_tx.send(DartSignalPack {
            message: StopListeningRequest { call_id: None },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        visualizer_dirs_tx.send(DartSignalPack {
            message: SetVisualizerPresetDirsRequest {
                paths: Vec::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        visualizer_scenes_tx.send(DartSignalPack {
            message: SetVisualizerScenesRequest {
                order: Vec::new(),
                disabled: Vec::new(),
                last: None,
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        library_test_tx.send(DartSignalPack {
            message: TestLibraryInitializedRequest {
                path: String::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        mix_with_others_tx.send(DartSignalPack {
            message: SetMixWithOthersRequest {
                enabled: false,
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        sync_settings_tx.send(DartSignalPack {
            message: GetSyncSettingsRequest {
                request_id: String::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        set_sync_settings_tx.send(DartSignalPack {
            message: SetSyncSettingsRequest {
                request_id: String::new(),
                listen: false,
                missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        system_info_tx.send(DartSignalPack {
            message: SystemInfoRequest { call_id: None },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        add_trusted_server_tx.send(DartSignalPack {
            message: AddTrustedServerRequest {
                certificate: None,
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        remove_trusted_client_tx.send(DartSignalPack {
            message: RemoveTrustedClientRequest {
                fingerprint: String::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        update_client_status_tx.send(DartSignalPack {
            message: UpdateClientStatusRequest {
                fingerprint: String::new(),
                status: ClientStatus::Approved,
                preset: None,
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        remove_trusted_server_tx.send(DartSignalPack {
            message: RemoveTrustedServerRequest {
                fingerprint: String::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        availability_tx.send(DartSignalPack {
            message: ServerAvailabilityTestRequest {
                url: String::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        register_device_tx.send(DartSignalPack {
            message: RegisterDeviceOnServerRequest {
                alias: String::new(),
                hosts: Vec::new(),
                fingerprint: String::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        check_device_tx.send(DartSignalPack {
            message: CheckDeviceOnServerRequest {
                alias: String::new(),
                hosts: Vec::new(),
                fingerprint: String::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        connect_tx.send(DartSignalPack {
            message: ConnectRequest {
                hosts: Vec::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );
        fetch_certificate_tx.send(DartSignalPack {
            message: FetchServerCertificateRequest {
                url: String::new(),
                call_id: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RequestFailedResponse"
        );

        let started = list_started.notified();
        list_tx.send(DartSignalPack {
            message: ListLibraryRecordsRequest { call_id: None },
            binary: Vec::new(),
        });
        timeout(Duration::from_secs(1), async {
            tokio::select! {
                _ = started => {},
                () = &mut loop_future => panic!("node request receiver loop exited"),
            }
        })
        .await
        .expect("blocked node handler did not start");
        controllers_tx.send(DartSignalPack {
            message: ListControllersRequest { call_id: None },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "ListControllersResponse"
        );
        release_list.notify_one();
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "ListLibraryRecordsResponse"
        );

        remove_tx.send(DartSignalPack {
            message: RemoveLibraryRecordRequest {
                call_id: None,
                record_id: "invalid".into(),
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RemoveLibraryRecordResponse"
        );

        local_tx.send(DartSignalPack {
            message: RegisterLocalLibraryRequest {
                call_id: None,
                path: String::new(),
                alias: String::new(),
                mode: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RegisterLibraryResponse"
        );

        remote_tx.send(DartSignalPack {
            message: RegisterRemoteControlRequest {
                call_id: None,
                fingerprint: String::new(),
                alias: String::new(),
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RegisterLibraryResponse"
        );

        subsonic_tx.send(DartSignalPack {
            message: RegisterSubsonicLibraryRequest {
                call_id: None,
                endpoint: String::new(),
                username: String::new(),
                password: String::new(),
                alias: String::new(),
                mode: None,
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RegisterLibraryResponse"
        );

        opendal_tx.send(DartSignalPack {
            message: RegisterOpenDalLibraryRequest {
                call_id: None,
                service: OpenDalLibraryService::WebDav {
                    endpoint: String::new(),
                    root: String::new(),
                    username: String::new(),
                    password: String::new(),
                },
                alias: String::new(),
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "RegisterLibraryResponse"
        );

        open_tx.send(DartSignalPack {
            message: OpenLibraryRequest {
                call_id: Some(23),
                record_id: String::new(),
            },
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "OpenLibraryResponse"
        );
    }

    #[test]
    fn standalone_receivers_are_catalogued() {
        assert_eq!(
            standalone_request_names(),
            ["InitializeNodeRequest", "RunFsSelfTestRequest"]
        );
    }
}
