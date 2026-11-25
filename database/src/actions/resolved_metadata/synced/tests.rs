use std::sync::{Arc, Mutex};

use anyhow::Result;
use sea_orm::{ActiveModelTrait, PaginatorTrait, TransactionTrait};

use crate::sync::model::{Collection, ObjectKey, Payload, RuneModel, Track};
use crate::test_fixtures::fixtures;

use super::*;

#[tokio::test]
async fn projection_budget_covers_full_lists_and_replacing_an_edition() -> Result<()> {
    let mut db = fixtures::memory_database(Uuid::new_v4()).await?;
    let key = ObjectKey {
        collection: Collection::Tracks,
        id: Uuid::new_v4(),
    };
    RuneModel::project(
        &db,
        &key,
        Some(&Payload::Track(Track {
            extension: "flac".into(),
            content_hash: Some("a".repeat(64)),
            content_size: Some(1),
            duration: "1".to_owned().try_into().map_err(anyhow::Error::msg)?,
            sample_rate: 44100,
            cover_art_id: None,
            metadata: Vec::new(),
        })),
    )
    .await?;

    // Change the source rows first so the measured hook invalidates and moves the old edition.
    let file = media_files::Entity::find()
        .filter(media_files::Column::SourceUuid.eq(key.id.to_string()))
        .one(&db)
        .await?
        .expect("fixture file was projected");
    let file_id = file.id;
    let mut source: media_files::ActiveModel = file.into();
    source.content_hash = Set(Some("b".repeat(64)));
    source.update(&db).await?;
    let album = Uuid::new_v4().to_string();
    portable_albums::Entity::insert(portable_albums::ActiveModel {
        source_uuid: Set(album.clone()),
        name: Set("Moved album".into()),
        group: Set("M".into()),
        ..Default::default()
    })
    .exec(&db)
    .await?;
    track_album_sources::Entity::insert(track_album_sources::ActiveModel {
        track_uuid: Set(key.id.to_string()),
        album_uuid: Set(album),
        source_uuid: Set(key.id.to_string()),
        ..Default::default()
    })
    .exec(&db)
    .await?;

    // Fill the projector's source limits while exercising both bulk insert paths.
    let genre_tags = TAG_LIMIT - 1;
    let genre_count = (LIST_BYTES - 1 + 2 * genre_tags) / 3;
    let tags = (0..TAG_LIMIT).map(|position| {
        let (name, value) = if position < genre_tags {
            let count = genre_count / genre_tags + usize::from(position < genre_count % genre_tags);
            ("genre", format!("{}G", "G\\\\".repeat(count - 1)))
        } else {
            ("artist", "A".into())
        };
        source_tags::ActiveModel {
            file_id: Set(file_id),
            position: Set(position as i32),
            meta_key: Set(name.into()),
            meta_value: Set(value),
            source_uuid: Set(Uuid::new_v4().to_string()),
            ..Default::default()
        }
    });
    source_tags::Entity::insert_many(tags)
        .exec_without_returning(&db)
        .await?;

    // Count only SQL from the hook, including statements in its lifecycle helpers.
    let calls = Arc::new(Mutex::new(Vec::new()));
    let measured = Arc::clone(&calls);
    db.set_metric_callback(move |info| measured.lock().unwrap().push(info.statement.sql.clone()));
    let tx = db.begin().await?;
    crate::actions::resolved_metadata::project_synced_track(&tx, key.id).await?;
    tx.commit().await?;

    let (reads, mutations) = {
        let calls = calls.lock().unwrap();
        let mut reads = 0;
        let mut mutations = 0;
        for sql in calls.iter() {
            match sql.split_whitespace().next() {
                // The tag query consumes TAG_LIMIT rows. Scalar reads consume at most one each.
                Some("SELECT") if sql.contains("\"music_metadata_source_tags\"") => {
                    reads += TAG_LIMIT as u64;
                }
                Some("SELECT") => reads += 1,
                Some("INSERT" | "UPDATE" | "DELETE") => mutations += 1,
                other => panic!("unmeasured statement {other:?}: {sql}"),
            }
        }
        (reads, mutations)
    };
    assert!(reads <= READ_LIMIT, "{reads} reads exceeds {READ_LIMIT}");
    assert!(
        mutations <= MUTATION_LIMIT,
        "{mutations} mutations exceeds {MUTATION_LIMIT}"
    );

    // Check that the full lists were written and the old derived edition was removed.
    assert_eq!(values::Entity::find().count(&db).await?, genre_count as u64);
    assert_eq!(credits::Entity::find().count(&db).await?, 1);
    assert_eq!(albums::Entity::find().count(&db).await?, 1);
    Ok(())
}
