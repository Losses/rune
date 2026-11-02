#![cfg(feature = "sync-crash-probe")]
#[path = "support/causal_library.rs"]
mod support;
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use database::{
    entities::{media_file_likes, media_file_stats, media_files},
    sync::{LibrarySyncStore, model::*},
};
use fsio::FsIo;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, EntityTrait,
    PaginatorTrait, Statement, TransactionTrait,
    sea_query::{Alias, Query},
};
use std::{
    path::Path,
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use sync::{self as p, ReceiveStore};
use sync_orm::{
    causal_capabilities::{CausalEndpoint, TransactionProjection},
    registry::{ProjectionBudget, RegistryProjection},
};
use uuid::Uuid;

async fn scalar(db: &DatabaseConnection, sql: &str, name: &str) -> Result<String> {
    let row = db
        .query_one_raw(Statement::from_string(DbBackend::Sqlite, sql))
        .await?
        .context("missing pragma")?;
    Ok(row
        .try_get::<String>("", name)
        .or_else(|_| row.try_get::<i64>("", name).map(|v| v.to_string()))?)
}
async fn connect(root: &Path, node: Uuid) -> Result<DatabaseConnection> {
    database::connection::connect_main_db(
        &FsIo::new(),
        root.to_str().context("non-UTF-8 root")?,
        None,
        &node.to_string(),
    )
    .await
}
fn checkpoint(page: &ChangePage) -> Result<p::CheckpointKey> {
    Ok(p::CheckpointKey {
        namespace: page.request.source.namespace,
        source_replica: page.request.source.replica,
        source_generation: page.request.source.generation,
        scope_id: p::scope_id(&page.request.scope)?,
    })
}
async fn completed_at(db: &DatabaseConnection) -> Result<Option<String>> {
    let row = db
        .query_one(
            &Query::select()
                .column(Alias::new("completed_at"))
                .from(Alias::new("sync_checkpoints"))
                .to_owned(),
        )
        .await?
        .context("checkpoint missing")?;
    Ok(row.try_get_by_index(0)?)
}
async fn fixture(root: &Path) -> Result<(String, Uuid, Uuid, ChangePage)> {
    let library = Uuid::new_v4();
    let node = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let url = format!("sqlite://{}?mode=rwc", root.join(".rune/.0.db").display());
    let db = connect(root, node).await?;
    let journal = scalar(&db, "PRAGMA journal_mode", "journal_mode").await?;
    let synchronous = scalar(&db, "PRAGMA synchronous", "synchronous").await?;
    println!("sqlite_journal_mode={journal} sqlite_synchronous={synchronous}");
    ensure!(
        journal == "delete" && matches!(synchronous.as_str(), "2" | "3"),
        "test requires rollback journal and FULL or EXTRA sync"
    );
    let target = LibrarySyncStore::open(db.clone(), node, Some(library)).await?;
    let source = support::memory(library, actor)?;
    let identity = source.identity().await?;
    let after = source.head().await?;
    let scope = Scope {
        collections: [
            Collection::Tracks,
            Collection::Likes,
            Collection::PlaybackStatistics,
        ]
        .into_iter()
        .collect(),
    };
    let target_identity = target.identity().await?;
    let tx = db.begin().await?;
    sync_orm::causal_progress::initialize_checkpoint_on(
        &tx,
        &target_identity,
        &p::CheckpointKey {
            namespace: library,
            source_replica: identity.replica,
            source_generation: identity.generation,
            scope_id: p::scope_id(&scope)?,
        },
        &scope,
        &after,
    )
    .await?;
    tx.commit().await?;
    std::fs::write(
        root.join("completion.json"),
        serde_json::to_vec(&completed_at(&db).await?)?,
    )?;
    let track = Uuid::new_v4();
    let mut expected = Vec::new();
    expected.push(
        source
            .write_local(
                support::key(Collection::Tracks, track),
                Some(support::track()),
            )
            .await?,
    );
    expected.push(
        source
            .write_local(
                support::key(Collection::Likes, track),
                Some(Payload::Liked(Liked {
                    track_id: track,
                    liked: true,
                    updated_at: "2026-11-01T00:00:00Z".into(),
                })),
            )
            .await?,
    );
    expected.push(
        source
            .write_local(
                support::key(
                    Collection::PlaybackStatistics,
                    Uuid::new_v5(&track, actor.as_bytes()),
                ),
                Some(Payload::PlaybackStatistics(PlaybackStatistics {
                    track_id: track,
                    device_id: actor,
                    skipped: 0,
                    played_through: 1,
                    updated_at: "2026-11-01T00:00:00Z".into(),
                })),
            )
            .await?,
    );
    let p::PageRead::Page(page) = source
        .read_page(PageRequest {
            source: identity,
            scope,
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
    assert!(page.done);
    assert_eq!(page.links.len(), 3);
    std::fs::write(root.join("page.json"), serde_json::to_vec(&page)?)?;
    std::fs::write(root.join("expected.json"), serde_json::to_vec(&expected)?)?;
    drop(target);
    db.close().await?;
    Ok((url, library, node, page))
}
fn stop(stage: &str) {
    if std::env::var("RUNE_SYNC_FAULT_STAGE").ok().as_deref() == Some(stage) {
        let marker = std::env::var_os("RUNE_SYNC_FAULT_MARKER").expect("missing fault marker");
        std::fs::write(marker, stage).expect("fault marker write failed");
        loop {
            std::thread::park();
        }
    }
}
struct PausingProjection(RegistryProjection<RuneModel>);
#[async_trait]
impl TransactionProjection<RuneModel> for PausingProjection {
    fn mutation_ceiling(&self, changes: &p::ProjectionBatch<RuneModel>) -> Result<u64> {
        self.0.mutation_ceiling(changes)
    }
    async fn apply(
        &self,
        tx: &DatabaseTransaction,
        changes: &p::ProjectionBatch<RuneModel>,
    ) -> Result<()> {
        self.0.apply(tx, changes).await?;
        stop("page_before_commit");
        Ok(())
    }
}
#[tokio::test]
#[ignore = "subprocess entry point"]
async fn crash_probe_child() -> Result<()> {
    let root = std::path::PathBuf::from(
        std::env::var_os("RUNE_SYNC_CRASH_ROOT").context("missing child root")?,
    );
    let library: Uuid = std::env::var("RUNE_SYNC_CRASH_LIBRARY")?.parse()?;
    let node: Uuid = std::env::var("RUNE_SYNC_CRASH_NODE")?.parse()?;
    let page: ChangePage = serde_json::from_slice(&std::fs::read(root.join("page.json"))?)?;
    let db = connect(&root, node).await?;
    let store = LibrarySyncStore::open(db, node, Some(library)).await?;
    let endpoint = CausalEndpoint::with_projection(
        store.record_store(),
        Arc::new(PausingProjection(RegistryProjection::new(
            ProjectionBudget {
                max_reads: 65536,
                max_mutations: 65536,
            },
        )?)),
    );
    endpoint
        .apply_page(&page, database::sync::session_limits().apply)
        .await?;
    stop("page_after_commit");
    anyhow::bail!("crash probe returned without being killed")
}
fn kill_at(root: &Path, url: &str, library: Uuid, node: Uuid, phase: &str) -> Result<()> {
    let marker = root.join("blocked");
    let mut child = Command::new(std::env::current_exe()?)
        .args(["--ignored", "--exact", "crash_probe_child", "--nocapture"])
        .env("RUNE_SYNC_CRASH_ROOT", root)
        .env("RUNE_SYNC_CRASH_URL", url)
        .env("RUNE_SYNC_CRASH_LIBRARY", library.to_string())
        .env("RUNE_SYNC_CRASH_NODE", node.to_string())
        .env("RUNE_SYNC_FAULT_STAGE", phase)
        .env("RUNE_SYNC_FAULT_MARKER", &marker)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let start = Instant::now();
    loop {
        if marker.exists() {
            break;
        }
        if let Some(status) = child.try_wait()? {
            anyhow::bail!("child exited before fault point: {status}");
        }
        if start.elapsed() > Duration::from_secs(30) {
            child.kill()?;
            child.wait()?;
            anyhow::bail!("child timed out before fault point");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.kill()?;
    ensure!(!child.wait()?.success(), "child survived termination");
    Ok(())
}
async fn inspect(
    root: &Path,
    _url: &str,
    library: Uuid,
    node: Uuid,
    page: &ChangePage,
    committed: bool,
) -> Result<()> {
    let db = connect(root, node).await?;
    assert_eq!(
        scalar(&db, "PRAGMA integrity_check", "integrity_check").await?,
        "ok"
    );
    let store = LibrarySyncStore::open(db.clone(), node, Some(library)).await?;
    let endpoint = store.endpoint();
    assert_eq!(
        endpoint.checkpoint(&checkpoint(page)?).await?,
        Some(if committed {
            page.next.clone()
        } else {
            page.request.after.clone()
        })
    );
    assert_eq!(store.head().await?.sequence, if committed { 3 } else { 0 });
    assert_eq!(
        media_files::Entity::find().count(&db).await?,
        u64::from(committed)
    );
    assert_eq!(
        media_file_likes::Entity::find().count(&db).await?,
        u64::from(committed)
    );
    assert_eq!(
        media_file_stats::Entity::find().count(&db).await?,
        u64::from(committed)
    );
    let before: Option<String> =
        serde_json::from_slice(&std::fs::read(root.join("completion.json"))?)?;
    if !committed {
        assert_eq!(
            completed_at(&db).await?,
            before,
            "completion advanced without business rows"
        );
    } else {
        assert_ne!(
            completed_at(&db).await?,
            before,
            "completed batch retained the old timestamp"
        );
    }
    let expected: Vec<Record> =
        serde_json::from_slice(&std::fs::read(root.join("expected.json"))?)?;
    for record in &expected {
        assert_eq!(
            store.get(&record.key).await?,
            committed.then(|| record.clone())
        );
    }
    let limits = database::sync::session_limits().apply;
    endpoint.apply_page(page, limits).await?;
    let settled = store.head().await?;
    endpoint.apply_page(page, limits).await?;
    assert_eq!(store.head().await?, settled);
    assert_eq!(settled.sequence, 3);
    for record in &expected {
        assert_eq!(store.get(&record.key).await?, Some(record.clone()));
    }
    assert!(
        !media_files::Entity::find()
            .one(&db)
            .await?
            .unwrap()
            .local_available
    );
    assert!(
        media_file_likes::Entity::find()
            .one(&db)
            .await?
            .unwrap()
            .liked
    );
    assert_eq!(
        media_file_stats::Entity::find()
            .one(&db)
            .await?
            .unwrap()
            .played_through,
        1
    );
    assert!(store.pending_references().await?.is_empty());
    let local = store
        .write_local(
            support::key(Collection::Playlists, Uuid::new_v4()),
            support::playlist("after restart"),
        )
        .await?;
    assert_eq!(local.revision.counter, 4);
    Ok(())
}
#[tokio::test]
async fn killed_before_commit_rolls_back_and_after_commit_replay_is_idempotent() -> Result<()> {
    for (phase, committed) in [("page_before_commit", false), ("page_after_commit", true)] {
        let dir = tempfile::tempdir()?;
        let (url, library, node, page) = fixture(dir.path()).await?;
        kill_at(dir.path(), &url, library, node, phase)?;
        inspect(dir.path(), &url, library, node, &page, committed).await?;
    }
    Ok(())
}
#[tokio::test]
async fn damaged_pending_journal_recovers_without_partial_page() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (url, library, node, page) = fixture(dir.path()).await?;
    kill_at(dir.path(), &url, library, node, "page_before_commit")?;
    let journal = dir.path().join(".rune/.0.db-journal");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&journal)
        .context("expected pending rollback journal")?;
    let length = file.metadata()?.len();
    ensure!(length > 1024, "journal unexpectedly short");
    file.set_len(length / 2)?;
    file.sync_all()?;
    println!("damaged_rollback_journal_bytes={length}->{}", length / 2);
    inspect(dir.path(), &url, library, node, &page, false).await
}
#[tokio::test]
#[ignore = "storage-fault fixture setup"]
async fn crash_probe_setup() -> Result<()> {
    let root = std::path::PathBuf::from(
        std::env::var_os("RUNE_SYNC_CRASH_ROOT").context("missing fixture root")?,
    );
    std::fs::create_dir_all(&root)?;
    let (url, library, node, _) = fixture(&root).await?;
    let manifest = serde_json::json!({"url":url,"library":library,"node":node,"database":root.join(".rune/.0.db")});
    std::fs::write(root.join("manifest.json"), serde_json::to_vec(&manifest)?)?;
    Ok(())
}
#[tokio::test]
#[ignore = "storage-fault fixture verification"]
async fn crash_probe_verify() -> Result<()> {
    let root = std::path::PathBuf::from(
        std::env::var_os("RUNE_SYNC_CRASH_ROOT").context("missing fixture root")?,
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json"))?)?;
    let url = manifest["url"].as_str().context("missing URL")?;
    let library: Uuid = manifest["library"]
        .as_str()
        .context("missing library")?
        .parse()?;
    let node: Uuid = manifest["node"].as_str().context("missing node")?.parse()?;
    let page: ChangePage = serde_json::from_slice(&std::fs::read(root.join("page.json"))?)?;
    let committed = match std::env::var("RUNE_SYNC_EXPECT_COMMITTED")?.as_str() {
        "true" => true,
        "false" => false,
        "auto" => {
            let db = connect(&root, node).await?;
            let store = LibrarySyncStore::open(db.clone(), node, Some(library)).await?;
            let progress = store.endpoint().checkpoint(&checkpoint(&page)?).await?;
            ensure!(
                progress == Some(page.request.after.clone()) || progress == Some(page.next.clone()),
                "recovery produced an intermediate checkpoint"
            );
            let committed = progress == Some(page.next.clone());
            println!("recovered_checkpoint={}", progress.unwrap().sequence);
            drop(store);
            db.close().await?;
            committed
        }
        _ => anyhow::bail!("invalid expected commit flag"),
    };
    inspect(&root, url, library, node, &page, committed).await
}
