use anyhow::Result;
use database::{
    actions::mixes::{
        MixModeUpdate, add_item_to_mix, create_mix, get_mix_queries_by_mix_id,
        initialize_mix_queries_before_replication, remove_mix, remove_mix_query,
        replace_mix_queries, update_mix,
    },
    entities::{mixes, mixes::Column, search_index},
    sync::model::{Collection, ObjectKey, Payload},
};
use playback::PlaybackMode;
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
            Some(PlaybackMode::Sequential),
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
        Some(PlaybackMode::Sequential),
        false,
        vec![("lib::all".into(), "true".into())],
    )
    .await?;
    let key = ObjectKey {
        collection: Collection::Mixes,
        id: mix.source_uuid.parse()?,
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
            MixModeUpdate::Set(PlaybackMode::Sequential),
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
            &mix.source_uuid,
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
        &mix.source_uuid,
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
        MixModeUpdate::Unchanged,
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
        MixModeUpdate::Unchanged,
        None,
        None,
    )
    .await?;
    remove_mix(&db, &store, mix.id).await?;
    assert!(library.get(&key).await?.is_none());
    Ok(())
}

use database::test_fixtures::causal_library as support;

#[tokio::test]
async fn each_mix_command_replicates_its_result() -> Result<()> {
    use database::actions::mixes::*;
    let namespace = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let source = support::SqlLibrary::new(namespace, actor).await?;
    let target = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = source.sync_store.record_store();
    let mix = create_mix(
        &source.db,
        &store,
        "Original".into(),
        "Group".into(),
        Some(PlaybackMode::Sequential),
        false,
        vec![("lib::all".into(), "true".into())],
    )
    .await?;
    let key = ObjectKey {
        collection: Collection::Mixes,
        id: mix.source_uuid.parse()?,
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
        MixModeUpdate::Unchanged,
        None,
        None,
    )
    .await?;
    check(&source, &target, &key).await?;
    replace_mix_queries(
        &source.db,
        &store,
        mix.id,
        &mix.source_uuid,
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
    let store = source.sync_store.record_store();
    let mix = create_mix(
        &source.db,
        &store,
        "Directories".into(),
        "Test".into(),
        Some(PlaybackMode::Sequential),
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
        &mix.source_uuid,
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
    let remote = query_views_on(&target.db, &mix.source_uuid).await?;
    assert_eq!(remote[1].clause_id, initial[0].clause_id);
    assert!(!remote[1].available);
    assert!(remote[1].parameter.is_empty());
    Ok(())
}

#[tokio::test]
async fn a_mix_reads_back_the_playback_mode_it_was_stored_with() -> Result<()> {
    use database::actions::mixes::{MixModeError, mix_playback_mode};
    use sea_orm::{ActiveModelTrait, Set};

    let db = Database::connect("sqlite::memory:").await?;
    let actor = Uuid::new_v4();
    fixtures::migrate(&db, actor).await?;
    let library = fixtures::open_library(db.clone(), actor, Some(Uuid::new_v4())).await?;
    let store = library.record_store();
    let cases = [
        (Some(PlaybackMode::Sequential), 0),
        (Some(PlaybackMode::RepeatOne), 1),
        (Some(PlaybackMode::RepeatAll), 2),
        (Some(PlaybackMode::Shuffle), 3),
        (None, 99),
    ];
    for (mode, stored) in cases {
        let created = create_mix(
            &db,
            &store,
            "mode".into(),
            "m".into(),
            mode,
            false,
            vec![("lib::all".into(), "true".into())],
        )
        .await?;
        let row = mixes::Entity::find_by_id(created.id)
            .one(&db)
            .await?
            .unwrap();
        assert_eq!(row.mode, Some(stored));
        assert_eq!(mix_playback_mode(&row)?, mode);
    }

    // An update sets the mode, including back to keeping the player's.
    let mix = create_mix(
        &db,
        &store,
        "update".into(),
        "m".into(),
        Some(PlaybackMode::Shuffle),
        false,
        vec![("lib::all".into(), "true".into())],
    )
    .await?;
    for (update, mode) in [
        (MixModeUpdate::KeepPlayerMode, None),
        (
            MixModeUpdate::Set(PlaybackMode::RepeatAll),
            Some(PlaybackMode::RepeatAll),
        ),
    ] {
        let updated = update_mix(&db, &store, mix.id, None, None, update, None, None).await?;
        assert_eq!(mix_playback_mode(&updated)?, mode);
    }
    let untouched = update_mix(
        &db,
        &store,
        mix.id,
        None,
        None,
        MixModeUpdate::Unchanged,
        None,
        None,
    )
    .await?;
    assert_eq!(
        mix_playback_mode(&untouched)?,
        Some(PlaybackMode::RepeatAll)
    );

    // Any other stored number is an error, never a mode.
    let mut broken: mixes::ActiveModel = untouched.clone().into();
    broken.mode = Set(Some(7));
    assert_eq!(
        mix_playback_mode(&broken.update(&db).await?),
        Err(MixModeError::Unknown(7))
    );
    let mut missing: mixes::ActiveModel = untouched.into();
    missing.mode = Set(None);
    // No stored value keeps the player's mode, like 99.
    assert_eq!(mix_playback_mode(&missing.update(&db).await?), Ok(None));
    Ok(())
}

#[tokio::test]
async fn a_mix_cannot_hold_the_queries_of_an_artist_page() -> Result<()> {
    use database::actions::mixes::executable_mix_queries;
    use database::entities::portable_artists as artists;
    use database::error::ExpectedError;
    use database::sync::model::Artist;
    use migration::catalog_identity::{self, ParentKind};
    use sea_orm::{ColumnTrait, QueryFilter};

    let library = support::SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    let id = catalog_identity::parent(ParentKind::Artist, "Ann");
    library
        .peer
        .write_local(
            support::key(Collection::Artists, id),
            Some(Payload::Artist(Artist {
                name: "Ann".into(),
                group: "Ann".into(),
            })),
        )
        .await?;
    let ann = artists::Entity::find()
        .filter(artists::Column::SourceUuid.eq(id.to_string()))
        .one(&library.db)
        .await?
        .expect("the artist was stored")
        .id;
    let store = library.sync_store.record_store();
    let refused = |error: anyhow::Error| {
        matches!(
            error.downcast_ref::<ExpectedError>(),
            Some(ExpectedError::InvalidInput { .. })
        )
    };

    // A mix is synced by what it names, and these name sections a device derives from its own
    // tags and play counts, alone or with the cover filter as an artist's page sends them.
    let page_queries = [
        vec![("lib::artist_section".to_owned(), format!("{ann}:albums"))],
        vec![
            ("lib::artist_tracks".to_owned(), ann.to_string()),
            ("filter::with_cover_art".to_owned(), "true".to_owned()),
        ],
    ];
    for queries in &page_queries {
        let error = create_mix(
            &library.db,
            &store,
            "Ann".into(),
            "Group".into(),
            None,
            false,
            queries.clone(),
        )
        .await
        .unwrap_err();
        assert!(refused(error), "{queries:?}");
    }

    let artist = vec![("lib::artist".to_owned(), ann.to_string())];
    let mix = create_mix(
        &library.db,
        &store,
        "Ann".into(),
        "Group".into(),
        None,
        false,
        artist.clone(),
    )
    .await?;
    for queries in &page_queries {
        let error = replace_mix_queries(
            &library.db,
            &store,
            mix.id,
            &mix.source_uuid,
            queries.clone(),
            None,
        )
        .await
        .unwrap_err();
        assert!(refused(error), "{queries:?}");
    }
    let error = add_item_to_mix(
        &library.db,
        &store,
        mix.id,
        "lib::artist_tracks".into(),
        ann.to_string(),
    )
    .await
    .unwrap_err();
    assert!(refused(error));
    assert_eq!(executable_mix_queries(&library.db, mix.id).await?, artist);
    Ok(())
}
