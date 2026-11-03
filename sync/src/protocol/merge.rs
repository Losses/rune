use super::*;
use crate::causal::Context;
use std::collections::BTreeMap;

/// Plan the collection-wide delta justified by a complete source leaf.
pub fn plan_leaf_merge<M: SyncModel>(
    collection: &M::Collection,
    current: &[Candidate<M>],
    incoming: &[Candidate<M>],
    source_context: &crate::causal::Context,
    locally_observed: &BTreeSet<Dot>,
) -> Result<Option<MergeDelta<M>>> {
    let current: BTreeMap<_, _> = current.iter().map(|c| (c.dot, c)).collect();
    let mut incoming_dots = BTreeSet::new();
    let mut added = Vec::new();
    for candidate in incoming {
        if candidate.key.collection != *collection {
            return Err(SyncError::Invalid("leaf collection mismatch".into()));
        }
        if !incoming_dots.insert(candidate.dot) {
            return Err(SyncError::ConflictingDot);
        }
        if let Some(existing) = current.get(&candidate.dot) {
            if *existing != candidate {
                return Err(SyncError::ConflictingDot);
            }
        } else if !locally_observed.contains(&candidate.dot) {
            added.push(candidate.clone());
        }
    }
    added.sort_by(|a, b| a.key.cmp(&b.key).then(a.dot.cmp(&b.dot)));
    let mut observed = Context::default();
    for dot in current.keys().filter(|d| !incoming_dots.contains(d)) {
        let dot = crate::causal::Dot {
            writer: dot.writer_incarnation,
            counter: dot.counter,
        };
        if source_context.contains(dot) {
            observed
                .insert(dot)
                .map_err(|e| SyncError::Invalid(e.to_string()))?;
        }
    }
    if added.is_empty() && observed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(MergeDelta {
            collection: collection.clone(),
            added,
            observed: exact_context(&observed),
        }))
    }
}
