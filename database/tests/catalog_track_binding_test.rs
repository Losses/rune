#[path = "support/causal_library.rs"]
mod support;
use anyhow::Result;
use database::{
    entities::{media_files, media_metadata},
    sync::model::*,
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

#[tokio::test]
async fn track_receive_preserves_local_file_fields_and_portable_custom_tags() -> Result<()> {
    let local = support::SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    let db = &local.db;
    let object = support::key(Collection::Tracks, Uuid::new_v4());
    let Payload::Track(mut track) = support::track() else {
        unreachable!()
    };
    track.metadata.push(MetadataTag {
        id: Uuid::new_v4(),
        position: 1,
        key: "custom".into(),
        value: "preserved".into(),
    });
    RuneModel::project(db, &object, Some(&Payload::Track(track.clone()))).await?;
    let file = media_files::Entity::find()
        .filter(media_files::Column::HlcUuid.eq(object.id.to_string()))
        .one(db)
        .await?
        .unwrap();
    assert!(!file.local_available);
    let local_id = file.id;
    let mut attached: media_files::ActiveModel = file.into();
    attached.local_available = Set(true);
    attached.directory = Set("Music".into());
    attached.file_name = Set("song.flac".into());
    attached.file_hash = Set("local CRC".into());
    attached.update(db).await?;
    RuneModel::project(db, &object, Some(&Payload::Track(track.clone()))).await?;
    assert!(
        media_files::Entity::find_by_id(local_id)
            .one(db)
            .await?
            .unwrap()
            .local_available
    );
    let tag_before = media_metadata::Entity::find()
        .filter(media_metadata::Column::MetaKey.eq("custom"))
        .one(db)
        .await?
        .unwrap();
    track.metadata[0].value = "Changed".into();
    track.content_hash = Some("b".repeat(64));
    track.content_size = Some(20);
    RuneModel::project(db, &object, Some(&Payload::Track(track))).await?;
    let changed = media_files::Entity::find_by_id(local_id)
        .one(db)
        .await?
        .unwrap();
    assert!(!changed.local_available);
    assert_eq!(changed.directory, "Music");
    assert_eq!(changed.file_name, "song.flac");
    assert_eq!(changed.file_hash, "local CRC");
    assert_eq!(
        media_metadata::Entity::find()
            .filter(media_metadata::Column::MetaKey.eq("custom"))
            .one(db)
            .await?
            .unwrap(),
        tag_before
    );
    Ok(())
}
