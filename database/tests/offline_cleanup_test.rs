#[path = "support/fixtures.rs"]
mod fixtures;

use anyhow::Result;
use database::{
    actions::{
        downloads::{cleanup_unselected_downloads, set_download_rule},
        metadata::scan_audio_file,
    },
    connection::RecommendationDbConnection,
    entities::media_files,
    sync::SeaOrmSyncStore,
};
use fsio::FsIo;
use sea_orm::{ActiveModelTrait, Database, EntityTrait, Set};
use uuid::Uuid;

#[tokio::test]
async fn cleanup_removes_only_unselected_managed_files() -> Result<()> {
    let node = Uuid::new_v4();
    let db = Database::connect("sqlite::memory:").await?;
    migration::up_with_actor(&db, Some(&node.to_string()), None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let root = tempfile::tempdir()?;
    let fs = FsIo::new();
    let managed = root.path().join("Rune Sync");
    std::fs::create_dir_all(&managed)?;
    let mut ids = Vec::new();
    for (index, directory) in [managed.as_path(), managed.as_path(), root.path()]
        .iter()
        .enumerate()
    {
        let path = root.path().join(format!("{index}.ogg"));
        std::fs::copy(fixtures::audio_fixture(), &path)?;
        let id = scan_audio_file(&fs, &db, &node.to_string(), root.path(), &path, &store).await?;
        if index < 2 {
            std::fs::rename(&path, directory.join(format!("{index}.ogg")))?;
        }
        let mut row: media_files::ActiveModel = media_files::Entity::find_by_id(id)
            .one(&db)
            .await?
            .unwrap()
            .into();
        row.directory = Set(if index < 2 {
            "Rune Sync".into()
        } else {
            String::new()
        });
        row.file_name = Set(format!("{index}.ogg"));
        row.update(&db).await?;
        ids.push(id);
    }
    set_download_rule(
        &db,
        vec![("lib::track".into(), ids[0].to_string())],
        None,
        true,
    )
    .await?;
    cleanup_unselected_downloads(
        &db,
        &RecommendationDbConnection::new(),
        &fs,
        root.path(),
        &store,
        &[],
    )
    .await?;
    assert!(managed.join("0.ogg").exists());
    assert!(!managed.join("1.ogg").exists());
    assert!(root.path().join("2.ogg").exists());
    let row = media_files::Entity::find_by_id(ids[1])
        .one(&db)
        .await?
        .unwrap();
    assert!(!row.local_available && row.directory.is_empty() && row.file_name.is_empty());
    Ok(())
}
