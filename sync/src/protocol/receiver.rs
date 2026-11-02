//! Receiver-only staging and coverage. Accepted values remain in the adapter.
use super::*;
pub const MAX_RECEIVER_METADATA_ITEMS: usize = 4096;
pub const MAX_RECEIVER_METADATA_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_RECEIVER_RETRY_RECEIPTS: usize = 1024;
pub const MAX_RECEIVER_EXPANDED_BYTES: u64 = DEFAULT_LOGICAL_BYTES;

#[derive(Clone)]
pub(crate) struct PendingRange<M: SyncModel> {
    pub(crate) summary: RangeSummary<M>,
    pub(crate) contexts: Vec<ContextSegment<M>>,
    pub(crate) context_next: ContextCursor,
    pub(crate) context_done: bool,
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
    pub(crate) published_completion: Option<ReceiveProgress>,
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
pub struct PreparedRange<M: SyncModel> {
    pub token: RangeAdmission<M>,
    pub event: RangeEvent<M>,
    pub segments: Vec<CausalSegment<M>>,
    pub dependencies: Vec<(ManifestRef, Vec<u8>)>,
    pub dependency_fragments: Vec<(Digest, Vec<Fragment>)>,
    /// Conservative resident allocation reservation for decoded evidence.
    pub resident_bytes_reserved: u64,
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
        let stages: Vec<_> = self.descriptors.iter().filter_map(|(id, descriptor)| matches!(&descriptor.origin, StageOrigin::Snapshot { session: owner, .. } if *owner == session).then_some(*id)).collect();
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
        pending.contexts.clear();
        pending.context_next = ContextCursor {
            segment_index: 0,
            next_lower: range.lower,
        };
        pending.context_done = false;
        pending.candidates.clear();
        pending.leaf_next = None;
        pending.leaf_done = false;
        pending.staged_bytes = 0;
        let stages: Vec<_> = self.descriptors.iter().filter_map(|(id, d)| matches!(&d.origin, StageOrigin::Snapshot { session: owner, range: staged, .. } if *owner == session && staged == range).then_some(*id)).collect();
        for id in stages {
            if self.descriptors.iter().any(|(stage, _)| *stage == id) {
                self.abandon_stage(id)?;
            }
        }
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
            StageOrigin::Snapshot {
                session,
                snapshot_id,
                range,
            } => {
                let active = self
                    .sessions
                    .iter()
                    .find(|s| s.id == *session)
                    .ok_or(SyncError::StaleWork)?;
                let pending = active
                    .pending
                    .iter()
                    .find(|p| p.summary.range == *range)
                    .ok_or(SyncError::StaleWork)?;
                if active.remote.snapshot_id != *snapshot_id
                    || !pending
                        .contexts
                        .iter()
                        .any(|c| c.context.manifest == descriptor.manifest)
                {
                    return Err(SyncError::Invalid("invalid snapshot stage origin".into()));
                }
                if pending
                    .staged_bytes
                    .checked_add(self.stages.origin_reserved_bytes(&descriptor.origin))
                    .and_then(|n| n.checked_add(descriptor.manifest.encoded_bytes))
                    .is_none_or(|n| n > limits.max_event_bytes)
                {
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
                StageOrigin::Snapshot { session, .. }
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
                context_next: ContextCursor {
                    segment_index: 0,
                    next_lower: summary.range.lower,
                },
                summary,
                contexts: Vec::new(),
                context_done: false,
                candidates: Vec::new(),
                leaf_next: None,
                leaf_done: false,
                staged_bytes: 0,
            })
            .collect();
        self.sessions.push(Reconciliation {
            id,
            remote,
            local,
            pending,
            completed: Vec::new(),
            revision: 0,
            published_completion: None,
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
                .filter(|p| !p.contexts.is_empty() || !p.candidates.is_empty())
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
    pub fn context_progress(&self, session: Uuid, range: &KeyRange<M>) -> Result<ContextProgress> {
        let p = self
            .sessions
            .iter()
            .find(|s| s.id == session)
            .and_then(|s| s.pending.iter().find(|p| &p.summary.range == range))
            .ok_or(SyncError::StaleWork)?;
        Ok(ContextProgress {
            next: p.context_next.clone(),
            complete: p.context_done,
        })
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
        if !old.contexts.is_empty()
            || !old.candidates.is_empty()
            || old.context_done
            || old.leaf_done
        {
            return Err(SyncError::Invalid("range already staged".into()));
        }
        let revision = active
            .revision
            .checked_add(1)
            .ok_or(SyncError::ClockExhausted)?;
        active.pending.splice(
            index..=index,
            children.into_iter().map(|summary| PendingRange {
                context_next: ContextCursor {
                    segment_index: 0,
                    next_lower: summary.range.lower,
                },
                summary,
                contexts: Vec::new(),
                context_done: false,
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
            || !p.contexts.is_empty()
            || !p.candidates.is_empty()
            || p.context_done
            || p.leaf_done
            || local.aggregate != source.aggregate
            || local.context_commitment != source.context_commitment
            || local.candidate_count != source.candidate_count
            || local.candidate_bytes != source.candidate_bytes
            || local.context_segments != source.context_segments
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
    pub fn stage_context_page(
        &mut self,
        session: Uuid,
        page: ContextPage<M>,
    ) -> Result<ContextProgress> {
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
        let origin = StageOrigin::Snapshot {
            session,
            snapshot_id: page.snapshot_id,
            range: page.range.clone(),
        };
        let dependency_bytes = self.stages.origin_reserved_bytes(&origin);
        let graph_limits = self.stages.origin_limits(&origin);
        let active = self
            .sessions
            .iter_mut()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?;
        if page.snapshot_id != active.remote.snapshot_id {
            return Err(SyncError::StaleWork);
        }
        let p = active
            .pending
            .iter_mut()
            .find(|p| p.summary.range == page.range)
            .ok_or(SyncError::StaleWork)?;
        let start = p.contexts.len().saturating_sub(page.segments.len());
        let prior = if start == 0 {
            page.range.lower
        } else {
            p.contexts[start - 1].range.upper
        };
        if (!page.segments.is_empty() || page.done)
            && p.context_next == page.next
            && p.context_done == page.done
            && p.contexts.ends_with(&page.segments)
            && page.after.segment_index == start as u64
            && page.after.next_lower == prior
        {
            return Ok(ContextProgress {
                next: p.context_next.clone(),
                complete: p.context_done,
            });
        }
        if total
            .checked_add(page_bytes)
            .is_none_or(|n| n > graph_limits.map_or(MAX_STAGE_BYTES, |v| v.max_total_bytes))
        {
            return Err(SyncError::StageQuota);
        }
        if p.staged_bytes
            .checked_add(page_bytes)
            .and_then(|n| n.checked_add(dependency_bytes))
            .is_none_or(|n| n > graph_limits.map_or(MAX_LOGICAL_BYTES, |v| v.max_event_bytes))
        {
            return Err(SyncError::LogicalLimit);
        }
        if p.context_done
            || p.context_next != page.after
            || page
                .after
                .segment_index
                .checked_add(page.segments.len() as u64)
                != Some(page.next.segment_index)
            || (page.segments.is_empty() && !page.done)
        {
            return Err(SyncError::Invalid("invalid context page".into()));
        }
        let mut lower = page.after.next_lower;
        for segment in &page.segments {
            if segment.range.collection != page.range.collection
                || segment.range.lower != lower
                || segment.context.manifest.kind != ManifestKind::Context
                || manifest_digest(&segment.context.manifest)?
                    != segment.context.manifest.manifest_digest
            {
                return Err(SyncError::Invalid("invalid context segment".into()));
            }
            lower = segment.range.upper;
        }
        if page.next.next_lower != lower
            || (page.done && lower != page.range.upper)
            || p.contexts.len() + page.segments.len() > MAX_LOGICAL_ITEMS as usize
        {
            return Err(SyncError::Invalid("incomplete context coverage".into()));
        }
        p.contexts.extend(page.segments);
        p.staged_bytes += page_bytes;
        p.context_next = page.next;
        p.context_done = page.done;
        Ok(ContextProgress {
            next: p.context_next.clone(),
            complete: p.context_done,
        })
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
        let origin = StageOrigin::Snapshot {
            session,
            snapshot_id: page.snapshot_id,
            range: page.range.clone(),
        };
        let dependency_bytes = self.stages.origin_reserved_bytes(&origin);
        let graph_limits = self.stages.origin_limits(&origin);
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
            .is_none_or(|n| n > graph_limits.map_or(MAX_STAGE_BYTES, |v| v.max_total_bytes))
        {
            return Err(SyncError::StageQuota);
        }
        if p.staged_bytes
            .checked_add(page_bytes)
            .and_then(|n| n.checked_add(dependency_bytes))
            .is_none_or(|n| n > graph_limits.map_or(MAX_LOGICAL_BYTES, |v| v.max_event_bytes))
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
fn local_manifest(kind: ManifestKind, bytes: &[u8], items: u64) -> Result<ManifestRef> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_LOGICAL_BYTES || items > MAX_LOGICAL_ITEMS {
        return Err(SyncError::LogicalLimit);
    }
    let mut m = ManifestRef {
        kind,
        manifest_digest: [0; 32],
        content_digest: content_digest(bytes)?,
        encoded_bytes: bytes.len() as u64,
        item_count: items,
        fragment_count: u32::try_from(bytes.len().div_ceil(64 * 1024))
            .map_err(|_| SyncError::LogicalLimit)?,
    };
    if m.fragment_count > MAX_FRAGMENTS {
        return Err(SyncError::LogicalLimit);
    }
    m.manifest_digest = manifest_digest(&m)?;
    Ok(m)
}
impl<M: SyncModel> ReceiverLedger<M> {
    /// Prepare and validate the whole immutable logical range without mutating coverage.
    pub fn prepare_range(&self, session: Uuid, range: &KeyRange<M>) -> Result<PreparedRange<M>> {
        let active = self
            .sessions
            .iter()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?;
        let p = active
            .pending
            .iter()
            .find(|p| &p.summary.range == range)
            .ok_or(SyncError::StaleWork)?;
        if !p.context_done || !p.leaf_done {
            return Err(SyncError::IncompleteCoverage);
        }
        let partition = canonical_context_partition(range, &p.contexts)?;
        let live = canonical_candidates(&p.candidates)?;
        let expanded = p.contexts.iter().try_fold(
            (live.len() as u64)
                .saturating_add(partition.len() as u64)
                .saturating_mul(4),
            |total, segment| {
                total
                    .checked_add(
                        segment
                            .context
                            .manifest
                            .encoded_bytes
                            .checked_mul(8)
                            .ok_or(SyncError::LogicalLimit)?,
                    )
                    .ok_or(SyncError::LogicalLimit)
            },
        )?;
        if expanded > MAX_RECEIVER_EXPANDED_BYTES {
            return Err(SyncError::WorkLimit);
        }
        let mut context_values = Vec::new();
        let mut dependencies = Vec::new();
        let mut dependency_fragments = Vec::new();
        for segment in &p.contexts {
            let (id,_)=self.descriptors.iter().find(|(_,d)|d.manifest==segment.context.manifest&&matches!(&d.origin,StageOrigin::Snapshot {session:id,snapshot_id,range:r} if *id==session&&*snapshot_id==active.remote.snapshot_id&&r==range)).ok_or(SyncError::MissingDependency(segment.context.manifest.manifest_digest))?;
            let bytes = self.stages.assembled(*id)?;
            let context = decode_context(&bytes)?;
            if context_digest(&context)? != segment.context.digest {
                return Err(SyncError::Invalid("context digest mismatch".into()));
            }
            context_values.push(context);
            dependencies.push((segment.context.manifest.clone(), bytes));
            if !dependency_fragments
                .iter()
                .any(|(digest, _)| *digest == segment.context.manifest.manifest_digest)
            {
                dependency_fragments.push((
                    segment.context.manifest.manifest_digest,
                    self.stages.fragments(*id)?,
                ));
            }
        }
        let mut dot_keys = std::collections::BTreeMap::new();
        let candidate_bytes = p.candidates.iter().try_fold(0u64, |n, c| {
            if c.namespace != active.remote.source.namespace {
                return Err(SyncError::Invalid("candidate namespace mismatch".into()));
            }
            if dot_keys.insert(c.dot, c.key.clone()).is_some() {
                return Err(SyncError::ConflictingDot);
            }
            n.checked_add(canonical_candidate(c)?.len() as u64)
                .ok_or(SyncError::LogicalLimit)
        })?;
        let mut hash = RangeHasher::new(
            &active.remote.scope,
            range.clone(),
            p.candidates.len() as u64,
            candidate_bytes,
            p.contexts.len() as u64,
        )?;
        for context in &p.contexts {
            hash.push_segment(&context.range, context.context.digest)?
        }
        for candidate in &p.candidates {
            hash.push_candidate(candidate)?
        }
        let (aggregate, context_commitment) = hash.finish_with_context()?;
        if aggregate != p.summary.aggregate
            || context_commitment != p.summary.context_commitment
            || p.summary.candidate_count != p.candidates.len() as u64
            || p.summary.candidate_bytes != candidate_bytes
            || p.summary.context_segments != p.contexts.len() as u64
        {
            return Err(SyncError::Invalid("range commitment mismatch".into()));
        }
        let partition_manifest = local_manifest(
            ManifestKind::ContextPartition,
            &partition,
            p.contexts.len() as u64,
        )?;
        let live_manifest = local_manifest(ManifestKind::Leaf, &live, p.candidates.len() as u64)?;
        dependencies.push((partition_manifest.clone(), partition));
        dependencies.push((live_manifest.clone(), live));
        let event = RangeEvent {
            range: range.clone(),
            scope: active.remote.scope.clone(),
            context_partition: partition_manifest,
            source: SnapshotRef {
                source: active.remote.source.clone(),
                snapshot_id: active.remote.snapshot_id,
                head: active.remote.head.clone(),
            },
            range_summary: aggregate,
            live_manifest,
        };
        let origin = StageOrigin::Snapshot {
            session,
            snapshot_id: active.remote.snapshot_id,
            range: range.clone(),
        };
        let limits = self.stages.origin_limits(&origin);
        let mut bytes = canonical_event(&Event::Range(Box::new(event.clone())))?.len() as u64;
        let mut items = 1u64;
        let mut counted = std::collections::BTreeSet::new();
        for (manifest, _) in &dependencies {
            if counted.insert(manifest.manifest_digest) {
                bytes = bytes
                    .checked_add(manifest.encoded_bytes)
                    .ok_or(SyncError::LogicalLimit)?;
                items = items
                    .checked_add(manifest.item_count)
                    .ok_or(SyncError::LogicalLimit)?;
            }
        }
        if bytes > limits.map_or(DEFAULT_LOGICAL_BYTES, |v| v.max_event_bytes)
            || items > limits.map_or(DEFAULT_LOGICAL_ITEMS, |v| v.max_event_items)
        {
            return Err(SyncError::LogicalLimit);
        }
        let segments: Vec<_> = p
            .contexts
            .iter()
            .zip(context_values)
            .map(|(segment, context)| {
                let values = p
                    .candidates
                    .iter()
                    .filter(|c| {
                        segment.range.lower.is_none_or(|v| c.key.id >= v)
                            && segment.range.upper.is_none_or(|v| c.key.id < v)
                    })
                    .cloned()
                    .collect();
                (segment.range.clone(), context, values)
            })
            .collect();
        validate_live_contexts(&segments)?;
        Ok(PreparedRange {
            token: RangeAdmission {
                session,
                range: range.clone(),
                ledger_revision: active.revision,
                summary: p.summary.clone(),
            },
            event,
            segments,
            dependencies,
            dependency_fragments,
            resident_bytes_reserved: expanded,
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
        active.pending.retain(|p| p.summary.range != token.range);
        active.completed.push(token.summary.clone());
        active.revision = revision;
        let stage=self.descriptors.iter().find_map(|(id,d)|matches!(&d.origin,StageOrigin::Snapshot {session,range,..} if *session==token.session&&*range==token.range).then_some(*id));
        if let Some(stage) = stage {
            self.abandon_stage(stage)?
        }
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
    /// Predict complete coverage without changing the ledger before SQL commit.
    pub fn prepare_completion_after_range(
        &self,
        token: &RangeAdmission<M>,
    ) -> Result<Option<CompletionAdmission>> {
        let active = self
            .sessions
            .iter()
            .find(|s| s.id == token.session)
            .ok_or(SyncError::StaleWork)?;
        if active.revision != token.ledger_revision
            || !active.pending.iter().any(|pending| {
                pending.summary == token.summary && pending.summary.range == token.range
            })
        {
            return Err(SyncError::StaleWork);
        }
        if active.pending.len() != 1 {
            return Ok(None);
        }
        Ok(Some(CompletionAdmission {
            session: token.session,
            ledger_revision: active
                .revision
                .checked_add(1)
                .ok_or(SyncError::ClockExhausted)?,
            checkpoint: CheckpointKey {
                namespace: active.remote.source.namespace,
                source_replica: active.remote.source.replica,
                source_generation: active.remote.source.generation,
                scope_id: scope_id(&active.remote.scope)?,
            },
            through: active.remote.head.clone(),
            expires_unix_ms: active.remote.expires_unix_ms,
        }))
    }
    /// Record a checkpoint committed with the last range while retaining the
    /// completed coverage receipt for normalization and response retries.
    pub fn mark_completion_published(
        &mut self,
        token: &CompletionAdmission,
        progress: ReceiveProgress,
    ) -> Result<()> {
        if self.prepare_completion(token.session)? != *token || progress != token.through {
            return Err(SyncError::StaleWork);
        }
        self.sessions
            .iter_mut()
            .find(|s| s.id == token.session)
            .ok_or(SyncError::StaleWork)?
            .published_completion = Some(progress);
        Ok(())
    }
    pub fn published_completion(&self, session: Uuid) -> Result<Option<ReceiveProgress>> {
        Ok(self
            .sessions
            .iter()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?
            .published_completion
            .clone())
    }
    pub fn prepare_completion(&self, session: Uuid) -> Result<CompletionAdmission> {
        let active = self
            .sessions
            .iter()
            .find(|s| s.id == session)
            .ok_or(SyncError::StaleWork)?;
        if !active.pending.is_empty() {
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
        self.sessions.retain(|s| s.id != token.session);
        self.reserve_retry_receipt();
        self.completed_sessions
            .push((token.session, committed.clone(), token.expires_unix_ms));
        Ok(committed)
    }
}
/// Complete event and its verified transitive range dependencies, ready for SQL admission.
#[derive(Clone, Debug)]
pub struct PreparedEvent<M: SyncModel> {
    pub origin: StageOrigin<M>,
    pub event: Event<M>,
    pub segments: Vec<CausalSegment<M>>,
    pub dependencies: Vec<(ManifestRef, Vec<u8>)>,
    pub dependency_fragments: Vec<(Digest, Vec<Fragment>)>,
    pub event_stage: Option<Uuid>,
    pub resident_bytes_reserved: u64,
}
impl<M: SyncModel> ReceiverLedger<M> {
    fn journal_dependency(
        &self,
        origin: &StageOrigin<M>,
        manifest: &ManifestRef,
    ) -> Result<(Vec<u8>, Vec<Fragment>)> {
        let (id, _) = self
            .descriptors
            .iter()
            .find(|(_, d)| d.origin == *origin && d.manifest == *manifest)
            .ok_or(SyncError::MissingDependency(manifest.manifest_digest))?;
        Ok((self.stages.assembled(*id)?, self.stages.fragments(*id)?))
    }
    pub fn prepare_event(&self, stage: Uuid) -> Result<PreparedEvent<M>> {
        let descriptor = self
            .descriptors
            .iter()
            .find(|(id, _)| *id == stage)
            .map(|(_, d)| d)
            .ok_or(SyncError::StaleWork)?;
        if !matches!(
            descriptor.manifest.kind,
            ManifestKind::KeyEvent | ManifestKind::RangeEvent
        ) {
            return Err(SyncError::Invalid("stage does not contain an event".into()));
        }
        let event = decode_event::<M>(&self.stages.assembled(stage)?)?;
        if !matches!(
            (&descriptor.manifest.kind, &event),
            (ManifestKind::KeyEvent, Event::Key(_)) | (ManifestKind::RangeEvent, Event::Range(_))
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
            Event::Range(range) => &range.range.collection,
        };
        if event_collection != collection {
            return Err(SyncError::Invalid("event collection mismatch".into()));
        }
        let mut dependencies = Vec::new();
        let mut dependency_fragments = Vec::new();
        let mut segments = Vec::new();
        let mut resident_bytes_reserved = (canonical_event(event)?.len() as u64).saturating_mul(4);
        match event {
            Event::Key(key) => {
                if key
                    .added
                    .iter()
                    .any(|c| c.namespace != request.source.namespace)
                {
                    return Err(SyncError::Invalid("candidate namespace mismatch".into()));
                }
            }
            Event::Range(range) => {
                if range.source.source.namespace != request.source.namespace {
                    return Err(SyncError::Invalid("range namespace mismatch".into()));
                }
                let (partition, fragments) =
                    self.journal_dependency(origin, &range.context_partition)?;
                let refs = decode_context_partition(&range.range, &partition)?;
                let partition_len = partition.len() as u64;
                dependencies.push((range.context_partition.clone(), partition));
                dependency_fragments.push((range.context_partition.manifest_digest, fragments));
                let (live, fragments) = self.journal_dependency(origin, &range.live_manifest)?;
                let candidates = decode_candidates::<M>(&live)?;
                let expanded = refs.iter().try_fold(
                    partition_len
                        .saturating_add(live.len() as u64)
                        .saturating_mul(4),
                    |total, segment| {
                        total
                            .checked_add(
                                segment
                                    .context
                                    .manifest
                                    .encoded_bytes
                                    .checked_mul(8)
                                    .ok_or(SyncError::LogicalLimit)?,
                            )
                            .ok_or(SyncError::LogicalLimit)
                    },
                )?;
                if expanded > MAX_RECEIVER_EXPANDED_BYTES {
                    return Err(SyncError::WorkLimit);
                }
                resident_bytes_reserved = expanded;
                dependencies.push((range.live_manifest.clone(), live));
                dependency_fragments.push((range.live_manifest.manifest_digest, fragments));
                let mut keys = std::collections::BTreeMap::new();
                let candidate_bytes = candidates.iter().try_fold(0u64, |n, c| {
                    if c.namespace != request.source.namespace {
                        return Err(SyncError::Invalid("candidate namespace mismatch".into()));
                    }
                    if keys.insert(c.dot, c.key.clone()).is_some() {
                        return Err(SyncError::ConflictingDot);
                    }
                    n.checked_add(canonical_candidate(c)?.len() as u64)
                        .ok_or(SyncError::LogicalLimit)
                })?;
                let mut hash = RangeHasher::new(
                    &range.scope,
                    range.range.clone(),
                    candidates.len() as u64,
                    candidate_bytes,
                    refs.len() as u64,
                )?;
                for segment in refs {
                    let (bytes, fragments) =
                        self.journal_dependency(origin, &segment.context.manifest)?;
                    let context = decode_context(&bytes)?;
                    if context_digest(&context)? != segment.context.digest {
                        return Err(SyncError::Invalid("context digest mismatch".into()));
                    }
                    hash.push_segment(&segment.range, segment.context.digest)?;
                    let values = candidates
                        .iter()
                        .filter(|c| {
                            segment.range.lower.is_none_or(|v| c.key.id >= v)
                                && segment.range.upper.is_none_or(|v| c.key.id < v)
                        })
                        .cloned()
                        .collect();
                    segments.push((segment.range, context, values));
                    dependencies.push((segment.context.manifest.clone(), bytes));
                    dependency_fragments
                        .push((segment.context.manifest.manifest_digest, fragments));
                }
                for candidate in &candidates {
                    hash.push_candidate(candidate)?
                }
                validate_live_contexts(&segments)?;
                if hash.finish()? != range.range_summary {
                    return Err(SyncError::Invalid("range commitment mismatch".into()));
                }
            }
        }
        let limits = self.stages.origin_limits(origin);
        let mut total_bytes = canonical_event(event)?.len() as u64;
        let mut total_items = 1u64;
        let mut counted = std::collections::BTreeSet::new();
        for (manifest, _) in &dependencies {
            if counted.insert(manifest.manifest_digest) {
                total_bytes = total_bytes
                    .checked_add(manifest.encoded_bytes)
                    .ok_or(SyncError::LogicalLimit)?;
                total_items = total_items
                    .checked_add(manifest.item_count)
                    .ok_or(SyncError::LogicalLimit)?;
            }
        }
        if total_bytes > limits.map_or(DEFAULT_LOGICAL_BYTES, |v| v.max_event_bytes)
            || total_items > limits.map_or(DEFAULT_LOGICAL_ITEMS, |v| v.max_event_items)
        {
            return Err(SyncError::LogicalLimit);
        }
        Ok(PreparedEvent {
            origin: origin.clone(),
            event: event.clone(),
            segments,
            dependencies,
            dependency_fragments,
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
fn validate_live_contexts<M: SyncModel>(segments: &[CausalSegment<M>]) -> Result<()> {
    for (_, context, candidates) in segments {
        let exact = causal_context(context)?;
        for candidate in candidates {
            let dot = crate::causal::Dot {
                writer: candidate.dot.writer_incarnation,
                counter: candidate.dot.counter,
            };
            if !exact.contains(dot) {
                return Err(SyncError::Invalid(
                    "live candidate absent from source context".into(),
                ));
            }
        }
    }
    Ok(())
}
