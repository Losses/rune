#[path = "support/causal_library.rs"]
mod support;

use anyhow::Result;
use database::{
    actions::fingerprint::{
        ComputedFingerprint, assign_computed_fingerprint, reset_duplicate_marks,
    },
    entities::{media_file_fingerprint, media_file_similarity, media_files},
    sync::{
        SeaOrmSyncStore,
        model::{Fingerprint, ScoreValue, Similarity},
    },
};
use migration::{Migrator, MigratorTrait};
use sea_orm::{
    ColumnTrait, ConnectionTrait, Database, EntityTrait, QueryFilter, Set, sea_query::Expr,
};
use uuid::Uuid;

#[tokio::test]
async fn fingerprint_source_change_and_failed_journal_leave_no_assignment() -> Result<()> {
    let node = Uuid::new_v4();
    let _ = migration::initialize_node_id(node.to_string());
    let db = Database::connect("sqlite::memory:").await?;
    Migrator::up(&db, None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let file_id = Uuid::new_v4();
    support::local_file(&db, 1, file_id, "file").await?;
    let observed = media_files::Entity::find_by_id(1).one(&db).await?.unwrap();
    media_files::Entity::update_many()
        .col_expr(media_files::Column::LastModified, Expr::value("changed"))
        .filter(media_files::Column::Id.eq(1))
        .exec(&db)
        .await?;
    assert!(
        assign_computed_fingerprint(
            &db,
            &store,
            &observed,
            ComputedFingerprint {
                algorithm: "chromaprint".into(),
                configuration: "default".into(),
                format_version: 1,
                bytes: vec![1, 2, 3, 4],
                is_duplicated: 0,
            },
        )
        .await
        .is_err()
    );
    assert!(
        media_file_fingerprint::Entity::find()
            .one(&db)
            .await?
            .is_none()
    );
    assert_eq!(
        sync_orm::causal_progress::head_on(&db, store.namespace())
            .await?
            .sequence,
        0
    );
    let current = media_files::Entity::find_by_id(1).one(&db).await?.unwrap();
    db.execute_unprepared("CREATE TRIGGER reject_fingerprint BEFORE INSERT ON media_file_fingerprint BEGIN SELECT RAISE(ABORT, 'test'); END").await?;
    assert!(
        assign_computed_fingerprint(
            &db,
            &store,
            &current,
            ComputedFingerprint {
                algorithm: "chromaprint".into(),
                configuration: "default".into(),
                format_version: 1,
                bytes: vec![1, 2, 3, 4],
                is_duplicated: 0,
            },
        )
        .await
        .is_err()
    );
    assert_eq!(
        sync_orm::causal_progress::head_on(&db, store.namespace())
            .await?
            .sequence,
        0
    );
    db.execute_unprepared("DROP TRIGGER reject_fingerprint")
        .await?;
    assign_computed_fingerprint(
        &db,
        &store,
        &current,
        ComputedFingerprint {
            algorithm: "chromaprint".into(),
            configuration: "default".into(),
            format_version: 1,
            bytes: vec![1, 2, 3, 4],
            is_duplicated: 0,
        },
    )
    .await?;
    let row = media_file_fingerprint::Entity::find()
        .one(&db)
        .await?
        .unwrap();
    assert_eq!(row.fingerprint, vec![1, 2, 3, 4]);
    assert_eq!(
        row.hlc_uuid,
        migration::catalog_identity::fingerprint(
            file_id,
            Some("chromaprint"),
            Some("default"),
            Some(1)
        )
        .to_string()
    );
    assert_eq!(
        sync_orm::causal_progress::head_on(&db, store.namespace())
            .await?
            .sequence,
        1
    );
    Ok(())
}

#[tokio::test]
async fn independent_configurations_coexist_in_primary_rows() -> Result<()> {
    use database::sync::{
        bindings,
        model::{Collection, ObjectKey, Payload},
    };
    use sea_orm::TransactionTrait;
    let node = Uuid::new_v4();
    let _ = migration::initialize_node_id(node.to_string());
    let db = Database::connect("sqlite::memory:").await?;
    Migrator::up(&db, None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    support::local_file(&db, 1, first, "a").await?;
    support::local_file(&db, 2, second, "b").await?;
    for config in ["a", "b"] {
        let tx = db.begin().await?;
        store.lock(&tx).await?;
        let key = ObjectKey {
            collection: Collection::Fingerprints,
            id: migration::catalog_identity::fingerprint(
                first,
                Some("algo"),
                Some(config),
                Some(1),
            ),
        };
        let record = store
            .author_on(
                &tx,
                key,
                Some(Payload::Fingerprint(Fingerprint {
                    file_id: first,
                    source_hash: None,
                    source_size: None,
                    algorithm: Some("algo".into()),
                    configuration: Some(config.into()),
                    format_version: Some(1),
                    bytes: vec![1, 2, 3, 4],
                    is_duplicated: 1,
                })),
            )
            .await?;
        bindings::apply(&tx, &record.key, record.value.as_ref()).await?;
        let key = ObjectKey {
            collection: Collection::Similarities,
            id: migration::catalog_identity::similarity(
                first,
                second,
                Some("algo"),
                Some(config),
                Some(1),
            ),
        };
        let record = store
            .author_on(
                &tx,
                key,
                Some(Payload::Similarity(Similarity {
                    first_file_id: first,
                    second_file_id: second,
                    first_source_hash: None,
                    first_source_size: None,
                    second_source_hash: None,
                    second_source_size: None,
                    algorithm: Some("algo".into()),
                    configuration: Some(config.into()),
                    format_version: Some(1),
                    score: ScoreValue(0.75f32.to_bits()),
                })),
            )
            .await?;
        bindings::apply(&tx, &record.key, record.value.as_ref()).await?;
        tx.commit().await?;
    }
    assert_eq!(
        media_file_fingerprint::Entity::find().all(&db).await?.len(),
        2
    );
    assert_eq!(
        media_file_similarity::Entity::find().all(&db).await?.len(),
        2
    );
    assert_eq!(
        sync_orm::causal_progress::head_on(&db, store.namespace())
            .await?
            .sequence,
        4
    );
    db.execute_unprepared("CREATE TRIGGER reject_reset BEFORE INSERT ON sync_journal BEGIN SELECT RAISE(ABORT, 'test'); END").await?;
    assert!(reset_duplicate_marks(&db, |_, _| {}, &store).await.is_err());
    assert!(
        media_file_fingerprint::Entity::find()
            .all(&db)
            .await?
            .iter()
            .all(|row| row.is_duplicated == 1)
    );
    db.execute_unprepared("DROP TRIGGER reject_reset").await?;
    assert_eq!(reset_duplicate_marks(&db, |_, _| {}, &store).await?, 2);
    for row in media_file_fingerprint::Entity::find().all(&db).await? {
        assert_eq!(row.is_duplicated, 0);
        let record = store
            .get_on(
                &db,
                &ObjectKey {
                    collection: Collection::Fingerprints,
                    id: row.hlc_uuid.parse()?,
                },
            )
            .await?
            .unwrap();
        assert!(
            matches!(record.value, Some(Payload::Fingerprint(value)) if value.is_duplicated == 0)
        );
    }
    Ok(())
}

#[tokio::test]
async fn legacy_fingerprint_does_not_block_supported_assignment() -> Result<()> {
    use database::actions::fingerprint::current_fingerprint;
    let node = Uuid::new_v4();
    let _ = migration::initialize_node_id(node.to_string());
    let db = Database::connect("sqlite::memory:").await?;
    Migrator::up(&db, None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let file_id = Uuid::new_v4();
    let legacy = migration::catalog_identity::fingerprint(file_id, None, None, None);
    support::local_file(&db, 1, file_id, "a").await?;
    media_file_fingerprint::Entity::insert(media_file_fingerprint::ActiveModel {
        track_uuid: Set(file_id.to_string()),
        fingerprint: Set(vec![1, 2, 3, 4]),
        is_duplicated: Set(1),
        hlc_uuid: Set(legacy.to_string()),
        ..Default::default()
    })
    .exec(&db)
    .await?;
    let file = media_files::Entity::find_by_id(1).one(&db).await?.unwrap();
    assign_computed_fingerprint(
        &db,
        &store,
        &file,
        ComputedFingerprint {
            algorithm: "chromaprint".into(),
            configuration: "rusty-chromaprint-0.3.0/default".into(),
            format_version: 1,
            bytes: vec![5, 6, 7, 8],
            is_duplicated: 0,
        },
    )
    .await?;
    let rows = media_file_fingerprint::Entity::find().all(&db).await?;
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| row.hlc_uuid == legacy.to_string()
        && row.is_duplicated == 1
        && row.fingerprint == vec![1, 2, 3, 4]));
    let current = current_fingerprint(&db, &file).await?.unwrap();
    assert_eq!(current.fingerprint, vec![5, 6, 7, 8]);
    media_files::Entity::update_many()
        .col_expr(media_files::Column::LastModified, Expr::value("new"))
        .col_expr(
            media_files::Column::ContentHash,
            Expr::value("a".repeat(64)),
        )
        .col_expr(media_files::Column::ContentSize, Expr::value(3))
        .filter(media_files::Column::Id.eq(1))
        .exec(&db)
        .await?;
    let changed = media_files::Entity::find_by_id(1).one(&db).await?.unwrap();
    assign_computed_fingerprint(
        &db,
        &store,
        &changed,
        ComputedFingerprint {
            algorithm: "chromaprint".into(),
            configuration: "rusty-chromaprint-0.3.0/default".into(),
            format_version: 1,
            bytes: vec![9, 10, 11, 12],
            is_duplicated: 0,
        },
    )
    .await?;
    let replaced = current_fingerprint(&db, &changed).await?.unwrap();
    assert_eq!(replaced.id, current.id);
    assert_eq!(replaced.fingerprint, vec![9, 10, 11, 12]);
    assert_eq!(
        media_file_fingerprint::Entity::find().all(&db).await?.len(),
        2
    );
    Ok(())
}

#[tokio::test]
async fn duplicate_filter_uses_supported_current_identity_once() -> Result<()> {
    use database::{
        actions::{fingerprint::get_duplicate_files, mixes::query_mix_media_files},
        connection::RecommendationDbConnection,
    };
    let node = Uuid::new_v4();
    let _ = migration::initialize_node_id(node.to_string());
    let db = Database::connect("sqlite::memory:").await?;
    Migrator::up(&db, None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let digest = blake3::hash(b"audio").to_hex().to_string();
    support::local_file(&db, 1, first, "a").await?;
    support::local_file(&db, 2, second, "b").await?;
    media_files::Entity::update_many()
        .col_expr(media_files::Column::ContentHash, Expr::value(digest))
        .col_expr(media_files::Column::ContentSize, Expr::value(5))
        .exec(&db)
        .await?;
    let legacy = migration::catalog_identity::fingerprint(first, None, None, None);
    media_file_fingerprint::Entity::insert(media_file_fingerprint::ActiveModel {
        track_uuid: Set(first.to_string()),
        fingerprint: Set(vec![1, 2, 3, 4]),
        is_duplicated: Set(0),
        hlc_uuid: Set(legacy.to_string()),
        ..Default::default()
    })
    .exec(&db)
    .await?;
    let file = media_files::Entity::find_by_id(1).one(&db).await?.unwrap();
    assign_computed_fingerprint(
        &db,
        &store,
        &file,
        ComputedFingerprint {
            algorithm: "chromaprint".into(),
            configuration: "rusty-chromaprint-0.3.0/default".into(),
            format_version: 1,
            bytes: vec![5, 6, 7, 8],
            is_duplicated: 1,
        },
    )
    .await?;
    assert_eq!(
        get_duplicate_files(&db)
            .await?
            .into_iter()
            .map(|f| f.id)
            .collect::<Vec<_>>(),
        vec![1]
    );
    let files = query_mix_media_files(
        &db,
        &RecommendationDbConnection::new(),
        vec![("lib::all".into(), "true".into())],
        0,
        20,
    )
    .await?;
    assert_eq!(files.into_iter().map(|f| f.id).collect::<Vec<_>>(), vec![2]);
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn attach_download_accepts_symlinked_root() -> Result<()> {
    use database::actions::{downloads::prepare_download, file::attach_download};
    use database::sync::model::BlobRef;
    use fsio::FsIo;
    use std::os::unix::fs::symlink;
    let node = Uuid::new_v4();
    let _ = migration::initialize_node_id(node.to_string());
    let db = Database::connect("sqlite::memory:").await?;
    Migrator::up(&db, None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let id = Uuid::new_v4();
    support::local_file(&db, 1, id, "file").await?;
    let data = b"downloaded audio";
    let blob = BlobRef {
        digest: blake3::hash(data).to_hex().to_string(),
        size: data.len() as u64,
    };
    media_files::Entity::update_many()
        .filter(media_files::Column::Id.eq(1))
        .col_expr(
            media_files::Column::ContentHash,
            Expr::value(Some(blob.digest.clone())),
        )
        .col_expr(
            media_files::Column::ContentSize,
            Expr::value(Some(blob.size as i64)),
        )
        .col_expr(media_files::Column::LocalAvailable, Expr::value(false))
        .exec(&db)
        .await?;
    let dir = tempfile::tempdir()?;
    let actual = dir.path().join("actual");
    std::fs::create_dir(&actual)?;
    let root = dir.path().join("link");
    symlink(&actual, &root)?;
    let fs = FsIo::new();
    let path = prepare_download(&db, &fs, &root, id, &blob, "ogg").await?;
    std::fs::write(&path, data)?;
    attach_download(&db, &store, &fs, &root, id, &blob, &path).await?;
    let row = media_files::Entity::find_by_id(1).one(&db).await?.unwrap();
    assert!(row.local_available);
    assert!(!std::path::Path::new(&row.directory).is_absolute());
    assert_eq!(actual.join(&row.directory).join(&row.file_name), path);
    Ok(())
}

#[tokio::test]
async fn oversized_fingerprint_is_local_only() -> Result<()> {
    use database::sync::model::{Collection, ObjectKey};
    use sea_orm::TransactionTrait;
    let node = Uuid::new_v4();
    let _ = migration::initialize_node_id(node.to_string());
    let db = Database::connect("sqlite::memory:").await?;
    Migrator::up(&db, None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let file_id = Uuid::new_v4();
    support::local_file(&db, 1, file_id, "file").await?;
    let file = media_files::Entity::find_by_id(1).one(&db).await?.unwrap();
    assign_computed_fingerprint(
        &db,
        &store,
        &file,
        ComputedFingerprint {
            algorithm: "chromaprint".into(),
            configuration: "default".into(),
            format_version: 1,
            bytes: vec![1; 40000],
            is_duplicated: 0,
        },
    )
    .await?;
    assert_eq!(
        media_file_fingerprint::Entity::find()
            .one(&db)
            .await?
            .unwrap()
            .fingerprint
            .len(),
        40000
    );
    let tx = db.begin().await?;
    let key = ObjectKey {
        collection: Collection::Fingerprints,
        id: migration::catalog_identity::fingerprint(
            file_id,
            Some("chromaprint"),
            Some("default"),
            Some(1),
        ),
    };
    assert!(store.get_on(&tx, &key).await?.is_none());
    tx.rollback().await?;
    assign_computed_fingerprint(
        &db,
        &store,
        &file,
        ComputedFingerprint {
            algorithm: "chromaprint".into(),
            configuration: "default".into(),
            format_version: 1,
            bytes: vec![1; 4],
            is_duplicated: 0,
        },
    )
    .await?;
    assign_computed_fingerprint(
        &db,
        &store,
        &file,
        ComputedFingerprint {
            algorithm: "chromaprint".into(),
            configuration: "default".into(),
            format_version: 1,
            bytes: vec![1; 40000],
            is_duplicated: 0,
        },
    )
    .await?;
    let tx = db.begin().await?;
    assert!(
        store
            .get_on(&tx, &key)
            .await?
            .is_none_or(|record| record.value.is_none())
    );
    tx.rollback().await?;
    assert_eq!(
        media_file_fingerprint::Entity::find()
            .one(&db)
            .await?
            .unwrap()
            .fingerprint
            .len(),
        40000
    );
    Ok(())
}

#[tokio::test]
async fn import_existing_skips_oversized_fingerprint() -> Result<()> {
    use database::sync::{
        LibrarySyncStore,
        model::{Collection, ObjectKey},
    };
    use sea_orm::TransactionTrait;
    let node = Uuid::new_v4();
    let _ = migration::initialize_node_id(node.to_string());
    let db = Database::connect("sqlite::memory:").await?;
    Migrator::up(&db, None).await?;
    let library = LibrarySyncStore::open(db.clone(), node, Some(Uuid::new_v4())).await?;
    let mut keys = Vec::new();
    for (n, size) in [(1, 40000), (2, 4)] {
        let id = Uuid::new_v4();
        support::local_file(&db, n, id, "file").await?;
        let key = ObjectKey {
            collection: Collection::Fingerprints,
            id: migration::catalog_identity::fingerprint(id, None, None, None),
        };
        media_file_fingerprint::Entity::insert(media_file_fingerprint::ActiveModel {
            track_uuid: Set(id.to_string()),
            fingerprint: Set(vec![1; size]),
            is_duplicated: Set(0),
            hlc_uuid: Set(key.id.to_string()),
            ..Default::default()
        })
        .exec(&db)
        .await?;
        keys.push(key);
    }
    library.import_existing(node).await?;
    let tx = db.begin().await?;
    let store = library.record_store();
    assert!(store.get_on(&tx, &keys[0]).await?.is_none());
    assert!(store.get_on(&tx, &keys[1]).await?.is_some());
    tx.rollback().await?;
    Ok(())
}
