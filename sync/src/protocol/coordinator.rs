//! Resumable directional coordinator for the causal store capabilities.
use super::*;

/// Resource limits remain explicit at every read, receive, and comparison.
#[derive(Clone, Copy, Debug)]
pub struct SessionLimits {
    pub pages: PageLimits,
    pub stages: StageLimits,
    pub apply: ApplyLimits,
    pub work: WorkLimits,
}

/// The coordinator keeps its continuation between calls; no transaction spans calls.
pub struct Coordinator<M: SyncModel> {
    pub scope: Scope<M>,
    pub limits: SessionLimits,
    source: Option<StoreIdentity>,
    remote_request_id: Uuid,
    local_request_id: Uuid,
    reconciliation_request_id: Uuid,
    key: Option<CheckpointKey>,
    progress: Option<ReceiveProgress>,
    through: Option<ReceiveProgress>,
    remote_snapshot: Option<SnapshotDescriptor<M>>,
    local_snapshot: Option<SnapshotDescriptor<M>>,
    reconciliation: Option<Uuid>,
    queue: Vec<RangeSummary<M>>,
    depths: Vec<u32>,
    child_ranges: Vec<KeyRange<M>>,
    child_summaries: Vec<RangeSummary<M>>,
    child_index: usize,
    contexts: Vec<(M::Collection, ManifestRef)>,
    restarts: u8,
    leaf_cursor: Option<CandidateCursor<M>>,
    stage: Option<Uuid>,
    graph_stage: Option<Uuid>,
    ordinal: u32,
    must_split: bool,
    journal: Option<JournalTransfer<M>>,
    object_bytes: Vec<u8>,
    recovery: Recovery,
    pass: PassUsage,
    phase: Phase<M>,
}
#[derive(Clone)]
enum Phase<M: SyncModel> {
    Source,
    Receiver,
    Checkpoint,
    History,
    OpenRemote(Option<WorkCursor>),
    OpenLocal(Option<WorkCursor>),
    Begin,
    Compare(Option<WorkCursor>),
    Split {
        remote: bool,
        cursor: Option<WorkCursor>,
    },
    SummarizeChild(Option<WorkCursor>),
    BindSplit,
    Equal,
    BeginObject,
    FetchObject,
    PutObject(Fragment),
    Leaf,
    StageLeaf(LeafPage<M>),
    Commit,
    DiscardRange,
    Promote,
    CompleteSnapshot,
    CloseRemote,
    CloseLocal,
    CaptureHead,
    Read,
    PreparePage(ChangePage<M>),
    BeginJournalObject,
    FetchJournalObject,
    PutJournalObject(Fragment),
    CommitJournalEvent,
    Apply(ChangePage<M>),
    Finish,
    Finished,
    CleanupStage,
    CleanupSession,
    CleanupRemote,
    CleanupLocal,
    Restart,
    Aborted,
}
#[derive(Clone, Copy)]
enum Recovery {
    Reconcile,
    Refresh,
    Reidentify,
    Abort,
}
#[derive(Clone)]
struct JournalTransfer<M: SyncModel> {
    request: PageRequest<M>,
    sequence: u64,
    collection: M::Collection,
    digest: Digest,
    manifest: ManifestRef,
    event_stage: Option<Uuid>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionAdvance {
    Incomplete,
    BudgetExhausted,
    Complete(ReceiveProgress),
    Cancelled,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PassUsage {
    pub requests: u32,
    pub encoded_bytes: u64,
}
impl<M: SyncModel> Coordinator<M> {
    fn start_leaf(&mut self) {
        self.leaf_cursor = None;
        self.phase = Phase::Leaf;
    }
    fn after_contexts(&self) -> Phase<M> {
        if self.queue.is_empty() {
            Phase::Promote
        } else {
            Phase::Compare(None)
        }
    }
    #[cfg(test)]
    pub(crate) fn phase_label(&self) -> &'static str {
        match self.phase {
            Phase::Compare(_) => "compare",
            Phase::Promote => "promote",
            Phase::Finished => "finished",
            _ => "other",
        }
    }
    pub fn new(scope: Scope<M>, mut limits: SessionLimits) -> Result<Self> {
        scope_id(&scope)?;
        if limits.pages.max_scanned == 0
            || limits.pages.max_scanned > MAX_PAGE_ITEMS
            || limits.pages.max_bytes == 0
            || limits.pages.max_bytes > MAX_PAGE_BYTES
            || limits.work.max_requests == 0
            || limits.work.max_requests > MAX_ROUND_REQUESTS
            || limits.work.max_bytes == 0
            || limits.work.max_bytes > MAX_ROUND_BYTES
            || limits.work.max_depth == 0
            || limits.work.max_depth > MAX_DEPTH
            || limits.work.max_rows_scanned == 0
            || limits.work.max_rows_scanned > MAX_ROWS
            || limits.work.max_mutations == 0
            || limits.work.max_mutations > MAX_MUTATIONS
            || limits.apply.max_projection_items == 0
            || limits.apply.max_projection_items > MAX_PROJECTION_ITEMS
            || limits.apply.max_projection_bytes == 0
            || limits.apply.max_projection_bytes > MAX_PROJECTION_BYTES
            || limits.apply.max_mutations == 0
            || limits.apply.max_mutations > MAX_MUTATIONS
            || limits.stages.max_stages == 0
            || limits.stages.max_stages > MAX_STAGES
            || limits.stages.max_total_bytes == 0
            || limits.stages.max_total_bytes > MAX_STAGE_BYTES
            || limits.stages.max_event_bytes == 0
            || limits.stages.max_event_bytes > MAX_LOGICAL_BYTES
            || limits.stages.max_event_items == 0
            || limits.stages.max_event_items > MAX_LOGICAL_ITEMS
            || limits.stages.max_fragment_bytes == 0
            || limits.stages.max_fragment_bytes > MAX_PAGE_BYTES
        {
            return Err(SyncError::Invalid("invalid session limits".into()));
        }
        if limits.pages.max_bytes > limits.work.max_bytes
            || limits.apply.max_mutations > limits.work.max_mutations
        {
            return Err(SyncError::Invalid(
                "session operation exceeds work budget".into(),
            ));
        }
        // Leave room for both encoded request and response in a bounded pass.
        let transferable = limits.work.max_bytes.saturating_sub(8192) / 2;
        if transferable == 0 {
            return Err(SyncError::LimitTooSmall { required: 8194 });
        }
        limits.pages.max_bytes = limits.pages.max_bytes.min(transferable);
        limits.stages.max_fragment_bytes = limits.stages.max_fragment_bytes.min(transferable);
        limits.apply.max_projection_bytes = limits
            .apply
            .max_projection_bytes
            .min(transferable / 2)
            .max(1);
        limits.apply.max_projection_items = limits
            .apply
            .max_projection_items
            .min(u32::try_from((transferable / 1024).max(1)).unwrap_or(u32::MAX));
        Ok(Self {
            scope,
            limits,
            source: None,
            remote_request_id: Uuid::new_v4(),
            local_request_id: Uuid::new_v4(),
            reconciliation_request_id: Uuid::new_v4(),
            key: None,
            progress: None,
            through: None,
            remote_snapshot: None,
            local_snapshot: None,
            reconciliation: None,
            queue: Vec::new(),
            depths: Vec::new(),
            child_ranges: Vec::new(),
            child_summaries: Vec::new(),
            child_index: 0,
            contexts: Vec::new(),
            restarts: 0,
            leaf_cursor: None,
            stage: None,
            graph_stage: None,
            ordinal: 0,
            must_split: false,
            journal: None,
            object_bytes: Vec::new(),
            recovery: Recovery::Refresh,
            pass: PassUsage::default(),
            phase: Phase::Source,
        })
    }
    /// Each call performs at most one endpoint operation. Caller may persist or
    /// abandon this continuation without affecting any committed checkpoint.
    pub fn start_abort(&mut self) {
        self.recovery = Recovery::Abort;
        self.phase = Phase::CleanupStage;
    }
    pub fn start_pass(&mut self) {
        self.pass = PassUsage::default();
    }
    pub fn pass_usage(&self) -> PassUsage {
        self.pass
    }
    pub async fn advance(
        &mut self,
        receiver: &dyn ReceiveStore<M>,
        sink: &dyn SnapshotSink<M>,
        local_source: &dyn SnapshotSource<M>,
        peer: &dyn SyncPeer<M>,
        remote_source: &dyn SnapshotSource<M>,
    ) -> Result<SessionAdvance> {
        match self
            .advance_inner(receiver, sink, local_source, peer, remote_source)
            .await
        {
            Err(SyncError::RetryAt(_)) => {
                self.recovery = Recovery::Refresh;
                self.phase = Phase::CleanupStage;
                Ok(SessionAdvance::Incomplete)
            }
            Err(SyncError::NeedsReconciliation(_)) => {
                self.recovery = Recovery::Reconcile;
                self.phase = Phase::CleanupStage;
                Ok(SessionAdvance::Incomplete)
            }
            Err(SyncError::SnapshotExpired | SyncError::StaleWork)
                if !matches!(
                    self.phase,
                    Phase::CleanupStage
                        | Phase::CleanupSession
                        | Phase::CleanupRemote
                        | Phase::CleanupLocal
                        | Phase::Restart
                ) =>
            {
                self.recovery = Recovery::Reidentify;
                self.phase = Phase::CleanupStage;
                Ok(SessionAdvance::Incomplete)
            }
            Err(SyncError::WorkLimit)
                if matches!(
                    self.phase,
                    Phase::Apply(_) | Phase::CommitJournalEvent | Phase::Promote
                ) =>
            {
                self.recovery = Recovery::Reconcile;
                self.phase = Phase::CleanupStage;
                Ok(SessionAdvance::Incomplete)
            }
            Err(SyncError::WorkLimit | SyncError::LogicalLimit)
                if matches!(self.phase, Phase::StageLeaf(_) | Phase::Commit)
                    && self
                        .depths
                        .first()
                        .is_some_and(|depth| *depth < self.limits.work.max_depth) =>
            {
                self.must_split = true;
                self.phase = Phase::DiscardRange;
                Ok(SessionAdvance::Incomplete)
            }
            result => result,
        }
    }
    async fn advance_inner(
        &mut self,
        receiver: &dyn ReceiveStore<M>,
        sink: &dyn SnapshotSink<M>,
        local_source: &dyn SnapshotSource<M>,
        peer: &dyn SyncPeer<M>,
        remote_source: &dyn SnapshotSource<M>,
    ) -> Result<SessionAdvance> {
        // One operation per advance. Charge canonical JSON argument and result
        // bytes, including errors; reserve the maximum response before sending.
        macro_rules! endpoint_result {
            ($args:expr, $response_cap:expr, $future:expr) => {{
                let request_bytes = serde_json::to_vec(&$args)
                    .map_err(|e| SyncError::Storage(e.to_string()))?
                    .len() as u64;
                let reserve = request_bytes
                    .checked_add($response_cap)
                    .ok_or(SyncError::WorkLimit)?;
                if reserve > self.limits.work.max_bytes {
                    return Err(SyncError::LimitTooSmall { required: reserve });
                }
                if self.pass.requests >= self.limits.work.max_requests
                    || self
                        .pass
                        .encoded_bytes
                        .checked_add(reserve)
                        .is_none_or(|v| v > self.limits.work.max_bytes)
                {
                    return Ok(SessionAdvance::BudgetExhausted);
                }
                self.pass.requests += 1;
                self.pass.encoded_bytes += request_bytes;
                let response = $future.await;
                let actual = serde_json::to_vec(&response)
                    .map_err(|e| SyncError::Storage(e.to_string()))?
                    .len() as u64;
                self.pass.encoded_bytes = self
                    .pass
                    .encoded_bytes
                    .checked_add(actual)
                    .ok_or(SyncError::WorkLimit)?;
                if actual > $response_cap {
                    return match response {
                        Err(error) => Err(error),
                        Ok(_) => Err(SyncError::LimitTooSmall { required: actual }),
                    };
                }
                response
            }};
        }
        macro_rules! endpoint {
            ($args:expr, $response_cap:expr, $future:expr) => {
                endpoint_result!($args, $response_cap, $future)?
            };
        }
        match self.phase.clone() {
            Phase::Source => {
                let source = endpoint!(("identity",), 4096, peer.identity());
                self.source = Some(source);
                self.phase = Phase::Receiver;
            }
            Phase::Receiver => {
                let receiver_id = endpoint!(("identity",), 4096, receiver.identity());
                let source = self.source.as_ref().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                if receiver_id.namespace != source.namespace
                    || receiver_id.model != source.model
                    || (receiver_id.replica == source.replica
                        && receiver_id.generation == source.generation)
                {
                    return Err(SyncError::IncompatibleModel);
                }
                self.key = Some(CheckpointKey {
                    namespace: source.namespace,
                    source_replica: source.replica,
                    source_generation: source.generation,
                    scope_id: scope_id(&self.scope)?,
                });
                self.phase = Phase::Checkpoint;
            }
            Phase::Checkpoint => {
                let key = self.key.clone().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                self.progress = endpoint!(("checkpoint", &key), 4096, receiver.checkpoint(&key));
                self.phase = Phase::History;
            }
            Phase::History => {
                let key = self.key.clone().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let progress = self.progress.clone();
                let status = endpoint!(
                    ("history", &key, &progress),
                    4096,
                    peer.history_status(&key, progress)
                );
                match status {
                    HistoryStatus::Continuous { head, .. } => {
                        self.through = Some(head);
                        self.phase = Phase::Read
                    }
                    HistoryStatus::GenerationChanged { .. } => {
                        self.recovery = Recovery::Reidentify;
                        self.phase = Phase::CleanupStage;
                    }
                    _ => {
                        self.remote_request_id = Uuid::new_v4();
                        self.local_request_id = Uuid::new_v4();
                        self.reconciliation_request_id = Uuid::new_v4();
                        self.phase = Phase::OpenRemote(None);
                    }
                }
            }
            Phase::OpenRemote(cursor) => match endpoint!(
                (
                    "open",
                    self.remote_request_id,
                    &self.scope,
                    self.limits.work,
                    &cursor
                ),
                self.limits.pages.max_bytes.saturating_add(256),
                remote_source.open_snapshot(
                    self.remote_request_id,
                    self.scope.clone(),
                    self.limits.work,
                    cursor,
                )
            ) {
                WorkOutcome::Ready(snapshot) => {
                    self.remote_snapshot = Some(snapshot);
                    self.phase = Phase::OpenLocal(None);
                }
                WorkOutcome::Incomplete { cursor } => self.phase = Phase::OpenRemote(Some(cursor)),
            },
            Phase::OpenLocal(cursor) => match endpoint!(
                (
                    "open",
                    self.local_request_id,
                    &self.scope,
                    self.limits.work,
                    &cursor
                ),
                self.limits.pages.max_bytes.saturating_add(256),
                local_source.open_snapshot(
                    self.local_request_id,
                    self.scope.clone(),
                    self.limits.work,
                    cursor,
                )
            ) {
                WorkOutcome::Ready(snapshot) => {
                    self.local_snapshot = Some(snapshot);
                    self.phase = Phase::Begin;
                }
                WorkOutcome::Incomplete { cursor } => self.phase = Phase::OpenLocal(Some(cursor)),
            },
            Phase::Begin => {
                let remote = self.remote_snapshot.clone().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let local = self.local_snapshot.clone().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let request_id = self.reconciliation_request_id;
                self.reconciliation = Some(request_id);
                let confirmed = endpoint!(
                    ("begin", request_id, &remote, &local),
                    4096,
                    sink.begin_reconciliation(request_id, remote.clone(), local.clone())
                );
                if confirmed != request_id {
                    return Err(SyncError::Invalid(
                        "reconciliation request id mismatch".into(),
                    ));
                }
                self.queue = remote.roots;
                self.depths = vec![0; self.queue.len()];
                self.contexts = remote
                    .contexts
                    .into_iter()
                    .map(|c| (c.collection, c.context.manifest))
                    .collect();
                self.phase = if self.contexts.is_empty() {
                    self.after_contexts()
                } else {
                    Phase::BeginObject
                };
            }
            Phase::Compare(cursor) => {
                let item = self.queue.first().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let local = self.local_snapshot.as_ref().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                match endpoint!(
                    (
                        "summarize",
                        local.snapshot_id,
                        &item.range,
                        self.limits.work,
                        &cursor
                    ),
                    self.limits.pages.max_bytes.saturating_add(256),
                    local_source.summarize(
                        local.snapshot_id,
                        item.range.clone(),
                        self.limits.work,
                        cursor.clone()
                    )
                ) {
                    WorkOutcome::Incomplete { cursor } => self.phase = Phase::Compare(Some(cursor)),
                    WorkOutcome::Ready(summary) => {
                        if summary.aggregate == item.aggregate
                            && summary.candidate_count == item.candidate_count
                            && summary.candidate_bytes == item.candidate_bytes
                        {
                            self.phase = Phase::Equal;
                        } else if (item.candidate_count.max(summary.candidate_count)
                            > u64::from(self.limits.apply.max_projection_items.min(128))
                            || item.candidate_bytes.max(summary.candidate_bytes)
                                > DEFAULT_PAGE_BYTES
                                    .min(self.limits.apply.max_projection_bytes / 2)
                                    .max(1))
                            && self.depths[0] < self.limits.work.max_depth
                        {
                            let remote = (item.candidate_bytes, item.candidate_count)
                                >= (summary.candidate_bytes, summary.candidate_count);
                            self.phase = Phase::Split {
                                remote,
                                cursor: None,
                            };
                        } else {
                            self.start_leaf();
                        }
                    }
                }
            }
            Phase::Split { remote, cursor } => {
                let range = self
                    .queue
                    .first()
                    .ok_or_else(|| {
                        SyncError::Invalid("coordinator invariant: missing state".into())
                    })?
                    .range
                    .clone();
                let source = if remote { remote_source } else { local_source };
                let id = if remote {
                    self.remote_snapshot.as_ref()
                } else {
                    self.local_snapshot.as_ref()
                }
                .ok_or_else(|| SyncError::Invalid("coordinator invariant: missing state".into()))?
                .snapshot_id;
                match endpoint!(
                    ("split", id, &range, self.limits.work, &cursor),
                    self.limits.pages.max_bytes.saturating_add(256),
                    source.split(id, range.clone(), self.limits.work, cursor.clone())
                ) {
                    WorkOutcome::Incomplete { cursor } => {
                        self.phase = Phase::Split {
                            remote,
                            cursor: Some(cursor),
                        }
                    }
                    WorkOutcome::Ready(children) if children.len() < 2 => {
                        if self.must_split {
                            if remote {
                                self.phase = Phase::Split {
                                    remote: false,
                                    cursor: None,
                                };
                                return Ok(SessionAdvance::Incomplete);
                            }
                            return Err(SyncError::LogicalLimit);
                        }
                        self.start_leaf()
                    }
                    WorkOutcome::Ready(children) => {
                        self.child_ranges = children;
                        self.child_summaries.clear();
                        self.child_index = 0;
                        self.phase = Phase::SummarizeChild(None);
                    }
                }
            }
            Phase::SummarizeChild(cursor) => {
                let range = self
                    .child_ranges
                    .get(self.child_index)
                    .ok_or_else(|| {
                        SyncError::Invalid("coordinator invariant: missing state".into())
                    })?
                    .clone();
                let snapshot = self
                    .remote_snapshot
                    .as_ref()
                    .ok_or_else(|| {
                        SyncError::Invalid("coordinator invariant: missing state".into())
                    })?
                    .snapshot_id;
                match endpoint!(
                    ("summarize", snapshot, &range, self.limits.work, &cursor),
                    self.limits.pages.max_bytes.saturating_add(256),
                    remote_source.summarize(
                        snapshot,
                        range.clone(),
                        self.limits.work,
                        cursor.clone()
                    )
                ) {
                    WorkOutcome::Incomplete { cursor } => {
                        self.phase = Phase::SummarizeChild(Some(cursor))
                    }
                    WorkOutcome::Ready(summary) => {
                        self.child_summaries.push(summary);
                        self.child_index += 1;
                        self.phase = if self.child_index == self.child_ranges.len() {
                            Phase::BindSplit
                        } else {
                            Phase::SummarizeChild(None)
                        };
                    }
                }
            }
            Phase::BindSplit => {
                let parent = self
                    .queue
                    .first()
                    .ok_or_else(|| {
                        SyncError::Invalid("coordinator invariant: missing state".into())
                    })?
                    .range
                    .clone();
                let session = self.reconciliation.ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                endpoint!(
                    ("split_range", session, &parent, &self.child_summaries),
                    4096,
                    sink.split_range(session, parent.clone(), self.child_summaries.clone())
                );
                let depth = self
                    .depths
                    .remove(0)
                    .checked_add(1)
                    .ok_or(SyncError::WorkLimit)?;
                self.queue.remove(0);
                self.depths
                    .splice(0..0, std::iter::repeat_n(depth, self.child_summaries.len()));
                self.queue.splice(0..0, self.child_summaries.drain(..));
                self.must_split = false;
                self.phase = Phase::Compare(None);
            }
            Phase::Equal => {
                let session = self.reconciliation.ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let summary = self
                    .queue
                    .first()
                    .ok_or_else(|| {
                        SyncError::Invalid("coordinator invariant: missing state".into())
                    })?
                    .clone();
                endpoint!(
                    ("accept_equal", session, &summary),
                    4096,
                    sink.accept_equal_range(session, summary.clone())
                );
                self.queue.remove(0);
                self.depths.remove(0);
                self.phase = self.after_contexts();
            }
            Phase::BeginObject => {
                let (collection, manifest) = self
                    .contexts
                    .first()
                    .ok_or_else(|| {
                        SyncError::Invalid("coordinator invariant: missing state".into())
                    })?
                    .clone();
                let remote = self.remote_snapshot.as_ref().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let descriptor = StageDescriptor {
                    origin: StageOrigin::SnapshotContext {
                        session: self.reconciliation.ok_or_else(|| {
                            SyncError::Invalid("coordinator invariant: missing state".into())
                        })?,
                        snapshot_id: remote.snapshot_id,
                        collection,
                    },
                    manifest,
                };
                let progress = endpoint!(
                    ("begin_stage", &descriptor, self.limits.stages),
                    4096,
                    receiver.begin_stage(descriptor.clone(), self.limits.stages)
                );
                self.stage = Some(progress.stage);
                self.graph_stage = Some(progress.stage);
                self.ordinal = progress.next_ordinal;
                self.phase = Phase::FetchObject;
            }
            Phase::FetchObject => {
                let remote = self.remote_snapshot.as_ref().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let manifest = self
                    .contexts
                    .first()
                    .ok_or_else(|| {
                        SyncError::Invalid("coordinator invariant: missing state".into())
                    })?
                    .1
                    .clone();
                let fragment = endpoint!(
                    (
                        "read_fragment",
                        remote.snapshot_id,
                        &manifest,
                        self.ordinal,
                        self.limits.stages.max_fragment_bytes
                    ),
                    self.limits.stages.max_fragment_bytes.saturating_add(256),
                    remote_source.read_fragment(
                        remote.snapshot_id,
                        manifest.clone(),
                        self.ordinal,
                        self.limits.stages.max_fragment_bytes
                    )
                );
                self.phase = Phase::PutObject(fragment);
            }
            Phase::PutObject(fragment) => {
                let stage = self.stage.ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let progress = endpoint!(
                    ("stage_fragment", stage, &fragment),
                    4096,
                    receiver.stage_fragment(stage, fragment.clone())
                );
                if progress.complete {
                    self.contexts.remove(0);
                    self.stage = None;
                    self.graph_stage = None;
                    self.phase = if self.contexts.is_empty() {
                        self.after_contexts()
                    } else {
                        Phase::BeginObject
                    };
                } else {
                    self.ordinal = progress.next_ordinal;
                    self.phase = Phase::FetchObject;
                }
            }
            Phase::Leaf => {
                let remote = self.remote_snapshot.as_ref().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let range = self
                    .queue
                    .first()
                    .ok_or_else(|| {
                        SyncError::Invalid("coordinator invariant: missing state".into())
                    })?
                    .range
                    .clone();
                let cursor = self.leaf_cursor.clone();
                let page = endpoint!(
                    (
                        "leaf",
                        remote.snapshot_id,
                        &range,
                        &cursor,
                        self.limits.pages
                    ),
                    self.limits.pages.max_bytes.saturating_add(256),
                    remote_source.leaf(
                        remote.snapshot_id,
                        range.clone(),
                        cursor.clone(),
                        self.limits.pages
                    )
                );
                self.phase = Phase::StageLeaf(page);
            }
            Phase::StageLeaf(page) => {
                let session = self.reconciliation.ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                endpoint!(
                    ("stage_leaf", session, &page),
                    4096,
                    sink.stage_leaf_page(session, page.clone())
                );
                self.leaf_cursor = page.next;
                self.phase = if page.done {
                    Phase::Commit
                } else {
                    Phase::Leaf
                };
            }
            Phase::Commit => {
                let range = self
                    .queue
                    .first()
                    .ok_or_else(|| {
                        SyncError::Invalid("coordinator invariant: missing state".into())
                    })?
                    .range
                    .clone();
                let session = self.reconciliation.ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let result = endpoint_result!(
                    ("commit_range", session, &range, self.limits.apply),
                    self.limits
                        .apply
                        .max_projection_bytes
                        .saturating_add(
                            u64::from(self.limits.apply.max_projection_items).saturating_mul(512)
                        )
                        .saturating_add(4096),
                    sink.commit_complete_range(session, range.clone(), self.limits.apply)
                );
                match result {
                    Ok(_) => (),
                    Err(SyncError::WorkLimit) if self.depths[0] < self.limits.work.max_depth => {
                        self.must_split = true;
                        self.phase = Phase::DiscardRange;
                        return Ok(SessionAdvance::Incomplete);
                    }
                    Err(e) => return Err(e),
                }
                self.graph_stage = None;
                self.restarts = 0;
                self.queue.remove(0);
                self.depths.remove(0);
                self.phase = self.after_contexts();
            }
            Phase::DiscardRange => {
                let range = self
                    .queue
                    .first()
                    .ok_or_else(|| {
                        SyncError::Invalid("coordinator invariant: missing state".into())
                    })?
                    .range
                    .clone();
                let session = self.reconciliation.ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                endpoint!(
                    ("discard_range", session, &range),
                    4096,
                    sink.discard_range(session, range.clone())
                );
                self.graph_stage = None;
                self.phase = Phase::Split {
                    remote: true,
                    cursor: None,
                };
            }
            Phase::Promote => {
                let session = self.reconciliation.ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let result = endpoint!(
                    ("promote", session, self.limits.apply),
                    4096,
                    sink.promote_step(session, self.limits.apply)
                );
                self.restarts = 0;
                if result.done {
                    self.phase = Phase::CompleteSnapshot;
                }
            }
            Phase::CompleteSnapshot => {
                let session = self.reconciliation.ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                self.progress = Some(endpoint!(
                    ("complete", session),
                    4096,
                    sink.complete_reconciliation(session)
                ));
                self.phase = Phase::CloseRemote;
            }
            Phase::CloseRemote => {
                let snapshot = self
                    .remote_snapshot
                    .as_ref()
                    .ok_or_else(|| {
                        SyncError::Invalid("coordinator invariant: missing state".into())
                    })?
                    .snapshot_id;
                match endpoint_result!(
                    ("close", snapshot),
                    4096,
                    remote_source.close_snapshot(snapshot)
                ) {
                    Ok(()) | Err(SyncError::SnapshotExpired) => (),
                    Err(e) => return Err(e),
                };
                self.phase = Phase::CloseLocal;
            }
            Phase::CloseLocal => {
                let snapshot = self
                    .local_snapshot
                    .as_ref()
                    .ok_or_else(|| {
                        SyncError::Invalid("coordinator invariant: missing state".into())
                    })?
                    .snapshot_id;
                match endpoint_result!(
                    ("close", snapshot),
                    4096,
                    local_source.close_snapshot(snapshot)
                ) {
                    Ok(()) | Err(SyncError::SnapshotExpired) => (),
                    Err(e) => return Err(e),
                };
                self.phase = Phase::CaptureHead;
            }
            Phase::CaptureHead => {
                self.through = Some(endpoint!(("head",), 4096, peer.head()));
                self.phase = Phase::History;
            }
            Phase::Read => {
                let after = self.progress.clone().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let through = self.through.clone().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                if after == through {
                    self.phase = Phase::Finish;
                } else {
                    let mut limits = self.limits.pages;
                    limits.max_scanned = 1;
                    let request = PageRequest {
                        source: self.source.clone().ok_or_else(|| {
                            SyncError::Invalid("coordinator invariant: missing state".into())
                        })?,
                        scope: self.scope.clone(),
                        after,
                        through,
                        limits,
                    };
                    match endpoint!(
                        ("read_page", &request),
                        self.limits.pages.max_bytes.saturating_add(256),
                        peer.read_page(request.clone())
                    ) {
                        PageRead::Page(page) => {
                            if page.request != request {
                                return Err(SyncError::Invalid("page request mismatch".into()));
                            }
                            self.phase = Phase::PreparePage(page);
                        }
                        PageRead::FragmentNeeded {
                            request: returned,
                            sequence,
                            collection,
                            event_digest,
                            manifest,
                        } => {
                            if returned != request
                                || request.after.sequence.checked_add(1) != Some(sequence)
                                || !request.scope.collections.contains(&collection)
                                || !matches!(
                                    manifest.kind,
                                    ManifestKind::KeyEvent | ManifestKind::MergeEvent
                                )
                            {
                                return Err(SyncError::Invalid("fragment request mismatch".into()));
                            }
                            self.journal = Some(JournalTransfer {
                                request,
                                sequence,
                                collection,
                                digest: event_digest,
                                manifest,
                                event_stage: None,
                            });
                            self.phase = Phase::BeginJournalObject;
                        }
                    }
                }
            }
            Phase::PreparePage(page) => {
                validate_scanned_links(&page)?;
                if let Some(link) = page
                    .links
                    .iter()
                    .find(|link| matches!(link.event, Some(EventRef::Fragmented(_))))
                {
                    let Some(EventRef::Fragmented(manifest)) = &link.event else {
                        return Err(SyncError::Invalid(
                            "coordinator invariant: unexpected state".into(),
                        ));
                    };
                    if page.links.len() != 1
                        || !matches!(
                            manifest.kind,
                            ManifestKind::KeyEvent | ManifestKind::MergeEvent
                        )
                    {
                        return Err(SyncError::Invalid("invalid fragmented page".into()));
                    }
                    self.journal = Some(JournalTransfer {
                        request: page.request.clone(),
                        sequence: link.sequence,
                        collection: link.collection.clone(),
                        digest: link.event_digest,
                        manifest: manifest.clone(),
                        event_stage: None,
                    });
                    self.phase = Phase::BeginJournalObject;
                } else {
                    self.phase = Phase::Apply(page);
                }
            }
            Phase::BeginJournalObject => {
                let transfer = self.journal.as_ref().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let manifest = transfer.manifest.clone();
                let descriptor = StageDescriptor {
                    origin: StageOrigin::Journal {
                        request: transfer.request.clone(),
                        sequence: transfer.sequence,
                        collection: transfer.collection.clone(),
                        event_digest: transfer.digest,
                    },
                    manifest,
                };
                let progress = endpoint!(
                    ("begin_stage", &descriptor, self.limits.stages),
                    4096,
                    receiver.begin_stage(descriptor.clone(), self.limits.stages)
                );
                self.stage = Some(progress.stage);
                self.graph_stage = Some(progress.stage);
                self.ordinal = progress.next_ordinal;
                self.object_bytes.clear();
                self.phase = Phase::FetchJournalObject;
            }
            Phase::FetchJournalObject => {
                let manifest = self
                    .journal
                    .as_ref()
                    .ok_or_else(|| {
                        SyncError::Invalid("coordinator invariant: missing state".into())
                    })?
                    .manifest
                    .clone();
                let fragment = endpoint!(
                    (
                        "read_fragment",
                        &manifest,
                        self.ordinal,
                        self.limits.stages.max_fragment_bytes
                    ),
                    self.limits.stages.max_fragment_bytes.saturating_add(256),
                    peer.read_fragment(
                        manifest.clone(),
                        self.ordinal,
                        self.limits.stages.max_fragment_bytes
                    )
                );
                if fragment.ordinal != self.ordinal
                    || fragment.offset != self.object_bytes.len() as u64
                    || fragment.manifest != manifest
                    || fragment_digest(&fragment)? != fragment.digest
                {
                    return Err(SyncError::Invalid("invalid fragment response".into()));
                }
                self.phase = Phase::PutJournalObject(fragment);
            }
            Phase::PutJournalObject(fragment) => {
                let stage = self.stage.ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let progress = endpoint!(
                    ("stage_fragment", stage, &fragment),
                    4096,
                    receiver.stage_fragment(stage, fragment.clone())
                );
                self.object_bytes.extend_from_slice(&fragment.bytes);
                if !progress.complete {
                    self.ordinal = progress.next_ordinal;
                    self.phase = Phase::FetchJournalObject;
                } else {
                    let transfer = self.journal.as_mut().ok_or_else(|| {
                        SyncError::Invalid("coordinator invariant: missing state".into())
                    })?;
                    let manifest = &transfer.manifest;
                    if self.object_bytes.len() as u64 != manifest.encoded_bytes
                        || content_digest(&self.object_bytes)? != manifest.content_digest
                    {
                        return Err(SyncError::Invalid("fragment content mismatch".into()));
                    }
                    let event = decode_event_remote::<M>(&self.object_bytes)?;
                    if event_digest(&event)? != transfer.digest
                        || !matches!(
                            (manifest.kind, &event),
                            (ManifestKind::KeyEvent, Event::Key(_))
                                | (ManifestKind::MergeEvent, Event::Merge(_))
                        )
                    {
                        return Err(SyncError::Invalid("fragmented event mismatch".into()));
                    }
                    transfer.event_stage = self.stage;
                    self.stage = None;
                    self.phase = Phase::CommitJournalEvent;
                }
            }
            Phase::CommitJournalEvent => {
                let transfer = self.journal.as_ref().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let key = self.key.clone().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let stage = transfer.event_stage.ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let result = endpoint!(
                    (
                        "commit_event",
                        &key,
                        transfer.sequence,
                        stage,
                        self.limits.apply
                    ),
                    self.limits
                        .apply
                        .max_projection_bytes
                        .saturating_add(
                            u64::from(self.limits.apply.max_projection_items).saturating_mul(512)
                        )
                        .saturating_add(4096),
                    receiver.commit_fragmented_event(
                        key.clone(),
                        transfer.sequence,
                        stage,
                        self.limits.apply
                    )
                );
                self.restarts = 0;
                self.progress = Some(result.progress);
                self.journal = None;
                self.graph_stage = None;
                self.phase = Phase::Read;
            }
            Phase::Apply(page) => {
                let result = endpoint!(
                    ("apply_page", &page, self.limits.apply),
                    self.limits
                        .apply
                        .max_projection_bytes
                        .saturating_add(
                            u64::from(self.limits.apply.max_projection_items).saturating_mul(512)
                        )
                        .saturating_add(4096),
                    receiver.apply_page(&page, self.limits.apply)
                );
                self.restarts = 0;
                self.progress = Some(result.progress);
                self.journal = None;
                self.graph_stage = None;
                self.phase = Phase::Read;
            }
            Phase::Finish => {
                let key = self.key.clone().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                let through = self.through.clone().ok_or_else(|| {
                    SyncError::Invalid("coordinator invariant: missing state".into())
                })?;
                endpoint!(
                    ("finish", &key, &through),
                    4096,
                    receiver.finish_session(key.clone(), through.clone())
                );
                self.phase = Phase::Finished;
            }
            Phase::Finished => {
                return Ok(SessionAdvance::Complete(self.progress.clone().ok_or_else(
                    || SyncError::Invalid("coordinator invariant: missing state".into()),
                )?));
            }
            Phase::CleanupStage => {
                if let Some(journal) = &self.journal {
                    let origin = StageOrigin::Journal {
                        request: journal.request.clone(),
                        sequence: journal.sequence,
                        collection: journal.collection.clone(),
                        event_digest: journal.digest,
                    };
                    endpoint!(
                        ("abandon_origin", &origin),
                        4096,
                        receiver.abandon_origin(origin.clone())
                    );
                    self.graph_stage = None;
                    self.stage = None;
                } else if let Some(stage) = self.graph_stage {
                    match endpoint_result!(
                        ("abandon_stage", stage),
                        4096,
                        receiver.abandon_stage(stage)
                    ) {
                        Ok(()) | Err(SyncError::StaleWork) => (),
                        Err(e) => return Err(e),
                    }
                    self.graph_stage = None;
                    self.stage = None;
                }
                self.phase = Phase::CleanupSession;
            }
            Phase::CleanupSession => {
                if let Some(session) = self.reconciliation {
                    endpoint!(
                        ("cancel", session),
                        4096,
                        sink.cancel_reconciliation(session)
                    );
                    self.reconciliation = None;
                }
                self.phase = Phase::CleanupRemote;
            }
            Phase::CleanupRemote => {
                endpoint!(
                    ("release", self.remote_request_id),
                    4096,
                    remote_source.release_request(self.remote_request_id)
                );
                self.phase = Phase::CleanupLocal;
            }
            Phase::CleanupLocal => {
                endpoint!(
                    ("release", self.local_request_id),
                    4096,
                    local_source.release_request(self.local_request_id)
                );
                self.phase = Phase::Restart;
            }
            Phase::Restart => {
                if !matches!(self.recovery, Recovery::Abort) {
                    self.restarts = self.restarts.saturating_add(1);
                    if self.restarts >= 8 {
                        return Err(SyncError::Invalid(
                            "coordinator restart limit exceeded".into(),
                        ));
                    }
                }
                self.remote_snapshot = None;
                self.local_snapshot = None;
                self.queue.clear();
                self.depths.clear();
                self.child_ranges.clear();
                self.child_summaries.clear();
                self.contexts.clear();
                self.journal = None;
                self.object_bytes.clear();
                self.must_split = false;
                self.remote_request_id = Uuid::new_v4();
                self.local_request_id = Uuid::new_v4();
                self.reconciliation_request_id = Uuid::new_v4();
                self.phase = match self.recovery {
                    Recovery::Reconcile => Phase::OpenRemote(None),
                    Recovery::Refresh => Phase::Checkpoint,
                    Recovery::Reidentify => Phase::Source,
                    Recovery::Abort => Phase::Aborted,
                };
            }
            Phase::Aborted => return Ok(SessionAdvance::Cancelled),
        }
        Ok(SessionAdvance::Incomplete)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Test;
    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    enum Collection {
        One,
    }
    impl SyncModel for Test {
        type Collection = Collection;
        type Payload = String;
        fn model_id() -> &'static str {
            "budget"
        }
        fn schema_id() -> &'static str {
            "one"
        }
        fn collections() -> Vec<Collection> {
            vec![Collection::One]
        }
        fn collection_tag(_: &Collection) -> &'static str {
            "one"
        }
        fn collection_from_tag(s: &str) -> Option<Collection> {
            (s == "one").then_some(Collection::One)
        }
        fn validate(_: &ObjectKey<Self>, _: &Option<String>) -> Result<()> {
            Ok(())
        }
        fn validate_revision(_: &ObjectKey<Self>, _: &Option<String>, _: &Revision) -> Result<()> {
            Ok(())
        }
    }
    #[tokio::test]
    async fn operation_larger_than_empty_pass_is_explicit_error() {
        let identity = StoreIdentity {
            namespace: Uuid::from_u128(1),
            model: ModelId {
                model: "budget".into(),
                schema: "one".into(),
            },
            replica: Uuid::from_u128(2),
            generation: Uuid::from_u128(3),
        };
        let store = std::sync::Arc::new(
            MemorySyncStore::<Test>::new(
                identity,
                LocalAuthor {
                    author: Uuid::from_u128(4),
                    writer_incarnation: Uuid::from_u128(5),
                },
            )
            .unwrap(),
        );
        let peer = StorePeer(store.clone());
        let limits = SessionLimits {
            pages: PageLimits {
                max_scanned: 1,
                max_bytes: 4096,
            },
            stages: StageLimits {
                max_total_bytes: DEFAULT_STAGE_BYTES,
                max_stages: DEFAULT_STAGES,
                max_event_bytes: DEFAULT_LOGICAL_BYTES,
                max_event_items: DEFAULT_LOGICAL_ITEMS,
                max_fragment_bytes: DEFAULT_PAGE_BYTES,
            },
            apply: ApplyLimits {
                max_projection_items: 1,
                max_projection_bytes: 1024,
                max_mutations: 1,
            },
            work: WorkLimits {
                max_requests: 1,
                max_bytes: 10000,
                max_depth: 1,
                max_rows_scanned: 1,
                max_mutations: 1,
            },
        };
        let mut c = Coordinator::new(
            Scope {
                collections: [Collection::One].into_iter().collect(),
            },
            limits,
        )
        .unwrap();
        c.stage = Some(Uuid::from_u128(6));
        c.phase = Phase::PutObject(Fragment {
            manifest: ManifestRef {
                kind: ManifestKind::Context,
                manifest_digest: [0; 32],
                content_digest: [0; 32],
                encoded_bytes: 9000,
                item_count: 1,
                fragment_count: 1,
            },
            ordinal: 0,
            offset: 0,
            bytes: vec![0; 9000],
            digest: [0; 32],
        });
        assert!(
            matches!(c.advance(&*store,&*store,&*store,&peer,&*store).await,Err(SyncError::LimitTooSmall {required}) if required>10000)
        );
        assert_eq!(c.pass_usage().requests, 0);
    }
}
