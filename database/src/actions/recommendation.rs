use anyhow::{Context, Result};
use sea_orm::entity::prelude::*;

use crate::actions::analysis::AggregatedAnalysisResult;
use crate::connection::{MainDbConnection, RecommendationDbConnection};
use crate::entities::{media_analysis, media_files};

use super::analysis::get_percentile_analysis_result;

/// The `n` vectors closest to `query` by Euclidean distance, nearest first.
///
/// The search is exact. Distances are the plain (not squared) Euclidean
/// distance, equal distances are ordered by file id. Returns fewer than `n`
/// results when there are not enough vectors.
fn nearest(vectors: &[(u32, [f32; 61])], query: &[f32; 61], n: usize) -> Vec<(u32, f32)> {
    if n == 0 || vectors.is_empty() {
        return Vec::new();
    }

    let mut candidates: Vec<(f32, u32)> = vectors
        .iter()
        .map(|(id, vector)| {
            let squared: f32 = vector
                .iter()
                .zip(query)
                .map(|(a, b)| (a - b) * (a - b))
                .sum();
            (squared, *id)
        })
        .collect();

    let order = |a: &(f32, u32), b: &(f32, u32)| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1));
    if n < candidates.len() {
        candidates.select_nth_unstable_by(n - 1, order);
        candidates.truncate(n);
    }
    candidates.sort_unstable_by(order);

    candidates
        .into_iter()
        .map(|(squared, id)| (id, squared.sqrt()))
        .collect()
}

/// Get recommendations for a given item.
///
/// # Arguments
/// * `recommend_db` - The in-memory vectors of all analysed tracks.
/// * `item_id` - The ID of the item for which to get recommendations.
/// * `n` - The number of recommendations to retrieve.
///
/// # Returns
/// * `Result<Vec<(u32, f32)>>` - Recommended item IDs and their distances,
///   nearest first. The item itself is included (at distance 0). Fails if the
///   item has no analysis data.
pub fn get_recommendation_by_file_id(
    recommend_db: &RecommendationDbConnection,
    item_id: i32,
    n: usize,
) -> Result<Vec<(u32, f32)>> {
    let item_id: u32 = item_id
        .try_into()
        .with_context(|| "Failed to convert item_id to u32")?;

    let vectors = recommend_db.vectors();
    let index = vectors
        .binary_search_by_key(&item_id, |(id, _)| *id)
        .ok()
        .with_context(|| "No results found for the given item_id")?;

    Ok(nearest(&vectors, &vectors[index].1, n))
}

/// Get recommendations for a feature vector.
///
/// # Arguments
/// * `recommend_db` - The in-memory vectors of all analysed tracks.
/// * `feature_vector` - The point to search around.
/// * `n` - The number of recommendations to retrieve.
///
/// # Returns
/// * `Result<Vec<(u32, f32)>>` - Recommended item IDs and their distances,
///   nearest first. Empty if there is no analysis data or `n` is 0.
pub fn get_recommendation_by_parameter(
    recommend_db: &RecommendationDbConnection,
    feature_vector: [f32; 61],
    n: usize,
) -> Result<Vec<(u32, f32)>> {
    Ok(nearest(&recommend_db.vectors(), &feature_vector, n))
}

/// Read all analysis data of `main_db` into the recommendation store.
pub(crate) async fn load_recommendation_vectors(
    main_db: &MainDbConnection,
    recommend_db: &RecommendationDbConnection,
) -> Result<()> {
    let analyzes = media_analysis::Entity::find()
        .all(main_db)
        .await
        .with_context(|| "Failed to read analysis data")?;

    let mut vectors = Vec::with_capacity(analyzes.len());
    for analysis in analyzes {
        let file_id: u32 = analysis.file_id.try_into()?;
        let parsed_result: AggregatedAnalysisResult = analysis.into();
        let vector: [f32; 61] = parsed_result.into();
        vectors.push((file_id, vector));
    }

    recommend_db.replace(vectors);
    Ok(())
}

/// Sync the recommendation store with the analysis data.
///
/// Reads all of `media_analysis` and replaces the in-memory vectors, so added
/// and removed analysis rows are both reflected.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `recommend_db` - The in-memory vectors of all analysed tracks.
///
/// # Returns
/// * `Result<()>` - A result indicating success or failure.
pub async fn sync_recommendation(
    main_db: &MainDbConnection,
    recommend_db: &RecommendationDbConnection,
) -> Result<()> {
    load_recommendation_vectors(main_db, recommend_db).await
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

#[cfg(test)]
mod tests {
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    use super::*;

    fn random_vectors(count: u32, seed: u64) -> Vec<(u32, [f32; 61])> {
        let mut rng = StdRng::seed_from_u64(seed);
        (1..=count)
            .map(|id| {
                let mut vector = [0f32; 61];
                for x in vector.iter_mut() {
                    *x = rng.gen_range(-1.0..1.0);
                }
                (id, vector)
            })
            .collect()
    }

    fn naive(vectors: &[(u32, [f32; 61])], query: &[f32; 61], n: usize) -> Vec<(u32, f32)> {
        let mut all: Vec<(u32, f32)> = vectors
            .iter()
            .map(|(id, v)| {
                let d: f32 = v.iter().zip(query).map(|(a, b)| (a - b) * (a - b)).sum();
                (*id, d.sqrt())
            })
            .collect();
        all.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        all.truncate(n);
        all
    }

    fn store(vectors: Vec<(u32, [f32; 61])>) -> RecommendationDbConnection {
        let db = RecommendationDbConnection::new();
        db.replace(vectors);
        db
    }

    #[test]
    fn matches_naive_reference_on_random_data() {
        let vectors = random_vectors(500, 1);
        let db = store(vectors.clone());
        let queries = random_vectors(20, 2);
        for (_, query) in queries {
            for n in [1, 10, 30, 499] {
                assert_eq!(
                    get_recommendation_by_parameter(&db, query, n).unwrap(),
                    naive(&vectors, &query, n)
                );
            }
        }
    }

    #[test]
    fn results_are_ascending_and_plain_euclidean() {
        let mut a = [0f32; 61];
        let mut b = [0f32; 61];
        a[0] = 3.0;
        b[0] = 1.0;
        let db = store(vec![(1, a), (2, b)]);
        let result = get_recommendation_by_parameter(&db, [0f32; 61], 2).unwrap();
        assert_eq!(result, vec![(2, 1.0), (1, 3.0)]);
    }

    #[test]
    fn equal_distances_are_ordered_by_file_id() {
        let db = store(vec![
            (9, [1.0; 61]),
            (3, [1.0; 61]),
            (5, [1.0; 61]),
            (1, [2.0; 61]),
        ]);
        let result = get_recommendation_by_parameter(&db, [1.0; 61], 3).unwrap();
        assert_eq!(
            result.iter().map(|r| r.0).collect::<Vec<_>>(),
            vec![3, 5, 9]
        );
        let result = get_recommendation_by_file_id(&db, 5, 10).unwrap();
        assert_eq!(
            result.iter().map(|r| r.0).collect::<Vec<_>>(),
            vec![3, 5, 9, 1]
        );
    }

    #[test]
    fn n_larger_than_count_returns_everything() {
        let db = store(random_vectors(7, 3));
        assert_eq!(
            get_recommendation_by_parameter(&db, [0f32; 61], 1000)
                .unwrap()
                .len(),
            7
        );
    }

    #[test]
    fn n_zero_and_empty_store_give_empty_results() {
        let db = store(random_vectors(7, 3));
        assert!(
            get_recommendation_by_parameter(&db, [0f32; 61], 0)
                .unwrap()
                .is_empty()
        );
        assert!(get_recommendation_by_file_id(&db, 1, 0).unwrap().is_empty());

        let empty = RecommendationDbConnection::new();
        assert!(
            get_recommendation_by_parameter(&empty, [0f32; 61], 10)
                .unwrap()
                .is_empty()
        );
        assert!(get_recommendation_by_file_id(&empty, 1, 10).is_err());
    }

    #[test]
    fn by_file_id_includes_the_item_itself_first() {
        let vectors = random_vectors(50, 4);
        let db = store(vectors.clone());
        let result = get_recommendation_by_file_id(&db, 7, 5).unwrap();
        assert_eq!(result[0], (7, 0.0));
        assert_eq!(result, naive(&vectors, &vectors[6].1, 5));
    }

    #[test]
    fn by_file_id_fails_for_unknown_or_negative_ids() {
        let db = store(random_vectors(5, 5));
        assert!(get_recommendation_by_file_id(&db, 99, 3).is_err());
        assert!(get_recommendation_by_file_id(&db, -1, 3).is_err());
    }

    #[test]
    fn replace_reflects_added_and_removed_vectors() {
        let db = store(vec![(1, [0.0; 61]), (2, [1.0; 61])]);
        assert_eq!(
            get_recommendation_by_parameter(&db, [0.0; 61], 10)
                .unwrap()
                .len(),
            2
        );

        db.replace(vec![(2, [1.0; 61]), (3, [0.0; 61]), (4, [0.5; 61])]);
        let ids: Vec<u32> = get_recommendation_by_parameter(&db, [0.0; 61], 10)
            .unwrap()
            .iter()
            .map(|r| r.0)
            .collect();
        assert_eq!(ids, vec![3, 4, 2]);

        db.replace(Vec::new());
        assert!(
            get_recommendation_by_parameter(&db, [0.0; 61], 10)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn replace_keeps_the_last_vector_of_a_duplicate_id() {
        let db = store(vec![(1, [5.0; 61]), (1, [0.0; 61])]);
        assert_eq!(db.vectors().len(), 1);
        assert_eq!(
            get_recommendation_by_parameter(&db, [0.0; 61], 1).unwrap(),
            vec![(1, 0.0)]
        );
    }
}
