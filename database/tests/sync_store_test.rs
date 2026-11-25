use anyhow::Result;
use database::sync::{SeaOrmSyncStore, model::*};
use database::test_fixtures::causal_library as support;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection};
use std::sync::Arc;
use support::{Peer, fixtures};
use sync::{self as p};
use sync_orm::causal_capabilities::CausalEndpoint;
use uuid::Uuid;
fn key() -> ObjectKey {
    support::key(Collection::Playlists, Uuid::new_v4())
}
fn scope() -> Scope {
    Scope {
        collections: [Collection::Playlists].into_iter().collect(),
    }
}
async fn migrate(db: &DatabaseConnection) -> Result<()> {
    fixtures::migrate(db, Uuid::new_v4()).await
}
async fn create(db: DatabaseConnection, ns: Uuid, author: Uuid) -> Result<Peer> {
    Ok(Arc::new(CausalEndpoint::new(
        SeaOrmSyncStore::create_empty(db, ns, author).await?,
    )))
}
async fn reopen(db: DatabaseConnection, ns: Uuid, author: Uuid) -> Result<Peer> {
    Ok(Arc::new(CausalEndpoint::new(
        SeaOrmSyncStore::open(db, ns, author).await?,
    )))
}
async fn initialize(source: &Peer, target: &Peer) -> Result<p::CheckpointKey> {
    support::exchange_scoped(source, target, scope()).await?;
    let identity = source.identity().await?;
    Ok(p::CheckpointKey {
        namespace: identity.namespace,
        source_replica: identity.replica,
        source_generation: identity.generation,
        scope_id: p::scope_id(&scope())?,
    })
}
async fn page(source: &Peer, after: p::ReceiveProgress) -> Result<p::ChangePage<RuneModel>> {
    let p::PageRead::Page(page) = source
        .read_page(p::PageRequest {
            source: source.identity().await?,
            scope: scope(),
            after,
            through: source.head().await?,
            limits: p::PageLimits {
                max_scanned: 8,
                max_bytes: 65536,
            },
        })
        .await?
    else {
        anyhow::bail!("fixture exceeded one page");
    };
    Ok(page)
}
#[tokio::test]
async fn persistent_store_replays_and_isolates_peers_and_libraries() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("sync.sqlite").display()
    );
    let db = Database::connect(&url).await?;
    migrate(&db).await?;
    let target_db = Database::connect("sqlite::memory:").await?;
    migrate(&target_db).await?;
    let ns = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let source = create(db.clone(), ns, Uuid::new_v4()).await?;
    let target = create(target_db.clone(), ns, actor).await?;
    let other = create(db.clone(), Uuid::new_v4(), Uuid::new_v4()).await?;
    let ck = initialize(&source, &target).await?;
    let after = target.checkpoint(&ck).await?.unwrap();
    let object = key();
    let record = source
        .write_local(object.clone(), support::playlist("first"))
        .await?;
    let page = page(&source, after).await?;
    assert!(
        other
            .apply_page(&page, support::limits().apply)
            .await
            .is_err()
    );
    target.apply_page(&page, support::limits().apply).await?;
    let head = target.head().await?;
    target.apply_page(&page, support::limits().apply).await?;
    assert_eq!(target.head().await?, head);
    let identity = target.identity().await?;
    let target = reopen(target_db.clone(), ns, Uuid::new_v4()).await?;
    assert_eq!(target.identity().await?, identity);
    assert_eq!(target.get(&object).await?, Some(record));
    assert_eq!(
        reopen(target_db, ns, actor).await?.identity().await?,
        identity
    );
    assert_eq!(
        target
            .write_local(object.clone(), support::playlist("second"))
            .await?
            .revision
            .counter,
        2
    );
    let third_db = Database::connect("sqlite::memory:").await?;
    migrate(&third_db).await?;
    let third = create(third_db, ns, Uuid::new_v4()).await?;
    support::exchange_scoped(&source, &third, scope()).await?;
    assert_eq!(third.get(&object).await?, source.get(&object).await?);
    assert!(other.get(&object).await?.is_none());
    Ok(())
}
#[tokio::test]
async fn rejected_page_and_failed_transaction_leave_no_partial_state() -> Result<()> {
    let ns = Uuid::new_v4();
    let a = support::memory(ns, Uuid::new_v4())?;
    let db = Database::connect("sqlite::memory:").await?;
    migrate(&db).await?;
    let b = create(db.clone(), ns, Uuid::new_v4()).await?;
    let ck = initialize(&a, &b).await?;
    let after = b.checkpoint(&ck).await?.unwrap();
    let initial = b.head().await?;
    let first = key();
    a.write_local(first.clone(), support::playlist("one"))
        .await?;
    a.write_local(key(), support::playlist("two")).await?;
    let original = page(&a, after.clone()).await?;
    let mut bad = original.clone();
    let Some(p::EventRef::Inline(p::Event::Key(delta))) = &mut bad.links[1].event else {
        unreachable!()
    };
    delta.added[0].payload = support::track();
    assert!(b.apply_page(&bad, support::limits().apply).await.is_err());
    assert_eq!(b.head().await?, initial);
    assert!(b.get(&first).await?.is_none());
    assert_eq!(b.checkpoint(&ck).await?, Some(after.clone()));
    db.execute_unprepared("CREATE TRIGGER reject_sync_journal BEFORE INSERT ON sync_journal WHEN NEW.sequence=2 BEGIN SELECT RAISE(ABORT, 'injected failure'); END").await?;
    assert!(
        b.apply_page(&original, support::limits().apply)
            .await
            .is_err()
    );
    assert_eq!(b.head().await?, initial);
    assert!(b.get(&first).await?.is_none());
    assert_eq!(b.checkpoint(&ck).await?, Some(after));
    db.execute_unprepared("DROP TRIGGER reject_sync_journal")
        .await?;
    assert_eq!(
        b.write_local(key(), support::playlist("local"))
            .await?
            .revision
            .counter,
        1
    );
    Ok(())
}
#[tokio::test]
async fn independent_connections_do_not_lose_concurrent_writes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("concurrent.sqlite").display()
    );
    let db1 = Database::connect(&url).await?;
    migrate(&db1).await?;
    let db2 = Database::connect(&url).await?;
    let ns = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let a = create(db1, ns, actor).await?;
    let b = reopen(db2, ns, actor).await?;
    let k1 = key();
    let k2 = key();
    let (r1, r2) = tokio::join!(
        a.write_local(k1.clone(), support::playlist("one")),
        b.write_local(k2.clone(), support::playlist("two"))
    );
    let successes = usize::from(r1.is_ok()) + usize::from(r2.is_ok());
    assert!(successes > 0);
    assert_eq!(a.head().await?.sequence, successes as u64);
    assert_eq!(a.get(&k1).await?.is_some(), r1.is_ok());
    assert_eq!(a.get(&k2).await?.is_some(), r2.is_ok());
    if r1.is_err() {
        a.write_local(k1, support::playlist("one")).await?;
    }
    if r2.is_err() {
        b.write_local(k2, support::playlist("two")).await?;
    }
    assert_eq!(a.head().await?.sequence, 2);
    Ok(())
}
#[tokio::test]
async fn filtered_pages_advance_and_reopen_preserves_history() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    migrate(&db).await?;
    let ns = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let store = create(db.clone(), ns, actor).await?;
    let initial = store.head().await?;
    store
        .write_local(
            support::key(Collection::Tracks, Uuid::new_v4()),
            Some(support::track()),
        )
        .await?;
    let object = key();
    store
        .write_local(object.clone(), support::playlist("playlist"))
        .await?;
    let identity = store.identity().await?;
    let request = p::PageRequest {
        source: identity.clone(),
        scope: scope(),
        after: initial,
        through: store.head().await?,
        limits: p::PageLimits {
            max_scanned: 1,
            max_bytes: 65536,
        },
    };
    let p::PageRead::Page(first) = store.read_page(request.clone()).await? else {
        unreachable!()
    };
    assert!(first.links.iter().all(|link| link.event.is_none()));
    assert_eq!(first.next.sequence, 1);
    let p::PageRead::Page(second) = store
        .read_page(p::PageRequest {
            after: first.next.clone(),
            ..request.clone()
        })
        .await?
    else {
        unreachable!()
    };
    assert!(second.done);
    let Some(p::EventRef::Inline(p::Event::Key(delta))) = &second.links[0].event else {
        unreachable!()
    };
    assert_eq!(delta.key, object);
    let reopened = reopen(db, ns, actor).await?;
    assert_eq!(reopened.identity().await?, identity);
    assert_eq!(
        reopened.read_page(request.clone()).await?,
        p::PageRead::Page(first)
    );
    let mut stale = request;
    stale.source.generation = Uuid::new_v4();
    assert!(reopened.read_page(stale).await.is_err());
    Ok(())
}
#[tokio::test]
async fn closed_database_reopens_with_committed_removal_and_logical_clock() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("restart.sqlite").display()
    );
    let db = Database::connect(&url).await?;
    migrate(&db).await?;
    let ns = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let source = support::memory(ns, Uuid::new_v4())?;
    let target = create(db.clone(), ns, actor).await?;
    let ck = initialize(&source, &target).await?;
    let after = target.checkpoint(&ck).await?.unwrap();
    let object = key();
    source
        .write_local(object.clone(), support::playlist("before removal"))
        .await?;
    source.write_local(object.clone(), None).await?;
    let page = page(&source, after).await?;
    let identity = target.identity().await?;
    target.apply_page(&page, support::limits().apply).await?;
    let head = target.head().await?;
    drop(target);
    db.close().await?;
    let db = Database::connect(&url).await?;
    let target = reopen(db, ns, actor).await?;
    assert_eq!(target.identity().await?, identity);
    assert_eq!(target.checkpoint(&ck).await?, Some(page.next.clone()));
    assert!(target.get(&object).await?.is_none());
    target.apply_page(&page, support::limits().apply).await?;
    assert_eq!(target.head().await?, head);
    assert_eq!(
        target
            .write_local(object, support::playlist("restored"))
            .await?
            .revision
            .counter,
        3
    );
    Ok(())
}
