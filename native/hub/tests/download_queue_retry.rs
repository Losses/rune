use database::actions::{
    downloads::{DownloadReport, set_download_rule},
    file::{blob_path_for_track, hash_file},
};
use database::sync::{
    LibrarySyncStore,
    model::{BlobRef, Collection, ObjectKey},
};
use hub::server::sync::downloads::download_requested_audio;
#[path = "support/causal_library.rs"]
mod support;
use anyhow::Result;
use axum::{Router, extract::Path, http::StatusCode, routing::get};
use database::{
    connection::{RecommendationDbConnection, initialize_db},
    entities::{media_files, sync_download_jobs, sync_download_rules},
};
use fsio::FsIo;
use sea_orm::{
    ColumnTrait, Database, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use uuid::Uuid;

struct Fixture {
    root: tempfile::TempDir,
    db: DatabaseConnection,
    fs: Arc<FsIo>,
    store: LibrarySyncStore,
    node: Uuid,
    tracks: Vec<(Uuid, BlobRef)>,
    fail: Arc<AtomicBool>,
    replace_during_download: Arc<AtomicBool>,
    gets: Arc<AtomicUsize>,
    base: String,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new() -> Result<Self> {
        let root = tempfile::tempdir()?;
        let db = Database::connect("sqlite::memory:").await?;
        let node = Uuid::new_v4();
        initialize_db(&db, &node.to_string()).await?;
        let fs = Arc::new(FsIo::new());
        let store = LibrarySyncStore::open(db.clone(), node, None).await?;
        let mut tracks = Vec::new();
        let mut bodies = std::collections::HashMap::new();
        for index in 0..2 {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../../assets/startup_{index}.ogg"));
            let blob = hash_file(&fs, &path)?;
            bodies.insert(blob.digest.clone(), std::fs::read(path)?);
            let id = Uuid::new_v4();
            store
                .write_local(
                    ObjectKey {
                        collection: Collection::Tracks,
                        id,
                    },
                    Some(support::audio(format!("Track {index}"), &blob)),
                )
                .await?;
            tracks.push((id, blob));
        }
        set_download_rule(&db, vec![("lib::all".into(), "true".into())], None, true).await?;
        let first = media_files::Entity::find()
            .filter(media_files::Column::HlcUuid.eq(tracks[0].0.to_string()))
            .one(&db)
            .await?
            .unwrap();
        set_download_rule(
            &db,
            vec![("lib::track".into(), first.id.to_string())],
            None,
            true,
        )
        .await?;
        // The missing saved mix is deliberately evaluated before the valid rules.
        sync_download_rules::Entity::insert(sync_download_rules::ActiveModel {
            id: Set("000".into()),
            queries: Set("[]".into()),
            mix_id: Set(Some(Uuid::new_v4().to_string())),
            enabled: Set(true),
        })
        .exec(&db)
        .await?;
        let metadata = support::memory(store.identity().await?.namespace, Uuid::new_v4())?;
        let fail = Arc::new(AtomicBool::new(true));
        let replace_during_download = Arc::new(AtomicBool::new(false));
        let gets = Arc::new(AtomicUsize::new(0));
        let rejected = tracks[0].1.digest.clone();
        let bytes = Arc::new(bodies);
        let request_fail = fail.clone();
        let request_gets = gets.clone();
        let request_replace = replace_during_download.clone();
        let request_store = store.clone();
        let first_track = tracks[0].0;
        let peer = http_request::sync::router(metadata).route(
            "/sync/blobs/{digest}",
            get(move |Path(digest): Path<String>| {
                let bytes = bytes.clone();
                let fail = request_fail.clone();
                let gets = request_gets.clone();
                let rejected = rejected.clone();
                let replace = request_replace.clone();
                let store = request_store.clone();
                async move {
                    gets.fetch_add(1, Ordering::SeqCst);
                    if digest == rejected && replace.swap(false, Ordering::SeqCst) {
                        // A metadata edit commits while the prior content is in flight.
                        store
                            .write_local(
                                ObjectKey {
                                    collection: Collection::Tracks,
                                    id: first_track,
                                },
                                Some(support::audio(
                                    "Changed during download",
                                    &BlobRef {
                                        digest: blake3::hash(b"new audio").to_hex().to_string(),
                                        size: 9,
                                    },
                                )),
                            )
                            .await
                            .unwrap();
                    }
                    if digest == rejected && fail.load(Ordering::SeqCst) {
                        (StatusCode::SERVICE_UNAVAILABLE, Vec::new())
                    } else {
                        (StatusCode::OK, bytes[&digest].clone())
                    }
                }
            }),
        );
        let app = Router::new().merge(peer.clone()).nest("/alternative", peer);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Ok(Self {
            root,
            db,
            fs,
            store,
            node,
            tracks,
            fail,
            replace_during_download,
            gets,
            base,
            server,
        })
    }

    async fn run(&self, base: &str) -> Result<DownloadReport> {
        download_requested_audio(
            self.db.clone(),
            &RecommendationDbConnection::new(),
            self.fs.clone(),
            self.root.path().into(),
            self.node,
            &http_request::sync::MetadataPeer::new(base, reqwest::Client::new()),
            Arc::new(|| {}),
        )
        .await
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[tokio::test]
async fn failed_rules_and_items_do_not_block_following_downloads_or_repeat_attempts() -> Result<()>
{
    let fixture = Fixture::new().await?;
    let report = fixture.run(&fixture.base).await?;
    assert_eq!(report.failed_rules, 1);
    assert_eq!(report.requested, 2);
    assert_eq!(report.downloaded, 1);
    assert_eq!(
        fixture.gets.load(Ordering::SeqCst),
        2,
        "overlapping rules attempted the failed file twice"
    );
    let job = sync_download_jobs::Entity::find()
        .one(&fixture.db)
        .await?
        .unwrap();
    assert_eq!(job.attempts, 1);
    assert!(job.last_error.is_some());
    assert!(
        chrono::DateTime::parse_from_rfc3339(job.retry_at.as_deref().unwrap())?
            > chrono::Utc::now()
    );
    assert_eq!(job.retry_source.as_deref(), Some(fixture.base.as_str()));
    fixture.fail.store(false, Ordering::SeqCst);
    assert_eq!(fixture.run(&fixture.base).await?.downloaded, 0);
    assert_eq!(
        fixture.gets.load(Ordering::SeqCst),
        2,
        "future retry issued an audio request"
    );
    sync_download_jobs::Entity::update_many()
        .col_expr(
            sync_download_jobs::Column::RetryAt,
            sea_orm::sea_query::Expr::value("2000-01-01T00:00:00Z"),
        )
        .exec(&fixture.db)
        .await?;
    assert_eq!(fixture.run(&fixture.base).await?.downloaded, 1);
    assert!(
        sync_download_jobs::Entity::find()
            .all(&fixture.db)
            .await?
            .is_empty()
    );
    assert_eq!(media_files::Entity::find().all(&fixture.db).await?.len(), 2);
    Ok(())
}

#[tokio::test]
async fn another_source_is_eligible_during_failed_source_backoff() -> Result<()> {
    let fixture = Fixture::new().await?;
    assert_eq!(fixture.run(&fixture.base).await?.downloaded, 1);
    fixture.fail.store(false, Ordering::SeqCst);
    assert_eq!(
        fixture
            .run(&format!("{}/alternative", fixture.base))
            .await?
            .downloaded,
        1
    );
    assert!(
        sync_download_jobs::Entity::find()
            .all(&fixture.db)
            .await?
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn preparation_failure_isolated_to_its_file() -> Result<()> {
    let fixture = Fixture::new().await?;
    fixture.fail.store(false, Ordering::SeqCst);
    let (track, blob) = &fixture.tracks[0];
    let path = blob_path_for_track(
        &fixture.fs,
        fixture.root.path(),
        *track,
        &blob.digest,
        "ogg",
    )?;
    let parent = path.parent().unwrap();
    std::fs::remove_dir(parent)?;
    std::fs::write(parent, b"blocked destination")?;
    let report = fixture.run(&fixture.base).await?;
    assert_eq!(report.requested, 2);
    assert_eq!(report.downloaded, 1);
    assert_eq!(fixture.gets.load(Ordering::SeqCst), 1);
    std::fs::remove_file(parent)?;
    assert_eq!(fixture.run(&fixture.base).await?.downloaded, 1);
    let files = media_files::Entity::find().all(&fixture.db).await?;
    for file in files {
        assert!(
            database::actions::file::is_materialized(
                &fixture.db,
                &fixture.fs,
                fixture.root.path(),
                file.id
            )
            .await?
        );
    }
    Ok(())
}

#[tokio::test]
async fn obsolete_attachment_does_not_block_the_following_file() -> Result<()> {
    let fixture = Fixture::new().await?;
    fixture.fail.store(false, Ordering::SeqCst);
    fixture
        .replace_during_download
        .store(true, Ordering::SeqCst);
    let report = fixture.run(&fixture.base).await?;
    assert_eq!(report.requested, 2);
    assert_eq!(report.downloaded, 1);
    let failed = sync_download_jobs::Entity::find()
        .one(&fixture.db)
        .await?
        .unwrap();
    assert_eq!(failed.file_uuid, fixture.tracks[0].0.to_string());
    assert!(failed.last_error.is_some());
    let files = media_files::Entity::find().all(&fixture.db).await?;
    assert_eq!(files.len(), 2);
    for file in files {
        assert_eq!(
            database::actions::file::is_materialized(
                &fixture.db,
                &fixture.fs,
                fixture.root.path(),
                file.id
            )
            .await?,
            file.hlc_uuid == fixture.tracks[1].0.to_string()
        );
    }
    Ok(())
}

#[tokio::test]
async fn obsolete_reservation_is_released_without_reimporting_managed_bytes() -> Result<()> {
    use database::actions::downloads::prepare_download;
    use database::actions::metadata::scan_audio_library;
    let fixture = Fixture::new().await?;
    let (track, blob) = &fixture.tracks[0];
    let path = prepare_download(
        &fixture.db,
        &fixture.fs,
        fixture.root.path(),
        *track,
        blob,
        "ogg",
    )
    .await?;
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg"),
        &path,
    )?;
    fixture
        .store
        .write_local(
            ObjectKey {
                collection: Collection::Tracks,
                id: *track,
            },
            Some(support::audio(
                "Changed expected content",
                &BlobRef {
                    digest: blake3::hash(b"replacement").to_hex().to_string(),
                    size: 11,
                },
            )),
        )
        .await?;
    sync_download_rules::Entity::update_many()
        .col_expr(
            sync_download_rules::Column::Enabled,
            sea_orm::sea_query::Expr::value(false),
        )
        .exec(&fixture.db)
        .await?;
    fixture.run(&fixture.base).await?;
    assert!(
        sync_download_jobs::Entity::find()
            .all(&fixture.db)
            .await?
            .is_empty()
    );
    assert!(path.is_file(), "reservation release must not delete files");
    let before = fixture.store.head().await?;
    let records = fixture.store.record_store();
    scan_audio_library(
        &fixture.fs,
        &fixture.db,
        &fixture.node.to_string(),
        fixture.root.path(),
        true,
        false,
        |_| {},
        None,
        &records,
    )
    .await?;
    assert_eq!(
        fixture.store.head().await?,
        before,
        "managed bytes must not author metadata"
    );
    assert_eq!(media_files::Entity::find().all(&fixture.db).await?.len(), 2);
    Ok(())
}

#[tokio::test]
async fn managed_corruption_is_repaired_without_metadata_authorship() -> Result<()> {
    use database::actions::file::ManagedAudioVerifier;
    let fixture = Fixture::new().await?;
    fixture.fail.store(false, Ordering::SeqCst);
    assert_eq!(fixture.run(&fixture.base).await?.downloaded, 2);
    let before_head = fixture.store.head().await?;
    let (track, blob) = &fixture.tracks[0];
    let before = media_files::Entity::find()
        .filter(media_files::Column::HlcUuid.eq(track.to_string()))
        .one(&fixture.db)
        .await?
        .unwrap();
    let path = fixture
        .root
        .path()
        .join(&before.directory)
        .join(&before.file_name);
    let mut bytes = std::fs::read(&path)?;
    bytes[0] ^= 0xff;
    std::fs::write(&path, bytes)?;
    let mut verifier = ManagedAudioVerifier::default();
    let mut invalidated = false;
    for _ in 0..4096 {
        let step = verifier
            .step(&fixture.db, &fixture.fs, fixture.root.path(), 1024)
            .await?;
        assert!(step.bytes_read <= 1024);
        invalidated |= step.invalidated;
        if invalidated {
            break;
        }
    }
    assert!(
        invalidated,
        "bounded verifier must reach and reject corrupted bytes"
    );
    let missing = media_files::Entity::find_by_id(before.id)
        .one(&fixture.db)
        .await?
        .unwrap();
    assert!(!missing.local_available);
    assert_eq!(missing.content_hash, before.content_hash);
    assert_eq!(fixture.store.head().await?, before_head);
    assert_eq!(fixture.run(&fixture.base).await?.downloaded, 1);
    let repaired = media_files::Entity::find_by_id(before.id)
        .one(&fixture.db)
        .await?
        .unwrap();
    assert!(repaired.local_available);
    assert_eq!(repaired.hlc_uuid, before.hlc_uuid);
    assert_eq!(hash_file(&fixture.fs, &path)?, *blob);
    assert_eq!(fixture.store.head().await?, before_head);
    Ok(())
}

#[tokio::test]
async fn stale_managed_verification_cannot_invalidate_changed_source() -> Result<()> {
    use database::actions::file::ManagedAudioVerifier;
    let fixture = Fixture::new().await?;
    fixture.fail.store(false, Ordering::SeqCst);
    fixture.run(&fixture.base).await?;
    let row = media_files::Entity::find()
        .order_by_asc(media_files::Column::Id)
        .one(&fixture.db)
        .await?
        .unwrap();
    let path = fixture
        .root
        .path()
        .join(&row.directory)
        .join(&row.file_name);
    let mut bytes = std::fs::read(&path)?;
    bytes[0] ^= 0xff;
    std::fs::write(&path, bytes)?;
    let mut verifier = ManagedAudioVerifier::default();
    assert_eq!(
        verifier
            .step(&fixture.db, &fixture.fs, fixture.root.path(), 1)
            .await?
            .bytes_read,
        1
    );
    let replacement = blake3::hash(b"replacement").to_hex().to_string();
    media_files::Entity::update_many()
        .col_expr(
            media_files::Column::ContentHash,
            sea_orm::sea_query::Expr::value(replacement),
        )
        .filter(media_files::Column::Id.eq(row.id))
        .exec(&fixture.db)
        .await?;
    let step = verifier
        .step(
            &fixture.db,
            &fixture.fs,
            fixture.root.path(),
            16 * 1024 * 1024,
        )
        .await?;
    assert!(!step.invalidated);
    assert!(
        media_files::Entity::find_by_id(row.id)
            .one(&fixture.db)
            .await?
            .unwrap()
            .local_available
    );
    Ok(())
}
