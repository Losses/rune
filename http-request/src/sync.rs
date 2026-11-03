//! Authenticated-caller HTTP adaptation of neutral metadata capabilities.
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::marker::PhantomData;
use std::{future::Future, pin::Pin};
use std::{sync::Arc, time::Duration};
use sync::SyncModel;
use sync::protocol as p;
use uuid::Uuid;

// Core page/fragment/admission limits still apply inside these envelope bounds.
const REQUEST_BYTES: usize = 9 * 1024 * 1024;
const RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

pub trait MetadataStore<M: SyncModel>:
    p::SyncStore<M> + p::SnapshotSource<M> + p::SnapshotSink<M>
{
}
impl<M: SyncModel, T: p::SyncStore<M> + p::SnapshotSource<M> + p::SnapshotSink<M>> MetadataStore<M>
    for T
{
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
#[serde(bound(serialize = "", deserialize = ""))]
enum MetadataOperation<M: SyncModel> {
    ReleaseRequest {
        request_id: Uuid,
    },
    CancelReconciliation {
        session: Uuid,
    },
    DiscardRange {
        session: Uuid,
        range: p::KeyRange<M>,
    },
    Identity,
    Head,
    History {
        key: p::CheckpointKey,
        progress: Option<p::ReceiveProgress>,
    },
    Checkpoint {
        key: p::CheckpointKey,
    },
    ReadPage {
        request: p::PageRequest<M>,
    },
    ReadFragment {
        manifest: p::ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    },
    ApplyPage {
        page: p::ChangePage<M>,
        limits: p::ApplyLimits,
    },
    BeginStage {
        descriptor: p::StageDescriptor<M>,
        limits: p::StageLimits,
    },
    StageFragment {
        stage: Uuid,
        fragment: p::Fragment,
    },
    StageStatus {
        stage: Uuid,
    },
    AbandonStage {
        stage: Uuid,
    },
    AbandonOrigin {
        origin: p::StageOrigin<M>,
    },
    CommitFragmentedEvent {
        key: p::CheckpointKey,
        sequence: u64,
        stage: Uuid,
        limits: p::ApplyLimits,
    },
    FinishSession {
        key: p::CheckpointKey,
        through: p::ReceiveProgress,
    },
    OpenSnapshot {
        request_id: Uuid,
        scope: p::Scope<M>,
        limits: p::WorkLimits,
        resume: Option<p::WorkCursor>,
    },
    Summarize {
        snapshot: Uuid,
        range: p::KeyRange<M>,
        limits: p::WorkLimits,
        resume: Option<p::WorkCursor>,
    },
    Split {
        snapshot: Uuid,
        range: p::KeyRange<M>,
        limits: p::WorkLimits,
        resume: Option<p::WorkCursor>,
    },
    Leaf {
        snapshot: Uuid,
        range: p::KeyRange<M>,
        after: Option<p::CandidateCursor<M>>,
        limits: p::PageLimits,
    },
    SnapshotFragment {
        snapshot: Uuid,
        manifest: p::ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    },
    CloseSnapshot {
        snapshot: Uuid,
    },
    CancelSnapshotBuild {
        cursor: p::WorkCursor,
    },
    BeginReconciliation {
        request_id: Uuid,
        remote: p::SnapshotDescriptor<M>,
        local: p::SnapshotDescriptor<M>,
    },
    Coverage {
        session: Uuid,
        ledger_revision: Option<u64>,
        after: u64,
        limits: p::PageLimits,
    },
    SplitRange {
        session: Uuid,
        parent: p::KeyRange<M>,
        children: Vec<p::RangeSummary<M>>,
    },
    AcceptEqualRange {
        session: Uuid,
        source: p::RangeSummary<M>,
    },
    LeafProgress {
        session: Uuid,
        range: p::KeyRange<M>,
    },
    StageLeafPage {
        session: Uuid,
        page: p::LeafPage<M>,
    },
    CommitCompleteRange {
        session: Uuid,
        range: p::KeyRange<M>,
        limits: p::ApplyLimits,
    },
    PromoteStep {
        session: Uuid,
        limits: p::ApplyLimits,
    },
    CompleteReconciliation {
        session: Uuid,
    },
}

/// The caller supplies a fresh authorized endpoint for each request.
pub type EndpointProvider<M> =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Arc<dyn MetadataStore<M>>> + Send>> + Send + Sync>;

pub fn router<M: SyncModel>(store: Arc<dyn MetadataStore<M>>) -> Router {
    router_with_provider(Arc::new(move || {
        let store = store.clone();
        Box::pin(async move { store })
    }))
}

pub fn router_with_provider<M: SyncModel>(provider: EndpointProvider<M>) -> Router {
    Router::new()
        .route("/sync/metadata", post(dispatch::<M>))
        .layer(DefaultBodyLimit::max(REQUEST_BYTES))
        .with_state(provider)
}

fn reply<T: Serialize>(result: p::Result<T>) -> Response {
    match serde_json::to_vec(&result) {
        Ok(bytes) if bytes.len() <= RESPONSE_BYTES => {
            ([(header::CONTENT_TYPE, "application/json")], bytes).into_response()
        }
        Ok(bytes) => Json(p::Result::<()>::Err(p::SyncError::LimitTooSmall {
            required: bytes.len() as u64,
        }))
        .into_response(),
        Err(error) => Json(p::Result::<()>::Err(p::SyncError::Storage(
            error.to_string(),
        )))
        .into_response(),
    }
}

async fn dispatch<M: SyncModel>(
    State(source): State<EndpointProvider<M>>,
    bytes: Bytes,
) -> Response {
    let store = source().await;
    let request: MetadataOperation<M> = match serde_json::from_slice(&bytes) {
        Ok(request) => request,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(p::Result::<()>::Err(p::SyncError::Invalid(
                    error.to_string(),
                ))),
            )
                .into_response();
        }
    };
    match request {
        MetadataOperation::ReleaseRequest { request_id } => {
            reply(p::SnapshotSource::release_request(&*store, request_id).await)
        }
        MetadataOperation::CancelReconciliation { session } => {
            reply(p::SnapshotSink::cancel_reconciliation(&*store, session).await)
        }
        MetadataOperation::DiscardRange { session, range } => {
            reply(p::SnapshotSink::discard_range(&*store, session, range).await)
        }
        MetadataOperation::Identity => reply(p::ReceiveStore::identity(&*store).await),
        MetadataOperation::Head => reply(p::SyncStore::head(&*store).await),
        MetadataOperation::History { key, progress } => {
            reply(p::SyncStore::history_status(&*store, &key, progress).await)
        }
        MetadataOperation::Checkpoint { key } => {
            reply(p::ReceiveStore::checkpoint(&*store, &key).await)
        }
        MetadataOperation::ReadPage { request } => {
            reply(p::SyncStore::read_page(&*store, request).await)
        }
        MetadataOperation::ReadFragment {
            manifest,
            ordinal,
            max_bytes,
        } => reply(p::SyncStore::read_fragment(&*store, manifest, ordinal, max_bytes).await),
        MetadataOperation::ApplyPage { page, limits } => {
            reply(p::ReceiveStore::apply_page(&*store, &page, limits).await)
        }
        MetadataOperation::BeginStage { descriptor, limits } => {
            reply(p::ReceiveStore::begin_stage(&*store, descriptor, limits).await)
        }
        MetadataOperation::StageFragment { stage, fragment } => {
            reply(p::ReceiveStore::stage_fragment(&*store, stage, fragment).await)
        }
        MetadataOperation::StageStatus { stage } => {
            reply(p::ReceiveStore::stage_status(&*store, stage).await)
        }
        MetadataOperation::AbandonStage { stage } => {
            reply(p::ReceiveStore::abandon_stage(&*store, stage).await)
        }
        MetadataOperation::AbandonOrigin { origin } => {
            reply(p::ReceiveStore::abandon_origin(&*store, origin).await)
        }
        MetadataOperation::CommitFragmentedEvent {
            key,
            sequence,
            stage,
            limits,
        } => reply(
            p::ReceiveStore::commit_fragmented_event(&*store, key, sequence, stage, limits).await,
        ),
        MetadataOperation::FinishSession { key, through } => {
            reply(p::ReceiveStore::finish_session(&*store, key, through).await)
        }
        MetadataOperation::OpenSnapshot {
            request_id,
            scope,
            limits,
            resume,
        } => reply(
            p::SnapshotSource::open_snapshot(&*store, request_id, scope, limits, resume).await,
        ),
        MetadataOperation::Summarize {
            snapshot,
            range,
            limits,
            resume,
        } => reply(p::SnapshotSource::summarize(&*store, snapshot, range, limits, resume).await),
        MetadataOperation::Split {
            snapshot,
            range,
            limits,
            resume,
        } => reply(p::SnapshotSource::split(&*store, snapshot, range, limits, resume).await),
        MetadataOperation::Leaf {
            snapshot,
            range,
            after,
            limits,
        } => reply(p::SnapshotSource::leaf(&*store, snapshot, range, after, limits).await),
        MetadataOperation::SnapshotFragment {
            snapshot,
            manifest,
            ordinal,
            max_bytes,
        } => reply(
            p::SnapshotSource::read_fragment(&*store, snapshot, manifest, ordinal, max_bytes).await,
        ),
        MetadataOperation::CloseSnapshot { snapshot } => {
            reply(p::SnapshotSource::close_snapshot(&*store, snapshot).await)
        }
        MetadataOperation::CancelSnapshotBuild { cursor } => {
            reply(p::SnapshotSource::cancel_snapshot_build(&*store, cursor).await)
        }
        MetadataOperation::BeginReconciliation {
            request_id,
            remote,
            local,
        } => reply(p::SnapshotSink::begin_reconciliation(&*store, request_id, remote, local).await),
        MetadataOperation::Coverage {
            session,
            ledger_revision,
            after,
            limits,
        } => {
            reply(p::SnapshotSink::coverage(&*store, session, ledger_revision, after, limits).await)
        }
        MetadataOperation::SplitRange {
            session,
            parent,
            children,
        } => reply(p::SnapshotSink::split_range(&*store, session, parent, children).await),
        MetadataOperation::AcceptEqualRange { session, source } => {
            reply(p::SnapshotSink::accept_equal_range(&*store, session, source).await)
        }
        MetadataOperation::LeafProgress { session, range } => {
            reply(p::SnapshotSink::leaf_progress(&*store, session, range).await)
        }
        MetadataOperation::StageLeafPage { session, page } => {
            reply(p::SnapshotSink::stage_leaf_page(&*store, session, page).await)
        }
        MetadataOperation::CommitCompleteRange {
            session,
            range,
            limits,
        } => reply(p::SnapshotSink::commit_complete_range(&*store, session, range, limits).await),
        MetadataOperation::PromoteStep { session, limits } => {
            reply(p::SnapshotSink::promote_step(&*store, session, limits).await)
        }
        MetadataOperation::CompleteReconciliation { session } => {
            reply(p::SnapshotSink::complete_reconciliation(&*store, session).await)
        }
    }
}

/// One authorized endpoint; no remote local-authoring capability is exposed.
#[derive(Clone)]
pub struct MetadataPeer<M: SyncModel> {
    client: reqwest::Client,
    base: String,
    model: PhantomData<M>,
}
impl<M: SyncModel> MetadataPeer<M> {
    pub fn new(base: impl Into<String>, client: reqwest::Client) -> Self {
        Self {
            client,
            base: base.into().trim_end_matches('/').to_owned(),
            model: PhantomData,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    async fn request<T: DeserializeOwned>(&self, operation: MetadataOperation<M>) -> p::Result<T> {
        let bytes = serde_json::to_vec(&operation)
            .map_err(|error| p::SyncError::Invalid(error.to_string()))?;
        if bytes.len() > REQUEST_BYTES {
            return Err(p::SyncError::LimitTooSmall {
                required: bytes.len() as u64,
            });
        }
        tokio::time::timeout(TIMEOUT, async {
            let mut response = self
                .client
                .post(format!("{}/sync/metadata", self.base))
                .header(header::CONTENT_TYPE, "application/json")
                .body(bytes)
                .send()
                .await
                .map_err(|error| p::SyncError::Transport(error.to_string()))?;
            // Valid engine errors retain their type.
            if response
                .content_length()
                .is_some_and(|n| n > RESPONSE_BYTES as u64)
            {
                return Err(p::SyncError::LogicalLimit);
            }
            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(p::SyncError::Unauthorized);
            }
            let mut body = Vec::new();
            while let Some(bytes) = response
                .chunk()
                .await
                .map_err(|error| p::SyncError::Transport(error.to_string()))?
            {
                if bytes.len() > RESPONSE_BYTES.saturating_sub(body.len()) {
                    return Err(p::SyncError::LogicalLimit);
                }
                body.extend_from_slice(&bytes);
            }
            if !status.is_success() {
                if let Ok(Err(error)) =
                    serde_json::from_slice::<p::Result<serde_json::Value>>(&body)
                {
                    return Err(error);
                }
                return Err(p::SyncError::Transport(format!(
                    "metadata HTTP status {status}"
                )));
            }
            serde_json::from_slice::<p::Result<T>>(&body).map_err(|error| {
                p::SyncError::Invalid(format!("invalid metadata response: {error}"))
            })?
        })
        .await
        .map_err(|_| p::SyncError::Transport("metadata request timed out".into()))?
    }
}

impl<M: SyncModel> p::SyncPeer<M> for MetadataPeer<M> {
    fn identity(&self) -> p::BoxFuture<'_, p::StoreIdentity> {
        Box::pin(async move { self.request(MetadataOperation::Identity).await })
    }
    fn head(&self) -> p::BoxFuture<'_, p::ReceiveProgress> {
        Box::pin(async move { self.request(MetadataOperation::Head).await })
    }
    fn history_status<'a>(
        &'a self,
        key: &'a p::CheckpointKey,
        progress: Option<p::ReceiveProgress>,
    ) -> p::BoxFuture<'a, p::HistoryStatus> {
        let key = key.clone();
        Box::pin(async move {
            self.request(MetadataOperation::History { key, progress })
                .await
        })
    }
    fn read_page(&self, request: p::PageRequest<M>) -> p::BoxFuture<'_, p::PageRead<M>> {
        Box::pin(async move { self.request(MetadataOperation::ReadPage { request }).await })
    }
    fn read_fragment(
        &self,
        manifest: p::ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> p::BoxFuture<'_, p::Fragment> {
        Box::pin(async move {
            self.request(MetadataOperation::ReadFragment {
                manifest,
                ordinal,
                max_bytes,
            })
            .await
        })
    }
}

impl<M: SyncModel> p::ReceiveStore<M> for MetadataPeer<M> {
    fn identity(&self) -> p::BoxFuture<'_, p::StoreIdentity> {
        Box::pin(async move { self.request(MetadataOperation::Identity).await })
    }
    fn checkpoint<'a>(
        &'a self,
        key: &'a p::CheckpointKey,
    ) -> p::BoxFuture<'a, Option<p::ReceiveProgress>> {
        let key = key.clone();
        Box::pin(async move { self.request(MetadataOperation::Checkpoint { key }).await })
    }
    fn apply_page<'a>(
        &'a self,
        page: &'a p::ChangePage<M>,
        limits: p::ApplyLimits,
    ) -> p::BoxFuture<'a, p::ApplyResult<M>> {
        let page = page.clone();
        Box::pin(async move {
            self.request(MetadataOperation::ApplyPage { page, limits })
                .await
        })
    }
    fn begin_stage(
        &self,
        descriptor: p::StageDescriptor<M>,
        limits: p::StageLimits,
    ) -> p::BoxFuture<'_, p::FragmentProgress> {
        Box::pin(async move {
            self.request(MetadataOperation::BeginStage { descriptor, limits })
                .await
        })
    }
    fn stage_fragment(
        &self,
        stage: Uuid,
        fragment: p::Fragment,
    ) -> p::BoxFuture<'_, p::FragmentProgress> {
        Box::pin(async move {
            self.request(MetadataOperation::StageFragment { stage, fragment })
                .await
        })
    }
    fn stage_status(&self, stage: Uuid) -> p::BoxFuture<'_, p::FragmentProgress> {
        Box::pin(async move { self.request(MetadataOperation::StageStatus { stage }).await })
    }
    fn abandon_stage(&self, stage: Uuid) -> p::BoxFuture<'_, ()> {
        Box::pin(async move {
            self.request(MetadataOperation::AbandonStage { stage })
                .await
        })
    }
    fn abandon_origin(&self, origin: p::StageOrigin<M>) -> p::BoxFuture<'_, ()> {
        Box::pin(async move {
            self.request(MetadataOperation::AbandonOrigin { origin })
                .await
        })
    }
    fn commit_fragmented_event(
        &self,
        key: p::CheckpointKey,
        sequence: u64,
        stage: Uuid,
        limits: p::ApplyLimits,
    ) -> p::BoxFuture<'_, p::ApplyResult<M>> {
        Box::pin(async move {
            self.request(MetadataOperation::CommitFragmentedEvent {
                key,
                sequence,
                stage,
                limits,
            })
            .await
        })
    }
    fn finish_session(
        &self,
        key: p::CheckpointKey,
        through: p::ReceiveProgress,
    ) -> p::BoxFuture<'_, ()> {
        Box::pin(async move {
            self.request(MetadataOperation::FinishSession { key, through })
                .await
        })
    }
}

impl<M: SyncModel> p::SnapshotSource<M> for MetadataPeer<M> {
    fn release_request(&self, request_id: Uuid) -> p::BoxFuture<'_, ()> {
        Box::pin(async move {
            self.request(MetadataOperation::ReleaseRequest { request_id })
                .await
        })
    }
    fn open_snapshot(
        &self,
        request_id: Uuid,
        scope: p::Scope<M>,
        limits: p::WorkLimits,
        resume: Option<p::WorkCursor>,
    ) -> p::BoxFuture<'_, p::WorkOutcome<p::SnapshotDescriptor<M>>> {
        Box::pin(async move {
            self.request(MetadataOperation::OpenSnapshot {
                request_id,
                scope,
                limits,
                resume,
            })
            .await
        })
    }
    fn summarize(
        &self,
        snapshot: Uuid,
        range: p::KeyRange<M>,
        limits: p::WorkLimits,
        resume: Option<p::WorkCursor>,
    ) -> p::BoxFuture<'_, p::WorkOutcome<p::RangeSummary<M>>> {
        Box::pin(async move {
            self.request(MetadataOperation::Summarize {
                snapshot,
                range,
                limits,
                resume,
            })
            .await
        })
    }
    fn split(
        &self,
        snapshot: Uuid,
        range: p::KeyRange<M>,
        limits: p::WorkLimits,
        resume: Option<p::WorkCursor>,
    ) -> p::BoxFuture<'_, p::WorkOutcome<Vec<p::KeyRange<M>>>> {
        Box::pin(async move {
            self.request(MetadataOperation::Split {
                snapshot,
                range,
                limits,
                resume,
            })
            .await
        })
    }
    fn leaf(
        &self,
        snapshot: Uuid,
        range: p::KeyRange<M>,
        after: Option<p::CandidateCursor<M>>,
        limits: p::PageLimits,
    ) -> p::BoxFuture<'_, p::LeafPage<M>> {
        Box::pin(async move {
            self.request(MetadataOperation::Leaf {
                snapshot,
                range,
                after,
                limits,
            })
            .await
        })
    }
    fn read_fragment(
        &self,
        snapshot: Uuid,
        manifest: p::ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> p::BoxFuture<'_, p::Fragment> {
        Box::pin(async move {
            self.request(MetadataOperation::SnapshotFragment {
                snapshot,
                manifest,
                ordinal,
                max_bytes,
            })
            .await
        })
    }
    fn close_snapshot(&self, snapshot: Uuid) -> p::BoxFuture<'_, ()> {
        Box::pin(async move {
            self.request(MetadataOperation::CloseSnapshot { snapshot })
                .await
        })
    }
    fn cancel_snapshot_build(&self, cursor: p::WorkCursor) -> p::BoxFuture<'_, ()> {
        Box::pin(async move {
            self.request(MetadataOperation::CancelSnapshotBuild { cursor })
                .await
        })
    }
}

impl<M: SyncModel> p::SnapshotSink<M> for MetadataPeer<M> {
    fn cancel_reconciliation(&self, session: Uuid) -> p::BoxFuture<'_, ()> {
        Box::pin(async move {
            self.request(MetadataOperation::CancelReconciliation { session })
                .await
        })
    }
    fn discard_range(&self, session: Uuid, range: p::KeyRange<M>) -> p::BoxFuture<'_, ()> {
        Box::pin(async move {
            self.request(MetadataOperation::DiscardRange { session, range })
                .await
        })
    }
    fn begin_reconciliation(
        &self,
        request_id: Uuid,
        remote: p::SnapshotDescriptor<M>,
        local: p::SnapshotDescriptor<M>,
    ) -> p::BoxFuture<'_, Uuid> {
        Box::pin(async move {
            self.request(MetadataOperation::BeginReconciliation {
                request_id,
                remote,
                local,
            })
            .await
        })
    }
    fn coverage(
        &self,
        session: Uuid,
        ledger_revision: Option<u64>,
        after: u64,
        limits: p::PageLimits,
    ) -> p::BoxFuture<'_, p::CoveragePage<M>> {
        Box::pin(async move {
            self.request(MetadataOperation::Coverage {
                session,
                ledger_revision,
                after,
                limits,
            })
            .await
        })
    }
    fn split_range(
        &self,
        session: Uuid,
        parent: p::KeyRange<M>,
        children: Vec<p::RangeSummary<M>>,
    ) -> p::BoxFuture<'_, p::CoverageProgress> {
        Box::pin(async move {
            self.request(MetadataOperation::SplitRange {
                session,
                parent,
                children,
            })
            .await
        })
    }
    fn accept_equal_range(
        &self,
        session: Uuid,
        source: p::RangeSummary<M>,
    ) -> p::BoxFuture<'_, p::CoverageProgress> {
        Box::pin(async move {
            self.request(MetadataOperation::AcceptEqualRange { session, source })
                .await
        })
    }
    fn leaf_progress(
        &self,
        session: Uuid,
        range: p::KeyRange<M>,
    ) -> p::BoxFuture<'_, p::LeafProgress<M>> {
        Box::pin(async move {
            self.request(MetadataOperation::LeafProgress { session, range })
                .await
        })
    }
    fn stage_leaf_page(
        &self,
        session: Uuid,
        page: p::LeafPage<M>,
    ) -> p::BoxFuture<'_, p::LeafProgress<M>> {
        Box::pin(async move {
            self.request(MetadataOperation::StageLeafPage { session, page })
                .await
        })
    }
    fn commit_complete_range(
        &self,
        session: Uuid,
        range: p::KeyRange<M>,
        limits: p::ApplyLimits,
    ) -> p::BoxFuture<'_, (p::CoverageProgress, p::ProjectionBatch<M>)> {
        Box::pin(async move {
            self.request(MetadataOperation::CommitCompleteRange {
                session,
                range,
                limits,
            })
            .await
        })
    }
    fn promote_step(
        &self,
        session: Uuid,
        limits: p::ApplyLimits,
    ) -> p::BoxFuture<'_, p::PromoteProgress> {
        Box::pin(async move {
            self.request(MetadataOperation::PromoteStep { session, limits })
                .await
        })
    }
    fn complete_reconciliation(&self, session: Uuid) -> p::BoxFuture<'_, p::ReceiveProgress> {
        Box::pin(async move {
            self.request(MetadataOperation::CompleteReconciliation { session })
                .await
        })
    }
}

/// Continuation for a bounded pull followed by a bounded push.
pub struct BidirectionalSession<M: SyncModel> {
    pull: sync::Coordinator<M>,
    push: sync::Coordinator<M>,
    pulled: Option<p::ReceiveProgress>,
    pushed: Option<p::ReceiveProgress>,
}

impl<M: SyncModel> BidirectionalSession<M> {
    pub fn new(scope: p::Scope<M>, limits: sync::SessionLimits) -> p::Result<Self> {
        Ok(Self {
            pull: sync::Coordinator::new(scope.clone(), limits)?,
            push: sync::Coordinator::new(scope, limits)?,
            pulled: None,
            pushed: None,
        })
    }

    pub async fn advance<L>(&mut self, local: &L, remote: &MetadataPeer<M>) -> p::Result<bool>
    where
        L: p::ReceiveStore<M> + p::SnapshotSink<M> + p::SnapshotSource<M> + p::SyncPeer<M>,
    {
        self.pull.start_pass();
        self.push.start_pass();
        while self.pulled.is_none() {
            match self
                .pull
                .advance(local, local, local, remote, remote)
                .await?
            {
                sync::SessionAdvance::Complete(progress) => self.pulled = Some(progress),
                sync::SessionAdvance::BudgetExhausted => return Ok(false),
                sync::SessionAdvance::Incomplete => (),
                sync::SessionAdvance::Cancelled => {
                    return Err(p::SyncError::Transport("metadata session cancelled".into()));
                }
            }
        }
        while self.pushed.is_none() {
            match self
                .push
                .advance(remote, remote, remote, local, local)
                .await?
            {
                sync::SessionAdvance::Complete(progress) => self.pushed = Some(progress),
                sync::SessionAdvance::BudgetExhausted => return Ok(false),
                sync::SessionAdvance::Incomplete => (),
                sync::SessionAdvance::Cancelled => {
                    return Err(p::SyncError::Transport("metadata session cancelled".into()));
                }
            }
        }
        Ok(true)
    }

    pub fn progress(&self) -> Option<(p::ReceiveProgress, p::ReceiveProgress)> {
        Some((self.pulled.clone()?, self.pushed.clone()?))
    }
}

/// Read a body of the advertised length, respecting the caller's deadline.
pub async fn fetch_bounded(
    client: &reqwest::Client,
    url: &str,
    expected: u64,
    deadline: tokio::time::Instant,
) -> anyhow::Result<Vec<u8>> {
    use anyhow::ensure;
    tokio::time::timeout_at(deadline, async {
        let mut response = client.get(url).send().await?.error_for_status()?;
        if let Some(length) = response.content_length() {
            ensure!(length == expected, "response size mismatch");
        }
        let capacity = usize::try_from(expected)?;
        let mut bytes = Vec::with_capacity(capacity);
        while let Some(part) = response.chunk().await? {
            ensure!(
                part.len() <= capacity.saturating_sub(bytes.len()),
                "response exceeds expected size"
            );
            bytes.extend_from_slice(&part);
        }
        ensure!(bytes.len() == capacity, "response size mismatch");
        Ok(bytes)
    })
    .await
    .map_err(|_| anyhow::anyhow!("request timed out"))?
}

/// A bounded traversal can resume from the last item it visited.
#[derive(Default)]
pub struct PageCursor {
    after: Option<Uuid>,
    complete: bool,
}
impl PageCursor {
    pub fn is_complete(&self) -> bool {
        self.complete
    }
}

/// Visit one ordered page, retaining progress when the visitor stops early.
pub async fn visit_page<T, P, PF, F, FF>(
    cursor: &mut PageCursor,
    page: P,
    mut visit: F,
) -> anyhow::Result<()>
where
    P: FnOnce(Option<Uuid>) -> PF,
    PF: Future<Output = anyhow::Result<(Vec<(Uuid, T)>, Option<Uuid>, bool)>>,
    F: FnMut(Uuid, T) -> FF,
    FF: Future<Output = anyhow::Result<std::ops::ControlFlow<()>>>,
{
    if cursor.complete {
        return Ok(());
    }
    let previous = cursor.after;
    let (records, next, done) = page(previous).await?;
    for (id, value) in records {
        if visit(id, value).await?.is_break() {
            return Ok(());
        }
        cursor.after = Some(id);
    }
    if done {
        cursor.complete = true;
    } else {
        anyhow::ensure!(next != previous && next.is_some(), "page made no progress");
        cursor.after = next;
    }
    Ok(())
}

#[cfg(test)]
mod traversal_tests {
    use super::{PageCursor, visit_page};
    use std::ops::ControlFlow;
    use uuid::Uuid;

    #[tokio::test]
    async fn page_visit_resumes_and_rejects_stalled_cursor() {
        let mut cursor = PageCursor::default();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        visit_page(
            &mut cursor,
            |after| async move {
                assert_eq!(after, None);
                Ok((vec![(a, ()), (b, ())], Some(b), false))
            },
            |id, _| async move {
                Ok(if id == b {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                })
            },
        )
        .await
        .unwrap();
        assert!(!cursor.is_complete());
        visit_page(
            &mut cursor,
            |after| async move {
                assert_eq!(after, Some(a));
                Ok((vec![(b, ())], None, true))
            },
            |_, _| async { Ok(ControlFlow::Continue(())) },
        )
        .await
        .unwrap();
        assert!(cursor.is_complete());

        let mut stalled = PageCursor::default();
        assert!(
            visit_page(
                &mut stalled,
                |_| async { Ok((Vec::<(Uuid, ())>::new(), None, false)) },
                |_, _| async { Ok(ControlFlow::Continue(())) }
            )
            .await
            .is_err()
        );
    }
    #[tokio::test]
    async fn bounded_pages_reach_later_items_after_isolated_failure() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut cursor = PageCursor::default();
        let visited = AtomicUsize::new(0);
        let failures = AtomicUsize::new(0);
        while !cursor.is_complete() {
            visit_page(
                &mut cursor,
                |after| async move {
                    let start = after.map_or(1, |id| id.as_u128() + 1);
                    let end = (start + 127).min(130);
                    let records = (start..=end).map(|n| (Uuid::from_u128(n), n)).collect();
                    Ok((records, Some(Uuid::from_u128(end)), end == 130))
                },
                |_, n| {
                    let visited = &visited;
                    let failures = &failures;
                    async move {
                        visited.fetch_add(1, Ordering::Relaxed);
                        if n == 1 {
                            failures.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(ControlFlow::Continue(()))
                    }
                },
            )
            .await
            .unwrap();
        }
        assert_eq!(visited.load(Ordering::Relaxed), 130);
        assert_eq!(failures.load(Ordering::Relaxed), 1);
    }
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    use p::{ReceiveStore, SnapshotSource, SyncStore};
    use tokio::net::TcpListener;

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Model;
    impl SyncModel for Model {
        type Collection = u8;
        type Payload = String;
        fn model_id() -> &'static str {
            "http-test"
        }
        fn schema_id() -> &'static str {
            "one"
        }
        fn collections() -> Vec<u8> {
            vec![1]
        }
        fn collection_tag(_: &u8) -> &'static str {
            "notes"
        }
        fn collection_from_tag(tag: &str) -> Option<u8> {
            (tag == "notes").then_some(1)
        }
        fn validate(_: &p::ObjectKey<Self>, _: &Option<String>) -> p::Result<()> {
            Ok(())
        }
    }
    fn store(replica: u128) -> Arc<p::MemorySyncStore<Model>> {
        Arc::new(
            p::MemorySyncStore::new(
                p::StoreIdentity {
                    namespace: Uuid::from_u128(1),
                    model: p::ModelId {
                        model: "http-test".into(),
                        schema: "one".into(),
                    },
                    replica: Uuid::from_u128(replica),
                    generation: Uuid::from_u128(10),
                },
                p::LocalAuthor {
                    author: Uuid::from_u128(replica),
                    writer_incarnation: Uuid::from_u128(replica + 100),
                },
            )
            .unwrap(),
        )
    }
    fn limits() -> p::SessionLimits {
        p::SessionLimits {
            pages: p::PageLimits {
                max_scanned: 128,
                max_bytes: p::DEFAULT_PAGE_BYTES,
            },
            stages: p::StageLimits {
                max_total_bytes: p::DEFAULT_STAGE_BYTES,
                max_stages: p::DEFAULT_STAGES,
                max_event_bytes: p::DEFAULT_LOGICAL_BYTES,
                max_event_items: p::DEFAULT_LOGICAL_ITEMS,
                max_fragment_bytes: p::DEFAULT_PAGE_BYTES,
            },
            apply: p::ApplyLimits {
                max_projection_items: 128,
                max_projection_bytes: p::DEFAULT_PROJECTION_BYTES,
                max_mutations: 1000,
            },
            work: p::WorkLimits {
                max_requests: 1000,
                max_bytes: p::DEFAULT_ROUND_BYTES,
                max_depth: 10,
                max_rows_scanned: 1000,
                max_mutations: 1000,
            },
        }
    }
    async fn serve(router: Router) -> (MetadataPeer<Model>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (
            MetadataPeer::new(format!("http://{address}"), reqwest::Client::new()),
            task,
        )
    }
    #[tokio::test]
    async fn metadata_response_errors_preserve_origin_and_unauthorized_type() {
        for (status, body, expected) in [
            (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                p::SyncError::Unauthorized,
            ),
            (StatusCode::OK, "{", p::SyncError::Invalid(String::new())),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                p::SyncError::Transport(String::new()),
            ),
            (
                StatusCode::OK,
                r#"{"Err":{"kind":"storage","data":"local corruption"}}"#,
                p::SyncError::Storage("local corruption".into()),
            ),
        ] {
            let router = Router::new().route(
                "/sync/metadata",
                post(move || async move { (status, body) }),
            );
            let (peer, task) = serve(router).await;
            let result = p::SyncPeer::identity(&peer).await.unwrap_err();
            assert_eq!(
                std::mem::discriminant(&result),
                std::mem::discriminant(&expected)
            );
            task.abort();
            let _ = task.await;
        }
    }
    #[test]
    fn promotion_wire_shape_replaces_context_page_operations() {
        let operation = MetadataOperation::<Model>::PromoteStep {
            session: Uuid::from_u128(1),
            limits: limits().apply,
        };
        let value = serde_json::to_value(operation).unwrap();
        assert_eq!(value["operation"], "promote_step");
        assert!(value.get("resume").is_none());
        assert!(value["limits"].get("max_projection_items").is_some());
        for name in [
            "context_page",
            "context_progress",
            "stage_context_page",
            "normalize_step",
        ] {
            let value = serde_json::json!({"operation":name});
            assert!(serde_json::from_value::<MetadataOperation<Model>>(value).is_err());
        }
    }
    #[tokio::test]
    async fn snapshot_context_and_promotion_roundtrip_over_http() {
        let a = store(2);
        let b = store(3);
        let key = p::ObjectKey {
            collection: 1,
            id: Uuid::from_u128(42),
        };
        // This context-only source requires promotion even though every range is equal.
        a.write_local(key.clone(), Some("deleted".into()))
            .await
            .unwrap();
        a.write_local(key.clone(), None).await.unwrap();
        let (remote, task) = serve(router(b.clone())).await;
        let peer = p::StorePeer(a.clone());
        let scope = p::Scope {
            collections: [1].into_iter().collect(),
        };
        let mut coordinator = p::Coordinator::new(scope.clone(), limits()).unwrap();
        let mut complete = false;
        for _ in 0..100 {
            coordinator.start_pass();
            if matches!(
                coordinator
                    .advance(&remote, &remote, &remote, &peer, &*a)
                    .await
                    .unwrap(),
                p::SessionAdvance::Complete(_)
            ) {
                complete = true;
                break;
            }
        }
        assert!(complete);
        assert_eq!(b.head().await.unwrap().sequence, 1);
        assert!(b.get(&key).await.unwrap().is_none());
        let p::WorkOutcome::Ready(snapshot) = remote
            .open_snapshot(Uuid::new_v4(), scope, limits().work, None)
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(snapshot.contexts.len(), 1);
        let fragment = SnapshotSource::read_fragment(
            &remote,
            snapshot.snapshot_id,
            snapshot.contexts[0].context.manifest.clone(),
            0,
            p::DEFAULT_PAGE_BYTES,
        )
        .await
        .unwrap();
        let context =
            p::causal_context(&p::decode_context_remote(&fragment.bytes).unwrap()).unwrap();
        assert!(context.contains(sync::causal::Dot {
            writer: Uuid::from_u128(102),
            counter: 1
        }));
        remote.close_snapshot(snapshot.snapshot_id).await.unwrap();
        // A stale live candidate is suppressed after that HTTP promotion.
        let delta = p::MergeDelta::<Model> {
            collection: 1,
            added: vec![p::Candidate {
                model: p::ModelId {
                    model: "http-test".into(),
                    schema: "one".into(),
                },
                namespace: Uuid::from_u128(1),
                key: key.clone(),
                dot: p::Dot {
                    writer_incarnation: Uuid::from_u128(102),
                    counter: 1,
                },
                revision: p::Revision {
                    counter: 1,
                    actor: Uuid::from_u128(2),
                },
                payload: "deleted".into(),
            }],
            observed: p::ExactContext { writers: vec![] },
        };
        let event = p::Event::Merge(Box::new(delta));
        let identity = p::ReceiveStore::identity(&*a).await.unwrap();
        let after = a.head().await.unwrap();
        let digest = p::event_digest(&event).unwrap();
        let next = p::ReceiveProgress {
            sequence: after.sequence + 1,
            anchor: p::next_anchor::<Model>(after.anchor, after.sequence + 1, &1, digest).unwrap(),
        };
        let page = p::ChangePage {
            request: p::PageRequest {
                source: identity,
                scope: p::Scope {
                    collections: [1].into_iter().collect(),
                },
                after,
                through: next.clone(),
                limits: limits().pages,
            },
            next,
            done: true,
            links: vec![p::ScanLink {
                sequence: 3,
                collection: 1,
                event_digest: digest,
                event: Some(p::EventRef::Inline(event)),
            }],
        };
        remote.apply_page(&page, limits().apply).await.unwrap();
        assert!(b.get(&key).await.unwrap().is_none());
        assert_eq!(b.head().await.unwrap().sequence, 1);
        task.abort();
        let _ = task.await;
    }
}
