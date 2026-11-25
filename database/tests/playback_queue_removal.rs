use anyhow::Result;
use database::{
    actions::{library::clean_local_orphans, mixes::query_mix_media_files},
    connection::RecommendationDbConnection,
    entities::media_files,
};
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, ConnectionTrait, Database, Set, Statement};
use uuid::Uuid;

#[tokio::test]
async fn fresh_library_has_no_legacy_queue_and_queue_mix_uses_session_values() -> Result<()> {
    let actor = Uuid::new_v4();
    let _ = migration::initialize_node_id(actor.to_string());
    let db = Database::connect("sqlite::memory:").await?;
    Migrator::up(&db, None).await?;

    let table_count = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            "SELECT COUNT(*) AS count FROM sqlite_master WHERE type = 'table' AND name = 'playback_queue'".to_owned(),
        ))
        .await?
        .expect("sqlite_master count")
        .try_get::<i64>("", "count")?;
    assert_eq!(
        table_count, 0,
        "fresh libraries must not create playback_queue"
    );

    for id in [10, 11] {
        media_files::ActiveModel {
            id: Set(id),
            file_name: Set(format!("{id}.ogg")),
            directory: Set(String::new()),
            extension: Set("ogg".into()),
            file_hash: Set(format!("hash-{id}")),
            last_modified: Set(String::new()),
            change_token: Set(None),
            cover_art_uuid: Set(None),
            cover_art_checked_at: Set(None),
            sample_rate: Set(0),
            duration: Set(Default::default()),
            local_available: Set(true),
            content_hash: Set(None),
            content_size: Set(None),
            source_uuid: Set(Uuid::new_v4().to_string()),
        }
        .insert(&db)
        .await?;
    }

    let rows = query_mix_media_files(
        &db,
        &RecommendationDbConnection::new(),
        vec![("lib::queue".into(), "true".into())],
        0,
        10,
        &[11],
    )
    .await?;
    assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), [11]);
    clean_local_orphans(&db).await?;
    Ok(())
}
