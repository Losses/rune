#[path = "support/fixtures.rs"]
mod fixtures;

use anyhow::Result;
use database::{
    connection::RecommendationDbConnection,
    entities::{media_cover_art, media_files, media_metadata},
    library_registry::{
        CatalogPlacement, FsAdapter, LibraryRecord, LibraryRecordId, ProtocolAdapter,
    },
    sync::LibrarySyncStore,
};
use fsio::FsIo;
use hub::{
    Signal,
    library_pool::{LibraryHandle, LibraryPool, LibraryResources},
    messages::{FetchMediaFilesRequest, SearchMediaFileSummaryRequest},
};
use sea_orm::{Database, EntityTrait, Set};
use std::sync::Arc;
use uuid::Uuid;

async fn request_library(
    db: sea_orm::DatabaseConnection,
    actor: Uuid,
    root: &str,
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
        fs: vec![FsAdapter::Local { path: root.into() }],
        catalog: CatalogPlacement::InMediaRoot,
        background_sync: false,
    };
    pool.register_ui(LibraryResources {
        library_id,
        display_name: root.into(),
        main_db: Arc::new(db),
        library_sync,
        recommend_db: Arc::new(RecommendationDbConnection::new()),
        fsio: Arc::new(FsIo::new()),
        media_root: root.into(),
        descriptor,
    })
    .await
}

#[tokio::test]
async fn media_file_handler_reports_absence_and_real_metadata_as_optional_values() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db = Database::connect("sqlite::memory:").await?;
    let actor = Uuid::new_v4();
    fixtures::migrate(&db, actor).await?;

    let cover_uuid = Uuid::new_v4().to_string();
    media_cover_art::Entity::insert(media_cover_art::ActiveModel {
        id: Set(91),
        source_uuid: Set(cover_uuid.clone()),
        file_hash: Set("cover-hash".into()),
        binary: Set(vec![1]),
        local_available: Set(true),
        ..Default::default()
    })
    .exec(&db)
    .await?;

    for (id, name, cover_art_uuid) in [
        (1, "no-metadata.ogg", None),
        (2, "metadata.ogg", Some(cover_uuid)),
        (3, "zero-track.ogg", None),
    ] {
        media_files::Entity::insert(media_files::ActiveModel {
            id: Set(id),
            source_uuid: Set(Uuid::new_v4().to_string()),
            file_name: Set(name.into()),
            directory: Set(String::new()),
            extension: Set("ogg".into()),
            file_hash: Set(name.into()),
            last_modified: Set("2020-01-01".into()),
            cover_art_uuid: Set(cover_art_uuid),
            sample_rate: Set(44100),
            duration: Set(1.into()),
            local_available: Set(false),
            ..Default::default()
        })
        .exec(&db)
        .await?;
    }
    for (file_id, track_number) in [(2, "7"), (3, "0")] {
        media_metadata::Entity::insert(media_metadata::ActiveModel {
            file_id: Set(file_id),
            position: Set(0),
            meta_key: Set("track_number".into()),
            meta_value: Set(track_number.into()),
            source_uuid: Set(Uuid::new_v4().to_string()),
            ..Default::default()
        })
        .exec(&db)
        .await?;
    }

    let library = request_library(db, actor, dir.path().to_str().unwrap(), dir.path()).await?;
    let request = FetchMediaFilesRequest {
        cursor: 0,
        page_size: 10,
        bake_cover_arts: false,
        call_id: None,
    };
    let response = request
        .handle(
            (Some(library.clone()), Arc::new(actor.to_string())),
            None,
            &request,
        )
        .await?
        .unwrap();
    let without = serde_json::to_value(
        response
            .media_files
            .iter()
            .find(|file| file.id == 1)
            .unwrap(),
    )?;
    let with = serde_json::to_value(
        response
            .media_files
            .iter()
            .find(|file| file.id == 2)
            .unwrap(),
    )?;
    assert_eq!(without["cover_art_id"], serde_json::Value::Null);
    assert_eq!(without["track_number"], serde_json::Value::Null);
    assert_eq!(with["cover_art_id"], 91);
    assert_eq!(with["track_number"], 7);
    let zero_track = serde_json::to_value(
        response
            .media_files
            .iter()
            .find(|file| file.id == 3)
            .unwrap(),
    )?;
    assert_eq!(zero_track["cover_art_id"], serde_json::Value::Null);
    assert_eq!(zero_track["track_number"], 0);

    let summary_request = SearchMediaFileSummaryRequest {
        n: 10,
        call_id: None,
    };
    let summaries = summary_request
        .handle((Some(library),), None, &summary_request)
        .await?
        .unwrap();
    let without_cover = serde_json::to_value(
        summaries
            .result
            .iter()
            .find(|summary| summary.id == 1)
            .unwrap(),
    )?;
    assert_eq!(without_cover["cover_art_id"], serde_json::Value::Null);
    Ok(())
}
