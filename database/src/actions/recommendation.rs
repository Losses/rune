use std::collections::HashSet;
use std::num::NonZeroUsize;

use anyhow::{bail, Context, Result};
use arroy::distances::Euclidean;
use arroy::{Reader, Writer};
use log::error;
use rand::rngs::StdRng;
use rand::SeedableRng;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal_macros::dec;
use sea_orm::entity::prelude::*;
use seq_macro::seq;

use crate::connection::{MainDbConnection, RecommendationDbConnection};
use crate::entities::{media_analysis, media_files};

use super::analysis::get_percentile_analysis_result;

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

    let results = reader
        .nns_by_vector(&rtxn, &feature_vector, n, Some(search_k), None)
        .with_context(|| "Failed to get recommendation by parameter");

    match results {
        Ok(results) => {
            if results.is_empty() {
                bail!("No results found for the given parameter")
            } else {
                Ok(results)
            }
        }
        Err(e) => {
            error!("{:#?}", e);
            Ok(vec![])
        }
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
    let analyzes = media_analysis::Entity::find().all(main_db).await?;

    // Track existing IDs in the main database
    let mut existing_ids: HashSet<i32> = HashSet::new();
    for analysis in &analyzes {
        existing_ids.insert(analysis.file_id);
    }

    // Open a write transaction for the recommendation database
    let mut wtxn = env.write_txn()?;
    let writer = Writer::<Euclidean>::new(arroy_db, 0, 61);

    // Insert or update analysis data in the recommendation database
    for analysis in analyzes {
        let mut vector = vec![
            analysis
                .rms
                .unwrap_or(dec!(0.0))
                .to_f32()
                .expect("Unable to convert RMS"),
            analysis
                .zcr
                .unwrap_or(dec!(0.0))
                .to_f32()
                .expect("Unable to convert ZCR"),
            analysis
                .energy
                .unwrap_or(dec!(0.0))
                .to_f32()
                .expect("Unable to convert Energy"),
            analysis
                .spectral_centroid
                .unwrap_or(dec!(0.0))
                .to_f32()
                .expect("Unable to convert Spectral Centroid"),
            analysis
                .spectral_flatness
                .unwrap_or(dec!(0.0))
                .to_f32()
                .expect("Unable to convert Spectral Flatness"),
            analysis
                .spectral_slope
                .unwrap_or(dec!(0.0))
                .to_f32()
                .expect("Unable to convert Spectral Slope"),
            analysis
                .spectral_rolloff
                .unwrap_or(dec!(0.0))
                .to_f32()
                .expect("Unable to convert Spectral Rolloff"),
            analysis
                .spectral_spread
                .unwrap_or(dec!(0.0))
                .to_f32()
                .expect("Unable to convert Spectral Spread"),
            analysis
                .spectral_skewness
                .unwrap_or(dec!(0.0))
                .to_f32()
                .expect("Unable to convert Spectral Skewness"),
            analysis
                .spectral_kurtosis
                .unwrap_or(dec!(0.0))
                .to_f32()
                .expect("Unable to convert Spectral Kurtosis"),
            analysis
                .perceptual_spread
                .unwrap_or(dec!(0.0))
                .to_f32()
                .expect("Unable to convert Perceptual Spread"),
            analysis
                .perceptual_sharpness
                .unwrap_or(dec!(0.0))
                .to_f32()
                .expect("Unable to convert Spectral Sharpness"),
        ];

        seq!(N in 0..12 {
            vector.push(analysis.chroma~N.unwrap_or(dec!(0.0)).to_f32().expect("Unable to convert Chroma"));
        });
        seq!(N in 0..24 {
            vector.push(analysis.perceptual_loudness~N.unwrap_or(dec!(0.0)).to_f32().expect("Unable to convert Perceptual Loudness"));
        });
        seq!(N in 0..13 {
            vector.push(analysis.mfcc~N.unwrap_or(dec!(0.0)).to_f32().expect("Unable to convert MFCC"));
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

pub async fn get_recommendation_by_percentile(
    main_db: &MainDbConnection,
    recommend_db: &RecommendationDbConnection,
    total_groups: usize,
    group_index: usize,
) -> Result<Vec<(u32, f32)>> {
    let p = 1.0 / (total_groups + 2) as f64 * (group_index + 1) as f64;

    let virtual_point = get_percentile_analysis_result(main_db, p).await?;
    let total_files = media_files::Entity::find()
        .count(main_db)
        .await
        .with_context(|| "Unable to get total files")? as usize;

    get_recommendation_by_parameter(recommend_db, virtual_point, total_files / total_groups)
}
