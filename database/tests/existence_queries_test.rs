use anyhow::Result;
use database::{
    actions::{
        analysis::if_analyze_exists, fingerprint::has_fingerprint, metadata::sync_file_descriptions,
    },
    connection::initialize_db,
    entities::media_files,
};
use fsio::FsIo;
use metadata::describe::describe_file;
use sea_orm::{ConnectionTrait, Database, EntityTrait};

#[tokio::test]
async fn existence_queries_preserve_filters_duplicates_and_errors() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    initialize_db(&db, "existence-test").await?;
    let fs = FsIo::new();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../assets")
        .canonicalize()?;
    let actor = uuid::Uuid::new_v4();
    let store =
        database::sync::LibrarySyncStore::open(db.clone(), actor, Some(uuid::Uuid::new_v4()))
            .await?;
    let node = fs.canonicalize(&root.join("startup_0.ogg"))?;
    let mut descriptions = vec![Some(describe_file(&fs, &node, &Some(root))?)];
    sync_file_descriptions(
        &fs,
        &db,
        "existence-test",
        &mut descriptions,
        false,
        &store.record_store(),
    )
    .await?;
    let file = media_files::Entity::find().one(&db).await?.unwrap();
    let id = file.id;
    let track_uuid = file.hlc_uuid;
    assert!(!if_analyze_exists(&db, id).await?);
    assert!(!has_fingerprint(&db, id).await?);

    for n in 0..2 {
        db.execute_unprepared(&format!(
            "INSERT INTO media_file_fingerprint (track_uuid,fingerprint,is_duplicated,hlc_uuid,created_at_hlc_ts,created_at_hlc_ver,created_at_hlc_nid,updated_at_hlc_ts,updated_at_hlc_ver,updated_at_hlc_nid) VALUES ('{track_uuid}',X'0102',0,'fingerprint-{n}','',0,'','',0,'')"
        )).await?;
        db.execute_unprepared(&format!(
            "INSERT INTO media_analysis (file_id,analysis_version,hlc_uuid,created_at_hlc_ts,created_at_hlc_ver,created_at_hlc_nid,updated_at_hlc_ts,updated_at_hlc_ver,updated_at_hlc_nid) VALUES ({id},1,'analysis-{n}','',0,'','',0,'')"
        )).await?;
    }
    assert!(has_fingerprint(&db, id).await?);
    assert!(!has_fingerprint(&db, id + 1).await?);
    assert!(!if_analyze_exists(&db, id).await?);
    db.execute_unprepared(&format!(
        "UPDATE media_analysis SET analysis_version = {}",
        analysis::CURRENT_ANALYSIS_VERSION
    ))
    .await?;
    assert!(if_analyze_exists(&db, id).await?);
    assert!(!if_analyze_exists(&db, id + 1).await?);

    db.execute_unprepared("DROP TABLE media_analysis").await?;
    db.execute_unprepared("DROP TABLE media_file_fingerprint")
        .await?;
    assert!(if_analyze_exists(&db, id).await.is_err());
    assert!(has_fingerprint(&db, id).await.is_err());
    Ok(())
}
