use anyhow::Result;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, EntityTrait, ExprTrait, PaginatorTrait, Set,
    TransactionTrait,
    sea_query::{Alias, Expr, Query},
};
use std::sync::Arc;
use sync::{self as p, ReceiveStore, SyncStore};
use sync_orm::{
    OrmReplica,
    causal_capabilities::CausalEndpoint,
    registry::{ProjectionBudget, RegistryProjection},
};
use uuid::Uuid;

mod notes {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "notes")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        #[sea_orm(unique)]
        pub uuid: String,
        pub text: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
sync_orm::row_policies! { pub Notes, Collection, Payload {
    model: "test.notes", schema: "1", collections: {
        Notes => { tag: "notes", entity: notes::Entity, value: Note, mapping: NoteMap,
            policy: { uuid: notes::Column::Uuid, fields: { text: String => notes::Column::Text },
                defaults: {}, limits: { text => (1, 128) }, }
        }
    }
} }
fn key(n: u128) -> p::ObjectKey<Notes> {
    p::ObjectKey {
        collection: Collection::Notes,
        id: Uuid::from_u128(n),
    }
}
fn note(text: &str) -> Option<Payload> {
    Some(Payload::Note(Note { text: text.into() }))
}
fn apply_limits() -> p::ApplyLimits {
    p::ApplyLimits {
        max_projection_items: 2,
        max_projection_bytes: 65536,
        max_mutations: 1024,
    }
}
async fn completion(
    db: &DatabaseConnection,
    checkpoint: &p::CheckpointKey,
) -> Result<Option<String>> {
    Ok(db
        .query_one(
            &Query::select()
                .column(Alias::new("completed_at"))
                .from(Alias::new("sync_checkpoints"))
                .and_where(
                    Expr::col(Alias::new("namespace")).eq(checkpoint.namespace.as_bytes().to_vec()),
                )
                .and_where(
                    Expr::col(Alias::new("source_replica"))
                        .eq(checkpoint.source_replica.as_bytes().to_vec()),
                )
                .to_owned(),
        )
        .await?
        .unwrap()
        .try_get_by_index(0)?)
}
async fn apply_on(
    store: &OrmReplica<Notes>,
    tx: &sea_orm::DatabaseTransaction,
    page: &p::ChangePage<Notes>,
) -> Result<()> {
    let result = store.apply_page_on(tx, page, apply_limits()).await?;
    for change in result.projection.changes {
        Notes::project_on(
            tx,
            &change.key,
            change.after.as_ref().map(|value| &value.payload),
        )
        .await?;
    }
    Ok(())
}
async fn page(
    source: &CausalEndpoint<Notes>,
    request: p::PageRequest<Notes>,
) -> Result<p::ChangePage<Notes>> {
    let p::PageRead::Page(page) = source.read_page(request).await? else {
        anyhow::bail!("fixture exceeded one page");
    };
    Ok(page)
}

pub async fn verify(a: DatabaseConnection, b: DatabaseConnection) -> Result<()> {
    for db in [&a, &b] {
        db.execute(
            &sea_orm::Schema::new(db.get_database_backend())
                .create_table_from_entity(notes::Entity),
        )
        .await?;
    }
    let namespace = Uuid::new_v4();
    let source = OrmReplica::<Notes>::create_empty(a.clone(), namespace, Uuid::new_v4()).await?;
    let target = OrmReplica::<Notes>::create_empty(b.clone(), namespace, Uuid::new_v4()).await?;
    let projection = || {
        Arc::new(
            RegistryProjection::new(ProjectionBudget {
                max_reads: 1024,
                max_mutations: 1024,
            })
            .expect("valid note registry"),
        )
    };
    let source = CausalEndpoint::with_projection(source, projection());
    let target = CausalEndpoint::with_projection(target, projection());
    let scope = p::Scope {
        collections: [Collection::Notes].into_iter().collect(),
    };
    let identity = source.identity().await?;
    let checkpoint = p::CheckpointKey {
        namespace,
        source_replica: identity.replica,
        source_generation: identity.generation,
        scope_id: p::scope_id(&scope)?,
    };
    let after = source.head().await?;
    let target_id = target.identity().await?;
    let tx = b.begin().await?;
    sync_orm::causal_progress::initialize_checkpoint_on(
        &tx,
        &target_id,
        &checkpoint,
        &scope,
        &after,
    )
    .await?;
    tx.commit().await?;
    let tx = a.begin().await?;
    source.store().author_on(&tx, key(4), note("hello")).await?;
    Notes::project_on(&tx, &key(4), note("hello").as_ref()).await?;
    tx.rollback().await?;
    assert_eq!(notes::Entity::find().count(&a).await?, 0);
    assert_eq!(source.head().await?, after);
    source.write_local(key(4), note("hello")).await?;
    let mut request = p::PageRequest {
        source: identity,
        scope,
        after: after.clone(),
        through: source.head().await?,
        limits: p::PageLimits {
            max_scanned: 1,
            max_bytes: 65536,
        },
    };
    let first = page(&source, request.clone()).await?;
    let initial_completion = completion(&b, &checkpoint).await?;
    let tx = b.begin().await?;
    apply_on(&target.store(), &tx, &first).await?;
    tx.rollback().await?;
    assert_eq!(notes::Entity::find().count(&b).await?, 0);
    assert!(target.get(&key(4)).await?.is_none());
    assert_eq!(target.checkpoint(&checkpoint).await?, Some(after));
    assert_eq!(completion(&b, &checkpoint).await?, initial_completion);
    target.apply_page(&first, apply_limits()).await?;
    let head = target.head().await?;
    target.apply_page(&first, apply_limits()).await?;
    assert_eq!(target.head().await?, head);
    assert_eq!(notes::Entity::find().count(&b).await?, 1);
    let completed = completion(&b, &checkpoint).await?.unwrap();
    source.write_local(key(5), note("five")).await?;
    source.write_local(key(6), note("six")).await?;
    request.after = first.next;
    request.through = source.head().await?;
    let partial = page(&source, request.clone()).await?;
    assert!(!partial.done);
    target.apply_page(&partial, apply_limits()).await?;
    assert_eq!(
        completion(&b, &checkpoint).await?.as_deref(),
        Some(completed.as_str())
    );
    request.after = partial.next.clone();
    let last = page(&source, request).await?;
    assert!(last.done);
    let tx = b.begin().await?;
    apply_on(&target.store(), &tx, &last).await?;
    assert!(
        notes::Entity::insert(notes::ActiveModel {
            id: Set(1),
            uuid: Set(key(4).id.to_string()),
            text: Set("duplicate".into())
        })
        .exec(&tx)
        .await
        .is_err()
    );
    tx.rollback().await?;
    assert!(target.get(&key(6)).await?.is_none());
    assert_eq!(notes::Entity::find().count(&b).await?, 2);
    assert_eq!(target.checkpoint(&checkpoint).await?, Some(partial.next));
    assert_eq!(
        completion(&b, &checkpoint).await?.as_deref(),
        Some(completed.as_str())
    );
    target.apply_page(&last, apply_limits()).await?;
    assert_eq!(notes::Entity::find().count(&b).await?, 3);
    assert_ne!(
        completion(&b, &checkpoint).await?.as_deref(),
        Some(completed.as_str())
    );
    let empty = Arc::new(p::MemorySyncStore::<Notes>::new(
        p::StoreIdentity {
            namespace,
            model: p::ModelId {
                model: "test.notes".into(),
                schema: "1".into(),
            },
            replica: Uuid::new_v4(),
            generation: Uuid::new_v4(),
        },
        p::LocalAuthor {
            author: Uuid::new_v4(),
            writer_incarnation: Uuid::new_v4(),
        },
    )?);
    let identity = empty.identity().await?;
    let scope = p::Scope {
        collections: [Collection::Notes].into_iter().collect(),
    };
    let empty_checkpoint = p::CheckpointKey {
        namespace,
        source_replica: identity.replica,
        source_generation: identity.generation,
        scope_id: p::scope_id(&scope)?,
    };
    let mut coordinator = p::Coordinator::new(
        scope,
        p::SessionLimits {
            pages: p::PageLimits {
                max_scanned: 4,
                max_bytes: 65536,
            },
            apply: apply_limits(),
            stages: p::StageLimits {
                max_total_bytes: p::DEFAULT_STAGE_BYTES,
                max_stages: p::DEFAULT_STAGES,
                max_event_bytes: p::DEFAULT_LOGICAL_BYTES,
                max_event_items: p::DEFAULT_LOGICAL_ITEMS,
                max_fragment_bytes: 65536,
            },
            work: p::WorkLimits {
                max_requests: 32,
                max_bytes: 1024 * 1024,
                max_depth: 32,
                max_rows_scanned: 16,
                max_mutations: 1024,
            },
        },
    )?;
    let peer = p::StorePeer(empty.clone());
    let head = target.head().await?;
    let mut complete = false;
    for _ in 0..20000 {
        match coordinator
            .advance(&target, &target, &target, &peer, &*empty)
            .await?
        {
            p::SessionAdvance::Complete(_) => {
                complete = true;
                break;
            }
            p::SessionAdvance::BudgetExhausted => coordinator.start_pass(),
            p::SessionAdvance::Incomplete => (),
            p::SessionAdvance::Cancelled => anyhow::bail!("unexpected cancellation"),
        }
    }
    assert!(complete);
    assert_eq!(target.head().await?, head);
    assert_eq!(notes::Entity::find().count(&b).await?, 3);
    assert_eq!(
        target.checkpoint(&empty_checkpoint).await?,
        Some(empty.head().await?)
    );
    assert!(completion(&b, &empty_checkpoint).await?.is_some());
    Ok(())
}
