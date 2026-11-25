use std::f64::consts::PI;
use std::path::Path;
use std::sync::Arc;

use analysis::utils::computing_device::ComputingDevice;
use anyhow::Result;
use database::actions::analysis::{
    AggregatedAnalysisResult, analysis_audio_library, get_analyze_count, if_analyze_exists,
};
use database::actions::recommendation::{get_recommendation_by_file_id, sync_recommendation};
use database::connection::RecommendationDbConnection;
use database::entities::{media_analysis, media_files};
use database::test_fixtures::fixtures;
use fsio::FsIo;
use migration::{Migrator, MigratorTrait};
use rust_decimal::prelude::ToPrimitive;
use sea_orm::{ConnectionTrait, Database, EntityTrait};
use uuid::Uuid;

fn write_tone(path: &Path, hz: f64) -> Result<()> {
    let rate = 44100u32;
    let frames = 44100u32;
    let size = frames * 2;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + size).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(rate.to_le_bytes());
    wav.extend((rate * 2).to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(size.to_le_bytes());
    for i in 0..frames {
        let x = (16000.0 * (2.0 * PI * hz * i as f64 / rate as f64).sin()) as i16;
        wav.extend(x.to_le_bytes());
    }
    std::fs::write(path, wav)?;
    Ok(())
}

#[tokio::test]
async fn changed_audio_is_reanalyzed_and_old_features_are_not_recommended() -> Result<()> {
    let actor = Uuid::new_v4();
    let db = fixtures::memory_database(actor).await?;
    let library = fixtures::open_library(db.clone(), actor, None).await?;
    let store = library.record_store();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tone.wav");
    let fs = FsIo::new();

    write_tone(&path, 440.0)?;
    assert_eq!(
        fixtures::scan_library(&fs, &db, &actor.to_string(), dir.path(), &store).await?,
        1
    );
    assert_eq!(
        analysis_audio_library(
            Arc::new(fs.clone()),
            &db,
            dir.path(),
            1,
            ComputingDevice::Cpu,
            |_, _| {},
            None,
        )
        .await?,
        1
    );

    let before = media_files::Entity::find().one(&db).await?.unwrap();
    let old = media_analysis::Entity::find().one(&db).await?.unwrap();
    assert!(if_analyze_exists(&db, before.id).await?);

    // Keep the same store across the scan, as a running library does.
    let recommendations = RecommendationDbConnection::new();
    sync_recommendation(&db, &recommendations).await?;
    assert!(get_recommendation_by_file_id(&recommendations, before.id, 1).is_ok());

    // Replace the audio at the same path. The scan keeps the track identity
    // but observes a different content hash.
    write_tone(&path, 2200.0)?;
    let changed = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    std::fs::File::options()
        .write(true)
        .open(&path)?
        .set_times(std::fs::FileTimes::new().set_modified(changed))?;
    assert_eq!(
        fixtures::scan_library(&fs, &db, &actor.to_string(), dir.path(), &store).await?,
        1
    );

    let after = media_files::Entity::find().one(&db).await?.unwrap();
    assert_eq!(before.id, after.id);
    assert_eq!(before.source_uuid, after.source_uuid);
    assert_ne!(before.content_hash, after.content_hash);

    // The old analysis belongs to the old bytes: it is not current, and its
    // features must not stay in a store that was loaded before the scan.
    assert!(
        !if_analyze_exists(&db, after.id).await?,
        "changed audio must not keep its old analysis"
    );
    sync_recommendation(&db, &recommendations).await?;
    assert!(
        get_recommendation_by_file_id(&recommendations, after.id, 1).is_err(),
        "old features must not be recommendation-eligible"
    );
    assert!(
        recommendations
            .vectors()
            .iter()
            .all(|(id, _)| *id != after.id as u32),
        "the reloaded store must not keep the changed track's old features"
    );

    // The next analysis run processes the new bytes and stores their features.
    assert_eq!(
        analysis_audio_library(
            Arc::new(fs.clone()),
            &db,
            dir.path(),
            1,
            ComputingDevice::Cpu,
            |_, _| {},
            None,
        )
        .await?,
        1,
        "changed audio must be analyzed again"
    );
    let new = media_analysis::Entity::find().one(&db).await?.unwrap();
    assert_eq!(new.file_id, after.id);
    assert_ne!(new.source_uuid, old.source_uuid);
    assert!(if_analyze_exists(&db, after.id).await?);

    let fresh = analysis::analysis::analyze_audio(
        &fs,
        path.to_str().unwrap(),
        1024,
        512,
        ComputingDevice::Cpu,
        None,
    )?
    .unwrap();
    let fresh = analysis::analysis::normalize_analysis_result(&fresh);
    let old_centroid = old.spectral_centroid.unwrap().to_f32().unwrap();
    let new_centroid = new.spectral_centroid.unwrap().to_f32().unwrap();
    assert!(
        (old_centroid - fresh.spectral_centroid).abs() > 0.2,
        "the old analysis must differ from the new audio"
    );
    assert!(
        (new_centroid - fresh.spectral_centroid).abs() < 0.01,
        "the stored analysis must describe the new audio"
    );

    sync_recommendation(&db, &recommendations).await?;
    assert!(get_recommendation_by_file_id(&recommendations, after.id, 1).is_ok());
    let vectors = recommendations.vectors();
    let loaded = vectors
        .iter()
        .find(|(id, _)| *id == after.id as u32)
        .expect("the new analysis must be loaded for recommendations")
        .1;
    let expected: [f32; 61] = AggregatedAnalysisResult::from(new).into();
    assert_eq!(loaded, expected);

    // An unchanged file is not analyzed again.
    assert_eq!(
        analysis_audio_library(
            Arc::new(fs),
            &db,
            dir.path(),
            1,
            ComputingDevice::Cpu,
            |_, _| {},
            None,
        )
        .await?,
        0
    );
    Ok(())
}

#[tokio::test]
async fn migration_drops_legacy_analysis_without_a_current_source_identity() -> Result<()> {
    let actor = Uuid::new_v4();
    let _ = migration::initialize_node_id(actor.to_string());
    let db = Database::connect("sqlite::memory:").await?;
    let steps = Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m20261112_000037_rename_source_uuid")
        .expect("migration 37 must be registered") as u32
        + 1;
    Migrator::up(&db, Some(steps)).await?;

    let matched_hash = "a".repeat(64);
    let matched_uuid = Uuid::new_v5(&Uuid::NAMESPACE_OID, b"RUNE_ANALYSIS::v2::same").to_string();
    // Row 1 is a file without content identity whose legacy analysis source
    // UUID does not match. Row 2 matches its file hash, but a legacy row still
    // cannot prove which bytes it analyzed.
    for (id, file_hash, content_hash, content_size, analysis_uuid) in [
        (1, "changed-hash", None, None, "mismatched"),
        (
            2,
            "same",
            Some(matched_hash.as_str()),
            Some(100),
            matched_uuid.as_str(),
        ),
    ] {
        let file_uuid = Uuid::new_v4();
        let content_hash = content_hash
            .map(|hash| format!("'{hash}'"))
            .unwrap_or_else(|| "NULL".into());
        let content_size = content_size
            .map(|size| size.to_string())
            .unwrap_or_else(|| "NULL".into());
        db.execute_unprepared(&format!(
            "INSERT INTO media_files \
             (id,file_name,directory,extension,file_hash,last_modified,sample_rate,duration,\
              source_uuid,content_hash,content_size,local_available) \
             VALUES({id},'f{id}.ogg','','ogg','{file_hash}','2020',44100,1,'{file_uuid}',\
              {content_hash},{content_size},1)"
        ))
        .await?;
        db.execute_unprepared(&format!(
            "INSERT INTO media_analysis(id,file_id,analysis_version,source_uuid) \
             VALUES({id},{id},2,'{analysis_uuid}')"
        ))
        .await?;
    }

    Migrator::up(&db, None).await?;

    assert!(
        !if_analyze_exists(&db, 1).await?,
        "a mismatched legacy analysis must not remain current"
    );
    assert!(!if_analyze_exists(&db, 2).await?);
    assert!(
        media_analysis::Entity::find().all(&db).await?.is_empty(),
        "legacy analyses cannot prove their source bytes and must be dropped"
    );
    assert_eq!(get_analyze_count(&db).await?, 0);
    let recommendations = RecommendationDbConnection::new();
    sync_recommendation(&db, &recommendations).await?;
    assert!(
        recommendations.vectors().is_empty(),
        "legacy features must not be recommendation-eligible"
    );
    Ok(())
}
