use anyhow::Result;
use sea_orm::{
    ConnectionTrait, Database, ExprTrait, TransactionTrait,
    sea_query::{Alias, ColumnDef, Expr, Query, Table},
};
use serde::{Deserialize, Serialize};
use sync::protocol::{self, ObjectKey, Observed, SyncModel};
use sync_orm::{causal_schema, causal_store::CausalReplica};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Notes;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Collection {
    Notes,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Note {
    text: String,
}
impl SyncModel for Notes {
    type Collection = Collection;
    type Payload = Note;
    fn model_id() -> &'static str {
        "notes"
    }
    fn schema_id() -> &'static str {
        "1"
    }
    fn collections() -> Vec<Collection> {
        vec![Collection::Notes]
    }
    fn collection_tag(_: &Collection) -> &'static str {
        "notes"
    }
    fn collection_from_tag(tag: &str) -> Option<Collection> {
        (tag == "notes").then_some(Collection::Notes)
    }
    fn validate(_: &ObjectKey<Self>, value: &Option<Note>) -> protocol::Result<()> {
        if value.as_ref().is_some_and(|note| note.text.is_empty()) {
            return Err(protocol::SyncError::Invalid("empty note".into()));
        }
        Ok(())
    }
}
#[tokio::test]
async fn author_and_projection_rollback_and_reveal_lower_survivor() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    db.execute(
        &Table::create()
            .table(Alias::new("notes"))
            .col(ColumnDef::new(Alias::new("id")).integer().primary_key())
            .col(ColumnDef::new(Alias::new("text")).text().not_null())
            .to_owned(),
    )
    .await?;
    let namespace = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let replica = CausalReplica::<Notes>::create_empty(db.clone(), namespace, actor).await?;
    let reopened = CausalReplica::<Notes>::open(db.clone(), namespace, actor).await?;
    assert_ne!(replica.writer_incarnation(), reopened.writer_incarnation());
    let key = ObjectKey {
        collection: Collection::Notes,
        id: Uuid::new_v4(),
    };
    let absent_key = ObjectKey {
        collection: Collection::Notes,
        id: Uuid::new_v4(),
    };
    let tx = db.begin().await?;
    let empty_receipt = replica.author_on(&tx, absent_key, None).await?;
    assert_eq!(empty_receipt.revision.counter, 1);
    assert!(empty_receipt.value.is_none());
    tx.commit().await?;
    let tx = db.begin().await?;
    let change = replica
        .write_observed_on(
            &tx,
            key.clone(),
            Some(Note {
                text: "aborted".into(),
            }),
            Observed::AllCurrent,
        )
        .await?;
    assert!(change.projection.after.is_some());
    tx.execute(
        &Query::insert()
            .into_table(Alias::new("notes"))
            .columns(["id", "text"].map(Alias::new))
            .values_panic([1.into(), "aborted".into()])
            .to_owned(),
    )
    .await?;
    tx.rollback().await?;
    assert!(
        db.query_one(
            &Query::select()
                .expr(Expr::val(1))
                .from(Alias::new("sync_journal"))
                .to_owned()
        )
        .await?
        .is_none()
    );
    let tx = db.begin().await?;
    let one = replica
        .write_observed_on(
            &tx,
            key.clone(),
            Some(Note {
                text: "lower".into(),
            }),
            Observed::AllCurrent,
        )
        .await?;
    tx.execute(
        &Query::insert()
            .into_table(Alias::new("notes"))
            .columns(["id", "text"].map(Alias::new))
            .values_panic([1.into(), "lower".into()])
            .to_owned(),
    )
    .await?;
    tx.commit().await?;
    let lower = one.projection.after.unwrap();
    assert_eq!(lower.dot.counter, 2);
    let tx = db.begin().await?;
    let two = replica
        .write_observed_on(
            &tx,
            key.clone(),
            Some(Note {
                text: "higher".into(),
            }),
            Observed::Exact(Default::default()),
        )
        .await?;
    tx.commit().await?;
    let higher = two.projection.after.unwrap();
    assert_ne!(higher.dot, lower.dot);
    let tx = db.begin().await?;
    let reveal = replica
        .write_observed_on(
            &tx,
            key.clone(),
            None,
            Observed::Exact([higher.dot].into_iter().collect()),
        )
        .await?;
    assert_eq!(reveal.projection.before.unwrap().payload.text, "higher");
    assert_eq!(reveal.projection.after.unwrap().payload.text, "lower");
    tx.execute(
        &Query::update()
            .table(Alias::new("notes"))
            .value(Alias::new("text"), "lower")
            .to_owned(),
    )
    .await?;
    tx.commit().await?;
    let displayed = replica.get_on(&db, &key).await?.unwrap();
    assert_eq!(displayed.value.unwrap().text, "lower");
    let tx = db.begin().await?;
    let page = replica
        .projected_records_on(
            &tx,
            Collection::Notes,
            None,
            protocol::PageLimits {
                max_scanned: 1,
                max_bytes: 1024,
            },
        )
        .await?;
    assert!(page.done);
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].key, key);
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn keyed_receive_retains_original_author_and_never_resurrects_observed_dot() -> Result<()> {
    use sync::protocol::{Candidate, Dot, KeyDelta, ModelId, Revision};
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let local = Uuid::new_v4();
    let original = Uuid::new_v4();
    let replica = CausalReplica::<Notes>::create_empty(db.clone(), ns, local).await?;
    let key = ObjectKey {
        collection: Collection::Notes,
        id: Uuid::new_v4(),
    };
    let dot = Dot {
        writer_incarnation: Uuid::new_v4(),
        counter: 1,
    };
    let candidate = Candidate {
        model: ModelId {
            model: "notes".into(),
            schema: "1".into(),
        },
        namespace: ns,
        key: key.clone(),
        dot,
        revision: Revision {
            counter: 1,
            actor: original,
        },
        payload: Note { text: "old".into() },
    };
    let remove = KeyDelta {
        key: key.clone(),
        author: Revision {
            counter: 2,
            actor: original,
        },
        removed: vec![dot],
        added: vec![],
    };
    let tx = db.begin().await?;
    let change = replica.apply_key_on(&tx, &remove).await?;
    assert!(change.after.is_none());
    tx.commit().await?;
    let late = KeyDelta {
        key: key.clone(),
        author: candidate.revision,
        removed: vec![],
        added: vec![candidate.clone()],
    };
    let tx = db.begin().await?;
    let change = replica.apply_key_on(&tx, &late).await?;
    assert!(change.after.is_none());
    tx.commit().await?;
    let other_dot = Dot {
        writer_incarnation: Uuid::new_v4(),
        counter: 1,
    };
    let current = Candidate {
        dot: other_dot,
        revision: Revision {
            counter: 3,
            actor: original,
        },
        payload: Note {
            text: "live".into(),
        },
        ..candidate
    };
    let add = KeyDelta {
        key,
        author: current.revision,
        removed: vec![],
        added: vec![current],
    };
    let tx = db.begin().await?;
    let change = replica.apply_key_on(&tx, &add).await?;
    assert_eq!(change.after.unwrap().revision.actor, original);
    tx.commit().await?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Protected;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ProtectedValue {
    owner: Uuid,
}
impl SyncModel for Protected {
    type Collection = Collection;
    type Payload = ProtectedValue;
    fn model_id() -> &'static str {
        "protected"
    }
    fn schema_id() -> &'static str {
        "1"
    }
    fn collections() -> Vec<Collection> {
        vec![Collection::Notes]
    }
    fn collection_tag(_: &Collection) -> &'static str {
        "notes"
    }
    fn collection_from_tag(tag: &str) -> Option<Collection> {
        (tag == "notes").then_some(Collection::Notes)
    }
    fn validate(_: &ObjectKey<Self>, value: &Option<ProtectedValue>) -> protocol::Result<()> {
        if value.is_none() {
            return Err(protocol::SyncError::Invalid("protected absence".into()));
        }
        Ok(())
    }
    fn validate_revision(
        _: &ObjectKey<Self>,
        value: &Option<ProtectedValue>,
        revision: &protocol::Revision,
    ) -> protocol::Result<()> {
        if value.as_ref().is_none_or(|v| v.owner != revision.actor) {
            return Err(protocol::SyncError::Invalid("invalid owner".into()));
        }
        Ok(())
    }
}
#[tokio::test]
async fn suppressed_late_protected_candidate_cannot_create_empty_state() -> Result<()> {
    use sync::protocol::{
        Candidate, Dot, ExactContext, KeyDelta, ManifestKind, ModelId, Revision, WriterContext,
    };
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let writer = Uuid::new_v4();
    let key = ObjectKey {
        collection: Collection::Notes,
        id: Uuid::new_v4(),
    };
    let store = CausalReplica::<Protected>::create_empty(db.clone(), ns, actor).await?;
    let context = ExactContext {
        writers: vec![WriterContext {
            writer_incarnation: writer,
            prefix: 1,
            exceptions: vec![],
        }],
    };
    let body = protocol::canonical_context(&context)?;
    let (manifest, parts) =
        sync_orm::causal_manifests::fragment_body(ManifestKind::Context, &body, 1, 1024)?;
    let tx = db.begin().await?;
    sync_orm::causal_manifests::publish_on(&tx, ns, &manifest, &parts, 1024, |bytes| {
        protocol::decode_context(bytes)?;
        Ok(vec![])
    })
    .await?;
    tx.execute(
        &Query::insert()
            .into_table(Alias::new("sync_context_intervals"))
            .columns(["namespace", "context_manifest", "writer", "first", "last"].map(Alias::new))
            .values_panic([
                ns.as_bytes().to_vec().into(),
                manifest.manifest_digest.to_vec().into(),
                writer.as_bytes().to_vec().into(),
                1_i64.into(),
                1_i64.into(),
            ])
            .to_owned(),
    )
    .await?;
    tx.execute(
        &Query::insert()
            .into_table(Alias::new("sync_context_ranges"))
            .columns(
                [
                    "namespace",
                    "collection",
                    "state_seq",
                    "ordinal",
                    "lower_bound",
                    "upper_bound",
                    "context_manifest",
                ]
                .map(Alias::new),
            )
            .values_panic([
                ns.as_bytes().to_vec().into(),
                b"notes".to_vec().into(),
                1_i64.into(),
                0_i64.into(),
                vec![0_u8].into(),
                vec![2_u8].into(),
                manifest.manifest_digest.to_vec().into(),
            ])
            .to_owned(),
    )
    .await?;
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_meta"))
            .value(Alias::new("state_seq"), 1_i64)
            .to_owned(),
    )
    .await?;
    tx.commit().await?;
    let candidate = Candidate {
        model: ModelId {
            model: "protected".into(),
            schema: "1".into(),
        },
        namespace: ns,
        key: key.clone(),
        dot: Dot {
            writer_incarnation: writer,
            counter: 1,
        },
        revision: Revision { counter: 1, actor },
        payload: ProtectedValue { owner: actor },
    };
    let delta = KeyDelta {
        key,
        author: candidate.revision,
        removed: vec![],
        added: vec![candidate],
    };
    let tx = db.begin().await?;
    assert!(store.apply_key_on(&tx, &delta).await.is_err());
    tx.rollback().await?;
    assert!(
        db.query_one(
            &Query::select()
                .expr(Expr::val(1))
                .from(Alias::new("sync_candidates"))
                .to_owned()
        )
        .await?
        .is_none()
    );
    assert!(
        db.query_one(
            &Query::select()
                .expr(Expr::val(1))
                .from(Alias::new("sync_journal"))
                .to_owned()
        )
        .await?
        .is_none()
    );
    Ok(())
}

#[tokio::test]
async fn anchored_inline_page_and_projection_commit_or_roll_back_together() -> Result<()> {
    use sync::protocol::{
        ApplyLimits, ChangePage, Event, EventRef, KeyDelta, PageLimits, PageRequest,
        ReceiveProgress, ScanLink, Scope,
    };
    let source_db = Database::connect("sqlite::memory:").await?;
    let target_db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&source_db).await?;
    causal_schema::install_fresh(&target_db).await?;
    target_db
        .execute(
            &Table::create()
                .table(Alias::new("notes"))
                .col(ColumnDef::new(Alias::new("id")).integer().primary_key())
                .col(ColumnDef::new(Alias::new("text")).text().not_null())
                .to_owned(),
        )
        .await?;
    let ns = Uuid::new_v4();
    let a = CausalReplica::<Notes>::create_empty(source_db.clone(), ns, Uuid::new_v4()).await?;
    let b = CausalReplica::<Notes>::create_empty(target_db.clone(), ns, Uuid::new_v4()).await?;
    let key = ObjectKey {
        collection: Collection::Notes,
        id: Uuid::new_v4(),
    };
    let tx = source_db.begin().await?;
    let candidate = a
        .write_observed_on(
            &tx,
            key.clone(),
            Some(Note {
                text: "source".into(),
            }),
            Observed::AllCurrent,
        )
        .await?
        .projection
        .after
        .unwrap();
    let source = a.identity_on(&tx).await?;
    tx.commit().await?;
    let event = Event::Key(KeyDelta {
        key,
        author: candidate.revision,
        removed: vec![],
        added: vec![candidate],
    });
    let digest = protocol::event_digest(&event)?;
    let genesis = protocol::initial_anchor(&source)?;
    let final_anchor = protocol::next_anchor::<Notes>(genesis, 1, &Collection::Notes, digest)?;
    let scope = Scope {
        collections: [Collection::Notes].into_iter().collect(),
    };
    let start = ReceiveProgress {
        sequence: 0,
        anchor: genesis,
    };
    let end = ReceiveProgress {
        sequence: 1,
        anchor: final_anchor,
    };
    let request = PageRequest {
        source: source.clone(),
        scope: scope.clone(),
        after: start.clone(),
        through: end.clone(),
        limits: PageLimits {
            max_scanned: 10,
            max_bytes: 1024 * 1024,
        },
    };
    let page = ChangePage {
        request,
        next: end.clone(),
        done: true,
        links: vec![ScanLink {
            sequence: 1,
            collection: Collection::Notes,
            event_digest: digest,
            event: Some(EventRef::Inline(event.clone())),
        }],
    };
    let source_page = a.read_page(page.request.clone()).await?;
    assert_eq!(source_page, protocol::PageRead::Page(page.clone()));
    let limits = ApplyLimits {
        max_projection_items: 10,
        max_projection_bytes: 1024 * 1024,
        max_mutations: 100,
    };
    let tx = target_db.begin().await?;
    assert!(b.apply_page_on(&tx, &page, limits).await.is_err());
    tx.rollback().await?;
    let scope_id = protocol::scope_id(&scope)?;
    let insert = Query::insert()
        .into_table(Alias::new("sync_checkpoints"))
        .columns(
            [
                "namespace",
                "source_replica",
                "source_generation",
                "scope_digest",
                "normalized_scope",
                "sequence",
                "anchor",
            ]
            .map(Alias::new),
        )
        .values_panic([
            ns.as_bytes().to_vec().into(),
            source.replica.as_bytes().to_vec().into(),
            source.generation.as_bytes().to_vec().into(),
            scope_id.to_vec().into(),
            b"notes".to_vec().into(),
            0_i64.into(),
            genesis.to_vec().into(),
        ])
        .to_owned();
    target_db.execute(&insert).await?;
    let mut staged_event = event.clone();
    if let Event::Key(delta) = &mut staged_event {
        delta.added[0].payload.text = "x".repeat(6000);
    }
    let digest = protocol::event_digest(&staged_event)?;
    let mut staged_request = page.request.clone();
    staged_request.limits.max_bytes = 3000;
    staged_request.through.anchor =
        protocol::next_anchor::<Notes>(genesis, 1, &Collection::Notes, digest)?;
    let prepared = protocol::PreparedEvent {
        origin: protocol::StageOrigin::Journal {
            request: staged_request.clone(),
            sequence: 1,
            collection: Collection::Notes,
            event_digest: digest,
        },
        event: staged_event.clone(),
        segments: vec![],
        dependencies: vec![],
        dependency_fragments: vec![],
        event_stage: Some(Uuid::new_v4()),
        resident_bytes_reserved: 0,
    };
    let oversized_page = ChangePage {
        request: staged_request.clone(),
        next: staged_request.through.clone(),
        done: true,
        links: vec![ScanLink {
            sequence: 1,
            collection: Collection::Notes,
            event_digest: digest,
            event: Some(EventRef::Inline(staged_event)),
        }],
    };
    let tx = target_db.begin().await?;
    assert!(b.apply_page_on(&tx, &oversized_page, limits).await.is_err());
    let mut invalid = prepared.clone();
    if let protocol::StageOrigin::Journal { event_digest, .. } = &mut invalid.origin {
        *event_digest = [7; 32];
    }
    assert!(
        b.apply_staged_event_on(&tx, &invalid, limits)
            .await
            .is_err()
    );
    let staged = b.apply_staged_event_on(&tx, &prepared, limits).await?;
    assert_eq!(staged.projection.changes.len(), 1);
    assert_eq!(staged.progress, staged_request.through);
    tx.rollback().await?;
    let tx = target_db.begin().await?;
    let applied = b.apply_page_on(&tx, &page, limits).await?;
    assert_eq!(applied.projection.changes.len(), 1);
    tx.execute(
        &Query::insert()
            .into_table(Alias::new("notes"))
            .columns(["id", "text"].map(Alias::new))
            .values_panic([1.into(), "source".into()])
            .to_owned(),
    )
    .await?;
    assert!(
        tx.execute(
            &Query::insert()
                .into_table(Alias::new("notes"))
                .columns(["id", "text"].map(Alias::new))
                .values_panic([1.into(), "duplicate".into()])
                .to_owned()
        )
        .await
        .is_err()
    );
    tx.rollback().await?;
    assert!(
        target_db
            .query_one(
                &Query::select()
                    .expr(Expr::val(1))
                    .from(Alias::new("sync_candidates"))
                    .to_owned()
            )
            .await?
            .is_none()
    );
    let check = Query::select()
        .columns(["sequence", "completed_at"].map(Alias::new))
        .from(Alias::new("sync_checkpoints"))
        .to_owned();
    let row = target_db.query_one(&check).await?.unwrap();
    assert_eq!(row.try_get_by_index::<i64>(0)?, 0);
    assert_eq!(row.try_get_by_index::<Option<String>>(1)?, None);
    let tx = target_db.begin().await?;
    let applied = b.apply_page_on(&tx, &page, limits).await?;
    assert_eq!(applied.progress, end);
    tx.execute(
        &Query::insert()
            .into_table(Alias::new("notes"))
            .columns(["id", "text"].map(Alias::new))
            .values_panic([1.into(), "source".into()])
            .to_owned(),
    )
    .await?;
    tx.commit().await?;
    let tx = target_db.begin().await?;
    let replay = b.apply_page_on(&tx, &page, limits).await?;
    assert_eq!(replay.projection.causal_changes, 0);
    tx.commit().await?;
    for sequence in [1_i64, 2] {
        let tx = target_db.begin().await?;
        let update = Query::update()
            .table(Alias::new("sync_checkpoints"))
            .value(Alias::new("sequence"), sequence)
            .value(Alias::new("anchor"), vec![42_u8; 32])
            .to_owned();
        tx.execute(&update).await?;
        let error = b.apply_page_on(&tx, &page, limits).await.unwrap_err();
        assert!(matches!(error.downcast_ref::<protocol::SyncError>(),
            Some(protocol::SyncError::RetryAt(progress)) if progress.sequence == sequence as u64 && progress.anchor == [42_u8; 32]));
        tx.rollback().await?;
    }
    let row = target_db.query_one(&check).await?.unwrap();
    assert_eq!(row.try_get_by_index::<i64>(0)?, 1);
    assert!(row.try_get_by_index::<Option<String>>(1)?.is_some());
    Ok(())
}

#[tokio::test]
async fn equal_revision_winner_uses_digest_before_dot() -> Result<()> {
    use sync::protocol::{Candidate, Dot, KeyDelta, ModelId, Revision};
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let store = CausalReplica::<Notes>::create_empty(db.clone(), ns, actor).await?;
    let key = ObjectKey {
        collection: Collection::Notes,
        id: Uuid::new_v4(),
    };
    let rev = Revision { counter: 1, actor };
    let make = |writer, text: String| Candidate {
        model: ModelId {
            model: "notes".into(),
            schema: "1".into(),
        },
        namespace: ns,
        key: key.clone(),
        dot: Dot {
            writer_incarnation: Uuid::from_u128(writer),
            counter: 1,
        },
        revision: rev,
        payload: Note { text },
    };
    let high_dot = make(2, "fixed".into());
    let high_digest = protocol::candidate_digest(&high_dot)?;
    let low_dot = (0..1000)
        .map(|n| make(1, format!("value-{n}")))
        .find(|candidate| protocol::candidate_digest(candidate).unwrap() > high_digest)
        .expect("a digest above the fixed candidate");
    assert!(low_dot.dot < high_dot.dot);
    let tx = db.begin().await?;
    let first = KeyDelta {
        key: key.clone(),
        author: rev,
        removed: vec![],
        added: vec![low_dot.clone()],
    };
    store.apply_key_on(&tx, &first).await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    let second = KeyDelta {
        key,
        author: rev,
        removed: vec![],
        added: vec![high_dot],
    };
    let change = store.apply_key_on(&tx, &second).await?;
    assert_eq!(change.after.unwrap(), low_dot);
    tx.commit().await?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OnlyStructural;
impl SyncModel for OnlyStructural {
    type Collection = Collection;
    type Payload = Note;
    fn model_id() -> &'static str {
        "structural"
    }
    fn schema_id() -> &'static str {
        "1"
    }
    fn collections() -> Vec<Collection> {
        vec![Collection::Notes]
    }
    fn collection_tag(_: &Collection) -> &'static str {
        "notes"
    }
    fn collection_from_tag(tag: &str) -> Option<Collection> {
        (tag == "notes").then_some(Collection::Notes)
    }
    fn validate(_: &ObjectKey<Self>, value: &Option<Note>) -> protocol::Result<()> {
        if value.is_none() {
            return Err(protocol::SyncError::Invalid("no standalone removal".into()));
        }
        Ok(())
    }
}
#[tokio::test]
async fn structural_protection_rejects_removal_even_with_a_survivor() -> Result<()> {
    use sync::protocol::{KeyDelta, Revision};
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let actor = Uuid::new_v4();
    let store =
        CausalReplica::<OnlyStructural>::create_empty(db.clone(), Uuid::new_v4(), actor).await?;
    let key = ObjectKey {
        collection: Collection::Notes,
        id: Uuid::new_v4(),
    };
    let tx = db.begin().await?;
    let first = store
        .write_observed_on(
            &tx,
            key.clone(),
            Some(Note { text: "a".into() }),
            Observed::AllCurrent,
        )
        .await?
        .projection
        .after
        .unwrap();
    tx.commit().await?;
    let tx = db.begin().await?;
    store
        .write_observed_on(
            &tx,
            key.clone(),
            Some(Note { text: "b".into() }),
            Observed::Exact(Default::default()),
        )
        .await?;
    tx.commit().await?;
    let journal = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sync_journal"))
        .to_owned();
    let candidates = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sync_candidates"))
        .to_owned();
    assert_eq!(db.query_all(&journal).await?.len(), 2);
    assert_eq!(db.query_all(&candidates).await?.len(), 2);
    let removal = KeyDelta {
        key,
        author: Revision { counter: 3, actor },
        removed: vec![first.dot],
        added: vec![],
    };
    let tx = db.begin().await?;
    assert!(store.apply_key_on(&tx, &removal).await.is_err());
    tx.rollback().await?;
    assert_eq!(db.query_all(&journal).await?.len(), 2);
    assert_eq!(db.query_all(&candidates).await?.len(), 2);
    Ok(())
}

#[tokio::test]
async fn key_bearing_observation_promotes_qualified_context_to_root() -> Result<()> {
    use sync::protocol::{
        Candidate, Dot, ExactContext, KeyDelta, ManifestKind, ModelId, Revision, WriterContext,
    };
    use sync_orm::causal_context::globally_observed_on;
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let writer = Uuid::new_v4();
    let key = ObjectKey {
        collection: Collection::Notes,
        id: Uuid::from_u128(20),
    };
    let store = CausalReplica::<Notes>::create_empty(db.clone(), ns, actor).await?;
    let context = ExactContext {
        writers: vec![WriterContext {
            writer_incarnation: writer,
            prefix: 1,
            exceptions: vec![],
        }],
    };
    let body = protocol::canonical_context(&context)?;
    let (manifest, parts) =
        sync_orm::causal_manifests::fragment_body(ManifestKind::Context, &body, 1, 1024)?;
    let tx = db.begin().await?;
    sync_orm::causal_manifests::publish_on(&tx, ns, &manifest, &parts, 1024, |bytes| {
        protocol::decode_context(bytes)?;
        Ok(vec![])
    })
    .await?;
    tx.execute(
        &Query::insert()
            .into_table(Alias::new("sync_context_intervals"))
            .columns(["namespace", "context_manifest", "writer", "first", "last"].map(Alias::new))
            .values_panic([
                ns.as_bytes().to_vec().into(),
                manifest.manifest_digest.to_vec().into(),
                writer.as_bytes().to_vec().into(),
                1_i64.into(),
                1_i64.into(),
            ])
            .to_owned(),
    )
    .await?;
    let mut lower = vec![1];
    lower.extend(Uuid::from_u128(10).as_bytes());
    let mut upper = vec![1];
    upper.extend(Uuid::from_u128(30).as_bytes());
    tx.execute(
        &Query::insert()
            .into_table(Alias::new("sync_context_ranges"))
            .columns(
                [
                    "namespace",
                    "collection",
                    "state_seq",
                    "ordinal",
                    "lower_bound",
                    "upper_bound",
                    "context_manifest",
                ]
                .map(Alias::new),
            )
            .values_panic([
                ns.as_bytes().to_vec().into(),
                b"notes".to_vec().into(),
                1_i64.into(),
                0_i64.into(),
                lower.into(),
                upper.into(),
                manifest.manifest_digest.to_vec().into(),
            ])
            .to_owned(),
    )
    .await?;
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_meta"))
            .value(Alias::new("state_seq"), 1_i64)
            .to_owned(),
    )
    .await?;
    assert!(
        !globally_observed_on(
            &tx,
            sync_orm::causal_context::GlobalObservation {
                namespace: ns,
                collection: b"notes",
                writer,
                counter: 1,
                floor: 0,
                state_seq: 1,
                max_assertions: 1024
            }
        )
        .await?
    );
    tx.commit().await?;
    let candidate = Candidate {
        model: ModelId {
            model: "notes".into(),
            schema: "1".into(),
        },
        namespace: ns,
        key: key.clone(),
        dot: Dot {
            writer_incarnation: writer,
            counter: 1,
        },
        revision: Revision { counter: 1, actor },
        payload: Note {
            text: "late".into(),
        },
    };
    let delta = KeyDelta {
        key,
        author: candidate.revision,
        removed: vec![],
        added: vec![candidate],
    };
    let tx = db.begin().await?;
    let projection = store.apply_key_on(&tx, &delta).await?;
    assert!(projection.after.is_none());
    assert!(
        globally_observed_on(
            &tx,
            sync_orm::causal_context::GlobalObservation {
                namespace: ns,
                collection: b"notes",
                writer,
                counter: 1,
                floor: 0,
                state_seq: 2,
                max_assertions: 1024
            }
        )
        .await?
    );
    tx.commit().await?;
    let tx = db.begin().await?;
    let replay = store.apply_key_on(&tx, &delta).await?;
    assert!(replay.after.is_none());
    tx.commit().await?;
    let journal = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sync_journal"))
        .to_owned();
    assert_eq!(db.query_all(&journal).await?.len(), 1);
    let tx = db.begin().await?;
    // A different writer remains known only inside a qualified interval.
    let qualified_writer = Uuid::new_v4();
    let qualified = ExactContext {
        writers: vec![WriterContext {
            writer_incarnation: qualified_writer,
            prefix: 1,
            exceptions: vec![],
        }],
    };
    let bytes = protocol::canonical_context(&qualified)?;
    let (qmanifest, qparts) =
        sync_orm::causal_manifests::fragment_body(ManifestKind::Context, &bytes, 1, 1024)?;
    sync_orm::causal_manifests::publish_on(&tx, ns, &qmanifest, &qparts, 1024, |body| {
        protocol::decode_context(body)?;
        Ok(vec![])
    })
    .await?;
    tx.execute(
        &Query::insert()
            .into_table(Alias::new("sync_context_intervals"))
            .columns(["namespace", "context_manifest", "writer", "first", "last"].map(Alias::new))
            .values_panic([
                ns.as_bytes().to_vec().into(),
                qmanifest.manifest_digest.to_vec().into(),
                qualified_writer.as_bytes().to_vec().into(),
                1_i64.into(),
                1_i64.into(),
            ])
            .to_owned(),
    )
    .await?;
    let mut lo = vec![1];
    lo.extend(Uuid::from_u128(10).as_bytes());
    let mut hi = vec![1];
    hi.extend(Uuid::from_u128(30).as_bytes());
    tx.execute(
        &Query::insert()
            .into_table(Alias::new("sync_context_ranges"))
            .columns(
                [
                    "namespace",
                    "collection",
                    "state_seq",
                    "ordinal",
                    "lower_bound",
                    "upper_bound",
                    "context_manifest",
                ]
                .map(Alias::new),
            )
            .values_panic([
                ns.as_bytes().to_vec().into(),
                b"notes".to_vec().into(),
                2_i64.into(),
                1_i64.into(),
                lo.into(),
                hi.into(),
                qmanifest.manifest_digest.to_vec().into(),
            ])
            .to_owned(),
    )
    .await?;
    sync_orm::causal_compaction::compact_on::<Notes>(&tx, ns, 2, 1024, 1024 * 1024).await?;
    assert!(
        globally_observed_on(
            &tx,
            sync_orm::causal_context::GlobalObservation {
                namespace: ns,
                collection: b"notes",
                writer,
                counter: 1,
                floor: 2,
                state_seq: 2,
                max_assertions: 1024
            }
        )
        .await?
    );
    assert!(
        !globally_observed_on(
            &tx,
            sync_orm::causal_context::GlobalObservation {
                namespace: ns,
                collection: b"notes",
                writer: qualified_writer,
                counter: 1,
                floor: 2,
                state_seq: 2,
                max_assertions: 1024
            }
        )
        .await?
    );
    let mut inside = vec![1];
    inside.extend(Uuid::from_u128(20).as_bytes());
    let mut outside = vec![1];
    outside.extend(Uuid::from_u128(40).as_bytes());
    let inside = sync_orm::causal_context::ContextScan::new(
        ns,
        b"notes".to_vec(),
        inside,
        qualified_writer,
        1,
        2,
        2,
    )?;
    let outside = sync_orm::causal_context::ContextScan::new(
        ns,
        b"notes".to_vec(),
        outside,
        qualified_writer,
        1,
        2,
        2,
    )?;
    assert!(matches!(
        sync_orm::causal_context::scan_on(&tx, inside, 1024).await?,
        sync_orm::causal_context::ScanResult::Observed
    ));
    assert!(matches!(
        sync_orm::causal_context::scan_on(&tx, outside, 1024).await?,
        sync_orm::causal_context::ScanResult::Absent
    ));
    let scope: protocol::Scope<Notes> = protocol::Scope {
        collections: [Collection::Notes].into_iter().collect(),
    };
    let work = protocol::WorkLimits {
        max_requests: 128,
        max_bytes: 1024 * 1024,
        max_depth: 64,
        max_rows_scanned: 1024,
        max_mutations: 1024,
    };
    let view = sync_orm::causal_snapshot::open_bounded_on(
        &tx,
        store.identity_on(&tx).await?,
        scope,
        10,
        100,
        work,
    )
    .await?;
    assert_eq!(view.roots[0].context_segments, 3);
    let range = view.roots[0].range.clone();
    let limits = protocol::PageLimits {
        max_scanned: 1,
        max_bytes: 1024 * 1024,
    };
    let first = sync_orm::causal_snapshot::context_page_on::<Notes>(
        &tx,
        ns,
        view.snapshot_id,
        11,
        range.clone(),
        protocol::ContextCursor {
            segment_index: 0,
            next_lower: None,
        },
        limits,
    )
    .await?;
    assert!(!first.done);
    let second = sync_orm::causal_snapshot::context_page_on::<Notes>(
        &tx,
        ns,
        view.snapshot_id,
        11,
        range.clone(),
        first.next,
        limits,
    )
    .await?;
    assert!(!second.done);
    let third = sync_orm::causal_snapshot::context_page_on::<Notes>(
        &tx,
        ns,
        view.snapshot_id,
        11,
        range,
        second.next,
        limits,
    )
    .await?;
    assert!(third.done);
    tx.commit().await?;
    assert_eq!(db.query_all(&journal).await?.len(), 0);
    Ok(())
}

#[tokio::test]
async fn bounded_floor_compaction_preserves_context_and_respects_a_pin() -> Result<()> {
    use sync::protocol::{KeyDelta, Revision};
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let store = CausalReplica::<Notes>::create_empty(db.clone(), ns, actor).await?;
    let key = ObjectKey {
        collection: Collection::Notes,
        id: Uuid::new_v4(),
    };
    let tx = db.begin().await?;
    let initial = store
        .write_observed_on(
            &tx,
            key.clone(),
            Some(Note {
                text: "removed".into(),
            }),
            Observed::AllCurrent,
        )
        .await?
        .projection
        .after
        .unwrap();
    tx.commit().await?;
    let pin = Uuid::new_v4();
    let tx = db.begin().await?;
    sync_orm::causal_leases::create_build_on(
        &tx,
        sync_orm::causal_leases::BuildLeaseRequest {
            namespace: ns,
            id: pin,
            scope: vec![1],
            now_ms: 0,
            expires_at_ms: 100,
            max_active: 10,
            max_ttl_ms: 1000,
        },
    )
    .await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    store
        .write_observed_on(&tx, key.clone(), None, Observed::AllCurrent)
        .await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    assert!(
        sync_orm::causal_compaction::compact_on::<Notes>(&tx, ns, 2, 1024, 1024 * 1024)
            .await
            .is_err()
    );
    tx.rollback().await?;
    let tx = db.begin().await?;
    sync_orm::causal_leases::revoke_on(&tx, ns, pin).await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    let step =
        sync_orm::causal_compaction::maintenance_step_on::<Notes>(&tx, ns, 1, 1024, 1024 * 1024)
            .await?;
    assert_eq!(
        step,
        sync_orm::causal_compaction::FloorStep::Advanced { state_seq: 1 }
    );
    tx.commit().await?;
    let tx = db.begin().await?;
    let step =
        sync_orm::causal_compaction::maintenance_step_on::<Notes>(&tx, ns, 1, 1024, 1024 * 1024)
            .await?;
    assert_eq!(
        step,
        sync_orm::causal_compaction::FloorStep::Advanced { state_seq: 2 }
    );
    tx.commit().await?;
    let tx = db.begin().await?;
    assert_eq!(
        sync_orm::causal_compaction::maintenance_step_on::<Notes>(&tx, ns, 1, 1024, 1024 * 1024)
            .await?,
        sync_orm::causal_compaction::FloorStep::CaughtUp
    );
    tx.commit().await?;
    let journal = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sync_journal"))
        .to_owned();
    assert!(db.query_one(&journal).await?.is_none());
    let candidates = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sync_candidates"))
        .to_owned();
    assert!(db.query_one(&candidates).await?.is_none());
    let stale = KeyDelta {
        key,
        author: Revision {
            counter: initial.revision.counter,
            actor,
        },
        removed: vec![],
        added: vec![initial],
    };
    let tx = db.begin().await?;
    let projection = store.apply_key_on(&tx, &stale).await?;
    assert!(projection.after.is_none());
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
async fn bounded_source_snapshot_pins_empty_and_older_state() -> Result<()> {
    use sync::protocol::{Scope, WorkLimits};
    let path = std::env::temp_dir().join(format!("causal-view-{}.sqlite", Uuid::new_v4()));
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let a = Database::connect(&url).await?;
    causal_schema::install_fresh(&a).await?;
    let b = Database::connect(&url).await?;
    let ns = Uuid::new_v4();
    let store = CausalReplica::<Notes>::create_empty(a.clone(), ns, Uuid::new_v4()).await?;
    let scope: Scope<Notes> = Scope {
        collections: [Collection::Notes].into_iter().collect(),
    };
    let limits = WorkLimits {
        max_requests: 128,
        max_bytes: 1024 * 1024,
        max_depth: 64,
        max_rows_scanned: 1024,
        max_mutations: 1024,
    };
    let tx = a.begin().await?;
    let identity = store.identity_on(&tx).await?;
    let empty = sync_orm::causal_snapshot::open_bounded_on(
        &tx,
        identity.clone(),
        scope.clone(),
        10,
        100,
        limits,
    )
    .await?;
    assert_eq!(empty.head.sequence, 0);
    assert_eq!(empty.roots[0].candidate_count, 0);
    tx.commit().await?;
    let key = ObjectKey {
        collection: Collection::Notes,
        id: Uuid::new_v4(),
    };
    let tx = a.begin().await?;
    store
        .author_on(&tx, key.clone(), Some(Note { text: "old".into() }))
        .await?;
    tx.commit().await?;
    let tx = a.begin().await?;
    let identity = store.identity_on(&tx).await?;
    let snapshot =
        sync_orm::causal_snapshot::open_bounded_on(&tx, identity, scope, 20, 100, limits).await?;
    assert_eq!(snapshot.head.sequence, 1);
    assert_eq!(snapshot.roots[0].candidate_count, 1);
    tx.commit().await?;
    let tx = a.begin().await?;
    store
        .author_on(&tx, key, Some(Note { text: "new".into() }))
        .await?;
    tx.commit().await?;
    let tx = b.begin().await?;
    let lease = sync_orm::causal_leases::ready_on(&tx, ns, snapshot.snapshot_id, 21).await?;
    assert_eq!(lease.state_seq, 1);
    let summary = sync_orm::causal_source::summary_ready_on::<Notes>(
        &tx,
        ns,
        snapshot.snapshot_id,
        &snapshot.roots[0].range,
        21,
    )
    .await?;
    assert_eq!(summary, snapshot.roots[0]);
    let mut missing_range = snapshot.roots[0].range.clone();
    missing_range.lower = Some(Uuid::from_u128(1));
    let error = sync_orm::causal_source::summary_ready_on::<Notes>(
        &tx,
        ns,
        snapshot.snapshot_id,
        &missing_range,
        21,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<protocol::SyncError>(),
        Some(protocol::SyncError::StaleWork)
    ));

    let old = sync_orm::causal_candidates::page_on(
        &tx,
        sync_orm::causal_candidates::CandidatePageRequest {
            namespace: ns,
            collection: b"notes",
            state_seq: lease.state_seq,
            lower: None,
            upper: None,
            after: None,
            max_items: 10,
            max_bytes: 1024 * 1024,
        },
    )
    .await?;
    assert_eq!(
        protocol::decode_candidate::<Notes>(&old.rows[0].body)?
            .payload
            .text,
        "old"
    );
    let leaf = sync_orm::causal_snapshot::leaf_on::<Notes>(
        &tx,
        ns,
        snapshot.snapshot_id,
        21,
        snapshot.roots[0].range.clone(),
        None,
        protocol::PageLimits {
            max_scanned: 10,
            max_bytes: 1024 * 1024,
        },
    )
    .await?;
    assert!(leaf.done);
    assert_eq!(leaf.candidates[0].payload.text, "old");
    let context = sync_orm::causal_snapshot::context_page_on::<Notes>(
        &tx,
        ns,
        snapshot.snapshot_id,
        21,
        snapshot.roots[0].range.clone(),
        protocol::ContextCursor {
            segment_index: 0,
            next_lower: None,
        },
        protocol::PageLimits {
            max_scanned: 10,
            max_bytes: 1024 * 1024,
        },
    )
    .await?;
    assert!(context.done);
    assert_eq!(context.segments.len(), 1);
    let root_digest: [u8; 32] = lease.retained_root.clone().unwrap().try_into().unwrap();
    let (_, root_bytes) =
        sync_orm::causal_manifests::read_on(&tx, ns, root_digest, 1024 * 1024, 100).await?;
    let root = protocol::decode_context_root::<Notes>(&snapshot.scope, &root_bytes)?;
    let live_bytes = protocol::canonical_candidates(&leaf.candidates)?;
    let (live, live_parts) = sync_orm::causal_manifests::fragment_body(
        protocol::ManifestKind::Leaf,
        &live_bytes,
        leaf.candidates.len() as u64,
        1024,
    )?;
    sync_orm::causal_manifests::publish_on(&tx, ns, &live, &live_parts, 1024 * 1024, |bytes| {
        protocol::decode_candidates::<Notes>(bytes)?;
        Ok(vec![])
    })
    .await?;
    let mut range_event = protocol::RangeEvent {
        range: snapshot.roots[0].range.clone(),
        scope: snapshot.scope.clone(),
        context_partition: root.partitions[0].partition.clone(),
        source: protocol::SnapshotRef {
            source: snapshot.source.clone(),
            snapshot_id: snapshot.snapshot_id,
            head: snapshot.head.clone(),
        },
        range_summary: snapshot.roots[0].aggregate,
        live_manifest: live,
    };
    {
        let validated =
            sync_orm::causal_range::validate_on(&tx, ns, &range_event, 1024 * 1024, 100, 100)
                .await?;
        assert_eq!(validated.candidates().len(), 1);
        let limited =
            match sync_orm::causal_range::validate_on(&tx, ns, &range_event, 1024 * 1024, 1, 100)
                .await
            {
                Ok(_) => panic!("insufficient range work was accepted"),
                Err(error) => error,
            };
        assert!(matches!(
            limited.downcast_ref::<protocol::SyncError>(),
            Some(protocol::SyncError::WorkLimit)
        ));
        let meta_query = Query::select()
            .columns(["floor_state_seq", "state_seq"].map(Alias::new))
            .from(Alias::new("sync_meta"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
            .to_owned();
        let meta = tx.query_one(&meta_query).await?.unwrap();
        let join = sync_orm::causal_range::candidate_join_on(
            &tx,
            &validated,
            meta.try_get_by_index(0)?,
            meta.try_get_by_index(1)?,
            100,
            1024 * 1024,
        )
        .await?;
        assert!(join.changes.is_empty() && join.close.is_empty() && join.open.is_empty());
        assert!(
            !sync_orm::causal_range::context_join_changed_on(
                &tx,
                &validated,
                meta.try_get_by_index(0)?,
                meta.try_get_by_index(1)?,
                100,
                1024 * 1024,
            )
            .await?
        );
        let replay = store
            .apply_range_on(
                &tx,
                &validated,
                protocol::ApplyLimits {
                    max_projection_items: 100,
                    max_projection_bytes: 1024 * 1024,
                    max_mutations: 1024,
                },
            )
            .await?;
        assert_eq!(replay.causal_changes, 0);
        assert!(replay.changes.is_empty());
    }
    // A distinct replica imports complete evidence, first rolls back its
    // projection, then commits and replays it without another journal event.
    let target = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&target).await?;
    let receiver = CausalReplica::<Notes>::create_empty(target.clone(), ns, Uuid::new_v4()).await?;
    let target_tx = target.begin().await?;
    let missing = match sync_orm::causal_range::validate_on(
        &target_tx,
        ns,
        &range_event,
        1024 * 1024,
        100,
        100,
    )
    .await
    {
        Ok(_) => panic!("missing evidence was accepted"),
        Err(error) => error,
    };
    assert!(matches!(missing.downcast_ref::<protocol::SyncError>(),
        Some(protocol::SyncError::MissingDependency(digest)) if *digest == range_event.context_partition.manifest_digest));
    for segment in &context.segments {
        let (manifest, bytes) = sync_orm::causal_manifests::read_on(
            &tx,
            ns,
            segment.context.manifest.manifest_digest,
            1024 * 1024,
            100,
        )
        .await?;
        let (copy, parts) = sync_orm::causal_manifests::fragment_body(
            protocol::ManifestKind::Context,
            &bytes,
            manifest.item_count,
            1024 * 1024,
        )?;
        assert_eq!(copy, manifest);
        sync_orm::causal_manifests::publish_on(
            &target_tx,
            ns,
            &copy,
            &parts,
            1024 * 1024,
            |body| {
                protocol::decode_context(body)?;
                Ok(vec![])
            },
        )
        .await?;
    }
    let (part, part_bytes) = sync_orm::causal_manifests::read_on(
        &tx,
        ns,
        range_event.context_partition.manifest_digest,
        1024 * 1024,
        100,
    )
    .await?;
    let (copy, parts) = sync_orm::causal_manifests::fragment_body(
        protocol::ManifestKind::ContextPartition,
        &part_bytes,
        part.item_count,
        1024 * 1024,
    )?;
    assert_eq!(copy, part);
    sync_orm::causal_manifests::publish_on(&target_tx, ns, &copy, &parts, 1024 * 1024, |body| {
        Ok(
            protocol::decode_context_partition::<Notes>(&range_event.range, body)?
                .iter()
                .map(|s| s.context.manifest.manifest_digest)
                .collect(),
        )
    })
    .await?;
    let (copy, parts) = sync_orm::causal_manifests::fragment_body(
        protocol::ManifestKind::Leaf,
        &live_bytes,
        leaf.candidates.len() as u64,
        1024,
    )?;
    assert_eq!(copy, range_event.live_manifest);
    sync_orm::causal_manifests::publish_on(&target_tx, ns, &copy, &parts, 1024 * 1024, |body| {
        protocol::decode_candidates::<Notes>(body)?;
        Ok(vec![])
    })
    .await?;
    target_tx.commit().await?;
    let limits = protocol::ApplyLimits {
        max_projection_items: 100,
        max_projection_bytes: 1024 * 1024,
        max_mutations: 1024,
    };
    for (attempt, commit) in [false, true, true].into_iter().enumerate() {
        let target_tx = target.begin().await?;
        let evidence = sync_orm::causal_range::validate_on(
            &target_tx,
            ns,
            &range_event,
            1024 * 1024,
            100,
            100,
        )
        .await?;
        let projection = receiver
            .apply_range_on(&target_tx, &evidence, limits)
            .await?;
        assert_eq!(projection.causal_changes, if attempt == 2 { 0 } else { 1 });
        assert_eq!(projection.changes.len(), if attempt == 2 { 0 } else { 1 });
        if !commit {
            assert_eq!(projection.changes.len(), 1);
            target_tx.rollback().await?;
        } else {
            target_tx.commit().await?;
        }
    }
    assert!(
        receiver
            .get_on(&target, &leaf.candidates[0].key)
            .await?
            .is_some()
    );
    let remote = receiver.identity_on(&target).await?;
    let genesis = protocol::initial_anchor(&remote)?;
    let event = protocol::Event::Range(Box::new(range_event.clone()));
    let end = protocol::ReceiveProgress {
        sequence: 1,
        anchor: protocol::next_anchor::<Notes>(
            genesis,
            1,
            &Collection::Notes,
            protocol::event_digest(&event)?,
        )?,
    };
    let request = protocol::PageRequest {
        source: remote.clone(),
        scope: snapshot.scope.clone(),
        after: protocol::ReceiveProgress {
            sequence: 0,
            anchor: genesis,
        },
        through: end.clone(),
        limits: protocol::PageLimits {
            max_scanned: 10,
            max_bytes: 1024 * 1024,
        },
    };
    let page = match receiver.read_page(request).await? {
        protocol::PageRead::Page(page) => page,
        _ => panic!("expected inline range page"),
    };
    let insert = Query::insert()
        .into_table(Alias::new("sync_checkpoints"))
        .columns(
            [
                "namespace",
                "source_replica",
                "source_generation",
                "scope_digest",
                "normalized_scope",
                "sequence",
                "anchor",
            ]
            .map(Alias::new),
        )
        .values_panic([
            ns.as_bytes().to_vec().into(),
            remote.replica.as_bytes().to_vec().into(),
            remote.generation.as_bytes().to_vec().into(),
            protocol::scope_id(&snapshot.scope)?.to_vec().into(),
            b"notes".to_vec().into(),
            0_i64.into(),
            genesis.to_vec().into(),
        ])
        .to_owned();
    tx.execute(&insert).await?;
    let received = store.apply_page_on(&tx, &page, limits).await?;
    assert_eq!(received.progress, end);
    assert!(received.projection.changes.is_empty());
    store
        .author_on(&tx, leaf.candidates[0].key.clone(), None)
        .await?;
    let empty_view = sync_orm::causal_snapshot::open_bounded_on(
        &tx,
        store.identity_on(&tx).await?,
        snapshot.scope.clone(),
        30,
        100,
        WorkLimits {
            max_requests: 128,
            max_bytes: 1024 * 1024,
            max_depth: 64,
            max_rows_scanned: 1024,
            max_mutations: 1024,
        },
    )
    .await?;
    assert_eq!(empty_view.roots[0].candidate_count, 0);
    let empty_context = sync_orm::causal_snapshot::context_page_on::<Notes>(
        &tx,
        ns,
        empty_view.snapshot_id,
        31,
        empty_view.roots[0].range.clone(),
        protocol::ContextCursor {
            segment_index: 0,
            next_lower: None,
        },
        protocol::PageLimits {
            max_scanned: 100,
            max_bytes: 1024 * 1024,
        },
    )
    .await?;
    let lease = sync_orm::causal_leases::ready_on(&tx, ns, empty_view.snapshot_id, 31).await?;
    let root_digest = lease.retained_root.unwrap().try_into().unwrap();
    let (_, root_body) =
        sync_orm::causal_manifests::read_on(&tx, ns, root_digest, 1024 * 1024, 100).await?;
    let root = protocol::decode_context_root::<Notes>(&empty_view.scope, &root_body)?;
    let destination_tx = target.begin().await?;
    for segment in &empty_context.segments {
        let (manifest, bytes) = sync_orm::causal_manifests::read_on(
            &tx,
            ns,
            segment.context.manifest.manifest_digest,
            1024 * 1024,
            100,
        )
        .await?;
        let (copy, parts) = sync_orm::causal_manifests::fragment_body(
            protocol::ManifestKind::Context,
            &bytes,
            manifest.item_count,
            1024 * 1024,
        )?;
        sync_orm::causal_manifests::publish_on(
            &destination_tx,
            ns,
            &copy,
            &parts,
            1024 * 1024,
            |body| {
                protocol::decode_context(body)?;
                Ok(vec![])
            },
        )
        .await?;
    }
    let part_ref = root.partitions[0].partition.clone();
    let (part, part_bytes) =
        sync_orm::causal_manifests::read_on(&tx, ns, part_ref.manifest_digest, 1024 * 1024, 100)
            .await?;
    let (copy, parts) = sync_orm::causal_manifests::fragment_body(
        protocol::ManifestKind::ContextPartition,
        &part_bytes,
        part.item_count,
        1024 * 1024,
    )?;
    sync_orm::causal_manifests::publish_on(
        &destination_tx,
        ns,
        &copy,
        &parts,
        1024 * 1024,
        |body| {
            Ok(
                protocol::decode_context_partition::<Notes>(&empty_view.roots[0].range, body)?
                    .iter()
                    .map(|s| s.context.manifest.manifest_digest)
                    .collect(),
            )
        },
    )
    .await?;
    let empty_body = protocol::canonical_candidates::<Notes>(&[])?;
    let (empty_live, parts) = sync_orm::causal_manifests::fragment_body(
        protocol::ManifestKind::Leaf,
        &empty_body,
        0,
        1024,
    )?;
    sync_orm::causal_manifests::publish_on(
        &destination_tx,
        ns,
        &empty_live,
        &parts,
        1024 * 1024,
        |body| {
            protocol::decode_candidates::<Notes>(body)?;
            Ok(vec![])
        },
    )
    .await?;
    let deletion = protocol::RangeEvent {
        range: empty_view.roots[0].range.clone(),
        scope: empty_view.scope.clone(),
        context_partition: part_ref,
        source: protocol::SnapshotRef {
            source: empty_view.source.clone(),
            snapshot_id: empty_view.snapshot_id,
            head: empty_view.head.clone(),
        },
        range_summary: empty_view.roots[0].aggregate,
        live_manifest: empty_live,
    };
    let evidence =
        sync_orm::causal_range::validate_on(&destination_tx, ns, &deletion, 1024 * 1024, 100, 100)
            .await?;
    let removed = receiver
        .apply_range_on(&destination_tx, &evidence, limits)
        .await?;
    assert_eq!(removed.changes.len(), 1);
    assert!(removed.changes[0].before.is_some() && removed.changes[0].after.is_none());
    destination_tx.commit().await?;
    assert!(
        receiver
            .get_on(&target, &leaf.candidates[0].key)
            .await?
            .is_none()
    );
    range_event.range_summary = [7; 32];
    assert!(
        sync_orm::causal_range::validate_on(&tx, ns, &range_event, 1024 * 1024, 100, 100)
            .await
            .is_err()
    );
    let fragment = sync_orm::causal_snapshot::fragment_on::<Notes>(
        &tx,
        ns,
        snapshot.snapshot_id,
        21,
        context.segments[0].context.manifest.clone(),
        0,
        1024 * 1024,
    )
    .await?;
    assert_eq!(
        protocol::context_digest(&protocol::decode_context(&fragment.bytes)?)?,
        context.segments[0].context.digest
    );
    assert!(
        sync_orm::causal_snapshot::fragment_on::<Notes>(
            &tx,
            ns,
            snapshot.snapshot_id,
            120,
            context.segments[0].context.manifest.clone(),
            0,
            1024 * 1024
        )
        .await
        .is_err()
    );
    assert!(
        sync_orm::causal_snapshot::leaf_on::<Notes>(
            &tx,
            ns,
            snapshot.snapshot_id,
            120,
            snapshot.roots[0].range.clone(),
            None,
            protocol::PageLimits {
                max_scanned: 10,
                max_bytes: 1024 * 1024
            }
        )
        .await
        .is_err()
    );
    range_event.range_summary = snapshot.roots[0].aggregate;
    let candidate = &leaf.candidates[0];
    let update = Query::update()
        .table(Alias::new("sync_candidates"))
        .value(Alias::new("body"), vec![0_u8])
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(
            Expr::col(Alias::new("writer")).eq(candidate
                .dot
                .writer_incarnation
                .as_bytes()
                .to_vec()),
        )
        .and_where(Expr::col(Alias::new("counter")).eq(candidate.dot.counter as i64))
        .to_owned();
    assert_eq!(tx.execute(&update).await?.rows_affected(), 1);
    assert!(
        sync_orm::causal_range::validate_on(&tx, ns, &range_event, 1024 * 1024, 100, 100)
            .await
            .is_err()
    );
    let restore = Query::update()
        .table(Alias::new("sync_candidates"))
        .value(
            Alias::new("body"),
            protocol::canonical_candidate(candidate)?,
        )
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(
            Expr::col(Alias::new("writer")).eq(candidate
                .dot
                .writer_incarnation
                .as_bytes()
                .to_vec()),
        )
        .and_where(Expr::col(Alias::new("counter")).eq(candidate.dot.counter as i64))
        .to_owned();
    tx.execute(&restore).await?;
    let delete = Query::delete()
        .from_table(Alias::new("sync_manifest_edges"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(
            Expr::col(Alias::new("parent_manifest"))
                .eq(range_event.context_partition.manifest_digest.to_vec()),
        )
        .to_owned();
    tx.execute(&delete).await?;
    assert!(
        sync_orm::causal_range::validate_on(&tx, ns, &range_event, 1024 * 1024, 100, 100)
            .await
            .is_err()
    );
    tx.rollback().await?;
    drop(store);
    a.close().await?;
    b.close().await?;
    std::fs::remove_file(path)?;
    Ok(())
}
