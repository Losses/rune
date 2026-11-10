#[macro_export]
macro_rules! implement_rinf_rust_signal_trait {
    ($($t:ty),*) => {
        $(
            impl $crate::RinfRustSignal for $t {
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
use requests::define_request_types;
use rinf::{DartSignal, DartSignalPack, RustSignal, SignalReceiver};
use subsonic::SubsonicProbe;
use uuid::Uuid;

use crate::backends::local::gui_request::{GuiRequest, listen_gui};
use crate::node::{NodeContext, NodeSlot};
use crate::request_error::{ExpectedRequestError, classify, request_failure_cause};
use crate::rpc::{
    RpcTarget, Slot, SubsonicTranslate, SubsonicTranslationFuture, TargetRequest,
    UnsupportedReason, narrow_slot,
};
use crate::server::server_request::serve_remote;
use crate::subsonic_binding::SubsonicBinding;
use crate::utils::{GlobalParams, RinfRustSignal};
use crate::{CallIdentified, Session, TypedRequest};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use crate::messages::*;
define_request_types!();

#[cfg(test)]
mod node_request_loop_tests {
    use std::{future::Future, pin::Pin};

    use anyhow::Result;
    use rinf::{DartSignalPack, signal_channel};
    use tokio::{
        sync::mpsc,
        time::{Duration, timeout},
    };

    use super::{
        NodeRequestHandlers, NodeRequestReceivers, bootstrap_request_names,
        node_request_loop_with_receivers,
    };
    use crate::messages::{
        ListLibraryRecordsRequest, ListLibraryRecordsResponse, OpenLibraryRequest,
        OpenLibraryResponse, OpenLibraryResult, RegisterLibraryOutcome, RegisterLibraryResponse,
        RegisterLocalLibraryRequest, RegisterRemoteControlRequest, RegisterSubsonicLibraryRequest,
        RemoveLibraryRecordOutcome, RemoveLibraryRecordRequest, RemoveLibraryRecordResponse,
    };
    use crate::node::NodeSlot;
    use crate::utils::RinfRustSignal;

    struct TestHandlers;

    #[allow(clippy::manual_async_fn)]
    impl NodeRequestHandlers for TestHandlers {
        fn handle_list_library_records_request(
            &self,
            _: &NodeSlot,
            _: &ListLibraryRecordsRequest,
        ) -> impl Future<Output = Result<Option<ListLibraryRecordsResponse>>> + Send {
            async {
                Ok(Some(ListLibraryRecordsResponse {
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
    async fn generated_node_loop_receives_every_catalogue_request() {
        let (list_tx, list_rx) = signal_channel();
        let (remove_tx, remove_rx) = signal_channel();
        let (local_tx, local_rx) = signal_channel();
        let (remote_tx, remote_rx) = signal_channel();
        let (subsonic_tx, subsonic_rx) = signal_channel();
        let (open_tx, open_rx) = signal_channel();
        let (output_tx, mut output_rx) = mpsc::unbounded_channel();
        let emit = move |signal: &dyn RinfRustSignal| {
            output_tx
                .send((signal.name(), signal.encode_to_vec().unwrap()))
                .unwrap();
        };
        let node = NodeSlot::new();
        let mut loop_future = Box::pin(node_request_loop_with_receivers(
            &node,
            TestHandlers,
            emit,
            NodeRequestReceivers {
                list_library_records_request_receiver: list_rx,
                remove_library_record_request_receiver: remove_rx,
                register_local_library_request_receiver: local_rx,
                register_remote_control_request_receiver: remote_rx,
                register_subsonic_library_request_receiver: subsonic_rx,
                open_library_request_receiver: open_rx,
            },
        ));

        list_tx.send(DartSignalPack {
            message: ListLibraryRecordsRequest {},
            binary: Vec::new(),
        });
        assert_eq!(
            response_or_loop_exit(&mut loop_future, &mut output_rx)
                .await
                .0,
            "ListLibraryRecordsResponse"
        );

        remove_tx.send(DartSignalPack {
            message: RemoveLibraryRecordRequest {
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

        open_tx.send(DartSignalPack {
            message: OpenLibraryRequest {
                call_id: 23,
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
    fn bootstrap_receivers_are_catalogued_as_separate_paths() {
        assert_eq!(
            bootstrap_request_names(),
            ["RunFsSelfTestRequest", "InitializeNodeRequest"]
        );
    }
}
