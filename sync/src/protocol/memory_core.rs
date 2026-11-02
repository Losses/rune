//! In-memory causal state used by the protocol memory store.
use super::*;
use crate::causal::{Candidate as CausalCandidate, CausalMap, Dot as CausalDot, Limits};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
type Rank = (Revision, Digest, CausalDot);
type Map<M> = CausalMap<Uuid, <M as SyncModel>::Payload, Rank>;
#[derive(Clone, Debug)]
pub struct JournalEntry<M: SyncModel> {
    pub sequence: u64,
    pub event: Event<M>,
    pub digest: Digest,
    pub anchor: Digest,
}
#[derive(Clone, Debug)]
pub struct MemoryCore<M: SyncModel> {
    pub identity: StoreIdentity,
    pub author: LocalAuthor,
    logical: u64,
    dot_counter: u64,
    collections: Arc<BTreeMap<M::Collection, Map<M>>>,
    journal: Arc<Vec<JournalEntry<M>>>,
}
fn error(e: anyhow::Error) -> SyncError {
    match e.downcast_ref::<crate::causal::CausalError>() {
        Some(crate::causal::CausalError::ConflictingDot) => SyncError::ConflictingDot,
        Some(crate::causal::CausalError::ClockExhausted) => SyncError::ClockExhausted,
        Some(crate::causal::CausalError::LogicalLimit(_)) => SyncError::LogicalLimit,
        _ => SyncError::Invalid(e.to_string()),
    }
}
fn causal_dot(d: Dot) -> CausalDot {
    CausalDot {
        writer: d.writer_incarnation,
        counter: d.counter,
    }
}
fn public_dot(d: CausalDot) -> Dot {
    Dot {
        writer_incarnation: d.writer,
        counter: d.counter,
    }
}
fn checked(n: u64) -> Result<u64> {
    n.checked_add(1)
        .filter(|n| *n <= MAX_NUMBER)
        .ok_or(SyncError::ClockExhausted)
}
impl<M: SyncModel> MemoryCore<M> {
    pub fn new(identity: StoreIdentity, author: LocalAuthor) -> Result<Self> {
        validate_model::<M>()?;
        initial_anchor(&identity)?;
        if identity.model.model != M::model_id()
            || identity.model.schema != M::schema_id()
            || author.author.is_nil()
            || author.writer_incarnation.is_nil()
        {
            return Err(SyncError::Invalid("invalid local identity".into()));
        }
        Ok(Self {
            identity,
            author,
            logical: 0,
            dot_counter: 0,
            collections: Arc::new(BTreeMap::new()),
            journal: Arc::new(Vec::new()),
        })
    }
    fn map(&self, collection: &M::Collection) -> Option<&Map<M>> {
        self.collections.get(collection)
    }
    fn map_mut(&mut self, collection: &M::Collection) -> Result<&mut Map<M>> {
        if !self.collections.contains_key(collection) {
            Arc::make_mut(&mut self.collections).insert(
                collection.clone(),
                CausalMap::new(Limits::default()).map_err(error)?,
            );
        }
        Ok(Arc::make_mut(&mut self.collections)
            .get_mut(collection)
            .unwrap())
    }
    pub fn head(&self) -> Result<ReceiveProgress> {
        if let Some(last) = self.journal.last() {
            Ok(ReceiveProgress {
                sequence: last.sequence,
                anchor: last.anchor,
            })
        } else {
            Ok(ReceiveProgress {
                sequence: 0,
                anchor: initial_anchor(&self.identity)?,
            })
        }
    }
    pub fn journal(&self) -> &[JournalEntry<M>] {
        &self.journal
    }
    pub fn export_collection(&self, collection: &M::Collection) -> Result<Vec<CausalSegment<M>>> {
        let segments = match self.map(collection) {
            Some(map) => map.export().map_err(error),
            None => CausalMap::new(Limits::default())
                .and_then(|map: Map<M>| map.export())
                .map_err(error),
        }?;
        Ok(segments
            .into_iter()
            .map(|segment| {
                let range = KeyRange {
                    collection: collection.clone(),
                    lower: segment.range.start,
                    upper: segment.range.end,
                };
                let candidates = segment
                    .candidates
                    .iter()
                    .map(|(id, candidate)| {
                        self.candidate_from(
                            &ObjectKey {
                                collection: collection.clone(),
                                id: *id,
                            },
                            candidate,
                        )
                    })
                    .collect();
                (range, exact_context(&segment.context), candidates)
            })
            .collect())
    }
    pub fn next_candidate(
        &self,
        range: &KeyRange<M>,
        after: Option<&CandidateCursor<M>>,
    ) -> Result<Option<Candidate<M>>> {
        let Some(map) = self.map(&range.collection) else {
            return Ok(None);
        };
        if after.is_some_and(|cursor| cursor.key.collection != range.collection) {
            return Err(SyncError::StaleWork);
        }
        let cursor = after.map(|cursor| (&cursor.key.id, causal_dot(cursor.dot)));
        Ok(map
            .next_live(cursor, range.lower.as_ref(), range.upper.as_ref())
            .map(|(id, c)| {
                self.candidate_from(
                    &ObjectKey {
                        collection: range.collection.clone(),
                        id,
                    },
                    &c,
                )
            }))
    }
    pub fn next_context_segment(
        &self,
        range: &KeyRange<M>,
        lower: Option<Uuid>,
    ) -> Result<(KeyRange<M>, ExactContext)> {
        let segment = match self.map(&range.collection) {
            Some(map) => map.next_segment(lower),
            None => CausalMap::<Uuid, M::Payload, Rank>::new(Limits::default())
                .map_err(error)?
                .next_segment(lower),
        };
        let upper = match (segment.range.end, range.upper) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, None) | (None, a) => a,
        };
        Ok((
            KeyRange {
                collection: range.collection.clone(),
                lower,
                upper,
            },
            exact_context(&segment.context),
        ))
    }
    fn candidate_from(
        &self,
        key: &ObjectKey<M>,
        c: &CausalCandidate<M::Payload, Rank>,
    ) -> Candidate<M> {
        Candidate {
            model: self.identity.model.clone(),
            namespace: self.identity.namespace,
            key: key.clone(),
            dot: public_dot(c.dot),
            revision: c.revision.0,
            payload: c.value.clone(),
        }
    }
    pub fn candidates(&self, key: &ObjectKey<M>) -> Vec<Candidate<M>> {
        self.map(&key.collection)
            .map(|m| {
                m.candidates(&key.id)
                    .map(|c| self.candidate_from(key, c))
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn winner(&self, key: &ObjectKey<M>) -> Option<Candidate<M>> {
        self.map(&key.collection)
            .and_then(|m| m.winner(&key.id))
            .map(|c| self.candidate_from(key, c))
    }
    fn validate_export(&self, collection: &M::Collection, keys: &BTreeSet<Uuid>) -> Result<()> {
        let Some(map) = self.map(collection) else {
            return Ok(());
        };
        // Reserve framing space for a complete single-key range and its manifests.
        // Scope tags are escaped JSON strings and can each expand sixfold.
        const FRAMING_RESERVE: u64 = (MAX_COLLECTIONS * MAX_TAG_BYTES * 6 + 8192) as u64;
        for segment in map.normalized().map_err(error)? {
            let context = exact_context(&segment.context);
            if (canonical_context(&context)?.len() as u64)
                .checked_mul(8)
                .and_then(|n| n.checked_add(FRAMING_RESERVE))
                .is_none_or(|n| n > MAX_RECEIVER_EXPANDED_BYTES)
            {
                return Err(SyncError::LogicalLimit);
            }
        }
        for id in keys {
            let key = ObjectKey {
                collection: collection.clone(),
                id: *id,
            };
            let candidates = self.candidates(&key);
            if candidates.len() as u64 > DEFAULT_LOGICAL_ITEMS {
                return Err(SyncError::LogicalLimit);
            }
            let context = exact_context(&map.effective_context(id).map_err(error)?);
            let context_bytes = canonical_context(&context)?.len() as u64;
            let candidate_bytes = canonical_candidates(&candidates)?.len() as u64;
            if context_bytes
                .checked_mul(8)
                .and_then(|n| {
                    candidate_bytes
                        .checked_mul(4)
                        .and_then(|c| n.checked_add(c))
                })
                .and_then(|n| n.checked_add(FRAMING_RESERVE))
                .is_none_or(|n| n > MAX_RECEIVER_EXPANDED_BYTES)
            {
                return Err(SyncError::LogicalLimit);
            }
        }
        Ok(())
    }
    fn append(&mut self, event: Event<M>) -> Result<()> {
        let digest = event_digest(&event)?;
        let collection = match &event {
            Event::Key(k) => &k.key.collection,
            Event::Range(r) => &r.range.collection,
        };
        let old = self.head()?;
        let sequence = checked(old.sequence)?;
        let anchor = next_anchor::<M>(old.anchor, sequence, collection, digest)?;
        Arc::make_mut(&mut self.journal).push(JournalEntry {
            sequence,
            event,
            digest,
            anchor,
        });
        Ok(())
    }
    /// One key delta; caller supplies already authenticated/ordered page context.
    fn apply_delta_inner(
        &mut self,
        delta: &KeyDelta<M>,
        relay: bool,
    ) -> Result<(bool, Option<ProjectionChange<M>>)> {
        canonical_event(&Event::Key(delta.clone()))?;
        let before = self.winner(&delta.key);
        if delta.author.counter == 0
            || delta.author.counter > MAX_NUMBER
            || delta.author.actor.is_nil()
        {
            return Err(SyncError::Invalid("invalid operation revision".into()));
        }
        self.logical = self.logical.max(delta.author.counter);
        let mut added = Vec::new();
        for candidate in &delta.added {
            if candidate.namespace != self.identity.namespace || candidate.key != delta.key {
                return Err(SyncError::Invalid(
                    "candidate key or namespace mismatch".into(),
                ));
            }
            validate_candidate(candidate)?;
            let (_, digest, _) = candidate_rank(candidate)?;
            self.logical = self.logical.max(candidate.revision.counter);
            let d = causal_dot(candidate.dot);
            added.push(CausalCandidate {
                dot: d,
                revision: (candidate.revision, digest, d),
                value: candidate.payload.clone(),
            });
        }
        let removed: Vec<_> = delta.removed.iter().copied().map(causal_dot).collect();
        let changed = self
            .map_mut(&delta.key.collection)?
            .merge_key_delta(delta.key.id, &removed, &added)
            .map_err(error)?;
        let after = self.winner(&delta.key);
        self.validate_export(&delta.key.collection, &[delta.key.id].into_iter().collect())?;
        if after.is_none() && (before.is_some() || !delta.added.is_empty()) {
            M::validate(&delta.key, &None)?;
        }
        if delta.added.is_empty() {
            M::validate(&delta.key, &None)?;
            M::validate_revision(&delta.key, &None, &delta.author)?;
        }
        if changed && relay {
            self.append(Event::Key(delta.clone()))?;
        }
        Ok((
            changed,
            if before != after {
                Some(ProjectionChange {
                    key: delta.key.clone(),
                    before,
                    after,
                })
            } else {
                None
            },
        ))
    }
    /// Copy/commit ensures hook, collision, limit and journal failures roll back.
    pub fn apply_delta(
        &mut self,
        delta: &KeyDelta<M>,
    ) -> Result<(bool, Option<ProjectionChange<M>>)> {
        let mut next = self.clone();
        let result = next.apply_delta_inner(delta, true)?;
        *self = next;
        Ok(result)
    }
    /// Apply already verified complete range dependencies against current state.
    /// The caller verifies that the supplied segments match the event manifests.
    pub fn apply_range(
        &mut self,
        event: &RangeEvent<M>,
        segments: &[CausalSegment<M>],
    ) -> Result<(bool, Vec<ProjectionChange<M>>)> {
        canonical_event(&Event::Range(Box::new(event.clone())))?;
        if event.source.source.namespace != self.identity.namespace {
            return Err(SyncError::Invalid("range namespace mismatch".into()));
        }
        if segments.is_empty() {
            return Err(SyncError::IncompleteCoverage);
        }
        let mut expected_lower = event.range.lower;
        let mut incoming =
            CausalMap::<Uuid, M::Payload, Rank>::new(Limits::default()).map_err(error)?;
        let mut keys = BTreeSet::new();
        let mut logical = self.logical;
        for (index, (bounds, context, candidates)) in segments.iter().enumerate() {
            if bounds.collection != event.range.collection
                || bounds.lower != expected_lower
                || (bounds.upper.is_none() && index + 1 != segments.len())
            {
                return Err(SyncError::Invalid("incomplete range partition".into()));
            }
            let key_range =
                crate::causal::KeyRange::new(bounds.lower, bounds.upper).map_err(error)?;
            let mut values = Vec::new();
            for candidate in candidates {
                if candidate.namespace != self.identity.namespace
                    || candidate.key.collection != bounds.collection
                    || !key_range.contains(&candidate.key.id)
                {
                    return Err(SyncError::Invalid(
                        "candidate outside range or namespace".into(),
                    ));
                }
                validate_candidate(candidate)?;
                let d = causal_dot(candidate.dot);
                let (_, digest, _) = candidate_rank(candidate)?;
                logical = logical.max(candidate.revision.counter);
                keys.insert(candidate.key.id);
                values.push((
                    candidate.key.id,
                    CausalCandidate {
                        dot: d,
                        revision: (candidate.revision, digest, d),
                        value: candidate.payload.clone(),
                    },
                ));
            }
            incoming
                .merge_range(&crate::causal::CompleteRange {
                    range: key_range,
                    context: causal_context(context)?,
                    candidates: values,
                })
                .map_err(error)?;
            expected_lower = bounds.upper;
        }
        if expected_lower != event.range.upper {
            return Err(SyncError::IncompleteCoverage);
        }
        for (range, _, candidates) in self.export_collection(&event.range.collection)? {
            for candidate in candidates {
                if event.range.lower.is_none_or(|x| candidate.key.id >= x)
                    && event.range.upper.is_none_or(|x| candidate.key.id < x)
                {
                    keys.insert(candidate.key.id);
                }
            }
            let _ = range;
        }
        let mut next = self.clone();
        let before: Vec<_> = keys
            .iter()
            .map(|id| {
                let key = ObjectKey {
                    collection: event.range.collection.clone(),
                    id: *id,
                };
                let winner = self.winner(&key);
                (key, winner)
            })
            .collect();
        let changed = next
            .map_mut(&event.range.collection)?
            .merge(&incoming)
            .map_err(error)?;
        next.validate_export(&event.range.collection, &keys)?;
        next.logical = logical;
        let mut projection = Vec::new();
        for (key, previous) in before {
            let after = next.winner(&key);
            if after.is_none() {
                M::validate(&key, &None)?;
            }
            if previous != after {
                projection.push(ProjectionChange {
                    key,
                    before: previous,
                    after,
                });
            }
        }
        if changed {
            next.append(Event::Range(Box::new(event.clone())))?;
        }
        *self = next;
        Ok((changed, projection))
    }
    pub fn write(
        &mut self,
        key: ObjectKey<M>,
        value: Option<M::Payload>,
        observed: Observed,
    ) -> Result<Record<M>> {
        let mut next = self.clone();
        let current = next.candidates(&key);
        let live: BTreeSet<_> = current.iter().map(|c| c.dot).collect();
        let selected = match observed {
            Observed::AllCurrent => live.clone(),
            Observed::Exact(s) => {
                if !s.is_subset(&live) {
                    return Err(SyncError::Invalid("observed dot not live".into()));
                }
                s
            }
        };
        let revision = Revision {
            counter: checked(next.logical)?,
            actor: next.author.author,
        };
        M::validate(&key, &value)?;
        M::validate_revision(&key, &value, &revision)?;
        next.logical = revision.counter;
        let mut added = Vec::new();
        if let Some(payload) = value.clone() {
            next.dot_counter = checked(next.dot_counter)?;
            let candidate = Candidate {
                model: next.identity.model.clone(),
                namespace: next.identity.namespace,
                key: key.clone(),
                dot: Dot {
                    writer_incarnation: next.author.writer_incarnation,
                    counter: next.dot_counter,
                },
                revision,
                payload,
            };
            validate_candidate(&candidate)?;
            added.push(candidate);
        }
        let delta = KeyDelta {
            key: key.clone(),
            author: revision,
            removed: selected.into_iter().collect(),
            added,
        };
        next.apply_delta_inner(&delta, true)?;
        *self = next;
        Ok(Record {
            key,
            value,
            revision,
        })
    }
}

#[cfg(test)]
mod causal_error_tests {
    use super::*;
    use crate::causal::CausalError;
    #[test]
    fn causal_errors_are_classified_by_type() {
        assert!(matches!(
            error(anyhow::Error::new(CausalError::ConflictingDot).context("different message")),
            SyncError::ConflictingDot
        ));
        assert!(matches!(
            error(CausalError::ClockExhausted.into()),
            SyncError::ClockExhausted
        ));
        assert!(matches!(
            error(CausalError::LogicalLimit("live limit").into()),
            SyncError::LogicalLimit
        ));
        assert!(matches!(
            error(anyhow::anyhow!("live limit")),
            SyncError::Invalid(_)
        ));
    }
}
