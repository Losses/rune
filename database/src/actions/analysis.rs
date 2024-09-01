use futures::stream::{self, StreamExt};
use log::{error, info};
use sea_orm::entity::prelude::*;
use sea_orm::ActiveValue;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::path::Path;
use tokio::task;
use tokio_util::sync::CancellationToken;

use analysis::analysis::{analyze_audio, normalize_analysis_result, NormalizedAnalysisResult};

use crate::entities::{media_analysis, media_files};

pub fn empty_progress_callback(_processed: usize, _total: usize) {}

/// Analyze the audio library by reading existing files, checking if they have been analyzed,
/// and performing audio analysis if not. The function uses cursor pagination to process files
/// in batches for memory efficiency and utilizes multi-core parallelism for faster processing.
/// The analysis results are normalized before being stored in the database.
///
/// # Arguments
/// * `db` - A reference to the database connection.
/// * `root_path` - The root path for the audio files.
/// * `batch_size` - The number of files to process in each batch.
/// * `progress_callback` - A callback function to report progress.
/// * `cancel_token` - An optional cancellation token to support task cancellation.
///
/// # Returns
/// * `Result<(), sea_orm::DbErr>` - A result indicating success or failure.
pub async fn analysis_audio_library<F>(
    db: &DatabaseConnection,
    root_path: &Path,
    batch_size: usize,
    progress_callback: F,
    cancel_token: Option<CancellationToken>,
) -> Result<usize, sea_orm::DbErr>
where
    F: Fn(usize, usize) + Send + Sync,
{
    let mut cursor = media_files::Entity::find().cursor_by(media_files::Column::Id);

    info!(
        "Starting audio library analysis with batch size: {}",
        batch_size
    );

    // Calculate the total number of tasks
    let total_tasks = media_files::Entity::find().count(db).await? as usize;

    let (tx, rx) = async_channel::bounded(batch_size);
    let mut total_processed = 0;

    // Producer task: fetch batches of files and send them to the consumer
    let producer = async {
        loop {
            // Check for cancellation
            if let Some(ref token) = cancel_token {
                if token.is_cancelled() {
                    info!("Cancellation requested. Exiting producer loop.");
                    break;
                }
            }

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
            // Check for cancellation
            if let Some(ref token) = cancel_token {
                if token.is_cancelled() {
                    info!("Cancellation requested. Exiting consumer loop.");
                    break;
                }
            }

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
                let task_count = tasks.len();
                let results: Vec<_> = stream::iter(tasks)
                    .buffer_unordered(batch_size)
                    .collect()
                    .await;
                tasks = Vec::new();

                for result in results {
                    if let Err(e) = result {
                        error!("Error processing file: {:?}", e);
                    }
                }

                // Update progress
                total_processed += task_count;
                progress_callback(total_processed, total_tasks);
            }
        }

        // Process remaining tasks
        if !tasks.is_empty() {
            let task_count = tasks.len();
            let results: Vec<_> = stream::iter(tasks)
                .buffer_unordered(batch_size)
                .collect()
                .await;
            for result in results {
                if let Err(e) = result {
                    error!("Error processing file: {:?}", e);
                }
            }

            // Update progress for remaining tasks
            total_processed += task_count;
            progress_callback(total_processed, total_tasks);
        }

        Ok::<(), sea_orm::DbErr>(())
    };

    // Run producer and consumer concurrently
    let (producer_result, consumer_result) = futures::join!(producer, consumer);

    producer_result?;
    consumer_result?;

    info!("Audio library analysis completed.");
    Ok(total_tasks)
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

/// Struct to store mean values of analysis results.
#[derive(Debug)]
pub struct AggregatedAnalysisResult {
    pub spectral_centroid: f64,
    pub spectral_flatness: f64,
    pub spectral_slope: f64,
    pub spectral_rolloff: f64,
    pub spectral_spread: f64,
    pub spectral_skewness: f64,
    pub spectral_kurtosis: f64,
    pub chromagram: [f64; 12],
}

/// Macro to process individual fields by updating their sum and count.
macro_rules! process_field {
    ($sum:expr, $count:expr, $result:expr, $field:ident) => {
        if let Some(value) = $result.$field {
            $sum.$field += value;
            $count.$field += 1.0;
        }
    };
}

/// Macro to process the chromagram array fields by updating their sum and count.
macro_rules! process_chromagram {
    ($sum:expr, $count:expr, $result:expr, $index:expr, $field:expr) => {
        if let Some(value) = $field {
            $sum.chromagram[$index] += value;
            $count.chromagram[$index] += 1.0;
        }
    };
}

/// Macro to calculate the mean of individual fields.
macro_rules! calculate_mean {
    ($sum:expr, $count:expr, $field:ident) => {
        if $count.$field > 0.0 {
            $sum.$field / $count.$field
        } else {
            0.0
        }
    };
}

/// Macro to calculate the mean of chromagram array fields.
macro_rules! calculate_chromagram_mean {
    ($sum:expr, $count:expr, $index:expr) => {
        if $count.chromagram[$index] > 0.0 {
            $sum.chromagram[$index] / $count.chromagram[$index]
        } else {
            0.0
        }
    };
}

/// Computes the centralized analysis result from the database.
///
/// This function retrieves analysis results based on specified file IDs,
/// sums the parameters, and calculates averages while handling potential `None` values.
///
/// # Arguments
///
/// * `db` - A reference to the database connection.
/// * `file_ids` - A vector of file IDs to filter the analysis results.
///
/// # Returns
///
/// * `AnalysisResultMean` - A struct containing the mean values of the analysis results.
///
/// # Example
///
/// ```rust
/// let db: DatabaseConnection = ...;
/// let file_ids = vec![1, 2, 3];
/// let result = get_centralized_analysis_result(&db, file_ids).await;
/// println!("{:?}", result);
/// ```
pub async fn get_centralized_analysis_result(
    db: &DatabaseConnection,
    file_ids: Vec<i32>,
) -> AggregatedAnalysisResult {
    let analysis_results = media_analysis::Entity::find()
        .filter(media_analysis::Column::FileId.is_in(file_ids))
        .all(db)
        .await
        .unwrap();

    let mut sum = AggregatedAnalysisResult {
        spectral_centroid: 0.0,
        spectral_flatness: 0.0,
        spectral_slope: 0.0,
        spectral_rolloff: 0.0,
        spectral_spread: 0.0,
        spectral_skewness: 0.0,
        spectral_kurtosis: 0.0,
        chromagram: [0.0; 12],
    };

    let mut count = AggregatedAnalysisResult {
        spectral_centroid: 0.0,
        spectral_flatness: 0.0,
        spectral_slope: 0.0,
        spectral_rolloff: 0.0,
        spectral_spread: 0.0,
        spectral_skewness: 0.0,
        spectral_kurtosis: 0.0,
        chromagram: [0.0; 12],
    };

    for result in analysis_results {
        process_field!(sum, count, result, spectral_centroid);
        process_field!(sum, count, result, spectral_flatness);
        process_field!(sum, count, result, spectral_slope);
        process_field!(sum, count, result, spectral_rolloff);
        process_field!(sum, count, result, spectral_spread);
        process_field!(sum, count, result, spectral_skewness);
        process_field!(sum, count, result, spectral_kurtosis);

        process_chromagram!(sum, count, result, 0, result.chroma0);
        process_chromagram!(sum, count, result, 1, result.chroma1);
        process_chromagram!(sum, count, result, 2, result.chroma2);
        process_chromagram!(sum, count, result, 3, result.chroma3);
        process_chromagram!(sum, count, result, 4, result.chroma4);
        process_chromagram!(sum, count, result, 5, result.chroma5);
        process_chromagram!(sum, count, result, 6, result.chroma6);
        process_chromagram!(sum, count, result, 7, result.chroma7);
        process_chromagram!(sum, count, result, 8, result.chroma8);
        process_chromagram!(sum, count, result, 9, result.chroma9);
        process_chromagram!(sum, count, result, 10, result.chroma10);
        process_chromagram!(sum, count, result, 11, result.chroma11);
    }

    AggregatedAnalysisResult {
        spectral_centroid: calculate_mean!(sum, count, spectral_centroid),
        spectral_flatness: calculate_mean!(sum, count, spectral_flatness),
        spectral_slope: calculate_mean!(sum, count, spectral_slope),
        spectral_rolloff: calculate_mean!(sum, count, spectral_rolloff),
        spectral_spread: calculate_mean!(sum, count, spectral_spread),
        spectral_skewness: calculate_mean!(sum, count, spectral_skewness),
        spectral_kurtosis: calculate_mean!(sum, count, spectral_kurtosis),
        chromagram: [
            calculate_chromagram_mean!(sum, count, 0),
            calculate_chromagram_mean!(sum, count, 1),
            calculate_chromagram_mean!(sum, count, 2),
            calculate_chromagram_mean!(sum, count, 3),
            calculate_chromagram_mean!(sum, count, 4),
            calculate_chromagram_mean!(sum, count, 5),
            calculate_chromagram_mean!(sum, count, 6),
            calculate_chromagram_mean!(sum, count, 7),
            calculate_chromagram_mean!(sum, count, 8),
            calculate_chromagram_mean!(sum, count, 9),
            calculate_chromagram_mean!(sum, count, 10),
            calculate_chromagram_mean!(sum, count, 11),
        ],
    }
}
