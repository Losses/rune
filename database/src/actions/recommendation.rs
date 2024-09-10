use std::collections::HashSet;
use std::num::NonZeroUsize;

use anyhow::{bail, Context, Result};
use arroy::distances::Euclidean;
use arroy::{Reader, Writer};
use futures::future::join_all;
use rand::rngs::StdRng;
use rand::SeedableRng;
use sea_orm::entity::prelude::*;
use sea_orm::{QueryOrder, QuerySelect};
use seq_macro::seq;

use crate::connection::{MainDbConnection, RecommendationDbConnection};
use crate::entities::{media_analysis, media_files};

/// Get recommendations for a given item.
///
/// # Arguments
/// * `main_db` - The tuple containing the LMDB environment and the Arroy database.
/// * `item_id` - The ID of the item for which to get recommendations.
/// * `n` - The number of recommendations to retrieve.
///
/// # Returns
/// * `Result<Vec<(usize, f32)>>` - A vector of recommended item IDs and their distances.
pub fn get_recommendation_by_file_id(
    main_db: &RecommendationDbConnection,
    item_id: i32,
    n: usize,
) -> Result<Vec<(u32, f32)>> {
    let env = main_db.env.clone();
    let db = main_db.db;
    let rtxn = env.read_txn()?;
    let reader = Reader::<Euclidean>::open(&rtxn, 0, db)?;
    let search_k = NonZeroUsize::new(n * reader.n_trees() * 15)
        .with_context(|| "Failed to create NonZeroUsize from search_k")?;

    let item_id: u32 = item_id
        .try_into()
        .with_context(|| "Failed to convert item_id to u32")?;

    let results = reader
        .nns_by_item(&rtxn, item_id, n, Some(search_k), None)?
        .with_context(|| "No results found for the given item_id")?;

    Ok(results)
}

/// Get recommendations for a given item.
///
/// # Arguments
/// * `recommend_db` - The tuple containing the LMDB environment and the Arroy database.
/// * `item_id` - The ID of the item for which to get recommendations.
/// * `n` - The number of recommendations to retrieve.
///
/// # Returns
/// * `Result<Vec<(usize, f32)>>` - A vector of recommended item IDs and their distances.
pub fn get_recommendation_by_parameter(
    recommend_db: &RecommendationDbConnection,
    feature_vector: [f32; 61],
    n: usize,
) -> Result<Vec<(u32, f32)>> {
    let env = recommend_db.env.clone();
    let db = recommend_db.db;
    let rtxn = env.read_txn()?;
    let reader = Reader::<Euclidean>::open(&rtxn, 0, db)?;
    let search_k = NonZeroUsize::new(n * reader.n_trees() * 15)
        .with_context(|| "Failed to create NonZeroUsize from search_k")?;

    let results = reader.nns_by_vector(&rtxn, &feature_vector, n, Some(search_k), None)?;

    if results.is_empty() {
        bail!("No results found for the given parameter")
    } else {
        Ok(results)
    }
}

/// Sync the recommendation database with the analysis data.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `recommend_db` - The tuple containing the LMDB environment and the Arroy database.
///
/// # Returns
/// * `Result<()>` - A result indicating success or failure.
pub async fn sync_recommendation(
    main_db: &MainDbConnection,
    recommend_db: &RecommendationDbConnection,
) -> Result<()> {
    let env = recommend_db.env.clone();
    let arroy_db = recommend_db.db;

    // Fetch all analysis data
    let analyses = media_analysis::Entity::find().all(main_db).await?;

    // Track existing IDs in the main database
    let mut existing_ids: HashSet<i32> = HashSet::new();
    for analysis in &analyses {
        existing_ids.insert(analysis.file_id);
    }

    // Open a write transaction for the recommendation database
    let mut wtxn = env.write_txn()?;
    let writer = Writer::<Euclidean>::new(arroy_db, 0, 61);

    // Insert or update analysis data in the recommendation database
    for analysis in analyses {
        let mut vector = vec![
            analysis.rms.unwrap_or(0.0) as f32,
            analysis.zcr.unwrap_or(0.0) as f32,
            analysis.energy.unwrap_or(0.0) as f32,
            analysis.spectral_centroid.unwrap_or(0.0) as f32,
            analysis.spectral_flatness.unwrap_or(0.0) as f32,
            analysis.spectral_slope.unwrap_or(0.0) as f32,
            analysis.spectral_rolloff.unwrap_or(0.0) as f32,
            analysis.spectral_spread.unwrap_or(0.0) as f32,
            analysis.spectral_skewness.unwrap_or(0.0) as f32,
            analysis.spectral_kurtosis.unwrap_or(0.0) as f32,
            analysis.perceptual_spread.unwrap_or(0.0) as f32,
            analysis.perceptual_sharpness.unwrap_or(0.0) as f32,
        ];

        seq!(N in 0..12 {
            vector.push(analysis.chroma~N.unwrap_or(0.0) as f32);
        });
        seq!(N in 0..24 {
            vector.push(analysis.perceptual_loudness~N.unwrap_or(0.0) as f32);
        });
        seq!(N in 0..13 {
            vector.push(analysis.mfcc~N.unwrap_or(0.0) as f32);
        });

        writer.add_item(
            &mut wtxn,
            (analysis.file_id as usize).try_into().unwrap(),
            &vector,
        )?;
    }

    // Build the index
    let mut rng = StdRng::seed_from_u64(42);
    writer.build(&mut wtxn, &mut rng, None)?;

    // Commit the transaction
    wtxn.commit()?;

    // Clean up the recommendation database by removing items not present in the main database
    let rtxn = env.read_txn()?;
    let reader = Reader::<Euclidean>::open(&rtxn, 0, arroy_db)?;
    for id in reader.item_ids() {
        if !existing_ids.contains(&(id as i32)) {
            let mut wtxn = env.write_txn()?;
            let writer = Writer::<Euclidean>::new(arroy_db, 0, 17);
            writer.del_item(&mut wtxn, id)?;
            wtxn.commit()?;
        }
    }

    Ok(())
}

pub async fn get_percentile(
    main_db: &MainDbConnection,
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

    let result = media_analysis::Entity::find()
        .select_only()
        .order_by_asc(column)
        .column(media_analysis::Column::FileId)
        .offset(index)
        .limit(1)
        .into_tuple::<f64>()
        .one(main_db)
        .await?;

    Ok(result.unwrap_or_default() as f32)
}

pub async fn recommend_by_percentile(
    main_db: &MainDbConnection,
    recommend_db: &RecommendationDbConnection,
    total_groups: usize,
    group_index: usize,
) -> Result<Vec<(u32, f32)>> {
    let columns = [
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
        media_analysis::Column::Chroma0,
        media_analysis::Column::Chroma1,
        media_analysis::Column::Chroma2,
        media_analysis::Column::Chroma3,
        media_analysis::Column::Chroma4,
        media_analysis::Column::Chroma5,
        media_analysis::Column::Chroma6,
        media_analysis::Column::Chroma7,
        media_analysis::Column::Chroma8,
        media_analysis::Column::Chroma9,
        media_analysis::Column::Chroma10,
        media_analysis::Column::Chroma11,
        media_analysis::Column::PerceptualSpread,
        media_analysis::Column::PerceptualSharpness,
        media_analysis::Column::PerceptualLoudness0,
        media_analysis::Column::PerceptualLoudness1,
        media_analysis::Column::PerceptualLoudness2,
        media_analysis::Column::PerceptualLoudness3,
        media_analysis::Column::PerceptualLoudness4,
        media_analysis::Column::PerceptualLoudness5,
        media_analysis::Column::PerceptualLoudness6,
        media_analysis::Column::PerceptualLoudness7,
        media_analysis::Column::PerceptualLoudness8,
        media_analysis::Column::PerceptualLoudness9,
        media_analysis::Column::PerceptualLoudness10,
        media_analysis::Column::PerceptualLoudness11,
        media_analysis::Column::PerceptualLoudness12,
        media_analysis::Column::PerceptualLoudness13,
        media_analysis::Column::PerceptualLoudness14,
        media_analysis::Column::PerceptualLoudness15,
        media_analysis::Column::PerceptualLoudness16,
        media_analysis::Column::PerceptualLoudness17,
        media_analysis::Column::PerceptualLoudness18,
        media_analysis::Column::PerceptualLoudness19,
        media_analysis::Column::PerceptualLoudness20,
        media_analysis::Column::PerceptualLoudness21,
        media_analysis::Column::PerceptualLoudness22,
        media_analysis::Column::PerceptualLoudness23,
        media_analysis::Column::Mfcc0,
        media_analysis::Column::Mfcc1,
        media_analysis::Column::Mfcc2,
        media_analysis::Column::Mfcc3,
        media_analysis::Column::Mfcc4,
        media_analysis::Column::Mfcc5,
        media_analysis::Column::Mfcc6,
        media_analysis::Column::Mfcc7,
        media_analysis::Column::Mfcc8,
        media_analysis::Column::Mfcc9,
        media_analysis::Column::Mfcc10,
        media_analysis::Column::Mfcc11,
        media_analysis::Column::Mfcc12,
    ];

    let total_files = media_files::Entity::find().count(main_db).await? as usize;

    let p = 1.0 / (total_groups as f64 + 2.0) * group_index as f64;
    let futures = columns
        .iter()
        .map(|column| get_percentile(main_db, total_files, *column, p));

    let percentiles = join_all(futures).await;

    let mut virtual_point = Vec::new();
    for percentile in percentiles {
        match percentile {
            Ok(value) => virtual_point.push(value),
            Err(e) => return Err(e),
        }
    }

    let virtual_point: [f32; 61] = virtual_point
        .try_into()
        .map_err(|_| anyhow::anyhow!("Failed to convert virtual_point to array"))?;

    get_recommendation_by_parameter(recommend_db, virtual_point, total_files / total_groups)
}
