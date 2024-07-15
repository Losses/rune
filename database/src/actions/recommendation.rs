use arroy::distances::Euclidean;
use arroy::{Database as ArroyDatabase, Reader, Writer};
use heed::Env;
use rand::rngs::StdRng;
use rand::SeedableRng;
use sea_orm::entity::prelude::*;
use std::collections::HashSet;
use std::num::NonZeroUsize;

/// Get recommendations for a given item.
///
/// # Arguments
/// * `db_conn` - The tuple containing the LMDB environment and the Arroy database.
/// * `item_id` - The ID of the item for which to get recommendations.
/// * `n` - The number of recommendations to retrieve.
///
/// # Returns
/// * `Result<Vec<(usize, f32)>, Box<dyn std::error::Error>>` - A vector of recommended item IDs and their distances.
pub fn get_recommendation(
    db_conn: &(Env, ArroyDatabase<Euclidean>),
    item_id: usize,
    n: usize,
) -> Result<Vec<(u32, f32)>, Box<dyn std::error::Error>> {
    let (env, db) = db_conn;
    let rtxn = env.read_txn()?;
    let reader = Reader::<Euclidean>::open(&rtxn, 0, *db)?;
    let search_k = NonZeroUsize::new(n * reader.n_trees() * 15)
        .ok_or("Failed to create NonZeroUsize from search_k")?;
    
    let item_id: u32 = item_id.try_into()
        .map_err(|_| "Failed to convert item_id to u32")?;
    
    let results = reader
        .nns_by_item(&rtxn, item_id, n, Some(search_k), None)?
        .ok_or("No results found for the given item_id")?;
    
    Ok(results)
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
    db: &DatabaseConnection,
    db_conn: &(Env, ArroyDatabase<Euclidean>),
) -> Result<(), Box<dyn std::error::Error>> {
    let (env, arroy_db) = db_conn;

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
    let writer = Writer::<Euclidean>::new(arroy_db.clone(), 0, 17); // Assuming 17 dimensions for the analysis data

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
    let reader = Reader::<Euclidean>::open(&rtxn, 0, *arroy_db)?;
    for id in reader.item_ids() {
        if !existing_ids.contains(&(id as i32)) {
            let mut wtxn = env.write_txn()?;
            let writer = Writer::<Euclidean>::new(arroy_db.clone(), 0, 17);
            writer.del_item(&mut wtxn, id)?;
            wtxn.commit()?;
        }
    }

    Ok(())
}
