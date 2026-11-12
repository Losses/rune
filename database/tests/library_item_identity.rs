#[path = "support/causal_library.rs"]
mod support;

use anyhow::Result;
use database::{
    actions::file::{file_ids_by_uuid, file_uuids_by_id},
    entities::media_files,
};
use sea_orm::{EntityTrait, Set};
use uuid::Uuid;

async fn insert_file(db: &sea_orm::DatabaseConnection, file_uuid: Uuid) -> Result<i32> {
    Ok(media_files::Entity::insert(media_files::ActiveModel {
        file_name: Set("item.ogg".into()),
        directory: Set(String::new()),
        extension: Set("ogg".into()),
        file_hash: Set(String::new()),
        last_modified: Set(String::new()),
        sample_rate: Set(1),
        duration: Set(1.into()),
        source_uuid: Set(file_uuid.to_string()),
        ..Default::default()
    })
    .exec(db)
    .await?
    .last_insert_id)
}

#[tokio::test]
async fn stable_file_identity_resolves_to_each_replica_local_id() -> Result<()> {
    let library_id = Uuid::new_v4();
    let actor_a = Uuid::new_v4();
    let actor_b = Uuid::new_v4();
    let a = support::SqlLibrary::new(library_id, actor_a).await?;
    let b = support::SqlLibrary::new(library_id, actor_b).await?;
    let file_uuid = Uuid::new_v4();
    let a_id = insert_file(&a.db, file_uuid).await?;
    let _ = insert_file(&b.db, Uuid::new_v4()).await?;
    let b_id = insert_file(&b.db, file_uuid).await?;
    assert_ne!(a_id, b_id);

    let a_ids = file_ids_by_uuid(&a.db, &[file_uuid]).await?;
    let b_ids = file_ids_by_uuid(&b.db, &[file_uuid]).await?;
    assert_eq!(a_ids.get(&file_uuid), Some(&a_id));
    assert_eq!(b_ids.get(&file_uuid), Some(&b_id));
    assert_eq!(
        file_uuids_by_id(&a.db, &[a_id]).await?.get(&a_id),
        Some(&file_uuid)
    );
    assert_eq!(
        file_uuids_by_id(&b.db, &[b_id]).await?.get(&b_id),
        Some(&file_uuid)
    );
    assert!(
        !file_ids_by_uuid(&a.db, &[Uuid::new_v4()])
            .await?
            .contains_key(&file_uuid)
    );
    Ok(())
}
