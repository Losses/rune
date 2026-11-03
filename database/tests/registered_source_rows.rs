use anyhow::Result;
use database::actions::mixes::{add_item_to_mix, executable_queries_on};
use database::{entities::*, sync::model::*};
use migration::{Migrator, MigratorTrait};
use sea_orm::{
    ColumnTrait, Database, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    TransactionTrait, sea_query::Expr,
};
use sync::protocol::{self as p, SyncModel};
use uuid::Uuid;

async fn database() -> Result<DatabaseConnection> {
    let _ = migration::initialize_node_id(Uuid::new_v4().to_string());
    let db = Database::connect("sqlite::memory:").await?;
    Migrator::up(&db, None).await?;
    RuneModel::validate_database_on(&db).await?;
    Ok(db)
}
fn key(collection: Collection, id: Uuid) -> p::ObjectKey<RuneModel> {
    p::ObjectKey { collection, id }
}
fn track(tags: Vec<MetadataTag>) -> Payload {
    Payload::Track(Track {
        extension: "flac".into(),
        content_hash: None,
        content_size: None,
        duration: "1.23456789".to_owned().try_into().unwrap(),
        sample_rate: 48000,
        cover_art_id: None,
        metadata: tags,
    })
}

#[tokio::test]
async fn repeated_tags_reorder_without_replacing_source_rows_or_local_file_state() -> Result<()> {
    let db = database().await?;
    let id = Uuid::new_v4();
    let key = key(Collection::Tracks, id);
    let first = MetadataTag {
        id: Uuid::new_v4(),
        position: 0,
        key: "artist".into(),
        value: "First".into(),
    };
    let second = MetadataTag {
        id: Uuid::new_v4(),
        position: 1,
        key: "artist".into(),
        value: "Second".into(),
    };
    let custom = MetadataTag {
        id: Uuid::new_v4(),
        position: 2,
        key: "custom tag".into(),
        value: "keep".into(),
    };
    RuneModel::project(
        &db,
        &key,
        Some(&track(vec![first.clone(), second.clone(), custom.clone()])),
    )
    .await?;
    let local = RuneModel::local_id_on(&db, &key).await?.unwrap();
    media_files::Entity::update_many()
        .col_expr(media_files::Column::FileName, Expr::value("local.flac"))
        .col_expr(media_files::Column::LocalAvailable, Expr::value(true))
        .filter(media_files::Column::Id.eq(local))
        .exec(&db)
        .await?;
    let previous = media_metadata::Entity::find()
        .order_by_asc(media_metadata::Column::HlcUuid)
        .all(&db)
        .await?;
    let mut swapped = vec![second, first, custom];
    for (position, tag) in swapped.iter_mut().enumerate() {
        tag.position = position as i32;
    }
    let payload = track(swapped);
    RuneModel::project(&db, &key, Some(&payload)).await?;
    assert_eq!(RuneModel::encode_on(&db, &key).await?, Some(payload));
    let after = media_metadata::Entity::find()
        .order_by_asc(media_metadata::Column::HlcUuid)
        .all(&db)
        .await?;
    assert_eq!(
        previous.iter().map(|row| row.id).collect::<Vec<_>>(),
        after.iter().map(|row| row.id).collect::<Vec<_>>()
    );
    let file = media_files::Entity::find_by_id(local as i32)
        .one(&db)
        .await?
        .unwrap();
    assert_eq!(file.file_name, "local.flac");
    assert!(file.local_available);
    Ok(())
}

#[tokio::test]
async fn reference_arrival_and_parent_removal_leave_membership_untouched() -> Result<()> {
    let db = database().await?;
    let track_id = Uuid::new_v4();
    let album_id = migration::catalog_identity::parent(
        migration::catalog_identity::ParentKind::Album,
        "Album",
    );
    let membership = key(Collection::TrackAlbums, track_id);
    let payload = Payload::TrackAlbum(TrackAlbum {
        track_id,
        album_id,
        track_number: Some(7),
    });
    RuneModel::project(&db, &membership, Some(&payload)).await?;
    let before = media_file_albums::Entity::find().one(&db).await?.unwrap();
    assert!(
        RuneModel::missing_references_on(&db, &payload, 1)
            .await
            .is_err()
    );
    assert_eq!(
        RuneModel::missing_references_on(&db, &payload, 2)
            .await?
            .len(),
        2
    );
    RuneModel::project(
        &db,
        &key(Collection::Tracks, track_id),
        Some(&track(vec![])),
    )
    .await?;
    RuneModel::project(
        &db,
        &key(Collection::Albums, album_id),
        Some(&Payload::Album(Album {
            name: "Album".into(),
            group: "A".into(),
        })),
    )
    .await?;
    assert!(
        RuneModel::missing_references_on(&db, &payload, 2)
            .await?
            .is_empty()
    );
    RuneModel::project(&db, &key(Collection::Albums, album_id), None).await?;
    assert_eq!(
        media_file_albums::Entity::find().one(&db).await?.unwrap(),
        before
    );
    assert_eq!(
        RuneModel::missing_references_on(&db, &payload, 2)
            .await?
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn owned_mix_list_preserves_local_directory_and_reports_unresolved_targets() -> Result<()> {
    use database::actions::mixes::{executable_queries_on, query_views_on, replace_mix_queries};
    let db = database().await?;
    let id = Uuid::new_v4();
    let object = key(Collection::Mixes, id);
    let payload = Payload::Mix(Mix {
        name: "Selection".into(),
        group: "S".into(),
        mode: Some(0),
        scriptlet_mode: false,
        locked: false,
        queries: vec![],
    });
    RuneModel::project(&db, &object, Some(&payload)).await?;
    let actor = Uuid::new_v4();
    let store =
        database::sync::LibrarySyncStore::open(db.clone(), actor, Some(Uuid::new_v4())).await?;
    let local_id = RuneModel::local_id_on(&db, &object).await?.unwrap() as i32;
    replace_mix_queries(
        &db,
        &store.record_store(),
        local_id,
        &id.to_string(),
        vec![
            ("lib::directory.deep".into(), "/local/music".into()),
            ("pipe::limit".into(), "50".into()),
        ],
        None,
    )
    .await?;
    let Some(Payload::Mix(mut value)) = RuneModel::encode_on(&db, &object).await? else {
        panic!()
    };
    let local_clause = value.queries[0].id;
    assert_eq!(value.queries[0].parameter, "");
    value.queries.swap(0, 1);
    for (position, query) in value.queries.iter_mut().enumerate() {
        query.position = position as i32;
    }
    RuneModel::project(&db, &object, Some(&Payload::Mix(value.clone()))).await?;
    let views = query_views_on(&db, &id.to_string()).await?;
    assert_eq!(views[1].clause_id, local_clause);
    assert_eq!(views[1].parameter, "/local/music");
    assert!(executable_queries_on(&db, &id.to_string()).await.is_ok());
    value.queries.push(MixClause {
        id: Uuid::new_v4(),
        position: 2,
        operator: "lib::track".into(),
        parameter: "".into(),
        target_uuid: Some(Uuid::new_v4()),
        group: 0,
    });
    let payload = Payload::Mix(value);
    RuneModel::project(&db, &object, Some(&payload)).await?;
    assert_eq!(
        RuneModel::missing_references_on(&db, &payload, 1)
            .await?
            .len(),
        1
    );
    assert!(executable_queries_on(&db, &id.to_string()).await.is_err());
    assert!(!query_views_on(&db, &id.to_string()).await?[2].available);
    Ok(())
}

#[tokio::test]
async fn statistics_keep_device_ownership_and_likes_are_independent() -> Result<()> {
    let db = database().await?;
    let track_id = Uuid::new_v4();
    for device_id in [Uuid::from_u128(1), Uuid::from_u128(2)] {
        let object = key(
            Collection::PlaybackStatistics,
            Uuid::new_v5(&track_id, device_id.as_bytes()),
        );
        let value = Some(Payload::PlaybackStatistics(PlaybackStatistics {
            track_id,
            device_id,
            skipped: 3,
            played_through: 5,
            updated_at: "2026-11-01T00:00:00Z".into(),
        }));
        RuneModel::validate_revision(
            &object,
            &value,
            &p::Revision {
                counter: 1,
                actor: device_id,
            },
        )?;
        assert!(
            RuneModel::validate_revision(
                &object,
                &value,
                &p::Revision {
                    counter: 1,
                    actor: Uuid::new_v4()
                }
            )
            .is_err()
        );
        RuneModel::project(&db, &object, value.as_ref()).await?;
        assert!(RuneModel::project(&db, &object, None).await.is_err());
    }
    let like = key(Collection::Likes, track_id);
    RuneModel::project(
        &db,
        &like,
        Some(&Payload::Liked(Liked {
            track_id,
            liked: true,
            updated_at: "2026-11-01T00:00:00Z".into(),
        })),
    )
    .await?;
    RuneModel::project(&db, &like, None).await?;
    assert_eq!(media_file_stats::Entity::find().all(&db).await?.len(), 2);
    Ok(())
}

#[tokio::test]
async fn source_scan_and_declared_effect_failure_are_transactional() -> Result<()> {
    use sea_orm::ConnectionTrait;
    let db = database().await?;
    let id = Uuid::new_v4();
    let object = key(Collection::Tracks, id);
    let payload = track(vec![MetadataTag {
        id: Uuid::new_v4(),
        position: 0,
        key: "track_title".into(),
        value: "Before".into(),
    }]);
    RuneModel::project(&db, &object, Some(&payload)).await?;
    let tx = db.begin().await?;
    let page = RuneModel::scan_on(
        &tx,
        &Collection::Tracks,
        None,
        sync_orm::row_policy::ScanLimits {
            max_records: 128,
            max_rows: 1024,
            max_bytes: 4 * 1024 * 1024,
            max_mutations: 128,
        },
    )
    .await?;
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].payload, payload);
    tx.commit().await?;
    db.execute_unprepared("CREATE TRIGGER reject_tag BEFORE UPDATE ON media_metadata BEGIN SELECT RAISE(ABORT, 'test fault'); END").await?;
    let Payload::Track(mut changed) = payload.clone() else {
        panic!()
    };
    changed.extension = "ogg".into();
    changed.metadata[0].value = "After".into();
    assert!(
        RuneModel::project(&db, &object, Some(&Payload::Track(changed)))
            .await
            .is_err()
    );
    assert_eq!(RuneModel::encode_on(&db, &object).await?, Some(payload));
    Ok(())
}

#[tokio::test]
async fn adding_a_mix_selection_preserves_unbound_directories_and_clause_groups() -> Result<()> {
    let db = database().await?;
    let id = Uuid::new_v4();
    let directory = Uuid::new_v4();
    let track = Uuid::new_v4();
    let value = Payload::Mix(Mix {
        name: "Unbound directory".into(),
        group: String::new(),
        mode: None,
        scriptlet_mode: false,
        locked: false,
        queries: vec![
            MixClause {
                id: directory,
                position: 0,
                operator: "lib::directory.deep".into(),
                parameter: String::new(),
                target_uuid: None,
                group: 7,
            },
            MixClause {
                id: Uuid::new_v4(),
                position: 1,
                operator: "lib::track".into(),
                parameter: String::new(),
                target_uuid: Some(track),
                group: 9,
            },
        ],
    });
    RuneModel::project(&db, &key(Collection::Mixes, id), Some(&value)).await?;
    let before = mix_queries::Entity::find()
        .order_by_asc(mix_queries::Column::Position)
        .all(&db)
        .await?;
    let actor = Uuid::new_v4();
    let store =
        database::sync::LibrarySyncStore::open(db.clone(), actor, Some(Uuid::new_v4())).await?;
    let local_id = RuneModel::local_id_on(&db, &key(Collection::Mixes, id))
        .await?
        .unwrap() as i32;
    add_item_to_mix(
        &db,
        &store.record_store(),
        local_id,
        "pipe::limit".into(),
        "50".into(),
    )
    .await?;
    let after = mix_queries::Entity::find()
        .order_by_asc(mix_queries::Column::Position)
        .all(&db)
        .await?;
    assert_eq!(
        &after[..2],
        before.as_slice(),
        "adding a clause rewrote existing selections"
    );
    assert!(after[0].local_directory.is_none());
    assert!(executable_queries_on(&db, &id.to_string()).await.is_err());
    Ok(())
}
