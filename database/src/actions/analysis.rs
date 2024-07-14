use sea_orm::entity::prelude::*;
use sea_orm::{ColumnTrait, CursorTrait, EntityTrait, QueryFilter, QueryOrder};
use std::path::{Path, PathBuf};
use tokio::task;
use futures::stream::{self, StreamExt};
use crate::entities::{media_files, media_analysis};
use analysis::analysis::{analyze_audio, normalize_analysis_result};

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

    loop {
        // Fetch the next batch of files
        let files: Vec<media_files::Model> = cursor.first(batch_size).all(db).await?;

        if files.is_empty() {
            break;
        }

        // Process each file in parallel using multiple cores
        let tasks: Vec<_> = files.into_iter().map(|file| {
            let db = db.clone();
            let root_path = root_path.to_path_buf();
            task::spawn(async move {
                process_file_if_needed(&db, &file, &root_path).await
            })
        }).collect();

        // Wait for all tasks to complete
        let results: Vec<_> = stream::iter(tasks).buffer_unordered(batch_size).collect().await;

        // Check for any errors
        for result in results {
            if let Err(e) = result {
                eprintln!("Error processing file: {:?}", e);
            }
        }

        // Move the cursor to the next batch
        if let Some(last_file) = files.last() {
            cursor.after(last_file.id);
        } else {
            break;
        }
    }

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
    root_path: &PathBuf,
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

    media_analysis::Entity::insert(new_analysis).exec(db).await.unwrap();
}