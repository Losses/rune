//! This module provides data chunking capabilities based on HLC timestamps.
//! It implements an exponential decay algorithm to create variable-sized chunks
//! and offers functionality to break down existing chunks into smaller ones.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use blake3::Hasher;
use log::{debug, info, warn};
use sea_orm::{DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::hlc::{calculate_hash as calculate_record_hash, HLCModel, HLCRecord, HLC};

const MILLISECONDS_PER_DAY: u64 = 24 * 60 * 60 * 1000;

/// Represents metadata for a chunk of data ordered by HLC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataChunk {
    /// HLC timestamp of the first record in the chunk.
    pub start_hlc: HLC,
    /// HLC timestamp of the last record in the chunk.
    pub end_hlc: HLC,
    /// Number of records within this chunk.
    pub count: u64,
    /// BLAKE3 hash representing the content of the chunk.
    /// Calculated based on the hashes of individual records within the chunk.
    pub chunk_hash: String,
}

/// Represents a sub-chunk created by breaking down a parent chunk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubDataChunk {
    /// The actual data chunk metadata for this sub-chunk.
    pub chunk: DataChunk,
    /// The HLC timestamp of the first record in the parent chunk.
    pub parent_start_hlc: HLC,
    /// The HLC timestamp of the last record in the parent chunk.
    pub parent_end_hlc: HLC,
    /// The hash of the parent chunk.
    pub parent_chunk_hash: String,
}

/// Configuration options for the chunk generation algorithm.
#[derive(Debug, Clone)]
pub struct ChunkingOptions {
    /// Minimum number of records in a chunk.
    pub min_size: u64,
    /// Maximum number of records in a chunk.
    pub max_size: u64,
    /// Exponential decay factor (α). Higher values mean chunk sizes increase faster for older data.
    /// Recommended: 0.3 for frequently changing data, 0.6 for stable data.
    pub alpha: f64,
    /// The Node ID used for HLC comparison if needed (e.g., comparing with HLC::new).
    pub node_id: Uuid,
}

impl Default for ChunkingOptions {
    fn default() -> Self {
        ChunkingOptions {
            min_size: 100,        // Example default minimum size
            max_size: 10000,      // Default max size as suggested
            alpha: 0.4,           // A balanced default alpha
            node_id: Uuid::nil(), // Default to nil, should be set meaningfully
        }
    }
}

/// Calculates the combined BLAKE3 hash for a slice of records.
///
/// This function iterates through the provided records, calculates the individual
/// hash for each record using `calculate_record_hash`, and then computes a
/// final BLAKE3 hash over the concatenation of these individual hex hash strings.
/// This serves as the `chunk_hash`.
///
/// Note: This is a simplified approach. A true Merkle tree would involve
/// pairing and hashing recursively, which is more complex but offers proofs of inclusion.
///
/// # Arguments
///
/// * `records`: A slice of `Model` instances that implement `HLCRecord`.
///
/// # Returns
///
/// A `Result` containing the hex-encoded BLAKE3 hash string for the chunk,
/// or an error if hashing fails for any record.
pub fn calculate_chunk_hash<E>(records: &[E::Model]) -> Result<String>
where
    E: HLCRecord + EntityTrait,
    E::Model: HLCRecord + Send + Sync + Serialize, // Ensure Model implements HLCRecord and Serialize
{
    if records.is_empty() {
        // Define a hash for an empty chunk (e.g., hash of an empty string)
        let mut hasher = Hasher::new();
        hasher.update(b"");
        return Ok(hasher.finalize().to_hex().to_string());
    }

    let mut chunk_hasher = Hasher::new();
    for record_model in records {
        // Calculate individual record hash
        // We need HLCRecord trait implemented for Model or accessible via Entity
        // Assuming Model implements HLCRecord:
        let record_hash_hex = calculate_record_hash(record_model)
            .with_context(|| "Failed to calculate hash for record".to_string())?; // Adjust based on how unique_id is accessed if needed

        // Update chunk hasher with the hex string bytes of the record hash
        chunk_hasher.update(record_hash_hex.as_bytes());
    }

    Ok(chunk_hasher.finalize().to_hex().to_string())
}

/// Generates data chunks for an entity using the exponential decay algorithm.
///
/// Fetches data ordered by `updated_at_hlc` and groups it into chunks where
/// recent data has smaller chunks and older data has larger chunks, up to `max_size`.
///
/// # Type Parameters
///
/// * `E`: The SeaORM `EntityTrait` representing the table to chunk. Must also implement `HLCModel`.
///        Its associated `Model` must implement `HLCRecord`.
///
/// # Arguments
///
/// * `db`: A database connection.
/// * `options`: `ChunkingOptions` specifying min/max sizes and alpha.
/// * `start_hlc_exclusive`: Optional HLC. If provided, chunking starts *after* this HLC.
///                          If `None`, starts from the beginning.
///
/// # Returns
///
/// A `Result` containing a vector of `DataChunk` metadata, ordered by HLC,
/// or an error if database access or hashing fails.
pub async fn generate_data_chunks<E>(
    db: &DatabaseConnection,
    options: &ChunkingOptions,
    start_hlc_exclusive: Option<HLC>,
) -> Result<Vec<DataChunk>>
where
    E: HLCModel + EntityTrait + Sync + HLCRecord,
    E::Model: HLCRecord + Clone + Serialize + Send + Sync, // Model needs HLCRecord, Clone, Serialize
    <E as EntityTrait>::Model: Sync,
{
    info!(
        "Starting chunk generation for entity {:?} with options: {:?}",
        std::any::type_name::<E>(),
        options
    );

    let mut chunks = Vec::new();
    let mut current_hlc = start_hlc_exclusive.unwrap_or_else(|| HLC::new(options.node_id)); // Start from beginning or specified point

    // Find the latest HLC in the dataset to calculate age relative to the "present"
    let latest_record = E::find()
        .order_by_desc(E::updated_at_time_column())
        .order_by_desc(E::updated_at_version_column())
        .one(db)
        .await
        .context("Failed to query latest record HLC")?;

    let latest_hlc_timestamp = match latest_record {
        Some(record) => record
            .updated_at_hlc()
            .map(|h| h.timestamp)
            .unwrap_or_else(|| {
                warn!(
                    "Latest record has no HLC timestamp, using current time for age calculation."
                );
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64
            }),
        None => {
            info!(
                "No records found for entity {:?}. No chunks generated.",
                std::any::type_name::<E>()
            );
            return Ok(chunks); // No data, no chunks
        }
    };

    debug!(
        "Latest HLC timestamp for age calculation: {}",
        latest_hlc_timestamp
    );

    loop {
        // Calculate age factor based on the start of the potential *next* chunk
        let age_millis = latest_hlc_timestamp.saturating_sub(current_hlc.timestamp);
        let age_days = age_millis as f64 / MILLISECONDS_PER_DAY as f64;
        // Using ceil as per formula: `ceil(age_factor)`
        // We interpret `age_factor` directly as age in days here.
        let age_factor_ceil = age_days.ceil();

        // Calculate dynamic window size using exponential decay formula
        let desired_size = options.min_size as f64 * (1.0 + options.alpha).powf(age_factor_ceil);
        let window_size = (desired_size.round() as u64)
            .max(options.min_size)
            .min(options.max_size);

        debug!(
            "Current HLC: {}, Age (days): {:.2}, AgeFactorCeil: {}, DesiredSize: {:.2}, WindowSize: {}",
            current_hlc, age_days, age_factor_ceil, desired_size, window_size
        );

        // Fetch the next batch of records up to window_size
        // We need a function like get_data_after_hlc but with a limit.
        // Let's implement the query directly here.
        let records: Vec<E::Model> = E::find()
            .filter(E::gt(&current_hlc)?) // Find records strictly *after* current HLC
            .order_by_asc(E::updated_at_time_column())
            .order_by_asc(E::updated_at_version_column())
            .limit(window_size) // Limit the number of records fetched
            .all(db)
            .await
            .with_context(|| {
                format!(
                    "Failed to fetch next batch of records after HLC {}",
                    current_hlc
                )
            })?;

        if records.is_empty() {
            debug!(
                "No more records found after HLC {}. Chunk generation complete.",
                current_hlc
            );
            break; // No more records
        }

        let first_record = records.first().unwrap(); // Safe because records is not empty
        let last_record = records.last().unwrap(); // Safe because records is not empty

        let chunk_start_hlc = first_record
            .updated_at_hlc()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Record {:?} is missing 'updated_at_hlc'",
                    first_record.unique_id()
                )
            })?
            .clone(); // Assuming unique_id() is available on Model via HLCRecord trait

        let chunk_end_hlc = last_record
            .updated_at_hlc()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Record {:?} is missing 'updated_at_hlc'",
                    last_record.unique_id()
                )
            })?
            .clone();

        let chunk_count = records.len() as u64;

        // Calculate hash for the chunk
        let chunk_hash = calculate_chunk_hash::<E>(&records).with_context(|| {
            format!(
                "Failed to calculate hash for chunk [{}-{}], count {}",
                chunk_start_hlc, chunk_end_hlc, chunk_count
            )
        })?;

        let data_chunk = DataChunk {
            start_hlc: chunk_start_hlc.clone(),
            end_hlc: chunk_end_hlc.clone(),
            count: chunk_count,
            chunk_hash,
        };

        debug!("Generated chunk: {:?}", data_chunk);
        chunks.push(data_chunk);

        // Update current_hlc to the end of the chunk we just created for the next iteration
        current_hlc = chunk_end_hlc;

        // Safety break in case of unexpected loop conditions (optional)
        if chunks.len() > 1_000_000 {
            // Arbitrary large number
            warn!(
                "Chunk generation exceeded 1,000,000 chunks, breaking loop. Check logic or data."
            );
            break;
        }
    }

    info!(
        "Successfully generated {} chunks for entity {:?}",
        chunks.len(),
        std::any::type_name::<E>()
    );
    Ok(chunks)
}

/// Breaks a given `DataChunk` definition into smaller sub-chunks.
///
/// Fetches the actual data corresponding to the `parent_chunk`'s HLC range,
/// verifies the count and hash, and then divides the data into smaller chunks
/// of the specified `sub_chunk_size`.
///
/// # Type Parameters
///
/// * `E`: The SeaORM `EntityTrait` representing the table. Must also implement `HLCModel`.
///        Its associated `Model` must implement `HLCRecord`.
///
/// # Arguments
///
/// * `db`: A database connection.
/// * `parent_chunk`: The `DataChunk` metadata defining the range to break down.
/// * `sub_chunk_size`: The desired number of records for each sub-chunk.
///
/// # Returns
///
/// A `Result` containing a vector of `SubChunkInfo` structs, each describing a
/// sub-chunk derived from the `parent_chunk`. Returns an error if:
///   - Database fetching fails.
///   - The actual data found in the range does not match the `parent_chunk`'s count or hash.
///   - Hashing fails.
pub async fn break_data_chunk<E>(
    db: &DatabaseConnection,
    parent_chunk: &DataChunk,
    sub_chunk_size: u64,
) -> Result<Vec<SubDataChunk>>
where
    E: HLCModel + EntityTrait + Sync + HLCRecord,
    E::Model: HLCRecord + Clone + Serialize + Send + Sync, // Model needs HLCRecord, Clone, Serialize
    <E as EntityTrait>::Model: Sync,
{
    info!(
        "Breaking down chunk [{}-{}] (Count: {}, Hash: {}) into sub-chunks of size {}",
        parent_chunk.start_hlc,
        parent_chunk.end_hlc,
        parent_chunk.count,
        parent_chunk.chunk_hash,
        sub_chunk_size
    );

    if sub_chunk_size == 0 {
        bail!("Sub-chunk size cannot be zero");
    }

    // Fetch *all* records within the parent chunk's HLC range
    // Need to handle pagination if the parent chunk is very large, or use `all()` if feasible.
    // For simplicity, assuming `all()` is acceptable for a single chunk breakdown.
    // If parent_chunk.count is very large, this could OOM. Consider iterative fetching if needed.

    let records: Vec<E::Model> = E::find()
        .filter(E::between(&parent_chunk.start_hlc, &parent_chunk.end_hlc)?)
        .order_by_asc(E::updated_at_time_column())
        .order_by_asc(E::updated_at_version_column())
        // .limit(parent_chunk.count + 1) // Fetch slightly more to detect inconsistencies? Or trust count.
        .all(db)
        .await
        .with_context(|| {
            format!(
                "Failed to fetch records for parent chunk [{}-{}]",
                parent_chunk.start_hlc, parent_chunk.end_hlc
            )
        })?;

    debug!("Fetched {} records for parent chunk.", records.len());

    // --- Verification Step ---
    // 1. Verify Count
    if records.len() as u64 != parent_chunk.count {
        warn!(
            "Count mismatch for chunk [{}-{}]: Expected {}, Found {}. Data may have changed.",
            parent_chunk.start_hlc,
            parent_chunk.end_hlc,
            parent_chunk.count,
            records.len()
        );
        // Depending on requirements, you might want to error out or proceed with found data.
        // Erroring out is safer for consistency checks.
        bail!(
            "Data inconsistency detected: Record count mismatch for chunk [{}-{}] (Expected {}, Found {}).",
            parent_chunk.start_hlc, parent_chunk.end_hlc, parent_chunk.count, records.len()
        );
    }

    // Handle the case where the parent chunk was legitimately empty
    if records.is_empty() {
        // Check if parent chunk also reported empty
        if parent_chunk.count == 0 {
            // Calculate expected hash for empty chunk
            let expected_empty_hash = calculate_chunk_hash::<E>(&records)?;
            if parent_chunk.chunk_hash == expected_empty_hash {
                info!(
                    "Parent chunk [{}-{}] was empty and verified. No sub-chunks to generate.",
                    parent_chunk.start_hlc, parent_chunk.end_hlc
                );
                return Ok(Vec::new()); // Correctly verified empty chunk
            } else {
                bail!(
                     "Data inconsistency detected: Parent chunk reported 0 count, but hash mismatch (Expected empty hash '{}', Found '{}').",
                     expected_empty_hash, parent_chunk.chunk_hash
                 );
            }
        } else {
            // This case should have been caught by the count check above, but for robustness:
            bail!(
                "Data inconsistency detected: Parent chunk expected {} records, but found 0.",
                parent_chunk.count
            );
        }
    }

    // 2. Verify Hash (only if records were found)
    let calculated_parent_hash = calculate_chunk_hash::<E>(&records).with_context(|| {
        format!(
            "Failed to recalculate hash for parent chunk [{}-{}] verification",
            parent_chunk.start_hlc, parent_chunk.end_hlc
        )
    })?;

    if calculated_parent_hash != parent_chunk.chunk_hash {
        warn!(
            "Hash mismatch for chunk [{}-{}]: Expected {}, Calculated {}. Data may have changed.",
            parent_chunk.start_hlc,
            parent_chunk.end_hlc,
            parent_chunk.chunk_hash,
            calculated_parent_hash
        );
        bail!(
            "Data inconsistency detected: Hash mismatch for chunk [{}-{}] (Expected '{}', Calculated '{}').",
            parent_chunk.start_hlc, parent_chunk.end_hlc, parent_chunk.chunk_hash, calculated_parent_hash
        );
    }

    debug!("Parent chunk data verified successfully.");

    // --- Sub-chunk Creation ---
    let mut sub_chunks_info = Vec::new();
    for sub_chunk_records in records.chunks(sub_chunk_size as usize) {
        if sub_chunk_records.is_empty() {
            continue; // Should not happen with `records.chunks` unless input was empty
        }

        let sub_first_record = sub_chunk_records.first().unwrap();
        let sub_last_record = sub_chunk_records.last().unwrap();

        let sub_start_hlc = sub_first_record
            .updated_at_hlc()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Sub-chunk record {:?} missing 'updated_at_hlc'",
                    sub_first_record.unique_id()
                )
            })?
            .clone();
        let sub_end_hlc = sub_last_record
            .updated_at_hlc()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Sub-chunk record {:?} missing 'updated_at_hlc'",
                    sub_last_record.unique_id()
                )
            })?
            .clone();
        let sub_count = sub_chunk_records.len() as u64;

        let sub_chunk_hash = calculate_chunk_hash::<E>(sub_chunk_records).with_context(|| {
            format!(
                "Failed to calculate hash for sub-chunk [{}-{}]",
                sub_start_hlc, sub_end_hlc
            )
        })?;

        let sub_data_chunk = DataChunk {
            start_hlc: sub_start_hlc,
            end_hlc: sub_end_hlc,
            count: sub_count,
            chunk_hash: sub_chunk_hash,
        };

        sub_chunks_info.push(SubDataChunk {
            chunk: sub_data_chunk,
            parent_start_hlc: parent_chunk.start_hlc.clone(),
            parent_end_hlc: parent_chunk.end_hlc.clone(),
            parent_chunk_hash: parent_chunk.chunk_hash.clone(),
        });
    }

    info!(
        "Successfully broke down parent chunk [{}-{}] into {} sub-chunks.",
        parent_chunk.start_hlc,
        parent_chunk.end_hlc,
        sub_chunks_info.len()
    );
    Ok(sub_chunks_info)
}

// --- Helper: Adjust HLCRecord Trait and calculate_hash (Conceptual) ---
// It's assumed that your HLCRecord trait methods (`updated_at_hlc`, `unique_id`, `data_for_hashing`)
// are implemented on the `Model` struct associated with your `Entity`.
// If `HLCRecord` is implemented on the `Entity` struct itself, you might need adjustments like:
/*
pub trait HLCRecord: EntityTrait + Sized + Send + Sync + 'static
where
    <Self as EntityTrait>::Model: Serialize + Send + Sync,
{
    // Static methods accessing Model data need the model passed in
    fn model_updated_at_hlc(model: &Self::Model) -> Option<HLC>;
    fn model_unique_id(model: &Self::Model) -> i32; // Or appropriate PK type
    fn model_data_for_hashing(model: &Self::Model) -> serde_json::Value;

    // Static methods defining columns remain the same
    fn get_primary_key_column() -> Self::Column; // Example name
    fn get_updated_at_hlc_time_column() -> Self::Column;
    fn get_updated_at_hlc_version_column() -> Self::Column;

     // Default implementations can use the static methods
    fn to_summary(model: &Self::Model) -> serde_json::Value {
        Self::model_data_for_hashing(model)
    }
    fn full_data(model: &Self::Model) -> serde_json::Value {
        Self::model_data_for_hashing(model)
    }
}

// calculate_record_hash would then use the static methods:
pub fn calculate_record_hash<E: HLCRecord>(model: &E::Model) -> Result<String> {
    let data = E::model_data_for_hashing(model); // Call static method
    let json_string = serde_json::to_string(&data).context("Failed to serialize data to JSON")?;
    let mut hasher = Hasher::new();
    hasher.update(json_string.as_bytes());
    Ok(hasher.finalize().to_hex().to_string())
}

// calculate_chunk_hash remains largely the same but calls the revised calculate_record_hash
pub fn calculate_chunk_hash<E>(records: &[E::Model]) -> Result<String>
where
    E: HLCRecord + EntityTrait,
    E::Model: Send + Sync + Serialize,
{
    // ... loop ...
        let record_hash_hex = calculate_record_hash::<E>(record_model) // Pass Entity type E
             .with_context(|| format!("Failed to calculate hash for record"))?;
    // ...
}
*/
// For this implementation, we assume the simpler case where HLCRecord is directly implemented
// or accessible on E::Model. Ensure your `calculate_record_hash` function signature and
// usage match your specific `HLCRecord` trait definition.

#[cfg(test)]
mod tests {
    // Mocking SeaORM and database interactions is complex.
    // These tests would ideally involve setting up an in-memory DB (like SQLite)
    // and populating it with test data.

    // Placeholder for conceptual tests:

    // #[tokio::test]
    // async fn test_generate_simple_chunks() {
    //     // 1. Setup mock DB connection
    //     // 2. Define MockEntity and implement HLCModel, HLCRecord
    //     // 3. Insert mock data with varying HLC timestamps
    //     // 4. Define ChunkingOptions
    //     // 5. Call generate_data_chunks
    //     // 6. Assert the number and properties of generated chunks
    // }

    // #[tokio::test]
    // async fn test_break_chunk() {
    //     // 1. Setup mock DB connection
    //     // 2. Define MockEntity and implement HLCModel, HLCRecord
    //     // 3. Insert mock data within a specific HLC range
    //     // 4. Manually create a DataChunk representing this range (calculate expected hash)
    //     // 5. Define sub_chunk_size
    //     // 6. Call break_data_chunk with the created DataChunk
    //     // 7. Assert the number and properties of generated SubChunkInfo
    //     // 8. Assert parent info in SubChunkInfo matches the input DataChunk
    // }

    // #[tokio::test]
    // async fn test_break_chunk_verification_fail_count() {
    //     // 1. Setup mock DB
    //     // 2. Insert N records
    //     // 3. Create DataChunk metadata expecting N+1 records
    //     // 4. Call break_data_chunk
    //     // 5. Assert that it returns an error indicating count mismatch
    // }

    // #[tokio::test]
    // async fn test_break_chunk_verification_fail_hash() {
    //     // 1. Setup mock DB
    //     // 2. Insert N records
    //     // 3. Create DataChunk metadata with correct count but wrong hash
    //     // 4. Call break_data_chunk
    //     // 5. Assert that it returns an error indicating hash mismatch
    // }
}
