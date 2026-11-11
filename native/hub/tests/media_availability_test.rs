#[path = "support/fixtures.rs"]
mod fixtures;
use anyhow::Result;
use database::connection::RecommendationDbConnection;
use database::{
    actions::{
        downloads::prepare_download,
        file::{
            AudioAvailability as LocalState, audio_availability, get_media_files,
            get_media_files_count, materialized_path, resolve_local_id,
        },
        metadata::get_metadata_summary_by_files,
    },
    entities::media_files,
    library_registry::{
        CatalogPlacement, FsAdapter, LibraryRecord, LibraryRecordId, ProtocolAdapter,
    },
    sync::LibrarySyncStore,
    sync::model::BlobRef,
};
use fsio::FsIo;
use hub::{
    Signal,
    library_pool::{LibraryHandle, LibraryPool, LibraryResources},
    messages::{AudioAvailability, SearchMediaFileSummaryRequest},
    utils::{files_to_playback_request, parse_media_files},
};
use sea_orm::sea_query;
use sea_orm::{ConnectionTrait, Database, EntityTrait, ExprTrait, Set};
use std::sync::Arc;
use uuid::Uuid;

async fn request_library(
    db: sea_orm::DatabaseConnection,
    fsio: Arc<FsIo>,
    media_root: String,
    actor: Uuid,
    config_path: &std::path::Path,
) -> Result<LibraryHandle> {
    let library_id = Uuid::new_v4();
    let library_sync = LibrarySyncStore::open(db.clone(), actor, Some(library_id)).await?;
    let pool = LibraryPool::new(config_path, config_path, &actor.to_string())?;
    let descriptor = LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: "Test library".into(),
        library_id: Some(library_id),
        protocol: ProtocolAdapter::Native,
        fs: vec![FsAdapter::Local {
            path: media_root.clone(),
        }],
        catalog: CatalogPlacement::InMediaRoot,
        background_sync: false,
    };
    pool.register_ui(LibraryResources {
        library_id,
        display_name: media_root.clone(),
        main_db: Arc::new(db),
        library_sync,
        recommend_db: Arc::new(RecommendationDbConnection::new()),
        fsio,
        media_root,
        descriptor,
    })
    .await
}

#[tokio::test]
async fn pending_rows_are_returned_and_reported_without_becoming_playable() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("local.ogg"), b"local")?;
    std::fs::write(dir.path().join("staged.ogg"), b"uncommitted")?;
    let db = Database::connect("sqlite::memory:").await?;
    let node = Uuid::new_v4();
    fixtures::migrate(&db, node).await?;
    let track = Uuid::new_v4();
    for (id, name, available) in [
        (10, "local.ogg", true),
        (20, "staged.ogg", false),
        (30, "missing.ogg", true),
    ] {
        media_files::Entity::insert(media_files::ActiveModel {
            id: Set(id),
            hlc_uuid: Set(if id == 20 { track } else { Uuid::new_v4() }.to_string()),
            file_name: Set(name.into()),
            directory: Set(String::new()),
            extension: Set("ogg".into()),
            file_hash: Set(name.into()),
            last_modified: Set("2020-01-01".into()),
            sample_rate: Set(44100),
            duration: Set(1.into()),
            local_available: Set(available),
            ..Default::default()
        })
        .exec(&db)
        .await?;
    }
    prepare_download(
        &db,
        &FsIo::new(),
        dir.path(),
        track,
        &BlobRef {
            digest: "a".repeat(64),
            size: 1,
        },
        "ogg",
    )
    .await?;
    assert_eq!(get_media_files_count(&db).await?, 3);
    let fsio = FsIo::new();
    let root = Arc::new(dir.path().to_string_lossy().into_owned());
    let mut states = Vec::new();
    for cursor in [0, 10, 20] {
        let page = get_media_files(&db, cursor, 1).await?;
        assert_eq!(page.len(), 1, "pending rows must not shorten a page");
        let summaries = get_metadata_summary_by_files(&db, page.clone()).await?;
        let parsed = parse_media_files(
            &fsio,
            &db,
            &node.to_string(),
            &node.to_string(),
            summaries,
            root.clone(),
        )
        .await?;
        assert_eq!(parsed.len(), 1);
        states.push((
            parsed[0].id,
            parsed[0].audio_availability,
            parsed[0].path.clone(),
        ));
        let handles = page.into_iter().map(Into::into).collect::<Vec<_>>();
        let queue =
            files_to_playback_request(&fsio, &db, &node.to_string(), &dir.path(), &handles).await?;
        assert_eq!(
            queue.len(),
            usize::from(cursor == 0),
            "pending and missing files cannot be queued"
        );
    }
    assert_eq!(states[0].1, AudioAvailability::Local);
    assert_eq!(states[1].1, AudioAvailability::ImportPending);
    assert_eq!(states[2].1, AudioAvailability::Unavailable);
    assert!(states[1].2.is_empty());
    assert!(states[2].2.is_empty());
    let request = SearchMediaFileSummaryRequest {
        call_id: None,
        n: 50,
    };
    let library = request_library(
        db.clone(),
        Arc::new(fsio.clone()),
        root.to_string(),
        node,
        dir.path(),
    )
    .await?;
    use sea_orm::sea_query::{Alias, Expr, Query};
    let query = Query::select()
        .expr_as(Expr::col(Alias::new("namespace")).count(), Alias::new("n"))
        .from(Alias::new("sync_meta"))
        .to_owned();
    let sync_metadata_before = db
        .query_one(&query)
        .await?
        .unwrap()
        .try_get::<i64>("", "n")?;
    let summary = request
        .handle((Some(library),), None, &request)
        .await?
        .unwrap();
    for (id, state, _) in &states {
        assert_eq!(
            summary
                .result
                .iter()
                .find(|row| row.id == *id)
                .unwrap()
                .audio_availability,
            *state
        );
    }
    assert_eq!(
        db.query_one(&query)
            .await?
            .unwrap()
            .try_get::<i64>("", "n")?,
        sync_metadata_before,
        "catalog reads must not change replication metadata"
    );
    let _store = fixtures::open_library(db.clone(), node, None).await?;
    assert_eq!(
        audio_availability(&db, &fsio, dir.path(), &[20])
            .await?
            .get(&20),
        Some(&LocalState::ImportPending)
    );
    assert_eq!(materialized_path(&db, &fsio, dir.path(), 20).await?, None);
    assert_eq!(resolve_local_id(&db, 20).await?, Some(20));
    Ok(())
}

#[tokio::test]
async fn queued_downloads_with_content_remain_streamable() -> Result<()> {
    use database::actions::{downloads::prepare_download, file::AudioAvailability as State};
    use database::entities::sync_download_jobs;
    use playback::player::PlaybackSource;
    use sea_orm::{ColumnTrait, QueryFilter};
    let root = tempfile::tempdir()?;
    let db = Database::connect("sqlite::memory:").await?;
    let actor = Uuid::new_v4();
    migration::up_with_actor(&db, Some(&actor.to_string()), None).await?;
    let track = Uuid::new_v4();
    let blob = BlobRef {
        digest: "a".repeat(64),
        size: 128,
    };
    let inserted = media_files::Entity::insert(media_files::ActiveModel {
        hlc_uuid: Set(track.to_string()),
        file_name: Set("remote.ogg".into()),
        directory: Set(String::new()),
        extension: Set("ogg".into()),
        file_hash: Set(String::new()),
        last_modified: Set("2020-01-01".into()),
        sample_rate: Set(44100),
        duration: Set(1.into()),
        local_available: Set(false),
        content_hash: Set(Some(blob.digest.clone())),
        content_size: Set(Some(blob.size as i64)),
        ..Default::default()
    })
    .exec(&db)
    .await?;
    let fsio = FsIo::new();
    let id = inserted.last_insert_id;
    let row = media_files::Entity::find_by_id(id).one(&db).await?.unwrap();
    let handles = vec![row.into()];
    let library = request_library(
        db.clone(),
        Arc::new(fsio.clone()),
        root.path().to_string_lossy().into_owned(),
        actor,
        root.path(),
    )
    .await?;
    prepare_download(&db, &fsio, root.path(), track, &blob, "ogg").await?;
    for retrying in [false, true] {
        if retrying {
            sync_download_jobs::Entity::update_many()
                .col_expr(
                    sync_download_jobs::Column::Attempts,
                    sea_query::Expr::value(3),
                )
                .col_expr(
                    sync_download_jobs::Column::RetryAt,
                    sea_query::Expr::value("2999-01-01T00:00:00Z"),
                )
                .filter(sync_download_jobs::Column::FileUuid.eq(track.to_string()))
                .exec(&db)
                .await?;
        }
        let state = audio_availability(&db, &fsio, root.path(), &[id]).await?;
        assert_eq!(state[&id], State::ImportPending);
        let summaries =
            get_metadata_summary_by_files(&db, get_media_files(&db, 0, 10).await?).await?;
        let parsed = parse_media_files(
            &fsio,
            &db,
            &actor.to_string(),
            &library.library_id.to_string(),
            summaries,
            Arc::new(root.path().to_string_lossy().into_owned()),
        )
        .await?;
        assert_eq!(
            parsed[0].audio_availability,
            AudioAvailability::RemoteOnly,
            "the client must retain its remote playback action while download is pending"
        );
        let request = SearchMediaFileSummaryRequest {
            call_id: None,
            n: 10,
        };
        let response = request
            .handle((Some(library.clone()),), None, &request)
            .await?
            .unwrap();
        assert_eq!(
            response.result[0].audio_availability,
            AudioAvailability::RemoteOnly
        );

        let queue =
            files_to_playback_request(&fsio, &db, &actor.to_string(), &root.path(), &handles)
                .await?;
        assert_eq!(queue.len(), 1);
        assert!(matches!(queue[0].1, PlaybackSource::LibraryStream));
        assert!(
            materialized_path(&db, &fsio, root.path(), id)
                .await?
                .is_none()
        );
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn scanned_symlinked_folder_is_playable() -> Result<()> {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    std::fs::copy(fixtures::audio_fixture(), outside.path().join("track.ogg"))?;
    symlink(outside.path(), root.path().join("linked"))?;
    let db = Database::connect("sqlite::memory:").await?;
    let node = Uuid::new_v4();
    let _ = migration::initialize_node_id(node.to_string());
    migration::up_with_actor(&db, Some(&node.to_string()), None).await?;
    let fs = Arc::new(FsIo::new());
    let store = fixtures::open_library(db.clone(), node, None).await?;
    fixtures::scan_library(
        &fs,
        &db,
        &node.to_string(),
        root.path(),
        &store.record_store(),
    )
    .await?;
    let rows = media_files::Entity::find().all(&db).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        materialized_path(&db, &fs, root.path(), rows[0].id).await?,
        Some(root.path().join("linked/track.ogg"))
    );
    let handles = rows.into_iter().map(Into::into).collect::<Vec<_>>();
    assert_eq!(
        files_to_playback_request(&fs, &db, &node.to_string(), &root.path(), &handles)
            .await?
            .len(),
        1
    );
    Ok(())
}
