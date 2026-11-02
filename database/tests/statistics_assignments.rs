#[path = "support/causal_library.rs"]
mod support;
use anyhow::Result;
use database::{
    actions::stats::{get_liked, increase_skipped, set_liked},
    entities::media_file_stats,
    sync::model::*,
};
use migration::{Migrator, MigratorTrait};
use sea_orm::{ConnectionTrait, Database, EntityTrait};
use uuid::Uuid;

#[tokio::test]
async fn device_rows_replay_and_likes_are_independent_and_atomic() -> Result<()> {
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let local = support::SqlLibrary::new(Uuid::new_v4(), a).await?;
    let db = &local.db;
    let track = Uuid::new_v4();
    local
        .peer
        .write_local(
            support::key(Collection::Tracks, track),
            Some(support::track()),
        )
        .await?;
    let store = local.library.record_store();
    set_liked(db, &store, 1, true).await?;
    increase_skipped(db, &store, 1, a).await?;
    support::fixture_statistics(db, 1, b, 4, 7, "receipt").await?;
    support::fixture_statistics(db, 1, b, 4, 7, "receipt").await?;
    support::fixture_statistics(db, 1, b, 5, 7, "receipt").await?;
    let rows = media_file_stats::Entity::find().all(db).await?;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .find(|r| r.device_id == a.to_string())
            .unwrap()
            .skipped,
        1
    );
    assert_eq!(
        rows.iter()
            .find(|r| r.device_id == b.to_string())
            .unwrap()
            .skipped,
        5
    );
    assert!(get_liked(db, 1).await?);
    let (first, second) = tokio::join!(
        increase_skipped(db, &store, 1, a),
        increase_skipped(db, &store, 1, a)
    );
    first?;
    second?;
    local.library.import_existing(a).await?;
    let head = local.peer.head().await?;
    local.library.import_existing(a).await?;
    assert_eq!(local.peer.head().await?, head);
    let foreign = support::key(
        Collection::PlaybackStatistics,
        Uuid::new_v5(&track, b.as_bytes()),
    );
    assert!(
        local.peer.get(&foreign).await?.is_none(),
        "initialization authored another device's row"
    );
    let own = support::key(
        Collection::PlaybackStatistics,
        Uuid::new_v5(&track, a.as_bytes()),
    );
    let Some(Payload::PlaybackStatistics(value)) = local.peer.get(&own).await?.unwrap().value
    else {
        anyhow::bail!("owner statistics missing");
    };
    assert_eq!(value.skipped, 3);
    db.execute_unprepared("CREATE TRIGGER reject_stats BEFORE INSERT ON sync_journal BEGIN SELECT RAISE(ABORT, 'injected statistics failure'); END").await?;
    assert!(increase_skipped(db, &store, 1, a).await.is_err());
    assert_eq!(
        media_file_stats::Entity::find()
            .all(db)
            .await?
            .iter()
            .find(|r| r.device_id == a.to_string())
            .unwrap()
            .skipped,
        3
    );
    assert_eq!(local.peer.head().await?, head);
    Ok(())
}
#[tokio::test]
async fn a_device_cannot_author_another_devices_statistics() -> Result<()> {
    let owner = Uuid::new_v4();
    let other = Uuid::new_v4();
    let track = Uuid::new_v4();
    let store = support::memory(Uuid::new_v4(), other)?;
    let key = support::key(
        Collection::PlaybackStatistics,
        Uuid::new_v5(&track, owner.as_bytes()),
    );
    assert!(
        store
            .write_local(
                key.clone(),
                Some(Payload::PlaybackStatistics(PlaybackStatistics {
                    track_id: track,
                    device_id: owner,
                    skipped: 10,
                    played_through: 20,
                    updated_at: "now".into(),
                }))
            )
            .await
            .is_err()
    );
    assert!(store.write_local(key, None).await.is_err());
    assert_eq!(store.head().await?.sequence, 0);
    Ok(())
}

#[tokio::test]
async fn mix_filters_and_orders_by_likes_and_per_device_totals() -> Result<()> {
    use database::{
        actions::mixes::query_mix_media_files,
        connection::RecommendationDbConnection,
        entities::{media_analysis, media_files},
    };
    use sea_orm::Set;
    let node = Uuid::new_v4();
    let _ = migration::initialize_node_id(node.to_string());
    let db = Database::connect("sqlite::memory:").await?;
    Migrator::up(&db, None).await?;
    for id in 1..=3 {
        media_files::Entity::insert(media_files::ActiveModel {
            id: Set(id),
            file_name: Set(format!("{id}.ogg")),
            directory: Set(String::new()),
            extension: Set("ogg".into()),
            file_hash: Set(id.to_string()),
            last_modified: Set("2020-01-01".into()),
            sample_rate: Set(44100),
            duration: Set(1.into()),
            hlc_uuid: Set(Uuid::new_v4().to_string()),
            local_available: Set(true),
            ..Default::default()
        })
        .exec(&db)
        .await?;
        media_analysis::Entity::insert(media_analysis::ActiveModel {
            file_id: Set(id),
            analysis_version: Set(analysis::CURRENT_ANALYSIS_VERSION),
            hlc_uuid: Set(Uuid::new_v4().to_string()),
            ..Default::default()
        })
        .exec(&db)
        .await?;
    }
    support::fixture_like(&db, 2, false, "now").await?;
    support::fixture_like(&db, 3, true, "now").await?;
    support::fixture_statistics(&db, 2, Uuid::new_v4(), 2, 1, "now").await?;
    support::fixture_statistics(&db, 2, Uuid::new_v4(), 3, 1, "now").await?;
    support::fixture_statistics(&db, 3, Uuid::new_v4(), 3, 6, "now").await?;
    let recommendations = RecommendationDbConnection::new();
    recommendations.replace([(1, [0.0; 61]), (2, [0.0; 61]), (3, [0.0; 61])]);
    let query = |operator: &str, value: &str| {
        vec![
            ("lib::all".into(), "true".into()),
            (operator.into(), value.into()),
        ]
    };
    let ids =
        |rows: Vec<media_files::Model>| rows.into_iter().map(|row| row.id).collect::<Vec<_>>();
    assert_eq!(
        ids(
            query_mix_media_files(&db, &recommendations, query("filter::liked", "true"), 0, 10)
                .await?
        ),
        [3]
    );
    assert_eq!(
        ids(query_mix_media_files(
            &db,
            &recommendations,
            query("filter::liked", "false"),
            0,
            10
        )
        .await?),
        [1, 2]
    );
    assert_eq!(
        ids(
            query_mix_media_files(&db, &recommendations, query("sort::skipped", "true"), 0, 10)
                .await?
        ),
        [1, 3, 2]
    );
    assert_eq!(
        ids(query_mix_media_files(
            &db,
            &recommendations,
            query("sort::playedthrough", "true"),
            0,
            10
        )
        .await?),
        [1, 2, 3]
    );
    let recommended = query_mix_media_files(
        &db,
        &recommendations,
        vec![
            ("lib::all".into(), "true".into()),
            ("sort::skipped".into(), "true".into()),
            ("sort::playedthrough".into(), "true".into()),
            ("pipe::recommend".into(), "-1".into()),
        ],
        0,
        10,
    )
    .await?;
    assert_eq!(ids(recommended), [1, 2, 3]);
    Ok(())
}
