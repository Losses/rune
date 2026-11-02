#[path = "support/causal_library.rs"]
mod support;
use anyhow::Result;
use database::{
    actions::cover_art as artwork,
    entities::{media_cover_art, media_files},
    sync::model::*,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set, sea_query::Expr};
use support::{SqlLibrary, key};
use uuid::Uuid;

#[tokio::test]
async fn corrupt_body_repairs_same_row_and_rolls_back_on_failure() -> Result<()> {
    let local = SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    let file = Uuid::new_v4();
    let body = b"verified original".to_vec();
    let digest = blake3::hash(&body).to_hex().to_string();
    let id = migration::catalog_identity::artwork(&digest);
    let Payload::Track(mut track) = support::track() else {
        unreachable!()
    };
    track.cover_art_id = Some(id);
    local
        .peer
        .write_local(key(Collection::Tracks, file), Some(Payload::Track(track)))
        .await?;
    assert_eq!(local.library.pending_references().await?.len(), 1);
    let assignment = Artwork {
        digest: digest.clone(),
        size: body.len() as i64,
        primary_color: Some(42),
    };
    local
        .peer
        .write_local(
            key(Collection::Artworks, id),
            Some(Payload::Artwork(assignment.clone())),
        )
        .await?;
    assert!(local.library.pending_references().await?.is_empty());
    let placeholder = media_cover_art::Entity::find()
        .filter(media_cover_art::Column::HlcUuid.eq(id.to_string()))
        .one(&local.db)
        .await?
        .unwrap();
    assert!(!placeholder.local_available);
    artwork::attach_received_body(
        &local.db,
        &local.library.record_store(),
        id,
        &assignment,
        body.clone(),
    )
    .await?;
    let local_id = placeholder.id;
    assert_eq!(
        media_cover_art::Entity::find_by_id(local_id)
            .one(&local.db)
            .await?
            .unwrap()
            .binary,
        body
    );
    media_cover_art::Entity::update_many()
        .col_expr(
            media_cover_art::Column::Binary,
            Expr::value(vec![1_u8, 2, 3]),
        )
        .filter(media_cover_art::Column::Id.eq(local_id))
        .exec(&local.db)
        .await?;
    let head = local.peer.head().await?;
    local.db.execute_unprepared("CREATE TRIGGER reject_art_repair BEFORE UPDATE OF binary ON media_cover_art BEGIN SELECT RAISE(ABORT, 'test'); END").await?;
    assert!(
        artwork::attach_received_body(
            &local.db,
            &local.library.record_store(),
            id,
            &assignment,
            body.clone()
        )
        .await
        .is_err()
    );
    let row = media_cover_art::Entity::find_by_id(local_id)
        .one(&local.db)
        .await?
        .unwrap();
    assert_eq!(row.binary, vec![1, 2, 3]);
    assert!(artwork::verify_body(id, &digest, body.len() as u64, &row.binary).is_err());
    local
        .db
        .execute_unprepared("DROP TRIGGER reject_art_repair")
        .await?;
    artwork::attach_received_body(
        &local.db,
        &local.library.record_store(),
        id,
        &assignment,
        body.clone(),
    )
    .await?;
    assert_eq!(
        media_cover_art::Entity::find_by_id(local_id)
            .one(&local.db)
            .await?
            .unwrap()
            .binary,
        body
    );
    assert_eq!(
        media_files::Entity::find()
            .one(&local.db)
            .await?
            .unwrap()
            .cover_art_uuid,
        Some(id.to_string())
    );
    assert_eq!(
        local.peer.head().await?,
        head,
        "body repair does not author metadata"
    );
    Ok(())
}

#[tokio::test]
async fn unrelated_local_body_does_not_satisfy_missing_artwork_reference() -> Result<()> {
    let local = SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    let stray_body = b"unrelated bytes".to_vec();
    let stray_digest = blake3::hash(&stray_body).to_hex().to_string();
    let stray_size = stray_body.len() as i64;
    let stray_id = migration::catalog_identity::artwork(&stray_digest);
    let body = b"expected bytes".to_vec();
    let digest = blake3::hash(&body).to_hex().to_string();
    let expected = migration::catalog_identity::artwork(&digest);
    media_cover_art::Entity::insert(media_cover_art::ActiveModel {
        file_hash: Set(format!("blake3:{stray_digest}")),
        binary: Set(stray_body),
        content_hash: Set(Some(stray_digest)),
        content_size: Set(Some(stray_size)),
        local_available: Set(true),
        hlc_uuid: Set(stray_id.to_string()),
        ..Default::default()
    })
    .exec(&local.db)
    .await?;
    let assignment = Artwork {
        digest: digest.clone(),
        size: body.len() as i64,
        primary_color: None,
    };
    assert!(
        artwork::attach_received_body(
            &local.db,
            &local.library.record_store(),
            expected,
            &assignment,
            b"unrelated bytes".to_vec()
        )
        .await
        .is_err()
    );
    let Payload::Track(mut track) = support::track() else {
        unreachable!()
    };
    track.cover_art_id = Some(expected);
    local
        .peer
        .write_local(
            key(Collection::Tracks, Uuid::new_v4()),
            Some(Payload::Track(track)),
        )
        .await?;
    assert_eq!(local.library.pending_references().await?.len(), 1);
    assert!(
        media_cover_art::Entity::find()
            .filter(media_cover_art::Column::HlcUuid.eq(expected.to_string()))
            .one(&local.db)
            .await?
            .is_none()
    );
    local
        .peer
        .write_local(
            key(Collection::Artworks, expected),
            Some(Payload::Artwork(assignment.clone())),
        )
        .await?;
    assert!(local.library.pending_references().await?.is_empty());
    artwork::attach_received_body(
        &local.db,
        &local.library.record_store(),
        expected,
        &assignment,
        body,
    )
    .await?;
    assert!(
        media_cover_art::Entity::find()
            .filter(media_cover_art::Column::HlcUuid.eq(expected.to_string()))
            .one(&local.db)
            .await?
            .unwrap()
            .local_available
    );
    Ok(())
}
