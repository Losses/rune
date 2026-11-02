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
        Arc::make_mut(&mut self.collections)
            .get_mut(collection)
            .ok_or(SyncError::Invalid("missing causal collection".into()))
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
    pub fn base(&self, collection: &M::Collection) -> crate::causal::Context {
        self.map(collection)
            .map(|m| m.base().clone())
            .unwrap_or_default()
    }
    pub fn live(&self, collection: &M::Collection) -> Vec<Candidate<M>> {
        let Some(map) = self.map(collection) else {
            return Vec::new();
        };
        let mut after = None;
        let mut live = Vec::new();
        while let Some((id, candidate)) =
            map.next_live(after.as_ref().map(|(key, dot)| (key, *dot)), None, None)
        {
            after = Some((id, candidate.dot));
            live.push(self.candidate_from(
                &ObjectKey {
                    collection: collection.clone(),
                    id,
                },
                &candidate,
            ));
        }
        live
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
        // Reserve framing space for a collection context and a complete single-key leaf.
        // Scope tags are escaped JSON strings and can each expand sixfold.
        const FRAMING_RESERVE: u64 = (MAX_COLLECTIONS * MAX_TAG_BYTES * 6 + 8192) as u64;
        for id in keys {
            let key = ObjectKey {
                collection: collection.clone(),
                id: *id,
            };
            let candidates = self.candidates(&key);
            if candidates.len() as u64 > DEFAULT_LOGICAL_ITEMS {
                return Err(SyncError::LogicalLimit);
            }
            let context = exact_context(map.base());
            let context_bytes = canonical_context(&context)?.len() as u64;
            let candidate_bytes = candidates.iter().try_fold(0u64, |n, c| {
                n.checked_add(canonical_candidate(c)?.len() as u64)
                    .ok_or(SyncError::LogicalLimit)
            })?;
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
            Event::Merge(m) => &m.collection,
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
    /// Apply and relay a collection-wide merge atomically.
    pub fn apply_merge(
        &mut self,
        delta: &MergeDelta<M>,
    ) -> Result<(bool, Vec<ProjectionChange<M>>)> {
        canonical_event(&Event::Merge(Box::new(delta.clone())))?;
        let observed = causal_context(&delta.observed)?;
        let mut keys: BTreeSet<_> = delta.added.iter().map(|c| c.key.id).collect();
        for candidate in self.live(&delta.collection) {
            if observed.contains(causal_dot(candidate.dot)) {
                keys.insert(candidate.key.id);
            }
        }
        let mut values = Vec::with_capacity(delta.added.len());
        let mut logical = self.logical;
        for c in &delta.added {
            if c.namespace != self.identity.namespace {
                return Err(SyncError::Invalid("candidate namespace mismatch".into()));
            }
            validate_candidate(c)?;
            let (_, digest, _) = candidate_rank(c)?;
            let dot = causal_dot(c.dot);
            values.push((
                c.key.id,
                CausalCandidate {
                    dot,
                    revision: (c.revision, digest, dot),
                    value: c.payload.clone(),
                },
            ));
            logical = logical.max(c.revision.counter);
        }
        let before: Vec<_> = keys
            .iter()
            .map(|id| {
                let key = ObjectKey {
                    collection: delta.collection.clone(),
                    id: *id,
                };
                let winner = self.winner(&key);
                (key, winner)
            })
            .collect();
        let mut next = self.clone();
        let changed = next
            .map_mut(&delta.collection)?
            .apply_merge(&values, &observed)
            .map_err(error)?;
        next.validate_export(&delta.collection, &keys)?;
        let mut changes = Vec::new();
        for (key, before) in before {
            let after = next.winner(&key);
            if after.is_none() {
                M::validate(&key, &None)?;
            }
            if before != after {
                changes.push(ProjectionChange { key, before, after });
            }
        }
        next.logical = logical;
        if changed {
            next.append(Event::Merge(Box::new(delta.clone())))?;
        }
        *self = next;
        Ok((changed, changes))
    }
    pub fn merge_leaf(
        &mut self,
        range: &KeyRange<M>,
        incoming: &[Candidate<M>],
        source_context: &crate::causal::Context,
    ) -> Result<(bool, Vec<ProjectionChange<M>>)> {
        let mut current = Vec::new();
        let mut after = None;
        while let Some(candidate) = self.next_candidate(range, after.as_ref())? {
            after = Some(CandidateCursor {
                key: candidate.key.clone(),
                dot: candidate.dot,
            });
            current.push(candidate);
        }
        let locally_observed = incoming
            .iter()
            .filter(|c| {
                self.map(&range.collection)
                    .is_some_and(|map| map.base().contains(causal_dot(c.dot)))
            })
            .map(|c| c.dot)
            .collect();
        match plan_leaf_merge(
            &range.collection,
            &current,
            incoming,
            source_context,
            &locally_observed,
        )? {
            Some(delta) => self.apply_merge(&delta),
            None => Ok((false, Vec::new())),
        }
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
