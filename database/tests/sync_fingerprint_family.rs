use anyhow::Result;
use database::test_fixtures::causal_library as support;
use database::{
    actions::fingerprint::{
        ComputedFingerprint, ComputedSimilarity, Configuration, assign_computed_fingerprint,
        assign_computed_similarity, compare_all_pairs, compute_file_fingerprints,
        default_similarity_configuration, get_duplicate_files, mark_duplicate_files,
        reset_duplicate_marks,
    },
    entities::{media_file_fingerprint, media_file_similarity, media_files},
    sync::{
        SeaOrmSyncStore, bindings,
        model::{Collection, Fingerprint, ObjectKey, Payload, ScoreValue, Similarity},
    },
};
use fsio::{FsIo, failing_read::OneFailingRead};
use migration::catalog_identity;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, Set, TransactionTrait, sea_query::Expr,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use support::fixtures;
use sync_orm::causal_progress;
use sync_orm::row_policy::BinaryValue;
use tag_editor::music_brainz::fingerprint::calc_fingerprint;
use uuid::Uuid;

#[tokio::test]
async fn fingerprint_source_change_and_failed_journal_leave_no_assignment() -> Result<()> {
    let node = Uuid::new_v4();
    let db = fixtures::memory_database(node).await?;
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
        causal_progress::head_on(&db, store.namespace())
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
        causal_progress::head_on(&db, store.namespace())
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
        row.source_uuid,
        catalog_identity::fingerprint(file_id, Some("chromaprint"), Some("default"), Some(1))
            .to_string()
    );
    assert_eq!(
        causal_progress::head_on(&db, store.namespace())
            .await?
            .sequence,
        1
    );
    Ok(())
}

#[tokio::test]
async fn fingerprint_write_failure_fails_the_batch_and_stops_queued_files() -> Result<()> {
    let node = Uuid::new_v4();
    let db = fixtures::memory_database(node).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let root = tempfile::tempdir()?;
    let fs = FsIo::new();
    fixtures::scan_audio_fixtures(
        &fs,
        &db,
        &node.to_string(),
        root.path(),
        &store,
        &["a.ogg", "b.ogg"],
    )
    .await?;
    let first_file = media_files::Entity::find()
        .order_by_asc(media_files::Column::Id)
        .one(&db)
        .await?
        .expect("scanned first audio file");
    db.execute_unprepared(&format!(
        "CREATE TRIGGER reject_fingerprint BEFORE INSERT ON media_file_fingerprint WHEN NEW.track_uuid = '{}' BEGIN SELECT RAISE(ABORT, 'test failure'); END",
        first_file.source_uuid
    ))
    .await?;

    let progress_count = Arc::new(AtomicUsize::new(0));
    let progress_count_callback = Arc::clone(&progress_count);
    let result = compute_file_fingerprints(
        Arc::new(fs),
        &db,
        root.path(),
        1,
        move |_, _| {
            progress_count_callback.fetch_add(1, Ordering::SeqCst);
        },
        None,
        &store,
    )
    .await;

    assert!(
        result.is_err(),
        "a fingerprint write failure must fail the batch"
    );
    assert_eq!(progress_count.load(Ordering::SeqCst), 0);
    assert_eq!(media_file_fingerprint::Entity::find().count(&db).await?, 0);
    Ok(())
}

#[tokio::test]
async fn a_read_error_while_decoding_stores_no_fingerprint() -> Result<()> {
    let node = Uuid::new_v4();
    let db = fixtures::memory_database(node).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let root = tempfile::tempdir()?;
    fixtures::scan_audio_fixtures(
        &FsIo::new(),
        &db,
        &node.to_string(),
        root.path(),
        &store,
        &["a.ogg"],
    )
    .await?;
    let track = root.path().join("a.ogg");
    let (full, _) = calc_fingerprint(&FsIo::new(), &track, &Configuration::default())?;
    let authored = causal_progress::head_on(&db, store.namespace())
        .await?
        .sequence;

    // While probing, the Ogg reader reads the headers and then, from byte 45658,
    // the rest of the file to find its length, so byte 45000 is first reached by
    // the decode loop, a few seconds into the track. Check that it still is: a
    // failure while probing or before the first buffer is reported otherwise.
    let direct = OneFailingRead::new(45_000);
    let decoded = calc_fingerprint(&direct.fs(), &track, &Configuration::default());
    assert!(direct.failed(), "the decoder never reached byte 45000");
    if let Err(error) = decoded {
        assert!(
            format!("{error:#}").contains("read audio: connection reset"),
            "the failure did not land in the decode loop: {error:#}"
        );
    }

    // The hash reads before the failure succeed, and so would the ones after it:
    // on master they run, and the partial fingerprint is stored.
    let fault = OneFailingRead::new(45_000);
    let processed = compute_file_fingerprints(
        Arc::new(fault.fs()),
        &db,
        root.path(),
        1,
        |_, _| {},
        None,
        &store,
    )
    .await?;

    assert!(fault.failed(), "the decoder never reached the failing read");
    assert_eq!(processed, 1);
    if let Some(row) = media_file_fingerprint::Entity::find().one(&db).await? {
        panic!(
            "a read error while decoding stored a fingerprint of {} bytes for a track of {} bytes",
            row.fingerprint.len(),
            full.len() * 4
        );
    }
    assert_eq!(
        causal_progress::head_on(&db, store.namespace())
            .await?
            .sequence,
        authored,
        "a read error while decoding authored a fingerprint record"
    );

    // The track has no fingerprint, so the next run computes the whole one.
    compute_file_fingerprints(
        Arc::new(FsIo::new()),
        &db,
        root.path(),
        1,
        |_, _| {},
        None,
        &store,
    )
    .await?;
    let row = media_file_fingerprint::Entity::find()
        .one(&db)
        .await?
        .expect("a clean run fingerprints the track");
    assert_eq!(
        row.fingerprint,
        full.into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>()
    );
    assert!(
        causal_progress::head_on(&db, store.namespace())
            .await?
            .sequence
            > authored
    );
    Ok(())
}

async fn fingerprinted_file(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    id: i32,
    file_uuid: Uuid,
    fingerprint: Vec<u32>,
) -> Result<media_files::Model> {
    let mut file = support::local_file(db, id, file_uuid, &format!("file-{id}")).await?;
    file.content_hash = Some(format!("{}", id).repeat(64));
    file.content_size = Some(100);
    let mut update: media_files::ActiveModel = file.clone().into();
    update.content_hash = ActiveValue::Set(file.content_hash.clone());
    update.content_size = ActiveValue::Set(file.content_size);
    file = update.update(db).await?;
    assign_computed_fingerprint(
        db,
        store,
        &file,
        ComputedFingerprint {
            algorithm: "chromaprint".into(),
            configuration: "rusty-chromaprint-0.3.0/default".into(),
            format_version: 1,
            bytes: fingerprint.into_iter().flat_map(u32::to_le_bytes).collect(),
            is_duplicated: 0,
        },
    )
    .await?;
    Ok(file)
}

async fn author_zero_similarity(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    first: &media_files::Model,
    second: &media_files::Model,
) -> Result<()> {
    let first_uuid = first.source_uuid.parse()?;
    let second_uuid = second.source_uuid.parse()?;
    let configuration = default_similarity_configuration();
    let key = ObjectKey {
        collection: Collection::Similarities,
        id: catalog_identity::similarity(
            first_uuid,
            second_uuid,
            Some("chromaprint-match"),
            Some(&configuration),
            Some(1),
        ),
    };
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let record = store
        .author_on(
            &tx,
            key,
            Some(Payload::Similarity(Similarity {
                first_file_id: first_uuid,
                second_file_id: second_uuid,
                first_source_hash: first.content_hash.clone(),
                first_source_size: first.content_size,
                second_source_hash: second.content_hash.clone(),
                second_source_size: second.content_size,
                algorithm: Some("chromaprint-match".into()),
                configuration: Some(configuration),
                format_version: Some(1),
                score: ScoreValue(0),
            })),
        )
        .await?;
    bindings::apply(&tx, &record.key, record.value.as_ref()).await?;
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
async fn comparison_authors_only_pairs_with_matching_segments() -> Result<()> {
    let node = Uuid::new_v4();
    let db = fixtures::memory_database(node).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let first = vec![0x1111_1111; 64];
    let first_file = fingerprinted_file(&db, &store, 1, Uuid::new_v4(), first.clone()).await?;
    let _second_file = fingerprinted_file(&db, &store, 2, Uuid::new_v4(), first).await?;
    let third_file =
        fingerprinted_file(&db, &store, 3, Uuid::new_v4(), vec![0xeeee_eeee; 64]).await?;
    author_zero_similarity(&db, &store, &first_file, &third_file).await?;

    compare_all_pairs(&db, |_, _| {}, &Configuration::default(), None, &store).await?;

    let rows = media_file_similarity::Entity::find().all(&db).await?;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].similarity > 0.0);
    Ok(())
}

#[tokio::test]
async fn zero_similarity_removes_the_synced_row() -> Result<()> {
    let node = Uuid::new_v4();
    let db = fixtures::memory_database(node).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let first_uuid = Uuid::new_v4();
    let second_uuid = Uuid::new_v4();
    let first = fingerprinted_file(&db, &store, 1, first_uuid, vec![1, 2, 3, 4]).await?;
    let second = fingerprinted_file(&db, &store, 2, second_uuid, vec![5, 6, 7, 8]).await?;
    let first_fingerprint = vec![1u32, 2, 3, 4]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
    let second_fingerprint = vec![5u32, 6, 7, 8]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
    let computation = |score| ComputedSimilarity {
        first_fingerprint: &first_fingerprint,
        second_fingerprint: &second_fingerprint,
        algorithm: "chromaprint-match".into(),
        configuration: "rusty-chromaprint-0.3.0/default".into(),
        format_version: 1,
        score,
    };
    assign_computed_similarity(&db, &store, &first, &second, computation(0.5)).await?;
    let sequence_after_match = causal_progress::head_on(&db, store.namespace())
        .await?
        .sequence;
    assert_eq!(
        media_file_similarity::Entity::find().all(&db).await?.len(),
        1
    );

    assign_computed_similarity(&db, &store, &first, &second, computation(0.0)).await?;

    assert!(
        media_file_similarity::Entity::find()
            .all(&db)
            .await?
            .is_empty()
    );
    assert_eq!(
        causal_progress::head_on(&db, store.namespace())
            .await?
            .sequence,
        sequence_after_match + 1
    );
    let key = ObjectKey {
        collection: Collection::Similarities,
        id: catalog_identity::similarity(
            first_uuid,
            second_uuid,
            Some("chromaprint-match"),
            Some("rusty-chromaprint-0.3.0/default"),
            Some(1),
        ),
    };
    assert!(store.get_on(&db, &key).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn missing_zero_score_pairs_preserve_zero_threshold_duplicate_marks() -> Result<()> {
    let node = Uuid::new_v4();
    let db = fixtures::memory_database(node).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let first = fingerprinted_file(&db, &store, 1, Uuid::new_v4(), vec![1; 64]).await?;
    let _second = fingerprinted_file(&db, &store, 2, Uuid::new_v4(), vec![1; 64]).await?;
    let third = fingerprinted_file(&db, &store, 3, Uuid::new_v4(), vec![u32::MAX; 64]).await?;
    compare_all_pairs(&db, |_, _| {}, &Configuration::default(), None, &store).await?;

    let first_uuid = first.source_uuid.parse()?;
    let third_uuid = third.source_uuid.parse()?;
    let key = ObjectKey {
        collection: Collection::Similarities,
        id: catalog_identity::similarity(
            first_uuid,
            third_uuid,
            Some("chromaprint-match"),
            Some("rusty-chromaprint-0.3.0/default"),
            Some(1),
        ),
    };
    author_zero_similarity(&db, &store, &first, &third).await?;

    let marked_before = mark_duplicate_files(&db, 0.0, |_, _| {}, None, &store).await?;
    let before: Vec<_> = media_file_fingerprint::Entity::find()
        .order_by_asc(media_file_fingerprint::Column::TrackUuid)
        .all(&db)
        .await?
        .into_iter()
        .map(|row| (row.track_uuid, row.is_duplicated))
        .collect();
    reset_duplicate_marks(&db, |_, _| {}, &store).await?;
    let zero_rows = media_file_similarity::Entity::find()
        .filter(media_file_similarity::Column::Similarity.eq(0.0))
        .all(&db)
        .await?;
    for row in zero_rows {
        let key = ObjectKey {
            id: row.source_uuid.parse()?,
            ..key.clone()
        };
        let tx = db.begin().await?;
        store.lock(&tx).await?;
        store.author_on(&tx, key.clone(), None).await?;
        bindings::apply(&tx, &key, None).await?;
        tx.commit().await?;
    }
    assert_eq!(
        mark_duplicate_files(&db, 0.0, |_, _| {}, None, &store).await?,
        marked_before
    );
    let after: Vec<_> = media_file_fingerprint::Entity::find()
        .order_by_asc(media_file_fingerprint::Column::TrackUuid)
        .all(&db)
        .await?
        .into_iter()
        .map(|row| (row.track_uuid, row.is_duplicated))
        .collect();
    assert_eq!(before, after);
    Ok(())
}

/// Each file's duplicate mark in the table and in its synced record, ordered by file id.
async fn duplicate_marks(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
) -> Result<Vec<(i32, i32, i32)>> {
    let mut marks = Vec::new();
    for file in media_files::Entity::find()
        .order_by_asc(media_files::Column::Id)
        .all(db)
        .await?
    {
        let row = media_file_fingerprint::Entity::find()
            .filter(media_file_fingerprint::Column::TrackUuid.eq(file.source_uuid.clone()))
            .one(db)
            .await?
            .unwrap();
        let record = store
            .get_on(
                db,
                &ObjectKey {
                    collection: Collection::Fingerprints,
                    id: row.source_uuid.parse()?,
                },
            )
            .await?
            .unwrap();
        let Some(Payload::Fingerprint(value)) = record.value else {
            panic!("fingerprint record without a fingerprint payload");
        };
        marks.push((file.id, row.is_duplicated, value.is_duplicated));
    }
    Ok(marks)
}

#[tokio::test]
async fn raising_the_threshold_unmarks_pairs_below_it() -> Result<()> {
    let node = Uuid::new_v4();
    let db = fixtures::memory_database(node).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    fingerprinted_file(&db, &store, 1, Uuid::new_v4(), vec![0x1111_1111; 64]).await?;
    fingerprinted_file(&db, &store, 2, Uuid::new_v4(), vec![0x1111_111e; 64]).await?;
    compare_all_pairs(&db, |_, _| {}, &Configuration::default(), None, &store).await?;
    let similarity = media_file_similarity::Entity::find()
        .one(&db)
        .await?
        .unwrap()
        .similarity;
    assert!(
        (0.85..0.95).contains(&similarity),
        "similarity {similarity}"
    );

    assert_eq!(
        mark_duplicate_files(&db, 0.85, |_, _| {}, None, &store).await?,
        1
    );
    assert_eq!(get_duplicate_files(&db).await?.len(), 1);
    assert_eq!(
        mark_duplicate_files(&db, 0.95, |_, _| {}, None, &store).await?,
        0
    );
    assert!(get_duplicate_files(&db).await?.is_empty());
    assert_eq!(
        duplicate_marks(&db, &store).await?,
        vec![(1, 0, 0), (2, 0, 0)]
    );
    Ok(())
}

#[tokio::test]
async fn raising_the_threshold_keeps_groups_that_still_qualify() -> Result<()> {
    let node = Uuid::new_v4();
    let db = fixtures::memory_database(node).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    // Files 1 and 2 are identical; file 3 is only similar to both.
    // With equal sample rates the smallest source identity is kept.
    fingerprinted_file(&db, &store, 1, Uuid::from_u128(1), vec![0x1111_1111; 64]).await?;
    fingerprinted_file(&db, &store, 2, Uuid::from_u128(2), vec![0x1111_1111; 64]).await?;
    fingerprinted_file(&db, &store, 3, Uuid::from_u128(3), vec![0x1111_111e; 64]).await?;
    compare_all_pairs(&db, |_, _| {}, &Configuration::default(), None, &store).await?;

    assert_eq!(
        mark_duplicate_files(&db, 0.85, |_, _| {}, None, &store).await?,
        2
    );
    assert_eq!(
        duplicate_marks(&db, &store).await?,
        vec![(1, 0, 0), (2, 1, 1), (3, 1, 1)]
    );
    assert_eq!(
        mark_duplicate_files(&db, 0.95, |_, _| {}, None, &store).await?,
        1
    );
    assert_eq!(
        duplicate_marks(&db, &store).await?,
        vec![(1, 0, 0), (2, 1, 1), (3, 0, 0)]
    );
    assert_eq!(
        get_duplicate_files(&db)
            .await?
            .into_iter()
            .map(|file| file.id)
            .collect::<Vec<_>>(),
        vec![2]
    );
    Ok(())
}

#[tokio::test]
async fn failed_mark_replacement_keeps_previous_marks() -> Result<()> {
    let node = Uuid::new_v4();
    let db = fixtures::memory_database(node).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    // Two unrelated pairs, each similar enough for the lower threshold only.
    fingerprinted_file(&db, &store, 1, Uuid::from_u128(1), vec![0x1111_1111; 64]).await?;
    fingerprinted_file(&db, &store, 2, Uuid::from_u128(2), vec![0x1111_111e; 64]).await?;
    fingerprinted_file(&db, &store, 3, Uuid::from_u128(3), vec![0x7777_7777; 64]).await?;
    fingerprinted_file(&db, &store, 4, Uuid::from_u128(4), vec![0x7777_7778; 64]).await?;
    compare_all_pairs(&db, |_, _| {}, &Configuration::default(), None, &store).await?;
    assert_eq!(
        mark_duplicate_files(&db, 0.85, |_, _| {}, None, &store).await?,
        2
    );
    let marks = duplicate_marks(&db, &store).await?;
    assert_eq!(marks, vec![(1, 0, 0), (2, 1, 1), (3, 0, 0), (4, 1, 1)]);
    let head = sync_orm::causal_progress::head_on(&db, store.namespace())
        .await?
        .sequence;
    let files = media_files::Entity::find().count(&db).await?;

    // The first cleared mark is journaled, the second one fails.
    db.execute_unprepared(&format!(
        "CREATE TRIGGER reject_second_mark BEFORE INSERT ON sync_journal WHEN NEW.sequence={} BEGIN SELECT RAISE(ABORT, 'injected failure'); END",
        head + 2
    ))
    .await?;
    assert!(
        mark_duplicate_files(&db, 0.95, |_, _| {}, None, &store)
            .await
            .is_err()
    );
    assert_eq!(duplicate_marks(&db, &store).await?, marks);
    assert_eq!(
        sync_orm::causal_progress::head_on(&db, store.namespace())
            .await?
            .sequence,
        head
    );
    assert_eq!(media_files::Entity::find().count(&db).await?, files);

    db.execute_unprepared("DROP TRIGGER reject_second_mark")
        .await?;
    assert_eq!(
        mark_duplicate_files(&db, 0.95, |_, _| {}, None, &store).await?,
        0
    );
    assert_eq!(
        duplicate_marks(&db, &store).await?,
        vec![(1, 0, 0), (2, 0, 0), (3, 0, 0), (4, 0, 0)]
    );
    assert_eq!(
        sync_orm::causal_progress::head_on(&db, store.namespace())
            .await?
            .sequence,
        head + 2
    );
    assert_eq!(media_files::Entity::find().count(&db).await?, files);
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
    let db = fixtures::memory_database(node).await?;
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
            id: catalog_identity::fingerprint(first, Some("algo"), Some(config), Some(1)),
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
                    bytes: BinaryValue(vec![1, 2, 3, 4]),
                    is_duplicated: 1,
                })),
            )
            .await?;
        bindings::apply(&tx, &record.key, record.value.as_ref()).await?;
        let key = ObjectKey {
            collection: Collection::Similarities,
            id: catalog_identity::similarity(first, second, Some("algo"), Some(config), Some(1)),
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
        causal_progress::head_on(&db, store.namespace())
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
                    id: row.source_uuid.parse()?,
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
    let db = fixtures::memory_database(node).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let file_id = Uuid::new_v4();
    let legacy = catalog_identity::fingerprint(file_id, None, None, None);
    support::local_file(&db, 1, file_id, "a").await?;
    media_file_fingerprint::Entity::insert(media_file_fingerprint::ActiveModel {
        track_uuid: Set(file_id.to_string()),
        fingerprint: Set(vec![1, 2, 3, 4]),
        is_duplicated: Set(1),
        source_uuid: Set(legacy.to_string()),
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
    assert!(rows.iter().any(|row| row.source_uuid == legacy.to_string()
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
    let db = fixtures::memory_database(node).await?;
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
    let legacy = catalog_identity::fingerprint(first, None, None, None);
    media_file_fingerprint::Entity::insert(media_file_fingerprint::ActiveModel {
        track_uuid: Set(first.to_string()),
        fingerprint: Set(vec![1, 2, 3, 4]),
        is_duplicated: Set(0),
        source_uuid: Set(legacy.to_string()),
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
        &[],
    )
    .await?;
    assert_eq!(files.into_iter().map(|f| f.id).collect::<Vec<_>>(), vec![2]);
    Ok(())
}

#[tokio::test]
async fn recommendations_exclude_verified_duplicates_only() -> Result<()> {
    use database::{
        actions::{
            fingerprint::assign_duplicate_mark, mixes::query_mix_media_files,
            recommendation::sync_recommendation,
        },
        connection::RecommendationDbConnection,
    };
    let node = Uuid::new_v4();
    let db = fixtures::memory_database(node).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    // Track 1 has a verified current duplicate mark and must stay hidden.
    fingerprinted_file(&db, &store, 1, Uuid::new_v4(), vec![1; 64]).await?;
    assign_duplicate_mark(&db, &store, 1, 1).await?;
    // Track 2 is the unmarked seed.
    fingerprinted_file(&db, &store, 2, Uuid::new_v4(), vec![1; 64]).await?;
    // Track 3 was marked, then its content changed, so the mark is stale.
    fingerprinted_file(&db, &store, 3, Uuid::new_v4(), vec![1; 64]).await?;
    assign_duplicate_mark(&db, &store, 3, 1).await?;
    media_files::Entity::update_many()
        .col_expr(
            media_files::Column::ContentHash,
            Expr::value("c".repeat(64)),
        )
        .filter(media_files::Column::Id.eq(3))
        .exec(&db)
        .await?;
    // Track 4 only has a marked fingerprint of an unsupported legacy format.
    let legacy_file = Uuid::new_v4();
    support::local_file(&db, 4, legacy_file, "file-4").await?;
    media_files::Entity::update_many()
        .col_expr(
            media_files::Column::ContentHash,
            Expr::value("d".repeat(64)),
        )
        .col_expr(media_files::Column::ContentSize, Expr::value(Some(100)))
        .filter(media_files::Column::Id.eq(4))
        .exec(&db)
        .await?;
    media_file_fingerprint::Entity::insert(media_file_fingerprint::ActiveModel {
        track_uuid: Set(legacy_file.to_string()),
        fingerprint: Set(vec![1, 2, 3, 4]),
        is_duplicated: Set(1),
        source_uuid: Set(catalog_identity::fingerprint(legacy_file, None, None, None).to_string()),
        ..Default::default()
    })
    .exec(&db)
    .await?;
    // Distance from the seed ranks the tracks 2, 1, 4, 3, unlike their ids.
    for (id, rms) in [(1, 0.21), (2, 0.2), (3, 0.4), (4, 0.3)] {
        // Track 3 was analysed again after its content changed above.
        let (content_hash, content_size) = match id {
            3 => (format!("'{}'", "c".repeat(64)), "100"),
            4 => (format!("'{}'", "d".repeat(64)), "100"),
            _ => (format!("'{}'", format!("{id}").repeat(64)), "100"),
        };
        db.execute_unprepared(&format!(
            "INSERT INTO media_analysis(file_id,analysis_version,rms,source_uuid,source_hash,source_size) \
             VALUES({id},{},{rms},'analysis-{id}',{content_hash},{content_size})",
            analysis::CURRENT_ANALYSIS_VERSION
        ))
        .await?;
    }
    let recommendations = RecommendationDbConnection::new();
    sync_recommendation(&db, &recommendations).await?;
    let ids = |files: Vec<media_files::Model>| files.into_iter().map(|f| f.id).collect::<Vec<_>>();

    let normal = query_mix_media_files(
        &db,
        &recommendations,
        vec![("lib::all".into(), "true".into())],
        0,
        20,
        &[],
    )
    .await?;
    assert_eq!(ids(normal), vec![2, 3, 4]);

    let suggested = query_mix_media_files(
        &db,
        &recommendations,
        vec![
            ("lib::track".into(), "2".into()),
            ("pipe::recommend".into(), "-1".into()),
        ],
        0,
        20,
        &[],
    )
    .await?;
    assert_eq!(ids(suggested), vec![2, 4, 3]);
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn attach_download_accepts_symlinked_root() -> Result<()> {
    use std::path::Path;

    use database::actions::{downloads::prepare_download, file::attach_download};
    use database::sync::model::BlobRef;
    use fsio::FsIo;
    use std::{fs, os::unix::fs::symlink};
    let node = Uuid::new_v4();
    let db = fixtures::memory_database(node).await?;
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
    fs::create_dir(&actual)?;
    let root = dir.path().join("link");
    symlink(&actual, &root)?;
    let fs = FsIo::new();
    let path = prepare_download(&db, &fs, &root, id, &blob, "ogg").await?;
    fs::write(&path, data)?;
    attach_download(&db, &store, &fs, &root, id, &blob, &path).await?;
    let row = media_files::Entity::find_by_id(1).one(&db).await?.unwrap();
    assert!(row.local_available);
    assert!(!Path::new(&row.directory).is_absolute());
    assert_eq!(actual.join(&row.directory).join(&row.file_name), path);
    Ok(())
}

#[tokio::test]
async fn oversized_fingerprint_is_local_only() -> Result<()> {
    use database::sync::model::{Collection, ObjectKey};
    use sea_orm::TransactionTrait;
    let node = Uuid::new_v4();
    let db = fixtures::memory_database(node).await?;
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
        id: catalog_identity::fingerprint(file_id, Some("chromaprint"), Some("default"), Some(1)),
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
    use database::sync::model::{Collection, ObjectKey};
    use sea_orm::TransactionTrait;
    let node = Uuid::new_v4();
    let db = fixtures::memory_database(node).await?;
    let library = fixtures::open_library(db.clone(), node, Some(Uuid::new_v4())).await?;
    let mut keys = Vec::new();
    for (n, size) in [(1, 40000), (2, 4)] {
        let id = Uuid::new_v4();
        support::local_file(&db, n, id, "file").await?;
        let key = ObjectKey {
            collection: Collection::Fingerprints,
            id: catalog_identity::fingerprint(id, None, None, None),
        };
        media_file_fingerprint::Entity::insert(media_file_fingerprint::ActiveModel {
            track_uuid: Set(id.to_string()),
            fingerprint: Set(vec![1; size]),
            is_duplicated: Set(0),
            source_uuid: Set(key.id.to_string()),
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
