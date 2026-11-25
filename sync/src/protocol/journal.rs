//! Anchored scanned-link validation; page-size encoding and session I/O are separate.
use super::*;
/// Validate every scanned commitment, including scope-filtered links.
pub fn validate_scanned_links<M: SyncModel>(page: &ChangePage<M>) -> Result<()> {
    let req = &page.request;
    if req.scope.collections.is_empty()
        || req.scope.collections.len() > MAX_COLLECTIONS
        || req.limits.max_scanned == 0
        || req.limits.max_scanned > MAX_PAGE_ITEMS
        || req.limits.max_bytes == 0
        || req.limits.max_bytes > MAX_PAGE_BYTES
    {
        return Err(SyncError::Invalid("invalid page limits or scope".into()));
    }
    if req.source.model.model != M::model_id() || req.source.model.schema != M::schema_id() {
        return Err(SyncError::IncompatibleModel);
    }
    if req.after.sequence > page.next.sequence
        || page.next.sequence > req.through.sequence
        || req.through.sequence > MAX_NUMBER
    {
        return Err(SyncError::Invalid("invalid page positions".into()));
    }
    if page.done != (page.next.sequence == req.through.sequence)
        || (page.done && page.next.anchor != req.through.anchor)
    {
        return Err(SyncError::Invalid("invalid completion".into()));
    }
    let count = page.next.sequence - req.after.sequence;
    if count != page.links.len() as u64
        || count > req.limits.max_scanned as u64
        || (!page.done && count == 0)
    {
        return Err(SyncError::Invalid("incomplete scanned links".into()));
    }
    let initial = initial_anchor(&req.source)?;
    if req.after.sequence == 0 && req.after.anchor != initial {
        return Err(SyncError::Invalid("invalid initial anchor".into()));
    }
    let mut previous = req.after.anchor;
    for (index, link) in page.links.iter().enumerate() {
        let sequence = req
            .after
            .sequence
            .checked_add(index as u64 + 1)
            .ok_or(SyncError::ClockExhausted)?;
        if link.sequence != sequence {
            return Err(SyncError::Invalid("noncontiguous scanned links".into()));
        }
        let in_scope = req.scope.collections.contains(&link.collection);
        if in_scope != link.event.is_some() {
            return Err(SyncError::Invalid("scope/link mismatch".into()));
        }
        if let Some(EventRef::Inline(event)) = &link.event {
            let collection = match event {
                Event::Key(k) => &k.key.collection,
                Event::Merge(m) => &m.collection,
            };
            if collection != &link.collection || event_digest(event)? != link.event_digest {
                return Err(SyncError::Invalid("event commitment mismatch".into()));
            }
            if let Event::Key(k) = event
                && k.added.iter().any(|c| c.namespace != req.source.namespace)
            {
                return Err(SyncError::Invalid("candidate namespace mismatch".into()));
            }
            if let Event::Merge(m) = event
                && m.added.iter().any(|c| c.namespace != req.source.namespace)
            {
                return Err(SyncError::Invalid("candidate namespace mismatch".into()));
            }
        }
        previous = next_anchor::<M>(previous, sequence, &link.collection, link.event_digest)?;
    }
    if previous != page.next.anchor {
        return Err(SyncError::Invalid("anchor mismatch".into()));
    }
    Ok(())
}
