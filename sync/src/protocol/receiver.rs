//! Receiver-only staging and coverage. Accepted values remain in the adapter.
use super::*;
pub const MAX_RECEIVER_METADATA_ITEMS: usize = 4096;
pub const MAX_RECEIVER_METADATA_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_RECEIVER_RETRY_RECEIPTS: usize = 1024;
pub const MAX_RECEIVER_EXPANDED_BYTES: u64 = DEFAULT_LOGICAL_BYTES;

#[derive(Clone)]
pub(crate) struct PendingRange<M: SyncModel> {
    pub(crate) summary: RangeSummary<M>,
    pub(crate) candidates: Vec<Candidate<M>>,
    pub(crate) leaf_next: Option<CandidateCursor<M>>,
    pub(crate) leaf_done: bool,
    pub(crate) staged_bytes: u64,
}
#[derive(Clone)]
pub(crate) struct Reconciliation<M: SyncModel> {
    pub(crate) id: Uuid,
    pub(crate) remote: SnapshotDescriptor<M>,
    pub(crate) local: SnapshotDescriptor<M>,
    pub(crate) pending: Vec<PendingRange<M>>,
    pub(crate) completed: Vec<RangeSummary<M>>,
    pub(crate) revision: u64,
    pub(crate) source_live: Vec<(M::Collection, crate::causal::Context)>,
    promotion: Promotion,
}
/// Receiver-only volatile work. It contains no accepted records or history.
#[derive(Clone)]
pub struct ReceiverLedger<M: SyncModel> {
    pub(crate) stages: FragmentStages<M>,
    pub(crate) descriptors: Vec<(Uuid, StageDescriptor<M>)>,
    pub(crate) stage_deadlines: Vec<(Uuid, u64)>,
    pub(crate) sessions: Vec<Reconciliation<M>>,
    pub(crate) completed_sessions: Vec<(Uuid, ReceiveProgress, u64)>,
    pub(crate) completed_events: Vec<(Uuid, CheckpointKey, u64, ReceiveProgress, u64)>,
}
impl<M: SyncModel> Default for ReceiverLedger<M> {
    fn default() -> Self {
        Self {
            stages: FragmentStages::default(),
            descriptors: Vec::new(),
            stage_deadlines: Vec::new(),
            sessions: Vec::new(),
            completed_sessions: Vec::new(),
            completed_events: Vec::new(),
        }
    }
}

/// Owned evidence produced only after complete page and dependency validation.
#[derive(Clone, Debug)]
pub struct PreparedLeaf<M: SyncModel> {
    pub token: RangeAdmission<M>,
    pub candidates: Vec<Candidate<M>>,
    pub source_context: crate::causal::Context,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Promotion {
    index: usize,
    arrivals_done: bool,
    after: Option<(Uuid, u64)>,
    done: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromotionStep<M: SyncModel> {
    Arrivals {
        collection: M::Collection,
        source_context: crate::causal::Context,
        source_live: crate::causal::Context,
    },
    Chunk {
        collection: M::Collection,
        chunk: crate::causal::Context,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotionAdmission<M: SyncModel> {
    pub session: Uuid,
    pub ledger_revision: u64,
    pub step: PromotionStep<M>,
    next: Promotion,
}
/// Revision-bound receipt; serialize access to the session across SQL commit
/// and finalize, or roll back the accepted-state transaction on a stale token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeAdmission<M: SyncModel> {
    pub session: Uuid,
    pub range: KeyRange<M>,
    pub ledger_revision: u64,
    pub summary: RangeSummary<M>,
}
impl<M: SyncModel> ReceiverLedger<M> {
    fn reserve_retry_receipt(&mut self) {
        while self.completed_sessions.len() + self.completed_events.len()
            >= MAX_RECEIVER_RETRY_RECEIPTS
        {
            if !self.completed_events.is_empty() {
                self.completed_events.remove(0);
            } else if !self.completed_sessions.is_empty() {
                self.completed_sessions.remove(0);
            } else {
                break;
            }
        }
    }
    fn metadata_usage(&self) -> Result<(usize, u64)> {
        let mut count = 0usize;
        let mut bytes = 0u64;
        for session in &self.sessions {
            count = count.checked_add(1).ok_or(SyncError::StageQuota)?;
            let remote_bytes = serde_json::to_vec(&session.remote)
                .map_err(|e| SyncError::Storage(e.to_string()))?
                .len() as u64;
            let local_bytes = serde_json::to_vec(&session.local)
                .map_err(|e| SyncError::Storage(e.to_string()))?
                .len() as u64;
            bytes = bytes
                .checked_add(remote_bytes)
                .and_then(|n| n.checked_add(local_bytes))
                .and_then(|n| n.checked_add(128))
                .ok_or(SyncError::StageQuota)?;
            for summary in session
                .pending
                .iter()
                .map(|p| &p.summary)
                .chain(session.completed.iter())
            {
                count = count.checked_add(1).ok_or(SyncError::StageQuota)?;
                bytes = bytes
                    .checked_add(
                        serde_json::to_vec(summary)
                            .map_err(|e| SyncError::Storage(e.to_string()))?
                            .len() as u64
                            + 64,
                    )
                    .ok_or(SyncError::StageQuota)?;
            }
        }
        for (_, descriptor) in &self.descriptors {
            count = count.checked_add(1).ok_or(SyncError::StageQuota)?;
            bytes = bytes
                .checked_add(
                    serde_json::to_vec(descriptor)
                        .map_err(|e| SyncError::Storage(e.to_string()))?
                        .len() as u64
                        + 64,
                )
                .ok_or(SyncError::StageQuota)?;
        }
        Ok((count, bytes))
    }
    /// Release only uncommitted coverage and staged graphs; accepted state belongs to the adapter.
    pub fn cancel_reconciliation(&mut self, session: Uuid) -> Result<()> {
        let stages: Vec<_> = self.descriptors.iter().filter_map(|(id, descriptor)| matches!(&descriptor.origin, StageOrigin::SnapshotContext { session: owner, .. } if *owner == session).then_some(*id)).collect();
        for id in stages {
            if self.descriptors.iter().any(|(stage, _)| *stage == id) {
                self.abandon_stage(id)?;
            }
        }
        self.sessions.retain(|active| active.id != session);
        Ok(())
    }
    pub fn discard_range(&mut self, session: Uuid, range: &KeyRange<M>) -> Result<()> {
        let active = self
            .sessions
            .iter_mut()
            .find(|v| v.id == session)
            .ok_or(SyncError::StaleWork)?;
        let pending = active
            .pending
            .iter_mut()
            .find(|v| &v.summary.range == range)
            .ok_or(SyncError::StaleWork)?;
        pending.candidates.clear();
        pending.leaf_next = None;
        pending.leaf_done = false;
        pending.staged_bytes = 0;
        Ok(())
    }
    pub fn new() -> Self {
        Self::default()
    }
    pub fn descriptors(
        &self,
        session: Uuid,
    ) -> Result<(SnapshotDescriptor<M>, SnapshotDescriptor<M>)> {
        let active = self
            .sessions
            .iter()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?;
        Ok((active.remote.clone(), active.local.clone()))
    }
    pub fn begin_stage(
        &mut self,
        id: Uuid,
        descriptor: StageDescriptor<M>,
        limits: StageLimits,
        deadline: u64,
    ) -> Result<FragmentProgress> {
        if let Some((existing, _)) = self.descriptors.iter().find(|(_, d)| *d == descriptor) {
            if self.stages.limits(*existing)? != limits {
                return Err(SyncError::Invalid("inconsistent graph stage limits".into()));
            }
            return self.stages.status(*existing);
        }
        let (items, bytes) = self.metadata_usage()?;
        let descriptor_bytes = serde_json::to_vec(&descriptor)
            .map_err(|e| SyncError::Storage(e.to_string()))?
            .len() as u64
            + 64;
        if items >= MAX_RECEIVER_METADATA_ITEMS
            || bytes
                .checked_add(descriptor_bytes)
                .is_none_or(|v| v > MAX_RECEIVER_METADATA_BYTES)
        {
            return Err(SyncError::StageQuota);
        }
        let pages = self
            .sessions
            .iter()
            .flat_map(|s| &s.pending)
            .try_fold(0u64, |n, p| {
                n.checked_add(p.staged_bytes).ok_or(SyncError::StageQuota)
            })?;
        if pages
            .checked_add(self.stages.reserved_bytes())
            .and_then(|n| n.checked_add(descriptor.manifest.encoded_bytes))
            .is_none_or(|n| n > limits.max_total_bytes)
        {
            return Err(SyncError::StageQuota);
        }
        match &descriptor.origin {
            StageOrigin::SnapshotContext {
                session,
                snapshot_id,
                collection,
            } => {
                let active = self
                    .sessions
                    .iter()
                    .find(|s| s.id == *session)
                    .ok_or(SyncError::StaleWork)?;
                if active.remote.snapshot_id != *snapshot_id
                    || !active.remote.contexts.iter().any(|c| {
                        c.collection == *collection && c.context.manifest == descriptor.manifest
                    })
                {
                    return Err(SyncError::Invalid(
                        "invalid snapshot context stage origin".into(),
                    ));
                }
                if descriptor.manifest.encoded_bytes > limits.max_event_bytes {
                    return Err(SyncError::LogicalLimit);
                }
            }
            StageOrigin::Journal {
                request,
                sequence,
                collection,
                ..
            } => {
                scope_id(&request.scope)?;
                initial_anchor(&request.source)?;
                if request.after.sequence.checked_add(1) != Some(*sequence)
                    || *sequence > request.through.sequence
                    || !request.scope.collections.contains(collection)
                {
                    return Err(SyncError::Invalid("invalid journal stage origin".into()));
                }
            }
        }
        let progress = self.stages.begin(id, descriptor.clone(), limits)?;
        self.descriptors.push((id, descriptor));
        self.stage_deadlines.push((id, deadline));
        Ok(progress)
    }
    pub fn stage_fragment(&mut self, id: Uuid, fragment: Fragment) -> Result<FragmentProgress> {
        self.stages.stage(id, fragment)
    }
    pub fn stage_status(&self, id: Uuid) -> Result<FragmentProgress> {
        self.stages.status(id)
    }
    pub fn abandon_stage(&mut self, id: Uuid) -> Result<()> {
        let ids = self.stages.abandon_group(id)?;
        self.descriptors.retain(|(id, _)| !ids.contains(id));
        self.stage_deadlines.retain(|(id, _)| !ids.contains(id));
        Ok(())
    }
    pub fn abandon_origin(&mut self, origin: &StageOrigin<M>) -> Result<()> {
        if let Some(id) = self
            .descriptors
            .iter()
            .find_map(|(id, descriptor)| (&descriptor.origin == origin).then_some(*id))
        {
            self.abandon_stage(id)?;
        }
        Ok(())
    }
    pub fn expire(&mut self, now: u64) {
        self.sessions
            .retain(|s| s.local.expires_unix_ms > now && s.remote.expires_unix_ms > now);
        self.completed_sessions.retain(|(_, _, until)| *until > now);
        self.completed_events
            .retain(|(_, _, _, _, until)| *until > now);
        let ids: Vec<_> = self
            .descriptors
            .iter()
            .filter_map(|(id, d)| match &d.origin {
                StageOrigin::SnapshotContext { session, .. }
                    if !self.sessions.iter().any(|s| s.id == *session) =>
                {
                    Some(*id)
                }
                _ => self
                    .stage_deadlines
                    .iter()
                    .find(|(stage, _)| stage == id)
                    .and_then(|(_, until)| (*until <= now).then_some(*id)),
            })
            .collect();
        for id in ids {
            let _ = self.abandon_stage(id);
        }
    }
    pub fn begin_reconciliation(
        &mut self,
        id: Uuid,
        remote: SnapshotDescriptor<M>,
        local: SnapshotDescriptor<M>,
        now: u64,
    ) -> Result<Uuid> {
        if id.is_nil() {
            return Err(SyncError::Invalid("nil reconciliation id".into()));
        }
        if remote.expires_unix_ms <= now || local.expires_unix_ms <= now {
            return Err(SyncError::SnapshotExpired);
        }
        initial_anchor(&remote.source)?;
        initial_anchor(&local.source)?;
        scope_id(&remote.scope)?;
        if remote.scope != local.scope
            || remote.source.namespace != local.source.namespace
            || remote.source.model != local.source.model
            || remote.source.model.model != M::model_id()
            || remote.source.model.schema != M::schema_id()
            || remote.snapshot_id.is_nil()
            || local.snapshot_id.is_nil()
            || remote.roots.len() != remote.scope.collections.len()
            || remote.head.sequence > MAX_NUMBER
            || local.head.sequence > MAX_NUMBER
            || (remote.head.sequence == 0 && remote.head.anchor != initial_anchor(&remote.source)?)
            || (local.head.sequence == 0 && local.head.anchor != initial_anchor(&local.source)?)
        {
            return Err(SyncError::Invalid(
                "incompatible snapshot descriptors".into(),
            ));
        }
        for descriptor in [&remote, &local] {
            canonical_context_root(&ContextRoot {
                scope: descriptor.scope.clone(),
                contexts: descriptor.contexts.clone(),
            })?;
        }
        let mut seen = std::collections::BTreeSet::new();
        for root in &remote.roots {
            if root.snapshot_id != remote.snapshot_id
                || root.range.lower.is_some()
                || root.range.upper.is_some()
                || !remote.scope.collections.contains(&root.range.collection)
                || !seen.insert(M::collection_tag(&root.range.collection))
            {
                return Err(SyncError::Invalid("invalid snapshot roots".into()));
            }
        }
        if let Some(existing) = self.sessions.iter().find(|session| session.id == id) {
            if existing.remote == remote && existing.local == local {
                return Ok(id);
            }
            return Err(SyncError::Invalid(
                "reconciliation request parameters changed".into(),
            ));
        }
        if self
            .completed_sessions
            .iter()
            .any(|(session, _, _)| *session == id)
        {
            return Err(SyncError::Invalid("duplicate reconciliation id".into()));
        }
        if self.sessions.len() >= DEFAULT_STAGES as usize {
            return Err(SyncError::StageQuota);
        }
        let (current_items, current_bytes) = self.metadata_usage()?;
        let descriptor_bytes = (serde_json::to_vec(&remote)
            .map_err(|e| SyncError::Storage(e.to_string()))?
            .len() as u64)
            .checked_add(
                serde_json::to_vec(&local)
                    .map_err(|e| SyncError::Storage(e.to_string()))?
                    .len() as u64,
            )
            .and_then(|n| n.checked_add(128))
            .ok_or(SyncError::StageQuota)?;
        let new_bytes = remote
            .roots
            .iter()
            .try_fold(descriptor_bytes, |n, summary| {
                n.checked_add(
                    serde_json::to_vec(summary)
                        .map_err(|e| SyncError::Storage(e.to_string()))?
                        .len() as u64
                        + 64,
                )
                .ok_or(SyncError::StageQuota)
            })?;
        if current_items
            .checked_add(remote.roots.len() + 1)
            .is_none_or(|v| v > MAX_RECEIVER_METADATA_ITEMS)
            || current_bytes
                .checked_add(new_bytes)
                .is_none_or(|v| v > MAX_RECEIVER_METADATA_BYTES)
        {
            return Err(SyncError::StageQuota);
        }
        let pending = remote
            .roots
            .iter()
            .cloned()
            .map(|summary| PendingRange {
                summary,
                candidates: Vec::new(),
                leaf_next: None,
                leaf_done: false,
                staged_bytes: 0,
            })
            .collect();
        self.sessions.push(Reconciliation {
            id,
            remote: remote.clone(),
            local,
            pending,
            completed: Vec::new(),
            revision: 0,
            source_live: remote
                .contexts
                .iter()
                .map(|c| (c.collection.clone(), crate::causal::Context::default()))
                .collect(),
            promotion: Promotion {
                index: 0,
                arrivals_done: false,
                after: None,
                done: remote.contexts.is_empty(),
            },
        });
        Ok(id)
    }
    pub fn progress(&self, session: Uuid) -> Result<CoverageProgress> {
        let active = self
            .sessions
            .iter()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?;
        Ok(CoverageProgress {
            ledger_revision: active.revision,
            outstanding_ranges: active.pending.len() as u64,
            staged_leaves: active
                .pending
                .iter()
                .filter(|p| !p.candidates.is_empty() || p.leaf_done)
                .count() as u64,
            complete: active.pending.is_empty(),
        })
    }
    pub fn completed_range(
        &self,
        session: Uuid,
        range: &KeyRange<M>,
    ) -> Result<Option<CoverageProgress>> {
        let active = self
            .sessions
            .iter()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?;
        if active
            .completed
            .iter()
            .any(|summary| &summary.range == range)
        {
            Ok(Some(self.progress(session)?))
        } else {
            Ok(None)
        }
    }
    pub fn leaf_progress(&self, session: Uuid, range: &KeyRange<M>) -> Result<LeafProgress<M>> {
        let p = self
            .sessions
            .iter()
            .find(|s| s.id == session)
            .and_then(|s| s.pending.iter().find(|p| &p.summary.range == range))
            .ok_or(SyncError::StaleWork)?;
        Ok(LeafProgress {
            last: p.leaf_next.clone(),
            complete: p.leaf_done,
        })
    }
}
impl<M: SyncModel> ReceiverLedger<M> {
    pub fn coverage(
        &self,
        session: Uuid,
        revision: Option<u64>,
        after: u64,
        limits: PageLimits,
    ) -> Result<CoveragePage<M>> {
        if limits.max_scanned == 0
            || limits.max_scanned > MAX_PAGE_ITEMS
            || limits.max_bytes == 0
            || limits.max_bytes > MAX_PAGE_BYTES
        {
            return Err(SyncError::Invalid("invalid page limits".into()));
        }
        let active = self
            .sessions
            .iter()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?;
        if revision.is_some_and(|v| v != active.revision) || after > active.pending.len() as u64 {
            return Err(SyncError::StaleWork);
        }
        let mut outstanding = Vec::new();
        let mut bytes = 0u64;
        for p in active.pending.iter().skip(after as usize) {
            let n = serde_json::to_vec(&p.summary.range)
                .map_err(|e| SyncError::Storage(e.to_string()))?
                .len() as u64;
            let candidate_next = after + outstanding.len() as u64 + 1;
            let envelope = CoveragePage::<M> {
                ledger_revision: active.revision,
                after,
                next: candidate_next,
                done: candidate_next == active.pending.len() as u64,
                outstanding: Vec::new(),
            };
            let base = serde_json::to_vec(&envelope)
                .map_err(|e| SyncError::Storage(e.to_string()))?
                .len() as u64;
            let required = base
                .checked_add(bytes)
                .and_then(|v| v.checked_add(n))
                .and_then(|v| v.checked_add(outstanding.len() as u64))
                .ok_or(SyncError::LogicalLimit)?;
            if outstanding.len() >= limits.max_scanned as usize || required > limits.max_bytes {
                if outstanding.is_empty() {
                    return Err(SyncError::LimitTooSmall { required });
                }
                break;
            }
            bytes += n;
            outstanding.push(p.summary.range.clone());
        }
        let next = after + outstanding.len() as u64;
        let page = CoveragePage {
            ledger_revision: active.revision,
            after,
            next,
            done: next == active.pending.len() as u64,
            outstanding,
        };
        let required = serde_json::to_vec(&page)
            .map_err(|e| SyncError::Storage(e.to_string()))?
            .len() as u64;
        if required > limits.max_bytes {
            return Err(SyncError::LimitTooSmall { required });
        }
        Ok(page)
    }
    pub fn split_range(
        &mut self,
        session: Uuid,
        parent: KeyRange<M>,
        children: Vec<RangeSummary<M>>,
    ) -> Result<CoverageProgress> {
        let (metadata_items, metadata_bytes) = self.metadata_usage()?;
        let children_bytes = children.iter().try_fold(0u64, |n, child| {
            n.checked_add(
                serde_json::to_vec(child)
                    .map_err(|e| SyncError::Storage(e.to_string()))?
                    .len() as u64
                    + 64,
            )
            .ok_or(SyncError::StageQuota)
        })?;
        let active = self
            .sessions
            .iter_mut()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?;
        if children.len() < 2 || children.len() as u64 > MAX_LOGICAL_ITEMS {
            return Err(SyncError::Invalid("invalid split size".into()));
        }
        let mut lower = parent.lower;
        for (i, child) in children.iter().enumerate() {
            if child.snapshot_id != active.remote.snapshot_id
                || child.range.collection != parent.collection
                || child.range.lower != lower
                || child
                    .range
                    .lower
                    .zip(child.range.upper)
                    .is_some_and(|(a, b)| a >= b)
                || (child.range.upper.is_none() && i + 1 < children.len())
            {
                return Err(SyncError::Invalid("invalid split coverage".into()));
            }
            lower = child.range.upper;
        }
        if lower != parent.upper {
            return Err(SyncError::IncompleteCoverage);
        }
        let index = match active
            .pending
            .iter()
            .position(|p| p.summary.range == parent)
        {
            Some(index) => index,
            None if children
                .iter()
                .all(|c| active.pending.iter().any(|p| p.summary == *c)) =>
            {
                return self.progress(session);
            }
            None => return Err(SyncError::StaleWork),
        };
        let old = &active.pending[index];
        let old_bytes = serde_json::to_vec(&old.summary)
            .map_err(|e| SyncError::Storage(e.to_string()))?
            .len() as u64
            + 64;
        if metadata_items
            .checked_sub(1)
            .and_then(|n| n.checked_add(children.len()))
            .is_none_or(|n| n > MAX_RECEIVER_METADATA_ITEMS)
            || metadata_bytes
                .checked_sub(old_bytes)
                .and_then(|n| n.checked_add(children_bytes))
                .is_none_or(|n| n > MAX_RECEIVER_METADATA_BYTES)
        {
            return Err(SyncError::StageQuota);
        }
        if !old.candidates.is_empty() || old.leaf_done {
            return Err(SyncError::Invalid("range already staged".into()));
        }
        let revision = active
            .revision
            .checked_add(1)
            .ok_or(SyncError::ClockExhausted)?;
        active.pending.splice(
            index..=index,
            children.into_iter().map(|summary| PendingRange {
                summary,
                candidates: Vec::new(),
                leaf_next: None,
                leaf_done: false,
                staged_bytes: 0,
            }),
        );
        active.revision = revision;
        self.progress(session)
    }
    pub fn accept_equal_range(
        &mut self,
        session: Uuid,
        source: RangeSummary<M>,
        local: &RangeSummary<M>,
    ) -> Result<CoverageProgress> {
        let active = self
            .sessions
            .iter_mut()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?;
        if active.completed.contains(&source) {
            return self.progress(session);
        }
        let p = active
            .pending
            .iter()
            .find(|p| p.summary.range == source.range)
            .ok_or(SyncError::StaleWork)?;
        if p.summary != source
            || local.snapshot_id != active.local.snapshot_id
            || local.range != source.range
            || !p.candidates.is_empty()
            || p.leaf_done
            || local.aggregate != source.aggregate
            || local.candidate_count != source.candidate_count
            || local.candidate_bytes != source.candidate_bytes
        {
            return Err(SyncError::Invalid(
                "range summaries differ or stage mismatch".into(),
            ));
        }
        let revision = active
            .revision
            .checked_add(1)
            .ok_or(SyncError::ClockExhausted)?;
        active.pending.retain(|p| p.summary.range != source.range);
        active.completed.push(source);
        active.revision = revision;
        self.progress(session)
    }
    pub fn stage_leaf_page(&mut self, session: Uuid, page: LeafPage<M>) -> Result<LeafProgress<M>> {
        let page_bytes = serde_json::to_vec(&page)
            .map_err(|e| SyncError::Storage(e.to_string()))?
            .len() as u64;
        if page_bytes > MAX_PAGE_BYTES {
            return Err(SyncError::LogicalLimit);
        }
        let total = self
            .sessions
            .iter()
            .flat_map(|s| &s.pending)
            .try_fold(self.stages.reserved_bytes(), |n, p| {
                n.checked_add(p.staged_bytes).ok_or(SyncError::StageQuota)
            })?;
        let active = self
            .sessions
            .iter_mut()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?;
        if page.snapshot_id != active.remote.snapshot_id {
            return Err(SyncError::StaleWork);
        }
        let namespace = active.remote.source.namespace;
        let p = active
            .pending
            .iter_mut()
            .find(|p| p.summary.range == page.range)
            .ok_or(SyncError::StaleWork)?;
        let start = p.candidates.len().saturating_sub(page.candidates.len());
        let previous = if start == 0 {
            None
        } else {
            let c = &p.candidates[start - 1];
            Some(CandidateCursor {
                key: c.key.clone(),
                dot: c.dot,
            })
        };
        if (!page.candidates.is_empty() || page.done)
            && p.leaf_next == page.next
            && p.leaf_done == page.done
            && p.candidates.ends_with(&page.candidates)
            && page.after == previous
        {
            return Ok(LeafProgress {
                last: p.leaf_next.clone(),
                complete: p.leaf_done,
            });
        }
        if total
            .checked_add(page_bytes)
            .is_none_or(|n| n > MAX_STAGE_BYTES)
        {
            return Err(SyncError::StageQuota);
        }
        if p.staged_bytes
            .checked_add(page_bytes)
            .is_none_or(|n| n > MAX_LOGICAL_BYTES)
        {
            return Err(SyncError::LogicalLimit);
        }
        if p.leaf_done || p.leaf_next != page.after || (page.candidates.is_empty() && !page.done) {
            return Err(SyncError::Invalid("invalid leaf page".into()));
        }
        let mut last = page.after.clone();
        for c in &page.candidates {
            validate_candidate(c)?;
            let cursor = CandidateCursor {
                key: c.key.clone(),
                dot: c.dot,
            };
            if c.namespace != namespace
                || c.key.collection != page.range.collection
                || page.range.lower.is_some_and(|x| c.key.id < x)
                || page.range.upper.is_some_and(|x| c.key.id >= x)
                || last.as_ref().is_some_and(|l| *l >= cursor)
            {
                return Err(SyncError::Invalid("invalid leaf candidate".into()));
            }
            last = Some(cursor);
        }
        if page.next != last
            || p.candidates.len() + page.candidates.len() > MAX_LOGICAL_ITEMS as usize
        {
            return Err(SyncError::LogicalLimit);
        }
        p.candidates.extend(page.candidates);
        p.staged_bytes += page_bytes;
        p.leaf_next = page.next;
        p.leaf_done = page.done;
        Ok(LeafProgress {
            last: p.leaf_next.clone(),
            complete: p.leaf_done,
        })
    }
}
impl<M: SyncModel> ReceiverLedger<M> {
    /// Load the source's collection context from its session-scoped stage.
    pub fn source_context(
        &self,
        session: Uuid,
        collection: &M::Collection,
    ) -> Result<crate::causal::Context> {
        let active = self
            .sessions
            .iter()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?;
        let context = &active
            .remote
            .contexts
            .iter()
            .find(|c| &c.collection == collection)
            .ok_or(SyncError::Invalid(
                "collection outside snapshot scope".into(),
            ))?
            .context;
        let (id, _) = self.descriptors.iter().find(|(_, d)| d.manifest == context.manifest
            && matches!(&d.origin, StageOrigin::SnapshotContext { session: owner, snapshot_id, collection: c }
                if *owner == session && *snapshot_id == active.remote.snapshot_id && c == collection))
            .ok_or(SyncError::MissingDependency(context.manifest.manifest_digest))?;
        let bytes = self.stages.assembled(*id)?;
        let decoded = decode_context_remote(&bytes)?;
        if context_digest(&decoded)? != context.digest {
            return Err(SyncError::Invalid("context digest mismatch".into()));
        }
        causal_context(&decoded)
    }
    /// Validate the complete immutable leaf without mutating coverage.
    pub fn prepare_range(&self, session: Uuid, range: &KeyRange<M>) -> Result<PreparedLeaf<M>> {
        let active = self
            .sessions
            .iter()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?;
        let pending = active
            .pending
            .iter()
            .find(|p| &p.summary.range == range)
            .ok_or(SyncError::StaleWork)?;
        if !pending.leaf_done {
            return Err(SyncError::IncompleteCoverage);
        }
        let context_ref = &active
            .remote
            .contexts
            .iter()
            .find(|c| c.collection == range.collection)
            .ok_or(SyncError::Invalid(
                "collection outside snapshot scope".into(),
            ))?
            .context;
        let mut dots = BTreeSet::new();
        let mut candidate_bytes = 0u64;
        for candidate in &pending.candidates {
            if candidate.namespace != active.remote.source.namespace {
                return Err(SyncError::Invalid("candidate namespace mismatch".into()));
            }
            if !dots.insert(candidate.dot) {
                return Err(SyncError::ConflictingDot);
            }
            candidate_bytes = candidate_bytes
                .checked_add(canonical_candidate(candidate)?.len() as u64)
                .ok_or(SyncError::LogicalLimit)?;
        }
        // Bound resident expansion before decoding the staged context.
        if candidate_bytes
            .checked_add(context_ref.manifest.encoded_bytes)
            .and_then(|n| n.checked_mul(4))
            .is_none_or(|n| n > MAX_RECEIVER_EXPANDED_BYTES)
        {
            return Err(SyncError::WorkLimit);
        }
        let source_context = self.source_context(session, &range.collection)?;
        for candidate in &pending.candidates {
            if !source_context.contains(crate::causal::Dot {
                writer: candidate.dot.writer_incarnation,
                counter: candidate.dot.counter,
            }) {
                return Err(SyncError::Invalid(
                    "live candidate absent from source context".into(),
                ));
            }
        }
        let mut hash = RangeHasher::new(
            &active.remote.scope,
            range.clone(),
            pending.candidates.len() as u64,
            candidate_bytes,
        )?;
        for c in &pending.candidates {
            hash.push_candidate(c)?;
        }
        if hash.finish()? != pending.summary.aggregate
            || pending.summary.candidate_count != pending.candidates.len() as u64
            || pending.summary.candidate_bytes != candidate_bytes
        {
            return Err(SyncError::Invalid("range commitment mismatch".into()));
        }
        Ok(PreparedLeaf {
            token: RangeAdmission {
                session,
                range: range.clone(),
                ledger_revision: active.revision,
                summary: pending.summary.clone(),
            },
            candidates: pending.candidates.clone(),
            source_context,
        })
    }
    /// Finalize only after the adapter's accepted-state transaction commits.
    /// The adapter serializes all admissions for this session through commit.
    pub fn finalize_range(&mut self, token: &RangeAdmission<M>) -> Result<CoverageProgress> {
        let active = self
            .sessions
            .iter_mut()
            .find(|s| s.id == token.session)
            .ok_or(SyncError::StaleWork)?;
        if active.completed.contains(&token.summary) {
            return self.progress(token.session);
        }
        if active.revision != token.ledger_revision
            || !active
                .pending
                .iter()
                .any(|p| p.summary == token.summary && p.summary.range == token.range)
        {
            return Err(SyncError::StaleWork);
        }
        let revision = active
            .revision
            .checked_add(1)
            .ok_or(SyncError::ClockExhausted)?;
        let mut source_live = active
            .source_live
            .iter()
            .find(|(c, _)| *c == token.range.collection)
            .ok_or(SyncError::StaleWork)?
            .1
            .clone();
        for candidate in &active
            .pending
            .iter()
            .find(|p| p.summary == token.summary)
            .ok_or(SyncError::StaleWork)?
            .candidates
        {
            source_live
                .insert(crate::causal::Dot {
                    writer: candidate.dot.writer_incarnation,
                    counter: candidate.dot.counter,
                })
                .map_err(|e| SyncError::Invalid(e.to_string()))?;
        }
        active
            .source_live
            .iter_mut()
            .find(|(c, _)| *c == token.range.collection)
            .ok_or(SyncError::StaleWork)?
            .1 = source_live;
        active.pending.retain(|p| p.summary.range != token.range);
        active.completed.push(token.summary.clone());
        active.revision = revision;
        self.progress(token.session)
    }
}
/// Checkpoint publication input, bound to a complete receiver ledger revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionAdmission {
    pub session: Uuid,
    pub ledger_revision: u64,
    pub checkpoint: CheckpointKey,
    pub through: ReceiveProgress,
    pub expires_unix_ms: u64,
}
impl<M: SyncModel> ReceiverLedger<M> {
    pub fn completed_session(&self, session: Uuid) -> Option<ReceiveProgress> {
        self.completed_sessions
            .iter()
            .find(|(id, _, _)| *id == session)
            .map(|(_, progress, _)| progress.clone())
    }
    pub fn prepare_promotion(
        &self,
        session: Uuid,
        max_intervals: usize,
    ) -> Result<Option<PromotionAdmission<M>>> {
        if max_intervals == 0 {
            return Err(SyncError::Invalid("empty promotion budget".into()));
        }
        let active = self
            .sessions
            .iter()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?;
        if !active.pending.is_empty() {
            return Err(SyncError::IncompleteCoverage);
        }
        if active.promotion.done {
            return Ok(None);
        }
        let collection = active.remote.contexts[active.promotion.index]
            .collection
            .clone();
        let source_context = self.source_context(session, &collection)?;
        let mut next = active.promotion.clone();
        let step = if !next.arrivals_done {
            next.arrivals_done = true;
            let source_live = active
                .source_live
                .iter()
                .find(|(c, _)| *c == collection)
                .ok_or(SyncError::StaleWork)?
                .1
                .clone();
            PromotionStep::Arrivals {
                collection,
                source_context,
                source_live,
            }
        } else {
            let (chunk, after) = source_context.chunk(next.after, max_intervals);
            if let Some(after) = after {
                next.after = Some(after);
            } else {
                next = Promotion {
                    index: next.index + 1,
                    arrivals_done: false,
                    after: None,
                    done: next.index + 1 == active.remote.contexts.len(),
                };
            }
            PromotionStep::Chunk { collection, chunk }
        };
        Ok(Some(PromotionAdmission {
            session,
            ledger_revision: active.revision,
            step,
            next,
        }))
    }
    pub fn finalize_promotion(&mut self, token: &PromotionAdmission<M>) -> Result<()> {
        let active = self
            .sessions
            .iter_mut()
            .find(|s| s.id == token.session)
            .ok_or(SyncError::StaleWork)?;
        if active.revision != token.ledger_revision {
            return Err(SyncError::StaleWork);
        }
        let revision = active
            .revision
            .checked_add(1)
            .ok_or(SyncError::ClockExhausted)?;
        active.promotion = token.next.clone();
        active.revision = revision;
        Ok(())
    }
    pub fn promotion_done(&self, session: Uuid) -> Result<bool> {
        Ok(self
            .sessions
            .iter()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?
            .promotion
            .done)
    }
    pub fn prepare_completion(&self, session: Uuid) -> Result<CompletionAdmission> {
        let active = self
            .sessions
            .iter()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?;
        if !active.pending.is_empty() || !active.promotion.done {
            return Err(SyncError::IncompleteCoverage);
        }
        Ok(CompletionAdmission {
            session,
            ledger_revision: active.revision,
            checkpoint: CheckpointKey {
                namespace: active.remote.source.namespace,
                source_replica: active.remote.source.replica,
                source_generation: active.remote.source.generation,
                scope_id: scope_id(&active.remote.scope)?,
            },
            through: active.remote.head.clone(),
            expires_unix_ms: active.remote.expires_unix_ms,
        })
    }
    /// Invoke only after the checkpoint transaction commits under the session lock.
    pub fn finalize_completion(
        &mut self,
        token: &CompletionAdmission,
        committed: ReceiveProgress,
    ) -> Result<ReceiveProgress> {
        if let Some((_, progress, _)) = self
            .completed_sessions
            .iter()
            .find(|(id, _, _)| *id == token.session)
        {
            if *progress == committed {
                return Ok(progress.clone());
            }
            return Err(SyncError::StaleWork);
        }
        let expected = self.prepare_completion(token.session)?;
        if expected != *token
            || committed.sequence < token.through.sequence
            || (committed.sequence == token.through.sequence
                && committed.anchor != token.through.anchor)
        {
            return Err(SyncError::StaleWork);
        }
        self.cancel_reconciliation(token.session)?;
        self.reserve_retry_receipt();
        self.completed_sessions
            .push((token.session, committed.clone(), token.expires_unix_ms));
        Ok(committed)
    }
}
/// Complete verified event, ready for accepted-state admission.
#[derive(Clone, Debug)]
pub struct PreparedEvent<M: SyncModel> {
    pub origin: StageOrigin<M>,
    pub event: Event<M>,
    pub event_stage: Option<Uuid>,
    pub resident_bytes_reserved: u64,
}
impl<M: SyncModel> ReceiverLedger<M> {
    pub fn prepare_event(&self, stage: Uuid) -> Result<PreparedEvent<M>> {
        let descriptor = self
            .descriptors
            .iter()
            .find(|(id, _)| *id == stage)
            .map(|(_, d)| d)
            .ok_or(SyncError::StaleWork)?;
        if !matches!(
            descriptor.manifest.kind,
            ManifestKind::KeyEvent | ManifestKind::MergeEvent
        ) {
            return Err(SyncError::Invalid("stage does not contain an event".into()));
        }
        let event = decode_event_remote::<M>(&self.stages.assembled(stage)?)?;
        if !matches!(
            (&descriptor.manifest.kind, &event),
            (ManifestKind::KeyEvent, Event::Key(_)) | (ManifestKind::MergeEvent, Event::Merge(_))
        ) {
            return Err(SyncError::Invalid("event manifest kind mismatch".into()));
        }
        let mut result = self.prepare_inline_event(&descriptor.origin, &event)?;
        result.event_stage = Some(stage);
        Ok(result)
    }
    pub fn prepare_inline_event(
        &self,
        origin: &StageOrigin<M>,
        event: &Event<M>,
    ) -> Result<PreparedEvent<M>> {
        let StageOrigin::Journal {
            request,
            sequence,
            collection,
            event_digest: expected,
        } = origin
        else {
            return Err(SyncError::Invalid("event origin is not journal".into()));
        };
        scope_id(&request.scope)?;
        let genesis = initial_anchor(&request.source)?;
        if request.source.model.model != M::model_id()
            || request.source.model.schema != M::schema_id()
            || request.through.sequence > MAX_NUMBER
            || (request.after.sequence == 0 && request.after.anchor != genesis)
        {
            return Err(SyncError::IncompatibleModel);
        }
        if request.after.sequence.checked_add(1) != Some(*sequence)
            || *sequence > request.through.sequence
            || !request.scope.collections.contains(collection)
            || event_digest(event)? != *expected
        {
            return Err(SyncError::Invalid("invalid event origin".into()));
        }
        let event_collection = match event {
            Event::Key(key) => &key.key.collection,
            Event::Merge(m) => &m.collection,
        };
        if event_collection != collection {
            return Err(SyncError::Invalid("event collection mismatch".into()));
        }
        let added = match event {
            Event::Key(k) => &k.added,
            Event::Merge(m) => &m.added,
        };
        if added
            .iter()
            .any(|c| c.namespace != request.source.namespace)
        {
            return Err(SyncError::Invalid("candidate namespace mismatch".into()));
        }
        let total_bytes = canonical_event(event)?.len() as u64;
        let resident_bytes_reserved = total_bytes.saturating_mul(4);
        if resident_bytes_reserved > MAX_RECEIVER_EXPANDED_BYTES {
            return Err(SyncError::WorkLimit);
        }
        let limits = self.stages.origin_limits(origin);
        if total_bytes > limits.map_or(DEFAULT_LOGICAL_BYTES, |v| v.max_event_bytes) {
            return Err(SyncError::LogicalLimit);
        }
        Ok(PreparedEvent {
            origin: origin.clone(),
            event: event.clone(),
            event_stage: None,
            resident_bytes_reserved,
        })
    }
    pub fn completed_event(
        &self,
        stage: Uuid,
        key: &CheckpointKey,
        sequence: u64,
    ) -> Result<Option<ReceiveProgress>> {
        let Some((_, saved_key, saved_sequence, progress, _)) = self
            .completed_events
            .iter()
            .find(|(id, _, _, _, _)| *id == stage)
        else {
            return Ok(None);
        };
        if saved_key != key || *saved_sequence != sequence {
            return Err(SyncError::Invalid("completed stage origin mismatch".into()));
        }
        Ok(Some(progress.clone()))
    }
    /// Call after accepted event and checkpoint commit while holding its admission lock.
    pub fn finalize_event(
        &mut self,
        prepared: &PreparedEvent<M>,
        key: CheckpointKey,
        progress: ReceiveProgress,
        expires_unix_ms: u64,
    ) -> Result<()> {
        let StageOrigin::Journal {
            request,
            sequence,
            collection,
            event_digest: expected_digest,
        } = &prepared.origin
        else {
            return Err(SyncError::Invalid("event origin is not journal".into()));
        };
        let expected = CheckpointKey {
            namespace: request.source.namespace,
            source_replica: request.source.replica,
            source_generation: request.source.generation,
            scope_id: scope_id(&request.scope)?,
        };
        if key != expected
            || progress.sequence != *sequence
            || progress.anchor
                != next_anchor::<M>(
                    request.after.anchor,
                    *sequence,
                    collection,
                    *expected_digest,
                )?
            || event_digest(&prepared.event)? != *expected_digest
        {
            return Err(SyncError::Invalid("event completion mismatch".into()));
        }
        if let Some(stage) = prepared.event_stage {
            if self.completed_event(stage, &key, *sequence)?.is_some() {
                return Ok(());
            }
            self.reserve_retry_receipt();
            self.completed_events
                .push((stage, key, *sequence, progress, expires_unix_ms));
        }
        let id = self
            .descriptors
            .iter()
            .find_map(|(id, d)| (d.origin == prepared.origin).then_some(*id));
        if let Some(id) = id {
            self.abandon_stage(id)?
        }
        Ok(())
    }
}
