use anyhow::Result;
use database::{
    actions::mixes::{
        add_item_to_mix, create_mix, get_mix_queries_by_mix_id,
        initialize_mix_queries_before_replication, remove_mix, remove_mix_query,
        replace_mix_queries, update_mix,
    },
    entities::{mixes, mixes::Column, search_index},
    sync::model::{Collection, ObjectKey, Payload},
};
use sea_orm::{Database, EntityTrait, PaginatorTrait};
use support::fixtures;
use uuid::Uuid;
#[tokio::test]
async fn commands_author_and_reject_invalid_queries_atomically() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    let actor = Uuid::new_v4();
    fixtures::migrate(&db, actor).await?;
    let library = fixtures::open_library(db.clone(), actor, Some(Uuid::new_v4())).await?;
    let store = library.record_store();
    let head = library.head().await?;
    let mix_count = mixes::Entity::find().count(&db).await?;
    let search_count = search_index::Entity::find().count(&db).await?;
    assert!(
        create_mix(
            &db,
            &store,
            "invalid".into(),
            "m".into(),
            false,
            0,
            false,
            vec![("invalid".into(), "x".into())]
        )
        .await
        .is_err()
    );
    assert_eq!(library.head().await?, head);
    assert_eq!(mixes::Entity::find().count(&db).await?, mix_count);
    assert_eq!(search_index::Entity::find().count(&db).await?, search_count);
    assert!(
        initialize_mix_queries_before_replication(&db)
            .await
            .is_err()
    );
    let mix = create_mix(
        &db,
        &store,
        "mix".into(),
        "m".into(),
        false,
        0,
        false,
        vec![("lib::all".into(), "true".into())],
    )
    .await?;
    let key = ObjectKey {
        collection: Collection::Mixes,
        id: mix.hlc_uuid.parse()?,
    };
    assert!(matches!(
        library.get(&key).await?.unwrap().value,
        Some(Payload::Mix { .. })
    ));
    let head = library.head().await?;
    assert!(
        update_mix(
            &db,
            &store,
            mix.id,
            Some("bad".into()),
            Some("m".into()),
            Some(false),
            Some(0),
            Some(false),
            Some(vec![("invalid".into(), "x".into())])
        )
        .await
        .is_err()
    );
    assert_eq!(library.head().await?, head);
    assert_eq!(
        mixes::Entity::find_by_id(mix.id)
            .one(&db)
            .await?
            .unwrap()
            .name,
        "mix"
    );
    let before_missing = library.head().await?;
    assert!(remove_mix(&db, &store, i32::MAX).await.is_err());
    assert!(remove_mix_query(&db, &store, i32::MAX).await.is_err());
    assert_eq!(library.head().await?, before_missing);
    let before = library.head().await?;
    assert!(
        replace_mix_queries(
            &db,
            &store,
            mix.id,
            &mix.hlc_uuid,
            vec![("invalid".into(), "x".into())],
            None
        )
        .await
        .is_err()
    );
    assert!(
        add_item_to_mix(&db, &store, mix.id, "invalid".into(), "x".into())
            .await
            .is_err()
    );
    assert_eq!(library.head().await?, before);
    let added = add_item_to_mix(&db, &store, mix.id, "pipe::limit".into(), "12".into()).await?;
    assert!(library.head().await?.sequence > before.sequence);
    let before = library.head().await?;
    remove_mix_query(&db, &store, added.id).await?;
    assert!(library.head().await?.sequence > before.sequence);
    assert_eq!(get_mix_queries_by_mix_id(&db, mix.id).await?.len(), 1);
    replace_mix_queries(
        &db,
        &store,
        mix.id,
        &mix.hlc_uuid,
        vec![("filter::liked".into(), "true".into())],
        None,
    )
    .await?;
    let before = library.head().await?;
    update_mix(
        &db,
        &store,
        mix.id,
        Some("renamed".into()),
        None,
        None,
        None,
        None,
        None,
    )
    .await?;
    assert!(library.head().await?.sequence > before.sequence);
    update_mix(
        &db,
        &store,
        mix.id,
        Some("renamed again".into()),
        None,
        None,
        None,
        None,
        None,
    )
    .await?;
    remove_mix(&db, &store, mix.id).await?;
    assert!(library.get(&key).await?.is_none());
    Ok(())
}

#[path = "support/causal_library.rs"]
mod support;

#[tokio::test]
async fn each_mix_command_replicates_its_result() -> Result<()> {
    use database::actions::mixes::*;
    let namespace = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let source = support::SqlLibrary::new(namespace, actor).await?;
    let target = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = source.library.record_store();
    let mix = create_mix(
        &source.db,
        &store,
        "Original".into(),
        "Group".into(),
        false,
        0,
        false,
        vec![("lib::all".into(), "true".into())],
    )
    .await?;
    let key = ObjectKey {
        collection: Collection::Mixes,
        id: mix.hlc_uuid.parse()?,
    };
    async fn check(
        source: &support::SqlLibrary,
        target: &support::SqlLibrary,
        key: &ObjectKey,
    ) -> Result<()> {
        support::exchange(&source.peer, &target.peer).await?;
        assert_eq!(
            source.peer.get(key).await?.and_then(|record| record.value),
            target.peer.get(key).await?.and_then(|record| record.value)
        );
        Ok(())
    }
    check(&source, &target, &key).await?;
    update_mix(
        &source.db,
        &store,
        mix.id,
        Some("Updated".into()),
        None,
        None,
        None,
        None,
        None,
    )
    .await?;
    check(&source, &target, &key).await?;
    replace_mix_queries(
        &source.db,
        &store,
        mix.id,
        &mix.hlc_uuid,
        vec![("filter::liked".into(), "true".into())],
        Some(2),
    )
    .await?;
    check(&source, &target, &key).await?;
    let clause =
        add_item_to_mix(&source.db, &store, mix.id, "pipe::limit".into(), "5".into()).await?;
    check(&source, &target, &key).await?;
    remove_mix_query(&source.db, &store, clause.id).await?;
    check(&source, &target, &key).await?;
    remove_mix(&source.db, &store, mix.id).await?;
    check(&source, &target, &key).await?;
    Ok(())
}

#[tokio::test]
async fn bootstrap_initializes_builtins_before_replication() -> Result<()> {
    use database::actions::mixes::{
        get_mix_queries_by_mix_id, initialize_mix_queries_before_replication,
    };
    use sea_orm::{ColumnTrait, QueryFilter};
    let db = Database::connect("sqlite::memory:").await?;
    fixtures::migrate(&db, Uuid::new_v4()).await?;
    initialize_mix_queries_before_replication(&db).await?;
    initialize_mix_queries_before_replication(&db).await?;
    let liked = mixes::Entity::find()
        .filter(Column::Name.eq("\u{200B}Liked"))
        .one(&db)
        .await?
        .unwrap();
    let queries = get_mix_queries_by_mix_id(&db, liked.id).await?;
    assert_eq!(
        queries
            .iter()
            .map(|q| (q.operator.as_str(), q.parameter.as_str()))
            .collect::<Vec<_>>(),
        vec![("lib::all", "true"), ("filter::liked", "true")]
    );
    Ok(())
}

#[tokio::test]
async fn directory_binding_stays_local_and_clause_identity_survives_reordering() -> Result<()> {
    use database::actions::mixes::*;
    let namespace = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let source = support::SqlLibrary::new(namespace, actor).await?;
    let target = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = source.library.record_store();
    let mix = create_mix(
        &source.db,
        &store,
        "Directories".into(),
        "Test".into(),
        false,
        0,
        false,
        vec![
            ("lib::directory.deep".into(), "/local/music".into()),
            ("pipe::limit".into(), "8".into()),
        ],
    )
    .await?;
    let initial = get_mix_queries_by_mix_id(&source.db, mix.id).await?;
    replace_mix_queries(
        &source.db,
        &store,
        mix.id,
        &mix.hlc_uuid,
        vec![
            ("pipe::limit".into(), "8".into()),
            ("lib::directory.deep".into(), "/local/music".into()),
        ],
        Some(7),
    )
    .await?;
    let reordered = get_mix_queries_by_mix_id(&source.db, mix.id).await?;
    assert_eq!(reordered[1].clause_id, initial[0].clause_id);
    assert_eq!(reordered[1].parameter, "/local/music");
    support::exchange(&source.peer, &target.peer).await?;
    let remote = query_views_on(&target.db, &mix.hlc_uuid).await?;
    assert_eq!(remote[1].clause_id, initial[0].clause_id);
    assert!(!remote[1].available);
    assert!(remote[1].parameter.is_empty());
    Ok(())
}
