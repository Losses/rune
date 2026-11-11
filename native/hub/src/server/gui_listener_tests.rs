use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use rinf::{DartSignal, DartSignalPack, SignalReceiver, SignalSender, signal_channel};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::{
    sync::mpsc,
    task::{JoinHandle, yield_now},
    time::{pause, resume, sleep, timeout},
};
use tokio_util::sync::CancellationToken;

use super::fixture;
use crate::backends::local::gui_request::{GuiRequest, listen_gui_with_io};
use crate::macros::{RequestAccess, RequestScope, RequestSlots};
use crate::messages::{
    CrashResponse, RequestErrorKind, RequestFailedResponse, RequestFailureCause, SearchForRequest,
    VolumeRequest, VolumeResponse,
};
use crate::request_error::ExpectedRequestError;
use crate::rpc::{
    Slot, TargetRequest,
};
use crate::utils::{GlobalParams, ParamsExtractor, RinfRustSignal};
use crate::{RequestCallId, ResponseCallId, Session, Signal, TypedRequest};

pub(super) struct Listener<R> {
    sender: SignalSender<DartSignalPack<R>>,
    output: mpsc::UnboundedReceiver<(String, Vec<u8>)>,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl<R> Listener<R> {
    pub(super) fn send(&self, message: R) {
        self.sender.send(DartSignalPack {
            message,
            binary: Vec::new(),
        });
    }

    pub(super) async fn receive<T: DeserializeOwned>(&mut self, name: &str) -> T {
        let (actual, bytes) = timeout(Duration::from_secs(5), self.output.recv())
            .await
            .unwrap()
            .expect("listener closed without a response");
        assert_eq!(
            actual,
            name,
            "unexpected GUI response: {}",
            rinf::deserialize::<RequestFailedResponse>(&bytes)
                .map(|response| format!(" {:?}: {}", response.kind, response.detail))
                .unwrap_or_default()
        );
        rinf::deserialize(&bytes).unwrap()
    }

    pub(super) async fn stop(self) {
        self.cancel.cancel();
        timeout(Duration::from_secs(5), self.task)
            .await
            .unwrap()
            .unwrap();
    }
}

pub(super) fn start<R>(params: &Arc<GlobalParams>) -> Listener<R>
where
    R: GuiRequest
        + TargetRequest
        + Signal<Response = <R as TypedRequest>::Response, Params = <R as ParamsExtractor>::Params>
        + ParamsExtractor
        + RequestCallId
        + DartSignal
        + Serialize
        + Send
        + Sync
        + 'static,
    <R as TypedRequest>::Response: DeserializeOwned + Send,
{
    let (sender, receiver) = signal_channel();
    let (output, received) = mpsc::unbounded_channel();
    let cancel = params.main_token.child_token();
    let task = tokio::spawn(listen_gui_with_io::<R>(
        params.clone(),
        cancel.clone(),
        receiver,
        move |signal| {
            output
                .send((signal.name(), signal.encode_to_vec().unwrap()))
                .unwrap();
        },
    ));
    Listener {
        sender,
        output: received,
        cancel,
        task,
    }
}

#[derive(Serialize, Deserialize)]
struct DelayedRequest {
    value: f32,
    delay_ms: u64,
    failure: u8,
}

impl TypedRequest for DelayedRequest {
    type Response = VolumeResponse;
    fn wire_name() -> &'static str {
        "DelayedRequest"
    }
    fn response_name() -> &'static str {
        "VolumeResponse"
    }
}
impl TargetRequest for DelayedRequest {
    const SCOPE: RequestScope = RequestScope::Node;
    const ACCESS: RequestAccess = RequestAccess::NodeLocal;
    const INTRODUCED_MINOR: u32 = 0;
    fn slot(_: &RequestSlots) -> Slot {
        Slot::Native
    }
}
impl RequestCallId for DelayedRequest {
    fn call_id(&self) -> Option<u64> {
        Some(91)
    }
}
impl DartSignal for DelayedRequest {
    fn get_dart_signal_receiver() -> SignalReceiver<DartSignalPack<Self>> {
        unreachable!("tests supply an isolated receiver")
    }
}
impl ParamsExtractor for DelayedRequest {
    type Params = ();
    fn extract_params(&self, _: &GlobalParams) {}
}
impl Signal for DelayedRequest {
    type Params = ();
    type Response = VolumeResponse;
    async fn handle(&self, _: (), _: Option<Session>, _: &Self) -> Result<Option<VolumeResponse>> {
        sleep(Duration::from_millis(self.delay_ms)).await;
        match self.failure {
            1 => Err(ExpectedRequestError::CertificateMismatch.into()),
            2 => Err(anyhow!("listener internal failure")),
            _ => Ok(Some(VolumeResponse { call_id: None, volume: self.value })),
        }
    }
}
impl GuiRequest for DelayedRequest {
    fn emit_response(
        request: &Self,
        response: Option<VolumeResponse>,
        emit: &impl Fn(&dyn RinfRustSignal),
    ) {
        if let Some(mut response) = response {
            response.set_call_id(request.call_id());
            emit(&response);
        }
    }
}

fn delayed(value: f32, delay_ms: u64, failure: u8) -> DelayedRequest {
    DelayedRequest {
        value,
        delay_ms,
        failure,
    }
}

#[tokio::test]
async fn gui_listener_dispatches_catalogue_request_and_emits_response() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let mut listener = start::<VolumeRequest>(&fixture.global_params);
    listener.send(VolumeRequest {
            call_id: None, volume: 0.37 });
    let response: VolumeResponse = listener.receive("VolumeResponse").await;
    assert_eq!(response.volume, 0.37);
    timeout(Duration::from_secs(10), async {
        loop {
            if fixture
                .global_params
                .player
                .lock()
                .await
                .get_status()
                .volume
                == response.volume
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    listener.stop().await;
    Ok(())
}

#[tokio::test]
async fn gui_listener_orders_same_type_without_blocking_other_types() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let mut slow = start::<DelayedRequest>(&fixture.global_params);
    let mut fast = start::<VolumeRequest>(&fixture.global_params);
    // Paused time makes the first handler slow deterministically, without wall-clock races.
    pause();
    slow.send(delayed(1.0, 1000, 0));
    slow.send(delayed(2.0, 0, 0));
    fast.send(VolumeRequest {
            call_id: None, volume: 0.63 });
    let response: VolumeResponse =
        timeout(Duration::from_millis(500), fast.receive("VolumeResponse")).await?;
    assert_eq!(response.volume, 0.63);
    let first: VolumeResponse = slow.receive("VolumeResponse").await;
    let second: VolumeResponse = slow.receive("VolumeResponse").await;
    assert_eq!(
        [first.volume, second.volume],
        [1.0, 2.0],
        "same-type requests completed out of order"
    );
    slow.stop().await;
    fast.stop().await;
    resume();
    Ok(())
}

#[tokio::test]
async fn gui_listener_failure_preserves_call_id_kind_and_cause() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let mut params = (*fixture.global_params).clone();
    params.library = None;
    let mut search = start::<SearchForRequest>(&Arc::new(params));
    search.send(SearchForRequest {
        call_id: Some(73),
        query_str: "missing".into(),
        fields: Vec::new(),
        n: 30,
    });
    let failure: RequestFailedResponse = search.receive("RequestFailedResponse").await;
    assert_eq!(failure.call_id, Some(73));
    assert_eq!(failure.request, "SearchForRequest");
    assert_eq!(failure.kind, RequestErrorKind::Unavailable);
    assert!(
        search.output.try_recv().is_err(),
        "expected failure emitted a crash"
    );
    search.stop().await;

    let mut listener = start::<DelayedRequest>(&fixture.global_params);
    listener.send(delayed(0.0, 0, 1));
    let failure: RequestFailedResponse = listener.receive("RequestFailedResponse").await;
    assert_eq!(failure.call_id, Some(91));
    assert_eq!(failure.kind, RequestErrorKind::InvalidInput);
    assert_eq!(
        failure.cause,
        Some(RequestFailureCause::CertificateMismatch)
    );
    assert!(
        listener.output.try_recv().is_err(),
        "expected failure emitted a crash"
    );
    listener.stop().await;
    Ok(())
}

#[tokio::test]
async fn gui_listener_internal_failure_emits_crash_and_continues() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let mut listener = start::<DelayedRequest>(&fixture.global_params);
    listener.send(delayed(0.0, 0, 2));
    listener.send(delayed(0.42, 0, 0));
    let failure: RequestFailedResponse = listener.receive("RequestFailedResponse").await;
    assert_eq!(failure.kind, RequestErrorKind::Internal);
    assert_eq!(failure.call_id, Some(91));
    let crash: CrashResponse = listener.receive("CrashResponse").await;
    assert!(crash.detail.contains("listener internal failure"));
    let response: VolumeResponse = listener.receive("VolumeResponse").await;
    assert_eq!(response.volume, 0.42);
    listener.stop().await;
    Ok(())
}

#[tokio::test]
async fn gui_listener_cancels_idle_pending_and_queued_requests() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    start::<VolumeRequest>(&fixture.global_params).stop().await;
    let mut listener = start::<DelayedRequest>(&fixture.global_params);
    listener.send(delayed(1.0, 60_000, 0));
    listener.send(delayed(2.0, 0, 0));
    yield_now().await;
    listener.cancel.cancel();
    timeout(Duration::from_secs(5), &mut listener.task).await??;
    assert!(listener.output.try_recv().is_err());
    // Cancellation is biased even when a request is already queued.
    let mut listener = start::<VolumeRequest>(&fixture.global_params);
    listener.send(VolumeRequest {
            call_id: None, volume: 0.12 });
    fixture.global_params.main_token.cancel();
    timeout(Duration::from_secs(5), &mut listener.task).await??;
    assert!(listener.output.try_recv().is_err());
    Ok(())
}
