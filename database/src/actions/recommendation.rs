use arroy::distances::Euclidean;
use arroy::{Reader, Writer};
use rand::rngs::StdRng;
use rand::SeedableRng;
use sea_orm::entity::prelude::*;
use std::collections::HashSet;
use std::num::NonZeroUsize;

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
    let search_k = NonZeroUsize::new(n * reader.n_trees() * 15)
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
        parameter.spectral_centroid,
        parameter.spectral_flatness,
        parameter.spectral_slope,
        parameter.spectral_rolloff,
        parameter.spectral_spread,
        // parameter.spectral_skewness,
        // parameter.spectral_kurtosis,
        parameter.chromagram[0],
        parameter.chromagram[1],
        parameter.chromagram[2],
        parameter.chromagram[3],
        parameter.chromagram[4],
        parameter.chromagram[5],
        parameter.chromagram[6],
        parameter.chromagram[7],
        parameter.chromagram[8],
        parameter.chromagram[9],
        parameter.chromagram[10],
        parameter.chromagram[11],
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
/// * `db` - A reference to the database connection.
/// * `db_conn` - The tuple containing the LMDB environment and the Arroy database.
///
/// # Returns
/// * `Result<(), Box<dyn std::error::Error>>` - A result indicating success or failure.
pub async fn sync_recommendation(
    db: &MainDbConnection,
    db_conn: &RecommendationDbConnection,
) -> Result<(), Box<dyn std::error::Error>> {
    let env = db_conn.env.clone();
    let arroy_db = db_conn.db;

    // Fetch all analysis data
    let analyses = crate::entities::media_analysis::Entity::find()
        .all(db)
        .await?;

    // Track existing IDs in the main database
    let mut existing_ids: HashSet<i32> = HashSet::new();
    for analysis in &analyses {
        existing_ids.insert(analysis.file_id);
    }

    // Open a write transaction for the recommendation database
    let mut wtxn = env.write_txn()?;
    let writer = Writer::<Euclidean>::new(arroy_db, 0, 17); // Assuming 17 dimensions for the analysis data

    // Insert or update analysis data in the recommendation database
    for analysis in analyses {
        let vector = vec![
            analysis.spectral_centroid.unwrap_or(0.0) as f32,
            analysis.spectral_flatness.unwrap_or(0.0) as f32,
            analysis.spectral_slope.unwrap_or(0.0) as f32,
            analysis.spectral_rolloff.unwrap_or(0.0) as f32,
            analysis.spectral_spread.unwrap_or(0.0) as f32,
            // analysis.spectral_skewness.unwrap_or(0.0) as f32,
            // analysis.spectral_kurtosis.unwrap_or(0.0) as f32,
            analysis.chroma0.unwrap_or(0.0) as f32,
            analysis.chroma1.unwrap_or(0.0) as f32,
            analysis.chroma2.unwrap_or(0.0) as f32,
            analysis.chroma3.unwrap_or(0.0) as f32,
            analysis.chroma4.unwrap_or(0.0) as f32,
            analysis.chroma5.unwrap_or(0.0) as f32,
            analysis.chroma6.unwrap_or(0.0) as f32,
            analysis.chroma7.unwrap_or(0.0) as f32,
            analysis.chroma8.unwrap_or(0.0) as f32,
            analysis.chroma9.unwrap_or(0.0) as f32,
            analysis.chroma10.unwrap_or(0.0) as f32,
            analysis.chroma11.unwrap_or(0.0) as f32,
        ];
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
