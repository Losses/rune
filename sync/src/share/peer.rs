//! Pull-only SyncPeer adapter for one immutable share device prefix.
use super::snapshot::SnapshotBinding;
use super::{
    BaseManifest, DevicePrefix, FormatError, Head, ObjectStore, Segment, SegmentRef, StoreError,
    decode_head, decode_library, decode_segment, head_key, library_key, segment_key,
};
use crate::protocol::{
    BoxFuture, ChangePage, CheckpointKey, Digest, Event, EventRef, Fragment, HistoryStatus,
    MAX_FRAGMENTS, MAX_PAGE_BYTES, MAX_PAGE_ITEMS, ManifestKind, ManifestRef, ModelId, PageLimits,
    PageRead, PageRequest, ReceiveProgress, Result as ProtocolResult, ScanLink, StoreIdentity,
    SyncError, SyncModel, SyncPeer, canonical_event, content_digest, event_digest, fragment_bounds,
    fragment_digest, initial_anchor, manifest_digest, next_anchor, scope_id,
};
use std::{
    collections::BTreeMap,
    fmt::Display,
    marker::PhantomData,
    path::PathBuf,
    str::FromStr,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

/// Reads the journal published under one `devices/<stream>.<generation>/` prefix.
pub struct SharePeer<M: SyncModel> {
    pub(super) store: Arc<dyn ObjectStore>,
    pub(super) library_id: Uuid,
    pub(super) stream: Uuid,
    pub(super) generation: Uuid,
    pub(super) prefix: String,
    pub(super) selected_base: Option<SelectedBase<M>>,
    pub(super) cache_dir: PathBuf,
    pub(super) snapshots: Mutex<BTreeMap<Uuid, SnapshotBinding<M>>>,
    marker: PhantomData<M>,
}

#[derive(Clone)]
pub struct SelectedBase<M: SyncModel> {
    pub owner_prefix: DevicePrefix,
    pub base_id: String,
    pub manifest: BaseManifest<M>,
}

impl<M: SyncModel> SharePeer<M> {
    /// Creates a read peer and uses `cache_dir` for its bounded content cache.
    pub fn new(
        store: Arc<dyn ObjectStore>,
        library_id: Uuid,
        prefix: impl Into<String>,
        cache_dir: impl Into<PathBuf>,
    ) -> ProtocolResult<Self> {
        let prefix = prefix.into();
        let parsed = DevicePrefix::from_str(&prefix)
            .map_err(|error| SyncError::Invalid(error.to_string()))?;
        let stream = parsed.stream;
        let generation = parsed.generation;
        if stream.is_nil() || generation.is_nil() || library_id.is_nil() {
            return Err(SyncError::Invalid("invalid share peer identity".into()));
        }
        Ok(Self {
            store,
            library_id,
            stream,
            generation,
            prefix,
            selected_base: None,
            cache_dir: cache_dir.into(),
            snapshots: Mutex::new(BTreeMap::new()),
            marker: std::marker::PhantomData,
        })
    }

    /// Creates a read peer that reconciles from a caller-selected, verified live base.
    pub fn new_with_base(
        store: Arc<dyn ObjectStore>,
        library_id: Uuid,
        prefix: impl Into<String>,
        cache_dir: impl Into<PathBuf>,
        selected_base: SelectedBase<M>,
    ) -> ProtocolResult<Self> {
        let mut peer = Self::new(store, library_id, prefix, cache_dir)?;
        let (manifest_id, _) =
            super::encode_manifest(&selected_base.manifest).map_err(permanent_format)?;
        if manifest_id != selected_base.base_id {
            return Err(SyncError::Invalid(
                "selected share base id does not match its manifest".into(),
            ));
        }
        if !selected_base.manifest.covers.contains_key(&peer.prefix) {
            return Err(SyncError::Invalid(
                "selected share base does not cover this stream".into(),
            ));
        }
        peer.selected_base = Some(selected_base);
        Ok(peer)
    }

    pub(super) fn identity_for(&self) -> StoreIdentity {
        StoreIdentity {
            namespace: self.library_id,
            model: ModelId {
                model: M::model_id().into(),
                schema: M::schema_id().into(),
            },
            replica: self.stream,
            generation: self.generation,
        }
    }

    pub(super) async fn read_head(&self) -> ProtocolResult<Head> {
        let library_object = self.store.get(&library_key()).await.map_err(store_error)?;
        let library = decode_library(&library_object.bytes).map_err(permanent_format)?;
        if library.library_id != self.library_id
            || library.model != M::model_id()
            || library.schema != M::schema_id()
        {
            return Err(SyncError::IncompatibleModel);
        }
        let keys = [head_key(&self.prefix, 0), head_key(&self.prefix, 1)];
        let mut valid = Vec::new();
        let mut checksum_failure = false;
        for key in keys {
            let key = key.map_err(permanent_format)?;
            match self.store.get(&key).await {
                Ok(value) => match decode_head(&value.bytes) {
                    Ok(head) => valid.push(head),
                    Err(FormatError::Checksum) => checksum_failure = true,
                    Err(error) => return Err(permanent_format(error)),
                },
                Err(StoreError::NotFound) => checksum_failure = true,
                Err(error) => return Err(store_error(error)),
            }
        }
        let head = valid
            .into_iter()
            .max_by_key(|head| head.counter)
            .ok_or_else(|| {
                if checksum_failure {
                    SyncError::Storage("both share head slots are invalid".into())
                } else {
                    SyncError::Storage("share head is unavailable".into())
                }
            })?;
        if head.library_id != self.library_id
            || head.stream != self.stream
            || head.generation != self.generation
        {
            return Err(SyncError::IncompatibleModel);
        }
        Ok(head)
    }

    fn progress(checkpoint: &super::Checkpoint) -> ReceiveProgress {
        ReceiveProgress {
            sequence: checkpoint.seq,
            anchor: checkpoint.anchor,
        }
    }

    async fn load_segment(&self, reference: &SegmentRef) -> ProtocolResult<Segment<M>> {
        let key = segment_key(
            &self.prefix,
            reference.first,
            reference.last,
            &reference.name,
        )
        .map_err(permanent_format)?;
        let object = self.store.get(&key).await.map_err(store_error)?;
        let bytes = object.bytes;
        let digest = blake3::hash(&bytes).to_hex().to_string();
        if bytes.len() as u64 != reference.bytes || !digest.starts_with(&reference.name) {
            return Err(SyncError::Storage("share segment digest mismatch".into()));
        }
        let segment = decode_segment::<M>(&bytes).map_err(permanent_format)?;
        if segment.links.first().map(|link| link.seq) != Some(reference.first)
            || segment.links.last().map(|link| link.seq) != Some(reference.last)
            || segment.prev_anchor != reference.prev_anchor
            || segment.last_anchor != reference.last_anchor
        {
            return Err(SyncError::Storage(
                "share segment reference mismatch".into(),
            ));
        }
        Ok(segment)
    }

    async fn checkpoint_anchor(
        &self,
        head: &Head,
        sequence: u64,
    ) -> ProtocolResult<Option<Digest>> {
        if sequence == 0 {
            return Ok(Some(initial_anchor(&self.identity_for())?));
        }
        if sequence == head.floor.seq {
            return Ok(Some(head.floor.anchor));
        }
        if sequence == head.head.seq {
            return Ok(Some(head.head.anchor));
        }
        if let Some(reference) = head
            .segments
            .iter()
            .find(|segment| segment.last == sequence)
        {
            return Ok(Some(reference.last_anchor));
        }
        let Some(reference) = head
            .segments
            .iter()
            .find(|segment| segment.first <= sequence && sequence <= segment.last)
        else {
            return Ok(None);
        };
        let segment = self.load_segment(reference).await?;
        let mut anchor = segment.prev_anchor;
        for link in &segment.links {
            let digest = event_digest(&link.event)?;
            anchor = next_anchor::<M>(anchor, link.seq, &link.collection, digest)?;
            if link.seq == sequence {
                return Ok(Some(anchor));
            }
        }
        Ok(None)
    }

    async fn below_floor_after_refresh(&self, head: &Head) -> ProtocolResult<HistoryStatus> {
        let latest = self.read_head().await?;
        let floor = if latest.counter >= head.counter {
            &latest.floor
        } else {
            &head.floor
        };
        Ok(HistoryStatus::BelowFloor {
            floor: Self::progress(floor),
        })
    }

    /// Returns the verified genesis checkpoint when this prefix still retains its full chain.
    pub async fn genesis_checkpoint(&self) -> ProtocolResult<Option<ReceiveProgress>> {
        let head = self.read_head().await?;
        if head.floor.seq == 0 {
            Ok(Some(ReceiveProgress {
                sequence: 0,
                anchor: initial_anchor(&self.identity_for())?,
            }))
        } else {
            Ok(None)
        }
    }

    async fn read_page_inner(&self, request: PageRequest<M>) -> ProtocolResult<PageRead<M>> {
        validate_limits(request.limits)?;
        scope_id(&request.scope)?;
        let identity = self.identity_for();
        if request.source != identity || request.after.sequence > request.through.sequence {
            return Err(SyncError::Invalid("invalid share page request".into()));
        }
        let head = self.read_head().await?;
        if request.through.sequence > head.head.seq {
            return Err(SyncError::RetryAt(Self::progress(&head.head)));
        }
        if request.after.sequence < head.floor.seq {
            return Err(SyncError::NeedsReconciliation(HistoryStatus::BelowFloor {
                floor: Self::progress(&head.floor),
            }));
        }
        let mut all = Vec::new();
        let mut prev: Option<(u64, Digest)> = None;
        for reference in &head.segments {
            if reference.last <= request.after.sequence
                || reference.first > request.through.sequence
            {
                continue;
            }
            let segment = match self.load_segment(reference).await {
                Ok(segment) => segment,
                Err(SyncError::Storage(message))
                    if message.contains("digest mismatch")
                        || message.contains("reference mismatch")
                        || message.contains("referenced share object is missing") =>
                {
                    return Err(SyncError::NeedsReconciliation(
                        self.below_floor_after_refresh(&head).await?,
                    ));
                }
                Err(SyncError::Transport(message)) => return Err(SyncError::Transport(message)),
                Err(error) => return Err(error),
            };
            if let Some((_, prior_anchor)) = prev {
                if segment.prev_anchor != prior_anchor {
                    return Err(SyncError::NeedsReconciliation(
                        self.below_floor_after_refresh(&head).await?,
                    ));
                }
            } else if reference.first == request.after.sequence.saturating_add(1)
                && segment.prev_anchor != request.after.anchor
            {
                return Err(SyncError::Invalid(
                    "share page start anchor mismatch".into(),
                ));
            }
            for link in segment.links {
                let digest = event_digest(&link.event)?;
                let anchor = next_anchor::<M>(
                    prev.map_or(segment.prev_anchor, |(_, anchor)| anchor),
                    link.seq,
                    &link.collection,
                    digest,
                )?;
                if let Some((prior_seq, _)) = prev
                    && prior_seq.checked_add(1) != Some(link.seq)
                {
                    return Err(SyncError::Storage("gap in share segment chain".into()));
                }
                if link.seq == request.after.sequence && anchor != request.after.anchor {
                    return Err(SyncError::Invalid(
                        "share page start anchor mismatch".into(),
                    ));
                }
                if link.seq == request.through.sequence && anchor != request.through.anchor {
                    return Err(SyncError::Invalid("share page end anchor mismatch".into()));
                }
                prev = Some((link.seq, anchor));
                if link.seq > request.after.sequence && link.seq <= request.through.sequence {
                    all.push((link, digest, anchor));
                }
            }
        }
        let mut links = Vec::new();
        let mut next = request.after.clone();
        for (link, digest, anchor) in all {
            if links.len() >= request.limits.max_scanned as usize {
                break;
            }
            if link.seq != next.sequence + 1 {
                return Err(SyncError::Storage(
                    "share journal coverage has a gap".into(),
                ));
            }
            let event = request
                .scope
                .collections
                .contains(&link.collection)
                .then(|| EventRef::Inline(link.event.clone()));
            let is_in_scope = event.is_some();
            let scan = ScanLink {
                sequence: link.seq,
                collection: link.collection.clone(),
                event_digest: digest,
                event,
            };
            let proposed = ReceiveProgress {
                sequence: link.seq,
                anchor,
            };
            let mut candidate = links.clone();
            candidate.push(scan.clone());
            let page = ChangePage {
                request: request.clone(),
                next: proposed.clone(),
                done: proposed.sequence == request.through.sequence,
                links: candidate,
            };
            if serde_json::to_vec(&page).map_err(storage_error)?.len() as u64
                > request.limits.max_bytes
            {
                if links.is_empty() && is_in_scope {
                    let manifest = event_manifest(&link.event, request.limits.max_bytes)?;
                    let limit = request.limits.max_bytes;
                    let result = PageRead::FragmentNeeded {
                        request,
                        sequence: link.seq,
                        collection: link.collection,
                        event_digest: digest,
                        manifest,
                    };
                    let size = serde_json::to_vec(&result).map_err(storage_error)?.len() as u64;
                    if size > limit {
                        return Err(SyncError::LimitTooSmall { required: size });
                    }
                    return Ok(result);
                }
                if links.is_empty() {
                    return Err(SyncError::LimitTooSmall {
                        required: serde_json::to_vec(&page).map_err(storage_error)?.len() as u64,
                    });
                }
                break;
            }
            links.push(scan);
            next = proposed;
        }
        let done = next.sequence == request.through.sequence;
        Ok(PageRead::Page(ChangePage {
            request,
            done,
            next,
            links,
        }))
    }
}

impl<M: SyncModel> SyncPeer<M> for SharePeer<M> {
    fn identity(&self) -> BoxFuture<'_, StoreIdentity> {
        Box::pin(async move {
            self.read_head().await?;
            Ok(self.identity_for())
        })
    }

    fn head(&self) -> BoxFuture<'_, ReceiveProgress> {
        Box::pin(async move { Ok(Self::progress(&self.read_head().await?.head)) })
    }

    fn history_status<'a>(
        &'a self,
        key: &'a CheckpointKey,
        progress: Option<ReceiveProgress>,
    ) -> BoxFuture<'a, HistoryStatus> {
        Box::pin(async move {
            let head = self.read_head().await?;
            let head_progress = Self::progress(&head.head);
            let floor = Self::progress(&head.floor);
            if key.namespace != self.library_id {
                return Err(SyncError::IncompatibleModel);
            }
            if key.source_replica != self.stream || key.source_generation != self.generation {
                return Ok(HistoryStatus::GenerationChanged {
                    current: head.generation,
                });
            }
            let Some(checkpoint) = progress else {
                return Ok(HistoryStatus::Uninitialized {
                    head: head_progress,
                });
            };
            if checkpoint.sequence > head.head.seq {
                return Ok(HistoryStatus::HeadBehind {
                    head: head.head.seq,
                });
            }
            if checkpoint.sequence < head.floor.seq {
                return Ok(HistoryStatus::BelowFloor { floor });
            }
            let expected = match self.checkpoint_anchor(&head, checkpoint.sequence).await {
                Ok(Some(anchor)) => anchor,
                Ok(None) => return Ok(HistoryStatus::BelowFloor { floor }),
                Err(SyncError::Transport(message)) => return Err(SyncError::Transport(message)),
                Err(SyncError::Storage(message))
                    if message.contains("digest mismatch")
                        || message.contains("reference mismatch")
                        || message.contains("referenced share object is missing") =>
                {
                    return self.below_floor_after_refresh(&head).await;
                }
                Err(error) => return Err(error),
            };
            if checkpoint.anchor != expected {
                Ok(HistoryStatus::AnchorMismatch { checkpoint })
            } else {
                Ok(HistoryStatus::Continuous {
                    head: head_progress,
                    floor,
                })
            }
        })
    }

    fn read_page(&self, request: PageRequest<M>) -> BoxFuture<'_, PageRead<M>> {
        Box::pin(async move { self.read_page_inner(request).await })
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
            let head = self.read_head().await?;
            let mut found: Option<Event<M>> = None;
            for reference in &head.segments {
                let segment = match self.load_segment(reference).await {
                    Ok(segment) => segment,
                    Err(SyncError::Storage(message))
                        if message.contains("digest mismatch")
                            || message.contains("reference mismatch")
                            || message.contains("referenced share object is missing") =>
                    {
                        return Err(SyncError::NeedsReconciliation(
                            self.below_floor_after_refresh(&head).await?,
                        ));
                    }
                    Err(error) => return Err(error),
                };
                for link in segment.links {
                    let bytes = canonical_event(&link.event)?;
                    let kind = match &link.event {
                        Event::Key(_) => ManifestKind::KeyEvent,
                        Event::Merge(_) => ManifestKind::MergeEvent,
                    };
                    if manifest.kind == kind
                        && manifest.encoded_bytes == bytes.len() as u64
                        && manifest.item_count == 1
                        && manifest.content_digest == content_digest(&bytes)?
                        && manifest_digest(&manifest)? == manifest.manifest_digest
                    {
                        found = Some(link.event);
                        break;
                    }
                }
                if found.is_some() {
                    break;
                }
            }
            let event = found
                .ok_or_else(|| SyncError::Storage("fragment event is no longer retained".into()))?;
            let bytes = canonical_event(&event)?;
            let (start, end) = fragment_bounds(&manifest, ordinal)?;
            let start = usize::try_from(start).map_err(|_| SyncError::LogicalLimit)?;
            let end = usize::try_from(end).map_err(|_| SyncError::LogicalLimit)?;
            let mut fragment = Fragment {
                manifest,
                ordinal,
                offset: start as u64,
                bytes: bytes[start..end].to_vec(),
                digest: [0; 32],
            };
            fragment.digest = fragment_digest(&fragment)?;
            let response_bytes = serde_json::to_vec(&fragment).map_err(storage_error)?.len() as u64;
            if response_bytes > max_bytes {
                return Err(SyncError::LimitTooSmall {
                    required: response_bytes,
                });
            }
            Ok(fragment)
        })
    }
}

fn validate_limits(limits: PageLimits) -> ProtocolResult<()> {
    if limits.max_scanned == 0
        || limits.max_scanned > MAX_PAGE_ITEMS
        || limits.max_bytes == 0
        || limits.max_bytes > MAX_PAGE_BYTES
    {
        Err(SyncError::Invalid("invalid page limits".into()))
    } else {
        Ok(())
    }
}

fn store_error(error: StoreError) -> SyncError {
    match error {
        StoreError::NotFound => SyncError::Storage("referenced share object is missing".into()),
        StoreError::Transient(message) => SyncError::Transport(message),
        StoreError::Permanent(message) => SyncError::Storage(message),
        StoreError::ClockSkew => SyncError::Transport(
            "S3 rejected the request because this device clock is skewed".into(),
        ),
        StoreError::PreconditionFailed => {
            SyncError::Storage("share object precondition failed".into())
        }
        StoreError::PermissionDenied => SyncError::Storage("share is read-only".into()),
    }
}

fn storage_error(error: impl Display) -> SyncError {
    SyncError::Storage(error.to_string())
}

fn permanent_format(error: FormatError) -> SyncError {
    match error {
        FormatError::UnknownVersion { object, version } => {
            SyncError::Storage(format!("unknown {object} format version {version}"))
        }
        FormatError::Checksum | FormatError::Digest => {
            SyncError::Storage("share object digest mismatch".into())
        }
        error => SyncError::Storage(error.to_string()),
    }
}

fn event_manifest<M: SyncModel>(event: &Event<M>, max_bytes: u64) -> ProtocolResult<ManifestRef> {
    const FRAGMENT_BYTES: usize = 16 * 1024;
    let bytes = canonical_event(event)?;
    let fragment_bytes = usize::try_from(max_bytes.saturating_sub(1024) / 2)
        .unwrap_or(usize::MAX)
        .min(FRAGMENT_BYTES);
    if fragment_bytes == 0 {
        return Err(SyncError::LimitTooSmall { required: 1026 });
    }
    let fragment_count = bytes.len().div_ceil(fragment_bytes);
    if fragment_count > MAX_FRAGMENTS as usize {
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
        fragment_count: u32::try_from(fragment_count).map_err(|_| SyncError::LogicalLimit)?,
    };
    manifest.manifest_digest = manifest_digest(&manifest)?;
    Ok(manifest)
}
