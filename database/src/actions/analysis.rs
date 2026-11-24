use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use fsio::FsIo;
use futures::future::join_all;
use log::info;
use paste::paste;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::prelude::ToPrimitive;
use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Condition, Expr};
use sea_orm::{
    ActiveValue, ExprTrait, JoinType, QueryOrder, QuerySelect, QueryTrait, TransactionTrait,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use seq_macro::seq;
use tokio_util::sync::CancellationToken;

use analysis::analysis::{NormalizedAnalysisResult, analyze_audio, normalize_analysis_result};
use analysis::utils::computing_device::ComputingDevice;
use uuid::Uuid;

use crate::actions::file::hash_file;
use crate::actions::utils::ParallelMediaFilesResult as BatchResult;
use crate::entities::{media_analysis, media_files};
use crate::parallel_media_files_processing;
use crate::sync::model::BlobRef;

pub fn empty_progress_callback(_processed: usize, _total: usize) {}

/// Match analysis rows that describe the file's current content.
///
/// A row is current only when it recorded the content hash and size it was
/// computed from and both still match the file. Rows without a recorded
/// identity (including every row written before migration 38) are never
/// current, so they are analyzed again.
pub(crate) fn current_analysis_condition() -> Condition {
    Condition::all()
        .add(media_analysis::Column::AnalysisVersion.eq(analysis::CURRENT_ANALYSIS_VERSION))
        .add(
            Expr::col((media_analysis::Entity, media_analysis::Column::SourceHash))
                .equals((media_files::Entity, media_files::Column::ContentHash)),
        )
        .add(
            Expr::col((media_analysis::Entity, media_analysis::Column::SourceSize))
                .equals((media_files::Entity, media_files::Column::ContentSize)),
        )
}

/// Analyze the audio library by reading existing files, checking if they have been analyzed,
/// and performing audio analysis if not. The function uses cursor pagination to process files
/// in batches for memory efficiency and utilizes multi-core parallelism for faster processing.
/// The analysis results are normalized before being stored in the database.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `lib_path` - The root path for the audio files.
/// * `batch_size` - The number of files to process in each batch.
/// * `progress_callback` - A callback function to report progress.
/// * `cancel_token` - An optional cancellation token to support task cancellation.
///
/// # Returns
/// * `Result<(), sea_orm::DbErr>` - A result indicating success or failure.
#[allow(clippy::too_many_arguments)]
pub async fn analysis_audio_library<F>(
    fsio: Arc<FsIo>,
    main_db: &DatabaseConnection,
    lib_path: &Path,
    batch_size: usize,
    computing_device: ComputingDevice,
    progress_callback: F,
    cancel_token: Option<CancellationToken>,
) -> Result<usize>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    let progress_callback = Arc::new(progress_callback);

    info!("Starting audio library analysis with batch size: {batch_size}");

    let current_analysis = media_analysis::Entity::find()
        .join(
            JoinType::InnerJoin,
            media_analysis::Relation::MediaFiles.def(),
        )
        .filter(current_analysis_condition())
        .select_only()
        .column(media_analysis::Column::FileId)
        .into_query();

    let cursor_query = media_files::Entity::find()
        .filter(media_files::Column::LocalAvailable.eq(true))
        .filter(media_files::Column::Id.not_in_subquery(current_analysis));

    let lib_path = Arc::new(lib_path.to_path_buf());
    parallel_media_files_processing!(
        main_db,
        batch_size,
        progress_callback,
        cancel_token,
        cursor_query,
        lib_path,
        fsio,
        move |fsio, file, lib_path, cancel_token| {
            analysis_file(fsio, file, lib_path, computing_device, cancel_token)
        },
        |db,
         file: media_files::Model,
         analysis_result: Result<Option<NormalizedAnalysisResult>>| async move {
            match analysis_result {
                Ok(analysis_result) => {
                    if let Some(x) = analysis_result {
                        match insert_analysis_result(db, &file, x).await {
                            Ok(()) => BatchResult::Processed,
                            Err(error) => BatchResult::TaskFailed(error),
                        }
                    } else {
                        BatchResult::Processed
                    }
                }
                Err(error) => BatchResult::FileFailed(error),
            }
        }
    )
}

/// The content identity a file must still have for its analysis to be stored.
fn verified_source(file: &media_files::Model) -> Option<BlobRef> {
    match (&file.content_hash, file.content_size) {
        (Some(digest), Some(size)) if size >= 0 => Some(BlobRef {
            digest: digest.clone(),
            size: size as u64,
        }),
        _ => None,
    }
}

/// Process a file if it has not been analyzed yet. Perform audio analysis and store the results
/// in the database.
///
/// The result is bound to the bytes that were analyzed: the file is hashed
/// before and after the computation, and a result whose bytes no longer match
/// the observed file is discarded.
///
/// # Arguments
/// * `db` - A reference to the database connection.
/// * `file` - A reference to the file model.
/// * `root_path` - The root path for the audio files.
/// * `cancel_token` - An optional cancellation token to support task cancellation.
fn analysis_file(
    fsio: &FsIo,
    file: &media_files::Model,
    lib_path: &Path,
    computing_device: ComputingDevice,
    cancel_token: Option<CancellationToken>,
) -> Result<Option<NormalizedAnalysisResult>> {
    // Construct the full path to the file
    let file_path = lib_path.join(&file.directory).join(&file.file_name);

    let Some(expected) = verified_source(file) else {
        info!(
            "Skipping analysis for file {} because it has no verified content identity",
            file.id
        );
        return Ok(None);
    };

    // The file row can be older than the bytes at the path. Only analyze the
    // content the row describes, as fingerprint computation does.
    if hash_file(fsio, &file_path)? != expected {
        info!(
            "Discarding analysis for file {} because its source changed before analysis",
            file.id
        );
        return Ok(None);
    }

    // Perform audio analysis
    let analysis_result = analyze_audio(
        fsio,
        file_path.to_str().expect("Unable to convert file path"),
        1024, // Example window size
        512,  // Example overlap size
        computing_device,
        cancel_token,
    )?;

    let Some(analysis_result) = analysis_result else {
        return Ok(None);
    };

    if hash_file(fsio, &file_path)? != expected {
        info!(
            "Discarding analysis for file {} because its source changed during analysis",
            file.id
        );
        return Ok(None);
    }

    // Normalize the analysis result
    Ok(Some(normalize_analysis_result(&analysis_result)))
}

/// Insert the normalized analysis result into the database.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `observed` - The file row the analysis was started from.
/// * `result` - The normalized analysis result.
async fn insert_analysis_result(
    main_db: &DatabaseConnection,
    observed: &media_files::Model,
    result: NormalizedAnalysisResult,
) -> Result<()> {
    // Only the small replacement is transactional; decoding and DSP happen beforehand.
    let txn = main_db.begin().await?;
    let current = media_files::Entity::find_by_id(observed.id)
        .one(&txn)
        .await?;
    let Some(current) = current else {
        info!(
            "Discarding analysis for file {} because the file disappeared",
            observed.id
        );
        txn.rollback().await?;
        return Ok(());
    };
    if current.source_uuid != observed.source_uuid
        || current.last_modified != observed.last_modified
        || current.content_hash != observed.content_hash
        || current.content_size != observed.content_size
        || !current.local_available
    {
        info!(
            "Discarding analysis for file {} because its source changed during analysis",
            observed.id
        );
        txn.rollback().await?;
        return Ok(());
    }
    let (Some(source_hash), Some(source_size)) =
        (current.content_hash.clone(), current.content_size)
    else {
        info!(
            "Discarding analysis for file {} because it has no verified content identity",
            observed.id
        );
        txn.rollback().await?;
        return Ok(());
    };

    let mut new_analysis = media_analysis::ActiveModel {
        file_id: ActiveValue::Set(current.id),
        analysis_version: ActiveValue::Set(analysis::CURRENT_ANALYSIS_VERSION),
        source_hash: ActiveValue::Set(Some(source_hash)),
        source_size: ActiveValue::Set(Some(source_size)),
        rms: ActiveValue::Set(Decimal::from_f32(result.raw.rms)),
        zcr: ActiveValue::Set(Decimal::from_f32(result.zcr)),
        energy: ActiveValue::Set(Decimal::from_f32(result.energy)),
        spectral_centroid: ActiveValue::Set(Decimal::from_f32(result.spectral_centroid)),
        spectral_flatness: ActiveValue::Set(Decimal::from_f32(result.spectral_flatness)),
        spectral_slope: ActiveValue::Set(Decimal::from_f32(result.spectral_slope)),
        spectral_rolloff: ActiveValue::Set(Decimal::from_f32(result.spectral_rolloff)),
        spectral_spread: ActiveValue::Set(Decimal::from_f32(result.spectral_spread)),
        spectral_skewness: ActiveValue::Set(Decimal::from_f32(result.spectral_skewness)),
        spectral_kurtosis: ActiveValue::Set(Decimal::from_f32(result.spectral_kurtosis)),
        perceptual_spread: ActiveValue::Set(Decimal::from_f32(result.raw.perceptual_spread)),
        perceptual_sharpness: ActiveValue::Set(Decimal::from_f32(result.raw.perceptual_sharpness)),
        source_uuid: ActiveValue::Set(
            Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!(
                    "RUNE_ANALYSIS::v{}::{}",
                    analysis::CURRENT_ANALYSIS_VERSION,
                    current.file_hash
                )
                .as_bytes(),
            )
            .to_string(),
        ),
        ..Default::default()
    };

    seq!(N in 0..12 {
        new_analysis.chroma~N = ActiveValue::Set(Decimal::from_f32(result.chroma[N]));
    });

    seq!(N in 0..24 {
        new_analysis.perceptual_loudness~N = ActiveValue::Set(Decimal::from_f32(result.raw.perceptual_loudness[N]));
    });

    seq!(N in 0..13 {
        new_analysis.mfcc~N = ActiveValue::Set(Decimal::from_f32(result.raw.mfcc[N]));
    });

    media_analysis::Entity::delete_many()
        .filter(media_analysis::Column::FileId.eq(current.id))
        .filter(media_analysis::Column::AnalysisVersion.lte(analysis::CURRENT_ANALYSIS_VERSION))
        .exec(&txn)
        .await?;
    media_analysis::Entity::insert(new_analysis)
        .exec(&txn)
        .await?;
    txn.commit().await?;

    Ok(())
}

/// Struct to store mean values of analysis results.
#[derive(Debug)]
pub struct AggregatedAnalysisResult {
    pub rms: f64,
    pub zcr: f64,
    pub energy: f64,
    pub spectral_centroid: f64,
    pub spectral_flatness: f64,
    pub spectral_slope: f64,
    pub spectral_rolloff: f64,
    pub spectral_spread: f64,
    pub spectral_skewness: f64,
    pub spectral_kurtosis: f64,
    pub chroma: [f64; 12],
    pub perceptual_spread: f64,
    pub perceptual_sharpness: f64,
    pub perceptual_loudness: [f64; 24],
    pub mfcc: [f64; 13],
}

impl From<AggregatedAnalysisResult> for [f32; 61] {
    fn from(val: AggregatedAnalysisResult) -> Self {
        [
            val.rms,
            val.zcr,
            val.energy,
            val.spectral_centroid,
            val.spectral_flatness,
            val.spectral_slope,
            val.spectral_rolloff,
            val.spectral_spread,
            val.spectral_skewness,
            val.spectral_kurtosis,
        ]
        .iter()
        .chain(&val.chroma)
        .chain(&vec![val.perceptual_spread, val.perceptual_sharpness])
        .chain(&val.perceptual_loudness)
        .chain(&val.mfcc)
        .map(|x| *x as f32)
        .collect::<Vec<f32>>()
        .try_into()
        .expect("Expected a Vec of length 61")
    }
}

impl From<media_analysis::Model> for AggregatedAnalysisResult {
    fn from(model: media_analysis::Model) -> Self {
        AggregatedAnalysisResult {
            rms: model.rms.unwrap_or_default().to_f64().unwrap_or_default(),
            zcr: model.zcr.unwrap_or_default().to_f64().unwrap_or_default(),
            energy: model
                .energy
                .unwrap_or_default()
                .to_f64()
                .unwrap_or_default(),
            spectral_centroid: model
                .spectral_centroid
                .unwrap_or_default()
                .to_f64()
                .unwrap_or_default(),
            spectral_flatness: model
                .spectral_flatness
                .unwrap_or_default()
                .to_f64()
                .unwrap_or_default(),
            spectral_slope: model
                .spectral_slope
                .unwrap_or_default()
                .to_f64()
                .unwrap_or_default(),
            spectral_rolloff: model
                .spectral_rolloff
                .unwrap_or_default()
                .to_f64()
                .unwrap_or_default(),
            spectral_spread: model
                .spectral_spread
                .unwrap_or_default()
                .to_f64()
                .unwrap_or_default(),
            spectral_skewness: model
                .spectral_skewness
                .unwrap_or_default()
                .to_f64()
                .unwrap_or_default(),
            spectral_kurtosis: model
                .spectral_kurtosis
                .unwrap_or_default()
                .to_f64()
                .unwrap_or_default(),
            chroma: {
                let mut chroma_array = [0.0; 12];
                seq!(N in 0..=11 {
                    chroma_array[N] = model.chroma~N.map(|d| d.to_f64().unwrap_or(0.0)).unwrap_or(0.0);
                });
                chroma_array
            },
            perceptual_spread: model
                .perceptual_spread
                .unwrap_or_default()
                .to_f64()
                .unwrap_or_default(),
            perceptual_sharpness: model
                .perceptual_sharpness
                .unwrap_or_default()
                .to_f64()
                .unwrap_or_default(),
            perceptual_loudness: {
                let mut loudness_array = [0.0; 24];
                seq!(N in 0..=23 {
                    loudness_array[N] = model.perceptual_loudness~N.map(|d| d.to_f64().unwrap_or(0.0)).unwrap_or(0.0);
                });
                loudness_array
            },
            mfcc: {
                let mut mfcc_array = [0.0; 13];
                seq!(N in 0..=12 {
                    mfcc_array[N] = model.mfcc~N.map(|d| d.to_f64().unwrap_or(0.0)).unwrap_or(0.0);
                });
                mfcc_array
            },
        }
    }
}

/// Macro to process individual fields by updating their sum and count.
macro_rules! process_field {
    ($sum:expr, $count:expr, $result:expr, $field:ident) => {
        if let Some(value) = $result.$field {
            $sum.$field += value.to_f64().expect("Unable to convert parameter");
            $count.$field += 1.0;
        }
    };
}

/// Macro to process array fields by updating their sum and count.
macro_rules! process_array {
    ($sum:expr, $count:expr, $result:expr, $field_prefix:ident, $size:expr) => {
        seq!(N in 0..$size {
            paste! {
                if let Some(value) = $result.[<$field_prefix N>] {
                    $sum.[<$field_prefix>][N] += value.to_f64().expect("Unable to convert parameter");
                    $count.[<$field_prefix>][N] += 1.0;
                }
            }
        });
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

/// Macro to calculate the mean of array fields.
macro_rules! calculate_array_mean {
    ($sum:expr, $count:expr, $field:ident, $size:expr) => {{
        let mut result = [0.0; $size];
        for i in 0..$size {
            result[i] = if $count.$field[i] > 0.0 {
                $sum.$field[i] / $count.$field[i]
            } else {
                0.0
            };
        }
        result
    }};
}

pub async fn if_analyze_exists(main_db: &DatabaseConnection, file_id: i32) -> Result<bool> {
    Ok(media_analysis::Entity::find()
        .join(
            JoinType::InnerJoin,
            media_analysis::Relation::MediaFiles.def(),
        )
        .filter(current_analysis_condition())
        .filter(media_analysis::Column::FileId.eq(file_id))
        .exists(main_db)
        .await?)
}

pub async fn get_analyze_count(main_db: &DatabaseConnection) -> Result<u64> {
    Ok(media_analysis::Entity::find()
        .join(
            JoinType::InnerJoin,
            media_analysis::Relation::MediaFiles.def(),
        )
        .filter(current_analysis_condition())
        .count(main_db)
        .await?)
}

/// Computes the centralized analysis result from the database.
///
/// This function retrieves analysis results based on specified file IDs,
/// sums the parameters, and calculates averages while handling potential `None` values.
///
/// # Arguments
///
/// * `main_db` - A reference to the database connection.
/// * `file_ids` - A vector of file IDs to filter the analysis results.
///
/// # Returns
///
/// * `AnalysisResultMean` - A struct containing the mean values of the analysis results.
///
pub async fn get_centralized_analysis_result(
    main_db: &DatabaseConnection,
    file_ids: Vec<i32>,
) -> Result<AggregatedAnalysisResult> {
    let analysis_results = media_analysis::Entity::find()
        .join(
            JoinType::InnerJoin,
            media_analysis::Relation::MediaFiles.def(),
        )
        .filter(current_analysis_condition())
        .filter(media_analysis::Column::FileId.is_in(file_ids))
        .all(main_db)
        .await?;

    let mut sum = AggregatedAnalysisResult {
        rms: 0.0,
        zcr: 0.0,
        energy: 0.0,
        spectral_centroid: 0.0,
        spectral_flatness: 0.0,
        spectral_slope: 0.0,
        spectral_rolloff: 0.0,
        spectral_spread: 0.0,
        spectral_skewness: 0.0,
        spectral_kurtosis: 0.0,
        chroma: [0.0; 12],
        perceptual_spread: 0.0,
        perceptual_sharpness: 0.0,
        perceptual_loudness: [0.0; 24],
        mfcc: [0.0; 13],
    };

    let mut count = AggregatedAnalysisResult {
        rms: 0.0,
        zcr: 0.0,
        energy: 0.0,
        spectral_centroid: 0.0,
        spectral_flatness: 0.0,
        spectral_slope: 0.0,
        spectral_rolloff: 0.0,
        spectral_spread: 0.0,
        spectral_skewness: 0.0,
        spectral_kurtosis: 0.0,
        chroma: [0.0; 12],
        perceptual_spread: 0.0,
        perceptual_sharpness: 0.0,
        perceptual_loudness: [0.0; 24],
        mfcc: [0.0; 13],
    };

    for result in analysis_results {
        process_field!(sum, count, result, rms);
        process_field!(sum, count, result, zcr);
        process_field!(sum, count, result, energy);
        process_field!(sum, count, result, spectral_centroid);
        process_field!(sum, count, result, spectral_flatness);
        process_field!(sum, count, result, spectral_slope);
        process_field!(sum, count, result, spectral_rolloff);
        process_field!(sum, count, result, spectral_spread);
        process_field!(sum, count, result, spectral_skewness);
        process_field!(sum, count, result, spectral_kurtosis);
        process_field!(sum, count, result, perceptual_spread);
        process_field!(sum, count, result, perceptual_sharpness);

        process_array!(sum, count, result, perceptual_loudness, 24);
        process_array!(sum, count, result, mfcc, 13);
        process_array!(sum, count, result, chroma, 12);
    }

    Ok(AggregatedAnalysisResult {
        rms: calculate_mean!(sum, count, rms),
        zcr: calculate_mean!(sum, count, zcr),
        energy: calculate_mean!(sum, count, energy),
        spectral_centroid: calculate_mean!(sum, count, spectral_centroid),
        spectral_flatness: calculate_mean!(sum, count, spectral_flatness),
        spectral_slope: calculate_mean!(sum, count, spectral_slope),
        spectral_rolloff: calculate_mean!(sum, count, spectral_rolloff),
        spectral_spread: calculate_mean!(sum, count, spectral_spread),
        spectral_skewness: calculate_mean!(sum, count, spectral_skewness),
        spectral_kurtosis: calculate_mean!(sum, count, spectral_kurtosis),
        perceptual_spread: calculate_mean!(sum, count, perceptual_spread),
        perceptual_sharpness: calculate_mean!(sum, count, perceptual_sharpness),
        chroma: calculate_array_mean!(sum, count, chroma, 12),
        perceptual_loudness: calculate_array_mean!(sum, count, perceptual_loudness, 24),
        mfcc: calculate_array_mean!(sum, count, mfcc, 13),
    })
}

pub async fn get_percentile(
    main_db: &DatabaseConnection,
    n: usize,
    column: media_analysis::Column,
    percentile: f64,
) -> Result<f32> {
    // Check if values are empty
    if n == 0 {
        return Ok(0.0);
    }

    // Calculate the rank
    let rank = percentile * (n as f64 - 1.0);
    let index = rank.round() as u64;

    let result = match media_analysis::Entity::find()
        .join(
            JoinType::InnerJoin,
            media_analysis::Relation::MediaFiles.def(),
        )
        .filter(current_analysis_condition())
        .select_only()
        .order_by_asc(column)
        .column(column)
        .offset(index)
        .limit(1)
        .into_tuple::<f32>()
        .one(main_db)
        .await
    {
        Ok(x) => x,
        Err(_) => Some(0.0),
    };
    // .with_context(|| "Unable to get analysis value")?;

    Ok(result.unwrap_or_default())
}

pub async fn get_percentile_analysis_result(
    main_db: &DatabaseConnection,
    percentile: f64,
) -> Result<[f32; 61]> {
    let columns: Vec<media_analysis::Column> = [
        media_analysis::Column::Rms,
        media_analysis::Column::Zcr,
        media_analysis::Column::Energy,
        media_analysis::Column::SpectralCentroid,
        media_analysis::Column::SpectralFlatness,
        media_analysis::Column::SpectralSlope,
        media_analysis::Column::SpectralRolloff,
        media_analysis::Column::SpectralSpread,
        media_analysis::Column::SpectralSkewness,
        media_analysis::Column::SpectralKurtosis,
    ]
    .into_iter()
    .chain(seq!(N in 0..12 {[
        #(media_analysis::Column::Chroma~N,)*
    ]}))
    .chain([
        media_analysis::Column::PerceptualSpread,
        media_analysis::Column::PerceptualSharpness,
    ])
    .chain(seq!(N in 0..24 {[
        #(media_analysis::Column::PerceptualLoudness~N,)*
    ]}))
    .chain(seq!(N in 0..13 {[
        #(media_analysis::Column::Mfcc~N,)*
    ]}))
    .collect();

    let total_files = get_analyze_count(main_db).await? as usize;

    let futures = columns
        .iter()
        .map(|column| get_percentile(main_db, total_files, *column, percentile));

    let percentiles = join_all(futures).await;

    let mut virtual_point = Vec::new();
    for percentile in percentiles {
        virtual_point.push(percentile.with_context(|| "Unable to calculate percentiles")?);
    }

    if virtual_point.len() != 61 {
        bail!(
            "Failed to convert virtual_point to array: incorrect length (got {}, expected {})",
            virtual_point.len(),
            61
        );
    }

    let virtual_point: [f32; 61] = virtual_point
        .try_into()
        .expect("Length checked above, this should never fail");

    Ok(virtual_point)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::fixtures::{audio_fixture, audio_fixture_at};
    use analysis::analysis::{AnalysisParameter, AnalysisResult, AudioStat};
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{Database, Set};

    fn normalized_result(spectral_centroid: f32) -> NormalizedAnalysisResult {
        normalize_analysis_result(&AnalysisResult {
            stat: AudioStat {
                sample_rate: 44100,
                duration: 1.0,
                total_samples: 44100,
            },
            parameters: AnalysisParameter {
                window_size: 1024,
                overlap_size: 512,
            },
            rms: 0.0,
            zcr: 0,
            energy: 0.0,
            spectral_centroid,
            spectral_flatness: 0.0,
            spectral_flux: 0.0,
            spectral_slope: 0.0,
            spectral_rolloff: 0.0,
            spectral_spread: 0.0,
            spectral_skewness: 0.0,
            spectral_kurtosis: 0.0,
            chromagram: [0.0; 12],
            perceptual_spread: 0.0,
            perceptual_sharpness: 0.0,
            perceptual_loudness: [0.0; 24],
            mfcc: [0.0; 13],
        })
    }

    #[tokio::test]
    async fn stale_result_is_not_stored_after_the_source_changes() -> Result<()> {
        let _ = migration::initialize_node_id(Uuid::new_v4().to_string());
        let db = Database::connect("sqlite::memory:").await?;
        Migrator::up(&db, None).await?;
        let observed = media_files::Entity::insert(media_files::ActiveModel {
            file_name: Set("track.ogg".into()),
            directory: Set(String::new()),
            extension: Set("ogg".into()),
            file_hash: Set("old-file-hash".into()),
            last_modified: Set("old".into()),
            sample_rate: Set(44100),
            duration: Set(1.into()),
            source_uuid: Set(Uuid::new_v4().to_string()),
            local_available: Set(true),
            content_hash: Set(Some("a".repeat(64))),
            content_size: Set(Some(100)),
            ..Default::default()
        })
        .exec_with_returning(&db)
        .await?;

        // A rescan observes new bytes while the old analysis is still running.
        media_files::Entity::update_many()
            .col_expr(media_files::Column::FileHash, Expr::value("new-file-hash"))
            .col_expr(media_files::Column::LastModified, Expr::value("new"))
            .col_expr(
                media_files::Column::ContentHash,
                Expr::value("b".repeat(64)),
            )
            .col_expr(media_files::Column::ContentSize, Expr::value(Some(200)))
            .filter(media_files::Column::Id.eq(observed.id))
            .exec(&db)
            .await?;

        insert_analysis_result(&db, &observed, normalized_result(0.5)).await?;

        assert_eq!(
            media_analysis::Entity::find().count(&db).await?,
            0,
            "an old computation must not become current after a rescan"
        );
        Ok(())
    }

    #[tokio::test]
    async fn analysis_of_bytes_that_do_not_match_the_file_row_is_discarded() -> Result<()> {
        let fsio = FsIo::new();
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("track.ogg");
        std::fs::copy(audio_fixture_at(1), &path)?;
        let expected = hash_file(&fsio, &audio_fixture())?;
        let observed = media_files::Model {
            id: 1,
            file_name: "track.ogg".into(),
            directory: String::new(),
            extension: "ogg".into(),
            file_hash: "file-hash".into(),
            last_modified: "2020".into(),
            change_token: None,
            cover_art_uuid: None,
            cover_art_checked_at: None,
            sample_rate: 44100,
            duration: 1.into(),
            local_available: true,
            content_hash: Some(expected.digest),
            content_size: Some(expected.size as i64),
            source_uuid: Uuid::new_v4().to_string(),
        };

        assert!(
            analysis_file(&fsio, &observed, dir.path(), ComputingDevice::Cpu, None)?.is_none(),
            "analysis must not describe bytes that differ from the observed file"
        );
        Ok(())
    }
}
