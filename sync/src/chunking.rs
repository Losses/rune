//! This module provides data chunking capabilities based on HLC timestamps.
//! It implements an exponential decay algorithm to create variable-sized chunks
//! and offers functionality to break down existing chunks into smaller ones.

use std::{
    cmp,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use blake3::Hasher;
use log::{debug, info, warn};
use sea_orm::{DatabaseConnection, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Use the refined HLC module functions/traits
use crate::hlc::{calculate_hash as calculate_record_hash, HLCModel, HLCQuery, HLCRecord, HLC};

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
    /// Minimum number of records in a chunk. Must be > 0.
    pub min_size: u64,
    /// Maximum number of records in a chunk. Must be >= min_size.
    pub max_size: u64,
    /// Exponential decay factor (α). Higher values mean chunk sizes increase faster for older data.
    /// Recommended: 0.3 for frequently changing data, 0.6 for stable data. Range [0, inf).
    pub alpha: f64,
    /// The Node ID used for HLC comparison if needed (e.g., comparing with HLC::new).
    pub node_id: Uuid,
}

/// Scenario: Frequently updated data (e.g., collaborative documents, user status, shared lists)
/// primarily accessed/synced by mobile devices or clients on potentially unreliable/low-bandwidth
/// networks.
///
/// Goal: Prioritize small chunks for recent data to minimize sync payload for frequent, small
/// changes. Keep maximum chunk size manageable for constrained devices/networks.
///
/// Rationale: Low min_size and alpha ensure high granularity for recent and moderately recent
/// changes. Low max_size prevents large downloads/processing burdens on less capable clients or
/// poor networks.
pub fn high_frequency_mobile_preset(node_id: Uuid) -> ChunkingOptions {
    ChunkingOptions {
        min_size: 50,    // Very small chunks for the latest data
        max_size: 2_000, // Relatively small max size for older data
        alpha: 0.25,     // Slow growth rate, keeps chunks smaller for longer
        node_id,
    }
}

/// Scenario: Primarily append-only data (e.g., logs, event streams, analytics) managed on backend
/// systems or synced between servers with good network connectivity. Old data rarely or never
/// changes.
///
/// Goal: Efficiently handle large volumes of historical data by grouping it into large chunks, while
/// still having reasonable chunks for recent additions. Minimize the total number of chunks for
/// history.
///
/// Rationale: Higher min_size suits batch appends. High alpha rapidly increases chunk size for older,
/// stable data. High max_size reduces the overall chunk count for deep history, optimizing
/// storage/transfer between capable systems.
pub fn append_optimized_backend_preset(node_id: Uuid) -> ChunkingOptions {
    ChunkingOptions {
        min_size: 250,    // Moderate size for recent batches of appends
        max_size: 20_000, // Allow very large chunks for stable historical data
        alpha: 0.7,       // Fast growth rate, quickly merges old data
        node_id,
    }
}

/// Scenario: General-purpose application data (e.g., project management items, settings,
/// reference data) synced with web or desktop clients on typical broadband networks. Data might see
/// occasional updates even when older.
///
/// Goal: A balanced approach offering good granularity for recent changes without creating excessive
/// numbers of chunks or overly large ones for history.
///
/// Rationale: A middle-ground configuration. min_size=100 is common. max_size=10000 aligns with the
/// original suggestion. alpha=0.45 provides a noticeable increase for older data but doesn't jump
/// to the max size immediately.
pub fn balanced_web_desktop_preset(node_id: Uuid) -> ChunkingOptions {
    ChunkingOptions {
        min_size: 100,    // Standard small size for recent items
        max_size: 10_000, // Standard max size limit
        alpha: 0.45,      // Moderate growth rate
        node_id,
    }
}

/// Scenario: Focused on optimizing the first time a client syncs a large existing dataset, potentially
/// on a good network. Less concerned about subsequent incremental syncs (though still functional).
///
/// Goal: Reduce the number of round trips/requests needed for the initial bulk download by using larger
/// chunks more quickly.
///
/// Rationale: Larger min_size and relatively high alpha/max_size reduce the total chunk count,
/// potentially speeding up a full dataset download where bandwidth isn't the primary bottleneck. Might
/// be less optimal for frequent small updates later.
pub fn initial_sync_optimized(node_id: Uuid) -> ChunkingOptions {
    ChunkingOptions {
        min_size: 500,    // Start with larger chunks
        max_size: 15_000, // Allow large historical chunks
        alpha: 0.6,       // Grow fairly quickly
        node_id,
    }
}

impl ChunkingOptions {
    /// Creates default chunking options.
    pub fn default(node_id: Uuid) -> Self {
        ChunkingOptions {
            min_size: 100,    // Small enough for reasonable recent granularity
            max_size: 10_000, // A widely accepted upper limit, prevents excessive size
            alpha: 0.4,       // Moderate growth, balances recency vs history
            node_id,
        }
    }

    /// Validates the chunking options.
    pub fn validate(&self) -> Result<()> {
        if self.min_size == 0 {
            bail!("min_size must be greater than 0");
        }
        if self.max_size < self.min_size {
            bail!("max_size must be greater than or equal to min_size");
        }
        if self.alpha < 0.0 {
            bail!("alpha must be non-negative");
        }
        Ok(())
    }
}

/// Calculates the combined BLAKE3 hash for a slice of HLCRecord models.
///
/// Hashes the canonical JSON representation of each record's `data_for_hashing`
/// and then computes a final BLAKE3 hash over the concatenation of these
/// individual hex hash strings.
///
/// # Arguments
///
/// * `records`: A slice of `Model` instances that implement `HLCRecord`.
///
/// # Returns
///
/// A `Result` containing the hex-encoded BLAKE3 hash string for the chunk.
pub fn calculate_chunk_hash<Model>(records: &[Model]) -> Result<String>
where
    Model: HLCRecord + Send + Sync, // Model implements HLCRecord
{
    if records.is_empty() {
        // Define a hash for an empty chunk (e.g., hash of an empty string)
        // Consistent definition is important.
        let hasher = Hasher::new();
        // hasher.update(b""); // Hash of empty string
        // Or hash of a specific marker? Let's stick with empty string for simplicity.
        return Ok(hasher.finalize().to_hex().to_string());
        // Alternative: Blake3 hash of empty input is af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262
        // return Ok("af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262".to_string());
    }

    let mut chunk_hasher = Hasher::new();
    for record_model in records {
        // Calculate individual record hash using the function from the hlc module
        let record_hash_hex = calculate_record_hash(record_model).with_context(|| {
            format!(
                "Failed to calculate hash for record ID {}", // Use unique_id for better context
                record_model.unique_id()
            )
        })?;

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
/// * `E`: The SeaORM `EntityTrait`. Its associated `Model` must implement `HLCRecord`.
///       `E` itself must implement `HLCModel`.
///
/// # Arguments
///
/// * `db`: A database connection.
/// * `options`: `ChunkingOptions` specifying min/max sizes and alpha.
/// * `start_hlc_exclusive`: Optional HLC. If provided, chunking starts *after* this HLC.
///
/// # Returns
///
/// A `Result` containing a vector of `DataChunk` metadata, ordered by HLC.
pub async fn generate_data_chunks<E>(
    db: &DatabaseConnection,
    options: &ChunkingOptions,
    start_hlc_exclusive: Option<HLC>,
) -> Result<Vec<DataChunk>>
where
    E: HLCModel + EntityTrait + Sync, // E needs HLCModel for queries
    E::Model: HLCRecord + Clone + Send + Sync, // Model needs HLCRecord
    <E as EntityTrait>::Model: Sync,
{
    options.validate()?; // Validate options first

    info!(
        "Starting chunk generation for entity {:?} with options: {:?}",
        std::any::type_name::<E>(),
        options
    );

    let mut chunks = Vec::new();
    // Start from HLC(0,0,node) or the specified start point
    let mut current_hlc = start_hlc_exclusive.unwrap_or_else(|| HLC::new(options.node_id));

    // Find the latest HLC in the dataset to calculate age relative to the "present"
    let latest_record_opt: Option<E::Model> = E::find()
        .order_by_hlc_desc::<E>()
        .one(db)
        .await
        .context("Failed to query latest record HLC")?;

    let latest_hlc_timestamp = match latest_record_opt {
        Some(record) => record
            .updated_at_hlc()
            .map(|h| h.timestamp)
            .unwrap_or_else(|| {
                // This case should ideally not happen if HLCs are mandatory on update
                warn!(
                    "Latest record {} has no HLC timestamp, using current time for age calculation.",
                    record.unique_id()
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

    // Ensure latest_hlc_timestamp is non-zero if data exists, otherwise age calculation might be weird.
    // If the latest timestamp is 0 (e.g., initial HLC), use current time? Or is 0 okay?
    // Let's assume 0 is okay, age will be large.
    // If latest_hlc_timestamp is 0 and current_hlc.timestamp is also 0, age is 0.
    let effective_latest_timestamp = if latest_hlc_timestamp == 0 {
        warn!("Latest HLC timestamp is 0, age calculation might result in larger initial chunks.");
        latest_hlc_timestamp // Proceed with 0
    } else {
        latest_hlc_timestamp
    };

    debug!(
        "Latest HLC timestamp for age calculation: {}",
        effective_latest_timestamp
    );

    let mut safety_count = 0;
    const MAX_ITERATIONS: u32 = 1_000_000; // Safety break

    loop {
        safety_count += 1;
        if safety_count > MAX_ITERATIONS {
            warn!(
                "Chunk generation exceeded {} iterations, breaking loop. Check logic or data.",
                MAX_ITERATIONS
            );
            // Consider returning an error instead?
            // return Err(anyhow::anyhow!("Chunk generation exceeded maximum iterations ({})", MAX_ITERATIONS));
            break;
        }

        // Calculate age factor based on the start HLC of the potential *next* chunk
        // Age is difference between latest update anywhere and the start of this chunk.
        let age_millis = effective_latest_timestamp.saturating_sub(current_hlc.timestamp);
        let age_days = age_millis as f64 / MILLISECONDS_PER_DAY as f64;

        // Ensure age_factor doesn't become negative if clock sync caused latest_ts < current_ts
        let non_negative_age_days = age_days.max(0.0);

        // Using ceil as per formula: `ceil(age_factor)`
        // We interpret `age_factor` directly as age in days here.
        let age_factor_ceil = non_negative_age_days.ceil();

        // Calculate dynamic window size using exponential decay formula
        // size = min_size * (1 + alpha) ^ ceil(age_days)
        let desired_size = options.min_size as f64 * (1.0 + options.alpha).powf(age_factor_ceil);

        // Clamp the size between min_size and max_size
        let window_size = cmp::min(
            cmp::max(desired_size.round() as u64, options.min_size),
            options.max_size,
        );

        debug!(
            "Current HLC: {}, Age (days): {:.2} (raw {:.2}), AgeFactorCeil: {}, DesiredSize: {:.2}, WindowSize: {}",
            current_hlc, non_negative_age_days, age_days, age_factor_ceil, desired_size, window_size
        );

        // Fetch the next batch of records strictly *after* current_hlc, up to window_size
        let records: Vec<E::Model> = E::find()
            .filter(E::gt(&current_hlc)?) // Use HLCModel::gt for correct comparison
            .order_by_hlc_asc::<E>() // Order consistently
            .limit(window_size) // Limit results
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
            break; // No more records to process
        }

        // Records vector is guaranteed non-empty here
        let first_record = records.first().unwrap();
        let last_record = records.last().unwrap();

        // Extract HLCs from the records themselves
        let chunk_start_hlc = first_record
            .updated_at_hlc()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Record {} is missing 'updated_at_hlc' (start of chunk)",
                    first_record.unique_id()
                )
            })?
            .clone();

        let chunk_end_hlc = last_record
            .updated_at_hlc()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Record {} is missing 'updated_at_hlc' (end of chunk)",
                    last_record.unique_id()
                )
            })?
            .clone();

        // Ensure HLC ordering within the fetched batch (should be guaranteed by query order)
        if chunk_start_hlc > chunk_end_hlc {
            // This indicates a potential issue with query ordering or HLC data corruption
            warn!("Inconsistent HLC order within fetched batch: Start {} > End {}. Record IDs: {} to {}",
                 chunk_start_hlc, chunk_end_hlc, first_record.unique_id(), last_record.unique_id());
            // Depending on requirements, maybe bail or log and continue? Bailing is safer.
            bail!("Detected inconsistent HLC order within fetched batch for chunking.");
        }

        let chunk_count = records.len() as u64;

        // Calculate hash for the chunk using the models fetched
        let chunk_hash = calculate_chunk_hash::<E::Model>(&records).with_context(|| {
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

        // Update current_hlc to the HLC of the *last* record processed in this chunk
        current_hlc = chunk_end_hlc;
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
/// * `E`: The SeaORM `EntityTrait`. Its associated `Model` must implement `HLCRecord`.
///       `E` itself must implement `HLCModel`.
///
/// # Arguments
///
/// * `db`: A database connection.
/// * `parent_chunk`: The `DataChunk` metadata defining the range to break down.
/// * `sub_chunk_size`: The desired number of records for each sub-chunk. Must be > 0.
///
/// # Returns
///
/// A `Result` containing a vector of `SubDataChunk` structs, or an error if verification fails.
pub async fn break_data_chunk<E>(
    db: &DatabaseConnection,
    parent_chunk: &DataChunk,
    sub_chunk_size: u64,
) -> Result<Vec<SubDataChunk>>
where
    E: HLCModel + EntityTrait + Sync,          // E needs HLCModel
    E::Model: HLCRecord + Clone + Send + Sync, // Model needs HLCRecord
    <E as EntityTrait>::Model: Sync,
{
    info!(
        "Breaking down chunk [{}-{}] (Count: {}, Hash: {:.8}) into sub-chunks of size {}",
        parent_chunk.start_hlc,
        parent_chunk.end_hlc,
        parent_chunk.count,
        parent_chunk.chunk_hash, // Log only prefix for brevity
        sub_chunk_size
    );

    if sub_chunk_size == 0 {
        bail!("Sub-chunk size cannot be zero");
    }
    // Ensure parent chunk definition itself is ordered correctly
    if parent_chunk.start_hlc > parent_chunk.end_hlc && parent_chunk.count > 0 {
        warn!(
            "Parent chunk has start_hlc > end_hlc but count > 0: [{}-{}] count {}",
            parent_chunk.start_hlc, parent_chunk.end_hlc, parent_chunk.count
        );
        // Bail? Or proceed assuming the range might be valid despite display order? Let's bail.
        bail!("Invalid parent chunk definition: start_hlc > end_hlc with non-zero count.");
    }

    // Handle the count == 0 case BEFORE attempting database query
    if parent_chunk.count == 0 {
        debug!("Parent chunk count is 0. Verifying empty hash directly.");
        // Verify hash against the expected empty hash.
        // Pass an empty slice to calculate_chunk_hash to get the canonical empty hash.
        let expected_empty_hash = calculate_chunk_hash::<E::Model>(&[])?; // Use empty slice
        if parent_chunk.chunk_hash != expected_empty_hash {
            bail!(
                     "Data inconsistency detected: Parent chunk reported 0 count, but hash mismatch (Expected empty hash '{}', Found '{}').",
                     expected_empty_hash, parent_chunk.chunk_hash
                 );
        }
        // If hash matches for empty chunk, verification passed.
        debug!("Parent chunk was empty and verified successfully.");
        return Ok(Vec::new()); // No sub-chunks for an empty parent. Return early.
    }

    // Fetch Records for Verification
    // Fetch *all* records within the parent chunk's HLC range (inclusive)
    // Use HLCModel::between for the correct range query.
    // Order consistently to match potential hash calculation order.
    // NOTE: This loads the entire parent chunk into memory. For extremely large parent chunks,
    // an iterative approach might be needed, but that complicates hash verification.
    let records: Vec<E::Model> = E::find()
        .filter(E::between(&parent_chunk.start_hlc, &parent_chunk.end_hlc)?)
        .order_by_hlc_asc::<E>()
        // .limit(parent_chunk.count + 1) // Fetch one extra to detect *more* records than expected?
        // Let's stick to fetching based on HLC range only for now. Count/Hash verification handles mismatches.
        .all(db)
        .await
        .with_context(|| {
            format!(
                "Failed to fetch records for parent chunk verification [{}-{}]",
                parent_chunk.start_hlc, parent_chunk.end_hlc
            )
        })?;

    debug!(
        "Fetched {} records for parent chunk verification.",
        records.len()
    );

    // Verification Step
    // 1. Verify Count
    if records.len() as u64 != parent_chunk.count {
        warn!(
            "Count mismatch for chunk [{}-{}]: Expected {}, Found {}. Data may have changed since chunk definition.",
            parent_chunk.start_hlc,
            parent_chunk.end_hlc,
            parent_chunk.count,
            records.len()
        );
        bail!(
            "Data inconsistency detected: Record count mismatch for chunk [{}-{}] (Expected {}, Found {}).",
            parent_chunk.start_hlc, parent_chunk.end_hlc, parent_chunk.count, records.len()
        );
    }

    // 2. Verify Hash (only if count matches and is non-zero)
    if parent_chunk.count > 0 {
        // Records vec is non-empty here because count > 0 and count matches len()
        let calculated_parent_hash =
            calculate_chunk_hash::<E::Model>(&records).with_context(|| {
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
    } else {
        // Parent chunk count is 0, verify hash against the expected empty hash.
        let expected_empty_hash = calculate_chunk_hash::<E::Model>(&records)?; // records is empty here
        if parent_chunk.chunk_hash != expected_empty_hash {
            bail!(
                 "Data inconsistency detected: Parent chunk reported 0 count, but hash mismatch (Expected empty hash '{}', Found '{}').",
                 expected_empty_hash, parent_chunk.chunk_hash
             );
        }
        // If hash matches for empty chunk, verification passed.
        debug!("Parent chunk was empty and verified successfully.");
        return Ok(Vec::new()); // No sub-chunks for an empty parent.
    }

    debug!(
        "Parent chunk data verified successfully (Count: {}, Hash: {:.8}).",
        parent_chunk.count, parent_chunk.chunk_hash
    );

    // Sub-chunk Creation
    let mut sub_chunks_info = Vec::new();
    // Use standard library's `chunks` method on the verified `records` vector
    for sub_chunk_records_slice in records.chunks(sub_chunk_size as usize) {
        // sub_chunk_records_slice is &[E::Model]

        // Should not be empty unless original records was empty (handled above)
        if sub_chunk_records_slice.is_empty() {
            continue;
        }

        let sub_first_record = sub_chunk_records_slice.first().unwrap();
        let sub_last_record = sub_chunk_records_slice.last().unwrap();

        // Extract HLCs from the actual records in the sub-chunk
        let sub_start_hlc = sub_first_record
            .updated_at_hlc()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Sub-chunk record {} missing 'updated_at_hlc'",
                    sub_first_record.unique_id()
                )
            })?
            .clone();
        let sub_end_hlc = sub_last_record
            .updated_at_hlc()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Sub-chunk record {} missing 'updated_at_hlc'",
                    sub_last_record.unique_id()
                )
            })?
            .clone();
        let sub_count = sub_chunk_records_slice.len() as u64;

        // Calculate hash for this specific sub-chunk
        let sub_chunk_hash = calculate_chunk_hash::<E::Model>(sub_chunk_records_slice)
            .with_context(|| {
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

#[cfg(test)]
pub mod tests {
    use chrono::{DateTime, TimeZone, Utc};
    use sea_orm::{
        ActiveModelBehavior, ActiveModelTrait, ConnectionTrait, Database, DbBackend, DbConn, DbErr,
        DerivePrimaryKey, DeriveRelation, EnumIter, PrimaryKeyTrait, Schema, Set,
    };

    use super::*;

    use crate::hlc::HLC;

    // Test-Specific Mock Entity Definition (Using TEXT for Timestamp)
    // This module defines the Entity, Model, and ActiveModel specifically for tests,
    // ensuring the timestamp is stored as an RFC3339 string (TEXT column).
    pub mod test_model_def {
        use super::*; // Inherit imports from parent test module
        use crate::hlc::{HLCModel, HLCRecord, HLC};
        use sea_orm::DeriveEntityModel;
        use serde_json::json;
        use uuid::Uuid;

        #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
        #[sea_orm(table_name = "mock_tasks")]
        pub struct Model {
            #[sea_orm(primary_key)]
            pub id: i32,
            pub content: String,
            #[sea_orm(column_type = "Text")] // Store timestamp as TEXT (RFC3339 string)
            pub updated_at_hlc_ts: String,
            pub updated_at_hlc_v: i32, // SeaORM prefers i32 for u32 typically
            pub updated_at_hlc_nid: Uuid,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {} // No relations needed for these tests
        impl ActiveModelBehavior for ActiveModel {} // Standard SeaORM requirement

        // Implement HLCRecord for the Test Model
        // This implementation handles the conversion between the RFC3339 string
        // stored in the DB and the HLC struct (with u64 ms timestamp) used internally.
        impl HLCRecord for Model {
            fn created_at_hlc(&self) -> Option<HLC> {
                None // Not used in these tests
            }

            fn updated_at_hlc(&self) -> Option<HLC> {
                // Parse the RFC3339 string back to u64 milliseconds since UNIX_EPOCH
                match DateTime::parse_from_rfc3339(&self.updated_at_hlc_ts) {
                    Ok(dt) => Some(HLC {
                        timestamp: dt.timestamp_millis() as u64, // Convert to u64 millis
                        version: self.updated_at_hlc_v as u32,   // Convert back to u32
                        node_id: self.updated_at_hlc_nid,
                    }),
                    Err(e) => {
                        // Log error or handle appropriately if parsing fails
                        eprintln!(
                            "Error parsing HLC timestamp string '{}': {}",
                            self.updated_at_hlc_ts, e
                        );
                        None // Return None if parsing fails
                    }
                }
            }

            fn unique_id(&self) -> String {
                self.id.to_string()
            }

            fn data_for_hashing(&self) -> serde_json::Value {
                // Define which fields contribute to the record's content hash
                // Exclude HLC fields themselves if hash represents content state only.
                json!({ "id": self.id, "content": self.content })
            }
        }

        // Implement HLCModel for the Test Entity
        // Maps HLC components to database columns for HLC-based queries.
        impl HLCModel for Entity {
            fn updated_at_time_column() -> Self::Column {
                Column::UpdatedAtHlcTs // The TEXT column holding the timestamp string
            }
            fn updated_at_version_column() -> Self::Column {
                Column::UpdatedAtHlcV // The version column
            }
            fn unique_id_column() -> Self::Column {
                Column::Id
            }
            // node_id column not currently required by HLCModel trait for queries
        }
    }

    // Use the test-specific model definitions
    // Bring the types from the dedicated test module into the main test scope.
    use test_model_def::{ActiveModel, Entity, Model};

    // Test Database Setup
    pub async fn setup_db() -> Result<DbConn, DbErr> {
        // Connect to an in-memory SQLite database for isolated tests
        let db = Database::connect("sqlite::memory:").await?;
        let schema = Schema::new(DbBackend::Sqlite);

        // Create the table using the test-specific Entity definition
        // This ensures the `updated_at_hlc_ts` column has the correct TEXT type.
        let create_table_stmt = schema.create_table_from_entity(Entity); // Use test_model_def::Entity
        db.execute(db.get_database_backend().build(&create_table_stmt))
            .await?;

        Ok(db)
    }

    // Timestamp Conversion Helper
    // Converts u64 milliseconds since epoch to RFC3339 string.
    // Required by `insert_task`. Assumed to exist in `crate::hlc` in the main code.
    fn hlc_timestamp_millis_to_rfc3339(millis: u64) -> Result<String> {
        // Create a NaiveDateTime from seconds and nanoseconds
        let seconds = (millis / 1000) as i64;
        let nanos = (millis % 1000 * 1_000_000) as u32; // Millis to Nanos
                                                        // Use Utc.timestamp_opt to handle potential out-of-range values gracefully
        match Utc.timestamp_opt(seconds, nanos) {
            chrono::LocalResult::Single(dt) => {
                // Format with full nanosecond precision and UTC offset ('Z' or +00:00)
                // chrono's default `to_rfc3339_opts` with SecondsFormat::Nanos includes enough precision.
                // The `FixedOffset` timezone ensures the '+00:00' suffix for consistency.
                Ok(dt.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
            }
            chrono::LocalResult::None => {
                bail!(
                    "Invalid timestamp milliseconds ({}): resulted in no valid DateTime",
                    millis
                )
            }
            chrono::LocalResult::Ambiguous(_, _) => {
                // This shouldn't happen with UTC timestamps
                bail!(
                    "Invalid timestamp milliseconds ({}): resulted in ambiguous DateTime",
                    millis
                )
            }
        }
    }

    // Test Data Insertion Helper
    // Inserts a task into the database, converting the HLC timestamp to RFC3339 string.
    pub async fn insert_task(
        db: &DbConn,
        id: i32,
        content: &str,
        hlc: &HLC,
    ) -> Result<Model, DbErr> {
        // Convert HLC timestamp (u64 ms) to RFC3339 string for storage
        let hlc_ts_str = hlc_timestamp_millis_to_rfc3339(hlc.timestamp)
            // Convert the anyhow::Error from the helper to DbErr for compatibility
            .map_err(|e| DbErr::Custom(format!("Failed to format HLC timestamp: {}", e)))?;

        // Create an ActiveModel with the data, including the formatted timestamp string
        let active_model = ActiveModel {
            id: Set(id),
            content: Set(content.to_string()),
            updated_at_hlc_ts: Set(hlc_ts_str), // Set the String value
            updated_at_hlc_v: Set(hlc.version as i32), // Convert u32 to i32
            updated_at_hlc_nid: Set(hlc.node_id),
        };
        // Insert into the database and return the resulting Model
        active_model.insert(db).await
    }

    // HLC Creation Helper
    // Convenience function for creating HLC instances in tests.
    fn hlc(ts_millis: u64, version: u32, node_str: &str) -> HLC {
        HLC {
            timestamp: ts_millis,
            version,
            node_id: Uuid::parse_str(node_str).expect("Invalid UUID string in test"),
        }
    }

    // Constant for Node ID used in tests
    const NODE1: &str = "11111111-1111-1111-1111-111111111111";

    // Unit Tests

    #[tokio::test]
    async fn test_calculate_chunk_hash_empty() -> Result<()> {
        let records: Vec<Model> = vec![]; // Use the test Model
        let hash = calculate_chunk_hash(&records)?;
        // Verify against the known Blake3 hash of an empty input
        assert_eq!(
            hash,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_calculate_chunk_hash_single() -> Result<()> {
        let h = hlc(100, 0, NODE1);
        // Manually construct a Model instance for hashing calculation verification
        // Note: We don't need to insert this into DB for this specific test.
        let record = Model {
            id: 1,
            content: "test".to_string(),
            // We need a valid RFC3339 string here if HLCRecord impl requires it,
            // but calculate_record_hash uses data_for_hashing which *doesn't* include timestamps.
            // So, the actual timestamp string value doesn't affect the hash calculation itself.
            // However, for completeness and realism, let's format it.
            updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(h.timestamp)?,
            updated_at_hlc_v: h.version as i32,
            updated_at_hlc_nid: h.node_id,
        };
        let records = vec![record]; // Use the test Model

        // Calculate the expected hash: hash(hex(hash(record1)))
        let record_hash = calculate_record_hash(&records[0])?;
        let mut expected_chunk_hasher = Hasher::new();
        expected_chunk_hasher.update(record_hash.as_bytes());
        let expected_hash = expected_chunk_hasher.finalize().to_hex().to_string();

        // Calculate the actual chunk hash using the function under test
        let hash = calculate_chunk_hash(&records)?;
        assert_eq!(hash, expected_hash);
        Ok(())
    }

    #[tokio::test]
    async fn test_calculate_chunk_hash_multiple() -> Result<()> {
        let h1 = hlc(100, 0, NODE1);
        let r1 = Model {
            // Use the test Model
            id: 1,
            content: "A".to_string(),
            updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(h1.timestamp)?,
            updated_at_hlc_v: h1.version as i32,
            updated_at_hlc_nid: h1.node_id,
        };
        let h2 = hlc(100, 1, NODE1); // Same timestamp, different version
        let r2 = Model {
            // Use the test Model
            id: 2,
            content: "B".to_string(),
            updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(h2.timestamp)?, // Format even if same ms
            updated_at_hlc_v: h2.version as i32,
            updated_at_hlc_nid: h2.node_id,
        };

        let records = vec![r1.clone(), r2.clone()];

        // Calculate expected hash: hash(hex(hash(r1)) + hex(hash(r2)))
        let hash1 = calculate_record_hash(&r1)?;
        let hash2 = calculate_record_hash(&r2)?;
        let mut expected_chunk_hasher = Hasher::new();
        expected_chunk_hasher.update(hash1.as_bytes());
        expected_chunk_hasher.update(hash2.as_bytes());
        let expected_hash = expected_chunk_hasher.finalize().to_hex().to_string();

        let hash = calculate_chunk_hash(&records)?;
        assert_eq!(hash, expected_hash);
        Ok(())
    }

    #[tokio::test]
    async fn test_generate_data_chunks_empty_db() -> Result<()> {
        let db = setup_db().await?;
        let options = ChunkingOptions::default(Uuid::parse_str(NODE1).unwrap());
        // Use the test Entity
        let chunks = generate_data_chunks::<Entity>(&db, &options, None).await?;
        assert!(chunks.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_generate_data_chunks_simple() -> Result<()> {
        let db = setup_db().await?;
        let node_id = Uuid::parse_str(NODE1).unwrap();
        let base_ts = 1700000000000; // Some base time in ms

        // Insert records using the helper (handles RFC3339 conversion)
        let h1 = hlc(base_ts + 100, 0, NODE1);
        insert_task(&db, 1, "Task 1", &h1).await?;
        let h2 = hlc(base_ts + 200, 0, NODE1);
        insert_task(&db, 2, "Task 2", &h2).await?;
        let h3 = hlc(base_ts + 300, 0, NODE1);
        insert_task(&db, 3, "Task 3", &h3).await?;
        let h4 = hlc(base_ts + 400, 0, NODE1);
        insert_task(&db, 4, "Task 4", &h4).await?;
        let h5 = hlc(base_ts + 500, 0, NODE1);
        insert_task(&db, 5, "Task 5", &h5).await?;

        let options = ChunkingOptions {
            min_size: 2,
            max_size: 3,
            alpha: 0.0, // Keep size constant for simplicity
            node_id,
        };

        // Use the test Entity
        let chunks = generate_data_chunks::<Entity>(&db, &options, None).await?;

        // Expected: Chunk1 (r1, r2), Chunk2 (r3, r4), Chunk3 (r5)
        // The logic remains the same as the internal representation (HLC struct) is used
        // for comparisons and calculations, correctly parsed from the RFC3339 string.
        assert_eq!(chunks.len(), 3);

        assert_eq!(chunks[0].start_hlc, h1);
        assert_eq!(chunks[0].end_hlc, h2);
        assert_eq!(chunks[0].count, 2);
        assert!(!chunks[0].chunk_hash.is_empty()); // Check hash exists

        assert_eq!(chunks[1].start_hlc, h3);
        assert_eq!(chunks[1].end_hlc, h4);
        assert_eq!(chunks[1].count, 2);
        assert!(!chunks[1].chunk_hash.is_empty());

        assert_eq!(chunks[2].start_hlc, h5);
        assert_eq!(chunks[2].end_hlc, h5);
        assert_eq!(chunks[2].count, 1);
        assert!(!chunks[2].chunk_hash.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn test_generate_data_chunks_with_alpha() -> Result<()> {
        let db = setup_db().await?;
        let node_id = Uuid::parse_str(NODE1).unwrap();
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let days_ago_30 = current_time.saturating_sub(30 * MILLISECONDS_PER_DAY);
        let days_ago_10 = current_time.saturating_sub(10 * MILLISECONDS_PER_DAY);
        let days_ago_1 = current_time.saturating_sub(MILLISECONDS_PER_DAY);

        // Ensure timestamps are distinct and positive
        let ts1 = days_ago_30;
        let ts2 = ts1 + 1000;
        let ts3 = days_ago_10;
        let ts4 = ts3 + 1000;
        let ts5 = days_ago_1;
        let ts6 = ts5 + 1000;
        let ts7 = current_time.saturating_sub(1000); // Most recent

        let h1 = hlc(ts1, 0, NODE1);
        insert_task(&db, 1, "Old 1", &h1).await?;
        let h2 = hlc(ts2, 0, NODE1);
        insert_task(&db, 2, "Old 2", &h2).await?;
        let h3 = hlc(ts3, 0, NODE1);
        insert_task(&db, 3, "Med 1", &h3).await?;
        let h4 = hlc(ts4, 0, NODE1);
        insert_task(&db, 4, "Med 2", &h4).await?;
        let h5 = hlc(ts5, 0, NODE1);
        insert_task(&db, 5, "New 1", &h5).await?;
        let h6 = hlc(ts6, 0, NODE1);
        insert_task(&db, 6, "New 2", &h6).await?;
        let h7 = hlc(ts7, 0, NODE1);
        insert_task(&db, 7, "Latest", &h7).await?; // Latest

        let options = ChunkingOptions {
            min_size: 1,
            max_size: 10,
            alpha: 0.1, // Slight increase with age
            node_id,
        };

        let chunks = generate_data_chunks::<Entity>(&db, &options, None).await?;

        // Trace: Latest = h7 (ts7)
        // Loop 1: current=0. age=(ts7 - 0)/days = large. ceil(age)=large.
        //        desired=1*(1.1)^large > 10. window=max(1,min(10, round(desired)))=10.
        //        Fetch(after 0, limit 10) -> r1..r7. Chunk [h1-h7], count 7. next=h7.
        // Loop 2: current=h7. Fetch(after h7, limit 10) -> []. Break.
        // Expected: One chunk containing all 7 records.
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_hlc, h1);
        assert_eq!(chunks[0].end_hlc, h7);
        assert_eq!(chunks[0].count, 7);

        // Try with min_size=3
        let options2 = ChunkingOptions {
            min_size: 3,
            max_size: 10,
            alpha: 0.1,
            node_id,
        };
        let chunks2 = generate_data_chunks::<Entity>(&db, &options2, None).await?;
        // Trace:
        // Loop 1: current=0. age=large. desired=3*(1.1)^large > 10. window=10.
        //        Fetch(limit 10) -> r1..r7. Chunk [h1-h7], count 7. next=h7.
        // Loop 2: current=h7. Fetch(limit 10) -> []. Break.
        assert_eq!(
            chunks2.len(),
            1,
            "With min_size=3, still expected 1 chunk due to large age"
        );
        assert_eq!(chunks2[0].count, 7);

        Ok(())
    }

    #[tokio::test]
    async fn test_break_data_chunk_success() -> Result<()> {
        let db = setup_db().await?;
        let base_ts = 1700000000000;

        // Insert 5 records
        let h1 = hlc(base_ts + 100, 0, NODE1);
        let _r1 = insert_task(&db, 1, "T1", &h1).await?;
        let h2 = hlc(base_ts + 200, 0, NODE1);
        let _r2 = insert_task(&db, 2, "T2", &h2).await?;
        let h3 = hlc(base_ts + 300, 0, NODE1);
        let _r3 = insert_task(&db, 3, "T3", &h3).await?;
        let h4 = hlc(base_ts + 400, 0, NODE1);
        let _r4 = insert_task(&db, 4, "T4", &h4).await?;
        let h5 = hlc(base_ts + 500, 0, NODE1);
        let _r5 = insert_task(&db, 5, "T5", &h5).await?;
        // Fetch them back to pass to calculate_chunk_hash in the correct order and state
        // (though constructing them manually would also work if `data_for_hashing` is simple)
        let all_records = Entity::find().order_by_hlc_asc::<Entity>().all(&db).await?; // Fetch using Test Entity
        assert_eq!(all_records.len(), 5);

        // Create the parent chunk definition using the fetched records for hash calculation
        let parent_chunk = DataChunk {
            start_hlc: h1.clone(),
            end_hlc: h5.clone(),
            count: 5,
            chunk_hash: calculate_chunk_hash(&all_records)?, // Hash based on fetched records
        };

        let sub_chunk_size = 2;
        // Use the test Entity
        let sub_chunks = break_data_chunk::<Entity>(&db, &parent_chunk, sub_chunk_size).await?;

        assert_eq!(sub_chunks.len(), 3); // Expect 2, 2, 1

        // Verify SubChunk 1
        assert_eq!(sub_chunks[0].chunk.start_hlc, h1);
        assert_eq!(sub_chunks[0].chunk.end_hlc, h2);
        assert_eq!(sub_chunks[0].chunk.count, 2);
        assert_eq!(
            sub_chunks[0].chunk.chunk_hash,
            calculate_chunk_hash(&all_records[0..2])? // Hash the slice
        );
        assert_eq!(sub_chunks[0].parent_start_hlc, parent_chunk.start_hlc);
        assert_eq!(sub_chunks[0].parent_end_hlc, parent_chunk.end_hlc);
        assert_eq!(sub_chunks[0].parent_chunk_hash, parent_chunk.chunk_hash);

        // Verify SubChunk 2
        assert_eq!(sub_chunks[1].chunk.start_hlc, h3);
        assert_eq!(sub_chunks[1].chunk.end_hlc, h4);
        assert_eq!(sub_chunks[1].chunk.count, 2);
        assert_eq!(
            sub_chunks[1].chunk.chunk_hash,
            calculate_chunk_hash(&all_records[2..4])?
        );
        assert_eq!(sub_chunks[1].parent_start_hlc, parent_chunk.start_hlc);
        assert_eq!(sub_chunks[1].parent_end_hlc, parent_chunk.end_hlc);
        assert_eq!(sub_chunks[1].parent_chunk_hash, parent_chunk.chunk_hash);

        // Verify SubChunk 3
        assert_eq!(sub_chunks[2].chunk.start_hlc, h5);
        assert_eq!(sub_chunks[2].chunk.end_hlc, h5);
        assert_eq!(sub_chunks[2].chunk.count, 1);
        assert_eq!(
            sub_chunks[2].chunk.chunk_hash,
            calculate_chunk_hash(&all_records[4..5])?
        );
        assert_eq!(sub_chunks[2].parent_start_hlc, parent_chunk.start_hlc);
        assert_eq!(sub_chunks[2].parent_end_hlc, parent_chunk.end_hlc);
        assert_eq!(sub_chunks[2].parent_chunk_hash, parent_chunk.chunk_hash);

        Ok(())
    }

    #[tokio::test]
    async fn test_break_data_chunk_empty_parent() -> Result<()> {
        let db = setup_db().await?;
        let base_ts = 1700000000000;

        // Insert data outside the range of the empty chunk (optional, but good sanity check)
        let h_outside = hlc(base_ts + 1000, 0, NODE1);
        insert_task(&db, 99, "Outside", &h_outside).await?;

        // Define an empty parent chunk
        let empty_start = hlc(base_ts + 100, 0, NODE1);
        let empty_end = hlc(base_ts + 500, 0, NODE1);
        let empty_records: Vec<Model> = vec![]; // Use test Model
        let parent_chunk = DataChunk {
            start_hlc: empty_start.clone(),
            end_hlc: empty_end.clone(),
            count: 0,
            chunk_hash: calculate_chunk_hash(&empty_records)?, // Hash of empty
        };

        let sub_chunk_size = 10;
        // Use the test Entity
        let sub_chunks = break_data_chunk::<Entity>(&db, &parent_chunk, sub_chunk_size).await?;

        // Expect successful verification of empty chunk and no sub-chunks generated
        assert!(sub_chunks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn test_break_chunk_verification_fail_count() -> Result<()> {
        let db = setup_db().await?;
        let base_ts = 1700000000000;

        // Insert 3 records
        let h1 = hlc(base_ts + 100, 0, NODE1);
        insert_task(&db, 1, "T1", &h1).await?;
        let h2 = hlc(base_ts + 200, 0, NODE1);
        insert_task(&db, 2, "T2", &h2).await?;
        let h3 = hlc(base_ts + 300, 0, NODE1);
        insert_task(&db, 3, "T3", &h3).await?;

        // Create parent chunk definition expecting 4 records (incorrect)
        let parent_chunk = DataChunk {
            start_hlc: h1.clone(),
            end_hlc: h3.clone(), // Range covers the 3 inserted records
            count: 4,            // Mismatched count
            chunk_hash: "dummy_hash".to_string(), // Hash doesn't matter yet
        };

        let sub_chunk_size = 2;
        // Use the test Entity
        let result = break_data_chunk::<Entity>(&db, &parent_chunk, sub_chunk_size).await;

        // Assert that the operation failed
        assert!(result.is_err());
        // Assert that the error message indicates a count mismatch
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Record count mismatch"));
        assert!(err_msg.contains("Expected 4, Found 3"));

        Ok(())
    }

    #[tokio::test]
    async fn test_break_chunk_verification_fail_hash() -> Result<()> {
        let db = setup_db().await?;
        let base_ts = 1700000000000;

        // Insert 2 records
        let h1 = hlc(base_ts + 100, 0, NODE1);
        let _r1 = insert_task(&db, 1, "T1", &h1).await?;
        let h2 = hlc(base_ts + 200, 0, NODE1);
        let _r2 = insert_task(&db, 2, "T2", &h2).await?;

        // Fetch the actual records to calculate the correct hash
        let actual_records = Entity::find()
            .filter(Entity::between(&h1, &h2)?) // Use Test Entity
            .order_by_hlc_asc::<Entity>() // Use Test Entity
            .all(&db)
            .await?;
        let correct_hash = calculate_chunk_hash(&actual_records)?;

        // Create parent chunk definition with correct count but incorrect hash
        let parent_chunk = DataChunk {
            start_hlc: h1.clone(),
            end_hlc: h2.clone(),
            count: 2,
            chunk_hash: "incorrect_hash_string_12345".to_string(), // Wrong hash
        };

        let sub_chunk_size = 1;
        // Use the test Entity
        let result = break_data_chunk::<Entity>(&db, &parent_chunk, sub_chunk_size).await;

        // Assert that the operation failed
        assert!(result.is_err());
        // Assert that the error message indicates a hash mismatch
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Hash mismatch"));
        assert!(err_msg.contains(&format!("Expected '{}'", parent_chunk.chunk_hash)));
        assert!(err_msg.contains(&format!("Calculated '{}'", correct_hash)));

        Ok(())
    }

    #[tokio::test]
    async fn test_break_chunk_verification_fail_empty_hash() -> Result<()> {
        let db = setup_db().await?;
        let base_ts = 1700000000000;

        // Define an empty parent chunk range
        let empty_start = hlc(base_ts + 100, 0, NODE1);
        let empty_end = hlc(base_ts + 500, 0, NODE1);
        // Calculate the correct hash for an empty chunk
        let correct_empty_hash = calculate_chunk_hash::<Model>(&[])?; // Use test Model

        // Create a parent chunk definition reporting 0 count but with an incorrect hash
        let parent_chunk = DataChunk {
            start_hlc: empty_start.clone(),
            end_hlc: empty_end.clone(),
            count: 0,
            chunk_hash: "non_empty_hash_or_just_plain_wrong".to_string(), // Incorrect hash
        };

        let sub_chunk_size = 10;
        // Use the test Entity
        let result = break_data_chunk::<Entity>(&db, &parent_chunk, sub_chunk_size).await;

        // Assert that the operation failed
        assert!(result.is_err());
        // Assert that the error message indicates a hash mismatch for an empty chunk
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Parent chunk reported 0 count, but hash mismatch"));
        assert!(err_msg.contains(&format!("Expected empty hash '{}'", correct_empty_hash)));
        assert!(err_msg.contains(&format!("Found '{}'", parent_chunk.chunk_hash)));

        Ok(())
    }

    #[tokio::test]
    async fn test_generate_chunks_start_hlc_exclusive() -> Result<()> {
        let db = setup_db().await?;
        let node_id = Uuid::parse_str(NODE1).unwrap();
        let base_ts = 1700000000000;

        // Insert 5 records
        let h1 = hlc(base_ts + 100, 0, NODE1);
        insert_task(&db, 1, "T1", &h1).await?;
        let h2 = hlc(base_ts + 200, 0, NODE1);
        insert_task(&db, 2, "T2", &h2).await?;
        let h3 = hlc(base_ts + 300, 0, NODE1);
        insert_task(&db, 3, "T3", &h3).await?;
        let h4 = hlc(base_ts + 400, 0, NODE1);
        insert_task(&db, 4, "T4", &h4).await?;
        let h5 = hlc(base_ts + 500, 0, NODE1);
        insert_task(&db, 5, "T5", &h5).await?;

        let options = ChunkingOptions {
            min_size: 1, // Small min size
            max_size: 5,
            alpha: 0.0, // No decay for simplicity
            node_id,
        };

        // Define the starting point *after* h2
        let start_after = h2.clone();

        // Generate chunks using the test Entity and the exclusive start HLC
        let chunks = generate_data_chunks::<Entity>(&db, &options, Some(start_after)).await?;

        // Trace: min=1, max=5, alpha=0. Latest=h5. Start current=h2.
        // Loop 1: current=h2. age=small. desired=1*(1)^n=1. window=max(1,min(5,1))=1.
        //        Fetch(after h2, limit 1) -> r3. Chunk [h3-h3], count 1. next=h3.
        // Loop 2: current=h3. age=smaller. desired=1. window=1.
        //        Fetch(after h3, limit 1) -> r4. Chunk [h4-h4], count 1. next=h4.
        // Loop 3: current=h4. age=smallest. desired=1. window=1.
        //        Fetch(after h4, limit 1) -> r5. Chunk [h5-h5], count 1. next=h5.
        // Loop 4: current=h5. Fetch(after h5, limit 1) -> []. Break.
        // Expected: 3 chunks, containing r3, r4, and r5 respectively.
        assert_eq!(chunks.len(), 3);

        assert_eq!(chunks[0].start_hlc, h3);
        assert_eq!(chunks[0].end_hlc, h3);
        assert_eq!(chunks[0].count, 1);

        assert_eq!(chunks[1].start_hlc, h4);
        assert_eq!(chunks[1].end_hlc, h4);
        assert_eq!(chunks[1].count, 1);

        assert_eq!(chunks[2].start_hlc, h5);
        assert_eq!(chunks[2].end_hlc, h5);
        assert_eq!(chunks[2].count, 1);

        Ok(())
    }

    #[test]
    fn test_preset_high_frequency_mobile() {
        let node_id = Uuid::new_v4();
        let options = high_frequency_mobile_preset(node_id);
        assert_eq!(options.min_size, 50);
        assert_eq!(options.max_size, 2_000);
        assert_eq!(options.alpha, 0.25);
        assert_eq!(options.node_id, node_id);
        assert!(options.validate().is_ok());
    }

    #[test]
    fn test_preset_append_optimized_backend() {
        let node_id = Uuid::new_v4();
        let options = append_optimized_backend_preset(node_id);
        assert_eq!(options.min_size, 250);
        assert_eq!(options.max_size, 20_000);
        assert_eq!(options.alpha, 0.7);
        assert_eq!(options.node_id, node_id);
        assert!(options.validate().is_ok());
    }

    #[test]
    fn test_preset_balanced_web_desktop() {
        let node_id = Uuid::new_v4();
        let options = balanced_web_desktop_preset(node_id);
        assert_eq!(options.min_size, 100);
        assert_eq!(options.max_size, 10_000);
        assert_eq!(options.alpha, 0.45);
        assert_eq!(options.node_id, node_id);
        assert!(options.validate().is_ok());
    }

    #[test]
    fn test_preset_initial_sync_optimized() {
        let node_id = Uuid::new_v4();
        let options = initial_sync_optimized(node_id);
        assert_eq!(options.min_size, 500);
        assert_eq!(options.max_size, 15_000);
        assert_eq!(options.alpha, 0.6);
        assert_eq!(options.node_id, node_id);
        assert!(options.validate().is_ok());
    }

    #[test]
    fn test_preset_default() {
        let node_id = Uuid::new_v4();
        let options = ChunkingOptions::default(node_id);
        assert_eq!(options.min_size, 100);
        assert_eq!(options.max_size, 10_000);
        assert_eq!(options.alpha, 0.4);
        assert_eq!(options.node_id, node_id);
        assert!(options.validate().is_ok());
    }

    #[test]
    fn test_chunking_options_validate_success() {
        let node_id = Uuid::new_v4();
        let options = ChunkingOptions {
            min_size: 10,
            max_size: 100,
            alpha: 0.5,
            node_id,
        };
        assert!(options.validate().is_ok());
    }

    #[test]
    fn test_chunking_options_validate_min_size_zero() {
        let node_id = Uuid::new_v4();
        let options = ChunkingOptions {
            min_size: 0, // Invalid
            max_size: 100,
            alpha: 0.5,
            node_id,
        };
        let result = options.validate();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "min_size must be greater than 0"
        );
    }

    #[test]
    fn test_chunking_options_validate_max_less_than_min() {
        let node_id = Uuid::new_v4();
        let options = ChunkingOptions {
            min_size: 100,
            max_size: 50, // Invalid
            alpha: 0.5,
            node_id,
        };
        let result = options.validate();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "max_size must be greater than or equal to min_size"
        );
    }

    #[test]
    fn test_chunking_options_validate_negative_alpha() {
        let node_id = Uuid::new_v4();
        let options = ChunkingOptions {
            min_size: 10,
            max_size: 100,
            alpha: -0.1, // Invalid
            node_id,
        };
        let result = options.validate();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "alpha must be non-negative"
        );
    }

    #[test]
    fn test_chunking_options_validate_max_equals_min() {
        let node_id = Uuid::new_v4();
        let options = ChunkingOptions {
            min_size: 50,
            max_size: 50, // Valid
            alpha: 0.5,
            node_id,
        };
        assert!(options.validate().is_ok());
    }

    #[test]
    fn test_chunking_options_validate_alpha_zero() {
        let node_id = Uuid::new_v4();
        let options = ChunkingOptions {
            min_size: 10,
            max_size: 100,
            alpha: 0.0, // Valid
            node_id,
        };
        assert!(options.validate().is_ok());
    }

    #[tokio::test]
    async fn test_break_chunk_invalid_sub_chunk_size() -> Result<()> {
        let db = setup_db().await?;
        let base_ts = 1700000000000;

        // Insert a dummy record, although it won't be reached due to early exit
        let h1 = hlc(base_ts + 100, 0, NODE1);
        insert_task(&db, 1, "T1", &h1).await?;

        // Create a valid parent chunk definition
        let parent_chunk = DataChunk {
            start_hlc: h1.clone(),
            end_hlc: h1.clone(), // Single record chunk
            count: 1,
            chunk_hash: calculate_chunk_hash(&[Model {
                // Manually construct for hash calc
                id: 1,
                content: "T1".to_string(),
                updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(h1.timestamp)?,
                updated_at_hlc_v: h1.version as i32,
                updated_at_hlc_nid: h1.node_id,
            }])?,
        };

        // Attempt to break with sub_chunk_size = 0
        let sub_chunk_size = 0; // Invalid size
        let result = break_data_chunk::<Entity>(&db, &parent_chunk, sub_chunk_size).await;

        // Assert that the operation failed
        assert!(result.is_err());
        // Assert that the error message indicates the specific reason
        let err_msg = result.err().unwrap().to_string();
        assert!(
            err_msg.contains("Sub-chunk size cannot be zero"),
            "Error message was: {}",
            err_msg
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_break_chunk_invalid_parent_definition() -> Result<()> {
        let db = setup_db().await?; // DB setup needed, though no fetch happens
        let base_ts = 1700000000000;

        // Define an invalid parent chunk where start > end but count > 0
        let h_start = hlc(base_ts + 200, 0, NODE1);
        let h_end = hlc(base_ts + 100, 0, NODE1); // start > end

        let parent_chunk = DataChunk {
            start_hlc: h_start.clone(),
            end_hlc: h_end.clone(),
            count: 1,                             // Non-zero count makes it invalid
            chunk_hash: "dummy_hash".to_string(), // Hash doesn't matter here
        };

        let sub_chunk_size = 10; // Valid sub-chunk size
        let result = break_data_chunk::<Entity>(&db, &parent_chunk, sub_chunk_size).await;

        // Assert that the operation failed
        assert!(result.is_err());
        // Assert that the error message indicates the specific reason
        let err_msg = result.err().unwrap().to_string();
        assert!(
            err_msg.contains(
                "Invalid parent chunk definition: start_hlc > end_hlc with non-zero count."
            ),
            "Error message was: {}",
            err_msg
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_break_chunk_valid_parent_start_gt_end_zero_count() -> Result<()> {
        // Test the edge case where start > end is technically allowed IF count is 0.
        // This case should proceed to the verification step for an empty chunk.
        let db = setup_db().await?;
        let base_ts = 1700000000000;

        let h_start = hlc(base_ts + 200, 0, NODE1);
        let h_end = hlc(base_ts + 100, 0, NODE1); // start > end

        // Create a parent chunk with start > end but count = 0 and correct empty hash
        let empty_records: Vec<Model> = vec![];
        let parent_chunk = DataChunk {
            start_hlc: h_start.clone(),
            end_hlc: h_end.clone(),
            count: 0, // Count is zero, so this *might* be valid if hash matches empty
            chunk_hash: calculate_chunk_hash(&empty_records)?,
        };

        let sub_chunk_size = 10;
        let result = break_data_chunk::<Entity>(&db, &parent_chunk, sub_chunk_size).await;

        // In this case, the function should proceed past the initial validation,
        // fetch records (find none in the weird range or any range if DB empty),
        // verify count (0 == 0), verify hash (empty == empty), and return Ok([]).
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty()); // Expect no sub-chunks

        Ok(())
    }
}
