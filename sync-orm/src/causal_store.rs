//! Transaction-bound causal authoring, reception, and accepted-state queries.
use crate::causal_writer::WriterIncarnation;
use anyhow::{Result, ensure};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, ExprTrait, FromQueryResult,
    TransactionTrait,
    sea_query::{Alias, Expr, Query},
};
use std::{marker::PhantomData, sync::Arc};
use sync::protocol::{self, ModelId, StoreIdentity, SyncModel};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct ProjectedPage<M: SyncModel> {
    pub records: Vec<protocol::Record<M>>,
    pub next: Option<Uuid>,
    pub done: bool,
}
#[derive(Clone, Debug)]
pub struct CommandResult<M: SyncModel> {
    pub record: protocol::Record<M>,
    pub projection: protocol::ProjectionChange<M>,
}
/// Inspect at most two namespace identities before an explicit open or import.
/// Listing includes incompatible models so callers cannot mistake them for an
/// empty database. This performs no initialization or mutation.
pub async fn list_namespace_ids_on<C: ConnectionTrait>(db: &C, limit: u64) -> Result<Vec<Uuid>> {
    ensure!(limit > 0 && limit <= 2, "invalid namespace lookup limit");
    let query = Query::select()
        .column(Alias::new("namespace"))
        .from(Alias::new("sync_meta"))
        .order_by(Alias::new("namespace"), sea_orm::sea_query::Order::Asc)
        .limit(limit)
        .to_owned();
    let mut namespaces = Vec::new();
    for row in db.query_all(&query).await? {
        let bytes: Vec<u8> = row.try_get_by_index(0)?;
        let id = Uuid::from_slice(&bytes)?;
        ensure!(!id.is_nil(), "invalid stored namespace");
        namespaces.push(id);
    }
    Ok(namespaces)
}
#[derive(Clone)]
pub struct CausalReplica<M: SyncModel> {
    db: DatabaseConnection,
    namespace: Uuid,
    writer: Arc<WriterIncarnation>,
    model: PhantomData<M>,
}
pub(crate) fn winner<M: SyncModel>(
    values: &[protocol::Candidate<M>],
) -> Result<Option<protocol::Candidate<M>>> {
    type CandidateRank = (protocol::Revision, protocol::Digest, protocol::Dot);
    let mut best: Option<(CandidateRank, &protocol::Candidate<M>)> = None;
    for candidate in values {
        let rank = protocol::candidate_rank(candidate)?;
        if best.as_ref().is_none_or(|(previous, _)| rank > *previous) {
            best = Some((rank, candidate));
        }
    }
    Ok(best.map(|(_, candidate)| candidate.clone()))
}
#[derive(FromQueryResult)]
struct MetaRow {
    model: String,
    schema: String,
    codec: String,
    replica: Vec<u8>,
    generation: Vec<u8>,
    head: i64,
    head_anchor: Vec<u8>,
    state_seq: i64,
    revision_counter: i64,
    history_floor: i64,
    floor_anchor: Vec<u8>,
    floor_state_seq: i64,
}
impl<M: SyncModel> CausalReplica<M> {
    /// Explicitly initialize a new namespace. This never derives causal history
    /// from business rows; importing those rows remains a separate operation.
    pub async fn create_empty(
        db: DatabaseConnection,
        namespace: Uuid,
        author: Uuid,
    ) -> Result<Self> {
        ensure!(
            !namespace.is_nil() && !author.is_nil(),
            "nil namespace or author"
        );
        protocol::validate_model::<M>()?;
        let writer = Arc::new(WriterIncarnation::new(author).map_err(anyhow::Error::msg)?);
        let identity = StoreIdentity {
            namespace,
            model: ModelId {
                model: M::model_id().into(),
                schema: M::schema_id().into(),
            },
            replica: Uuid::new_v4(),
            generation: Uuid::new_v4(),
        };
        let anchor = protocol::initial_anchor(&identity)?;
        let tx = crate::causal_leases::begin_write(&db).await?;
        let q = Query::insert()
            .into_table(Alias::new("sync_meta"))
            .columns(
                [
                    "namespace",
                    "model",
                    "schema",
                    "codec",
                    "replica",
                    "generation",
                    "head",
                    "head_anchor",
                    "state_seq",
                    "revision_counter",
                    "history_floor",
                    "floor_anchor",
                    "floor_state_seq",
                    "retained_bytes",
                ]
                .map(Alias::new),
            )
            .values_panic([
                namespace.as_bytes().to_vec().into(),
                identity.model.model.clone().into(),
                identity.model.schema.clone().into(),
                "canonical-json-1".into(),
                identity.replica.as_bytes().to_vec().into(),
                identity.generation.as_bytes().to_vec().into(),
                0_i64.into(),
                anchor.to_vec().into(),
                0_i64.into(),
                0_i64.into(),
                0_i64.into(),
                anchor.to_vec().into(),
                0_i64.into(),
                0_i64.into(),
            ])
            .to_owned();
        tx.execute(&q).await?;
        tx.commit().await?;
        Ok(Self {
            db,
            namespace,
            writer,
            model: PhantomData,
        })
    }
    /// Reopening retains replica/history and creates a distinct writer incarnation.
    pub async fn open(db: DatabaseConnection, namespace: Uuid, author: Uuid) -> Result<Self> {
        ensure!(
            !namespace.is_nil() && !author.is_nil(),
            "nil namespace or author"
        );
        protocol::validate_model::<M>()?;
        let writer = Arc::new(WriterIncarnation::new(author).map_err(anyhow::Error::msg)?);
        let replica = Self {
            db,
            namespace,
            writer,
            model: PhantomData,
        };
        let tx = replica.db.begin().await?;
        replica.meta_on(&tx).await?;
        tx.commit().await?;
        Ok(replica)
    }
    pub(crate) async fn state_seq_on(&self, tx: &DatabaseTransaction) -> Result<i64> {
        Ok(self.meta_on(tx).await?.1.state_seq)
    }
    pub fn namespace(&self) -> Uuid {
        self.namespace
    }
    /// Serialize a caller-owned mutation and verify this namespace's model.
    pub async fn lock(&self, tx: &DatabaseTransaction) -> Result<()> {
        crate::causal_leases::lock_on(tx, self.namespace).await?;
        self.meta_on(tx).await?;
        Ok(())
    }
    pub(crate) fn connection(&self) -> &DatabaseConnection {
        &self.db
    }
    pub fn author(&self) -> Uuid {
        self.writer.author()
    }
    pub fn writer_incarnation(&self) -> Uuid {
        self.writer.id()
    }
    async fn meta_on<C: ConnectionTrait>(&self, tx: &C) -> Result<(StoreIdentity, MetaRow)> {
        let q = Query::select()
            .columns(
                [
                    "model",
                    "schema",
                    "codec",
                    "replica",
                    "generation",
                    "head",
                    "head_anchor",
                    "state_seq",
                    "revision_counter",
                    "history_floor",
                    "floor_anchor",
                    "floor_state_seq",
                ]
                .map(Alias::new),
            )
            .from(Alias::new("sync_meta"))
            .and_where(Expr::col(Alias::new("namespace")).eq(self.namespace.as_bytes().to_vec()))
            .to_owned();
        let row = MetaRow::find_by_statement(tx.get_database_backend().build(&q))
            .one(tx)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("missing causal namespace; explicit initialization/import required")
            })?;
        ensure!(
            row.model == M::model_id()
                && row.schema == M::schema_id()
                && row.codec == "canonical-json-1",
            "incompatible model or codec"
        );
        let identity = StoreIdentity {
            namespace: self.namespace,
            model: ModelId {
                model: row.model.clone(),
                schema: row.schema.clone(),
            },
            replica: Uuid::from_slice(&row.replica)?,
            generation: Uuid::from_slice(&row.generation)?,
        };
        ensure!(
            !identity.replica.is_nil()
                && !identity.generation.is_nil()
                && row.head >= 0
                && row.state_seq >= 0
                && row.revision_counter >= 0
                && row.history_floor >= 0
                && row.history_floor <= row.head
                && row.floor_state_seq >= 0
                && row.floor_state_seq <= row.state_seq
                && row.head_anchor.len() == 32
                && row.floor_anchor.len() == 32,
            "invalid causal replica metadata"
        );
        Ok((identity, row))
    }
    /// Author one local assignment and return the exact displayed transition.
    /// The caller must apply that transition and commit in this same transaction.
    /// Failures require rollback; the allocated writer dot is never reused.
    pub async fn write_observed_on(
        &self,
        tx: &DatabaseTransaction,
        key: protocol::ObjectKey<M>,
        value: Option<M::Payload>,
        observed: protocol::Observed,
    ) -> Result<CommandResult<M>> {
        use protocol::{Candidate, Dot, Event, KeyDelta, ManifestKind, ProjectionChange, Revision};
        ensure!(!key.id.is_nil(), "nil object UUID");
        M::validate(&key, &value)?;
        crate::causal_leases::lock_on(tx, self.namespace).await?;
        let (identity, meta) = self.meta_on(tx).await?;
        let tag = M::collection_tag(&key.collection);
        ensure!(
            M::collection_from_tag(tag) == Some(key.collection.clone()),
            "invalid collection tag"
        );
        let page = crate::causal_candidates::page_on(
            tx,
            crate::causal_candidates::CandidatePageRequest {
                namespace: self.namespace,
                collection: tag.as_bytes(),
                state_seq: meta.state_seq,
                lower: Some(key.id),
                upper: key.id.as_u128().checked_add(1).map(Uuid::from_u128),
                after: None,
                max_items: 1024,
                max_bytes: 4 * 1024 * 1024,
            },
        )
        .await?;
        let mut current = Vec::new();
        for row in page.rows {
            let candidate: Candidate<M> = protocol::decode_candidate(&row.body)?;
            if candidate.key.id != key.id {
                break;
            }
            ensure!(
                candidate.namespace == self.namespace
                    && candidate.key == key
                    && candidate.dot.writer_incarnation.as_bytes() == row.writer.as_slice()
                    && i64::try_from(candidate.dot.counter)? == row.counter
                    && candidate.revision.actor.as_bytes() == row.author.as_slice()
                    && i64::try_from(candidate.revision.counter)? == row.revision
                    && protocol::candidate_digest(&candidate)?.as_slice() == row.digest.as_slice(),
                "stored candidate identity mismatch"
            );
            current.push(candidate);
        }
        // A page filled by this key cannot be treated as a complete observation.
        if !page.done
            && page
                .next
                .as_ref()
                .is_some_and(|cursor| cursor.object == key.id)
        {
            return Err(protocol::SyncError::WorkLimit.into());
        }
        let before = winner(&current)?;
        let known: std::collections::BTreeSet<Dot> =
            current.iter().map(|candidate| candidate.dot).collect();
        let removed = match observed {
            protocol::Observed::AllCurrent => known.clone(),
            protocol::Observed::Exact(dots) => {
                ensure!(
                    dots.is_subset(&known),
                    "observed dots are not current for key"
                );
                dots
            }
        };
        ensure!(removed.len() <= 64, protocol::SyncError::WorkLimit);
        let remaining: Vec<_> = current
            .into_iter()
            .filter(|candidate| !removed.contains(&candidate.dot))
            .collect();
        let revision = Revision {
            counter: u64::try_from(meta.revision_counter)?
                .checked_add(1)
                .filter(|n| *n <= protocol::MAX_NUMBER)
                .ok_or_else(|| anyhow::Error::new(protocol::SyncError::ClockExhausted))?,
            actor: self.writer.author(),
        };
        M::validate_revision(&key, &value, &revision)?;
        let receipt = protocol::Record {
            key: key.clone(),
            value: value.clone(),
            revision,
        };
        if value.is_none() && removed.is_empty() {
            let q = Query::update()
                .table(Alias::new("sync_meta"))
                .value(
                    Alias::new("revision_counter"),
                    i64::try_from(revision.counter)?,
                )
                .and_where(
                    Expr::col(Alias::new("namespace")).eq(self.namespace.as_bytes().to_vec()),
                )
                .to_owned();
            ensure!(
                tx.execute(&q).await?.rows_affected() == 1,
                "missing causal namespace"
            );
            return Ok(CommandResult {
                record: receipt,
                projection: ProjectionChange {
                    key,
                    before: before.clone(),
                    after: before,
                },
            });
        }
        let mut added = Vec::new();
        if let Some(payload) = value {
            let allocated = self
                .writer
                .allocate()
                .map_err(|_| protocol::SyncError::ClockExhausted)?;
            let candidate = Candidate {
                model: identity.model.clone(),
                namespace: self.namespace,
                key: key.clone(),
                dot: Dot {
                    writer_incarnation: allocated.writer_incarnation,
                    counter: allocated.counter,
                },
                revision,
                payload,
            };
            protocol::canonical_candidate(&candidate)?;
            added.push(candidate);
        }
        let mut after_candidates = remaining;
        after_candidates.extend(added.iter().cloned());
        let after = winner(&after_candidates)?;
        if after.is_none() {
            M::validate(&key, &None)?;
        }
        let state = meta
            .state_seq
            .checked_add(1)
            .ok_or_else(|| anyhow::Error::new(protocol::SyncError::ClockExhausted))?;
        let sequence = meta
            .head
            .checked_add(1)
            .ok_or_else(|| anyhow::Error::new(protocol::SyncError::ClockExhausted))?;
        let event = Event::Key(KeyDelta {
            key: key.clone(),
            author: revision,
            removed: removed.iter().copied().collect(),
            added: added.clone(),
        });
        let event_bytes = protocol::canonical_event(&event)?;
        let digest = protocol::event_digest(&event)?;
        let old_anchor: [u8; 32] = meta
            .head_anchor
            .clone()
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid anchor"))?;
        let new_anchor = protocol::next_anchor::<M>(
            old_anchor,
            u64::try_from(sequence)?,
            &key.collection,
            digest,
        )?;
        if let Some(candidate) = added.first() {
            let mut context = sync::causal::Context::default();
            context.insert(sync::causal::Dot::new(
                candidate.dot.writer_incarnation,
                candidate.dot.counter,
            )?)?;
            let mut budget = u64::MAX;
            crate::causal_observed::insert_on(
                tx,
                self.namespace,
                tag.as_bytes(),
                state,
                &context,
                &mut budget,
            )
            .await?;
            let bytes = protocol::canonical_candidate(candidate)?;
            let candidate_digest = protocol::candidate_digest(candidate)?;
            let q = Query::insert()
                .into_table(Alias::new("sync_candidates"))
                .columns(
                    [
                        "namespace",
                        "collection",
                        "writer",
                        "counter",
                        "object",
                        "author",
                        "revision",
                        "body",
                        "digest",
                        "retention_seq",
                        "live_from",
                    ]
                    .map(Alias::new),
                )
                .values_panic([
                    self.namespace.as_bytes().to_vec().into(),
                    tag.as_bytes().to_vec().into(),
                    candidate.dot.writer_incarnation.as_bytes().to_vec().into(),
                    i64::try_from(candidate.dot.counter)?.into(),
                    key.id.as_bytes().to_vec().into(),
                    revision.actor.as_bytes().to_vec().into(),
                    i64::try_from(revision.counter)?.into(),
                    bytes.into(),
                    candidate_digest.to_vec().into(),
                    state.into(),
                    state.into(),
                ])
                .to_owned();
            tx.execute(&q).await?;
        }
        for dot in &removed {
            let q = Query::update()
                .table(Alias::new("sync_candidates"))
                .value(Alias::new("live_until"), state)
                .and_where(
                    Expr::col(Alias::new("namespace")).eq(self.namespace.as_bytes().to_vec()),
                )
                .and_where(Expr::col(Alias::new("collection")).eq(tag.as_bytes().to_vec()))
                .and_where(
                    Expr::col(Alias::new("writer")).eq(dot.writer_incarnation.as_bytes().to_vec()),
                )
                .and_where(Expr::col(Alias::new("counter")).eq(i64::try_from(dot.counter)?))
                .and_where(Expr::col(Alias::new("live_until")).is_null())
                .to_owned();
            ensure!(
                tx.execute(&q).await?.rows_affected() == 1,
                "observed candidate changed"
            );
        }
        let (manifest, fragments) = crate::causal_manifests::fragment_body(
            ManifestKind::KeyEvent,
            &event_bytes,
            1,
            1024 * 1024,
        )?;
        crate::causal_manifests::publish_on(
            tx,
            self.namespace,
            &manifest,
            &fragments,
            protocol::MAX_LOGICAL_BYTES,
            |bytes| {
                protocol::decode_event::<M>(bytes)?;
                Ok(vec![])
            },
        )
        .await?;
        let q = Query::insert()
            .into_table(Alias::new("sync_journal"))
            .columns(
                [
                    "namespace",
                    "generation",
                    "sequence",
                    "state_seq",
                    "collection",
                    "event_digest",
                    "event_manifest",
                    "anchor_after",
                ]
                .map(Alias::new),
            )
            .values_panic([
                self.namespace.as_bytes().to_vec().into(),
                identity.generation.as_bytes().to_vec().into(),
                sequence.into(),
                state.into(),
                tag.as_bytes().to_vec().into(),
                digest.to_vec().into(),
                manifest.manifest_digest.to_vec().into(),
                new_anchor.to_vec().into(),
            ])
            .to_owned();
        tx.execute(&q).await?;
        let q = Query::update()
            .table(Alias::new("sync_meta"))
            .value(Alias::new("head"), sequence)
            .value(Alias::new("head_anchor"), new_anchor.to_vec())
            .value(Alias::new("state_seq"), state)
            .value(
                Alias::new("revision_counter"),
                i64::try_from(revision.counter)?,
            )
            .and_where(Expr::col(Alias::new("namespace")).eq(self.namespace.as_bytes().to_vec()))
            .to_owned();
        ensure!(
            tx.execute(&q).await?.rows_affected() == 1,
            "missing causal namespace"
        );
        Ok(CommandResult {
            record: receipt,
            projection: ProjectionChange { key, before, after },
        })
    }
    pub async fn write_local_projected_on(
        &self,
        tx: &DatabaseTransaction,
        key: protocol::ObjectKey<M>,
        value: Option<M::Payload>,
    ) -> Result<CommandResult<M>> {
        self.write_observed_on(tx, key, value, protocol::Observed::AllCurrent)
            .await
    }
    pub async fn author_on(
        &self,
        tx: &DatabaseTransaction,
        key: protocol::ObjectKey<M>,
        value: Option<M::Payload>,
    ) -> Result<protocol::Record<M>> {
        Ok(self.write_local_projected_on(tx, key, value).await?.record)
    }
    pub async fn write_local_on(
        &self,
        tx: &DatabaseTransaction,
        key: protocol::ObjectKey<M>,
        value: Option<M::Payload>,
    ) -> Result<protocol::Record<M>> {
        self.author_on(tx, key, value).await
    }
    /// Apply one already-received keyed event inside the caller transaction.
    /// The caller must combine projection and any receive checkpoint with this
    /// operation and roll back the entire transaction on failure.
    pub async fn apply_key_on(
        &self,
        tx: &DatabaseTransaction,
        delta: &protocol::KeyDelta<M>,
    ) -> Result<protocol::ProjectionChange<M>> {
        crate::causal_leases::lock_on(tx, self.namespace).await?;
        let event = protocol::Event::Key(delta.clone());
        protocol::canonical_event(&event)?;
        if delta.added.is_empty() {
            M::validate(&delta.key, &None)?;
            M::validate_revision(&delta.key, &None, &delta.author)?;
        }
        ensure!(delta.removed.len() <= 64, protocol::SyncError::WorkLimit);
        let tag = M::collection_tag(&delta.key.collection);
        for dot in &delta.removed {
            if let Some(row) = tx
                .query_one(
                    &Query::select()
                        .column(Alias::new("object"))
                        .from(Alias::new("sync_candidates"))
                        .and_where(
                            Expr::col(Alias::new("namespace"))
                                .eq(self.namespace.as_bytes().to_vec()),
                        )
                        .and_where(Expr::col(Alias::new("collection")).eq(tag.as_bytes().to_vec()))
                        .and_where(
                            Expr::col(Alias::new("writer"))
                                .eq(dot.writer_incarnation.as_bytes().to_vec()),
                        )
                        .and_where(Expr::col(Alias::new("counter")).eq(i64::try_from(dot.counter)?))
                        .to_owned(),
                )
                .await?
            {
                ensure!(
                    row.try_get_by_index::<Vec<u8>>(0)? == delta.key.id.as_bytes(),
                    protocol::SyncError::ConflictingDot
                );
            }
        }
        let before = winner(&self.key_candidates_on(tx, &delta.key).await?)?;
        let mut observed = sync::causal::Context::default();
        for dot in &delta.removed {
            observed.insert(sync::causal::Dot::new(dot.writer_incarnation, dot.counter)?)?;
        }
        let merge = protocol::MergeDelta {
            collection: delta.key.collection.clone(),
            added: delta.added.clone(),
            observed: protocol::exact_context(&observed),
        };
        let limits = protocol::ApplyLimits {
            max_projection_items: protocol::MAX_PROJECTION_ITEMS,
            max_projection_bytes: protocol::MAX_PROJECTION_BYTES,
            max_mutations: protocol::MAX_MUTATIONS,
        };
        let mut budget = u64::MAX;
        self.apply_delta_on(
            tx,
            &merge,
            &event,
            delta.author.counter,
            limits,
            &mut budget,
        )
        .await?;
        let after = winner(&self.key_candidates_on(tx, &delta.key).await?)?;
        if after.is_none() {
            M::validate(&delta.key, &None)?;
        }
        Ok(protocol::ProjectionChange {
            key: delta.key.clone(),
            before,
            after,
        })
    }
    /// Apply an anchored inline keyed page within the caller transaction.
    /// Initial contact requires reconciliation, and fragmented events require
    /// staging. Each event is validated before its accepted-state join.
    pub async fn apply_page_on(
        &self,
        tx: &DatabaseTransaction,
        page: &protocol::ChangePage<M>,
        limits: protocol::ApplyLimits,
    ) -> Result<protocol::ApplyResult<M>> {
        self.apply_page_inner_on(tx, page, limits, true).await
    }

    /// Commit a complete staged journal event. The event and original source
    /// commitment are checked again; only the inline transport envelope size
    /// is inapplicable after the event has been transferred in fragments.
    pub async fn apply_staged_event_on(
        &self,
        tx: &DatabaseTransaction,
        prepared: &protocol::PreparedEvent<M>,
        limits: protocol::ApplyLimits,
    ) -> Result<protocol::ApplyResult<M>> {
        ensure!(
            prepared.event_stage.is_some(),
            "event has not been completely staged"
        );
        let protocol::StageOrigin::Journal {
            request,
            sequence,
            collection,
            event_digest,
        } = &prepared.origin
        else {
            anyhow::bail!("staged event has no journal origin");
        };
        ensure!(
            request.after.sequence.checked_add(1) == Some(*sequence),
            "invalid staged event sequence"
        );
        let anchor =
            protocol::next_anchor::<M>(request.after.anchor, *sequence, collection, *event_digest)?;
        let page = protocol::ChangePage {
            request: request.clone(),
            next: protocol::ReceiveProgress {
                sequence: *sequence,
                anchor,
            },
            done: *sequence == request.through.sequence,
            links: vec![protocol::ScanLink {
                sequence: *sequence,
                collection: collection.clone(),
                event_digest: *event_digest,
                event: Some(protocol::EventRef::Inline(prepared.event.clone())),
            }],
        };
        self.apply_page_inner_on(tx, &page, limits, false).await
    }

    async fn apply_page_inner_on(
        &self,
        tx: &DatabaseTransaction,
        page: &protocol::ChangePage<M>,
        limits: protocol::ApplyLimits,
        enforce_envelope: bool,
    ) -> Result<protocol::ApplyResult<M>> {
        use protocol::{ApplyResult, Event, EventRef, ProjectionBatch, ReceiveProgress};
        ensure!(
            limits.max_projection_items > 0
                && limits.max_projection_items <= protocol::MAX_PROJECTION_ITEMS
                && limits.max_projection_bytes > 0
                && limits.max_projection_bytes <= protocol::MAX_PROJECTION_BYTES
                && limits.max_mutations > 0
                && limits.max_mutations <= protocol::MAX_MUTATIONS,
            "invalid apply limits"
        );
        protocol::validate_scanned_links(page)?;
        if enforce_envelope {
            ensure!(
                serde_json::to_vec(page)?.len() as u64 <= page.request.limits.max_bytes,
                "page exceeds byte limit"
            );
        }
        ensure!(
            page.request.source.namespace == self.namespace,
            "namespace mismatch"
        );
        let mut estimated_mutations = 1_u64;
        for link in &page.links {
            match &link.event {
                None => {}
                Some(EventRef::Inline(Event::Key(delta))) => {
                    ensure!(delta.removed.len() <= 64, protocol::SyncError::WorkLimit);
                    estimated_mutations = estimated_mutations
                        .checked_add(
                            20 + 2 * (delta.removed.len() as u64) + 2 * (delta.added.len() as u64),
                        )
                        .ok_or_else(|| anyhow::anyhow!("mutation limit"))?;
                }
                Some(EventRef::Inline(Event::Merge(_))) => {}
                Some(EventRef::Fragmented(_)) => {
                    anyhow::bail!("fragmented event requires complete staging")
                }
            }
        }
        ensure!(
            estimated_mutations <= limits.max_mutations,
            protocol::SyncError::WorkLimit
        );
        let mut range_budget = limits.max_mutations - estimated_mutations;
        crate::causal_leases::lock_on(tx, self.namespace).await?;
        let (local, _) = self.meta_on(tx).await?;
        ensure!(
            page.request.source.replica != local.replica
                || page.request.source.generation != local.generation,
            "self synchronization"
        );
        let scope_id = protocol::scope_id(&page.request.scope)?;
        let lookup = Query::select()
            .columns(["sequence", "anchor"].map(Alias::new))
            .from(Alias::new("sync_checkpoints"))
            .and_where(Expr::col(Alias::new("namespace")).eq(self.namespace.as_bytes().to_vec()))
            .and_where(
                Expr::col(Alias::new("source_replica")).eq(page
                    .request
                    .source
                    .replica
                    .as_bytes()
                    .to_vec()),
            )
            .and_where(
                Expr::col(Alias::new("source_generation")).eq(page
                    .request
                    .source
                    .generation
                    .as_bytes()
                    .to_vec()),
            )
            .and_where(Expr::col(Alias::new("scope_digest")).eq(scope_id.to_vec()))
            .to_owned();
        let row = tx.query_one(&lookup).await?.ok_or_else(|| {
            anyhow::anyhow!(protocol::SyncError::NeedsReconciliation(
                protocol::HistoryStatus::Uninitialized {
                    head: page.request.through.clone()
                }
            ))
        })?;
        let seq: u64 = u64::try_from(row.try_get_by_index::<i64>(0)?)?;
        let anchor: Vec<u8> = row.try_get_by_index(1)?;
        let anchor: [u8; 32] = anchor
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid checkpoint anchor"))?;
        let progress = ReceiveProgress {
            sequence: seq,
            anchor,
        };
        if page.next == progress {
            return Ok(ApplyResult {
                progress,
                projection: ProjectionBatch {
                    changes: vec![],
                    causal_changes: 0,
                    pending: 0,
                },
            });
        }
        if page.request.after != progress || page.next.sequence <= seq {
            return Err(protocol::SyncError::RetryAt(progress).into());
        }
        let mut changes = Vec::new();
        let mut projection_bytes = 0_u64;
        let mut causal_changes = 0_u64;
        for link in &page.links {
            if let Some(EventRef::Inline(Event::Key(delta))) = &link.event {
                let before_head = self.meta_on(tx).await?.1.head;
                let change = self.apply_key_on(tx, delta).await?;
                let after_head = self.meta_on(tx).await?.1.head;
                if after_head > before_head {
                    causal_changes += 1;
                }
                if change.before != change.after {
                    ensure!(
                        changes.len() < limits.max_projection_items as usize,
                        protocol::SyncError::WorkLimit
                    );
                    for candidate in [&change.before, &change.after].into_iter().flatten() {
                        projection_bytes = projection_bytes
                            .checked_add(protocol::canonical_candidate(candidate)?.len() as u64)
                            .ok_or_else(|| anyhow::Error::new(protocol::SyncError::WorkLimit))?;
                    }
                    ensure!(
                        projection_bytes <= limits.max_projection_bytes,
                        protocol::SyncError::WorkLimit
                    );
                    changes.push(change);
                }
            }
            if let Some(EventRef::Inline(Event::Merge(range))) = &link.event {
                ensure!(range_budget > 0, protocol::SyncError::WorkLimit);
                let (_, batch) = self
                    .apply_merge_on(tx, range, limits, &mut range_budget)
                    .await?;
                causal_changes = causal_changes
                    .checked_add(batch.causal_changes)
                    .ok_or_else(|| anyhow::anyhow!("causal change overflow"))?;
                for change in batch.changes {
                    ensure!(
                        changes.len() < limits.max_projection_items as usize,
                        protocol::SyncError::WorkLimit
                    );
                    for candidate in [&change.before, &change.after].into_iter().flatten() {
                        projection_bytes = projection_bytes
                            .checked_add(protocol::canonical_candidate(candidate)?.len() as u64)
                            .ok_or_else(|| anyhow::Error::new(protocol::SyncError::WorkLimit))?;
                    }
                    ensure!(
                        projection_bytes <= limits.max_projection_bytes,
                        protocol::SyncError::WorkLimit
                    );
                    changes.push(change);
                }
            }
        }
        let mut update = Query::update();
        update
            .table(Alias::new("sync_checkpoints"))
            .value(Alias::new("sequence"), i64::try_from(page.next.sequence)?)
            .value(Alias::new("anchor"), page.next.anchor.to_vec());
        if page.done {
            update.value(Alias::new("completed_at"), chrono::Utc::now().to_rfc3339());
        }
        update
            .and_where(Expr::col(Alias::new("namespace")).eq(self.namespace.as_bytes().to_vec()))
            .and_where(
                Expr::col(Alias::new("source_replica")).eq(page
                    .request
                    .source
                    .replica
                    .as_bytes()
                    .to_vec()),
            )
            .and_where(
                Expr::col(Alias::new("source_generation")).eq(page
                    .request
                    .source
                    .generation
                    .as_bytes()
                    .to_vec()),
            )
            .and_where(Expr::col(Alias::new("scope_digest")).eq(scope_id.to_vec()))
            .and_where(Expr::col(Alias::new("sequence")).eq(i64::try_from(seq)?))
            .and_where(Expr::col(Alias::new("anchor")).eq(anchor.to_vec()));
        ensure!(
            tx.execute(&update).await?.rows_affected() == 1,
            "concurrent checkpoint change"
        );
        Ok(ApplyResult {
            progress: page.next.clone(),
            projection: ProjectionBatch {
                changes,
                causal_changes,
                pending: 0,
            },
        })
    }
    /// Read the current displayed winner without inspecting business tables.
    /// The candidate set is bounded and a single SELECT supplies its snapshot.
    pub async fn get_on<C: ConnectionTrait>(
        &self,
        db: &C,
        key: &protocol::ObjectKey<M>,
    ) -> Result<Option<protocol::Record<M>>> {
        ensure!(!key.id.is_nil(), "nil object UUID");
        let tag = M::collection_tag(&key.collection);
        ensure!(
            M::collection_from_tag(tag) == Some(key.collection.clone()),
            "invalid collection tag"
        );
        let q = Query::select()
            .columns(
                [
                    "object",
                    "writer",
                    "counter",
                    "author",
                    "revision",
                    "body",
                    "digest",
                    "live_from",
                    "live_until",
                ]
                .map(Alias::new),
            )
            .from(Alias::new("sync_candidates"))
            .and_where(Expr::col(Alias::new("namespace")).eq(self.namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("collection")).eq(tag.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("object")).eq(key.id.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("live_from")).is_not_null())
            .and_where(Expr::col(Alias::new("live_until")).is_null())
            .order_by(Alias::new("writer"), sea_orm::sea_query::Order::Asc)
            .order_by(Alias::new("counter"), sea_orm::sea_query::Order::Asc)
            .limit(1025)
            .to_owned();
        let rows = crate::causal_candidates::CandidateRow::find_by_statement(
            db.get_database_backend().build(&q),
        )
        .all(db)
        .await?;
        ensure!(rows.len() <= 1024, "candidate work limit exceeded");
        let mut candidates = Vec::with_capacity(rows.len());
        let mut bytes = 0_usize;
        for row in rows {
            bytes = bytes
                .checked_add(row.body.len())
                .ok_or_else(|| anyhow::anyhow!("candidate size overflow"))?;
            ensure!(
                row.body.len() <= protocol::MAX_CANDIDATE_BYTES as usize
                    && bytes <= protocol::MAX_PROJECTION_BYTES as usize,
                "candidate work limit exceeded"
            );
            let candidate = protocol::decode_candidate::<M>(&row.body)?;
            ensure!(
                candidate.namespace == self.namespace
                    && candidate.key == *key
                    && candidate.dot.writer_incarnation.as_bytes() == row.writer.as_slice()
                    && i64::try_from(candidate.dot.counter)? == row.counter
                    && candidate.revision.actor.as_bytes() == row.author.as_slice()
                    && i64::try_from(candidate.revision.counter)? == row.revision
                    && protocol::candidate_digest(&candidate)?.as_slice() == row.digest.as_slice(),
                "stored candidate identity mismatch"
            );
            candidates.push(candidate);
        }
        Ok(winner(&candidates)?.map(|candidate| protocol::Record {
            key: key.clone(),
            value: Some(candidate.payload),
            revision: candidate.revision,
        }))
    }
    /// Bounded keyset traversal for dependency resolution and explicit import.
    /// The caller owns the transaction, keeping all key reads at one snapshot.
    pub async fn projected_records_on(
        &self,
        tx: &DatabaseTransaction,
        collection: M::Collection,
        after: Option<Uuid>,
        limits: protocol::PageLimits,
    ) -> Result<ProjectedPage<M>> {
        ensure!(
            limits.max_scanned > 0
                && limits.max_scanned <= protocol::MAX_PAGE_ITEMS
                && limits.max_bytes > 0
                && limits.max_bytes <= protocol::MAX_PAGE_BYTES
                && after.is_none_or(|id| !id.is_nil()),
            "invalid projected page limits"
        );
        let tag = M::collection_tag(&collection);
        ensure!(
            M::collection_from_tag(tag) == Some(collection.clone()),
            "invalid collection tag"
        );
        let mut q = Query::select();
        q.distinct()
            .column(Alias::new("object"))
            .from(Alias::new("sync_candidates"))
            .and_where(Expr::col(Alias::new("namespace")).eq(self.namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("collection")).eq(tag.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("live_from")).is_not_null())
            .and_where(Expr::col(Alias::new("live_until")).is_null());
        if let Some(id) = after {
            q.and_where(Expr::col(Alias::new("object")).gt(id.as_bytes().to_vec()));
        }
        q.order_by(Alias::new("object"), sea_orm::sea_query::Order::Asc)
            .limit(u64::from(limits.max_scanned) + 1);
        let rows = tx.query_all(&q.to_owned()).await?;
        let mut records = Vec::new();
        let mut next = after;
        let mut used = 0_u64;
        for (index, row) in rows.iter().enumerate() {
            if index >= limits.max_scanned as usize {
                return Ok(ProjectedPage {
                    records,
                    next,
                    done: false,
                });
            }
            let bytes: Vec<u8> = row.try_get_by_index(0)?;
            let id = Uuid::from_slice(&bytes)?;
            ensure!(!id.is_nil(), "invalid stored key");
            let key = protocol::ObjectKey {
                collection: collection.clone(),
                id,
            };
            let record = self
                .get_on(tx, &key)
                .await?
                .ok_or_else(|| anyhow::anyhow!("current key has no winner"))?;
            let size = (serde_json::to_vec(&record)?.len() as u64)
                .checked_add(64)
                .ok_or_else(|| anyhow::anyhow!("page size overflow"))?;
            if used.checked_add(size).is_none_or(|n| n > limits.max_bytes) {
                ensure!(
                    !records.is_empty(),
                    "projected record exceeds page byte limit"
                );
                return Ok(ProjectedPage {
                    records,
                    next,
                    done: false,
                });
            }
            used += size;
            next = Some(id);
            records.push(record);
        }
        Ok(ProjectedPage {
            records,
            next,
            done: true,
        })
    }
    async fn anchor_at_on(
        &self,
        tx: &DatabaseTransaction,
        meta: &MetaRow,
        sequence: u64,
    ) -> Result<[u8; 32]> {
        let sequence = i64::try_from(sequence)?;
        ensure!(
            sequence >= meta.history_floor && sequence <= meta.head,
            "history position is unavailable"
        );
        if sequence == meta.history_floor {
            if sequence == meta.head {
                ensure!(
                    meta.floor_anchor == meta.head_anchor,
                    "head anchor mismatch"
                );
            }
            return meta
                .floor_anchor
                .clone()
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid floor anchor"));
        }
        let q = Query::select()
            .column(Alias::new("anchor_after"))
            .from(Alias::new("sync_journal"))
            .and_where(Expr::col(Alias::new("namespace")).eq(self.namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("generation")).eq(meta.generation.clone()))
            .and_where(Expr::col(Alias::new("sequence")).eq(sequence))
            .to_owned();
        let value: Vec<u8> = tx
            .query_one(&q)
            .await?
            .ok_or_else(|| anyhow::anyhow!("history sequence is missing"))?
            .try_get_by_index(0)?;
        if sequence == meta.head {
            ensure!(value == meta.head_anchor, "head anchor mismatch");
        }
        value
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid journal anchor"))
    }
    /// Read an immutable interval of the current publication history. The
    /// transaction is local to this request and is never held over transport.
    pub async fn read_page(
        &self,
        request: protocol::PageRequest<M>,
    ) -> Result<protocol::PageRead<M>> {
        use protocol::{ChangePage, EventRef, PageRead, ReceiveProgress, ScanLink};
        ensure!(
            request.limits.max_scanned > 0
                && request.limits.max_scanned <= protocol::MAX_PAGE_ITEMS
                && request.limits.max_bytes > 0
                && request.limits.max_bytes <= protocol::MAX_PAGE_BYTES,
            "invalid page limits"
        );
        protocol::scope_id(&request.scope)?;
        let tx = self.db.begin().await?;
        let (identity, meta) = self.meta_on(&tx).await?;
        ensure!(request.source == identity, "source identity mismatch");
        ensure!(
            request.after.sequence <= request.through.sequence,
            "invalid page range"
        );
        ensure!(
            self.anchor_at_on(&tx, &meta, request.after.sequence)
                .await?
                == request.after.anchor,
            "history anchor mismatch"
        );
        ensure!(
            self.anchor_at_on(&tx, &meta, request.through.sequence)
                .await?
                == request.through.anchor,
            "history anchor mismatch"
        );
        let q = Query::select()
            .columns(
                [
                    "sequence",
                    "collection",
                    "event_digest",
                    "event_manifest",
                    "anchor_after",
                ]
                .map(Alias::new),
            )
            .from(Alias::new("sync_journal"))
            .and_where(Expr::col(Alias::new("namespace")).eq(self.namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("generation")).eq(meta.generation.clone()))
            .and_where(Expr::col(Alias::new("sequence")).gt(i64::try_from(request.after.sequence)?))
            .and_where(
                Expr::col(Alias::new("sequence")).lte(i64::try_from(request.through.sequence)?),
            )
            .order_by(Alias::new("sequence"), sea_orm::sea_query::Order::Asc)
            .limit(u64::from(request.limits.max_scanned))
            .to_owned();
        let rows = tx.query_all(&q).await?;
        let mut page = ChangePage {
            request: request.clone(),
            next: request.after.clone(),
            done: request.after.sequence == request.through.sequence,
            links: vec![],
        };
        let digit_count = |n: u64| n.to_string().len();
        let initial_size = serde_json::to_vec(&page)?.len();
        let base = initial_size - digit_count(page.next.sequence) - if page.done { 4 } else { 5 };
        ensure!(
            initial_size as u64 <= request.limits.max_bytes,
            "page header exceeds limit"
        );
        let mut link_bytes = 0_usize;
        let mut previous = request.after.anchor;
        for row in rows {
            let sequence = u64::try_from(row.try_get_by_index::<i64>(0)?)?;
            ensure!(sequence == page.next.sequence + 1, "journal gap");
            let tag: Vec<u8> = row.try_get_by_index(1)?;
            let tag = std::str::from_utf8(&tag)?;
            let collection = M::collection_from_tag(tag)
                .ok_or_else(|| anyhow::anyhow!("unknown journal collection"))?;
            ensure!(
                M::collection_tag(&collection) == tag,
                "invalid journal collection"
            );
            let digest: Vec<u8> = row.try_get_by_index(2)?;
            let digest: [u8; 32] = digest
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid event digest"))?;
            let manifest_digest: Vec<u8> = row.try_get_by_index(3)?;
            let manifest_digest: [u8; 32] = manifest_digest
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid event manifest"))?;
            let stored_anchor: Vec<u8> = row.try_get_by_index(4)?;
            let anchor: [u8; 32] = stored_anchor
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid journal anchor"))?;
            ensure!(
                protocol::next_anchor::<M>(previous, sequence, &collection, digest)? == anchor,
                "journal continuity mismatch"
            );
            let scoped = request.scope.collections.contains(&collection);
            let mut needed = None;
            let event = if scoped {
                let manifest =
                    crate::causal_manifests::reference_on(&tx, self.namespace, manifest_digest)
                        .await?;
                ensure!(
                    matches!(
                        manifest.kind,
                        protocol::ManifestKind::KeyEvent | protocol::ManifestKind::MergeEvent
                    ),
                    "invalid event manifest kind"
                );
                if manifest.encoded_bytes > request.limits.max_bytes {
                    needed = Some(manifest);
                    None
                } else {
                    let (_, body) = crate::causal_manifests::read_on(
                        &tx,
                        self.namespace,
                        manifest_digest,
                        request.limits.max_bytes,
                        protocol::MAX_FRAGMENTS,
                    )
                    .await?;
                    let event = protocol::decode_event::<M>(&body)?;
                    ensure!(
                        protocol::event_digest(&event)? == digest,
                        "event digest mismatch"
                    );
                    Some(EventRef::Inline(event))
                }
            } else {
                None
            };
            let link = ScanLink {
                sequence,
                collection: collection.clone(),
                event_digest: digest,
                event,
            };
            let new_link_size = serde_json::to_vec(&link)?.len();
            let done = sequence == request.through.sequence;
            let size = base
                + digit_count(sequence)
                + if done { 4 } else { 5 }
                + link_bytes
                + new_link_size
                + page.links.len();
            if needed.is_some() || size as u64 > request.limits.max_bytes {
                if page.links.is_empty() {
                    let manifest = match needed {
                        Some(value) => value,
                        None => {
                            crate::causal_manifests::reference_on(
                                &tx,
                                self.namespace,
                                manifest_digest,
                            )
                            .await?
                        }
                    };
                    ensure!(scoped, "page limit cannot fit filtered link");
                    tx.commit().await?;
                    return Ok(PageRead::FragmentNeeded {
                        request,
                        sequence,
                        collection,
                        event_digest: digest,
                        manifest,
                    });
                }
                break;
            }
            link_bytes += new_link_size;
            page.links.push(link);
            page.next = ReceiveProgress { sequence, anchor };
            page.done = done;
            previous = anchor;
        }
        ensure!(
            page.done || !page.links.is_empty(),
            "journal did not advance"
        );
        protocol::validate_scanned_links(&page)?;
        ensure!(
            serde_json::to_vec(&page)?.len() as u64 <= request.limits.max_bytes,
            "page exceeds limit"
        );
        tx.commit().await?;
        Ok(PageRead::Page(page))
    }
    pub async fn identity_on<C: ConnectionTrait>(&self, tx: &C) -> Result<StoreIdentity> {
        Ok(self.meta_on(tx).await?.0)
    }
}

impl<M: SyncModel> CausalReplica<M> {
    pub(crate) async fn key_candidates_on(
        &self,
        tx: &DatabaseTransaction,
        key: &protocol::ObjectKey<M>,
    ) -> Result<Vec<protocol::Candidate<M>>> {
        self.key_candidates_at_on(tx, key, self.meta_on(tx).await?.1.state_seq)
            .await
    }
    async fn key_candidates_at_on(
        &self,
        tx: &DatabaseTransaction,
        key: &protocol::ObjectKey<M>,
        state: i64,
    ) -> Result<Vec<protocol::Candidate<M>>> {
        let page = crate::causal_candidates::page_on(
            tx,
            crate::causal_candidates::CandidatePageRequest {
                namespace: self.namespace,
                collection: M::collection_tag(&key.collection).as_bytes(),
                state_seq: state,
                lower: Some(key.id),
                upper: key.id.as_u128().checked_add(1).map(Uuid::from_u128),
                after: None,
                max_items: 1024,
                max_bytes: 4 * 1024 * 1024,
            },
        )
        .await?;
        ensure!(page.done, protocol::SyncError::WorkLimit);
        page.rows
            .into_iter()
            .map(|row| {
                let c = protocol::decode_candidate::<M>(&row.body)?;
                ensure!(
                    c.namespace == self.namespace
                        && c.key == *key
                        && c.dot.writer_incarnation.as_bytes() == row.writer.as_slice()
                        && i64::try_from(c.dot.counter)? == row.counter
                        && c.revision.actor.as_bytes() == row.author.as_slice()
                        && i64::try_from(c.revision.counter)? == row.revision
                        && protocol::candidate_digest(&c)?.as_slice() == row.digest,
                    "stored candidate identity mismatch"
                );
                Ok(c)
            })
            .collect()
    }

    /// Apply a complete collection-wide delta and relay its original event.
    /// The caller owns projection, commit, and rollback on any failure.
    pub async fn apply_merge_on(
        &self,
        tx: &DatabaseTransaction,
        delta: &protocol::MergeDelta<M>,
        limits: protocol::ApplyLimits,
        budget: &mut u64,
    ) -> Result<(bool, protocol::ProjectionBatch<M>)> {
        ensure!(
            limits.max_projection_items > 0
                && limits.max_projection_items <= protocol::MAX_PROJECTION_ITEMS
                && limits.max_projection_bytes > 0
                && limits.max_projection_bytes <= protocol::MAX_PROJECTION_BYTES
                && limits.max_mutations > 0
                && limits.max_mutations <= protocol::MAX_MUTATIONS,
            protocol::SyncError::Invalid("invalid merge apply limits".into())
        );
        *budget = std::cmp::min(*budget, limits.max_mutations)
            .checked_sub(1)
            .ok_or(protocol::SyncError::WorkLimit)?;
        crate::causal_leases::lock_on(tx, self.namespace).await?;
        let event = protocol::Event::Merge(Box::new(delta.clone()));
        protocol::canonical_event(&event)?;
        let revision = delta
            .added
            .iter()
            .map(|c| c.revision.counter)
            .max()
            .unwrap_or(0);
        self.apply_delta_on(tx, delta, &event, revision, limits, budget)
            .await
    }

    async fn apply_delta_on(
        &self,
        tx: &DatabaseTransaction,
        delta: &protocol::MergeDelta<M>,
        event: &protocol::Event<M>,
        revision: u64,
        limits: protocol::ApplyLimits,
        budget: &mut u64,
    ) -> Result<(bool, protocol::ProjectionBatch<M>)> {
        use std::collections::{BTreeMap, BTreeSet};
        let (identity, meta) = self.meta_on(tx).await?;
        let tag = M::collection_tag(&delta.collection).as_bytes();
        let observed = protocol::causal_context(&delta.observed)?;
        let mut context = observed.clone();
        let mut close = BTreeSet::new();
        let mut keys = BTreeSet::new();
        for (writer, a, b) in observed.intervals() {
            let q = Query::select()
                .columns(["object", "writer", "counter"].map(Alias::new))
                .from(Alias::new("sync_candidates"))
                .and_where(
                    Expr::col(Alias::new("namespace")).eq(self.namespace.as_bytes().to_vec()),
                )
                .and_where(Expr::col(Alias::new("collection")).eq(tag.to_vec()))
                .and_where(Expr::col(Alias::new("writer")).eq(writer.as_bytes().to_vec()))
                .and_where(
                    Expr::col(Alias::new("counter")).between(i64::try_from(a)?, i64::try_from(b)?),
                )
                .and_where(Expr::col(Alias::new("live_from")).is_not_null())
                .and_where(Expr::col(Alias::new("live_until")).is_null())
                .limit(std::cmp::min(*budget, protocol::MAX_MUTATIONS).saturating_add(1))
                .to_owned();
            for row in tx.query_all(&q).await? {
                keys.insert(Uuid::from_slice(&row.try_get_by_index::<Vec<u8>>(0)?)?);
                close.insert((
                    row.try_get_by_index::<Vec<u8>>(1)?,
                    row.try_get_by_index::<i64>(2)?,
                ));
                ensure!(
                    close.len() as u64 <= *budget,
                    protocol::SyncError::WorkLimit
                );
            }
        }
        let mut open = Vec::new();
        for candidate in &delta.added {
            ensure!(
                candidate.namespace == self.namespace,
                protocol::SyncError::Invalid("candidate namespace mismatch".into())
            );
            protocol::validate_candidate(candidate)?;
            keys.insert(candidate.key.id);
            let dot =
                sync::causal::Dot::new(candidate.dot.writer_incarnation, candidate.dot.counter)?;
            context.insert(dot)?;
            let q = Query::select()
                .columns(["object", "body"].map(Alias::new))
                .from(Alias::new("sync_candidates"))
                .and_where(
                    Expr::col(Alias::new("namespace")).eq(self.namespace.as_bytes().to_vec()),
                )
                .and_where(Expr::col(Alias::new("collection")).eq(tag.to_vec()))
                .and_where(Expr::col(Alias::new("writer")).eq(dot.writer.as_bytes().to_vec()))
                .and_where(Expr::col(Alias::new("counter")).eq(i64::try_from(dot.counter)?))
                .to_owned();
            if let Some(row) = tx.query_one(&q).await? {
                ensure!(
                    row.try_get_by_index::<Vec<u8>>(0)? == candidate.key.id.as_bytes()
                        && row.try_get_by_index::<Vec<u8>>(1)?
                            == protocol::canonical_candidate(candidate)?,
                    protocol::SyncError::ConflictingDot
                );
            } else if !crate::causal_observed::contains_on(tx, self.namespace, tag, dot).await? {
                open.push(candidate);
            }
        }
        let mut before = BTreeMap::new();
        for id in &keys {
            let key = protocol::ObjectKey {
                collection: delta.collection.clone(),
                id: *id,
            };
            before.insert(*id, winner(&self.key_candidates_on(tx, &key).await?)?);
        }
        let state = meta
            .state_seq
            .checked_add(1)
            .ok_or(protocol::SyncError::ClockExhausted)?;
        let context_changed =
            crate::causal_observed::insert_on(tx, self.namespace, tag, state, &context, budget)
                .await?;
        let changed = context_changed || !close.is_empty() || !open.is_empty();
        for (writer, counter) in close {
            *budget = budget
                .checked_sub(1)
                .ok_or(protocol::SyncError::WorkLimit)?;
            tx.execute(
                &Query::update()
                    .table(Alias::new("sync_candidates"))
                    .value(Alias::new("live_until"), state)
                    .and_where(
                        Expr::col(Alias::new("namespace")).eq(self.namespace.as_bytes().to_vec()),
                    )
                    .and_where(Expr::col(Alias::new("collection")).eq(tag.to_vec()))
                    .and_where(Expr::col(Alias::new("writer")).eq(writer))
                    .and_where(Expr::col(Alias::new("counter")).eq(counter))
                    .to_owned(),
            )
            .await?;
        }
        for c in open {
            *budget = budget
                .checked_sub(1)
                .ok_or(protocol::SyncError::WorkLimit)?;
            tx.execute(
                &Query::insert()
                    .into_table(Alias::new("sync_candidates"))
                    .columns(
                        [
                            "namespace",
                            "collection",
                            "writer",
                            "counter",
                            "object",
                            "author",
                            "revision",
                            "body",
                            "digest",
                            "retention_seq",
                            "live_from",
                        ]
                        .map(Alias::new),
                    )
                    .values_panic([
                        self.namespace.as_bytes().to_vec().into(),
                        tag.to_vec().into(),
                        c.dot.writer_incarnation.as_bytes().to_vec().into(),
                        i64::try_from(c.dot.counter)?.into(),
                        c.key.id.as_bytes().to_vec().into(),
                        c.revision.actor.as_bytes().to_vec().into(),
                        i64::try_from(c.revision.counter)?.into(),
                        protocol::canonical_candidate(c)?.into(),
                        protocol::candidate_digest(c)?.to_vec().into(),
                        state.into(),
                        state.into(),
                    ])
                    .to_owned(),
            )
            .await?;
        }
        let mut batch = protocol::ProjectionBatch {
            changes: vec![],
            causal_changes: u64::from(changed),
            pending: 0,
        };
        let mut projection_bytes = 0u64;
        for id in keys {
            let key = protocol::ObjectKey {
                collection: delta.collection.clone(),
                id,
            };
            let after = winner(&self.key_candidates_at_on(tx, &key, state).await?)?;
            if after.is_none() {
                M::validate(&key, &None)?;
            }
            let previous = before.remove(&id).flatten();
            if previous != after {
                for c in [&previous, &after].into_iter().flatten() {
                    projection_bytes = projection_bytes
                        .checked_add(protocol::canonical_candidate(c)?.len() as u64)
                        .ok_or(protocol::SyncError::WorkLimit)?;
                }
                batch.changes.push(protocol::ProjectionChange {
                    key,
                    before: previous,
                    after,
                });
            }
        }
        ensure!(
            batch.changes.len() as u64 <= u64::from(limits.max_projection_items)
                && projection_bytes <= limits.max_projection_bytes,
            protocol::SyncError::WorkLimit
        );
        let observed_revision = std::cmp::max(meta.revision_counter, i64::try_from(revision)?);
        if changed {
            let body = protocol::canonical_event(event)?;
            let kind = match event {
                protocol::Event::Key(_) => protocol::ManifestKind::KeyEvent,
                protocol::Event::Merge(_) => protocol::ManifestKind::MergeEvent,
            };
            let (manifest, fragments) =
                crate::causal_manifests::fragment_body(kind, &body, 1, 1024 * 1024)?;
            *budget = budget
                .checked_sub(fragments.len() as u64 + 4)
                .ok_or(protocol::SyncError::WorkLimit)?;
            crate::causal_manifests::publish_on(
                tx,
                self.namespace,
                &manifest,
                &fragments,
                protocol::MAX_LOGICAL_BYTES,
                |body| {
                    protocol::decode_event::<M>(body)?;
                    Ok(vec![])
                },
            )
            .await?;
            let sequence = meta
                .head
                .checked_add(1)
                .ok_or(protocol::SyncError::ClockExhausted)?;
            let digest = protocol::event_digest(event)?;
            let old_anchor = meta
                .head_anchor
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid journal anchor"))?;
            let anchor =
                protocol::next_anchor::<M>(old_anchor, sequence as u64, &delta.collection, digest)?;
            tx.execute(
                &Query::insert()
                    .into_table(Alias::new("sync_journal"))
                    .columns(
                        [
                            "namespace",
                            "generation",
                            "sequence",
                            "state_seq",
                            "collection",
                            "event_digest",
                            "event_manifest",
                            "anchor_after",
                        ]
                        .map(Alias::new),
                    )
                    .values_panic([
                        self.namespace.as_bytes().to_vec().into(),
                        identity.generation.as_bytes().to_vec().into(),
                        sequence.into(),
                        state.into(),
                        tag.to_vec().into(),
                        digest.to_vec().into(),
                        manifest.manifest_digest.to_vec().into(),
                        anchor.to_vec().into(),
                    ])
                    .to_owned(),
            )
            .await?;
            tx.execute(
                &Query::update()
                    .table(Alias::new("sync_meta"))
                    .value(Alias::new("head"), sequence)
                    .value(Alias::new("head_anchor"), anchor.to_vec())
                    .value(Alias::new("state_seq"), state)
                    .value(Alias::new("revision_counter"), observed_revision)
                    .and_where(
                        Expr::col(Alias::new("namespace")).eq(self.namespace.as_bytes().to_vec()),
                    )
                    .to_owned(),
            )
            .await?;
        } else if observed_revision > meta.revision_counter {
            *budget = budget
                .checked_sub(1)
                .ok_or(protocol::SyncError::WorkLimit)?;
            tx.execute(
                &Query::update()
                    .table(Alias::new("sync_meta"))
                    .value(Alias::new("revision_counter"), observed_revision)
                    .and_where(
                        Expr::col(Alias::new("namespace")).eq(self.namespace.as_bytes().to_vec()),
                    )
                    .to_owned(),
            )
            .await?;
        }
        Ok((changed, batch))
    }
}
