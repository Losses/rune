use analysis::{CURRENT_ANALYSIS_VERSION, utils::computing_device::ComputingDevice};
use anyhow::Result;
use database::{
    actions::{
        analysis::{
            analysis_audio_library, get_analyze_count, get_centralized_analysis_result,
            if_analyze_exists,
        },
        metadata::sync_file_descriptions,
        mixes::query_mix_media_files,
        recommendation::{get_recommendation_by_file_id, sync_recommendation},
    },
    connection::RecommendationDbConnection,
    entities::media_analysis,
};
use fsio::FsIo;
use metadata::describe::describe_file;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ConnectionTrait, Database, EntityTrait, PaginatorTrait};
use std::{path::Path, sync::Arc};

#[tokio::test]
async fn legacy_rows_are_invalid_and_replaced_only_after_success() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    let actor = uuid::Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")?;
    let _ = migration::initialize_node_id(actor.to_string());
    let prior_count = Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m20261031_000030_add_analysis_version")
        .expect("analysis version migration must be registered");
    Migrator::up(&db, Some(prior_count as u32)).await?;
    db.execute_unprepared("INSERT INTO media_files(id,file_name,directory,extension,file_hash,last_modified,sample_rate,duration,hlc_uuid) VALUES(1,'startup_0.ogg','','ogg','legacy','2020',44100,1,'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')").await?;
    db.execute_unprepared(
        "INSERT INTO media_analysis (file_id,rms,hlc_uuid) VALUES (1,999,'legacy')",
    )
    .await?;
    Migrator::up(&db, None).await?;
    let fs = Arc::new(FsIo::new());
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../assets")
        .canonicalize()?;
    let store = database::sync::LibrarySyncStore::open(db.clone(), actor, None).await?;
    let node = fs.canonicalize(&root.join("startup_0.ogg"))?;
    let mut descriptions = vec![Some(describe_file(&fs, &node, &Some(root.clone()))?)];
    sync_file_descriptions(
        &fs,
        &db,
        &actor.to_string(),
        &mut descriptions,
        false,
        &store.record_store(),
    )
    .await?;
    let old = media_analysis::Entity::find().one(&db).await?.unwrap();
    assert_eq!(old.analysis_version, 1);
    assert_eq!(get_analyze_count(&db).await?, 0);
    assert!(!if_analyze_exists(&db, 1).await?);
    assert_eq!(
        get_centralized_analysis_result(&db, vec![1]).await?.rms,
        0.0
    );
    let recommendations = RecommendationDbConnection::new();
    sync_recommendation(&db, &recommendations).await?;
    assert!(get_recommendation_by_file_id(&recommendations, 1, 1).is_err());
    for (value, count) in [("true", 0), ("false", 1)] {
        assert_eq!(
            query_mix_media_files(
                &db,
                &recommendations,
                vec![
                    ("lib::all".into(), "true".into()),
                    ("filter::analyzed".into(), value.into())
                ],
                0,
                10
            )
            .await?
            .len(),
            count
        );
    }
    // Failed insertion must roll back deletion and never mark the file current.
    db.execute_unprepared("CREATE TRIGGER reject_analysis BEFORE INSERT ON media_analysis BEGIN SELECT RAISE(ABORT, 'test failure'); END").await?;
    analysis_audio_library(
        fs.clone(),
        &db,
        &root,
        1,
        ComputingDevice::Cpu,
        |_, _| {},
        None,
    )
    .await?;
    assert_eq!(media_analysis::Entity::find().one(&db).await?.unwrap(), old);
    assert_eq!(get_analyze_count(&db).await?, 0);
    db.execute_unprepared("DROP TRIGGER reject_analysis")
        .await?;
    analysis_audio_library(
        fs.clone(),
        &db,
        &root,
        1,
        ComputingDevice::Cpu,
        |_, _| {},
        None,
    )
    .await?;
    assert_eq!(media_analysis::Entity::find().count(&db).await?, 1);
    let new = media_analysis::Entity::find().one(&db).await?.unwrap();
    assert_eq!(new.analysis_version, CURRENT_ANALYSIS_VERSION);
    assert_ne!(new.hlc_uuid, old.hlc_uuid);
    assert_eq!(get_analyze_count(&db).await?, 1);
    assert!(if_analyze_exists(&db, 1).await?);
    sync_recommendation(&db, &recommendations).await?;
    assert_eq!(
        get_recommendation_by_file_id(&recommendations, 1, 1)?.len(),
        1
    );
    assert_eq!(
        analysis_audio_library(fs, &db, &root, 1, ComputingDevice::Cpu, |_, _| {}, None).await?,
        0
    );
    assert_eq!(media_analysis::Entity::find().one(&db).await?.unwrap(), new);
    // Old clients that omit the new column must still write invalid version 1.
    db.execute_unprepared(
        "INSERT INTO media_analysis (file_id,rms,hlc_uuid) VALUES (1,999,'late-legacy')",
    )
    .await?;
    assert_eq!(get_analyze_count(&db).await?, 1);
    sync_recommendation(&db, &recommendations).await?;
    assert_eq!(recommendations.vectors().len(), 1);
    Ok(())
}

#[test]
fn silence_short_clips_and_sample_rates_produce_finite_features() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for rate in [11025u32, 44100, 48000] {
        for frames in [1usize, 1023, 1024, 1025, rate as usize] {
            for silent in [false, true] {
                let bytes = (frames * 2) as u32;
                let mut wav = Vec::new();
                wav.extend(b"RIFF");
                wav.extend((36 + bytes).to_le_bytes());
                wav.extend(b"WAVEfmt ");
                wav.extend(16u32.to_le_bytes());
                wav.extend(1u16.to_le_bytes());
                wav.extend(1u16.to_le_bytes());
                wav.extend(rate.to_le_bytes());
                wav.extend((rate * 2).to_le_bytes());
                wav.extend(2u16.to_le_bytes());
                wav.extend(16u16.to_le_bytes());
                wav.extend(b"data");
                wav.extend(bytes.to_le_bytes());
                for i in 0..frames {
                    let x = if silent {
                        0
                    } else {
                        // Taper the one-second frequency fixture to exclude onset/EOF
                        // broadband transients from its centroid expectation.
                        let envelope = if frames == rate as usize {
                            (std::f64::consts::PI * i as f64 / (frames - 1) as f64)
                                .sin()
                                .powi(2)
                        } else {
                            1.0
                        };
                        (16000.0
                            * envelope
                            * (2.0 * std::f64::consts::PI * 1100.0 * (i as f64 + 0.5)
                                / rate as f64)
                                .sin()) as i16
                    };
                    wav.extend(x.to_le_bytes());
                }
                let path = dir.path().join("test.wav");
                std::fs::write(&path, wav)?;
                let result = analysis::analysis::analyze_audio(
                    &FsIo::new(),
                    path.to_str().unwrap(),
                    1024,
                    512,
                    ComputingDevice::Cpu,
                    None,
                )?
                .unwrap();
                assert!(result.is_finite());
                let normalized = analysis::analysis::normalize_analysis_result(&result);
                assert!(normalized.energy.is_finite() && normalized.zcr.is_finite());
                assert_eq!(result.stat.total_samples, frames);
                assert_eq!(result.stat.sample_rate, rate);
                if silent {
                    assert_eq!(result.energy, 0.0);
                    assert_eq!(result.spectral_centroid, 0.0);
                }
                if frames == rate as usize && !silent {
                    let centroid_hz = result.spectral_centroid * 11025.0 / 1024.0;
                    assert!(
                        (centroid_hz - 1100.0).abs() < 40.0,
                        "rate={rate}, centroid={centroid_hz}"
                    );
                }
            }
        }
    }
    Ok(())
}
