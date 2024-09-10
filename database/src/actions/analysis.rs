use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use log::{error, info};
use tokio::task;
use tokio::sync::Semaphore;
use sea_orm::entity::prelude::*;
use sea_orm::FromQueryResult;
use sea_orm::QuerySelect;
use sea_orm::{ActiveValue, TransactionTrait};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use tokio_util::sync::CancellationToken;

use analysis::analysis::{analyze_audio, normalize_analysis_result, NormalizedAnalysisResult};

use crate::entities::{media_analysis, media_files};

use super::utils::DatabaseExecutor;

pub fn empty_progress_callback(_processed: usize, _total: usize) {}

#[derive(Debug, FromQueryResult)]
struct FileIdResult {
    file_id: i32, // or whatever the type of FileId is
}

pub async fn analysis_audio_library<F>(
    main_db: &DatabaseConnection,
    lib_path: &Path,
    batch_size: usize,
    progress_callback: F,
    cancel_token: Option<CancellationToken>,
) -> Result<usize>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    info!(
        "Starting audio library analysis with batch size: {}",
        batch_size
    );

    let total_tasks = media_files::Entity::find().count(main_db).await? as usize;

    let existed_tasks: Vec<i32> = media_analysis::Entity::find()
        .select_only()
        .column(media_analysis::Column::FileId)
        .into_model::<FileIdResult>()
        .all(main_db)
        .await
        .unwrap()
        .into_iter()
        .map(|x| x.file_id)
        .collect();

    info!("Media files already analysed: {}", existed_tasks.len());

    let mut cursor = media_files::Entity::find()
        .filter(media_files::Column::Id.is_not_in(existed_tasks.clone()))
        .cursor_by(media_files::Column::Id);

    let mut total_processed = existed_tasks.len();
    let lib_path = Arc::new(lib_path.to_path_buf());
    let semaphore = Arc::new(Semaphore::new(4)); // Adjust the concurrency level as needed

    loop {
        // Fetch the next batch of files
        let files: Vec<media_files::Model> = cursor
            .first(batch_size.try_into().unwrap())
            .all(main_db)
            .await?;

        if files.is_empty() {
            break;
        }

        // Check for cancellation
        if let Some(ref token) = cancel_token {
            if token.is_cancelled() {
                info!("Cancellation requested. Exiting loop.");
                break;
            }
        }

        info!("Starting a new batch: {} tasks", files.len());

        // Parallel processing using Tokio tasks
        let mut tasks = Vec::with_capacity(files.len());

        for file in &files {
            let lib_path = Arc::clone(&lib_path);
            let semaphore = Arc::clone(&semaphore);
            let permit = semaphore.acquire_owned().await.unwrap();
            let file = file.clone();

            let handle = task::spawn(async move {
                let _permit = permit; // Ensure the permit is held for the duration of the task
                let result = analysis_file(&file, &lib_path).await;
                info!("Analysed: {}", file.file_name);
                Ok::<_, sea_orm::DbErr>((file.id, Some(result)))
            });

            tasks.push(handle);
        }

        // Await all the futures
        let analysis_results: Vec<_> = futures::future::join_all(tasks).await;

        // Start a transaction
        let txn = main_db.begin().await?;

        for result in analysis_results {
            match result {
                Ok(Ok((file_id, Some(normalized_result)))) => {
                    insert_analysis_result(&txn, file_id, normalized_result?).await?;
                    total_processed += 1;
                }
                Ok(Ok((_, None))) => {} // File was already processed
                Ok(Err(e)) => {
                    error!("Error processing file: {:?}", e);
                }
                Err(e) => {
                    error!("Task join error: {:?}", e);
                }
            }
        }

        // Commit the transaction
        txn.commit().await?;

        // Update progress
        progress_callback(total_processed, total_tasks);

        // Move the cursor to the next batch
        if let Some(last_file) = files.last() {
            info!("Moving cursor after file ID: {}", last_file.id);
            cursor.after(last_file.id);
        }
    }

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
async fn analysis_file(
    file: &media_files::Model,
    lib_path: &Path,
) -> Result<NormalizedAnalysisResult> {
    // Construct the full path to the file
    let file_path = lib_path.join(&file.directory).join(&file.file_name);

    // Perform audio analysis
    let analysis_result = analyze_audio(
        file_path.to_str().unwrap(),
        1024, // Example window size
        512,  // Example overlap size
    );

    // Normalize the analysis result
    Ok(normalize_analysis_result(&analysis_result?))
}

/// Insert the normalized analysis result into the database.
///
/// # Arguments
/// * `db` - A reference to the database connection.
/// * `file_id` - The ID of the file being analyzed.
/// * `result` - The normalized analysis result.
async fn insert_analysis_result<E>(
    db: &E,
    file_id: i32,
    result: NormalizedAnalysisResult,
) -> Result<(), sea_orm::DbErr>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let new_analysis = media_analysis::ActiveModel {
        file_id: ActiveValue::Set(file_id),
        rms: ActiveValue::Set(Some(result.raw.rms as f64)),
        zcr: ActiveValue::Set(Some(result.zcr as f64)),
        energy: ActiveValue::Set(Some(result.energy as f64)),
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
        perceptual_spread: ActiveValue::Set(Some(result.raw.perceptual_spread as f64)),
        perceptual_sharpness: ActiveValue::Set(Some(result.raw.perceptual_sharpness as f64)),
        perceptual_loudness0: ActiveValue::Set(Some(result.raw.perceptual_loudness[0] as f64)),
        perceptual_loudness1: ActiveValue::Set(Some(result.raw.perceptual_loudness[1] as f64)),
        perceptual_loudness2: ActiveValue::Set(Some(result.raw.perceptual_loudness[2] as f64)),
        perceptual_loudness3: ActiveValue::Set(Some(result.raw.perceptual_loudness[3] as f64)),
        perceptual_loudness4: ActiveValue::Set(Some(result.raw.perceptual_loudness[4] as f64)),
        perceptual_loudness5: ActiveValue::Set(Some(result.raw.perceptual_loudness[5] as f64)),
        perceptual_loudness6: ActiveValue::Set(Some(result.raw.perceptual_loudness[6] as f64)),
        perceptual_loudness7: ActiveValue::Set(Some(result.raw.perceptual_loudness[7] as f64)),
        perceptual_loudness8: ActiveValue::Set(Some(result.raw.perceptual_loudness[8] as f64)),
        perceptual_loudness9: ActiveValue::Set(Some(result.raw.perceptual_loudness[9] as f64)),
        perceptual_loudness10: ActiveValue::Set(Some(result.raw.perceptual_loudness[10] as f64)),
        perceptual_loudness11: ActiveValue::Set(Some(result.raw.perceptual_loudness[11] as f64)),
        perceptual_loudness12: ActiveValue::Set(Some(result.raw.perceptual_loudness[12] as f64)),
        perceptual_loudness13: ActiveValue::Set(Some(result.raw.perceptual_loudness[13] as f64)),
        perceptual_loudness14: ActiveValue::Set(Some(result.raw.perceptual_loudness[14] as f64)),
        perceptual_loudness15: ActiveValue::Set(Some(result.raw.perceptual_loudness[15] as f64)),
        perceptual_loudness16: ActiveValue::Set(Some(result.raw.perceptual_loudness[16] as f64)),
        perceptual_loudness17: ActiveValue::Set(Some(result.raw.perceptual_loudness[17] as f64)),
        perceptual_loudness18: ActiveValue::Set(Some(result.raw.perceptual_loudness[18] as f64)),
        perceptual_loudness19: ActiveValue::Set(Some(result.raw.perceptual_loudness[19] as f64)),
        perceptual_loudness20: ActiveValue::Set(Some(result.raw.perceptual_loudness[20] as f64)),
        perceptual_loudness21: ActiveValue::Set(Some(result.raw.perceptual_loudness[21] as f64)),
        perceptual_loudness22: ActiveValue::Set(Some(result.raw.perceptual_loudness[22] as f64)),
        perceptual_loudness23: ActiveValue::Set(Some(result.raw.perceptual_loudness[23] as f64)),
        mfcc0: ActiveValue::Set(Some(result.raw.mfcc[0] as f64)),
        mfcc1: ActiveValue::Set(Some(result.raw.mfcc[1] as f64)),
        mfcc2: ActiveValue::Set(Some(result.raw.mfcc[2] as f64)),
        mfcc3: ActiveValue::Set(Some(result.raw.mfcc[3] as f64)),
        mfcc4: ActiveValue::Set(Some(result.raw.mfcc[4] as f64)),
        mfcc5: ActiveValue::Set(Some(result.raw.mfcc[5] as f64)),
        mfcc6: ActiveValue::Set(Some(result.raw.mfcc[6] as f64)),
        mfcc7: ActiveValue::Set(Some(result.raw.mfcc[7] as f64)),
        mfcc8: ActiveValue::Set(Some(result.raw.mfcc[8] as f64)),
        mfcc9: ActiveValue::Set(Some(result.raw.mfcc[9] as f64)),
        mfcc10: ActiveValue::Set(Some(result.raw.mfcc[10] as f64)),
        mfcc11: ActiveValue::Set(Some(result.raw.mfcc[11] as f64)),
        mfcc12: ActiveValue::Set(Some(result.raw.mfcc[12] as f64)),
        ..Default::default()
    };

    media_analysis::Entity::insert(new_analysis)
        .exec(db)
        .await?;

    Ok(())
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
