use futures::stream::{self, StreamExt};
use log::{error, info};
use sea_orm::entity::prelude::*;
use sea_orm::ActiveValue;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::path::Path;
use tokio::task;

use analysis::analysis::{analyze_audio, normalize_analysis_result, NormalizedAnalysisResult};

use crate::entities::{media_analysis, media_files};

/// Analyze the audio library by reading existing files, checking if they have been analyzed,
/// and performing audio analysis if not. The function uses cursor pagination to process files
/// in batches for memory efficiency and utilizes multi-core parallelism for faster processing.
/// The analysis results are normalized before being stored in the database.
///
/// # Arguments
/// * `db` - A reference to the database connection.
/// * `root_path` - The root path for the audio files.
/// * `batch_size` - The number of files to process in each batch.
///
/// # Returns
/// * `Result<(), sea_orm::DbErr>` - A result indicating success or failure.
pub async fn analysis_audio_library(
    db: &DatabaseConnection,
    root_path: &Path,
    batch_size: usize,
) -> Result<(), sea_orm::DbErr> {
    let mut cursor = media_files::Entity::find().cursor_by(media_files::Column::Id);

    info!(
        "Starting audio library analysis with batch size: {}",
        batch_size
    );

    let (tx, rx) = async_channel::bounded(batch_size);

    // Producer task: fetch batches of files and send them to the consumer
    let producer = async {
        loop {
            // Fetch the next batch of files
            let files: Vec<media_files::Model> =
                cursor.first(batch_size.try_into().unwrap()).all(db).await?;

            if files.is_empty() {
                info!("No more files to process. Exiting loop.");
                break;
            }

            for file in &files {
                tx.send(file.clone()).await.unwrap();
            }

            // Move the cursor to the next batch
            if let Some(last_file) = files.last() {
                info!("Moving cursor after file ID: {}", last_file.id);
                cursor.after(last_file.id);
            } else {
                break;
            }
        }

        drop(tx); // Close the channel to signal consumers to stop
        Ok::<(), sea_orm::DbErr>(())
    };

    // Consumer task: process files as they are received
    let consumer = async {
        let mut tasks = Vec::new();

        while let Ok(file) = rx.recv().await {
            let db = db.clone();
            let root_path = root_path.to_path_buf();
            let file_id = file.id;

            let task = task::spawn(async move {
                info!("Processing file with ID: {}", file_id);
                process_file_if_needed(&db, &file, &root_path).await
            });

            tasks.push(task);

            // Process tasks in parallel up to the batch size
            if tasks.len() >= batch_size {
                let results: Vec<_> = stream::iter(tasks).buffer_unordered(batch_size).collect().await;
                tasks = Vec::new();

                for result in results {
                    if let Err(e) = result {
                        error!("Error processing file: {:?}", e);
                    }
                }
            }
        }

        // Process remaining tasks
        if !tasks.is_empty() {
            let results: Vec<_> = stream::iter(tasks).buffer_unordered(batch_size).collect().await;
            for result in results {
                if let Err(e) = result {
                    error!("Error processing file: {:?}", e);
                }
            }
        }

        Ok::<(), sea_orm::DbErr>(())
    };

    // Run producer and consumer concurrently
    let (producer_result, consumer_result) = futures::join!(producer, consumer);

    producer_result?;
    consumer_result?;

    info!("Audio library analysis completed.");
    Ok(())
}

/// Process a file if it has not been analyzed yet. Perform audio analysis and store the results
/// in the database.
///
/// # Arguments
/// * `db` - A reference to the database connection.
/// * `file` - A reference to the file model.
/// * `root_path` - The root path for the audio files.
async fn process_file_if_needed(
    db: &DatabaseConnection,
    file: &media_files::Model,
    root_path: &Path,
) {
    // Check if the file has already been analyzed
    let existing_analysis = media_analysis::Entity::find()
        .filter(media_analysis::Column::FileId.eq(file.id))
        .one(db)
        .await
        .unwrap();

    if existing_analysis.is_some() {
        // File has already been analyzed, skip it
        return;
    }

    // Construct the full path to the file
    let file_path = root_path.join(&file.file_name);

    // Perform audio analysis
    let analysis_result = analyze_audio(
        file_path.to_str().unwrap(),
        1024, // Example window size
        512,  // Example overlap size
    );

    // Normalize the analysis result
    let normalized_result = normalize_analysis_result(analysis_result);

    // Insert the analysis result into the database
    insert_analysis_result(db, file.id, normalized_result).await;
}

/// Insert the normalized analysis result into the database.
///
/// # Arguments
/// * `db` - A reference to the database connection.
/// * `file_id` - The ID of the file being analyzed.
/// * `result` - The normalized analysis result.
async fn insert_analysis_result(
    db: &DatabaseConnection,
    file_id: i32,
    result: NormalizedAnalysisResult,
) {
    let new_analysis = media_analysis::ActiveModel {
        file_id: ActiveValue::Set(file_id),
        sample_rate: ActiveValue::Set(result.stat.sample_rate as i32),
        duration: ActiveValue::Set(result.stat.duration),
        spectral_centroid: ActiveValue::Set(Some(result.spectral_centroid as f64)),
        spectral_flatness: ActiveValue::Set(Some(result.spectral_flatness as f64)),
        spectral_slope: ActiveValue::Set(Some(result.spectral_slope as f64)),
        spectral_rolloff: ActiveValue::Set(Some(result.spectral_rolloff as f64)),
        spectral_spread: ActiveValue::Set(Some(result.spectral_spread as f64)),
        spectral_skewness: ActiveValue::Set(Some(result.spectral_skewness as f64)),
        spectral_kurtosis: ActiveValue::Set(Some(result.spectral_kurtosis as f64)),
        chroma0: ActiveValue::Set(Some(result.chromagram[0] as f64)),
        chroma1: ActiveValue::Set(Some(result.chromagram[1] as f64)),
        chroma2: ActiveValue::Set(Some(result.chromagram[2] as f64)),
        chroma3: ActiveValue::Set(Some(result.chromagram[3] as f64)),
        chroma4: ActiveValue::Set(Some(result.chromagram[4] as f64)),
        chroma5: ActiveValue::Set(Some(result.chromagram[5] as f64)),
        chroma6: ActiveValue::Set(Some(result.chromagram[6] as f64)),
        chroma7: ActiveValue::Set(Some(result.chromagram[7] as f64)),
        chroma8: ActiveValue::Set(Some(result.chromagram[8] as f64)),
        chroma9: ActiveValue::Set(Some(result.chromagram[9] as f64)),
        chroma10: ActiveValue::Set(Some(result.chromagram[10] as f64)),
        chroma11: ActiveValue::Set(Some(result.chromagram[11] as f64)),
        ..Default::default()
    };

    media_analysis::Entity::insert(new_analysis)
        .exec(db)
        .await
        .unwrap();
}

pub async fn get_duration_by_file_id(
    db: &DatabaseConnection,
    file_id: i32,
) -> Result<f64, sea_orm::DbErr> {
    let analysis_entry: Option<media_analysis::Model> = media_analysis::Entity::find()
        .filter(media_analysis::Column::FileId.eq(file_id))
        .one(db)
        .await?;

    if let Some(entry) = analysis_entry {
        Ok(entry.duration)
    } else {
        Err(sea_orm::DbErr::RecordNotFound(
            "Analysis record not found".to_string(),
        ))
    }
}
