#[path = "support/causal_library.rs"]
mod support;
use anyhow::Result;
use database::{
    entities::{albums, playlists},
    sync::model::*,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set, sea_query::Expr};
use uuid::Uuid;

#[tokio::test]
async fn explicit_import_rejects_incompatible_parent_and_assigns_playlist_id() -> Result<()> {
    let local = support::SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    albums::Entity::insert(albums::ActiveModel {
        name: Set("legacy".into()),
        group: Set("l".into()),
        hlc_uuid: Set(Uuid::new_v4().to_string()),
        ..Default::default()
    })
    .exec(&local.db)
    .await?;
    playlists::Entity::insert(playlists::ActiveModel {
        name: Set("legacy".into()),
        group: Set("l".into()),
        hlc_uuid: Set(String::new()),
        ..Default::default()
    })
    .exec(&local.db)
    .await?;
    assert!(
        local.library.import_basic().await.is_err(),
        "incompatible parent UUID was rewritten"
    );
    let album_id = migration::catalog_identity::parent(
        migration::catalog_identity::ParentKind::Album,
        "legacy",
    );
    albums::Entity::update_many()
        .col_expr(albums::Column::HlcUuid, Expr::value(album_id.to_string()))
        .filter(albums::Column::Name.eq("legacy"))
        .exec(&local.db)
        .await?;
    local.library.import_basic().await?;
    assert_eq!(
        albums::Entity::find()
            .one(&local.db)
            .await?
            .unwrap()
            .hlc_uuid,
        album_id.to_string()
    );
    let playlist_id = playlists::Entity::find()
        .one(&local.db)
        .await?
        .unwrap()
        .hlc_uuid;
    Uuid::parse_str(&playlist_id)?;
    let head = local.peer.head().await?;
    local.library.import_basic().await?;
    assert_eq!(local.peer.head().await?, head);
    assert_eq!(
        playlists::Entity::find()
            .one(&local.db)
            .await?
            .unwrap()
            .hlc_uuid,
        playlist_id
    );
    Ok(())
}
#[tokio::test]
async fn importing_an_unresolved_mix_preserves_its_target_uuid() -> Result<()> {
    let local = support::SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    let mix = Uuid::new_v4();
    let missing_playlist = Uuid::new_v4();
    let value = Payload::Mix(Mix {
        name: "Pending playlist".into(),
        group: String::new(),
        mode: Some(0),
        scriptlet_mode: false,
        locked: false,
        queries: vec![MixClause {
            id: Uuid::new_v4(),
            position: 0,
            operator: "lib::playlist".into(),
            parameter: String::new(),
            target_uuid: Some(missing_playlist),
            group: 0,
        }],
    });
    let key = support::key(Collection::Mixes, mix);
    RuneModel::project(&local.db, &key, Some(&value)).await?;
    local.library.import_basic().await?;
    assert_eq!(local.peer.get(&key).await?.unwrap().value, Some(value));
    let views = database::actions::mixes::query_views_on(&local.db, &mix.to_string()).await?;
    assert!(!views[0].available);
    assert_eq!(views[0].target_uuid, Some(missing_playlist));
    assert!(
        database::actions::mixes::executable_queries_on(&local.db, &mix.to_string())
            .await
            .is_err()
    );
    let head = local.peer.head().await?;
    local.library.import_basic().await?;
    assert_eq!(local.peer.head().await?, head);
    Ok(())
}
