//! This module provides core components for building a data synchronization system.
//! It leverages Hybrid Logical Clocks (HLC) to ensure causality and ordering of events
//! across distributed nodes. The key features include:
//!
//! 1.  **Hybrid Logical Clock (HLC):**
//!     -   Implementation of the `HLC` struct, combining physical time, a logical counter,
//!         and a node ID to generate monotonically increasing timestamps.
//!     -   Functions for HLC generation, parsing from string, and initial HLC creation.
//!
//! 2.  **`HLCRecord` Trait:**
//!     -   A trait `HLCRecord` designed to be implemented by SeaORM entities.
//!     -   Provides a standardized interface for accessing HLC timestamps (`created_at_hlc`, `updated_at_hlc`),
//!         unique identifiers, and data representations for hashing and synchronization.
//!     -   Methods for retrieving full data, summary data, and data for hashing, allowing for
//!         different levels of data granularity in synchronization processes.
//!     -   Helper methods to access SeaORM entity metadata like entity type, primary key column,
//!         and HLC-related columns.
//!
//! 3.  **Data Hashing:**
//!     -   `calculate_hash` function to compute BLAKE3 hashes of `HLCRecord` instances.
//!     -   Ensures data integrity and facilitates change detection by hashing the JSON representation
//!         of the entity's data.
//!
//! 4.  **HLC-Based Data Retrieval:**
//!     -   Functions to query data based on HLC timestamps:
//!         -   `get_data_before_hlc`: Retrieve records created/updated before or at a given HLC.
//!         -   `get_data_after_hlc`: Retrieve records created/updated after a given HLC.
//!         -   `get_data_in_hlc_range`: Retrieve records within a specified HLC range.
//!     -   These functions enable efficient synchronization by allowing nodes to fetch only the data
//!         that has changed since a specific point in time or within a specific time window.
//!
//! This module is designed to be highly reusable and extensible, providing a solid foundation for
//! building robust and consistent data synchronization mechanisms in distributed systems using Rust
//! and SeaORM.

use std::{
    cmp::Ordering,
    str::FromStr,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use blake3::Hasher;
use chrono::{LocalResult, TimeZone, Utc};
use sea_orm::{
    entity::prelude::*, Condition, DatabaseConnection, PaginatorTrait, QueryFilter, QueryOrder,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Converts a Unix timestamp (in milliseconds since epoch) to an RFC3339 formatted string for DB.
/// SeaORM often uses chrono::DateTime<Utc> which maps well to RFC3339.
/// NOTE: HLC uses milliseconds, but DB timestamp columns might store seconds or need specific formats.
/// This function assumes the DB expects RFC3339 UTC. Adjust if your DB type differs.
pub fn hlc_timestamp_millis_to_rfc3339(millis: u64) -> Result<String> {
    // Convert milliseconds to seconds and nanoseconds
    let secs = (millis / 1000) as i64;
    let nanos = ((millis % 1000) * 1_000_000) as u32;

    match Utc.timestamp_opt(secs, nanos) {
        LocalResult::Single(datetime) => {
            // Format to RFC3339 with milliseconds precision
            let rfc3339_string = datetime.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            Ok(rfc3339_string)
        }
        LocalResult::None => {
            bail!("HLC Milliseconds timestamp is out of range: {}", millis)
        }
        LocalResult::Ambiguous(..) => {
            // Ambiguous should not happen for UTC timestamps
            bail!(
                "HLC Milliseconds timestamp is ambiguous (should not happen for UTC): {}",
                millis
            )
        }
    }
}

#[cfg(test)]
mod timestamp_tests {
    // Renamed inner mod tests to avoid conflict
    use super::*;

    #[test]
    fn test_hlc_millis_to_rfc3339_valid() -> Result<()> {
        let millis: u64 = 1678886400123; // March 15, 2023 00:00:00.123 UTC
        let rfc3339_string = hlc_timestamp_millis_to_rfc3339(millis)?;
        // chrono format includes '+00:00' instead of 'Z' sometimes, both are valid RFC3339 UTC.
        assert!(
            rfc3339_string == "2023-03-15T00:00:00.123Z"
                || rfc3339_string == "2023-03-15T00:00:00.123+00:00"
        );
        Ok(())
    }

    #[test]
    fn test_hlc_millis_to_rfc3339_zero() -> Result<()> {
        let millis: u64 = 0; // Epoch
        let rfc3339_string = hlc_timestamp_millis_to_rfc3339(millis)?;
        assert!(
            rfc3339_string == "1970-01-01T00:00:00.000Z"
                || rfc3339_string == "1970-01-01T00:00:00.000+00:00"
        );
        Ok(())
    }

    #[test]
    fn test_hlc_millis_out_of_range() {
        // chrono's range is roughly +/- 262,000 years from 1970.
        // u64::MAX milliseconds is far beyond that.
        let invalid_millis: u64 = u64::MAX;
        let result = hlc_timestamp_millis_to_rfc3339(invalid_millis);
        assert!(result.is_err());
        eprintln!("Out of range error: {:?}", result.err().unwrap()); // Print error for info
    }
}

/// Represents a Hybrid Logical Clock (HLC).
///
/// An HLC combines a physical timestamp with a logical counter to ensure
/// monotonically increasing timestamps across a distributed system, even
/// with clock skew.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HLC {
    /// Physical timestamp component, in milliseconds since the Unix epoch.
    pub timestamp: u64,
    /// Logical counter component, incremented for events within the same millisecond.
    pub version: u32,
    /// Unique identifier for the node that generated this HLC.
    pub node_id: Uuid,
}

impl std::fmt::Display for HLC {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}-{:08x}-{}",
            self.timestamp, self.version, self.node_id
        )
    }
}

pub struct SyncTaskContext {
    node_id: Uuid,
    last_hlc: Mutex<HLC>,
}

impl SyncTaskContext {
    pub fn new(node_id: Uuid) -> Self {
        SyncTaskContext {
            node_id,
            last_hlc: Mutex::new(HLC::new(node_id)),
        }
    }

    pub fn generate_hlc(&self) -> HLC {
        HLC::generate(self)
    }
}

impl HLC {
    /// Creates an initial HLC with timestamp 0 and counter 0.
    ///
    /// This is often used as a starting point or default value.
    pub fn new(node_id: Uuid) -> Self {
        HLC {
            timestamp: 0,
            version: 0,
            node_id,
        }
    }

    /// Generates a new HLC, ensuring monotonicity.
    ///
    /// It compares the current system time with the timestamp of the last generated HLC.
    /// If the current time is ahead, it uses the current time and resets the counter.
    /// If it's the same, it increments the counter. If clock skew is detected (current time is behind),
    /// it uses the last HLC's timestamp and increments the counter to maintain order.
    pub fn generate(context: &SyncTaskContext) -> Self {
        let mut last_hlc = context.last_hlc.lock().unwrap();
        let current_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_millis() as u64;

        let (timestamp, counter) = match current_timestamp.cmp(&last_hlc.timestamp) {
            Ordering::Greater => (current_timestamp, 0),
            Ordering::Equal => (current_timestamp, last_hlc.version + 1),
            Ordering::Less => {
                // Clock skew detected, use last HLC's timestamp
                (last_hlc.timestamp, last_hlc.version + 1)
            }
        };

        // Basic overflow check for counter (though u32::MAX is large)
        if counter == 0 && last_hlc.version == u32::MAX && timestamp == last_hlc.timestamp {
            // Extremely unlikely: wrapped u32 within the same millisecond.
            // Options: panic, log error, or potentially bump timestamp artificially (less ideal).
            // For now, let's panic as it indicates a potentially serious issue or misuse.
            panic!("HLC counter overflow detected within a single millisecond. Timestamp: {}, Node: {}", timestamp, context.node_id);
        }

        let new_hlc = HLC {
            timestamp,
            version: counter,
            node_id: context.node_id,
        };
        *last_hlc = new_hlc.clone();
        new_hlc
    }
}

impl FromStr for HLC {
    type Err = anyhow::Error;

    /// Parses an HLC from a string representation.
    ///
    /// The string format is expected to be "timestamp-counterHex-node_id".
    fn from_str(hlc_str: &str) -> Result<Self> {
        let parts: Vec<&str> = hlc_str.splitn(3, '-').collect();
        if parts.len() != 3 {
            bail!(
                "Invalid HLC string format: expected 'timestamp-counterHex-node_id', got '{}'",
                hlc_str
            );
        }

        let timestamp = parts[0]
            .parse::<u64>()
            .with_context(|| format!("Invalid timestamp format in HLC: '{}'", parts[0]))?;
        // Ensure counter part has expected hex length if needed, though from_str_radix handles it.
        let counter = u32::from_str_radix(parts[1], 16)
            .with_context(|| format!("Invalid hex counter format in HLC: '{}'", parts[1]))?;
        let node_id = Uuid::parse_str(parts[2])
            .with_context(|| format!("Invalid node ID format in HLC: '{}'", parts[2]))?;

        Ok(HLC {
            timestamp,
            version: counter,
            node_id,
        })
    }
}

/// A trait for SeaORM *Models* that need HLC timestamps and hashing.
///
/// Models implementing this trait can be tracked for changes over time
/// using Hybrid Logical Clocks and their data can be hashed for integrity checks.
/// Note: Implemented on the `Model` struct, not the `Entity`.
pub trait HLCRecord: Clone + Send + Sync + 'static {
    /// Returns the HLC timestamp when the record was created (if available).
    /// Implementors should fetch this from the model's fields.
    fn created_at_hlc(&self) -> Option<HLC>;

    /// Returns the HLC timestamp when the record was last updated.
    /// Implementors should fetch this from the model's fields.
    /// This is crucial for ordering and chunking.
    fn updated_at_hlc(&self) -> Option<HLC>;

    /// Returns a unique identifier for the record, suitable for logging or comparison.
    /// Often the primary key, but needs conversion to a common type like String or i64 if PK varies.
    fn unique_id(&self) -> String; // Changed to String for more flexibility

    /// Returns the data of the record as a JSON value for hashing.
    ///
    /// This should include all fields that are relevant for determining
    /// if the record has changed. Exclude fields like `updated_at_hlc` itself
    /// if the hash should only represent the *content*.
    fn data_for_hashing(&self) -> serde_json::Value;

    /// Returns a summary representation of the record data as a JSON value.
    /// Defaults to `data_for_hashing()`.
    fn to_summary(&self) -> serde_json::Value {
        self.data_for_hashing()
    }

    /// Returns the full data of the record as a JSON value.
    /// Defaults to `data_for_hashing()`.
    fn full_data(&self) -> serde_json::Value {
        self.data_for_hashing()
    }
}

/// Trait for SeaORM Entities to provide HLC column information for querying.
pub trait HLCModel: EntityTrait + Sized + Send + Sync + 'static {
    /// Returns the SeaORM column definition for the HLC timestamp component.
    /// Assumes this column stores a value comparable via RFC3339 strings (like DATETIME or TIMESTAMP).
    fn updated_at_time_column() -> Self::Column;

    /// Returns the SeaORM column definition for the HLC version/counter component.
    /// Assumes this column stores an integer type (like INTEGER or BIGINT).
    fn updated_at_version_column() -> Self::Column;

    /// Returns the SeaORM column definition for the unique identifier.
    fn unique_id_column() -> Self::Column;

    /// Creates a SeaORM condition for records strictly greater than the given HLC.
    fn gt(hlc: &HLC) -> Result<Condition> {
        let timestamp_str = hlc_timestamp_millis_to_rfc3339(hlc.timestamp)
            .with_context(|| format!("Failed to format GT timestamp for HLC {}", hlc))?;

        Ok(Condition::any()
            .add(Self::updated_at_time_column().gt(timestamp_str.clone()))
            .add(
                Self::updated_at_time_column()
                    .eq(timestamp_str)
                    .and(Self::updated_at_version_column().gt(hlc.version as i32)), // Cast u32 to i32 if column is INTEGER
            ))
    }

    /// Creates a SeaORM condition for records less than the given HLC.
    fn lt(hlc: &HLC) -> Result<Condition> {
        let timestamp_str = hlc_timestamp_millis_to_rfc3339(hlc.timestamp)
            .with_context(|| format!("Failed to format LT timestamp for HLC {}", hlc))?;

        Ok(Condition::any()
            .add(Self::updated_at_time_column().lt(timestamp_str.clone()))
            .add(
                Self::updated_at_time_column()
                    .eq(timestamp_str)
                    .and(Self::updated_at_version_column().lt(hlc.version as i32)),
            ))
    }

    /// Creates a SeaORM condition for records greater than or equal to the given HLC.
    fn gte(hlc: &HLC) -> Result<Condition> {
        let timestamp_str = hlc_timestamp_millis_to_rfc3339(hlc.timestamp)
            .with_context(|| format!("Failed to format GTE timestamp for HLC {}", hlc))?;

        Ok(Condition::any()
            .add(Self::updated_at_time_column().gt(timestamp_str.clone()))
            .add(
                Self::updated_at_time_column()
                    .eq(timestamp_str)
                    .and(Self::updated_at_version_column().gte(hlc.version as i32)),
            ))
    }

    /// Creates a SeaORM condition for records less than or equal to the given HLC.
    fn lte(hlc: &HLC) -> Result<Condition> {
        let timestamp_str = hlc_timestamp_millis_to_rfc3339(hlc.timestamp)
            .with_context(|| format!("Failed to format LTE timestamp for HLC {}", hlc))?;

        Ok(Condition::any()
            .add(Self::updated_at_time_column().lt(timestamp_str.clone()))
            .add(
                Self::updated_at_time_column()
                    .eq(timestamp_str)
                    .and(Self::updated_at_version_column().lte(hlc.version as i32)),
            ))
    }

    /// Creates a SeaORM condition for records within the given HLC range (inclusive).
    fn between(start_hlc: &HLC, end_hlc: &HLC) -> Result<Condition> {
        // Ensure start <= end for logical consistency, though DB might handle it.
        if start_hlc > end_hlc {
            bail!(
                "Start HLC {} must be less than or equal to End HLC {} for between condition",
                start_hlc,
                end_hlc
            );
        }

        // Use GTE for start and LTE for end
        Ok(Condition::all()
            .add(Self::gte(start_hlc)?)
            .add(Self::lte(end_hlc)?))
    }
}

/// Extension trait for SeaORM queries to add HLC-based ordering.
pub trait HLCQuery: Sized + QueryOrder {
    /// Orders the query results by HLC timestamp and version in ascending order.
    fn order_by_hlc_asc<E>(self) -> Self
    where
        E: EntityTrait + HLCModel,
    {
        self.order_by_asc(E::updated_at_time_column())
            .order_by_asc(E::updated_at_version_column())
    }

    /// Orders the query results by HLC timestamp and version in descending order.
    fn order_by_hlc_desc<E>(self) -> Self
    where
        E: EntityTrait + HLCModel,
    {
        self.order_by_desc(E::updated_at_time_column())
            .order_by_desc(E::updated_at_version_column())
    }
}

// Auto-implement HLCQuery for any type that implements QueryOrder.
impl<T: QueryOrder + Sized> HLCQuery for T {}

/// Calculates the BLAKE3 hash of the data for an HLCRecord Model.
///
/// Serializes the data returned by `data_for_hashing()` into *canonical* JSON format
/// (sorted keys) and then calculates the BLAKE3 hash of the JSON string.
/// Using canonical JSON ensures the hash is consistent regardless of field order during serialization.
pub fn calculate_hash<R>(record: &R) -> Result<String>
where
    R: HLCRecord,
{
    let data = record.data_for_hashing();
    // Use serde_json::to_vec for canonical representation
    let json_bytes =
        serde_json::to_vec(&data).context("Failed to serialize data to canonical JSON bytes")?;

    let mut hasher = Hasher::new();
    hasher.update(&json_bytes);
    let hash_bytes = hasher.finalize();
    Ok(hash_bytes.to_hex().to_string())
}

/// Fetches data created or updated before or at a given HLC, paginated.
pub async fn get_data_before_hlc<E>(
    db: &DatabaseConnection,
    hlc: &HLC,
    page: u64,
    page_size: u64,
) -> Result<Vec<E::Model>>
where
    E: HLCModel + EntityTrait + Sync,
    E::Model: HLCRecord + Send + Sync, // Model needs HLCRecord
    <E as EntityTrait>::Model: Sync,
{
    let paginator = E::find()
        .filter(E::lte(hlc)?)
        .order_by_hlc_desc::<E>() // Often want newest-first within the 'before' range
        .paginate(db, page_size);

    paginator
        .fetch_page(page)
        .await
        .context("Failed to fetch page for get_data_before_hlc")
}

/// Fetches data created or updated after a given HLC, paginated.
pub async fn get_data_after_hlc<E>(
    db: &DatabaseConnection,
    hlc: &HLC,
    page: u64,
    page_size: u64,
) -> Result<Vec<E::Model>>
where
    E: HLCModel + EntityTrait + Sync,
    E::Model: HLCRecord + Send + Sync, // Model needs HLCRecord
    <E as EntityTrait>::Model: Sync,
{
    let paginator = E::find()
        .filter(E::gt(hlc)?)
        .order_by_hlc_asc::<E>() // Usually want oldest-first for 'after' range (sync order)
        .paginate(db, page_size);

    paginator
        .fetch_page(page)
        .await
        .context("Failed to fetch page for get_data_after_hlc")
}

/// Fetches data created or updated within a specified HLC range (inclusive), paginated.
pub async fn get_data_in_hlc_range<E>(
    db: &DatabaseConnection,
    start_hlc: &HLC,
    end_hlc: &HLC,
    page: u64,
    page_size: u64,
) -> Result<Vec<E::Model>>
where
    E: HLCModel + EntityTrait + Sync,
    E::Model: HLCRecord + Send + Sync, // Model needs HLCRecord
    <E as EntityTrait>::Model: Sync,
{
    let paginator = E::find()
        .filter(E::between(start_hlc, end_hlc)?)
        .order_by_hlc_asc::<E>() // Usually want oldest-first within a range
        .paginate(db, page_size);

    paginator
        .fetch_page(page)
        .await
        .context("Failed to fetch page for get_data_in_hlc_range")
}

// Example Usage Helper (if needed, outside of tests)
#[allow(dead_code)]
pub fn create_hlc(ts: u64, v: u32, node_id_str: &str) -> HLC {
    HLC {
        timestamp: ts,
        version: v,
        node_id: Uuid::parse_str(node_id_str).unwrap(),
    }
}
