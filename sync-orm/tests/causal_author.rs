#[path = "support/fixtures.rs"]
mod fixtures;

use anyhow::Result;
use sea_orm::{
    ConnectionTrait, Database, ExprTrait, TransactionTrait,
    sea_query::{Alias, ColumnDef, Expr, Query, Table},
};
use serde::{Deserialize, Serialize};
use sync::causal::Context;
use sync::causal::Dot;
use sync::protocol::{self, ObjectKey, Observed, SyncModel};
use sync_orm::causal_compaction::FloorStep;
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
    let db = fixtures::fresh_database().await?;
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
    assert_eq!(replica.writer_incarnation(), reopened.writer_incarnation());
    let mut incarnation = reopened.writer_incarnation();
    for _ in 0..9 {
        let reopened = CausalReplica::<Notes>::open(db.clone(), namespace, actor).await?;
        assert_eq!(incarnation, reopened.writer_incarnation());
        incarnation = reopened.writer_incarnation();
    }
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
async fn ten_reopens_author_with_one_writer_context() -> Result<()> {
    let db = fixtures::fresh_database().await?;
    let namespace = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let mut replica = CausalReplica::<Notes>::create_empty(db.clone(), namespace, actor).await?;
    for index in 0..10 {
        if index != 0 {
            replica = CausalReplica::<Notes>::open(db.clone(), namespace, actor).await?;
        }
        let tx = db.begin().await?;
        replica
            .write_observed_on(
                &tx,
                ObjectKey {
                    collection: Collection::Notes,
                    id: Uuid::new_v4(),
                },
                Some(Note {
                    text: format!("note {index}"),
                }),
                Observed::AllCurrent,
            )
            .await?;
        tx.commit().await?;
    }
    let rows = db
        .query_all(
            &Query::select()
                .column(Alias::new("writer"))
                .from(Alias::new("sync_candidates"))
                .to_owned(),
        )
        .await?;
    let writers = rows
        .into_iter()
        .map(|row| row.try_get_by_index::<Vec<u8>>(0))
        .collect::<std::result::Result<std::collections::BTreeSet<_>, _>>()?;
    assert_eq!(writers.len(), 1);
    Ok(())
}

#[tokio::test]
async fn abandoned_reservation_is_skipped_after_reopen() -> Result<()> {
    let db = fixtures::fresh_database().await?;
    let namespace = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let initial = CausalReplica::<Notes>::create_empty(db.clone(), namespace, actor).await?;
    let incarnation = initial.writer_incarnation();
    drop(initial);
    let crashed = CausalReplica::<Notes>::open(db.clone(), namespace, actor).await?;
    assert_eq!(crashed.writer_incarnation(), incarnation);
    drop(crashed);
    let recovered = CausalReplica::<Notes>::open(db.clone(), namespace, actor).await?;
    let tx = db.begin().await?;
    recovered
        .write_observed_on(
            &tx,
            ObjectKey {
                collection: Collection::Notes,
                id: Uuid::new_v4(),
            },
            Some(Note {
                text: "after crash".into(),
            }),
            Observed::AllCurrent,
        )
        .await?;
    tx.commit().await?;
    let row = db
        .query_one(
            &Query::select()
                .columns(["writer", "counter"].map(Alias::new))
                .from(Alias::new("sync_candidates"))
                .to_owned(),
        )
        .await?
        .expect("authored candidate exists");
    assert_eq!(
        Uuid::from_slice(&row.try_get_by_index::<Vec<u8>>(0)?)?,
        incarnation
    );
    assert!(row.try_get_by_index::<i64>(1)? > 1_000_000_000);
    Ok(())
}

#[tokio::test]
async fn copied_store_rotates_for_another_author_and_both_continue() -> Result<()> {
    let original_path =
        std::env::temp_dir().join(format!("sync-writer-original-{}.sqlite", Uuid::new_v4()));
    let copy_path =
        std::env::temp_dir().join(format!("sync-writer-copy-{}.sqlite", Uuid::new_v4()));
    let original_url = format!("sqlite://{}?mode=rwc", original_path.display());
    let copy_url = format!("sqlite://{}?mode=rwc", copy_path.display());
    let original_db = Database::connect(&original_url).await?;
    causal_schema::install_fresh(&original_db).await?;
    let namespace = Uuid::new_v4();
    let author_a = Uuid::new_v4();
    let author_b = Uuid::new_v4();
    let created =
        CausalReplica::<Notes>::create_empty(original_db.clone(), namespace, author_a).await?;
    let original_incarnation = created.writer_incarnation();
    drop(created);
    original_db.close().await?;
    std::fs::copy(&original_path, &copy_path)?;
    let original_db = Database::connect(&original_url).await?;
    let copy_db = Database::connect(&copy_url).await?;
    let original = CausalReplica::<Notes>::open(original_db.clone(), namespace, author_a).await?;
    let copied = CausalReplica::<Notes>::open(copy_db.clone(), namespace, author_b).await?;
    assert_eq!(original.writer_incarnation(), original_incarnation);
    assert_ne!(copied.writer_incarnation(), original.writer_incarnation());
    for (replica, db, note) in [
        (&original, &original_db, "original"),
        (&copied, &copy_db, "copy"),
    ] {
        let tx = db.begin().await?;
        replica
            .write_observed_on(
                &tx,
                ObjectKey {
                    collection: Collection::Notes,
                    id: Uuid::new_v4(),
                },
                Some(Note { text: note.into() }),
                Observed::AllCurrent,
            )
            .await?;
        tx.commit().await?;
    }
    original_db.close().await?;
    copy_db.close().await?;
    let _ = std::fs::remove_file(original_path);
    let _ = std::fs::remove_file(copy_path);
    Ok(())
}

#[tokio::test]
async fn legacy_store_adopts_and_retains_writer_incarnation() -> Result<()> {
    let db = fixtures::fresh_database().await?;
    let namespace = Uuid::new_v4();
    let actor = Uuid::new_v4();
    CausalReplica::<Notes>::create_empty(db.clone(), namespace, actor).await?;
    db.execute_unprepared("ALTER TABLE sync_meta DROP COLUMN writer_incarnation")
        .await?;
    db.execute_unprepared("ALTER TABLE sync_meta DROP COLUMN writer_author")
        .await?;
    db.execute_unprepared("ALTER TABLE sync_meta DROP COLUMN writer_reserved_counter")
        .await?;
    let first = CausalReplica::<Notes>::open(db.clone(), namespace, actor).await?;
    let adopted = first.writer_incarnation();
    drop(first);
    let second = CausalReplica::<Notes>::open(db.clone(), namespace, actor).await?;
    assert_eq!(second.writer_incarnation(), adopted);
    Ok(())
}

#[tokio::test]
async fn restored_store_rotates_when_config_sidecar_token_changes() -> Result<()> {
    let db = fixtures::fresh_database().await?;
    let namespace = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let file_token = Uuid::new_v4();
    let original = CausalReplica::<Notes>::create_empty_with_file_token(
        db.clone(),
        namespace,
        actor,
        Some(file_token),
    )
    .await?;
    let original_incarnation = original.writer_incarnation();
    let tx = db.begin().await?;
    original
        .write_observed_on(
            &tx,
            ObjectKey {
                collection: Collection::Notes,
                id: Uuid::new_v4(),
            },
            Some(Note {
                text: "before restore".into(),
            }),
            Observed::AllCurrent,
        )
        .await?;
    tx.commit().await?;
    let restored = CausalReplica::<Notes>::open_with_file_token(
        db.clone(),
        namespace,
        actor,
        Some(Uuid::new_v4()),
    )
    .await?;
    assert_ne!(restored.writer_incarnation(), original_incarnation);
    let old_dot = db
        .query_one(
            &Query::select()
                .column(Alias::new("writer"))
                .from(Alias::new("sync_candidates"))
                .to_owned(),
        )
        .await?
        .expect("pre-restore causal dot remains stored")
        .try_get_by_index::<Vec<u8>>(0)?;
    assert_eq!(Uuid::from_slice(&old_dot)?, original_incarnation);
    Ok(())
}

#[tokio::test]
async fn inconsistent_counter_reservation_rotates_even_after_candidate_reclamation() -> Result<()> {
    let db = fixtures::fresh_database().await?;
    let namespace = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let replica = CausalReplica::<Notes>::create_empty(db.clone(), namespace, actor).await?;
    let incarnation = replica.writer_incarnation();
    let key = ObjectKey {
        collection: Collection::Notes,
        id: Uuid::new_v4(),
    };
    let tx = db.begin().await?;
    replica
        .write_observed_on(
            &tx,
            key.clone(),
            Some(Note {
                text: "value".into(),
            }),
            Observed::AllCurrent,
        )
        .await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    replica
        .write_observed_on(&tx, key, None, Observed::AllCurrent)
        .await?;
    tx.commit().await?;
    db.execute(
        &Query::delete()
            .from_table(Alias::new("sync_candidates"))
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
            .to_owned(),
    )
    .await?;
    db.execute(
        &Query::update()
            .table(Alias::new("sync_meta"))
            .value(Alias::new("writer_reserved_counter"), 0_i64)
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
            .to_owned(),
    )
    .await?;
    let reopened = CausalReplica::<Notes>::open(db, namespace, actor).await?;
    assert_ne!(reopened.writer_incarnation(), incarnation);
    Ok(())
}

#[tokio::test]
async fn keyed_receive_retains_original_author_and_never_resurrects_observed_dot() -> Result<()> {
    use sync::protocol::{Candidate, Dot, KeyDelta, ModelId, Revision};
    let db = fixtures::fresh_database().await?;
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
        Candidate, Dot, ExactContext, KeyDelta, ModelId, Revision, WriterContext,
    };
    let db = fixtures::fresh_database().await?;
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
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    sync_orm::causal_observed::insert_on(
        &tx,
        ns,
        b"notes",
        1,
        &protocol::causal_context(&context)?,
        &mut 1024,
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
    let db = fixtures::fresh_database().await?;
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
    let db = fixtures::fresh_database().await?;
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
async fn observed_context_is_collection_wide_and_suppresses_late_candidates() -> Result<()> {
    let db = fixtures::fresh_database().await?;
    let ns = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let store = CausalReplica::<Notes>::create_empty(db.clone(), ns, actor).await?;
    let key = ObjectKey {
        collection: Collection::Notes,
        id: Uuid::new_v4(),
    };
    let tx = db.begin().await?;
    let initial = store
        .write_local_projected_on(&tx, key.clone(), Some(Note { text: "old".into() }))
        .await?
        .projection
        .after
        .unwrap();
    tx.commit().await?;
    let mut context = Context::default();
    context.insert(Dot::new(
        initial.dot.writer_incarnation,
        initial.dot.counter,
    )?)?;
    let delta = protocol::MergeDelta {
        collection: Collection::Notes,
        added: vec![],
        observed: protocol::exact_context(&context),
    };
    let limits = protocol::ApplyLimits {
        max_projection_items: 100,
        max_projection_bytes: 65536,
        max_mutations: 1024,
    };
    let tx = db.begin().await?;
    let (changed, projection) = store.apply_merge_on(&tx, &delta, limits, &mut 1024).await?;
    assert!(changed);
    assert!(projection.changes[0].after.is_none());
    tx.commit().await?;
    let tx = db.begin().await?;
    assert!(
        !store
            .apply_merge_on(&tx, &delta, limits, &mut 1024)
            .await?
            .0
    );
    assert!(
        sync_orm::causal_observed::contains_on(
            &tx,
            ns,
            b"notes",
            Dot::new(initial.dot.writer_incarnation, initial.dot.counter)?
        )
        .await?
    );
    let stale = protocol::KeyDelta {
        key: key.clone(),
        author: initial.revision,
        removed: vec![],
        added: vec![initial.clone()],
    };
    assert!(store.apply_key_on(&tx, &stale).await?.after.is_none());
    let mut bad = initial;
    bad.payload.text = "conflicting".into();
    assert!(matches!(
        store
            .apply_merge_on(
                &tx,
                &protocol::MergeDelta {
                    collection: Collection::Notes,
                    added: vec![bad],
                    observed: protocol::ExactContext { writers: vec![] }
                },
                limits,
                &mut 1024
            )
            .await
            .unwrap_err()
            .downcast_ref(),
        Some(protocol::SyncError::ConflictingDot)
    ));
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn bounded_floor_compaction_preserves_context_and_respects_a_pin() -> Result<()> {
    use sync::protocol::{KeyDelta, Revision};
    let db = fixtures::fresh_database().await?;
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
    assert_eq!(step, FloorStep::Advanced { state_seq: 1 });
    tx.commit().await?;
    let tx = db.begin().await?;
    let step =
        sync_orm::causal_compaction::maintenance_step_on::<Notes>(&tx, ns, 1, 1024, 1024 * 1024)
            .await?;
    assert_eq!(step, FloorStep::Advanced { state_seq: 2 });
    tx.commit().await?;
    let tx = db.begin().await?;
    assert_eq!(
        sync_orm::causal_compaction::maintenance_step_on::<Notes>(&tx, ns, 1, 1024, 1024 * 1024)
            .await?,
        FloorStep::CaughtUp
    );
    tx.commit().await?;
    let tx = db.begin().await?;
    sync_orm::causal_compaction::prune_history_step_on(&tx, ns, 1024).await?;
    sync_orm::causal_compaction::reclaim_rows_step_on(&tx, ns, 1024).await?;
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
    let context = &snapshot.contexts[0].context;
    let fragment = sync_orm::causal_snapshot::fragment_on::<Notes>(
        &tx,
        ns,
        snapshot.snapshot_id,
        21,
        context.manifest.clone(),
        0,
        1024 * 1024,
    )
    .await?;
    assert_eq!(
        protocol::context_digest(&protocol::decode_context(&fragment.bytes)?)?,
        context.digest
    );
    assert!(
        sync_orm::causal_snapshot::fragment_on::<Notes>(
            &tx,
            ns,
            snapshot.snapshot_id,
            120,
            context.manifest.clone(),
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
    tx.rollback().await?;
    drop(store);
    a.close().await?;
    b.close().await?;
    std::fs::remove_file(path)?;
    Ok(())
}
