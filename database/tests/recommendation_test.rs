use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use rand::{Rng, SeedableRng, rngs::StdRng};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, EntityTrait, QueryOrder, Statement};

use ::database::{
    actions::analysis::AggregatedAnalysisResult,
    actions::recommendation::{
        get_recommendation_by_file_id, get_recommendation_by_parameter,
        get_recommendation_by_percentile, sync_recommendation,
    },
    connection::{
        RecommendationDbConnection, connect_fake_recommendation_db, connect_recommendation_db,
        initialize_db,
    },
    entities::media_analysis,
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
    // 10 files / 2 groups = 5 recommendations per group. Group 0 is at the
    // 1/4 percentile, rank round(0.25 * 9) = 2 (rms 3), and group 1 at 1/2,
    // rank round(0.5 * 9) = 5 (rms 6). Equal distances go by file id.
    assert_eq!(low, vec![(3, 0.0), (2, 1.0), (4, 1.0), (1, 2.0), (5, 2.0)]);
    assert_eq!(high, vec![(6, 0.0), (5, 1.0), (7, 1.0), (4, 2.0), (8, 2.0)]);
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

#[cfg(unix)]
#[tokio::test]
async fn connect_does_not_fail_when_the_legacy_file_cannot_be_deleted() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let db = setup_db().await?;
    insert_file(&db, 1).await?;
    insert_analysis(&db, 1, 2.0, 0.0).await?;
    let dir = tempfile::tempdir()?;
    let lib_path = dir.path().to_str().unwrap();
    let rune_dir = dir.path().join(".rune");
    std::fs::create_dir_all(&rune_dir)?;
    std::fs::write(rune_dir.join(".analysis.db"), b"lmdb")?;
    // A read-only directory does not allow deleting its entries.
    std::fs::set_permissions(&rune_dir, std::fs::Permissions::from_mode(0o555))?;

    let result = connect_recommendation_db(&FsIo::new(), lib_path, None, &db).await;
    std::fs::set_permissions(&rune_dir, std::fs::Permissions::from_mode(0o755))?;

    let recommend_db = result?;
    assert_eq!(
        get_recommendation_by_parameter(&recommend_db, point(0.0, 0.0), 5)?,
        vec![(1, 2.0)]
    );
    Ok(())
}

#[tokio::test]
async fn connect_deletes_the_legacy_directory_and_keeps_other_files() -> Result<()> {
    let db = setup_db().await?;
    let dir = tempfile::tempdir()?;
    let lib_path = dir.path().to_str().unwrap();
    let rune_dir = dir.path().join(".rune");
    let legacy_dir = rune_dir.join(".analysis");
    std::fs::create_dir_all(legacy_dir.join("nested"))?;
    std::fs::write(legacy_dir.join("data.mdb"), b"lmdb")?;
    std::fs::write(legacy_dir.join("nested").join("lock.mdb"), b"lock")?;
    std::fs::write(rune_dir.join(".analysis.db"), b"lmdb")?;
    for keep in [".0.db", ".android-fs.db", ".analysis.backup", "analysis"] {
        std::fs::write(rune_dir.join(keep), b"keep")?;
    }

    connect_recommendation_db(&FsIo::new(), lib_path, None, &db).await?;

    assert!(!legacy_dir.exists());
    assert!(!rune_dir.join(".analysis.db").exists());
    for keep in [".0.db", ".android-fs.db", ".analysis.backup", "analysis"] {
        assert_eq!(std::fs::read(rune_dir.join(keep))?, b"keep", "{keep}");
    }
    Ok(())
}

#[tokio::test]
async fn connect_works_in_redirected_storage() -> Result<()> {
    let db = setup_db().await?;
    insert_file(&db, 1).await?;
    insert_analysis(&db, 1, 2.0, 0.0).await?;

    let lib = tempfile::tempdir()?;
    let storage = tempfile::tempdir()?;
    let lib_path = lib.path().to_str().unwrap();
    let db_path = storage.path().to_str().unwrap();
    let uuid = "0b1c3a4e-5f60-4d7a-8b9c-0d1e2f3a4b5c";
    let rune_dir = lib.path().join(".rune");
    std::fs::create_dir_all(&rune_dir)?;
    std::fs::write(rune_dir.join(".redirect"), uuid)?;
    let db_dir = storage.path().join(uuid);
    std::fs::create_dir_all(db_dir.join(".analysis"))?;
    std::fs::write(db_dir.join(".analysis").join("data.mdb"), b"lmdb")?;
    std::fs::write(db_dir.join(".analysis.db"), b"lmdb")?;
    std::fs::write(db_dir.join(".0.db"), b"keep")?;
    // The library's own .rune directory is not touched in this mode.
    std::fs::write(rune_dir.join(".analysis.db"), b"other")?;

    let recommend_db =
        connect_recommendation_db(&FsIo::new(), lib_path, Some(db_path), &db).await?;

    assert!(!db_dir.join(".analysis").exists());
    assert!(!db_dir.join(".analysis.db").exists());
    assert_eq!(std::fs::read(db_dir.join(".0.db"))?, b"keep");
    assert!(rune_dir.join(".analysis.db").exists());
    assert_eq!(
        get_recommendation_by_parameter(&recommend_db, point(0.0, 0.0), 5)?,
        vec![(1, 2.0)]
    );
    Ok(())
}

#[tokio::test]
async fn connect_opens_with_an_empty_store_when_loading_fails() -> Result<()> {
    let db = setup_db().await?;
    db.execute_unprepared("DROP TABLE media_analysis").await?;
    let dir = tempfile::tempdir()?;
    let lib_path = dir.path().to_str().unwrap();

    let recommend_db = connect_recommendation_db(&FsIo::new(), lib_path, None, &db).await?;
    assert!(get_recommendation_by_parameter(&recommend_db, point(0.0, 0.0), 5)?.is_empty());
    Ok(())
}

#[tokio::test]
async fn sync_failure_keeps_the_previous_vectors() -> Result<()> {
    let db = setup_db().await?;
    insert_file(&db, 1).await?;
    insert_analysis(&db, 1, 2.0, 0.0).await?;
    let recommend_db = RecommendationDbConnection::new();
    sync_recommendation(&db, &recommend_db).await?;

    db.execute_unprepared("DROP TABLE media_analysis").await?;
    assert!(sync_recommendation(&db, &recommend_db).await.is_err());
    assert_eq!(
        get_recommendation_by_parameter(&recommend_db, point(0.0, 0.0), 5)?,
        vec![(1, 2.0)]
    );
    Ok(())
}

#[tokio::test]
async fn rows_with_a_negative_file_id_are_skipped() -> Result<()> {
    let db = setup_db().await?;
    for id in [1, 2] {
        insert_file(&db, id).await?;
        insert_analysis(&db, id, id as f64, 0.0).await?;
    }
    insert_file(&db, -5).await?;
    insert_analysis(&db, -5, 0.5, 0.0).await?;

    let recommend_db = RecommendationDbConnection::new();
    sync_recommendation(&db, &recommend_db).await?;
    assert_eq!(
        get_recommendation_by_parameter(&recommend_db, point(0.0, 0.0), 10)?,
        vec![(1, 1.0), (2, 2.0)]
    );

    let dir = tempfile::tempdir()?;
    let recommend_db =
        connect_recommendation_db(&FsIo::new(), dir.path().to_str().unwrap(), None, &db).await?;
    assert_eq!(recommend_db.vectors().len(), 2);
    Ok(())
}

#[tokio::test]
async fn ties_at_the_cut_off_keep_the_lowest_ids() -> Result<()> {
    let db = setup_db().await?;
    // Three equally distant tracks, inserted out of order, and one far away.
    for (id, x) in [(7, 1.0), (3, 1.0), (5, 1.0), (1, 9.0)] {
        insert_file(&db, id).await?;
        insert_analysis(&db, id, x, 0.0).await?;
    }
    let recommend_db = RecommendationDbConnection::new();
    sync_recommendation(&db, &recommend_db).await?;
    let result = get_recommendation_by_parameter(&recommend_db, point(0.0, 0.0), 2)?;
    assert_eq!(result, vec![(3, 1.0), (5, 1.0)]);
    Ok(())
}

/// The vectors read for recommendations must equal the ones built from the
/// entity models, bit for bit.
#[tokio::test]
async fn loaded_vectors_are_bit_identical_to_the_model_conversion() -> Result<()> {
    let db = setup_db().await?;
    let columns: Vec<String> = db
        .query_all(Statement::from_string(
            db.get_database_backend(),
            "PRAGMA table_info(media_analysis)",
        ))
        .await?
        .into_iter()
        .map(|r| r.try_get::<String>("", "name").unwrap())
        .filter(|c| {
            ![
                "id",
                "file_id",
                "hlc_uuid",
                "created_at_hlc_ts",
                "created_at_hlc_ver",
                "created_at_hlc_nid",
                "updated_at_hlc_ts",
                "updated_at_hlc_ver",
                "updated_at_hlc_nid",
            ]
            .contains(&c.as_str())
        })
        .collect();
    assert_eq!(columns.len(), 61);

    let mut rng = StdRng::seed_from_u64(7);
    for id in 1..=300 {
        insert_file(&db, id).await?;
        // Mixed magnitudes and many digits, some NULLs.
        let values: Vec<String> = columns
            .iter()
            .map(|_| match rng.gen_range(0..6) {
                0 => "NULL".to_string(),
                1 => format!("{}", rng.gen_range(-1.0e6f64..1.0e6)),
                2 => format!("{}", rng.gen_range(-1.0e-6f64..1.0e-6)),
                3 => format!("{}", rng.r#gen::<f64>().sin() * 1.0e-12),
                _ => format!("{}", rng.gen_range(-1.0f64..1.0)),
            })
            .collect();
        db.execute_unprepared(&format!(
            "INSERT INTO media_analysis (file_id, {}, hlc_uuid, created_at_hlc_ts, created_at_hlc_ver, \
             created_at_hlc_nid, updated_at_hlc_ts, updated_at_hlc_ver, updated_at_hlc_nid) \
             VALUES ({id}, {}, 'a{id}', '', 0, '', '', 0, '')",
            columns.join(", "),
            values.join(", ")
        ))
        .await?;
    }

    let recommend_db = RecommendationDbConnection::new();
    sync_recommendation(&db, &recommend_db).await?;
    let loaded = recommend_db.vectors();
    assert_eq!(loaded.len(), 300);

    let models = media_analysis::Entity::find()
        .order_by_asc(media_analysis::Column::FileId)
        .all(&db)
        .await?;
    for (model, (id, vector)) in models.into_iter().zip(loaded.iter()) {
        assert_eq!(model.file_id as u32, *id);
        let expected: [f32; 61] = AggregatedAnalysisResult::from(model).into();
        for (i, (a, b)) in expected.iter().zip(vector).enumerate() {
            assert_eq!(a.to_bits(), b.to_bits(), "file {id}, column {i}");
        }
    }
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

/// Run with `cargo test -p database --release --test recommendation_test -- --ignored --nocapture perf_load`.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn perf_load_100k_rows() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let url = format!("sqlite:{}?mode=rwc", dir.path().join("bench.db").display());
    let db = Database::connect(&url).await?;
    initialize_db(&db, "recommendation-bench").await?;

    let rows = 100_000;
    let columns: Vec<String> = db
        .query_all(Statement::from_string(
            db.get_database_backend(),
            "PRAGMA table_info(media_analysis)",
        ))
        .await?
        .into_iter()
        .map(|r| r.try_get::<String>("", "name").unwrap())
        .filter(|c| c.as_str() != "id" && c.as_str() != "file_id" && !c.contains("hlc"))
        .collect();
    db.execute_unprepared("BEGIN").await?;
    for id in 1..=rows {
        insert_file(&db, id).await?;
        let values: Vec<String> = (0..columns.len())
            .map(|i| format!("{}", (id as f64 * 0.37 + i as f64).sin()))
            .collect();
        db.execute_unprepared(&format!(
            "INSERT INTO media_analysis (file_id, {}, hlc_uuid, created_at_hlc_ts, created_at_hlc_ver, \
             created_at_hlc_nid, updated_at_hlc_ts, updated_at_hlc_ver, updated_at_hlc_nid) \
             VALUES ({id}, {}, 'a{id}', '', 0, '', '', 0, '')",
            columns.join(", "),
            values.join(", ")
        ))
        .await?;
    }
    db.execute_unprepared("COMMIT").await?;

    let recommend_db = RecommendationDbConnection::new();
    let mut times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        sync_recommendation(&db, &recommend_db).await?;
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    assert_eq!(recommend_db.vectors().len(), rows as usize);
    times.sort_by(|a, b| a.total_cmp(b));
    println!(
        "load of {rows} rows: median {:.0} ms, min {:.0} ms, max {:.0} ms",
        times[times.len() / 2],
        times[0],
        times[times.len() - 1]
    );
    Ok(())
}
