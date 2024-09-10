use std::collections::HashSet;
use std::num::NonZeroUsize;

use arroy::distances::Euclidean;
use arroy::{Reader, Writer};
use rand::rngs::StdRng;
use rand::SeedableRng;
use sea_orm::entity::prelude::*;
use seq_macro::seq;

use crate::connection::{MainDbConnection, RecommendationDbConnection};

use super::analysis::AggregatedAnalysisResult;

/// Get recommendations for a given item.
///
/// # Arguments
/// * `db_conn` - The tuple containing the LMDB environment and the Arroy database.
/// * `item_id` - The ID of the item for which to get recommendations.
/// * `n` - The number of recommendations to retrieve.
///
/// # Returns
/// * `Result<Vec<(usize, f32)>, Box<dyn std::error::Error>>` - A vector of recommended item IDs and their distances.
pub fn get_recommendation_by_file_id(
    db_conn: &RecommendationDbConnection,
    item_id: i32,
    n: usize,
) -> Result<Vec<(u32, f32)>, Box<dyn std::error::Error>> {
    let env = db_conn.env.clone();
    let db = db_conn.db;
    let rtxn = env.read_txn()?;
    let reader = Reader::<Euclidean>::open(&rtxn, 0, db)?;
    let search_k = NonZeroUsize::new(n * reader.n_trees() * 61)
        .ok_or("Failed to create NonZeroUsize from search_k")?;

    let item_id: u32 = item_id
        .try_into()
        .map_err(|_| "Failed to convert item_id to u32")?;

    let results = reader
        .nns_by_item(&rtxn, item_id, n, Some(search_k), None)?
        .ok_or("No results found for the given item_id")?;

    Ok(results)
}

/// Get recommendations for a given item.
///
/// # Arguments
/// * `db_conn` - The tuple containing the LMDB environment and the Arroy database.
/// * `item_id` - The ID of the item for which to get recommendations.
/// * `n` - The number of recommendations to retrieve.
///
/// # Returns
/// * `Result<Vec<(usize, f32)>, Box<dyn std::error::Error>>` - A vector of recommended item IDs and their distances.
pub fn get_recommendation_by_parameter(
    db_conn: &RecommendationDbConnection,
    parameter: AggregatedAnalysisResult,
    n: usize,
) -> Result<Vec<(u32, f32)>, Box<dyn std::error::Error>> {
    let env = db_conn.env.clone();
    let db = db_conn.db;
    let rtxn = env.read_txn()?;
    let reader = Reader::<Euclidean>::open(&rtxn, 0, db)?;
    let search_k = NonZeroUsize::new(n * reader.n_trees() * 15)
        .ok_or("Failed to create NonZeroUsize from search_k")?;

    let feature_vector: Vec<f32> = vec![
        parameter.rms,
        parameter.zcr,
        parameter.energy,
        parameter.spectral_centroid,
        parameter.spectral_flatness,
        parameter.spectral_slope,
        parameter.spectral_rolloff,
        parameter.spectral_spread,
        parameter.spectral_skewness,
        parameter.spectral_kurtosis,
        parameter.chroma[0],
        parameter.chroma[1],
        parameter.chroma[2],
        parameter.chroma[3],
        parameter.chroma[4],
        parameter.chroma[5],
        parameter.chroma[6],
        parameter.chroma[7],
        parameter.chroma[8],
        parameter.chroma[9],
        parameter.chroma[10],
        parameter.chroma[11],
        parameter.perceptual_spread,
        parameter.perceptual_sharpness,
        parameter.perceptual_loudness[0],
        parameter.perceptual_loudness[1],
        parameter.perceptual_loudness[2],
        parameter.perceptual_loudness[3],
        parameter.perceptual_loudness[4],
        parameter.perceptual_loudness[5],
        parameter.perceptual_loudness[6],
        parameter.perceptual_loudness[7],
        parameter.perceptual_loudness[8],
        parameter.perceptual_loudness[9],
        parameter.perceptual_loudness[10],
        parameter.perceptual_loudness[11],
        parameter.perceptual_loudness[12],
        parameter.perceptual_loudness[13],
        parameter.perceptual_loudness[14],
        parameter.perceptual_loudness[15],
        parameter.perceptual_loudness[16],
        parameter.perceptual_loudness[17],
        parameter.perceptual_loudness[18],
        parameter.perceptual_loudness[19],
        parameter.perceptual_loudness[20],
        parameter.perceptual_loudness[21],
        parameter.perceptual_loudness[22],
        parameter.perceptual_loudness[23],
        parameter.mfcc[0],
        parameter.mfcc[1],
        parameter.mfcc[2],
        parameter.mfcc[3],
        parameter.mfcc[4],
        parameter.mfcc[5],
        parameter.mfcc[6],
        parameter.mfcc[7],
        parameter.mfcc[8],
        parameter.mfcc[9],
        parameter.mfcc[10],
        parameter.mfcc[11],
        parameter.mfcc[12],
    ]
    .into_iter()
    .map(|x| x as f32)
    .collect();

    let results = reader.nns_by_vector(&rtxn, &feature_vector, n, Some(search_k), None)?;

    if results.is_empty() {
        Err("No results found for the given parameter".into())
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
/// * `Result<(), Box<dyn std::error::Error>>` - A result indicating success or failure.
pub async fn sync_recommendation(
    main_db: &MainDbConnection,
    recommend_db: &RecommendationDbConnection,
) -> Result<(), Box<dyn std::error::Error>> {
    let env = recommend_db.env.clone();
    let arroy_db = recommend_db.db;

    // Fetch all analysis data
    let analyses = crate::entities::media_analysis::Entity::find()
        .all(main_db)
        .await?;

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
