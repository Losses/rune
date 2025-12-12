use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use fsio::FsIo;
use futures::future::join_all;
use log::info;
use paste::paste;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::prelude::ToPrimitive;
use sea_orm::entity::prelude::*;
use sea_orm::{ActiveValue, QueryOrder, QuerySelect};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use seq_macro::seq;
use tokio_util::sync::CancellationToken;

use analysis::analysis::{NormalizedAnalysisResult, analyze_audio, normalize_analysis_result};
use analysis::utils::computing_device::ComputingDevice;
use uuid::Uuid;

use crate::entities::{media_analysis, media_files};
use crate::parallel_media_files_processing;

pub fn empty_progress_callback(_processed: usize, _total: usize) {}

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
    node_id: &str,
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

    let existed_ids: Vec<i32> = media_analysis::Entity::find()
        .select_only()
        .column(media_analysis::Column::FileId)
        .distinct()
        .into_tuple::<i32>()
        .all(main_db)
        .await?;

    let cursor_query =
        media_files::Entity::find().filter(media_files::Column::Id.is_not_in(existed_ids));

    let lib_path = Arc::new(lib_path.to_path_buf());
    let node_id = Arc::new(node_id.to_owned());
    // In analysis we don't strictly need HLC context yet, but macro requires it.
    // Creating a dummy context or using real one if available.
    // Ideally we should pass real node_id UUID to create proper context.
    // For now, let's try to parse node_id string to UUID.
    let node_uuid = Uuid::from_str(&node_id).unwrap_or_default();
    let hlc_context = Arc::new(sync::hlc::SyncTaskContext::new(node_uuid));

    parallel_media_files_processing!(
        main_db,
        batch_size,
        progress_callback,
        cancel_token,
        cursor_query,
        lib_path,
        fsio,
        node_id,
        hlc_context,
        move |fsio, file, lib_path, cancel_token| {
            analysis_file(fsio, file, lib_path, computing_device, cancel_token)
        },
        |db,
         file: media_files::Model,
         node_id: Arc<String>,
         _hlc_context: Arc<sync::hlc::SyncTaskContext>,
         analysis_result: Result<Option<NormalizedAnalysisResult>>| async move {
            match analysis_result {
                Ok(analysis_result) => {
                    if let Some(x) = analysis_result {
                        match insert_analysis_result(db, &node_id, file.id, &file.file_hash, x)
                            .await
                        {
                            Ok(_) => debug!("Finished analysis: {}", file.id),
                            Err(e) => error!("Failed to insert analysis result: {e}"),
                        }
                    };
                }
                Err(e) => error!("Failed to analyze track: {e}"),
            }
        }
    )
}

/// Process a file if it has not been analyzed yet. Perform audio analysis and store the results
/// in the database.
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

    // Perform audio analysis
    let analysis_result = analyze_audio(
        fsio,
        file_path.to_str().expect("Unable to convert file path"),
        1024, // Example window size
        512,  // Example overlap size
        computing_device,
        cancel_token,
    )?;

    if analysis_result.is_none() {
        return Ok(None);
    }

    let analysis_result = analysis_result.expect("Analysis result should never be none");

    // Normalize the analysis result
    Ok(Some(normalize_analysis_result(&analysis_result)))
}

/// Insert the normalized analysis result into the database.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `file_id` - The ID of the file being analyzed.
/// * `result` - The normalized analysis result.
async fn insert_analysis_result(
    main_db: &DatabaseConnection,
    node_id: &str,
    file_id: i32,
    file_hash: &str,
    result: NormalizedAnalysisResult,
) -> Result<()> {
    let mut new_analysis = media_analysis::ActiveModel {
        file_id: ActiveValue::Set(file_id),
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
        hlc_uuid: ActiveValue::Set(
            Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("RUNE_ANALYSIS::{file_hash}").as_bytes(),
            )
            .to_string(),
        ),
        created_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
        updated_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
        created_at_hlc_ver: ActiveValue::Set(0),
        updated_at_hlc_ver: ActiveValue::Set(0),
        created_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
        updated_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
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

    media_analysis::Entity::insert(new_analysis)
        .exec(main_db)
        .await?;

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
        .filter(media_analysis::Column::FileId.eq(file_id))
        .count(main_db)
        .await?
        != 0)
}

pub async fn get_analyze_count(main_db: &DatabaseConnection) -> Result<u64> {
    Ok(media_analysis::Entity::find().count(main_db).await?)
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

    let total_files = media_files::Entity::find()
        .count(main_db)
        .await
        .with_context(|| "Unable to get total files")? as usize;

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
