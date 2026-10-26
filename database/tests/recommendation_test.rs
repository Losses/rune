use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use rand::{Rng, SeedableRng, rngs::StdRng};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection};

use ::database::{
    actions::recommendation::{
        get_recommendation_by_file_id, get_recommendation_by_parameter,
        get_recommendation_by_percentile, sync_recommendation,
    },
    connection::{
        RecommendationDbConnection, connect_fake_recommendation_db, connect_recommendation_db,
        initialize_db,
    },
};
use ::fsio::FsIo;

async fn setup_db() -> Result<DatabaseConnection> {
    let db = Database::connect("sqlite::memory:").await?;
    initialize_db(&db, "recommendation-test").await?;
    Ok(db)
}

async fn insert_file(db: &DatabaseConnection, id: i32) -> Result<()> {
    db.execute_unprepared(&format!(
        "INSERT INTO media_files (id, file_name, directory, extension, file_hash, last_modified, \
         sample_rate, duration, hlc_uuid, created_at_hlc_ts, created_at_hlc_ver, created_at_hlc_nid, \
         updated_at_hlc_ts, updated_at_hlc_ver, updated_at_hlc_nid) \
         VALUES ({id}, 'f{id}.mp3', '.', 'mp3', 'hash{id}', '0', 44100, 1.0, 'file-{id}', '', 0, '', '', 0, '')"
    ))
    .await?;
    Ok(())
}

/// A track whose analysis has `rms = x` and `zcr = y`, everything else NULL (0).
async fn insert_analysis(db: &DatabaseConnection, file_id: i32, x: f64, y: f64) -> Result<()> {
    db.execute_unprepared(&format!(
        "INSERT INTO media_analysis (file_id, rms, zcr, hlc_uuid, created_at_hlc_ts, \
         created_at_hlc_ver, created_at_hlc_nid, updated_at_hlc_ts, updated_at_hlc_ver, updated_at_hlc_nid) \
         VALUES ({file_id}, {x}, {y}, 'analysis-{file_id}', '', 0, '', '', 0, '')"
    ))
    .await?;
    Ok(())
}

fn point(x: f32, y: f32) -> [f32; 61] {
    let mut v = [0f32; 61];
    v[0] = x;
    v[1] = y;
    v
}

#[tokio::test]
async fn sync_reads_analysis_rows_and_queries_them() -> Result<()> {
    let db = setup_db().await?;
    let recommend_db = RecommendationDbConnection::new();

    // No analysis data yet: empty results, no errors.
    sync_recommendation(&db, &recommend_db).await?;
    assert!(get_recommendation_by_parameter(&recommend_db, point(0.0, 0.0), 5)?.is_empty());
    assert!(get_recommendation_by_file_id(&recommend_db, 1, 5).is_err());

    for (id, x, y) in [(1, 0.0, 0.0), (2, 3.0, 4.0), (3, 1.0, 0.0), (4, 10.0, 0.0)] {
        insert_file(&db, id).await?;
        insert_analysis(&db, id, x, y).await?;
    }
    // Not synced yet: the store still shows the old state.
    assert!(get_recommendation_by_parameter(&recommend_db, point(0.0, 0.0), 5)?.is_empty());

    sync_recommendation(&db, &recommend_db).await?;
    let result = get_recommendation_by_parameter(&recommend_db, point(0.0, 0.0), 3)?;
    assert_eq!(result, vec![(1, 0.0), (3, 1.0), (2, 5.0)]);

    // By file id includes the item itself.
    let result = get_recommendation_by_file_id(&recommend_db, 2, 2)?;
    assert_eq!(result[0], (2, 0.0));
    assert_eq!(result[1].0, 3);

    // Rows removed from the database disappear after the next sync.
    db.execute_unprepared("DELETE FROM media_analysis WHERE file_id = 3")
        .await?;
    sync_recommendation(&db, &recommend_db).await?;
    let ids: Vec<u32> = get_recommendation_by_parameter(&recommend_db, point(0.0, 0.0), 10)?
        .into_iter()
        .map(|r| r.0)
        .collect();
    assert_eq!(ids, vec![1, 2, 4]);
    Ok(())
}

#[tokio::test]
async fn percentile_recommendation_uses_the_stored_vectors() -> Result<()> {
    let db = setup_db().await?;
    let recommend_db = RecommendationDbConnection::new();
    for id in 1..=10 {
        insert_file(&db, id).await?;
        insert_analysis(&db, id, id as f64, 0.0).await?;
    }
    sync_recommendation(&db, &recommend_db).await?;

    let low = get_recommendation_by_percentile(&db, &recommend_db, 2, 0).await?;
    let high = get_recommendation_by_percentile(&db, &recommend_db, 2, 1).await?;
    // 10 files / 2 groups = 5 recommendations per group.
    assert_eq!(low.len(), 5);
    assert_eq!(high.len(), 5);
    assert!(low.iter().map(|r| r.0).sum::<u32>() < high.iter().map(|r| r.0).sum::<u32>());
    assert!(low.windows(2).all(|w| w[0].1 <= w[1].1));
    Ok(())
}

#[tokio::test]
async fn connect_loads_vectors_and_deletes_the_legacy_file() -> Result<()> {
    let db = setup_db().await?;
    insert_file(&db, 1).await?;
    insert_analysis(&db, 1, 2.0, 0.0).await?;

    let dir = tempfile::tempdir()?;
    let lib_path = dir.path().to_str().unwrap();
    let legacy = dir.path().join(".rune").join(".analysis.db");
    std::fs::create_dir_all(legacy.parent().unwrap())?;
    std::fs::write(&legacy, b"lmdb")?;

    let fsio = FsIo::new();
    let recommend_db = connect_recommendation_db(&fsio, lib_path, None, &db).await?;
    assert!(!Path::new(&legacy).exists());
    assert_eq!(
        get_recommendation_by_parameter(&recommend_db, point(0.0, 0.0), 5)?,
        vec![(1, 2.0)]
    );

    // Nothing to delete the second time: still fine.
    connect_recommendation_db(&fsio, lib_path, None, &db).await?;
    Ok(())
}

#[tokio::test]
async fn connect_does_not_fail_when_the_legacy_file_cannot_be_deleted() -> Result<()> {
    let db = setup_db().await?;
    let dir = tempfile::tempdir()?;
    let lib_path = dir.path().to_str().unwrap();
    // A directory named like the legacy file cannot be removed with remove_file.
    let legacy = dir.path().join(".rune").join(".analysis.db");
    std::fs::create_dir_all(&legacy)?;

    let recommend_db = connect_recommendation_db(&FsIo::new(), lib_path, None, &db).await?;
    assert!(get_recommendation_by_parameter(&recommend_db, point(0.0, 0.0), 5)?.is_empty());
    Ok(())
}

#[test]
fn fake_recommendation_db_is_empty() -> Result<()> {
    let recommend_db = connect_fake_recommendation_db()?;
    assert!(get_recommendation_by_parameter(&recommend_db, point(0.0, 0.0), 5)?.is_empty());
    Ok(())
}

/// Run with `cargo test -p database --release --test recommendation_test -- --ignored --nocapture`.
#[test]
#[ignore]
fn perf_query_over_100k_vectors() {
    let mut rng = StdRng::seed_from_u64(1);
    let recommend_db = RecommendationDbConnection::new();
    recommend_db.replace((1..=100_000u32).map(|id| {
        let mut v = [0f32; 61];
        v.iter_mut().for_each(|x| *x = rng.gen_range(-1.0..1.0));
        (id, v)
    }));

    let mut times: Vec<f64> = (0..200)
        .map(|_| {
            let mut q = [0f32; 61];
            q.iter_mut().for_each(|x| *x = rng.gen_range(-1.0..1.0));
            let start = Instant::now();
            let result = get_recommendation_by_parameter(&recommend_db, q, 30).unwrap();
            assert_eq!(result.len(), 30);
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(|a, b| a.total_cmp(b));
    println!(
        "100000 vectors, top 30: median {:.2} ms, min {:.2} ms, max {:.2} ms",
        times[times.len() / 2],
        times[0],
        times[times.len() - 1]
    );
}
