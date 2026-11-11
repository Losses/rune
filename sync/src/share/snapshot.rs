use super::{
    BaseManifest, CollectionSummary, FormatError, SharePeer, StoreError, base_key, chunk_key,
    decode_chunk, decode_manifest,
};
use crate::protocol::{
    BoxFuture, Candidate, CandidateCursor, CollectionContext, ContextRef, Fragment, KeyRange,
    LeafPage, MAX_DEPTH, MAX_LOGICAL_BYTES, MAX_MUTATIONS, MAX_PAGE_BYTES, MAX_PAGE_ITEMS,
    MAX_ROUND_BYTES, MAX_ROUND_REQUESTS, MAX_ROWS, ManifestKind, ManifestRef, PageLimits,
    RangeHasher, RangeSummary, ReceiveProgress, Result as ProtocolResult, Scope,
    SnapshotDescriptor, SnapshotSource, StoreIdentity, SyncError, SyncModel, WorkCursor,
    WorkLimits, WorkOutcome, canonical_candidate, content_digest, context_digest, decode_context,
    fragment_bounds, fragment_digest, manifest_digest, scope_id,
};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};
use uuid::Uuid;

const CACHE_LIMIT_BYTES: u64 = 128 * 1024 * 1024;

pub(super) struct SnapshotBinding<M: SyncModel> {
    pub(super) request_id: Uuid,
    pub(super) owner_prefix: String,
    pub(super) manifest: BaseManifest<M>,
    pub(super) scope: Scope<M>,
    pub(super) id: Uuid,
    pub(super) head: ReceiveProgress,
    pub(super) roots: Vec<RangeSummary<M>>,
    pub(super) contexts: Vec<CollectionContext<M>>,
}

impl<M: SyncModel> SharePeer<M> {
    fn snapshot_binding(&self, snapshot: Uuid) -> ProtocolResult<SnapshotBinding<M>> {
        self.snapshots
            .lock()
            .map_err(|_| SyncError::Storage("share snapshot lock poisoned".into()))?
            .get(&snapshot)
            .map(|binding| SnapshotBinding {
                request_id: binding.request_id,
                owner_prefix: binding.owner_prefix.clone(),
                manifest: binding.manifest.clone(),
                scope: binding.scope.clone(),
                id: binding.id,
                head: binding.head.clone(),
                roots: binding.roots.clone(),
                contexts: binding.contexts.clone(),
            })
            .ok_or(SyncError::SnapshotExpired)
    }

    async fn read_base(&self) -> ProtocolResult<(String, BaseManifest<M>, ReceiveProgress)> {
        let head = self.read_head().await?;
        if let Some(selected) = &self.selected_base {
            let covers = selected
                .manifest
                .covers
                .get(&self.prefix)
                .ok_or(SyncError::SnapshotSourceUnavailable)?;
            return Ok((
                selected.owner_prefix.to_string(),
                selected.manifest.clone(),
                ReceiveProgress {
                    sequence: covers.seq,
                    anchor: covers.anchor,
                },
            ));
        }
        let reference = &head.floor_base;
        let key = base_key(&reference.owner_prefix, &reference.base_id).map_err(format_error)?;
        let object = match self.store.get(&key).await {
            Ok(object) => object,
            Err(StoreError::NotFound) => return Err(SyncError::SnapshotSourceUnavailable),
            Err(error) => return Err(object_store_error(error)),
        };
        let manifest = decode_manifest::<M>(&object.bytes, &reference.base_id)
            .map_err(|_| SyncError::SnapshotSourceUnavailable)?;
        let key = format!("{}.{}", self.stream, self.generation);
        let covers = manifest
            .covers
            .get(&key)
            .ok_or(SyncError::SnapshotSourceUnavailable)?;
        if covers.seq != reference.covers_seq || covers.anchor != reference.covers_anchor {
            return Err(SyncError::SnapshotSourceUnavailable);
        }
        let progress = ReceiveProgress {
            sequence: covers.seq,
            anchor: covers.anchor,
        };
        Ok((reference.owner_prefix.clone(), manifest, progress))
    }

    async fn load_object(&self, prefix: &str, digest: &str) -> ProtocolResult<Vec<u8>> {
        let cache_path = cache_path(&self.cache_dir, digest);
        if let Ok(bytes) = fs::read(&cache_path)
            && let Ok(decoded) = decode_content(digest, &bytes)
        {
            let _ =
                fs::File::open(&cache_path).and_then(|file| file.set_modified(SystemTime::now()));
            return Ok(decoded);
        }
        let _ = fs::remove_file(&cache_path);
        let key = chunk_key(prefix, digest).map_err(format_error)?;
        let object = match self.store.get(&key).await {
            Ok(object) => object,
            Err(StoreError::NotFound) => return Err(SyncError::SnapshotSourceUnavailable),
            Err(error) => return Err(object_store_error(error)),
        };
        let decoded = decode_content(digest, &object.bytes)
            .map_err(|_| SyncError::SnapshotSourceUnavailable)?;
        cache_write(&self.cache_dir, digest, &object.bytes)?;
        Ok(decoded)
    }
}

impl<M: SyncModel> SnapshotSource<M> for SharePeer<M> {
    fn release_request(&self, request_id: Uuid) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.snapshots
                .lock()
                .map_err(|_| SyncError::Storage("share snapshot lock poisoned".into()))?
                .retain(|_, binding| binding.request_id != request_id);
            Ok(())
        })
    }

    fn close_snapshot(&self, snapshot: Uuid) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.snapshots
                .lock()
                .map_err(|_| SyncError::Storage("share snapshot lock poisoned".into()))?
                .remove(&snapshot);
            Ok(())
        })
    }

    fn cancel_snapshot_build(&self, _cursor: WorkCursor) -> BoxFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn open_snapshot(
        &self,
        request_id: Uuid,
        scope: Scope<M>,
        limits: WorkLimits,
        resume: Option<WorkCursor>,
    ) -> BoxFuture<'_, WorkOutcome<SnapshotDescriptor<M>>> {
        Box::pin(async move {
            validate_work_limits(limits)?;
            scope_id(&scope)?;
            if request_id.is_nil() {
                return Err(SyncError::Invalid(
                    "invalid immutable base snapshot request".into(),
                ));
            }
            if scope != full_model_scope::<M>() {
                return Err(SyncError::Invalid(
                    "share snapshots require the full model scope".into(),
                ));
            }
            if let Some(existing) = self
                .snapshots
                .lock()
                .map_err(|_| SyncError::Storage("share snapshot lock poisoned".into()))?
                .values()
                .find(|binding| binding.request_id == request_id)
            {
                if existing.scope != scope {
                    return Err(SyncError::Invalid(
                        "snapshot request parameters changed".into(),
                    ));
                }
                return Ok(WorkOutcome::Ready(descriptor_from_binding(
                    self.identity_for(),
                    existing,
                )));
            }
            if resume.is_some() {
                return Err(SyncError::StaleWork);
            }
            let (owner_prefix, manifest, head) = self.read_base().await?;
            let mut roots = Vec::new();
            let mut contexts = Vec::new();
            let mut collections = scope.collections.iter().collect::<Vec<_>>();
            collections.sort_by_key(|collection| M::collection_tag(collection));
            for collection in collections {
                let tag = M::collection_tag(collection);
                let summary = manifest
                    .collections
                    .get(tag)
                    .ok_or(SyncError::IncompatibleModel)?;
                roots.push(summary.root.clone());
                contexts.push(
                    self.context_entry(&owner_prefix, collection, summary)
                        .await?,
                );
            }
            if roots.len() != scope.collections.len() {
                return Err(SyncError::IncompatibleModel);
            }
            let snapshot_id = Uuid::new_v4();
            for root in &mut roots {
                root.snapshot_id = snapshot_id;
            }
            let binding = SnapshotBinding {
                request_id,
                owner_prefix,
                manifest,
                scope: scope.clone(),
                id: snapshot_id,
                head,
                roots,
                contexts,
            };
            let descriptor = descriptor_from_binding(self.identity_for(), &binding);
            self.snapshots
                .lock()
                .map_err(|_| SyncError::Storage("share snapshot lock poisoned".into()))?
                .insert(snapshot_id, binding);
            Ok(WorkOutcome::Ready(descriptor))
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
            validate_work_limits(limits)?;
            if resume.is_some() {
                return Err(SyncError::StaleWork);
            }
            let binding = self.snapshot_binding(snapshot)?;
            if binding.scope != full_model_scope::<M>() {
                return Err(SyncError::Invalid(
                    "share summaries require the full model scope".into(),
                ));
            }
            validate_range(&binding.scope, &range)?;
            let collection = binding
                .manifest
                .collections
                .get(M::collection_tag(&range.collection))
                .ok_or(SyncError::IncompatibleModel)?;
            let summary = known_summary(collection, &range, binding.id).ok_or_else(|| {
                SyncError::Invalid("requested summary is not present in the base manifest".into())
            })?;
            Ok(WorkOutcome::Ready(summary))
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
            validate_work_limits(limits)?;
            if resume.is_some() {
                return Err(SyncError::StaleWork);
            }
            let binding = self.snapshot_binding(snapshot)?;
            validate_range(&binding.scope, &range)?;
            let collection = binding
                .manifest
                .collections
                .get(M::collection_tag(&range.collection))
                .ok_or(SyncError::IncompatibleModel)?;
            let children: Vec<_> = collection
                .buckets
                .iter()
                .filter(|bucket| ranges_intersect(&bucket.range, &range))
                .map(|bucket| bucket.range.clone())
                .collect();
            Ok(WorkOutcome::Ready(children))
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
                return Err(SyncError::Invalid("invalid snapshot leaf limits".into()));
            }
            let binding = self.snapshot_binding(snapshot)?;
            validate_range(&binding.scope, &range)?;
            if after.as_ref().is_some_and(|cursor| {
                cursor.key.collection != range.collection
                    || range.lower.is_some_and(|lower| cursor.key.id < lower)
                    || range.upper.is_some_and(|upper| cursor.key.id >= upper)
            }) {
                return Err(SyncError::Invalid("snapshot cursor outside range".into()));
            }
            let collection = binding
                .manifest
                .collections
                .get(M::collection_tag(&range.collection))
                .ok_or(SyncError::IncompatibleModel)?;
            let mut candidates = self
                .candidates_in_range(&binding.owner_prefix, collection, &range)
                .await?;
            candidates.sort_by_key(|candidate| CandidateCursor {
                key: candidate.key.clone(),
                dot: candidate.dot,
            });
            if let Some(cursor) = &after {
                candidates.retain(|candidate| {
                    CandidateCursor {
                        key: candidate.key.clone(),
                        dot: candidate.dot,
                    } > *cursor
                });
            }
            let mut selected = Vec::new();
            let remaining = candidates.len();
            for candidate in candidates.into_iter().take(limits.max_scanned as usize) {
                canonical_candidate(&candidate)?;
                selected.push(candidate);
                let proposed = leaf_page(snapshot, range.clone(), after.clone(), &selected);
                let response_bytes = serde_json::to_vec(&proposed)
                    .map_err(|error| SyncError::Storage(error.to_string()))?
                    .len() as u64;
                if response_bytes > limits.max_bytes {
                    selected.pop();
                    if selected.is_empty() {
                        return Err(SyncError::LimitTooSmall {
                            required: response_bytes,
                        });
                    }
                    break;
                }
            }
            let next = selected.last().map(|candidate| CandidateCursor {
                key: candidate.key.clone(),
                dot: candidate.dot,
            });
            let done = selected.len() == remaining;
            Ok(LeafPage {
                snapshot_id: snapshot,
                range,
                after,
                next,
                done,
                candidates: selected,
            })
        })
    }

    fn read_fragment(
        &self,
        snapshot: Uuid,
        manifest_ref: ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> BoxFuture<'_, Fragment> {
        Box::pin(async move {
            if max_bytes == 0 || max_bytes > MAX_PAGE_BYTES {
                return Err(SyncError::Invalid("invalid fragment limit".into()));
            }
            let binding = self.snapshot_binding(snapshot)?;
            for collection in &binding.scope.collections {
                let summary = binding
                    .manifest
                    .collections
                    .get(M::collection_tag(collection))
                    .ok_or(SyncError::IncompatibleModel)?;
                let entry = self
                    .context_entry(&binding.owner_prefix, collection, summary)
                    .await?;
                if entry.context.manifest != manifest_ref {
                    continue;
                }
                let bytes = self.context_bytes(&binding.owner_prefix, summary).await?;
                let (start, end) = fragment_bounds(&manifest_ref, ordinal)?;
                let start = usize::try_from(start).map_err(|_| SyncError::LogicalLimit)?;
                let end = usize::try_from(end).map_err(|_| SyncError::LogicalLimit)?;
                let mut fragment = Fragment {
                    manifest: manifest_ref,
                    ordinal,
                    offset: start as u64,
                    bytes: bytes[start..end].to_vec(),
                    digest: [0; 32],
                };
                fragment.digest = fragment_digest(&fragment)?;
                let response_size = serde_json::to_vec(&fragment)
                    .map_err(|error| SyncError::Storage(error.to_string()))?
                    .len() as u64;
                if response_size > max_bytes {
                    return Err(SyncError::LimitTooSmall {
                        required: response_size,
                    });
                }
                return Ok(fragment);
            }
            Err(SyncError::SnapshotExpired)
        })
    }
}

impl<M: SyncModel> SharePeer<M> {
    async fn context_entry(
        &self,
        prefix: &str,
        collection: &M::Collection,
        summary: &CollectionSummary<M>,
    ) -> ProtocolResult<CollectionContext<M>> {
        let bytes = self.context_bytes(prefix, summary).await?;
        let context = decode_context(&bytes)?;
        let mut manifest = ManifestRef {
            kind: ManifestKind::Context,
            manifest_digest: [0; 32],
            content_digest: content_digest(&bytes)?,
            encoded_bytes: bytes.len() as u64,
            item_count: context.writers.len() as u64,
            fragment_count: summary.context_fragments.len() as u32,
        };
        manifest.manifest_digest = manifest_digest(&manifest)?;
        let context_digest = context_digest(&context)?;
        Ok(CollectionContext {
            collection: collection.clone(),
            context: ContextRef {
                digest: context_digest,
                manifest,
            },
        })
    }

    async fn context_bytes(
        &self,
        prefix: &str,
        summary: &CollectionSummary<M>,
    ) -> ProtocolResult<Vec<u8>> {
        let mut bytes = Vec::new();
        for name in &summary.context_fragments {
            bytes.extend(self.load_object(prefix, name).await?);
        }
        if bytes.is_empty() || bytes.len() > MAX_LOGICAL_BYTES as usize {
            return Err(SyncError::SnapshotSourceUnavailable);
        }
        Ok(bytes)
    }

    async fn candidates_in_range(
        &self,
        prefix: &str,
        summary: &CollectionSummary<M>,
        range: &KeyRange<M>,
    ) -> ProtocolResult<Vec<Candidate<M>>> {
        // Base exporters hash all buckets against the full model scope, even when
        // a reader asks this immutable source for only one collection.
        let scope = full_model_scope::<M>();
        let mut candidates = Vec::new();
        for bucket in &summary.buckets {
            if !ranges_intersect(&bucket.range, range) {
                continue;
            }
            let bytes = self.load_object(prefix, &bucket.chunk).await?;
            let value = decode_chunk(&bucket.chunk, &gzip_bytes(&bytes)?)
                .map_err(|_| SyncError::SnapshotSourceUnavailable)?;
            let wire: serde_json::Value = serde_json::from_slice(&value.canonical_bytes)
                .map_err(|error| SyncError::Storage(error.to_string()))?;
            let rows: Vec<Candidate<M>> = serde_json::from_value(wire["candidates"].clone())
                .map_err(|error| SyncError::Storage(error.to_string()))?;
            let mut candidate_bytes = 0u64;
            let mut hasher = RangeHasher::new(
                &scope,
                bucket.range.clone(),
                bucket.candidate_count,
                bucket.candidate_bytes,
            )?;
            for (index, candidate) in rows.iter().enumerate() {
                canonical_candidate(candidate)?;
                let cursor = CandidateCursor {
                    key: candidate.key.clone(),
                    dot: candidate.dot,
                };
                if candidate.key.collection != bucket.range.collection
                    || bucket
                        .range
                        .lower
                        .is_some_and(|lower| candidate.key.id < lower)
                    || bucket
                        .range
                        .upper
                        .is_some_and(|upper| candidate.key.id >= upper)
                    || (index > 0
                        && cursor
                            <= (CandidateCursor {
                                key: rows[index - 1].key.clone(),
                                dot: rows[index - 1].dot,
                            }))
                {
                    return Err(SyncError::SnapshotSourceUnavailable);
                }
                candidate_bytes = candidate_bytes
                    .checked_add(canonical_candidate(candidate)?.len() as u64)
                    .ok_or(SyncError::LogicalLimit)?;
                hasher.push_candidate(candidate)?;
            }
            if rows.len() as u64 != bucket.candidate_count
                || candidate_bytes != bucket.candidate_bytes
                || hasher.finish()? != bucket.aggregate
            {
                return Err(SyncError::SnapshotSourceUnavailable);
            }
            for candidate in rows {
                if candidate.key.collection == range.collection
                    && range.lower.is_none_or(|lower| candidate.key.id >= lower)
                    && range.upper.is_none_or(|upper| candidate.key.id < upper)
                {
                    candidates.push(candidate);
                }
            }
        }
        Ok(candidates)
    }
}

fn full_model_scope<M: SyncModel>() -> Scope<M> {
    Scope {
        collections: M::collections().into_iter().collect(),
    }
}

fn known_summary<M: SyncModel>(
    collection: &CollectionSummary<M>,
    range: &KeyRange<M>,
    snapshot_id: Uuid,
) -> Option<RangeSummary<M>> {
    if &collection.root.range == range {
        let mut root = collection.root.clone();
        root.snapshot_id = snapshot_id;
        Some(root)
    } else {
        collection
            .buckets
            .iter()
            .find(|bucket| &bucket.range == range)
            .map(|bucket| RangeSummary {
                snapshot_id,
                range: bucket.range.clone(),
                candidate_count: bucket.candidate_count,
                candidate_bytes: bucket.candidate_bytes,
                aggregate: bucket.aggregate,
            })
    }
}

fn descriptor_from_binding<M: SyncModel>(
    source: StoreIdentity,
    binding: &SnapshotBinding<M>,
) -> SnapshotDescriptor<M> {
    SnapshotDescriptor {
        source,
        scope: binding.scope.clone(),
        snapshot_id: binding.id,
        head: binding.head.clone(),
        roots: binding.roots.clone(),
        contexts: binding.contexts.clone(),
        expires_unix_ms: u64::MAX,
    }
}

fn validate_range<M: SyncModel>(scope: &Scope<M>, range: &KeyRange<M>) -> ProtocolResult<()> {
    if !scope.collections.contains(&range.collection)
        || range
            .lower
            .zip(range.upper)
            .is_some_and(|(lower, upper)| lower >= upper)
    {
        Err(SyncError::Invalid("invalid base range".into()))
    } else {
        Ok(())
    }
}

fn leaf_page<M: SyncModel>(
    snapshot_id: Uuid,
    range: KeyRange<M>,
    after: Option<CandidateCursor<M>>,
    candidates: &[Candidate<M>],
) -> LeafPage<M> {
    LeafPage {
        snapshot_id,
        range,
        after,
        next: candidates.last().map(|candidate| CandidateCursor {
            key: candidate.key.clone(),
            dot: candidate.dot,
        }),
        done: false,
        candidates: candidates.to_vec(),
    }
}

fn ranges_intersect<M: SyncModel>(left: &KeyRange<M>, right: &KeyRange<M>) -> bool {
    left.collection == right.collection
        && !left
            .upper
            .zip(right.lower)
            .is_some_and(|(upper, lower)| upper <= lower)
        && !right
            .upper
            .zip(left.lower)
            .is_some_and(|(upper, lower)| upper <= lower)
}

fn decode_content(digest: &str, gzip: &[u8]) -> ProtocolResult<Vec<u8>> {
    let decoder = GzDecoder::new(gzip);
    let mut bytes = Vec::new();
    decoder
        .take(MAX_LOGICAL_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| SyncError::SnapshotSourceUnavailable)?;
    if bytes.len() as u64 > MAX_LOGICAL_BYTES || blake3::hash(&bytes).to_hex().as_str() != digest {
        return Err(SyncError::SnapshotSourceUnavailable);
    }
    Ok(bytes)
}

fn gzip_bytes(bytes: &[u8]) -> ProtocolResult<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .and_then(|_| encoder.finish())
        .map_err(|error| SyncError::Storage(error.to_string()))
}

fn cache_path(directory: &Path, digest: &str) -> PathBuf {
    directory.join(format!("{digest}.gz"))
}

fn cache_write(directory: &Path, digest: &str, bytes: &[u8]) -> ProtocolResult<()> {
    if bytes.len() as u64 > CACHE_LIMIT_BYTES {
        return Ok(());
    }
    fs::create_dir_all(directory).map_err(storage_error)?;
    let destination = cache_path(directory, digest);
    let temporary = directory.join(format!(".{digest}.{}.tmp", Uuid::new_v4()));
    fs::write(&temporary, bytes).map_err(storage_error)?;
    if fs::rename(&temporary, &destination).is_err() {
        if destination.exists() {
            fs::remove_file(&destination).map_err(storage_error)?;
            fs::rename(&temporary, &destination).map_err(storage_error)?;
        } else {
            return Err(SyncError::Storage(
                "could not install share cache object".into(),
            ));
        }
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).map_err(storage_error)? {
        let entry = entry.map_err(storage_error)?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("gz") {
            continue;
        }
        let metadata = entry.metadata().map_err(storage_error)?;
        files.push((
            metadata.len(),
            metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            path,
        ));
    }
    let mut total = files.iter().map(|(size, _, _)| *size).sum::<u64>();
    files.sort_by_key(|(_, modified, _)| *modified);
    for (size, _, path) in files {
        if total <= CACHE_LIMIT_BYTES {
            break;
        }
        fs::remove_file(path).map_err(storage_error)?;
        total = total.saturating_sub(size);
    }
    Ok(())
}

fn validate_work_limits(limits: WorkLimits) -> ProtocolResult<()> {
    if limits.max_requests == 0
        || limits.max_requests > MAX_ROUND_REQUESTS
        || limits.max_bytes == 0
        || limits.max_bytes > MAX_ROUND_BYTES
        || limits.max_depth == 0
        || limits.max_depth > MAX_DEPTH
        || limits.max_rows_scanned == 0
        || limits.max_rows_scanned > MAX_ROWS
        || limits.max_mutations == 0
        || limits.max_mutations > MAX_MUTATIONS
    {
        Err(SyncError::Invalid("invalid snapshot work limits".into()))
    } else {
        Ok(())
    }
}

fn format_error(error: FormatError) -> SyncError {
    SyncError::Storage(error.to_string())
}

fn object_store_error(error: StoreError) -> SyncError {
    match error {
        StoreError::NotFound => SyncError::SnapshotSourceUnavailable,
        StoreError::Transient(message) => SyncError::Transport(message),
        StoreError::Permanent(message) => SyncError::Storage(message),
        StoreError::ClockSkew => SyncError::Transport(
            "S3 rejected the request because this device clock is skewed".into(),
        ),
        StoreError::PreconditionFailed => {
            SyncError::Storage("unexpected object precondition".into())
        }
        StoreError::PermissionDenied => SyncError::Storage("share is read-only".into()),
        StoreError::Unauthorized => SyncError::Storage("share credentials rejected".into()),
    }
}

fn storage_error(error: io::Error) -> SyncError {
    SyncError::Storage(error.to_string())
}
