//! Atomic in-memory implementation of the causal protocol.
use super::memory_core::MemoryCore;
use super::receiver::ReceiverLedger;
use super::*;
use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard};

#[derive(Clone)]
struct State<M: SyncModel> {
    #[cfg(test)]
    fail_next_apply: Option<SyncError>,
    core: MemoryCore<M>,
    checkpoints: Vec<(CheckpointKey, ReceiveProgress)>,
    accepted_anchors: Vec<(CheckpointKey, ReceiveProgress)>,
    receiver: ReceiverLedger<M>,
    snapshots: Vec<Snapshot<M>>,
    retired_requests: std::collections::BTreeMap<Uuid, u64>,
    summaries: Vec<SummaryWork<M>>,
    builds: Vec<SnapshotBuild<M>>,
    splits: Vec<SplitWork<M>>,
    completed_work: Vec<CompletedWork<M>>,
}

impl<M: SyncModel> Deref for State<M> {
    type Target = ReceiverLedger<M>;
    fn deref(&self) -> &Self::Target {
        &self.receiver
    }
}
impl<M: SyncModel> DerefMut for State<M> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.receiver
    }
}
fn remember_retired<M: SyncModel>(state: &mut State<M>, request: Uuid, now: u64) {
    if !state.retired_requests.contains_key(&request)
        && state.retired_requests.len() >= MAX_RECEIVER_RETRY_RECEIPTS
        && let Some(id) = state
            .retired_requests
            .iter()
            .min_by_key(|(_, deadline)| *deadline)
            .map(|(id, _)| *id)
    {
        state.retired_requests.remove(&id);
    }
    state
        .retired_requests
        .insert(request, now.saturating_add(DEFAULT_SNAPSHOT_TTL_MS));
}

#[derive(Clone)]
struct Snapshot<M: SyncModel> {
    request_id: Uuid,
    request_limits: WorkLimits,
    descriptor: SnapshotDescriptor<M>,
    core: MemoryCore<M>,
    objects: Vec<(ManifestRef, Vec<u8>)>,
    ready: bool,
    summary_cache: Vec<RangeSummary<M>>,
}
#[derive(Clone)]
struct SummaryWork<M: SyncModel> {
    id: Uuid,
    snapshot: Uuid,
    range: KeyRange<M>,
    position: u64,
    phase: SummaryPhase,
    candidate: Option<CandidateCursor<M>>,
    candidate_count: u64,
    candidate_bytes: u64,
    hash: Option<RangeHasher<M>>,
    last_input: Option<u64>,
}
#[derive(Clone, Copy)]
enum SummaryPhase {
    CountCandidates,
    HashCandidates,
}
#[derive(Clone)]
struct SnapshotBuild<M: SyncModel> {
    id: Uuid,
    snapshot: Uuid,
    collections: Vec<M::Collection>,
    index: usize,
    summary: Option<Uuid>,
    position: u64,
}
#[derive(Clone)]
struct SplitWork<M: SyncModel> {
    id: Uuid,
    snapshot: Uuid,
    range: KeyRange<M>,
    position: u64,
    phase: SplitPhase,
    candidate: Option<CandidateCursor<M>>,
    last_key: Option<Uuid>,
    distinct: u64,
    selected: u64,
    total_bytes: u64,
    selected_bytes: u64,
    last_input: Option<u64>,
}
#[derive(Clone)]
enum CompletedWork<M: SyncModel> {
    Open {
        id: Uuid,
        input: u64,
        scope: Scope<M>,
        snapshot: SnapshotDescriptor<M>,
    },
    Summary {
        id: Uuid,
        input: u64,
        snapshot: Uuid,
        range: KeyRange<M>,
        result: RangeSummary<M>,
    },
    Split {
        id: Uuid,
        input: u64,
        snapshot: Uuid,
        range: KeyRange<M>,
        result: Vec<KeyRange<M>>,
    },
}
impl<M: SyncModel> CompletedWork<M> {
    fn snapshot_id(&self) -> Uuid {
        match self {
            Self::Open { snapshot, .. } => snapshot.snapshot_id,
            Self::Summary { snapshot, .. } | Self::Split { snapshot, .. } => *snapshot,
        }
    }
}
#[derive(Clone, Copy)]
enum SplitPhase {
    CountCandidates,
    FindCandidate,
}

pub struct MemorySyncStore<M: SyncModel> {
    state: Mutex<State<M>>,
}
impl<M: SyncModel> MemorySyncStore<M> {
    pub fn new(identity: StoreIdentity, author: LocalAuthor) -> Result<Self> {
        Ok(Self {
            state: Mutex::new(State {
                #[cfg(test)]
                fail_next_apply: None,
                core: MemoryCore::new(identity, author)?,
                checkpoints: Vec::new(),
                accepted_anchors: Vec::new(),
                receiver: ReceiverLedger::default(),
                snapshots: Vec::new(),
                retired_requests: Default::default(),
                summaries: Vec::new(),
                builds: Vec::new(),
                splits: Vec::new(),
                completed_work: Vec::new(),
            }),
        })
    }
    fn lock(&self) -> Result<MutexGuard<'_, State<M>>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| SyncError::Storage("memory store lock poisoned".into()))?;
        let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let expired_requests: Vec<_> = state
            .snapshots
            .iter()
            .filter(|view| view.descriptor.expires_unix_ms <= now)
            .map(|view| view.request_id)
            .collect();
        state.retired_requests.retain(|_, deadline| *deadline > now);
        while state.retired_requests.len() >= MAX_RECEIVER_RETRY_RECEIPTS {
            if let Some(id) = state
                .retired_requests
                .iter()
                .min_by_key(|(_, deadline)| *deadline)
                .map(|(id, _)| *id)
            {
                state.retired_requests.remove(&id);
            } else {
                break;
            }
        }
        for request in expired_requests {
            remember_retired(&mut state, request, now);
        }
        state
            .snapshots
            .retain(|view| view.descriptor.expires_unix_ms > now);
        let snapshot_ids: std::collections::BTreeSet<_> = state
            .snapshots
            .iter()
            .map(|view| view.descriptor.snapshot_id)
            .collect();
        state
            .summaries
            .retain(|work| snapshot_ids.contains(&work.snapshot));
        state
            .builds
            .retain(|work| snapshot_ids.contains(&work.snapshot));
        state
            .splits
            .retain(|work| snapshot_ids.contains(&work.snapshot));
        state
            .completed_work
            .retain(|work| snapshot_ids.contains(&work.snapshot_id()));
        state.sessions.retain(|session| {
            session.local.expires_unix_ms > now && session.remote.expires_unix_ms > now
        });
        state
            .completed_sessions
            .retain(|(_, _, deadline)| *deadline > now);
        state
            .completed_events
            .retain(|(_, _, _, _, deadline)| *deadline > now);
        let expired: Vec<_> = state
            .descriptors
            .iter()
            .filter_map(|(id, descriptor)| match &descriptor.origin {
                StageOrigin::SnapshotContext { session, .. }
                    if !state.sessions.iter().any(|active| active.id == *session) =>
                {
                    Some(*id)
                }
                _ => None,
            })
            .collect();
        let timed_out: Vec<_> = state
            .stage_deadlines
            .iter()
            .filter_map(|(id, deadline)| (*deadline <= now).then_some(*id))
            .collect();
        for id in expired.into_iter().chain(timed_out) {
            if let Ok(ids) = state.stages.abandon_group(id) {
                state.descriptors.retain(|(stage, _)| !ids.contains(stage));
                state
                    .stage_deadlines
                    .retain(|(stage, _)| !ids.contains(stage));
            }
        }
        Ok(state)
    }
}
fn checkpoint_key<M: SyncModel>(request: &PageRequest<M>) -> Result<CheckpointKey> {
    Ok(CheckpointKey {
        namespace: request.source.namespace,
        source_replica: request.source.replica,
        source_generation: request.source.generation,
        scope_id: scope_id(&request.scope)?,
    })
}
fn lookup(s: &[(CheckpointKey, ReceiveProgress)], k: &CheckpointKey) -> Option<ReceiveProgress> {
    s.iter().find(|(key, _)| key == k).map(|(_, p)| p.clone())
}
fn set_checkpoint(
    s: &mut Vec<(CheckpointKey, ReceiveProgress)>,
    k: CheckpointKey,
    p: ReceiveProgress,
) {
    if let Some((_, v)) = s.iter_mut().find(|(key, _)| *key == k) {
        *v = p
    } else {
        s.push((k, p))
    }
}
const FRAGMENT_BYTES: usize = 64 * 1024;
const MAX_SOURCE_WORK_ITEMS: usize = 256;
fn memory_fragment_bounds(
    bytes: &[u8],
    manifest: &ManifestRef,
    ordinal: u32,
) -> Result<(usize, usize)> {
    if manifest.encoded_bytes != bytes.len() as u64 {
        return Err(SyncError::Invalid("fragment content size mismatch".into()));
    }
    let (start, end) = super::fragment_bounds(manifest, ordinal)?;
    Ok((
        usize::try_from(start).map_err(|_| SyncError::LogicalLimit)?,
        usize::try_from(end).map_err(|_| SyncError::LogicalLimit)?,
    ))
}
fn check_fragment_size(fragment: &Fragment, limit: u64) -> Result<()> {
    let size = serde_json::to_vec(fragment)
        .map_err(|e| SyncError::Storage(e.to_string()))?
        .len() as u64;
    if size > limit {
        return Err(SyncError::LimitTooSmall { required: size });
    }
    Ok(())
}
fn event_manifest<M: SyncModel>(
    event: &Event<M>,
    max_bytes: u64,
) -> Result<(ManifestRef, Vec<u8>)> {
    let bytes = canonical_event(event)?;
    let chunk = usize::try_from(max_bytes.saturating_sub(1024) / 2)
        .unwrap_or(usize::MAX)
        .min(FRAGMENT_BYTES);
    if chunk == 0 {
        return Err(SyncError::LimitTooSmall { required: 1026 });
    }
    let fragments = bytes.len().div_ceil(chunk);
    if fragments > MAX_FRAGMENTS as usize {
        return Err(SyncError::LogicalLimit);
    }
    let mut manifest = ManifestRef {
        kind: match event {
            Event::Key(_) => ManifestKind::KeyEvent,
            Event::Merge(_) => ManifestKind::MergeEvent,
        },
        manifest_digest: [0; 32],
        content_digest: content_digest(&bytes)?,
        encoded_bytes: bytes.len() as u64,
        item_count: 1,
        fragment_count: u32::try_from(fragments).map_err(|_| SyncError::LogicalLimit)?,
    };
    manifest.manifest_digest = manifest_digest(&manifest)?;
    Ok((manifest, bytes))
}
fn check_limits(l: ApplyLimits, changes: &[ProjectionChange<impl SyncModel>]) -> Result<()> {
    if l.max_projection_items == 0
        || l.max_projection_items > MAX_PROJECTION_ITEMS
        || l.max_projection_bytes == 0
        || l.max_projection_bytes > MAX_PROJECTION_BYTES
        || l.max_mutations == 0
        || l.max_mutations > MAX_MUTATIONS
    {
        return Err(SyncError::Invalid("invalid apply limits".into()));
    }
    if changes.len() > l.max_projection_items as usize {
        return Err(SyncError::WorkLimit);
    }
    let mut bytes = 0u64;
    for c in changes {
        for v in [&c.before, &c.after].into_iter().flatten() {
            bytes = bytes
                .checked_add(canonical_candidate(v)?.len() as u64)
                .ok_or(SyncError::WorkLimit)?;
        }
    }
    if bytes > l.max_projection_bytes {
        return Err(SyncError::WorkLimit);
    }
    Ok(())
}
fn apply<M: SyncModel>(
    s: &mut State<M>,
    page: &ChangePage<M>,
    limits: ApplyLimits,
    check_page_bytes: bool,
) -> Result<ApplyResult<M>> {
    validate_scanned_links(page)?;
    let encoded = serde_json::to_vec(page).map_err(|e| SyncError::Storage(e.to_string()))?;
    if check_page_bytes && encoded.len() as u64 > page.request.limits.max_bytes {
        return Err(SyncError::LimitTooSmall {
            required: encoded.len() as u64,
        });
    }
    if page.request.source.namespace != s.core.identity.namespace
        || page.request.source.model != s.core.identity.model
    {
        return Err(SyncError::IncompatibleModel);
    }
    let key = checkpoint_key(&page.request)?;
    match lookup(&s.checkpoints, &key) {
        Some(p) if p == page.request.after => (),
        Some(p) if p == page.next => {
            return Ok(ApplyResult {
                progress: p,
                projection: ProjectionBatch {
                    changes: Vec::new(),
                    causal_changes: 0,
                    pending: 0,
                },
            });
        }
        Some(p)
            if p.sequence > page.next.sequence
                && s.accepted_anchors
                    .iter()
                    .any(|(old_key, old)| old_key == &key && old == &page.next) =>
        {
            return Ok(ApplyResult {
                progress: p,
                projection: ProjectionBatch {
                    changes: Vec::new(),
                    causal_changes: 0,
                    pending: 0,
                },
            });
        }
        Some(p) => return Err(SyncError::RetryAt(p)),
        None => {
            return Err(SyncError::NeedsReconciliation(
                HistoryStatus::Uninitialized {
                    head: page.request.through.clone(),
                },
            ));
        }
    }
    if page.links.len() as u64 > limits.max_mutations {
        return Err(SyncError::WorkLimit);
    }
    let mut changes = Vec::new();
    let mut causal_changes = 0;
    for link in &page.links {
        match &link.event {
            Some(EventRef::Inline(Event::Key(delta))) => {
                let (changed, projection) = s.core.apply_delta(delta)?;
                causal_changes += u64::from(changed);
                if let Some(c) = projection {
                    changes.push(c)
                }
            }
            Some(EventRef::Inline(Event::Merge(delta))) => {
                let (changed, projection) = s.core.apply_merge(delta)?;
                causal_changes += u64::from(changed);
                changes.extend(projection);
            }
            Some(EventRef::Fragmented(_)) => {
                return Err(SyncError::MissingDependency(link.event_digest));
            }
            None => (),
        }
    }
    let mut preceding = page.request.after.clone();
    for link in &page.links {
        let mut link_request = page.request.clone();
        link_request.after = preceding.clone();
        let stage = s
            .descriptors
            .iter()
            .find_map(|(id, descriptor)| match &descriptor.origin {
                StageOrigin::Journal {
                    request,
                    sequence,
                    event_digest,
                    ..
                } if request == &link_request
                    && *sequence == link.sequence
                    && *event_digest == link.event_digest =>
                {
                    Some(*id)
                }
                _ => None,
            });
        if let Some(stage) = stage {
            let ids = s.stages.abandon_group(stage)?;
            s.descriptors.retain(|(id, _)| !ids.contains(id));
            s.stage_deadlines.retain(|(id, _)| !ids.contains(id));
        }
        preceding = ReceiveProgress {
            sequence: link.sequence,
            anchor: next_anchor::<M>(
                preceding.anchor,
                link.sequence,
                &link.collection,
                link.event_digest,
            )?,
        };
    }
    check_limits(limits, &changes)?;
    let mut anchor = page.request.after.anchor;
    s.accepted_anchors
        .push((key.clone(), page.request.after.clone()));
    for link in &page.links {
        anchor = next_anchor::<M>(anchor, link.sequence, &link.collection, link.event_digest)?;
        s.accepted_anchors.push((
            key.clone(),
            ReceiveProgress {
                sequence: link.sequence,
                anchor,
            },
        ));
    }
    set_checkpoint(&mut s.checkpoints, key, page.next.clone());
    Ok(ApplyResult {
        progress: page.next.clone(),
        projection: ProjectionBatch {
            causal_changes,
            pending: 0,
            changes,
        },
    })
}
impl<M: SyncModel> ReceiveStore<M> for MemorySyncStore<M> {
    fn identity(&self) -> BoxFuture<'_, StoreIdentity> {
        Box::pin(async move { Ok(self.lock()?.core.identity.clone()) })
    }
    fn checkpoint<'a>(&'a self, key: &'a CheckpointKey) -> BoxFuture<'a, Option<ReceiveProgress>> {
        Box::pin(async move { Ok(lookup(&self.lock()?.checkpoints, key)) })
    }
    fn apply_page<'a>(
        &'a self,
        page: &'a ChangePage<M>,
        limits: ApplyLimits,
    ) -> BoxFuture<'a, ApplyResult<M>> {
        Box::pin(async move {
            let mut guard = self.lock()?;
            #[cfg(test)]
            if let Some(error) = guard.fail_next_apply.take() {
                return Err(error);
            }
            let mut next = guard.clone();
            let out = apply(&mut next, page, limits, true)?;
            *guard = next;
            Ok(out)
        })
    }
    fn begin_stage(
        &self,
        descriptor: StageDescriptor<M>,
        limits: StageLimits,
    ) -> BoxFuture<'_, FragmentProgress> {
        Box::pin(async move {
            let mut state = self.lock()?;
            if let StageOrigin::Journal { request, .. } = &descriptor.origin {
                if request.source.namespace != state.core.identity.namespace
                    || request.source.model != state.core.identity.model
                {
                    return Err(SyncError::IncompatibleModel);
                }
                let checkpoint = lookup(&state.checkpoints, &checkpoint_key(request)?).ok_or(
                    SyncError::NeedsReconciliation(HistoryStatus::Uninitialized {
                        head: request.through.clone(),
                    }),
                )?;
                if request.after.sequence < checkpoint.sequence
                    || (request.after.sequence == checkpoint.sequence
                        && request.after.anchor != checkpoint.anchor)
                {
                    return Err(SyncError::RetryAt(checkpoint));
                }
            }
            let deadline = (chrono::Utc::now().timestamp_millis().max(0) as u64)
                .checked_add(DEFAULT_SNAPSHOT_TTL_MS)
                .ok_or(SyncError::ClockExhausted)?;
            state
                .receiver
                .begin_stage(Uuid::new_v4(), descriptor, limits, deadline)
        })
    }
    fn stage_fragment(&self, stage: Uuid, fragment: Fragment) -> BoxFuture<'_, FragmentProgress> {
        Box::pin(async move { self.lock()?.receiver.stage_fragment(stage, fragment) })
    }
    fn stage_status(&self, stage: Uuid) -> BoxFuture<'_, FragmentProgress> {
        Box::pin(async move { self.lock()?.receiver.stage_status(stage) })
    }
    fn abandon_stage(&self, stage: Uuid) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.lock()?.receiver.abandon_stage(stage) })
    }
    fn abandon_origin(&self, origin: StageOrigin<M>) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.lock()?.receiver.abandon_origin(&origin) })
    }
    fn commit_fragmented_event(
        &self,
        key: CheckpointKey,
        sequence: u64,
        stage: Uuid,
        limits: ApplyLimits,
    ) -> BoxFuture<'_, ApplyResult<M>> {
        Box::pin(async move {
            let mut guard = self.lock()?;
            if let Some((_, saved_key, saved_sequence, progress, _)) = guard
                .completed_events
                .iter()
                .find(|(id, _, _, _, _)| *id == stage)
            {
                if saved_key != &key || *saved_sequence != sequence {
                    return Err(SyncError::Invalid("completed stage origin mismatch".into()));
                }
                return Ok(ApplyResult {
                    progress: progress.clone(),
                    projection: ProjectionBatch {
                        changes: Vec::new(),
                        causal_changes: 0,
                        pending: 0,
                    },
                });
            }
            let mut s = guard.clone();
            let prepared = s.receiver.prepare_event(stage)?;
            let (request, expected_sequence, collection, digest) = match prepared.origin.clone() {
                StageOrigin::Journal {
                    request,
                    sequence,
                    collection,
                    event_digest,
                } => (request, sequence, collection, event_digest),
                _ => return Err(SyncError::Invalid("not a journal stage".into())),
            };
            if checkpoint_key(&request)? != key
                || sequence != expected_sequence
                || sequence
                    != request
                        .after
                        .sequence
                        .checked_add(1)
                        .ok_or(SyncError::ClockExhausted)?
            {
                return Err(SyncError::Invalid("stage origin mismatch".into()));
            }
            let event = prepared.event.clone();
            if event_digest(&event)? != digest {
                return Err(SyncError::Invalid("event digest mismatch".into()));
            }
            let next = ReceiveProgress {
                sequence,
                anchor: next_anchor::<M>(request.after.anchor, sequence, &collection, digest)?,
            };
            let page = ChangePage {
                done: next.sequence == request.through.sequence,
                next,
                request,
                links: vec![ScanLink {
                    sequence,
                    collection,
                    event_digest: digest,
                    event: Some(EventRef::Inline(event)),
                }],
            };
            let out = apply(&mut s, &page, limits, false)?;
            if s.descriptors.iter().any(|(id, _)| *id == stage) {
                let ids = s.stages.abandon_group(stage)?;
                s.descriptors.retain(|(id, _)| !ids.contains(id));
                s.stage_deadlines.retain(|(id, _)| !ids.contains(id));
            }
            let deadline = (chrono::Utc::now().timestamp_millis().max(0) as u64)
                .checked_add(DEFAULT_SNAPSHOT_TTL_MS)
                .ok_or(SyncError::ClockExhausted)?;
            s.receiver
                .finalize_event(&prepared, key, out.progress.clone(), deadline)?;
            *guard = s;
            Ok(out)
        })
    }
    fn finish_session(&self, key: CheckpointKey, through: ReceiveProgress) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if lookup(&self.lock()?.checkpoints, &key) == Some(through) {
                Ok(())
            } else {
                Err(SyncError::IncompleteCoverage)
            }
        })
    }
}
impl<M: SyncModel> SyncStore<M> for MemorySyncStore<M> {
    fn local_author(&self) -> BoxFuture<'_, LocalAuthor> {
        Box::pin(async move { Ok(self.lock()?.core.author) })
    }
    fn head(&self) -> BoxFuture<'_, ReceiveProgress> {
        Box::pin(async move { self.lock()?.core.head() })
    }
    fn history_status<'a>(
        &'a self,
        key: &'a CheckpointKey,
        progress: Option<ReceiveProgress>,
    ) -> BoxFuture<'a, HistoryStatus> {
        Box::pin(async move {
            let s = self.lock()?;
            let head = s.core.head()?;
            let floor = ReceiveProgress {
                sequence: 0,
                anchor: initial_anchor(&s.core.identity)?,
            };
            if key.namespace != s.core.identity.namespace
                || key.source_replica != s.core.identity.replica
            {
                return Err(SyncError::Invalid("history source mismatch".into()));
            }
            if key.source_generation != s.core.identity.generation {
                return Ok(HistoryStatus::GenerationChanged {
                    current: s.core.identity.generation,
                });
            }
            let Some(p) = progress else {
                return Ok(HistoryStatus::Uninitialized { head });
            };
            if p.sequence > head.sequence {
                return Ok(HistoryStatus::HeadBehind {
                    head: head.sequence,
                });
            }
            let expected = if p.sequence == 0 {
                floor.anchor
            } else {
                s.core.journal()[(p.sequence - 1) as usize].anchor
            };
            if expected != p.anchor {
                return Ok(HistoryStatus::AnchorMismatch { checkpoint: p });
            }
            Ok(HistoryStatus::Continuous { head, floor })
        })
    }
    fn get<'a>(&'a self, key: &'a ObjectKey<M>) -> BoxFuture<'a, Option<Record<M>>> {
        Box::pin(async move {
            Ok(self.lock()?.core.winner(key).map(|c| Record {
                key: c.key,
                value: Some(c.payload),
                revision: c.revision,
            }))
        })
    }
    fn candidates<'a>(
        &'a self,
        key: &'a ObjectKey<M>,
        after: Option<Dot>,
        limits: PageLimits,
    ) -> BoxFuture<'a, CandidatePage<M>> {
        Box::pin(async move {
            if limits.max_scanned == 0
                || limits.max_scanned > MAX_PAGE_ITEMS
                || limits.max_bytes == 0
                || limits.max_bytes > MAX_PAGE_BYTES
            {
                return Err(SyncError::Invalid("invalid page limits".into()));
            }
            let s = self.lock()?;
            let mut all = s.core.candidates(key);
            all.sort_by_key(|c| c.dot);
            let mut values = Vec::new();
            let mut used = 0u64;
            let mut remaining = false;
            for c in all.into_iter().filter(|c| after.is_none_or(|d| c.dot > d)) {
                let bytes = canonical_candidate(&c)?.len() as u64;
                if values.len() >= limits.max_scanned as usize || used + bytes > limits.max_bytes {
                    if values.is_empty() {
                        return Err(SyncError::LimitTooSmall { required: bytes });
                    }
                    remaining = true;
                    break;
                }
                used += bytes;
                values.push(c);
            }
            let next = values.last().map(|c| c.dot);
            Ok(CandidatePage {
                values,
                next,
                done: !remaining,
            })
        })
    }
    fn write_local(
        &self,
        key: ObjectKey<M>,
        value: Option<M::Payload>,
    ) -> BoxFuture<'_, Record<M>> {
        self.write_observed(key, value, Observed::AllCurrent)
    }
    fn write_observed(
        &self,
        key: ObjectKey<M>,
        value: Option<M::Payload>,
        observed: Observed,
    ) -> BoxFuture<'_, Record<M>> {
        Box::pin(async move { self.lock()?.core.write(key, value, observed) })
    }
    fn read_page(&self, request: PageRequest<M>) -> BoxFuture<'_, PageRead<M>> {
        Box::pin(async move {
            scope_id(&request.scope)?;
            if request.limits.max_scanned == 0
                || request.limits.max_scanned > MAX_PAGE_ITEMS
                || request.limits.max_bytes == 0
                || request.limits.max_bytes > MAX_PAGE_BYTES
            {
                return Err(SyncError::Invalid("invalid page limits".into()));
            }
            let s = self.lock()?;
            if request.source != s.core.identity {
                return Err(SyncError::Invalid("source identity mismatch".into()));
            }
            let head = s.core.head()?;
            if request.after.sequence > request.through.sequence
                || request.through.sequence > head.sequence
            {
                return Err(SyncError::Invalid("invalid page positions".into()));
            }
            let anchor = |seq: u64| -> Digest {
                if seq == 0 {
                    initial_anchor(&s.core.identity).expect("validated identity")
                } else {
                    s.core.journal()[(seq - 1) as usize].anchor
                }
            };
            if anchor(request.after.sequence) != request.after.anchor
                || anchor(request.through.sequence) != request.through.anchor
            {
                return Err(SyncError::Invalid("anchor mismatch".into()));
            }
            let mut next = request.after.clone();
            let mut links = Vec::new();
            let mut link_bytes = 0u64;
            for e in s
                .core
                .journal()
                .iter()
                .skip(request.after.sequence as usize)
                .take((request.through.sequence - request.after.sequence) as usize)
            {
                if links.len() >= request.limits.max_scanned as usize {
                    break;
                }
                let collection = match &e.event {
                    Event::Key(k) => k.key.collection.clone(),
                    Event::Merge(m) => m.collection.clone(),
                };
                let event = if request.scope.collections.contains(&collection) {
                    Some(EventRef::Inline(e.event.clone()))
                } else {
                    None
                };
                let link = ScanLink {
                    sequence: e.sequence,
                    collection: collection.clone(),
                    event_digest: e.digest,
                    event,
                };
                let proposed = ReceiveProgress {
                    sequence: e.sequence,
                    anchor: e.anchor,
                };
                let page = ChangePage {
                    request: request.clone(),
                    next: proposed.clone(),
                    done: proposed.sequence == request.through.sequence,
                    links: Vec::new(),
                };
                let base_size = serde_json::to_vec(&page)
                    .map_err(|e| SyncError::Storage(e.to_string()))?
                    .len() as u64;
                let candidate_bytes = serde_json::to_vec(&link)
                    .map_err(|e| SyncError::Storage(e.to_string()))?
                    .len() as u64;
                let size = base_size
                    .checked_add(link_bytes)
                    .and_then(|n| n.checked_add(candidate_bytes))
                    .and_then(|n| n.checked_add(links.len() as u64))
                    .ok_or(SyncError::LogicalLimit)?;
                if size > request.limits.max_bytes {
                    if links.is_empty() {
                        if request.scope.collections.contains(&collection) {
                            let (manifest, _) = event_manifest(&e.event, request.limits.max_bytes)?;
                            let result = PageRead::FragmentNeeded {
                                request,
                                sequence: e.sequence,
                                collection,
                                event_digest: e.digest,
                                manifest,
                            };
                            let size = serde_json::to_vec(&result)
                                .map_err(|e| SyncError::Storage(e.to_string()))?
                                .len() as u64;
                            if size
                                > match &result {
                                    PageRead::FragmentNeeded { request, .. } => {
                                        request.limits.max_bytes
                                    }
                                    PageRead::Page(_) => {
                                        return Err(SyncError::Invalid(
                                            "unexpected fragment page".into(),
                                        ));
                                    }
                                }
                            {
                                return Err(SyncError::LimitTooSmall { required: size });
                            }
                            return Ok(result);
                        }
                        return Err(SyncError::LimitTooSmall { required: size });
                    }
                    break;
                }
                link_bytes += candidate_bytes;
                links.push(link);
                next = proposed;
            }
            let page = ChangePage {
                done: next.sequence == request.through.sequence,
                request,
                next,
                links,
            };
            let size = serde_json::to_vec(&page)
                .map_err(|e| SyncError::Storage(e.to_string()))?
                .len() as u64;
            if size > page.request.limits.max_bytes {
                return Err(SyncError::LimitTooSmall { required: size });
            }
            Ok(PageRead::Page(page))
        })
    }
    fn read_fragment(
        &self,
        manifest: ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> BoxFuture<'_, Fragment> {
        Box::pin(async move {
            if max_bytes == 0 || max_bytes > MAX_PAGE_BYTES {
                return Err(SyncError::Invalid("invalid fragment limit".into()));
            }
            let s = self.lock()?;
            for e in s.core.journal() {
                let bytes = canonical_event(&e.event)?;
                let kind = match &e.event {
                    Event::Key(_) => ManifestKind::KeyEvent,
                    Event::Merge(_) => ManifestKind::MergeEvent,
                };
                if manifest.kind != kind
                    || manifest.encoded_bytes != bytes.len() as u64
                    || manifest.item_count != 1
                    || manifest.content_digest != content_digest(&bytes)?
                    || manifest_digest(&manifest)? != manifest.manifest_digest
                {
                    continue;
                }
                if ordinal >= manifest.fragment_count {
                    return Err(SyncError::Invalid("fragment ordinal".into()));
                }
                let (start, end) = memory_fragment_bounds(&bytes, &manifest, ordinal)?;
                if (end - start) as u64 > max_bytes {
                    return Err(SyncError::LimitTooSmall {
                        required: (end - start) as u64,
                    });
                }
                let mut fragment = Fragment {
                    manifest,
                    ordinal,
                    offset: start as u64,
                    bytes: bytes[start..end].to_vec(),
                    digest: [0; 32],
                };
                fragment.digest = fragment_digest(&fragment)?;
                check_fragment_size(&fragment, max_bytes)?;
                return Ok(fragment);
            }
            Err(SyncError::StaleWork)
        })
    }
}

fn manifest_for(kind: ManifestKind, bytes: &[u8], items: u64) -> Result<ManifestRef> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_LOGICAL_BYTES || items > MAX_LOGICAL_ITEMS {
        return Err(SyncError::LogicalLimit);
    }
    let fragments = bytes.len().div_ceil(FRAGMENT_BYTES);
    let mut result = ManifestRef {
        kind,
        manifest_digest: [0; 32],
        content_digest: content_digest(bytes)?,
        encoded_bytes: bytes.len() as u64,
        item_count: items,
        fragment_count: u32::try_from(fragments).map_err(|_| SyncError::LogicalLimit)?,
    };
    if result.fragment_count > MAX_FRAGMENTS {
        return Err(SyncError::LogicalLimit);
    }
    result.manifest_digest = manifest_digest(&result)?;
    Ok(result)
}
fn validate_work(l: WorkLimits) -> Result<()> {
    if l.max_requests == 0
        || l.max_requests > MAX_ROUND_REQUESTS
        || l.max_bytes == 0
        || l.max_bytes > MAX_ROUND_BYTES
        || l.max_depth == 0
        || l.max_depth > MAX_DEPTH
        || l.max_rows_scanned == 0
        || l.max_rows_scanned > MAX_ROWS
        || l.max_mutations == 0
        || l.max_mutations > MAX_MUTATIONS
    {
        return Err(SyncError::Invalid("invalid work limits".into()));
    }
    Ok(())
}
fn view_mut<M: SyncModel>(state: &mut State<M>, id: Uuid) -> Result<&mut Snapshot<M>> {
    let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let view = state
        .snapshots
        .iter_mut()
        .find(|v| v.descriptor.snapshot_id == id)
        .ok_or(SyncError::SnapshotExpired)?;
    if now >= view.descriptor.expires_unix_ms {
        return Err(SyncError::SnapshotExpired);
    }
    if !view.ready {
        return Err(SyncError::StaleWork);
    }
    Ok(view)
}
impl<M: SyncModel> SnapshotSource<M> for MemorySyncStore<M> {
    fn release_request(&self, request_id: Uuid) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let mut state = self.lock()?;
            if let Some(snapshot) = state
                .snapshots
                .iter()
                .find(|view| view.request_id == request_id)
                .map(|view| view.descriptor.snapshot_id)
            {
                state
                    .snapshots
                    .retain(|view| view.descriptor.snapshot_id != snapshot);
                state.builds.retain(|work| work.snapshot != snapshot);
                state.summaries.retain(|work| work.snapshot != snapshot);
                state.splits.retain(|work| work.snapshot != snapshot);
                remember_retired(
                    &mut state,
                    request_id,
                    chrono::Utc::now().timestamp_millis().max(0) as u64,
                );
            }
            Ok(())
        })
    }
    fn close_snapshot(&self, snapshot: Uuid) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let mut s = self.lock()?;
            if let Some(request_id) = s
                .snapshots
                .iter()
                .find(|view| view.descriptor.snapshot_id == snapshot)
                .map(|view| view.request_id)
            {
                remember_retired(
                    &mut s,
                    request_id,
                    chrono::Utc::now().timestamp_millis().max(0) as u64,
                );
            }
            let before = s.snapshots.len();
            s.snapshots.retain(|v| v.descriptor.snapshot_id != snapshot);
            if before == s.snapshots.len() {
                return Err(SyncError::SnapshotExpired);
            }
            s.summaries.retain(|work| work.snapshot != snapshot);
            s.builds.retain(|work| work.snapshot != snapshot);
            s.splits.retain(|work| work.snapshot != snapshot);
            Ok(())
        })
    }
    fn cancel_snapshot_build(&self, cursor: WorkCursor) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let mut s = self.lock()?;
            if cursor.generation != s.core.identity.generation {
                return Err(SyncError::StaleWork);
            }
            let index = s
                .builds
                .iter()
                .position(|work| work.id == cursor.operation_id && work.position == cursor.position)
                .ok_or(SyncError::StaleWork)?;
            let build = s.builds.remove(index);
            if let Some(request_id) = s
                .snapshots
                .iter()
                .find(|view| view.descriptor.snapshot_id == build.snapshot)
                .map(|view| view.request_id)
            {
                remember_retired(
                    &mut s,
                    request_id,
                    chrono::Utc::now().timestamp_millis().max(0) as u64,
                );
            }
            s.summaries.retain(|work| work.snapshot != build.snapshot);
            s.snapshots
                .retain(|view| view.descriptor.snapshot_id != build.snapshot);
            Ok(())
        })
    }
    fn open_snapshot(
        &self,
        request_id: Uuid,
        scope: Scope<M>,
        limits: WorkLimits,
        resume: Option<WorkCursor>,
    ) -> BoxFuture<'_, WorkOutcome<SnapshotDescriptor<M>>> {
        Box::pin(async move {
            validate_work(limits)?;
            scope_id(&scope)?;
            if request_id.is_nil() {
                return Err(SyncError::Invalid("nil snapshot request id".into()));
            }
            let mut s = self.lock()?;
            if s.retired_requests.contains_key(&request_id) {
                return Err(SyncError::SnapshotExpired);
            }
            if let Some(existing) = s
                .snapshots
                .iter()
                .find(|view| view.request_id == request_id)
            {
                if existing.descriptor.scope != scope || existing.request_limits != limits {
                    return Err(SyncError::Invalid(
                        "snapshot request parameters changed".into(),
                    ));
                }
                if resume.is_none() {
                    if existing.ready {
                        return Ok(WorkOutcome::Ready(existing.descriptor.clone()));
                    }
                    let build = s
                        .builds
                        .iter()
                        .find(|build| build.snapshot == existing.descriptor.snapshot_id)
                        .ok_or(SyncError::StaleWork)?;
                    return Ok(WorkOutcome::Incomplete {
                        cursor: WorkCursor {
                            operation_id: build.id,
                            generation: s.core.identity.generation,
                            position: build.position,
                        },
                    });
                }
            } else if resume.is_some() {
                return Err(SyncError::SnapshotExpired);
            }
            let generation = s.core.identity.generation;
            let new_build = resume.is_none();
            let input_position = resume.as_ref().map(|cursor| cursor.position);
            let build_id = if let Some(cursor) = resume {
                if cursor.generation != generation {
                    return Err(SyncError::StaleWork);
                }
                if let Some(CompletedWork::Open { snapshot, .. }) = s.completed_work.iter().find(|work| matches!(work, CompletedWork::Open { id, input, scope: saved_scope, snapshot, .. } if *id == cursor.operation_id && *input == cursor.position && *saved_scope == scope && s.snapshots.iter().any(|view| view.request_id == request_id && view.descriptor.snapshot_id == snapshot.snapshot_id))) { return Ok(WorkOutcome::Ready(snapshot.clone())); }
                let build = s
                    .builds
                    .iter()
                    .find(|b| b.id == cursor.operation_id)
                    .ok_or(SyncError::StaleWork)?;
                let view = s
                    .snapshots
                    .iter()
                    .find(|v| v.descriptor.snapshot_id == build.snapshot)
                    .ok_or(SyncError::SnapshotExpired)?;
                if view.descriptor.scope != scope || view.request_id != request_id {
                    return Err(SyncError::StaleWork);
                }
                if cursor.position.checked_add(1) == Some(build.position) {
                    return Ok(WorkOutcome::Incomplete {
                        cursor: WorkCursor {
                            operation_id: build.id,
                            generation,
                            position: build.position,
                        },
                    });
                }
                if build.position != cursor.position {
                    return Err(SyncError::StaleWork);
                }
                cursor.operation_id
            } else {
                if s.snapshots.len() >= DEFAULT_STAGES as usize {
                    return Err(SyncError::StageQuota);
                }
                let snapshot = Uuid::new_v4();
                let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
                let descriptor = SnapshotDescriptor {
                    source: s.core.identity.clone(),
                    scope: scope.clone(),
                    snapshot_id: snapshot,
                    head: s.core.head()?,
                    roots: Vec::new(),
                    contexts: Vec::new(),
                    expires_unix_ms: now
                        .checked_add(DEFAULT_SNAPSHOT_TTL_MS)
                        .ok_or(SyncError::ClockExhausted)?,
                };
                let core = s.core.clone();
                s.snapshots.push(Snapshot {
                    request_id,
                    request_limits: limits,
                    descriptor,
                    core,
                    objects: Vec::new(),
                    ready: false,
                    summary_cache: Vec::new(),
                });
                let mut collections: Vec<_> = scope.collections.iter().cloned().collect();
                collections.sort_by_key(M::collection_tag);
                let id = Uuid::new_v4();
                s.builds.push(SnapshotBuild {
                    id,
                    snapshot,
                    collections,
                    index: 0,
                    summary: None,
                    position: 0,
                });
                id
            };
            let index = s
                .builds
                .iter()
                .position(|b| b.id == build_id)
                .ok_or(SyncError::StaleWork)?;
            let mut build = s.builds[index].clone();
            let new_summary = build.summary.is_none();
            let summary_id = if let Some(id) = build.summary {
                id
            } else {
                let range = KeyRange {
                    collection: build.collections[build.index].clone(),
                    lower: None,
                    upper: None,
                };
                if s.summaries.len() + s.splits.len() >= MAX_SOURCE_WORK_ITEMS {
                    if new_build {
                        s.builds.remove(index);
                        s.snapshots
                            .retain(|view| view.descriptor.snapshot_id != build.snapshot);
                    }
                    return Err(SyncError::StageQuota);
                }
                let work = create_summary(build.snapshot, range);
                let id = work.id;
                s.summaries.push(work);
                build.summary = Some(id);
                id
            };
            let stepped = step_summary(&mut s, summary_id, limits);
            let (summary, _) = match stepped {
                Ok(value) => value,
                Err(error) => {
                    if new_summary {
                        s.summaries.retain(|w| w.id != summary_id);
                    }
                    if new_build {
                        s.builds.remove(index);
                        s.snapshots
                            .retain(|v| v.descriptor.snapshot_id != build.snapshot);
                    }
                    return Err(error);
                }
            };
            if let Some(summary) = summary {
                let view = s
                    .snapshots
                    .iter_mut()
                    .find(|v| v.descriptor.snapshot_id == build.snapshot)
                    .ok_or(SyncError::SnapshotExpired)?;
                let context = exact_context(&view.core.base(&summary.range.collection));
                let bytes = canonical_context(&context)?;
                let manifest =
                    manifest_for(ManifestKind::Context, &bytes, context.writers.len() as u64)?;
                view.descriptor.contexts.push(CollectionContext {
                    collection: summary.range.collection.clone(),
                    context: ContextRef {
                        digest: context_digest(&context)?,
                        manifest: manifest.clone(),
                    },
                });
                view.objects.push((manifest, bytes));
                view.descriptor.roots.push(summary);
                build.index += 1;
                build.summary = None;
            }
            if build.index == build.collections.len() {
                s.builds.remove(index);
                let view = s
                    .snapshots
                    .iter_mut()
                    .find(|v| v.descriptor.snapshot_id == build.snapshot)
                    .ok_or(SyncError::SnapshotExpired)?;
                view.ready = true;
                let descriptor = view.descriptor.clone();
                if let Some(input) = input_position {
                    if s.completed_work.len() >= MAX_RECEIVER_RETRY_RECEIPTS {
                        s.completed_work.remove(0);
                    }
                    s.completed_work.push(CompletedWork::Open {
                        id: build_id,
                        input,
                        scope,
                        snapshot: descriptor.clone(),
                    });
                }
                return Ok(WorkOutcome::Ready(descriptor));
            }
            build.position = build
                .position
                .checked_add(1)
                .ok_or(SyncError::ClockExhausted)?;
            let position = build.position;
            s.builds[index] = build;
            Ok(WorkOutcome::Incomplete {
                cursor: WorkCursor {
                    operation_id: build_id,
                    generation,
                    position,
                },
            })
        })
    }
    fn summarize(
        &self,
        snapshot: Uuid,
        range: KeyRange<M>,
        limits: WorkLimits,
        resume: Option<WorkCursor>,
    ) -> BoxFuture<'_, WorkOutcome<RangeSummary<M>>> {
        Box::pin(async move {
            validate_work(limits)?;
            let mut s = self.lock()?;
            let generation = s.core.identity.generation;
            let view = view_mut(&mut s, snapshot)?;
            if !view
                .descriptor
                .scope
                .collections
                .contains(&range.collection)
                || range.lower.zip(range.upper).is_some_and(|(a, b)| a >= b)
            {
                return Err(SyncError::Invalid("invalid summary range".into()));
            }
            if resume.is_none()
                && let Some(summary) = view
                    .summary_cache
                    .iter()
                    .find(|summary| summary.range == range)
            {
                return Ok(WorkOutcome::Ready(summary.clone()));
            }
            let input_position = resume.as_ref().map(|cursor| cursor.position);
            let (id, created) = if let Some(cursor) = resume {
                if cursor.generation != generation {
                    return Err(SyncError::StaleWork);
                }
                if let Some(CompletedWork::Summary { result, .. }) = s.completed_work.iter().find(|work| matches!(work, CompletedWork::Summary { id, input, snapshot: saved_snapshot, range: saved_range, .. } if *id == cursor.operation_id && *input == cursor.position && *saved_snapshot == snapshot && *saved_range == range)) { return Ok(WorkOutcome::Ready(result.clone())); }
                let work = s
                    .summaries
                    .iter()
                    .find(|w| w.id == cursor.operation_id)
                    .ok_or(SyncError::StaleWork)?;
                if work.snapshot != snapshot || work.range != range {
                    return Err(SyncError::StaleWork);
                }
                if work.position != cursor.position && work.last_input == Some(cursor.position) {
                    return Ok(WorkOutcome::Incomplete {
                        cursor: WorkCursor {
                            operation_id: work.id,
                            generation,
                            position: work.position,
                        },
                    });
                }
                if work.position != cursor.position {
                    return Err(SyncError::StaleWork);
                }
                (cursor.operation_id, false)
            } else {
                if s.summaries.len() + s.splits.len() >= MAX_SOURCE_WORK_ITEMS {
                    return Err(SyncError::StageQuota);
                }
                let work = create_summary(snapshot, range.clone());
                let id = work.id;
                s.summaries.push(work);
                (id, true)
            };
            let (result, position) = match step_summary(&mut s, id, limits) {
                Ok(value) => value,
                Err(error) => {
                    if created {
                        s.summaries.retain(|w| w.id != id)
                    }
                    return Err(error);
                }
            };
            match result {
                Some(summary) => {
                    if let Some(input) = input_position {
                        if s.completed_work.len() >= MAX_RECEIVER_RETRY_RECEIPTS {
                            s.completed_work.remove(0);
                        }
                        s.completed_work.push(CompletedWork::Summary {
                            id,
                            input,
                            snapshot,
                            range,
                            result: summary.clone(),
                        });
                    }
                    Ok(WorkOutcome::Ready(summary))
                }
                None => {
                    if let Some(work) = s.summaries.iter_mut().find(|work| work.id == id) {
                        work.last_input = input_position;
                    }
                    Ok(WorkOutcome::Incomplete {
                        cursor: WorkCursor {
                            operation_id: id,
                            generation,
                            position,
                        },
                    })
                }
            }
        })
    }
    fn split(
        &self,
        snapshot: Uuid,
        range: KeyRange<M>,
        limits: WorkLimits,
        resume: Option<WorkCursor>,
    ) -> BoxFuture<'_, WorkOutcome<Vec<KeyRange<M>>>> {
        Box::pin(async move {
            validate_work(limits)?;
            let mut s = self.lock()?;
            let generation = s.core.identity.generation;
            let view = view_mut(&mut s, snapshot)?;
            if !view
                .descriptor
                .scope
                .collections
                .contains(&range.collection)
                || range.lower.zip(range.upper).is_some_and(|(a, b)| a >= b)
            {
                return Err(SyncError::Invalid("invalid split range".into()));
            }
            let input_position = resume.as_ref().map(|cursor| cursor.position);
            let (id, created) = if let Some(cursor) = resume {
                if cursor.generation != generation {
                    return Err(SyncError::StaleWork);
                }
                if let Some(CompletedWork::Split { result, .. }) = s.completed_work.iter().find(|work| matches!(work, CompletedWork::Split { id, input, snapshot: saved_snapshot, range: saved_range, .. } if *id == cursor.operation_id && *input == cursor.position && *saved_snapshot == snapshot && *saved_range == range)) { return Ok(WorkOutcome::Ready(result.clone())); }
                let work = s
                    .splits
                    .iter()
                    .find(|w| w.id == cursor.operation_id)
                    .ok_or(SyncError::StaleWork)?;
                if work.snapshot != snapshot || work.range != range {
                    return Err(SyncError::StaleWork);
                }
                if work.position != cursor.position && work.last_input == Some(cursor.position) {
                    return Ok(WorkOutcome::Incomplete {
                        cursor: WorkCursor {
                            operation_id: work.id,
                            generation,
                            position: work.position,
                        },
                    });
                }
                if work.position != cursor.position {
                    return Err(SyncError::StaleWork);
                }
                (cursor.operation_id, false)
            } else {
                let id = Uuid::new_v4();
                if s.summaries.len() + s.splits.len() >= MAX_SOURCE_WORK_ITEMS {
                    return Err(SyncError::StageQuota);
                }
                s.splits.push(SplitWork {
                    id,
                    snapshot,
                    range: range.clone(),
                    position: 0,
                    phase: SplitPhase::CountCandidates,
                    candidate: None,
                    last_key: None,
                    distinct: 0,
                    selected: 0,
                    total_bytes: 0,
                    selected_bytes: 0,
                    last_input: None,
                });
                (id, true)
            };
            let index = s
                .splits
                .iter()
                .position(|w| w.id == id)
                .ok_or(SyncError::StaleWork)?;
            let mut work = s.splits[index].clone();
            let view = s
                .snapshots
                .iter()
                .find(|v| v.descriptor.snapshot_id == snapshot && v.ready)
                .ok_or(SyncError::SnapshotExpired)?;
            let result = match advance_split(view, &mut work, limits) {
                Ok(value) => value,
                Err(error) => {
                    if created {
                        s.splits.remove(index);
                    }
                    return Err(error);
                }
            };
            match result {
                Some(children) => {
                    s.splits.remove(index);
                    if let Some(input) = input_position {
                        if s.completed_work.len() >= MAX_RECEIVER_RETRY_RECEIPTS {
                            s.completed_work.remove(0);
                        }
                        s.completed_work.push(CompletedWork::Split {
                            id,
                            input,
                            snapshot,
                            range,
                            result: children.clone(),
                        });
                    }
                    Ok(WorkOutcome::Ready(children))
                }
                None => {
                    let position = work.position;
                    work.last_input = input_position;
                    s.splits[index] = work;
                    Ok(WorkOutcome::Incomplete {
                        cursor: WorkCursor {
                            operation_id: id,
                            generation,
                            position,
                        },
                    })
                }
            }
        })
    }
    fn leaf(
        &self,
        snapshot: Uuid,
        range: KeyRange<M>,
        after: Option<CandidateCursor<M>>,
        limits: PageLimits,
    ) -> BoxFuture<'_, LeafPage<M>> {
        Box::pin(async move {
            if limits.max_scanned == 0
                || limits.max_scanned > MAX_PAGE_ITEMS
                || limits.max_bytes == 0
                || limits.max_bytes > MAX_PAGE_BYTES
            {
                return Err(SyncError::Invalid("invalid page limits".into()));
            }
            let mut s = self.lock()?;
            let view = view_mut(&mut s, snapshot)?;
            if !view
                .descriptor
                .scope
                .collections
                .contains(&range.collection)
                || range.lower.zip(range.upper).is_some_and(|(a, b)| a >= b)
                || after.as_ref().is_some_and(|c| {
                    c.key.collection != range.collection
                        || range.lower.is_some_and(|v| c.key.id < v)
                        || range.upper.is_some_and(|v| c.key.id >= v)
                })
            {
                return Err(SyncError::StaleWork);
            }
            let mut candidates = Vec::new();
            let mut cursor = after.clone();
            let mut encoded_items = 0u64;
            let mut done = false;
            loop {
                let next = view.core.next_candidate(&range, cursor.as_ref())?;
                let Some(candidate) = next else {
                    done = true;
                    break;
                };
                let next_cursor = CandidateCursor {
                    key: candidate.key.clone(),
                    dot: candidate.dot,
                };
                let item_size = serde_json::to_vec(&candidate)
                    .map_err(|e| SyncError::Storage(e.to_string()))?
                    .len() as u64;
                let empty = LeafPage::<M> {
                    snapshot_id: snapshot,
                    range: range.clone(),
                    after: after.clone(),
                    next: Some(next_cursor.clone()),
                    done: false,
                    candidates: Vec::new(),
                };
                let envelope = serde_json::to_vec(&empty)
                    .map_err(|e| SyncError::Storage(e.to_string()))?
                    .len() as u64;
                let size = envelope
                    .checked_add(encoded_items)
                    .and_then(|n| n.checked_add(item_size))
                    .and_then(|n| n.checked_add(candidates.len() as u64))
                    .ok_or(SyncError::LogicalLimit)?;
                if candidates.len() >= limits.max_scanned as usize || size > limits.max_bytes {
                    if candidates.is_empty() {
                        return Err(SyncError::LimitTooSmall { required: size });
                    }
                    break;
                }
                encoded_items += item_size;
                cursor = Some(next_cursor);
                candidates.push(candidate);
            }
            let page = LeafPage {
                snapshot_id: snapshot,
                range,
                after,
                next: cursor,
                done,
                candidates,
            };
            let size = serde_json::to_vec(&page)
                .map_err(|e| SyncError::Storage(e.to_string()))?
                .len() as u64;
            if size > limits.max_bytes {
                return Err(SyncError::LimitTooSmall { required: size });
            }
            Ok(page)
        })
    }
    fn read_fragment(
        &self,
        snapshot: Uuid,
        manifest: ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> BoxFuture<'_, Fragment> {
        Box::pin(async move {
            if max_bytes == 0 || max_bytes > MAX_PAGE_BYTES {
                return Err(SyncError::Invalid("invalid fragment limit".into()));
            }
            let mut s = self.lock()?;
            let view = view_mut(&mut s, snapshot)?;
            let (_, bytes) = view
                .objects
                .iter()
                .find(|(m, _)| *m == manifest)
                .ok_or(SyncError::StaleWork)?;
            if ordinal >= manifest.fragment_count {
                return Err(SyncError::Invalid("fragment ordinal".into()));
            }
            let (start, end) = memory_fragment_bounds(bytes, &manifest, ordinal)?;
            if (end - start) as u64 > max_bytes {
                return Err(SyncError::LimitTooSmall {
                    required: (end - start) as u64,
                });
            }
            let mut f = Fragment {
                manifest,
                ordinal,
                offset: start as u64,
                bytes: bytes[start..end].to_vec(),
                digest: [0; 32],
            };
            f.digest = fragment_digest(&f)?;
            check_fragment_size(&f, max_bytes)?;
            Ok(f)
        })
    }
}

impl<M: SyncModel> SnapshotSink<M> for MemorySyncStore<M> {
    fn discard_range(&self, session: Uuid, range: KeyRange<M>) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.lock()?.receiver.discard_range(session, &range) })
    }
    fn cancel_reconciliation(&self, session: Uuid) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.lock()?.receiver.cancel_reconciliation(session) })
    }
    fn begin_reconciliation(
        &self,
        request_id: Uuid,
        remote: SnapshotDescriptor<M>,
        local: SnapshotDescriptor<M>,
    ) -> BoxFuture<'_, Uuid> {
        Box::pin(async move {
            let mut state = self.lock()?;
            if view_mut(&mut state, local.snapshot_id)?.descriptor != local {
                return Err(SyncError::Invalid("local snapshot mismatch".into()));
            }
            let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
            state
                .receiver
                .begin_reconciliation(request_id, remote, local, now)
        })
    }

    fn coverage(
        &self,
        session: Uuid,
        revision: Option<u64>,
        after: u64,
        limits: PageLimits,
    ) -> BoxFuture<'_, CoveragePage<M>> {
        Box::pin(async move {
            self.lock()?
                .receiver
                .coverage(session, revision, after, limits)
        })
    }

    fn split_range(
        &self,
        session: Uuid,
        parent: KeyRange<M>,
        children: Vec<RangeSummary<M>>,
    ) -> BoxFuture<'_, CoverageProgress> {
        Box::pin(async move { self.lock()?.receiver.split_range(session, parent, children) })
    }

    fn accept_equal_range(
        &self,
        session: Uuid,
        source: RangeSummary<M>,
    ) -> BoxFuture<'_, CoverageProgress> {
        Box::pin(async move {
            let mut state = self.lock()?;
            let active = state
                .sessions
                .iter()
                .find(|s| s.id == session)
                .ok_or(SyncError::StaleWork)?;
            if active.completed.contains(&source) {
                return state.receiver.progress(session);
            }
            let local_id = active.local.snapshot_id;
            let local = view_mut(&mut state, local_id)?
                .summary_cache
                .iter()
                .find(|summary| summary.range == source.range)
                .cloned()
                .ok_or(SyncError::StaleWork)?;
            state.receiver.accept_equal_range(session, source, &local)
        })
    }

    fn leaf_progress(&self, session: Uuid, range: KeyRange<M>) -> BoxFuture<'_, LeafProgress<M>> {
        Box::pin(async move { self.lock()?.receiver.leaf_progress(session, &range) })
    }

    fn stage_leaf_page(&self, session: Uuid, page: LeafPage<M>) -> BoxFuture<'_, LeafProgress<M>> {
        Box::pin(async move { self.lock()?.receiver.stage_leaf_page(session, page) })
    }

    fn commit_complete_range(
        &self,
        session: Uuid,
        range: KeyRange<M>,
        limits: ApplyLimits,
    ) -> BoxFuture<'_, (CoverageProgress, ProjectionBatch<M>)> {
        Box::pin(async move {
            let mut guard = self.lock()?;
            let mut state = guard.clone();
            let active = state
                .sessions
                .iter()
                .find(|s| s.id == session)
                .ok_or(SyncError::StaleWork)?;
            if active
                .completed
                .iter()
                .any(|summary| summary.range == range)
            {
                return Ok((
                    state.receiver.progress(session)?,
                    ProjectionBatch {
                        changes: Vec::new(),
                        causal_changes: 0,
                        pending: 0,
                    },
                ));
            }
            let prepared = state.receiver.prepare_range(session, &range)?;
            if prepared.candidates.len() as u64 + 1 > limits.max_mutations {
                return Err(SyncError::WorkLimit);
            }
            let (changed, changes) =
                state
                    .core
                    .merge_leaf(&range, &prepared.candidates, &prepared.source_context)?;
            check_limits(limits, &changes)?;
            let progress = state.receiver.finalize_range(&prepared.token)?;
            *guard = state;
            Ok((
                progress,
                ProjectionBatch {
                    changes,
                    causal_changes: u64::from(changed),
                    pending: 0,
                },
            ))
        })
    }
    fn promote_step(&self, session: Uuid, limits: ApplyLimits) -> BoxFuture<'_, PromoteProgress> {
        Box::pin(async move {
            check_limits(limits, &[] as &[ProjectionChange<M>])?;
            let mut guard = self.lock()?;
            let mut state = guard.clone();
            let Some(token) = state
                .receiver
                .prepare_promotion(session, (limits.max_mutations / 4).clamp(1, 1024) as usize)?
            else {
                return Ok(PromoteProgress {
                    done: true,
                    causal_changes: 0,
                });
            };
            let (collection, observed) = match &token.step {
                PromotionStep::Arrivals {
                    collection,
                    source_context,
                    source_live,
                } => {
                    let (_, local) = state.receiver.descriptors(session)?;
                    let snapshot = view_mut(&mut state, local.snapshot_id)?;
                    let mut observed = crate::causal::Context::default();
                    let pinned = snapshot.core.clone();
                    let mut arrivals = 0u64;
                    for c in state.core.live(collection) {
                        if !pinned.candidates(&c.key).iter().any(|v| v.dot == c.dot) {
                            arrivals += 1;
                            if arrivals > limits.max_mutations {
                                return Err(SyncError::WorkLimit);
                            }
                            let dot = crate::causal::Dot {
                                writer: c.dot.writer_incarnation,
                                counter: c.dot.counter,
                            };
                            if source_context.contains(dot) && !source_live.contains(dot) {
                                observed
                                    .insert(dot)
                                    .map_err(|e| SyncError::Invalid(e.to_string()))?;
                            }
                        }
                    }
                    (collection.clone(), observed)
                }
                PromotionStep::Chunk { collection, chunk } => (
                    collection.clone(),
                    chunk
                        .difference(&state.core.base(collection))
                        .map_err(|e| SyncError::Invalid(e.to_string()))?,
                ),
            };
            let mut causal_changes = 0;
            if !observed.is_empty() {
                let (changed, changes) = state.core.apply_merge(&MergeDelta {
                    collection,
                    added: Vec::new(),
                    observed: exact_context(&observed),
                })?;
                check_limits(limits, &changes)?;
                causal_changes = u64::from(changed);
            }
            state.receiver.finalize_promotion(&token)?;
            let done = state.receiver.promotion_done(session)?;
            *guard = state;
            Ok(PromoteProgress {
                done,
                causal_changes,
            })
        })
    }

    fn complete_reconciliation(&self, session: Uuid) -> BoxFuture<'_, ReceiveProgress> {
        Box::pin(async move {
            let mut s = self.lock()?;
            if let Some((_, progress, _)) = s
                .completed_sessions
                .iter()
                .find(|(id, _, _)| *id == session)
            {
                return Ok(progress.clone());
            }
            let index = s
                .sessions
                .iter()
                .position(|x| x.id == session)
                .ok_or(SyncError::StaleWork)?;
            let active = s.sessions[index].clone();
            if !active.pending.is_empty() {
                return Err(SyncError::IncompleteCoverage);
            }
            let key = CheckpointKey {
                namespace: active.remote.source.namespace,
                source_replica: active.remote.source.replica,
                source_generation: active.remote.source.generation,
                scope_id: scope_id(&active.remote.scope)?,
            };
            let progress = match lookup(&s.checkpoints, &key) {
                Some(existing) if existing.sequence > active.remote.head.sequence => {
                    return Err(SyncError::RetryAt(existing));
                }
                Some(existing)
                    if existing.sequence == active.remote.head.sequence
                        && existing.anchor != active.remote.head.anchor =>
                {
                    return Err(SyncError::RetryAt(existing));
                }
                _ => active.remote.head.clone(),
            };
            let token = s.receiver.prepare_completion(session)?;
            s.receiver.finalize_completion(&token, progress.clone())?;
            s.accepted_anchors.push((key.clone(), progress.clone()));
            set_checkpoint(&mut s.checkpoints, key, progress.clone());
            Ok(progress)
        })
    }
}

fn create_summary<M: SyncModel>(snapshot: Uuid, range: KeyRange<M>) -> SummaryWork<M> {
    SummaryWork {
        id: Uuid::new_v4(),
        snapshot,
        range,
        position: 0,
        phase: SummaryPhase::CountCandidates,
        candidate: None,
        candidate_count: 0,
        candidate_bytes: 0,
        hash: None,
        last_input: None,
    }
}
/// A step scans at most the requested number of entries and canonical bytes.
fn advance_summary<M: SyncModel>(
    view: &Snapshot<M>,
    work: &mut SummaryWork<M>,
    limits: WorkLimits,
) -> Result<Option<RangeSummary<M>>> {
    let mut rows = 0u64;
    let mut bytes = 0u64;
    loop {
        if rows == limits.max_rows_scanned {
            return Ok(None);
        }
        match work.phase {
            SummaryPhase::CountCandidates | SummaryPhase::HashCandidates => {
                let candidate = view
                    .core
                    .next_candidate(&work.range, work.candidate.as_ref())?;
                let Some(candidate) = candidate else {
                    match work.phase {
                        SummaryPhase::CountCandidates => {
                            work.hash = Some(RangeHasher::new(
                                &view.descriptor.scope,
                                work.range.clone(),
                                work.candidate_count,
                                work.candidate_bytes,
                            )?);
                            work.phase = SummaryPhase::HashCandidates;
                            work.candidate = None;
                            continue;
                        }
                        SummaryPhase::HashCandidates => {
                            let aggregate =
                                work.hash.take().ok_or(SyncError::StaleWork)?.finish()?;
                            return Ok(Some(RangeSummary {
                                snapshot_id: work.snapshot,
                                range: work.range.clone(),
                                candidate_count: work.candidate_count,
                                candidate_bytes: work.candidate_bytes,
                                aggregate,
                            }));
                        }
                    }
                };
                let encoded = canonical_candidate(&candidate)?;
                let size = encoded.len() as u64;
                if rows == limits.max_rows_scanned
                    || bytes.checked_add(size).is_none_or(|n| n > limits.max_bytes)
                {
                    if rows == 0 {
                        return Err(SyncError::LimitTooSmall { required: size });
                    }
                    return Ok(None);
                }
                rows += 1;
                bytes += size;
                work.position = work
                    .position
                    .checked_add(1)
                    .ok_or(SyncError::ClockExhausted)?;
                match work.phase {
                    SummaryPhase::CountCandidates => {
                        work.candidate_count = work
                            .candidate_count
                            .checked_add(1)
                            .ok_or(SyncError::LogicalLimit)?;
                        work.candidate_bytes = work
                            .candidate_bytes
                            .checked_add(size)
                            .ok_or(SyncError::LogicalLimit)?;
                    }
                    SummaryPhase::HashCandidates => work
                        .hash
                        .as_mut()
                        .ok_or(SyncError::StaleWork)?
                        .push_candidate(&candidate)?,
                }
                work.candidate = Some(CandidateCursor {
                    key: candidate.key,
                    dot: candidate.dot,
                });
            }
        }
    }
}
fn step_summary<M: SyncModel>(
    state: &mut State<M>,
    id: Uuid,
    limits: WorkLimits,
) -> Result<(Option<RangeSummary<M>>, u64)> {
    let index = state
        .summaries
        .iter()
        .position(|w| w.id == id)
        .ok_or(SyncError::StaleWork)?;
    let mut work = state.summaries[index].clone();
    let view = state
        .snapshots
        .iter()
        .find(|v| v.descriptor.snapshot_id == work.snapshot)
        .ok_or(SyncError::SnapshotExpired)?;
    let result = advance_summary(view, &mut work, limits)?;
    let position = work.position;
    if let Some(summary) = &result {
        state.summaries.remove(index);
        let view = state
            .snapshots
            .iter_mut()
            .find(|v| v.descriptor.snapshot_id == work.snapshot)
            .ok_or(SyncError::SnapshotExpired)?;
        if !view.summary_cache.iter().any(|s| s.range == summary.range) {
            if view.summary_cache.len() >= MAX_RECEIVER_METADATA_ITEMS {
                view.summary_cache.remove(0);
            }
            view.summary_cache.push(summary.clone())
        }
    } else {
        state.summaries[index] = work;
    }
    Ok((result, position))
}
fn split_result<M: SyncModel>(range: &KeyRange<M>, pivot: Option<Uuid>) -> Vec<KeyRange<M>> {
    match pivot {
        None => vec![range.clone()],
        Some(p) => vec![
            KeyRange {
                collection: range.collection.clone(),
                lower: range.lower,
                upper: Some(p),
            },
            KeyRange {
                collection: range.collection.clone(),
                lower: Some(p),
                upper: range.upper,
            },
        ],
    }
}
fn advance_split<M: SyncModel>(
    view: &Snapshot<M>,
    work: &mut SplitWork<M>,
    limits: WorkLimits,
) -> Result<Option<Vec<KeyRange<M>>>> {
    let mut rows = 0u64;
    let mut bytes = 0u64;
    loop {
        if rows == limits.max_rows_scanned {
            return Ok(None);
        }
        match work.phase {
            SplitPhase::CountCandidates | SplitPhase::FindCandidate => {
                let next = view
                    .core
                    .next_candidate(&work.range, work.candidate.as_ref())?;
                let Some(candidate) = next else {
                    match work.phase {
                        SplitPhase::CountCandidates => {
                            if work.distinct == 0 {
                                return Ok(Some(vec![work.range.clone()]));
                            }
                            work.phase = SplitPhase::FindCandidate;
                            work.candidate = None;
                            work.last_key = None;
                            work.selected = 0;
                            work.selected_bytes = 0;
                            continue;
                        }
                        SplitPhase::FindCandidate => {
                            return Ok(Some(split_result(&work.range, None)));
                        }
                    }
                };
                let size = canonical_candidate(&candidate)?.len() as u64;
                if rows == limits.max_rows_scanned
                    || bytes.checked_add(size).is_none_or(|n| n > limits.max_bytes)
                {
                    if rows == 0 {
                        return Err(SyncError::LimitTooSmall { required: size });
                    }
                    return Ok(None);
                }
                rows += 1;
                bytes += size;
                work.position = work
                    .position
                    .checked_add(1)
                    .ok_or(SyncError::ClockExhausted)?;
                let key = candidate.key.id;
                if matches!(work.phase, SplitPhase::CountCandidates) {
                    work.total_bytes = work
                        .total_bytes
                        .checked_add(size)
                        .ok_or(SyncError::LogicalLimit)?;
                }
                if work.last_key != Some(key) && work.range.lower.is_none_or(|lower| key > lower) {
                    match work.phase {
                        SplitPhase::CountCandidates => {
                            work.distinct = work
                                .distinct
                                .checked_add(1)
                                .ok_or(SyncError::LogicalLimit)?
                        }
                        SplitPhase::FindCandidate => {
                            if work.selected_bytes >= work.total_bytes / 2 {
                                return Ok(Some(split_result(&work.range, Some(key))));
                            }
                            work.selected += 1;
                        }
                    }
                }
                if matches!(work.phase, SplitPhase::FindCandidate) {
                    work.selected_bytes = work
                        .selected_bytes
                        .checked_add(size)
                        .ok_or(SyncError::LogicalLimit)?;
                }
                work.last_key = Some(key);
                work.candidate = Some(CandidateCursor {
                    key: candidate.key,
                    dot: candidate.dot,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Model;
    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    enum Collection {
        Notes,
    }
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct Value {
        n: u32,
    }
    impl SyncModel for Model {
        type Collection = Collection;
        type Payload = Value;
        fn model_id() -> &'static str {
            "memory-test"
        }
        fn schema_id() -> &'static str {
            "one"
        }
        fn collections() -> Vec<Collection> {
            vec![Collection::Notes]
        }
        fn collection_tag(_: &Collection) -> &'static str {
            "notes"
        }
        fn collection_from_tag(s: &str) -> Option<Collection> {
            (s == "notes").then_some(Collection::Notes)
        }
        fn validate(_: &ObjectKey<Self>, _: &Option<Value>) -> Result<()> {
            Ok(())
        }
        fn validate_revision(_: &ObjectKey<Self>, _: &Option<Value>, _: &Revision) -> Result<()> {
            Ok(())
        }
    }
    fn store(n: u128) -> MemorySyncStore<Model> {
        MemorySyncStore::new(
            StoreIdentity {
                namespace: Uuid::from_u128(1),
                model: ModelId {
                    model: "memory-test".into(),
                    schema: "one".into(),
                },
                replica: Uuid::from_u128(n),
                generation: Uuid::from_u128(n + 10),
            },
            LocalAuthor {
                author: Uuid::from_u128(n),
                writer_incarnation: Uuid::from_u128(n + 20),
            },
        )
        .unwrap()
    }
    #[tokio::test]
    async fn pages_require_first_contact_and_commit_checkpoints_atomically() {
        let a = store(2);
        let b = store(3);
        let scope = Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let key = ObjectKey {
            collection: Collection::Notes,
            id: Uuid::from_u128(42),
        };
        a.write_local(key.clone(), Some(Value { n: 4 }))
            .await
            .unwrap();
        let source = ReceiveStore::identity(&a).await.unwrap();
        let initial = ReceiveProgress {
            sequence: 0,
            anchor: initial_anchor(&source).unwrap(),
        };
        let through = a.head().await.unwrap();
        let request = PageRequest {
            source,
            scope,
            after: initial.clone(),
            through,
            limits: PageLimits {
                max_scanned: 10,
                max_bytes: 65536,
            },
        };
        let PageRead::Page(page) = a.read_page(request.clone()).await.unwrap() else {
            panic!()
        };
        let limits = ApplyLimits {
            max_projection_items: 10,
            max_projection_bytes: 65536,
            max_mutations: 10,
        };
        assert!(matches!(
            b.apply_page(&page, limits).await,
            Err(SyncError::NeedsReconciliation(_))
        ));
        let ck = checkpoint_key(&request).unwrap();
        set_checkpoint(&mut b.lock().unwrap().checkpoints, ck.clone(), initial);
        let tiny = ApplyLimits {
            max_projection_bytes: 1,
            ..limits
        };
        assert!(b.apply_page(&page, tiny).await.is_err());
        assert!(b.get(&key).await.unwrap().is_none());
        assert_eq!(b.checkpoint(&ck).await.unwrap().unwrap().sequence, 0);
        let applied = b.apply_page(&page, limits).await.unwrap();
        assert_eq!(applied.projection.changes.len(), 1);
        assert_eq!(
            b.get(&key).await.unwrap().unwrap().value,
            Some(Value { n: 4 })
        );
        assert!(
            b.apply_page(&page, limits)
                .await
                .unwrap()
                .projection
                .changes
                .is_empty()
        );
    }
    #[tokio::test]
    async fn complete_range_removes_observed_and_keeps_concurrent_candidate() {
        let a = store(2);
        let b = store(3);
        let key = ObjectKey {
            collection: Collection::Notes,
            id: Uuid::from_u128(42),
        };
        a.write_local(key.clone(), Some(Value { n: 1 }))
            .await
            .unwrap();
        let first = a.lock().unwrap().core.journal()[0].event.clone();
        let Event::Key(first) = first else { panic!() };
        b.lock().unwrap().core.apply_delta(&first).unwrap();
        b.write_observed(
            key.clone(),
            Some(Value { n: 2 }),
            Observed::Exact(Default::default()),
        )
        .await
        .unwrap();
        a.write_local(key.clone(), None).await.unwrap();
        let source = a.lock().unwrap().core.clone();
        let range = KeyRange {
            collection: Collection::Notes,
            lower: None,
            upper: None,
        };
        let (changed, changes) = b
            .lock()
            .unwrap()
            .core
            .merge_leaf(
                &range,
                &source.live(&Collection::Notes),
                &source.base(&Collection::Notes),
            )
            .unwrap();
        assert!(changed);
        assert_eq!(
            b.get(&key).await.unwrap().unwrap().value,
            Some(Value { n: 2 })
        );
        assert!(changes.is_empty());
        let (changed, _) = b.lock().unwrap().core.apply_delta(&first).unwrap();
        assert!(!changed);
    }
    #[test]
    fn page_rejects_merge_from_another_namespace() {
        let source = store(2).lock().unwrap().core.identity.clone();
        let foreign = Candidate::<Model> {
            model: source.model.clone(),
            namespace: Uuid::from_u128(900),
            key: ObjectKey {
                collection: Collection::Notes,
                id: Uuid::from_u128(42),
            },
            dot: Dot {
                writer_incarnation: Uuid::from_u128(3),
                counter: 1,
            },
            revision: Revision {
                actor: Uuid::from_u128(4),
                counter: 1,
            },
            payload: Value { n: 1 },
        };
        let event = Event::Merge(Box::new(MergeDelta {
            collection: Collection::Notes,
            added: vec![foreign],
            observed: ExactContext { writers: vec![] },
        }));
        let digest = event_digest(&event).unwrap();
        let encoded = canonical_event(&event).unwrap();
        assert_eq!(decode_event::<Model>(&encoded).unwrap(), event);
        let start = ReceiveProgress {
            sequence: 0,
            anchor: initial_anchor(&source).unwrap(),
        };
        let end = ReceiveProgress {
            sequence: 1,
            anchor: next_anchor::<Model>(start.anchor, 1, &Collection::Notes, digest).unwrap(),
        };
        let request = PageRequest {
            source,
            scope: Scope {
                collections: [Collection::Notes].into_iter().collect(),
            },
            after: start,
            through: end.clone(),
            limits: PageLimits {
                max_scanned: 1,
                max_bytes: 65536,
            },
        };
        let page = ChangePage {
            request,
            next: end,
            done: true,
            links: vec![ScanLink {
                sequence: 1,
                collection: Collection::Notes,
                event_digest: digest,
                event: Some(EventRef::Inline(event)),
            }],
        };
        assert!(matches!(
            validate_scanned_links(&page),
            Err(SyncError::Invalid(_))
        ));
    }
    #[tokio::test]
    async fn snapshot_is_immutable_and_exports_exact_context() {
        let store = store(2);
        let key = ObjectKey {
            collection: Collection::Notes,
            id: Uuid::from_u128(42),
        };
        store
            .write_local(key.clone(), Some(Value { n: 1 }))
            .await
            .unwrap();
        let work = WorkLimits {
            max_requests: 10,
            max_bytes: 65536,
            max_depth: 10,
            max_rows_scanned: 100,
            max_mutations: 100,
        };
        let WorkOutcome::Ready(snapshot) = store
            .open_snapshot(
                Uuid::from_u128(800),
                Scope {
                    collections: [Collection::Notes].into_iter().collect(),
                },
                work,
                None,
            )
            .await
            .unwrap()
        else {
            panic!()
        };
        store
            .write_local(key.clone(), Some(Value { n: 2 }))
            .await
            .unwrap();
        let root = snapshot.roots[0].range.clone();
        let page = store
            .leaf(
                snapshot.snapshot_id,
                root.clone(),
                None,
                PageLimits {
                    max_scanned: 10,
                    max_bytes: 65536,
                },
            )
            .await
            .unwrap();
        assert!(page.done);
        assert_eq!(page.candidates.len(), 1);
        assert_eq!(page.candidates[0].payload, Value { n: 1 });
        let reference = &snapshot.contexts[0].context;
        let fragment = SnapshotSource::read_fragment(
            &store,
            snapshot.snapshot_id,
            reference.manifest.clone(),
            0,
            65536,
        )
        .await
        .unwrap();
        assert_eq!(
            context_digest(&decode_context(&fragment.bytes).unwrap()).unwrap(),
            reference.digest
        );
        let WorkOutcome::Ready(summary) = store
            .summarize(snapshot.snapshot_id, root, work, None)
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(summary, snapshot.roots[0]);
        store.close_snapshot(snapshot.snapshot_id).await.unwrap();
        assert!(matches!(
            store
                .leaf(
                    snapshot.snapshot_id,
                    snapshot.roots[0].range.clone(),
                    None,
                    PageLimits {
                        max_scanned: 10,
                        max_bytes: 65536
                    }
                )
                .await,
            Err(SyncError::SnapshotExpired)
        ));
    }
    #[tokio::test]
    async fn first_contact_stages_complete_range_before_checkpoint() {
        let a = store(2);
        let b = store(3);
        let key = ObjectKey {
            collection: Collection::Notes,
            id: Uuid::from_u128(42),
        };
        a.write_local(key.clone(), Some(Value { n: 7 }))
            .await
            .unwrap();
        let scope = Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let work = WorkLimits {
            max_requests: 10,
            max_bytes: 65536,
            max_depth: 10,
            max_rows_scanned: 100,
            max_mutations: 100,
        };
        let WorkOutcome::Ready(remote) = a
            .open_snapshot(Uuid::from_u128(800), scope.clone(), work, None)
            .await
            .unwrap()
        else {
            panic!()
        };
        let WorkOutcome::Ready(local) = b
            .open_snapshot(Uuid::from_u128(800), scope, work, None)
            .await
            .unwrap()
        else {
            panic!()
        };
        let session = b
            .begin_reconciliation(Uuid::from_u128(900), remote.clone(), local.clone())
            .await
            .unwrap();
        assert_eq!(
            b.begin_reconciliation(Uuid::from_u128(900), remote.clone(), local.clone())
                .await
                .unwrap(),
            session
        );
        let mut altered = remote.clone();
        altered.roots[0].aggregate[0] ^= 1;
        assert!(matches!(
            b.begin_reconciliation(Uuid::from_u128(900), altered, local)
                .await,
            Err(SyncError::Invalid(_))
        ));
        let range = remote.roots[0].range.clone();
        assert!(matches!(
            b.complete_reconciliation(session).await,
            Err(SyncError::IncompleteCoverage)
        ));
        let pages = PageLimits {
            max_scanned: 10,
            max_bytes: 65536,
        };
        let stage_limits = StageLimits {
            max_total_bytes: DEFAULT_STAGE_BYTES,
            max_stages: DEFAULT_STAGES,
            max_event_bytes: DEFAULT_LOGICAL_BYTES,
            max_event_items: DEFAULT_LOGICAL_ITEMS,
            max_fragment_bytes: DEFAULT_PAGE_BYTES,
        };
        for context in remote.contexts.clone() {
            let descriptor = StageDescriptor {
                origin: StageOrigin::SnapshotContext {
                    session,
                    snapshot_id: remote.snapshot_id,
                    collection: context.collection,
                },
                manifest: context.context.manifest.clone(),
            };
            let stage = b
                .begin_stage(descriptor.clone(), stage_limits)
                .await
                .unwrap();
            assert_eq!(
                b.begin_stage(descriptor, stage_limits).await.unwrap().stage,
                stage.stage
            );
            for ordinal in 0..context.context.manifest.fragment_count {
                let fragment = SnapshotSource::read_fragment(
                    &a,
                    remote.snapshot_id,
                    context.context.manifest.clone(),
                    ordinal,
                    DEFAULT_PAGE_BYTES,
                )
                .await
                .unwrap();
                b.stage_fragment(stage.stage, fragment).await.unwrap();
            }
        }
        let leaf = a
            .leaf(remote.snapshot_id, range.clone(), None, pages)
            .await
            .unwrap();
        b.stage_leaf_page(session, leaf.clone()).await.unwrap();
        b.stage_leaf_page(session, leaf).await.unwrap();
        let limits = ApplyLimits {
            max_projection_items: 10,
            max_projection_bytes: 65536,
            max_mutations: 100,
        };
        let mut isolated = b.lock().unwrap().receiver.clone();
        let prepared = isolated.prepare_range(session, &range).unwrap();
        assert!(!isolated.progress(session).unwrap().complete);
        isolated
            .sessions
            .iter_mut()
            .find(|s| s.id == session)
            .unwrap()
            .revision += 1;
        assert!(matches!(
            isolated.finalize_range(&prepared.token),
            Err(SyncError::StaleWork)
        ));
        let (progress, projection) = b
            .commit_complete_range(session, range.clone(), limits)
            .await
            .unwrap();
        assert!(progress.complete);
        assert_eq!(projection.changes.len(), 1);
        assert!(
            b.commit_complete_range(session, range, limits)
                .await
                .unwrap()
                .1
                .changes
                .is_empty()
        );
        assert!(matches!(
            b.complete_reconciliation(session).await,
            Err(SyncError::IncompleteCoverage)
        ));
        while !b.promote_step(session, limits).await.unwrap().done {}
        let copy = MemorySyncStore {
            state: Mutex::new(b.lock().unwrap().clone()),
        };
        copy.cancel_reconciliation(session).await.unwrap();
        copy.cancel_reconciliation(session).await.unwrap();
        assert_eq!(
            copy.get(&key).await.unwrap().unwrap().value,
            Some(Value { n: 7 })
        );
        let checkpoint = CheckpointKey {
            namespace: remote.source.namespace,
            source_replica: remote.source.replica,
            source_generation: remote.source.generation,
            scope_id: scope_id(&remote.scope).unwrap(),
        };
        assert!(copy.checkpoint(&checkpoint).await.unwrap().is_none());
        assert_eq!(
            b.get(&key).await.unwrap().unwrap().value,
            Some(Value { n: 7 })
        );
        assert_eq!(
            b.complete_reconciliation(session).await.unwrap(),
            remote.head
        );
        assert_eq!(
            b.complete_reconciliation(session).await.unwrap(),
            remote.head
        );
        // Merge events can share an anchored page with an earlier key event.
        let multi_source = store(6);
        let multi_target = store(7);
        multi_source
            .write_local(
                ObjectKey {
                    collection: Collection::Notes,
                    id: Uuid::from_u128(77),
                },
                Some(Value { n: 77 }),
            )
            .await
            .unwrap();
        let event = {
            let state = b.lock().unwrap();
            let Event::Merge(event) = state.core.journal()[0].event.clone() else {
                panic!()
            };
            event
        };
        multi_source
            .lock()
            .unwrap()
            .core
            .apply_merge(&event)
            .unwrap();
        let multi_id = ReceiveStore::identity(&multi_source).await.unwrap();
        let multi_start = ReceiveProgress {
            sequence: 0,
            anchor: initial_anchor(&multi_id).unwrap(),
        };
        let multi_request = PageRequest {
            source: multi_id,
            scope: remote.scope.clone(),
            after: multi_start.clone(),
            through: multi_source.head().await.unwrap(),
            limits: pages,
        };
        let multi_key = checkpoint_key(&multi_request).unwrap();
        set_checkpoint(
            &mut multi_target.lock().unwrap().checkpoints,
            multi_key,
            multi_start,
        );
        let PageRead::Page(multi_page) =
            multi_source.read_page(multi_request.clone()).await.unwrap()
        else {
            panic!()
        };
        assert_eq!(multi_page.links.len(), 2);
        multi_target.apply_page(&multi_page, limits).await.unwrap();
        assert_eq!(
            multi_target.get(&key).await.unwrap().unwrap().value,
            Some(Value { n: 7 })
        );
        // Relay the committed merge after the original snapshot is closed.
        a.close_snapshot(remote.snapshot_id).await.unwrap();
        let c = store(4);
        let identity = ReceiveStore::identity(&b).await.unwrap();
        let initial = ReceiveProgress {
            sequence: 0,
            anchor: initial_anchor(&identity).unwrap(),
        };
        let request = PageRequest {
            source: identity,
            scope: remote.scope.clone(),
            after: initial.clone(),
            through: b.head().await.unwrap(),
            limits: pages,
        };
        let ck = checkpoint_key(&request).unwrap();
        set_checkpoint(&mut c.lock().unwrap().checkpoints, ck, initial);
        let PageRead::Page(page) = b.read_page(request.clone()).await.unwrap() else {
            panic!()
        };
        assert!(matches!(
            &page.links[0].event,
            Some(EventRef::Inline(Event::Merge(_)))
        ));
        c.apply_page(&page, limits).await.unwrap();
        assert_eq!(
            c.get(&key).await.unwrap().unwrap().value,
            Some(Value { n: 7 })
        );
        let d = store(5);
        let source = ReceiveStore::identity(&b).await.unwrap();
        let initial = ReceiveProgress {
            sequence: 0,
            anchor: initial_anchor(&source).unwrap(),
        };
        let key_id = CheckpointKey {
            namespace: source.namespace,
            source_replica: source.replica,
            source_generation: source.generation,
            scope_id: scope_id(&remote.scope).unwrap(),
        };
        set_checkpoint(&mut d.lock().unwrap().checkpoints, key_id, initial);
        let b = std::sync::Arc::new(b);
        let peer = StorePeer(b.clone());
        let limits = super::super::coordinator::SessionLimits {
            pages,
            stages: stage_limits,
            apply: limits,
            work,
        };
        let mut coordinator =
            super::super::coordinator::Coordinator::new(remote.scope.clone(), limits).unwrap();
        let mut done = false;
        for _ in 0..100 {
            coordinator.start_pass();
            if matches!(
                coordinator.advance(&d, &d, &d, &peer, &*b).await.unwrap(),
                super::super::coordinator::SessionAdvance::Complete(_)
            ) {
                done = true;
                break;
            }
        }
        assert!(done);
        assert_eq!(
            d.get(&key).await.unwrap().unwrap().value,
            Some(Value { n: 7 })
        );
    }
    #[tokio::test]
    async fn expired_stages_release_quota_without_advancing_progress() {
        let a = store(2);
        let b = store(3);
        let scope = Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let source = ReceiveStore::identity(&a).await.unwrap();
        let initial = ReceiveProgress {
            sequence: 0,
            anchor: initial_anchor(&source).unwrap(),
        };
        let request = PageRequest {
            source,
            scope,
            after: initial.clone(),
            through: ReceiveProgress {
                sequence: 1,
                anchor: [1; 32],
            },
            limits: PageLimits {
                max_scanned: 1,
                max_bytes: 65536,
            },
        };
        let ck = checkpoint_key(&request).unwrap();
        set_checkpoint(
            &mut b.lock().unwrap().checkpoints,
            ck.clone(),
            initial.clone(),
        );
        let mut manifest = ManifestRef {
            kind: ManifestKind::KeyEvent,
            manifest_digest: [0; 32],
            content_digest: content_digest(b"x").unwrap(),
            encoded_bytes: 1,
            item_count: 1,
            fragment_count: 1,
        };
        manifest.manifest_digest = manifest_digest(&manifest).unwrap();
        let limits = StageLimits {
            max_total_bytes: 10,
            max_stages: 1,
            max_event_bytes: 10,
            max_event_items: 10,
            max_fragment_bytes: 10,
        };
        let desc = StageDescriptor {
            origin: StageOrigin::Journal {
                request,
                sequence: 1,
                collection: Collection::Notes,
                event_digest: [1; 32],
            },
            manifest,
        };
        let stage = b.begin_stage(desc.clone(), limits).await.unwrap();
        b.lock()
            .unwrap()
            .stage_deadlines
            .iter_mut()
            .find(|(id, _)| *id == stage.stage)
            .unwrap()
            .1 = 0;
        assert!(matches!(
            b.stage_status(stage.stage).await,
            Err(SyncError::StaleWork)
        ));
        assert_eq!(b.checkpoint(&ck).await.unwrap(), Some(initial));
        let stage = b.begin_stage(desc.clone(), limits).await.unwrap();
        b.abandon_origin(desc.origin.clone()).await.unwrap();
        b.abandon_origin(desc.origin).await.unwrap();
        assert!(matches!(
            b.stage_status(stage.stage).await,
            Err(SyncError::StaleWork)
        ));
    }
    #[tokio::test]
    async fn coordinator_completes_first_contact_and_tail() {
        use super::super::coordinator::{Coordinator, SessionAdvance, SessionLimits};
        let a = std::sync::Arc::new(store(2));
        let b = std::sync::Arc::new(store(3));
        let key = ObjectKey {
            collection: Collection::Notes,
            id: Uuid::from_u128(42),
        };
        a.write_local(key.clone(), Some(Value { n: 8 }))
            .await
            .unwrap();
        let peer = StorePeer(a.clone());
        let limits = SessionLimits {
            pages: PageLimits {
                max_scanned: 10,
                max_bytes: 65536,
            },
            stages: StageLimits {
                max_total_bytes: DEFAULT_STAGE_BYTES,
                max_stages: DEFAULT_STAGES,
                max_event_bytes: DEFAULT_LOGICAL_BYTES,
                max_event_items: DEFAULT_LOGICAL_ITEMS,
                max_fragment_bytes: DEFAULT_PAGE_BYTES,
            },
            apply: ApplyLimits {
                max_projection_items: 10,
                max_projection_bytes: 65536,
                max_mutations: 100,
            },
            work: WorkLimits {
                max_requests: 10,
                max_bytes: 65536,
                max_depth: 10,
                max_rows_scanned: 100,
                max_mutations: 100,
            },
        };
        let mut coordinator = Coordinator::new(
            Scope {
                collections: [Collection::Notes].into_iter().collect(),
            },
            limits,
        )
        .unwrap();
        let mut complete = false;
        for _ in 0..100 {
            coordinator.start_pass();
            if matches!(
                coordinator
                    .advance(&*b, &*b, &*b, &peer, &*a)
                    .await
                    .unwrap(),
                SessionAdvance::Complete(_)
            ) {
                complete = true;
                break;
            }
        }
        assert!(complete);
        assert_eq!(
            b.get(&key).await.unwrap().unwrap().value,
            Some(Value { n: 8 })
        );
    }
    struct CountingSource {
        inner: Arc<MemorySyncStore<Model>>,
        calls: std::sync::atomic::AtomicUsize,
        candidates: std::sync::atomic::AtomicUsize,
        stale_open: bool,
        open_calls: std::sync::atomic::AtomicUsize,
    }
    impl SnapshotSource<Model> for CountingSource {
        fn release_request(&self, id: Uuid) -> BoxFuture<'_, ()> {
            self.inner.release_request(id)
        }
        fn close_snapshot(&self, id: Uuid) -> BoxFuture<'_, ()> {
            self.inner.close_snapshot(id)
        }
        fn cancel_snapshot_build(&self, c: WorkCursor) -> BoxFuture<'_, ()> {
            self.inner.cancel_snapshot_build(c)
        }
        fn open_snapshot(
            &self,
            id: Uuid,
            scope: Scope<Model>,
            limits: WorkLimits,
            resume: Option<WorkCursor>,
        ) -> BoxFuture<'_, WorkOutcome<SnapshotDescriptor<Model>>> {
            self.open_calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if self.stale_open {
                return Box::pin(async { Err(SyncError::StaleWork) });
            }
            self.inner.open_snapshot(id, scope, limits, resume)
        }
        fn summarize(
            &self,
            id: Uuid,
            range: KeyRange<Model>,
            limits: WorkLimits,
            resume: Option<WorkCursor>,
        ) -> BoxFuture<'_, WorkOutcome<RangeSummary<Model>>> {
            self.inner.summarize(id, range, limits, resume)
        }
        fn split(
            &self,
            id: Uuid,
            range: KeyRange<Model>,
            limits: WorkLimits,
            resume: Option<WorkCursor>,
        ) -> BoxFuture<'_, WorkOutcome<Vec<KeyRange<Model>>>> {
            self.inner.split(id, range, limits, resume)
        }
        fn leaf(
            &self,
            id: Uuid,
            range: KeyRange<Model>,
            after: Option<CandidateCursor<Model>>,
            limits: PageLimits,
        ) -> BoxFuture<'_, LeafPage<Model>> {
            Box::pin(async move {
                let page = self.inner.leaf(id, range, after, limits).await?;
                self.calls
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.candidates
                    .fetch_add(page.candidates.len(), std::sync::atomic::Ordering::Relaxed);
                Ok(page)
            })
        }
        fn read_fragment(
            &self,
            id: Uuid,
            manifest: ManifestRef,
            ordinal: u32,
            max: u64,
        ) -> BoxFuture<'_, Fragment> {
            SnapshotSource::read_fragment(&*self.inner, id, manifest, ordinal, max)
        }
    }
    #[tokio::test]
    async fn one_write_snapshot_transfers_only_changed_leaf() {
        use super::super::coordinator::{Coordinator, SessionAdvance, SessionLimits};
        use std::sync::atomic::Ordering;
        let a = Arc::new(store(2));
        let b = Arc::new(store(3));
        let c = Arc::new(store(4));
        for n in 1..=1024 {
            a.write_local(
                ObjectKey {
                    collection: Collection::Notes,
                    id: Uuid::from_u128(n),
                },
                Some(Value { n: n as u32 }),
            )
            .await
            .unwrap();
        }
        let limits = SessionLimits {
            pages: PageLimits {
                max_scanned: 100,
                max_bytes: 1024 * 1024,
            },
            stages: StageLimits {
                max_total_bytes: DEFAULT_STAGE_BYTES,
                max_stages: DEFAULT_STAGES,
                max_event_bytes: DEFAULT_LOGICAL_BYTES,
                max_event_items: DEFAULT_LOGICAL_ITEMS,
                max_fragment_bytes: DEFAULT_PAGE_BYTES,
            },
            apply: ApplyLimits {
                max_projection_items: 128,
                max_projection_bytes: DEFAULT_PROJECTION_BYTES,
                max_mutations: 1000,
            },
            work: WorkLimits {
                max_requests: 1000,
                max_bytes: DEFAULT_ROUND_BYTES,
                max_depth: 10,
                max_rows_scanned: 1000,
                max_mutations: 1000,
            },
        };
        async fn run(
            dst: &Arc<MemorySyncStore<Model>>,
            src: &Arc<MemorySyncStore<Model>>,
            source: &dyn SnapshotSource<Model>,
            limits: SessionLimits,
        ) {
            let peer = StorePeer(src.clone());
            let mut coordinator = Coordinator::new(
                Scope {
                    collections: [Collection::Notes].into_iter().collect(),
                },
                limits,
            )
            .unwrap();
            for _ in 0..3000 {
                coordinator.start_pass();
                if matches!(
                    coordinator
                        .advance(&**dst, &**dst, &**dst, &peer, source)
                        .await
                        .unwrap(),
                    SessionAdvance::Complete(_)
                ) {
                    return;
                }
            }
            panic!("session did not complete");
        }
        run(&c, &a, &*a, limits).await;
        run(&b, &c, &*c, limits).await;
        a.write_local(
            ObjectKey {
                collection: Collection::Notes,
                id: Uuid::from_u128(500),
            },
            Some(Value { n: 9999 }),
        )
        .await
        .unwrap();
        let head = b.head().await.unwrap().sequence;
        let counting = CountingSource::new(a.clone());
        run(&b, &a, &counting, limits).await;
        assert_eq!(
            b.get(&ObjectKey {
                collection: Collection::Notes,
                id: Uuid::from_u128(500)
            })
            .await
            .unwrap()
            .unwrap()
            .value,
            Some(Value { n: 9999 })
        );
        let calls = counting.calls.load(Ordering::Relaxed);
        let candidates = counting.candidates.load(Ordering::Relaxed);
        eprintln!(
            "leaf calls: {calls}, leaf candidates: {candidates}, head growth: {}",
            b.head().await.unwrap().sequence - head
        );
        assert!(
            calls <= 4 && candidates <= 256,
            "leaf calls: {calls}, leaf candidates: {candidates}"
        );
        assert!(b.head().await.unwrap().sequence <= head + 2);
    }
    #[tokio::test]
    async fn coordinator_subdivides_large_differing_root() {
        use super::super::coordinator::{Coordinator, SessionAdvance, SessionLimits};
        let a = std::sync::Arc::new(store(2));
        let b = std::sync::Arc::new(store(3));
        for n in 1..=130 {
            a.write_local(
                ObjectKey {
                    collection: Collection::Notes,
                    id: Uuid::from_u128(n),
                },
                Some(Value { n: n as u32 }),
            )
            .await
            .unwrap();
        }
        let peer = StorePeer(a.clone());
        let limits = SessionLimits {
            pages: PageLimits {
                max_scanned: 100,
                max_bytes: 1024 * 1024,
            },
            stages: StageLimits {
                max_total_bytes: DEFAULT_STAGE_BYTES,
                max_stages: DEFAULT_STAGES,
                max_event_bytes: DEFAULT_LOGICAL_BYTES,
                max_event_items: DEFAULT_LOGICAL_ITEMS,
                max_fragment_bytes: DEFAULT_PAGE_BYTES,
            },
            apply: ApplyLimits {
                max_projection_items: 128,
                max_projection_bytes: DEFAULT_PROJECTION_BYTES,
                max_mutations: 1000,
            },
            work: WorkLimits {
                max_requests: 1000,
                max_bytes: DEFAULT_ROUND_BYTES,
                max_depth: 10,
                max_rows_scanned: 1000,
                max_mutations: 1000,
            },
        };
        let mut coordinator = Coordinator::new(
            Scope {
                collections: [Collection::Notes].into_iter().collect(),
            },
            limits,
        )
        .unwrap();
        let mut complete = false;
        for _ in 0..300 {
            coordinator.start_pass();
            if matches!(
                coordinator
                    .advance(&*b, &*b, &*b, &peer, &*a)
                    .await
                    .unwrap(),
                SessionAdvance::Complete(_)
            ) {
                complete = true;
                break;
            }
        }
        assert!(complete);
        assert_eq!(
            b.get(&ObjectKey {
                collection: Collection::Notes,
                id: Uuid::from_u128(130)
            })
            .await
            .unwrap()
            .unwrap()
            .value,
            Some(Value { n: 130 })
        );
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct BigModel;
    impl SyncModel for BigModel {
        type Collection = Collection;
        type Payload = String;
        fn model_id() -> &'static str {
            "big"
        }
        fn schema_id() -> &'static str {
            "one"
        }
        fn collections() -> Vec<Collection> {
            vec![Collection::Notes]
        }
        fn collection_tag(_: &Collection) -> &'static str {
            "notes"
        }
        fn collection_from_tag(s: &str) -> Option<Collection> {
            (s == "notes").then_some(Collection::Notes)
        }
        fn validate(_: &ObjectKey<Self>, _: &Option<String>) -> Result<()> {
            Ok(())
        }
        fn validate_revision(_: &ObjectKey<Self>, _: &Option<String>, _: &Revision) -> Result<()> {
            Ok(())
        }
    }
    fn big_store(n: u128) -> MemorySyncStore<BigModel> {
        MemorySyncStore::new(
            StoreIdentity {
                namespace: Uuid::from_u128(1),
                model: ModelId {
                    model: "big".into(),
                    schema: "one".into(),
                },
                replica: Uuid::from_u128(n),
                generation: Uuid::from_u128(n + 10),
            },
            LocalAuthor {
                author: Uuid::from_u128(n),
                writer_incarnation: Uuid::from_u128(n + 20),
            },
        )
        .unwrap()
    }
    #[tokio::test]
    async fn coordinator_receives_fragmented_journal_event() {
        use super::super::coordinator::{Coordinator, SessionAdvance, SessionLimits};
        let a = std::sync::Arc::new(big_store(2));
        let b = std::sync::Arc::new(big_store(3));
        let peer = StorePeer(a.clone());
        let scope = Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let source = ReceiveStore::identity(&*a).await.unwrap();
        let start = ReceiveProgress {
            sequence: 0,
            anchor: initial_anchor(&source).unwrap(),
        };
        let checkpoint = CheckpointKey {
            namespace: source.namespace,
            source_replica: source.replica,
            source_generation: source.generation,
            scope_id: scope_id(&scope).unwrap(),
        };
        set_checkpoint(&mut b.lock().unwrap().checkpoints, checkpoint, start);
        let key = ObjectKey {
            collection: Collection::Notes,
            id: Uuid::from_u128(42),
        };
        let value = "x".repeat(6000);
        a.write_local(key.clone(), Some(value.clone()))
            .await
            .unwrap();
        let limits = SessionLimits {
            pages: PageLimits {
                max_scanned: 1,
                max_bytes: 3000,
            },
            stages: StageLimits {
                max_total_bytes: DEFAULT_STAGE_BYTES,
                max_stages: DEFAULT_STAGES,
                max_event_bytes: DEFAULT_LOGICAL_BYTES,
                max_event_items: DEFAULT_LOGICAL_ITEMS,
                max_fragment_bytes: DEFAULT_PAGE_BYTES,
            },
            apply: ApplyLimits {
                max_projection_items: 10,
                max_projection_bytes: 65536,
                max_mutations: 100,
            },
            work: WorkLimits {
                max_requests: 100,
                max_bytes: DEFAULT_ROUND_BYTES,
                max_depth: 10,
                max_rows_scanned: 100,
                max_mutations: 100,
            },
        };
        let mut coordinator = Coordinator::new(scope, limits).unwrap();
        let mut complete = false;
        for _ in 0..100 {
            coordinator.start_pass();
            if matches!(
                coordinator
                    .advance(&*b, &*b, &*b, &peer, &*a)
                    .await
                    .unwrap(),
                SessionAdvance::Complete(_)
            ) {
                complete = true;
                break;
            }
        }
        assert!(complete);
        assert_eq!(b.get(&key).await.unwrap().unwrap().value, Some(value));
    }
    #[test]
    fn default_fragment_layout_avoids_excessive_request_count() {
        let bytes = vec![7u8; 1024 * 1024];
        let manifest = manifest_for(ManifestKind::Context, &bytes, 1).unwrap();
        assert_eq!(manifest.fragment_count, 16);
        let (start, end) = memory_fragment_bounds(&bytes, &manifest, 0).unwrap();
        assert_eq!(end - start, 64 * 1024);
        assert_eq!((MAX_LOGICAL_BYTES as usize).div_ceil(FRAGMENT_BYTES), 1024);
    }
    #[tokio::test]
    async fn source_snapshot_build_resumes_under_one_row_budget() {
        let source = store(2);
        let scope = Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        for n in 1..=4 {
            source
                .write_local(
                    ObjectKey {
                        collection: Collection::Notes,
                        id: Uuid::from_u128(n),
                    },
                    Some(Value { n: n as u32 }),
                )
                .await
                .unwrap();
        }
        let work = WorkLimits {
            max_requests: 1,
            max_bytes: 65536,
            max_depth: 10,
            max_rows_scanned: 1,
            max_mutations: 10,
        };
        let WorkOutcome::Incomplete { cursor } = source
            .open_snapshot(Uuid::from_u128(800), scope.clone(), work, None)
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            source
                .open_snapshot(Uuid::from_u128(800), scope.clone(), work, None)
                .await
                .unwrap(),
            WorkOutcome::Incomplete {
                cursor: cursor.clone()
            }
        );
        assert!(matches!(
            source
                .open_snapshot(
                    Uuid::from_u128(800),
                    scope.clone(),
                    WorkLimits {
                        max_rows_scanned: 2,
                        ..work
                    },
                    None
                )
                .await,
            Err(SyncError::Invalid(_))
        ));
        let stale = cursor.clone();
        source
            .write_local(
                ObjectKey {
                    collection: Collection::Notes,
                    id: Uuid::from_u128(99),
                },
                Some(Value { n: 99 }),
            )
            .await
            .unwrap();
        let first = source
            .open_snapshot(
                Uuid::from_u128(800),
                scope.clone(),
                work,
                Some(cursor.clone()),
            )
            .await
            .unwrap();
        let retry = source
            .open_snapshot(Uuid::from_u128(800), scope.clone(), work, Some(cursor))
            .await
            .unwrap();
        assert_eq!(first, retry);
        let WorkOutcome::Incomplete { cursor } = first else {
            panic!()
        };
        let mut next = Some(cursor);
        let mut result = None;
        let mut final_input = None;
        for _ in 0..100 {
            final_input = next.clone();
            match source
                .open_snapshot(Uuid::from_u128(800), scope.clone(), work, next.take())
                .await
                .unwrap()
            {
                WorkOutcome::Incomplete { cursor } => next = Some(cursor),
                WorkOutcome::Ready(snapshot) => {
                    result = Some(snapshot);
                    break;
                }
            }
        }
        let snapshot = result.unwrap();
        assert_eq!(
            source
                .open_snapshot(Uuid::from_u128(800), scope.clone(), work, None)
                .await
                .unwrap(),
            WorkOutcome::Ready(snapshot.clone())
        );
        assert_eq!(
            source
                .open_snapshot(Uuid::from_u128(800), scope.clone(), work, final_input)
                .await
                .unwrap(),
            WorkOutcome::Ready(snapshot.clone())
        );
        assert_eq!(snapshot.roots[0].candidate_count, 4);
        assert!(matches!(
            source
                .open_snapshot(
                    Uuid::from_u128(800),
                    scope.clone(),
                    work,
                    Some(stale.clone())
                )
                .await,
            Err(SyncError::StaleWork)
        ));
        let leaf = source
            .leaf(
                snapshot.snapshot_id,
                snapshot.roots[0].range.clone(),
                None,
                PageLimits {
                    max_scanned: 10,
                    max_bytes: 65536,
                },
            )
            .await
            .unwrap();
        assert_eq!(leaf.candidates.len(), 4);
        source.close_snapshot(snapshot.snapshot_id).await.unwrap();
        assert!(matches!(
            source
                .open_snapshot(Uuid::from_u128(800), scope.clone(), work, None)
                .await,
            Err(SyncError::SnapshotExpired)
        ));
        *source
            .lock()
            .unwrap()
            .retired_requests
            .get_mut(&Uuid::from_u128(800))
            .unwrap() = 0;
        assert!(matches!(
            source
                .open_snapshot(Uuid::from_u128(800), scope.clone(), work, None)
                .await
                .unwrap(),
            WorkOutcome::Incomplete { .. }
        ));
        assert!(matches!(
            source
                .open_snapshot(Uuid::from_u128(800), scope, work, Some(stale))
                .await,
            Err(SyncError::StaleWork)
        ));
    }
    #[tokio::test]
    async fn range_split_resumes_with_one_row_budget() {
        let source = store(2);
        let scope = Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        for n in 1..=8 {
            source
                .write_local(
                    ObjectKey {
                        collection: Collection::Notes,
                        id: Uuid::from_u128(n),
                    },
                    Some(Value { n: n as u32 }),
                )
                .await
                .unwrap();
        }
        let wide = WorkLimits {
            max_requests: 10,
            max_bytes: 65536,
            max_depth: 10,
            max_rows_scanned: 100,
            max_mutations: 100,
        };
        let WorkOutcome::Ready(snapshot) = source
            .open_snapshot(Uuid::from_u128(800), scope, wide, None)
            .await
            .unwrap()
        else {
            panic!()
        };
        let work = WorkLimits {
            max_rows_scanned: 1,
            ..wide
        };
        let range = snapshot.roots[0].range.clone();
        let mut next = None;
        let mut children = None;
        for _ in 0..100 {
            match source
                .split(snapshot.snapshot_id, range.clone(), work, next.take())
                .await
                .unwrap()
            {
                WorkOutcome::Incomplete { cursor } => next = Some(cursor),
                WorkOutcome::Ready(result) => {
                    children = Some(result);
                    break;
                }
            }
        }
        let children = children.unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].lower, range.lower);
        assert_eq!(children[0].upper, children[1].lower);
        assert_eq!(children[1].upper, range.upper);
    }
    #[tokio::test]
    async fn fragmented_commit_retry_returns_committed_progress() {
        let source = big_store(2);
        let receiver = big_store(3);
        let scope = Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let identity = ReceiveStore::identity(&source).await.unwrap();
        let initial = ReceiveProgress {
            sequence: 0,
            anchor: initial_anchor(&identity).unwrap(),
        };
        let ck = CheckpointKey {
            namespace: identity.namespace,
            source_replica: identity.replica,
            source_generation: identity.generation,
            scope_id: scope_id(&scope).unwrap(),
        };
        set_checkpoint(
            &mut receiver.lock().unwrap().checkpoints,
            ck.clone(),
            initial.clone(),
        );
        source
            .write_local(
                ObjectKey {
                    collection: Collection::Notes,
                    id: Uuid::from_u128(42),
                },
                Some("x".repeat(6000)),
            )
            .await
            .unwrap();
        let request = PageRequest {
            source: identity,
            scope,
            after: initial,
            through: source.head().await.unwrap(),
            limits: PageLimits {
                max_scanned: 1,
                max_bytes: 3000,
            },
        };
        let PageRead::FragmentNeeded {
            sequence,
            collection,
            event_digest,
            manifest,
            ..
        } = source.read_page(request.clone()).await.unwrap()
        else {
            panic!()
        };
        let limits = StageLimits {
            max_total_bytes: DEFAULT_STAGE_BYTES,
            max_stages: DEFAULT_STAGES,
            max_event_bytes: DEFAULT_LOGICAL_BYTES,
            max_event_items: DEFAULT_LOGICAL_ITEMS,
            max_fragment_bytes: DEFAULT_PAGE_BYTES,
        };
        let stage = receiver
            .begin_stage(
                StageDescriptor {
                    origin: StageOrigin::Journal {
                        request,
                        sequence,
                        collection,
                        event_digest,
                    },
                    manifest: manifest.clone(),
                },
                limits,
            )
            .await
            .unwrap();
        for ordinal in 0..manifest.fragment_count {
            let fragment = SyncStore::read_fragment(&source, manifest.clone(), ordinal, 3000)
                .await
                .unwrap();
            receiver
                .stage_fragment(stage.stage, fragment)
                .await
                .unwrap();
        }
        let apply = ApplyLimits {
            max_projection_items: 10,
            max_projection_bytes: 65536,
            max_mutations: 100,
        };
        let prepared = receiver
            .lock()
            .unwrap()
            .receiver
            .prepare_event(stage.stage)
            .unwrap();
        assert!(matches!(prepared.event, Event::Key(_)));
        assert!(
            receiver
                .lock()
                .unwrap()
                .receiver
                .completed_event(stage.stage, &ck, sequence)
                .unwrap()
                .is_none()
        );
        let first = receiver
            .commit_fragmented_event(ck.clone(), sequence, stage.stage, apply)
            .await
            .unwrap();
        let retry = receiver
            .commit_fragmented_event(ck, sequence, stage.stage, apply)
            .await
            .unwrap();
        assert_eq!(first.progress, retry.progress);
        assert!(retry.projection.changes.is_empty());
    }
    #[tokio::test]
    async fn coordinator_splits_two_keys_for_single_projection_limit() {
        use super::super::coordinator::{Coordinator, SessionAdvance, SessionLimits};
        let a = std::sync::Arc::new(store(2));
        let b = std::sync::Arc::new(store(3));
        for n in 1..=2 {
            a.write_local(
                ObjectKey {
                    collection: Collection::Notes,
                    id: Uuid::from_u128(n),
                },
                Some(Value { n: n as u32 }),
            )
            .await
            .unwrap();
        }
        let peer = StorePeer(a.clone());
        let limits = SessionLimits {
            pages: PageLimits {
                max_scanned: 10,
                max_bytes: 65536,
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
                max_projection_bytes: 65536,
                max_mutations: 100,
            },
            work: WorkLimits {
                max_requests: 100,
                max_bytes: DEFAULT_ROUND_BYTES,
                max_depth: 10,
                max_rows_scanned: 100,
                max_mutations: 100,
            },
        };
        let mut coordinator = Coordinator::new(
            Scope {
                collections: [Collection::Notes].into_iter().collect(),
            },
            limits,
        )
        .unwrap();
        let mut done = false;
        for _ in 0..200 {
            coordinator.start_pass();
            if matches!(
                coordinator
                    .advance(&*b, &*b, &*b, &peer, &*a)
                    .await
                    .unwrap(),
                SessionAdvance::Complete(_)
            ) {
                done = true;
                break;
            }
        }
        assert!(done);
        for n in 1..=2 {
            assert_eq!(
                b.get(&ObjectKey {
                    collection: Collection::Notes,
                    id: Uuid::from_u128(n)
                })
                .await
                .unwrap()
                .unwrap()
                .value,
                Some(Value { n: n as u32 })
            );
        }
    }
    #[tokio::test]
    async fn lost_initial_open_replies_can_be_released_by_request_id() {
        let source = store(2);
        let scope = Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let work = WorkLimits {
            max_requests: 10,
            max_bytes: 65536,
            max_depth: 10,
            max_rows_scanned: 10,
            max_mutations: 10,
        };
        for n in 1..=4 {
            let id = Uuid::from_u128(1000 + n);
            source
                .open_snapshot(id, scope.clone(), work, None)
                .await
                .unwrap();
            source.release_request(id).await.unwrap();
            source.release_request(id).await.unwrap();
        }
        assert!(matches!(
            source
                .open_snapshot(Uuid::from_u128(2000), scope, work, None)
                .await
                .unwrap(),
            WorkOutcome::Ready(_)
        ));
    }
    #[tokio::test]
    async fn coordinator_yields_and_resumes_at_pass_limits() {
        use super::super::coordinator::{Coordinator, SessionAdvance, SessionLimits};
        let a = std::sync::Arc::new(store(2));
        let b = std::sync::Arc::new(store(3));
        let key = ObjectKey {
            collection: Collection::Notes,
            id: Uuid::from_u128(33),
        };
        a.write_local(key.clone(), Some(Value { n: 33 }))
            .await
            .unwrap();
        let peer = StorePeer(a.clone());
        let limits = SessionLimits {
            pages: PageLimits {
                max_scanned: 10,
                max_bytes: 32768,
            },
            stages: StageLimits {
                max_total_bytes: DEFAULT_STAGE_BYTES,
                max_stages: DEFAULT_STAGES,
                max_event_bytes: DEFAULT_LOGICAL_BYTES,
                max_event_items: DEFAULT_LOGICAL_ITEMS,
                max_fragment_bytes: DEFAULT_PAGE_BYTES,
            },
            apply: ApplyLimits {
                max_projection_items: 10,
                max_projection_bytes: 8192,
                max_mutations: 10,
            },
            work: WorkLimits {
                max_requests: 1,
                max_bytes: 65536,
                max_depth: 10,
                max_rows_scanned: 10,
                max_mutations: 10,
            },
        };
        let mut coordinator = Coordinator::new(
            Scope {
                collections: [Collection::Notes].into_iter().collect(),
            },
            limits,
        )
        .unwrap();
        let mut yields = 0;
        let mut complete = false;
        for _ in 0..300 {
            match coordinator
                .advance(&*b, &*b, &*b, &peer, &*a)
                .await
                .unwrap()
            {
                SessionAdvance::BudgetExhausted => {
                    let usage = coordinator.pass_usage();
                    assert!(usage.requests <= 1 && usage.encoded_bytes <= 65536);
                    yields += 1;
                    coordinator.start_pass();
                }
                SessionAdvance::Complete(_) => {
                    complete = true;
                    break;
                }
                SessionAdvance::Incomplete => (),
                SessionAdvance::Cancelled => panic!(),
            }
        }
        assert!(complete && yields > 1);
        assert_eq!(
            b.get(&key).await.unwrap().unwrap().value,
            Some(Value { n: 33 })
        );
    }
    #[tokio::test]
    async fn coverage_charges_empty_response_envelope() {
        let a = store(2);
        let b = store(3);
        let scope = Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let work = WorkLimits {
            max_requests: 10,
            max_bytes: 65536,
            max_depth: 10,
            max_rows_scanned: 10,
            max_mutations: 10,
        };
        let WorkOutcome::Ready(remote) = a
            .open_snapshot(Uuid::from_u128(51), scope.clone(), work, None)
            .await
            .unwrap()
        else {
            panic!()
        };
        let WorkOutcome::Ready(local) = b
            .open_snapshot(Uuid::from_u128(52), scope, work, None)
            .await
            .unwrap()
        else {
            panic!()
        };
        let session = b
            .begin_reconciliation(Uuid::from_u128(900), remote, local)
            .await
            .unwrap();
        assert!(
            matches!(b.coverage(session,None,1,PageLimits {max_scanned:1,max_bytes:1}).await,Err(SyncError::LimitTooSmall {required}) if required>1)
        );
    }
    #[tokio::test]
    async fn coverage_churn_is_rejected_before_metadata_growth() {
        let a = store(2);
        let b = store(3);
        let scope = Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let work = WorkLimits {
            max_requests: 10,
            max_bytes: 65536,
            max_depth: 10,
            max_rows_scanned: 10,
            max_mutations: 10,
        };
        let WorkOutcome::Ready(remote) = a
            .open_snapshot(Uuid::from_u128(71), scope.clone(), work, None)
            .await
            .unwrap()
        else {
            panic!()
        };
        let WorkOutcome::Ready(local) = b
            .open_snapshot(Uuid::from_u128(72), scope, work, None)
            .await
            .unwrap()
        else {
            panic!()
        };
        let session = b
            .begin_reconciliation(Uuid::from_u128(900), remote.clone(), local)
            .await
            .unwrap();
        let mut state = b.lock().unwrap();
        let active = state.sessions.iter_mut().find(|s| s.id == session).unwrap();
        active.completed = vec![remote.roots[0].clone(); MAX_RECEIVER_METADATA_ITEMS - 1];
        let root = remote.roots[0].range.clone();
        let mut left = remote.roots[0].clone();
        left.range.upper = Some(Uuid::from_u128(10));
        let mut right = remote.roots[0].clone();
        right.range.lower = Some(Uuid::from_u128(10));
        assert!(matches!(
            state.receiver.split_range(session, root, vec![left, right]),
            Err(SyncError::StageQuota)
        ));
        assert_eq!(
            state.receiver.progress(session).unwrap().outstanding_ranges,
            1
        );
    }
    #[tokio::test]
    async fn completed_reconciliation_receipts_are_bounded() {
        let a = store(2);
        let b = store(3);
        let scope = Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let work = WorkLimits {
            max_requests: 10,
            max_bytes: 65536,
            max_depth: 10,
            max_rows_scanned: 10,
            max_mutations: 10,
        };
        let WorkOutcome::Ready(remote) = a
            .open_snapshot(Uuid::from_u128(81), scope.clone(), work, None)
            .await
            .unwrap()
        else {
            panic!()
        };
        let WorkOutcome::Ready(local) = b
            .open_snapshot(Uuid::from_u128(82), scope, work, None)
            .await
            .unwrap()
        else {
            panic!()
        };
        let mut ledger = ReceiverLedger::<Model>::new();
        for n in 1..=(MAX_RECEIVER_RETRY_RECEIPTS + 5) {
            let id = Uuid::from_u128(1000 + n as u128);
            ledger
                .begin_reconciliation(id, remote.clone(), local.clone(), 0)
                .unwrap();
            ledger
                .accept_equal_range(id, remote.roots[0].clone(), &local.roots[0])
                .unwrap();
            let context = remote.contexts[0].clone();
            let stage_id = Uuid::new_v4();
            let stage = ledger
                .begin_stage(
                    stage_id,
                    StageDescriptor {
                        origin: StageOrigin::SnapshotContext {
                            session: id,
                            snapshot_id: remote.snapshot_id,
                            collection: context.collection.clone(),
                        },
                        manifest: context.context.manifest.clone(),
                    },
                    session_limits().stages,
                    u64::MAX,
                )
                .unwrap();
            let fragment = SnapshotSource::read_fragment(
                &a,
                remote.snapshot_id,
                context.context.manifest.clone(),
                0,
                DEFAULT_PAGE_BYTES,
            )
            .await
            .unwrap();
            ledger.stage_fragment(stage.stage, fragment).unwrap();
            while let Some(token) = ledger.prepare_promotion(id, 100).unwrap() {
                ledger.finalize_promotion(&token).unwrap();
            }
            let token = ledger.prepare_completion(id).unwrap();
            ledger
                .finalize_completion(&token, remote.head.clone())
                .unwrap();
        }
        assert_eq!(ledger.completed_sessions.len(), MAX_RECEIVER_RETRY_RECEIPTS);
        assert!(ledger.completed_session(Uuid::from_u128(1001)).is_none());
    }
    #[tokio::test]
    async fn coordinator_abort_releases_opened_source_and_preserves_state() {
        use super::super::coordinator::{Coordinator, SessionAdvance, SessionLimits};
        let a = std::sync::Arc::new(store(2));
        let b = std::sync::Arc::new(store(3));
        let peer = StorePeer(a.clone());
        let limits = SessionLimits {
            pages: PageLimits {
                max_scanned: 10,
                max_bytes: 32768,
            },
            stages: StageLimits {
                max_total_bytes: DEFAULT_STAGE_BYTES,
                max_stages: DEFAULT_STAGES,
                max_event_bytes: DEFAULT_LOGICAL_BYTES,
                max_event_items: DEFAULT_LOGICAL_ITEMS,
                max_fragment_bytes: DEFAULT_PAGE_BYTES,
            },
            apply: ApplyLimits {
                max_projection_items: 10,
                max_projection_bytes: 8192,
                max_mutations: 10,
            },
            work: WorkLimits {
                max_requests: 20,
                max_bytes: 65536,
                max_depth: 10,
                max_rows_scanned: 10,
                max_mutations: 10,
            },
        };
        let mut coordinator = Coordinator::new(
            Scope {
                collections: [Collection::Notes].into_iter().collect(),
            },
            limits,
        )
        .unwrap();
        for _ in 0..5 {
            coordinator.start_pass();
            coordinator
                .advance(&*b, &*b, &*b, &peer, &*a)
                .await
                .unwrap();
        }
        assert_eq!(a.lock().unwrap().snapshots.len(), 1);
        coordinator.start_abort();
        let mut cancelled = false;
        for _ in 0..10 {
            coordinator.start_pass();
            if matches!(
                coordinator
                    .advance(&*b, &*b, &*b, &peer, &*a)
                    .await
                    .unwrap(),
                SessionAdvance::Cancelled
            ) {
                cancelled = true;
                break;
            }
        }
        assert!(cancelled);
        assert!(a.lock().unwrap().snapshots.is_empty());
        assert!(b.lock().unwrap().checkpoints.is_empty());
    }
    #[tokio::test]
    async fn concurrent_candidates_at_one_key_use_one_projection_item() {
        use super::super::coordinator::{Coordinator, SessionAdvance, SessionLimits};
        let a = std::sync::Arc::new(store(2));
        let b = std::sync::Arc::new(store(3));
        let c = store(4);
        let key = ObjectKey {
            collection: Collection::Notes,
            id: Uuid::from_u128(7),
        };
        a.write_local(key.clone(), Some(Value { n: 1 }))
            .await
            .unwrap();
        c.write_local(key.clone(), Some(Value { n: 2 }))
            .await
            .unwrap();
        let Event::Key(delta) = c.lock().unwrap().core.journal()[0].event.clone() else {
            panic!()
        };
        a.lock().unwrap().core.apply_delta(&delta).unwrap();
        let peer = StorePeer(a.clone());
        let limits = SessionLimits {
            pages: PageLimits {
                max_scanned: 10,
                max_bytes: 32768,
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
                max_projection_bytes: 8192,
                max_mutations: 100,
            },
            work: WorkLimits {
                max_requests: 100,
                max_bytes: 65536,
                max_depth: 10,
                max_rows_scanned: 100,
                max_mutations: 100,
            },
        };
        let mut coordinator = Coordinator::new(
            Scope {
                collections: [Collection::Notes].into_iter().collect(),
            },
            limits,
        )
        .unwrap();
        let mut done = false;
        for _ in 0..200 {
            coordinator.start_pass();
            if matches!(
                coordinator
                    .advance(&*b, &*b, &*b, &peer, &*a)
                    .await
                    .unwrap(),
                SessionAdvance::Complete(_)
            ) {
                done = true;
                break;
            }
        }
        assert!(done);
        assert_eq!(
            b.candidates(
                &key,
                None,
                PageLimits {
                    max_scanned: 10,
                    max_bytes: 32768
                }
            )
            .await
            .unwrap()
            .values
            .len(),
            2
        );
    }
    #[tokio::test]
    async fn journal_work_limit_falls_back_to_complete_comparison() {
        use super::super::coordinator::{Coordinator, SessionAdvance, SessionLimits};
        let a = std::sync::Arc::new(store(2));
        let b = std::sync::Arc::new(store(3));
        let scope = Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let id = ReceiveStore::identity(&*a).await.unwrap();
        let initial = ReceiveProgress {
            sequence: 0,
            anchor: initial_anchor(&id).unwrap(),
        };
        let ck = CheckpointKey {
            namespace: id.namespace,
            source_replica: id.replica,
            source_generation: id.generation,
            scope_id: scope_id(&scope).unwrap(),
        };
        set_checkpoint(&mut b.lock().unwrap().checkpoints, ck, initial);
        let key = ObjectKey {
            collection: Collection::Notes,
            id: Uuid::from_u128(44),
        };
        a.write_local(key.clone(), Some(Value { n: 44 }))
            .await
            .unwrap();
        b.lock().unwrap().fail_next_apply = Some(SyncError::WorkLimit);
        let peer = StorePeer(a.clone());
        let limits = SessionLimits {
            pages: PageLimits {
                max_scanned: 10,
                max_bytes: 32768,
            },
            stages: StageLimits {
                max_total_bytes: DEFAULT_STAGE_BYTES,
                max_stages: DEFAULT_STAGES,
                max_event_bytes: DEFAULT_LOGICAL_BYTES,
                max_event_items: DEFAULT_LOGICAL_ITEMS,
                max_fragment_bytes: DEFAULT_PAGE_BYTES,
            },
            apply: ApplyLimits {
                max_projection_items: 10,
                max_projection_bytes: 8192,
                max_mutations: 10,
            },
            work: WorkLimits {
                max_requests: 100,
                max_bytes: 65536,
                max_depth: 10,
                max_rows_scanned: 100,
                max_mutations: 100,
            },
        };
        let mut coordinator = Coordinator::new(scope, limits).unwrap();
        let mut done = false;
        for _ in 0..200 {
            coordinator.start_pass();
            if matches!(
                coordinator
                    .advance(&*b, &*b, &*b, &peer, &*a)
                    .await
                    .unwrap(),
                SessionAdvance::Complete(_)
            ) {
                done = true;
                break;
            }
        }
        assert!(done);
        assert_eq!(
            b.get(&key).await.unwrap().unwrap().value,
            Some(Value { n: 44 })
        );
    }
    #[tokio::test]
    async fn coordinator_preserves_storage_errors_without_reconciliation_fallback() {
        use super::super::coordinator::{Coordinator, SessionAdvance, SessionLimits};
        let a = std::sync::Arc::new(store(2));
        let b = std::sync::Arc::new(store(3));
        let scope = Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let id = ReceiveStore::identity(&*a).await.unwrap();
        let initial = ReceiveProgress {
            sequence: 0,
            anchor: initial_anchor(&id).unwrap(),
        };
        let ck = CheckpointKey {
            namespace: id.namespace,
            source_replica: id.replica,
            source_generation: id.generation,
            scope_id: scope_id(&scope).unwrap(),
        };
        set_checkpoint(
            &mut b.lock().unwrap().checkpoints,
            ck.clone(),
            initial.clone(),
        );
        a.write_local(
            ObjectKey {
                collection: Collection::Notes,
                id: Uuid::from_u128(44),
            },
            Some(Value { n: 44 }),
        )
        .await
        .unwrap();
        b.lock().unwrap().fail_next_apply = Some(SyncError::Storage("deliberate fault".into()));
        let peer = StorePeer(a.clone());
        let limits = SessionLimits {
            pages: PageLimits {
                max_scanned: 10,
                max_bytes: 32768,
            },
            stages: StageLimits {
                max_total_bytes: DEFAULT_STAGE_BYTES,
                max_stages: DEFAULT_STAGES,
                max_event_bytes: DEFAULT_LOGICAL_BYTES,
                max_event_items: DEFAULT_LOGICAL_ITEMS,
                max_fragment_bytes: DEFAULT_PAGE_BYTES,
            },
            apply: ApplyLimits {
                max_projection_items: 10,
                max_projection_bytes: 8192,
                max_mutations: 10,
            },
            work: WorkLimits {
                max_requests: 100,
                max_bytes: 65536,
                max_depth: 10,
                max_rows_scanned: 100,
                max_mutations: 100,
            },
        };
        let mut coordinator = Coordinator::new(scope, limits).unwrap();
        let mut seen = false;
        for _ in 0..30 {
            coordinator.start_pass();
            match coordinator.advance(&*b, &*b, &*b, &peer, &*a).await {
                Err(SyncError::Storage(message)) => {
                    assert_eq!(message, "deliberate fault");
                    seen = true;
                    break;
                }
                Ok(SessionAdvance::Complete(_)) => panic!(),
                Ok(_) => (),
                Err(e) => panic!("{e:?}"),
            }
        }
        assert!(seen);
        assert_eq!(coordinator.pass_usage().requests, 1);
        assert!(coordinator.pass_usage().encoded_bytes > 0);
        assert_eq!(b.checkpoint(&ck).await.unwrap(), Some(initial));
        assert!(a.lock().unwrap().snapshots.is_empty());
    }
    fn session_limits() -> SessionLimits {
        SessionLimits {
            pages: PageLimits {
                max_scanned: 100,
                max_bytes: DEFAULT_PAGE_BYTES,
            },
            stages: StageLimits {
                max_total_bytes: DEFAULT_STAGE_BYTES,
                max_stages: DEFAULT_STAGES,
                max_event_bytes: DEFAULT_LOGICAL_BYTES,
                max_event_items: DEFAULT_LOGICAL_ITEMS,
                max_fragment_bytes: DEFAULT_PAGE_BYTES,
            },
            apply: ApplyLimits {
                max_projection_items: 128,
                max_projection_bytes: DEFAULT_PROJECTION_BYTES,
                max_mutations: 1000,
            },
            work: WorkLimits {
                max_requests: 1000,
                max_bytes: DEFAULT_ROUND_BYTES,
                max_depth: 10,
                max_rows_scanned: 1000,
                max_mutations: 1000,
            },
        }
    }
    fn scope() -> Scope<Model> {
        Scope {
            collections: [Collection::Notes].into_iter().collect(),
        }
    }
    fn key(n: u128) -> ObjectKey<Model> {
        ObjectKey {
            collection: Collection::Notes,
            id: Uuid::from_u128(n),
        }
    }
    async fn run_session(
        dst: &Arc<MemorySyncStore<Model>>,
        src: &Arc<MemorySyncStore<Model>>,
        source: &dyn SnapshotSource<Model>,
        limits: SessionLimits,
    ) {
        let mut coordinator = Coordinator::new(scope(), limits).unwrap();
        finish_coordinator(&mut coordinator, dst, src, source).await;
    }
    async fn finish_coordinator(
        coordinator: &mut Coordinator<Model>,
        dst: &Arc<MemorySyncStore<Model>>,
        src: &Arc<MemorySyncStore<Model>>,
        source: &dyn SnapshotSource<Model>,
    ) {
        let peer = StorePeer(src.clone());
        for _ in 0..4000 {
            coordinator.start_pass();
            if matches!(
                coordinator
                    .advance(&**dst, &**dst, &**dst, &peer, source)
                    .await
                    .unwrap(),
                SessionAdvance::Complete(_)
            ) {
                return;
            }
        }
        panic!("session did not complete");
    }
    impl CountingSource {
        fn new(inner: Arc<MemorySyncStore<Model>>) -> Self {
            Self {
                inner,
                calls: 0.into(),
                candidates: 0.into(),
                stale_open: false,
                open_calls: 0.into(),
            }
        }
    }
    async fn relay_setup() -> (
        Arc<MemorySyncStore<Model>>,
        Arc<MemorySyncStore<Model>>,
        Arc<MemorySyncStore<Model>>,
    ) {
        let a = Arc::new(store(2));
        let b = Arc::new(store(3));
        let c = Arc::new(store(4));
        let d = Arc::new(store(5));
        for n in 1..=512 {
            a.write_local(key(n), Some(Value { n: n as u32 }))
                .await
                .unwrap();
        }
        let limits = session_limits();
        run_session(&c, &a, &*a, limits).await;
        run_session(&b, &c, &*c, limits).await;
        run_session(&d, &a, &*a, limits).await;
        a.write_local(key(9000), Some(Value { n: 9000 }))
            .await
            .unwrap();
        run_session(&d, &a, &*a, limits).await;
        a.write_local(key(9000), None).await.unwrap();
        (a, b, d)
    }
    #[tokio::test]
    async fn create_delete_elsewhere_promotes_once_and_suppresses_stale() {
        use std::sync::atomic::Ordering;
        let (a, b, d) = relay_setup().await;
        let source = CountingSource::new(a.clone());
        let before = b.head().await.unwrap().sequence;
        run_session(&b, &a, &source, session_limits()).await;
        assert_eq!(source.calls.load(Ordering::Relaxed), 0);
        assert_eq!(b.head().await.unwrap().sequence, before + 1);
        assert!(
            a.lock()
                .unwrap()
                .core
                .base(&Collection::Notes)
                .is_subset(&b.lock().unwrap().core.base(&Collection::Notes))
        );
        run_session(&b, &d, &*d, session_limits()).await;
        assert!(b.get(&key(9000)).await.unwrap().is_none());
    }
    #[tokio::test]
    async fn post_snapshot_arrival_is_removed_by_promotion() {
        use std::sync::atomic::Ordering;
        let (a, b, d) = relay_setup().await;
        let source = CountingSource::new(a.clone());
        let peer = StorePeer(a.clone());
        let mut coordinator = Coordinator::new(scope(), session_limits()).unwrap();
        for _ in 0..1000 {
            coordinator.start_pass();
            coordinator
                .advance(&*b, &*b, &*b, &peer, &source)
                .await
                .unwrap();
            if coordinator.phase_label() == "compare" {
                break;
            }
        }
        assert_eq!(coordinator.phase_label(), "compare");
        run_session(&b, &d, &*d, session_limits()).await;
        assert_eq!(
            b.get(&key(9000)).await.unwrap().unwrap().value,
            Some(Value { n: 9000 })
        );
        finish_coordinator(&mut coordinator, &b, &a, &source).await;
        assert!(b.get(&key(9000)).await.unwrap().is_none());
        assert_eq!(source.calls.load(Ordering::Relaxed), 0);
    }
    async fn open(store: &MemorySyncStore<Model>, limits: WorkLimits) -> SnapshotDescriptor<Model> {
        let request = Uuid::new_v4();
        let mut cursor = None;
        loop {
            match store
                .open_snapshot(request, scope(), limits, cursor)
                .await
                .unwrap()
            {
                WorkOutcome::Ready(descriptor) => return descriptor,
                WorkOutcome::Incomplete { cursor: next } => cursor = Some(next),
            }
        }
    }
    async fn stage_contexts(
        dst: &MemorySyncStore<Model>,
        src: &MemorySyncStore<Model>,
        session: Uuid,
        remote: &SnapshotDescriptor<Model>,
    ) {
        for context in &remote.contexts {
            let stage = dst
                .begin_stage(
                    StageDescriptor {
                        origin: StageOrigin::SnapshotContext {
                            session,
                            snapshot_id: remote.snapshot_id,
                            collection: context.collection.clone(),
                        },
                        manifest: context.context.manifest.clone(),
                    },
                    session_limits().stages,
                )
                .await
                .unwrap();
            for ordinal in 0..context.context.manifest.fragment_count {
                let fragment = SnapshotSource::read_fragment(
                    src,
                    remote.snapshot_id,
                    context.context.manifest.clone(),
                    ordinal,
                    DEFAULT_PAGE_BYTES,
                )
                .await
                .unwrap();
                dst.stage_fragment(stage.stage, fragment).await.unwrap();
            }
        }
    }
    #[tokio::test]
    async fn source_live_post_snapshot_arrival_survives_promotion() {
        let a = Arc::new(store(2));
        let b = Arc::new(store(3));
        a.write_local(key(42), Some(Value { n: 42 })).await.unwrap();
        let peer = StorePeer(a.clone());
        let mut coordinator = Coordinator::new(scope(), session_limits()).unwrap();
        for _ in 0..100 {
            coordinator.start_pass();
            coordinator
                .advance(&*b, &*b, &*b, &peer, &*a)
                .await
                .unwrap();
            if coordinator.phase_label() == "compare" {
                break;
            }
        }
        assert_eq!(coordinator.phase_label(), "compare");
        let Event::Key(delta) = a.lock().unwrap().core.journal()[0].event.clone() else {
            panic!()
        };
        b.lock().unwrap().core.apply_delta(&delta).unwrap();
        let head = b.head().await.unwrap();
        finish_coordinator(&mut coordinator, &b, &a, &*a).await;
        assert_eq!(
            b.get(&key(42)).await.unwrap().unwrap().value,
            Some(Value { n: 42 })
        );
        assert_eq!(b.head().await.unwrap(), head);
    }
    #[tokio::test]
    async fn promotion_chunks_resume_and_are_idempotent() {
        let a = store(2);
        let b = store(3);
        let writer = Uuid::from_u128(999);
        let mut context = crate::causal::Context::default();
        for n in 1..=3000 {
            context
                .insert(crate::causal::Dot {
                    writer,
                    counter: 2 * n,
                })
                .unwrap();
        }
        a.lock()
            .unwrap()
            .core
            .apply_merge(&MergeDelta {
                collection: Collection::Notes,
                added: vec![],
                observed: exact_context(&context),
            })
            .unwrap();
        let mut limits = session_limits();
        limits.apply.max_mutations = 400;
        let remote = open(&a, limits.work).await;
        let local = open(&b, limits.work).await;
        let session = b
            .begin_reconciliation(Uuid::new_v4(), remote.clone(), local)
            .await
            .unwrap();
        assert!(matches!(
            b.lock().unwrap().receiver.prepare_promotion(session, 100),
            Err(SyncError::IncompleteCoverage)
        ));
        stage_contexts(&b, &a, session, &remote).await;
        b.accept_equal_range(session, remote.roots[0].clone())
            .await
            .unwrap();
        let mut steps = 0;
        loop {
            let receiver = b.lock().unwrap().receiver.clone();
            let first = receiver.prepare_promotion(session, 100).unwrap();
            assert_eq!(first, receiver.prepare_promotion(session, 100).unwrap());
            let token = first.unwrap();
            let progress = b.promote_step(session, limits.apply).await.unwrap();
            assert!(matches!(
                b.lock().unwrap().receiver.finalize_promotion(&token),
                Err(SyncError::StaleWork)
            ));
            steps += 1;
            if progress.done {
                break;
            }
            assert!(matches!(
                b.complete_reconciliation(session).await,
                Err(SyncError::IncompleteCoverage)
            ));
        }
        assert!(steps >= 31);
        assert!(context.is_subset(&b.lock().unwrap().core.base(&Collection::Notes)));
        let head = b.head().await.unwrap();
        assert_eq!(
            b.promote_step(session, limits.apply).await.unwrap(),
            PromoteProgress {
                done: true,
                causal_changes: 0
            }
        );
        assert_eq!(b.head().await.unwrap(), head);
        b.complete_reconciliation(session).await.unwrap();
        assert!(b.lock().unwrap().descriptors.is_empty());
    }
    #[tokio::test]
    async fn converged_pair_first_contact_is_empty() {
        use std::sync::atomic::Ordering;
        let a = Arc::new(store(2));
        let b = Arc::new(store(3));
        let c = Arc::new(store(4));
        for n in 1..=256 {
            a.write_local(key(n), Some(Value { n: n as u32 }))
                .await
                .unwrap();
        }
        run_session(&c, &a, &*a, session_limits()).await;
        run_session(&b, &c, &*c, session_limits()).await;
        let before = b.head().await.unwrap();
        let source = CountingSource::new(a.clone());
        run_session(&b, &a, &source, session_limits()).await;
        assert_eq!(source.calls.load(Ordering::Relaxed), 0);
        assert_eq!(b.head().await.unwrap(), before);
    }
    #[tokio::test]
    async fn coordinator_stops_after_eight_stale_restarts() {
        use std::sync::atomic::Ordering;
        let a = Arc::new(store(2));
        let b = Arc::new(store(3));
        let mut source = CountingSource::new(a.clone());
        source.stale_open = true;
        let peer = StorePeer(a.clone());
        let mut coordinator = Coordinator::new(scope(), session_limits()).unwrap();
        for _ in 0..300 {
            coordinator.start_pass();
            match coordinator.advance(&*b, &*b, &*b, &peer, &source).await {
                Err(SyncError::Invalid(message)) => {
                    assert_eq!(message, "coordinator restart limit exceeded");
                    assert_eq!(source.open_calls.load(Ordering::Relaxed), 8);
                    assert!(b.lock().unwrap().checkpoints.is_empty());
                    return;
                }
                Ok(SessionAdvance::Incomplete) => (),
                other => panic!("unexpected result: {other:?}"),
            }
        }
        panic!("coordinator kept restarting");
    }
}
