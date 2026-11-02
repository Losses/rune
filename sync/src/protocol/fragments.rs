//! Bounded fragment staging. Acceptance and checkpoint advancement are separate.
use super::*;
use std::collections::BTreeMap;
/// Deterministic half-open content interval for an immutable manifest fragment.
/// Chunks use ceil(encoded_bytes / fragment_count); layouts with an empty final
/// chunk are invalid and must not be published.
pub fn fragment_bounds(manifest: &ManifestRef, ordinal: u32) -> Result<(u64, u64)> {
    if manifest.encoded_bytes == 0
        || manifest.fragment_count == 0
        || manifest.fragment_count > MAX_FRAGMENTS
        || ordinal >= manifest.fragment_count
    {
        return Err(SyncError::Invalid("invalid fragment layout".into()));
    }
    let count = u64::from(manifest.fragment_count);
    let chunk = manifest.encoded_bytes.div_ceil(count);
    let last_start = (count - 1)
        .checked_mul(chunk)
        .ok_or(SyncError::LogicalLimit)?;
    if last_start >= manifest.encoded_bytes {
        return Err(SyncError::Invalid("empty final fragment".into()));
    }
    let start = u64::from(ordinal)
        .checked_mul(chunk)
        .ok_or(SyncError::LogicalLimit)?;
    Ok((
        start,
        start
            .checked_add(chunk)
            .ok_or(SyncError::LogicalLimit)?
            .min(manifest.encoded_bytes),
    ))
}
fn lp(v: &mut Vec<u8>, s: &[u8]) -> Result<()> {
    let n = u32::try_from(s.len()).map_err(|_| SyncError::LogicalLimit)?;
    v.extend(n.to_be_bytes());
    v.extend(s);
    Ok(())
}
fn kind(k: ManifestKind) -> &'static str {
    match k {
        ManifestKind::KeyEvent => "key_event",
        ManifestKind::MergeEvent => "merge_event",
        ManifestKind::Context => "context",
        ManifestKind::ContextRoot => "context_root",
    }
}
pub fn content_digest(bytes: &[u8]) -> Result<Digest> {
    let mut v = Vec::new();
    lp(&mut v, b"sync.manifest.content")?;
    lp(&mut v, bytes)?;
    Ok(*blake3::hash(&v).as_bytes())
}
pub fn manifest_digest(m: &ManifestRef) -> Result<Digest> {
    let mut v = Vec::new();
    lp(&mut v, b"sync.manifest")?;
    lp(&mut v, kind(m.kind).as_bytes())?;
    v.extend(m.encoded_bytes.to_be_bytes());
    v.extend(m.item_count.to_be_bytes());
    v.extend((m.fragment_count as u64).to_be_bytes());
    v.extend(m.content_digest);
    Ok(*blake3::hash(&v).as_bytes())
}
pub fn fragment_digest(f: &Fragment) -> Result<Digest> {
    let mut v = Vec::new();
    lp(&mut v, b"sync.fragment")?;
    v.extend(f.manifest.manifest_digest);
    v.extend((f.ordinal as u64).to_be_bytes());
    v.extend(f.offset.to_be_bytes());
    lp(&mut v, &f.bytes)?;
    Ok(*blake3::hash(&v).as_bytes())
}
// Collection contexts share a snapshot session slot; size limits remain per collection.
fn same_admission<M: SyncModel>(a: &StageOrigin<M>, b: &StageOrigin<M>) -> bool {
    match (a, b) {
        (
            StageOrigin::SnapshotContext {
                session: a,
                snapshot_id: x,
                ..
            },
            StageOrigin::SnapshotContext {
                session: b,
                snapshot_id: y,
                ..
            },
        ) => a == b && x == y,
        _ => a == b,
    }
}
#[derive(Clone)]
struct Stage<M: SyncModel> {
    descriptor: StageDescriptor<M>,
    limits: StageLimits,
    fragments: BTreeMap<u32, Fragment>,
}
#[derive(Clone)]
pub struct FragmentStages<M: SyncModel> {
    stages: BTreeMap<Uuid, Stage<M>>,
    reserved: u64,
    config: StageLimits,
}
impl<M: SyncModel> Default for FragmentStages<M> {
    fn default() -> Self {
        Self {
            stages: BTreeMap::new(),
            reserved: 0,
            config: StageLimits {
                max_total_bytes: DEFAULT_STAGE_BYTES,
                max_stages: DEFAULT_STAGES,
                max_event_bytes: DEFAULT_LOGICAL_BYTES,
                max_event_items: DEFAULT_LOGICAL_ITEMS,
                max_fragment_bytes: DEFAULT_PAGE_BYTES,
            },
        }
    }
}
impl<M: SyncModel> FragmentStages<M> {
    pub fn reserved_bytes(&self) -> u64 {
        self.reserved
    }
    pub fn limits(&self, id: Uuid) -> Result<StageLimits> {
        Ok(self.stages.get(&id).ok_or(SyncError::StaleWork)?.limits)
    }
    pub fn origin_reserved_bytes(&self, origin: &StageOrigin<M>) -> u64 {
        self.stages
            .values()
            .filter(|stage| &stage.descriptor.origin == origin)
            .map(|stage| stage.descriptor.manifest.encoded_bytes)
            .sum()
    }
    pub fn origin_limits(&self, origin: &StageOrigin<M>) -> Option<StageLimits> {
        self.stages
            .values()
            .find(|stage| &stage.descriptor.origin == origin)
            .map(|stage| stage.limits)
    }
    pub fn with_limits(config: StageLimits) -> Result<Self> {
        if config.max_total_bytes == 0
            || config.max_total_bytes > MAX_STAGE_BYTES
            || config.max_stages == 0
            || config.max_stages > MAX_STAGES
            || config.max_event_bytes == 0
            || config.max_event_bytes > MAX_LOGICAL_BYTES
            || config.max_event_items == 0
            || config.max_event_items > MAX_LOGICAL_ITEMS
            || config.max_fragment_bytes == 0
            || config.max_fragment_bytes > MAX_PAGE_BYTES
        {
            return Err(SyncError::Invalid("invalid store stage limits".into()));
        }
        Ok(Self {
            stages: BTreeMap::new(),
            reserved: 0,
            config,
        })
    }
    pub fn begin(
        &mut self,
        id: Uuid,
        descriptor: StageDescriptor<M>,
        limits: StageLimits,
    ) -> Result<FragmentProgress> {
        let m = &descriptor.manifest;
        if id.is_nil() || self.stages.contains_key(&id) {
            return Err(SyncError::Invalid("invalid stage id".into()));
        }
        if limits.max_total_bytes == 0
            || limits.max_total_bytes > MAX_STAGE_BYTES
            || limits.max_stages == 0
            || limits.max_stages > MAX_STAGES
            || limits.max_event_bytes == 0
            || limits.max_event_bytes > MAX_LOGICAL_BYTES
            || limits.max_event_items == 0
            || limits.max_event_items > MAX_LOGICAL_ITEMS
            || limits.max_fragment_bytes == 0
            || limits.max_fragment_bytes > MAX_PAGE_BYTES
        {
            return Err(SyncError::Invalid("invalid stage limits".into()));
        }
        if limits.max_total_bytes > self.config.max_total_bytes
            || limits.max_stages > self.config.max_stages
            || limits.max_event_bytes > self.config.max_event_bytes
            || limits.max_event_items > self.config.max_event_items
            || limits.max_fragment_bytes > self.config.max_fragment_bytes
        {
            return Err(SyncError::Invalid(
                "stage limits exceed store configuration".into(),
            ));
        }
        if m.encoded_bytes == 0
            || m.encoded_bytes > limits.max_event_bytes
            || m.item_count > limits.max_event_items
            || m.fragment_count == 0
            || m.fragment_count > MAX_FRAGMENTS
        {
            return Err(SyncError::LogicalLimit);
        }
        if manifest_digest(m)? != m.manifest_digest {
            return Err(SyncError::Invalid("manifest digest mismatch".into()));
        }
        fragment_bounds(m, m.fragment_count - 1)?;
        let next = self
            .reserved
            .checked_add(m.encoded_bytes)
            .ok_or(SyncError::StageQuota)?;
        let mut origins = Vec::new();
        let mut event_bytes = m.encoded_bytes;
        let mut event_items = m.item_count;
        let mut dependencies = 1u64;
        for stage in self.stages.values() {
            if !origins
                .iter()
                .any(|origin| same_admission(origin, &stage.descriptor.origin))
            {
                origins.push(stage.descriptor.origin.clone());
            }
            if stage.descriptor.origin == descriptor.origin {
                if stage.limits != limits {
                    return Err(SyncError::Invalid("inconsistent graph stage limits".into()));
                }
                dependencies += 1;
                if stage.descriptor.manifest == descriptor.manifest {
                    return Err(SyncError::StageQuota);
                }
                event_bytes = event_bytes
                    .checked_add(stage.descriptor.manifest.encoded_bytes)
                    .ok_or(SyncError::LogicalLimit)?;
                event_items = event_items
                    .checked_add(stage.descriptor.manifest.item_count)
                    .ok_or(SyncError::LogicalLimit)?;
            }
        }
        if event_bytes > limits.max_event_bytes
            || event_items > limits.max_event_items
            || dependencies > MAX_LOGICAL_ITEMS
        {
            return Err(SyncError::LogicalLimit);
        }
        let new_origin = !origins
            .iter()
            .any(|origin| same_admission(origin, &descriptor.origin));
        if next > limits.max_total_bytes
            || (new_origin && origins.len() >= limits.max_stages as usize)
        {
            return Err(SyncError::StageQuota);
        }
        self.reserved = next;
        self.stages.insert(
            id,
            Stage {
                descriptor,
                limits,
                fragments: BTreeMap::new(),
            },
        );
        self.status(id)
    }
    pub fn stage(&mut self, id: Uuid, fragment: Fragment) -> Result<FragmentProgress> {
        let stage = self.stages.get_mut(&id).ok_or(SyncError::StaleWork)?;
        let (start, end) = fragment_bounds(&fragment.manifest, fragment.ordinal)?;
        if fragment.manifest != stage.descriptor.manifest
            || fragment.ordinal >= fragment.manifest.fragment_count
            || fragment.bytes.is_empty()
            || fragment.bytes.len() as u64 > stage.limits.max_fragment_bytes
            || fragment.offset != start
            || fragment.bytes.len() as u64 != end - start
            || fragment
                .offset
                .checked_add(fragment.bytes.len() as u64)
                .is_none_or(|end| end > fragment.manifest.encoded_bytes)
            || fragment_digest(&fragment)? != fragment.digest
        {
            return Err(SyncError::Invalid("invalid fragment".into()));
        }
        if let Some(old) = stage.fragments.get(&fragment.ordinal) {
            if old != &fragment {
                return Err(SyncError::ConflictingFragment);
            }
        } else {
            let start = fragment.offset;
            let end = start + fragment.bytes.len() as u64;
            if stage
                .fragments
                .values()
                .any(|f| start < f.offset + f.bytes.len() as u64 && f.offset < end)
            {
                return Err(SyncError::ConflictingFragment);
            }
            stage.fragments.insert(fragment.ordinal, fragment);
        }
        self.status(id)
    }
    pub fn status(&self, id: Uuid) -> Result<FragmentProgress> {
        let stage = self.stages.get(&id).ok_or(SyncError::StaleWork)?;
        let mut ordinal = 0u32;
        let mut offset = 0u64;
        while let Some(f) = stage.fragments.get(&ordinal) {
            if f.offset != offset {
                break;
            }
            offset += f.bytes.len() as u64;
            ordinal += 1;
        }
        Ok(FragmentProgress {
            stage: id,
            next_ordinal: ordinal,
            next_offset: offset,
            complete: ordinal == stage.descriptor.manifest.fragment_count
                && offset == stage.descriptor.manifest.encoded_bytes,
        })
    }
    pub fn assembled(&self, id: Uuid) -> Result<Vec<u8>> {
        let state = self.status(id)?;
        if !state.complete {
            return Err(SyncError::IncompleteCoverage);
        }
        let stage = self.stages.get(&id).unwrap();
        let mut out = Vec::with_capacity(stage.descriptor.manifest.encoded_bytes as usize);
        for f in stage.fragments.values() {
            out.extend(&f.bytes);
        }
        if content_digest(&out)? != stage.descriptor.manifest.content_digest {
            return Err(SyncError::Invalid("content digest mismatch".into()));
        }
        Ok(out)
    }
    /// Return the verified complete original fragments for durable relay.
    pub fn fragments(&self, id: Uuid) -> Result<Vec<Fragment>> {
        self.assembled(id)?;
        Ok(self
            .stages
            .get(&id)
            .ok_or(SyncError::StaleWork)?
            .fragments
            .values()
            .cloned()
            .collect())
    }
    pub fn abandon(&mut self, id: Uuid) -> Result<()> {
        let stage = self.stages.remove(&id).ok_or(SyncError::StaleWork)?;
        self.reserved -= stage.descriptor.manifest.encoded_bytes;
        Ok(())
    }
    /// Release every manifest staged for the same logical admission.
    pub fn abandon_group(&mut self, id: Uuid) -> Result<Vec<Uuid>> {
        let origin = self
            .stages
            .get(&id)
            .ok_or(SyncError::StaleWork)?
            .descriptor
            .origin
            .clone();
        let ids: Vec<_> = self
            .stages
            .iter()
            .filter_map(|(stage, entry)| (entry.descriptor.origin == origin).then_some(*stage))
            .collect();
        for stage in &ids {
            self.abandon(*stage)?;
        }
        Ok(ids)
    }
}
