use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow};
use rinf::{DartSignal, DartSignalPack, SignalReceiver, SignalSender, signal_channel};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::{
    sync::{Notify, mpsc},
    task::JoinHandle,
    time::{pause, resume, sleep},
};
use tokio_util::sync::CancellationToken;

use super::fixture;
use crate::backends::local::gui_request::{GuiRequest, listen_gui_with_io};
use crate::macros::{BodyLibraries, RequestAccess, RequestScope, RequestSlots};
use crate::messages::{
    RequestErrorKind, RequestFailedResponse, RequestFailureCause, SearchForRequest,
    VolumeRequest, VolumeResponse,
};
use crate::request_error::ExpectedRequestError;
use crate::rpc::{
    Slot, TargetRequest,
};
use crate::utils::{GlobalParams, ParamsExtractor, RinfRustSignal};
use crate::{RequestCallId, ResponseCallId, Session, Signal, TypedRequest};
use crate::test_wait::{TEST_DEADLINE, wait_until};
use tokio::time as tokio_time;

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
        let (actual, bytes) = tokio_time::timeout(TEST_DEADLINE, self.output.recv())
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
        tokio_time::timeout(TEST_DEADLINE, self.task)
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
    #[serde(skip)]
    started: Option<Arc<Notify>>,
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
    const BODY_LIBRARIES: BodyLibraries = BodyLibraries::NoLibrary;
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
        if let Some(started) = &self.started {
            started.notify_one();
        }
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
        started: None,
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
    wait_until("player volume to reflect the response", || async {
        Ok((fixture
            .global_params
            .player
            .lock()
            .await
            .get_status()
            .volume
            == response.volume)
            .then_some(()))
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
    let response: VolumeResponse = tokio_time::timeout(
        TEST_DEADLINE,
        fast.receive("VolumeResponse"),
    )
    .await?;
    assert_eq!(response.volume, 0.63);
    tokio_time::advance(Duration::from_secs(1)).await;
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
        refresh: false,
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
async fn gui_listener_internal_failure_does_not_emit_crash() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let mut listener = start::<DelayedRequest>(&fixture.global_params);
    listener.send(delayed(0.0, 0, 2));
    listener.send(delayed(0.42, 0, 0));
    let failure: RequestFailedResponse = listener.receive("RequestFailedResponse").await;
    assert_eq!(failure.kind, RequestErrorKind::Internal);
    assert_eq!(failure.call_id, Some(91));
    let response: VolumeResponse = listener.receive("VolumeResponse").await;
    assert_eq!(response.volume, 0.42);
    assert!(
        listener.output.try_recv().is_err(),
        "internal request failure should not emit CrashResponse"
    );
    listener.stop().await;
    Ok(())
}

#[tokio::test]
async fn gui_listener_cancels_idle_pending_and_queued_requests() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    start::<VolumeRequest>(&fixture.global_params).stop().await;
    let mut listener = start::<DelayedRequest>(&fixture.global_params);
    let started = Arc::new(Notify::new());
    listener.send(DelayedRequest {
        value: 1.0,
        delay_ms: 60_000,
        failure: 0,
        started: Some(started.clone()),
    });
    listener.send(delayed(2.0, 0, 0));
    tokio_time::timeout(TEST_DEADLINE, started.notified())
        .await
        .context("first delayed request did not start")?;
    listener.cancel.cancel();
    tokio_time::timeout(TEST_DEADLINE, &mut listener.task).await??;
    assert!(listener.output.try_recv().is_err());
    // Cancellation is biased even when a request is already queued.
    let mut listener = start::<VolumeRequest>(&fixture.global_params);
    listener.send(VolumeRequest {
            call_id: None, volume: 0.12 });
    fixture.global_params.main_token.cancel();
    tokio_time::timeout(TEST_DEADLINE, &mut listener.task).await??;
    assert!(listener.output.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn playlist_requests_reject_invalid_input_with_call_id() -> Result<()> {
    use crate::messages::{
        FetchPlaylistOccurrencesRequest, FetchPlaylistOccurrencesResponse,
        PlaylistOccurrenceCursor, RemoveItemFromPlaylistRequest,
    };
    use database::actions::playlists::create_playlist;
    use uuid::Uuid;

    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let library = fixture.global_params.test_library();
    let playlist = create_playlist(
        &library.main_db,
        &library.library_sync.record_store(),
        "Paging test".into(),
        "Playlists".into(),
    )
    .await?;
    let mut fetch = start::<FetchPlaylistOccurrencesRequest>(&fixture.global_params);
    let page_request = |call_id, playlist_id, page_size, after| FetchPlaylistOccurrencesRequest {
        call_id: Some(call_id),
        request_id: format!("page-{call_id}"),
        playlist_id,
        after,
        page_size,
        bake_cover_arts: false,
    };

    fetch.send(page_request(1, playlist.id, 128, None));
    let page: FetchPlaylistOccurrencesResponse =
        fetch.receive("FetchPlaylistOccurrencesResponse").await;
    assert_eq!(page.call_id, Some(1));
    assert!(page.entries.is_empty());

    fetch.send(page_request(2, i32::MAX, 1, None));
    let failure: RequestFailedResponse = fetch.receive("RequestFailedResponse").await;
    assert_eq!(failure.call_id, Some(2));
    assert_eq!(failure.kind, RequestErrorKind::NotFound);

    for (call_id, page_size, cursor) in [
        (3, 0, None),
        (4, 129, None),
        (5, 1, Some("not-a-uuid".into())),
        (6, 1, Some(Uuid::nil().to_string())),
    ] {
        fetch.send(page_request(
            call_id,
            playlist.id,
            page_size,
            cursor.map(|occurrence_id| PlaylistOccurrenceCursor {
                position: 0,
                occurrence_id,
            }),
        ));
        let failure: RequestFailedResponse = fetch.receive("RequestFailedResponse").await;
        assert_eq!(failure.call_id, Some(call_id));
        assert_eq!(
            failure.kind,
            RequestErrorKind::InvalidInput,
            "page request {call_id}: {}",
            failure.detail
        );
    }
    fetch.stop().await;

    let mut remove = start::<RemoveItemFromPlaylistRequest>(&fixture.global_params);
    remove.send(RemoveItemFromPlaylistRequest {
        call_id: Some(7),
        playlist_id: playlist.id,
        occurrence_id: Uuid::nil().to_string(),
    });
    let failure: RequestFailedResponse = remove.receive("RequestFailedResponse").await;
    assert_eq!(failure.call_id, Some(7));
    assert_eq!(failure.kind, RequestErrorKind::InvalidInput);
    remove.stop().await;
    Ok(())
}

#[tokio::test]
async fn playlist_database_error_does_not_crash_and_recovers_after_repair() -> Result<()> {
    use std::fs;
    use std::path::Path;

    use database::actions::metadata::scan_audio_file;
    use database::actions::playlists::{add_item_to_playlist, create_playlist};
    use database::entities::media_files;
    use sea_orm::sea_query::{Alias, Table};
    use sea_orm::{ConnectionTrait, EntityTrait};

    use crate::messages::{FetchPlaylistOccurrencesRequest, FetchPlaylistOccurrencesResponse};

    let dir = tempfile::tempdir()?;
    let fixture = fixture(dir.path()).await?;
    let library = fixture.global_params.test_library();
    let media_root = Path::new(&library.media_root);
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
    let target = media_root.join("recovery.ogg");
    fs::copy(&source, &target)?;
    scan_audio_file(
        &library.fsio,
        &library.main_db,
        &fixture.global_params.node_id,
        media_root,
        &target,
        &library.library_sync.record_store(),
    )
    .await?;
    let file = media_files::Entity::find()
        .one(&*library.main_db)
        .await?
        .context("scanned media file")?;
    let playlist = create_playlist(
        &library.main_db,
        &library.library_sync.record_store(),
        "Recovery playlist".into(),
        "Playlists".into(),
    )
    .await?;
    let mut expected = Vec::with_capacity(61);
    for position in 0..61 {
        let entry = add_item_to_playlist(
            &library.main_db,
            &library.library_sync.record_store(),
            playlist.id,
            file.id,
            Some(position),
        )
        .await?;
        expected.push((entry.position, entry.source_uuid));
    }

    let mut fetch = start::<FetchPlaylistOccurrencesRequest>(&fixture.global_params);
    let page_request = |call_id, after| FetchPlaylistOccurrencesRequest {
        call_id: Some(call_id),
        request_id: format!("recovery-{call_id}"),
        playlist_id: playlist.id,
        after,
        page_size: 20,
        bake_cover_arts: false,
    };

    // First page succeeds before the database fault.
    fetch.send(page_request(1, None));
    let first: FetchPlaylistOccurrencesResponse =
        fetch.receive("FetchPlaylistOccurrencesResponse").await;
    assert_eq!(first.call_id, Some(1));
    assert_eq!(first.entries.len(), 20);
    assert!(first.next.is_some());

    // Induce a real SQLite table error during the cursor-page request.
    library
        .main_db
        .execute(
            &Table::rename()
                .table(
                    Alias::new("media_file_playlists"),
                    Alias::new("media_file_playlists_fault"),
                )
                .to_owned(),
        )
        .await?;
    fetch.send(page_request(2, first.next.clone()));
    let failure: RequestFailedResponse = fetch.receive("RequestFailedResponse").await;
    assert_eq!(failure.call_id, Some(2));
    assert_eq!(failure.kind, RequestErrorKind::Internal);

    // Repair the database and retry in the same process; no CrashResponse should arrive.
    library
        .main_db
        .execute(
            &Table::rename()
                .table(
                    Alias::new("media_file_playlists_fault"),
                    Alias::new("media_file_playlists"),
                )
                .to_owned(),
        )
        .await?;

    let mut recovered = Vec::new();
    let mut cursor = None;
    let mut call_id = 3;
    loop {
        fetch.send(page_request(call_id, cursor));
        let page: FetchPlaylistOccurrencesResponse =
            fetch.receive("FetchPlaylistOccurrencesResponse").await;
        assert_eq!(page.call_id, Some(call_id));
        recovered.extend(
            page.entries
                .into_iter()
                .map(|entry| (entry.position, entry.occurrence_id)),
        );
        call_id += 1;
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(recovered, expected);
    assert!(
        fetch.output.try_recv().is_err(),
        "database error recovery emitted an unexpected extra signal"
    );
    fetch.stop().await;
    Ok(())
}
