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
use sea_orm::{entity::prelude::*, Condition, DatabaseConnection, PaginatorTrait, QueryOrder};
use serde::{Deserialize, Serialize};

/// Converts a Unix timestamp (in seconds since epoch) to an RFC3339 formatted string.
///
/// This function takes a u64 representing the number of non-leap seconds since
/// January 1, 1970 0:00:00 UTC (Unix timestamp) and converts it into an RFC3339
/// formatted string representing the corresponding UTC datetime.
///
/// It handles potential errors such as out-of-range timestamps and returns a `Result`
/// to indicate success or failure.
///
/// # Arguments
///
/// * `timestamp` - A u64 representing the Unix timestamp (seconds since epoch).
///
/// # Returns
///
/// * `Result<String, Box<dyn std::error::Error>>` -
///     - `Ok(String)`:  If the conversion is successful, returns a `String` containing
///       the RFC3339 formatted datetime string in UTC.
///     - `Err(Box<dyn std::error::Error>)`: If an error occurs during the conversion,
///       returns an `Err` containing a boxed error trait object describing the error.
///       The possible errors include:
///         - Timestamp out of range: If the provided timestamp is outside the valid range
///           that `chrono` can handle, a "Timestamp out of range" error will be returned.
///
/// # Examples
///
/// ```
/// use timestamp_to_rfc3339::timestamp_to_rfc3339; // Assuming your crate is named timestamp_to_rfc3339
///
/// fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let timestamp: u64 = 1678886400; // Example timestamp (March 15, 2023 00:00:00 UTC)
///     let rfc3339_string = timestamp_to_rfc3339(timestamp)?;
///     println!("RFC3339 String: {}", rfc3339_string); // Output: RFC3339 String: 2023-03-15T00:00:00.000Z
///
///     let invalid_timestamp: u64 = u64::MAX; // A very large timestamp, likely out of range
///     let result = timestamp_to_rfc3339(invalid_timestamp);
///     if let Err(error) = result {
///         eprintln!("Error: {}", error); // Output: Error: Timestamp is out of range
///     }
///
///     Ok(())
/// }
/// ```
///
/// # Errors
///
/// This function can return an error if the provided timestamp is out of the representable range
/// for `chrono::DateTime`.  The error kind will be described in the `Err` variant of the `Result`.
pub fn timestamp_to_rfc3339(timestamp: u64) -> Result<String> {
    let secs = timestamp as i64; // Convert u64 to i64 for chrono::timestamp_opt

    match Utc.timestamp_opt(secs, 0) {
        LocalResult::Single(datetime) => {
            // Format the DateTime object to RFC3339 string.
            // We use ".000Z" to ensure milliseconds are included and the 'Z' for UTC timezone.
            let rfc3339_string = datetime.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
            Ok(rfc3339_string)
        }
        LocalResult::None => {
            bail!("Timestamp is out of range") // Return an error if timestamp is invalid
        }
        LocalResult::Ambiguous(..) => {
            // Ambiguous should not happen for UTC timestamps, but we handle it for completeness.
            bail!("Timestamp is ambiguous (should not happen for UTC)")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_timestamp() -> Result<(), Box<dyn std::error::Error>> {
        let timestamp: u64 = 1678886400; // March 15, 2023 00:00:00 UTC
        let rfc3339_string = timestamp_to_rfc3339(timestamp)?;
        assert_eq!(rfc3339_string, "2023-03-15T00:00:00.000Z");
        Ok(())
    }

    #[test]
    fn test_example_timestamp_from_doc() -> Result<(), Box<dyn std::error::Error>> {
        let timestamp: u64 = 1431648000; // May 15, 2015 00:00:00 UTC from the example
        let rfc3339_string = timestamp_to_rfc3339(timestamp)?;
        assert_eq!(rfc3339_string, "2015-05-15T00:00:00.000Z");
        Ok(())
    }

    #[test]
    fn test_out_of_range_timestamp_max() {
        let invalid_timestamp: u64 = u64::MAX;
        let result = timestamp_to_rfc3339(invalid_timestamp);
        assert!(result.is_err());
        if let Err(error) = result {
            assert_eq!(error.to_string(), "Timestamp is out of range");
        }
    }

    #[test]
    fn test_out_of_range_timestamp_zero_minus_one() {
        let invalid_timestamp: u64 = 0; // Minimum possible u64, but let's try something conceptually before epoch
        let secs: i64 = invalid_timestamp as i64 - 1; // Make it negative
        let invalid_timestamp_neg = secs as u64; // Convert back to u64 (wraps around) - this is a very large u64 representing a negative i64.
        let result = timestamp_to_rfc3339(invalid_timestamp_neg); // Pass the large u64
        assert!(result.is_err());
        if let Err(error) = result {
            assert_eq!(error.to_string(), "Timestamp is out of range");
        }
    }
}

use uuid::Uuid;

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
    /// The string format is expected to be "timestamp-counter-node\_id".
    fn from_str(hlc_str: &str) -> Result<Self> {
        let parts: Vec<&str> = hlc_str.splitn(3, '-').collect();
        if parts.len() != 3 {
            bail!("Invalid HLC string format");
        }

        let timestamp = parts[0]
            .parse::<u64>()
            .context("Invalid timestamp format in HLC")?;
        let counter = u32::from_str_radix(parts[1], 16).context("Invalid counter format in HLC")?;
        let node_id = Uuid::parse_str(parts[2]).context("Invalid node ID format in HLC")?;

        Ok(HLC {
            timestamp,
            version: counter,
            node_id,
        })
    }
}

/// A trait for SeaORM entities that need HLC timestamps and hashing.
///
/// Entities implementing this trait can be tracked for changes over time
/// using Hybrid Logical Clocks and their data can be hashed for integrity checks.
pub trait HLCRecord: EntityTrait + Sized + Send + Sync + 'static {
    /// Returns the HLC timestamp when the record was created.
    ///
    /// Should return `None` if the timestamp is not set.
    fn created_at_hlc(&self) -> Option<HLC>;

    /// Returns the HLC timestamp when the record was last updated.
    ///
    /// Should return `None` if the timestamp is not set.
    fn updated_at_hlc(&self) -> Option<HLC>;

    /// Returns the unique identifier of the record. Typically the primary key.
    fn unique_id(&self) -> i32;

    /// Returns the data of the record as a JSON value for hashing.
    ///
    /// This should include all fields that are relevant for determining
    /// if the record has changed.
    fn data_for_hashing(&self) -> serde_json::Value;

    /// Returns a summary representation of the record data as a JSON value.
    ///
    /// This can be used for lightweight listings or change detection without
    /// needing the full record data. Defaults to `data_for_hashing()` if not specifically implemented.
    fn to_summary(&self) -> serde_json::Value {
        self.data_for_hashing() // Default to full data if summary is not specifically implemented
    }

    /// Returns the full data of the record as a JSON value.
    ///
    /// Defaults to `data_for_hashing()` if not specifically implemented.
    /// Provides access to the complete record data.
    fn full_data(&self) -> serde_json::Value {
        self.data_for_hashing() // Default to data for hashing if full data is not specifically different
    }

    /// Gets the primary key column of the entity.
    fn get_primary_key() -> String;

    /// Gets the column for 'created_at_hlc'.
    fn get_created_at_hlc() -> HLC;

    /// Gets the column for 'updated_at_hlc'.
    fn get_updated_at_hlc() -> HLC;
}

pub trait HLCModel: EntityTrait + Sized + Send + Sync + 'static {
    fn updated_at_time_column() -> Self::Column;
    fn updated_at_version_column() -> Self::Column;

    fn gt(hlc: &HLC) -> Result<Condition> {
        let timestamp = timestamp_to_rfc3339(hlc.timestamp)?;

        Ok(Condition::any()
            .add(Self::updated_at_time_column().gt(&timestamp))
            .add(
                Self::updated_at_time_column()
                    .eq(&timestamp)
                    .and(Self::updated_at_version_column().gt(hlc.version)),
            ))
    }

    fn lt(hlc: &HLC) -> Result<Condition> {
        let timestamp = timestamp_to_rfc3339(hlc.timestamp)?;

        Ok(Condition::any()
            .add(Self::updated_at_time_column().lt(&timestamp))
            .add(
                Self::updated_at_time_column()
                    .eq(&timestamp)
                    .and(Self::updated_at_version_column().lt(hlc.version)),
            ))
    }

    fn gte(hlc: &HLC) -> Result<Condition> {
        let timestamp = timestamp_to_rfc3339(hlc.timestamp)?;

        Ok(Condition::any()
            .add(Self::updated_at_time_column().gt(&timestamp))
            .add(
                Self::updated_at_time_column()
                    .eq(&timestamp)
                    .and(Self::updated_at_version_column().gte(hlc.version)),
            ))
    }

    fn lte(hlc: &HLC) -> Result<Condition> {
        let timestamp = timestamp_to_rfc3339(hlc.timestamp)?;

        Ok(Condition::any()
            .add(Self::updated_at_time_column().lt(&timestamp))
            .add(
                Self::updated_at_time_column()
                    .eq(&timestamp)
                    .and(Self::updated_at_version_column().lte(hlc.version)),
            ))
    }

    fn between(start_hlc: &HLC, end_hlc: &HLC) -> Result<Condition> {
        let start_timestamp = timestamp_to_rfc3339(start_hlc.timestamp)?;
        let end_timestamp = timestamp_to_rfc3339(end_hlc.timestamp)?;

        Ok(Condition::all()
            .add(
                Condition::any()
                    .add(Self::updated_at_time_column().gt(&start_timestamp))
                    .add(
                        Self::updated_at_time_column()
                            .eq(&start_timestamp)
                            .and(Self::updated_at_version_column().gte(start_hlc.version)),
                    ),
            )
            .add(
                Condition::any()
                    .add(Self::updated_at_time_column().lt(&end_timestamp))
                    .add(
                        Self::updated_at_time_column()
                            .eq(&end_timestamp)
                            .and(Self::updated_at_version_column().lte(end_hlc.version)),
                    ),
            ))
    }
}

/// Calculates the BLAKE3 hash of the data for an HLCRecord.
///
/// Serializes the data returned by `data_for_hashing()` into JSON format
/// and then calculates the BLAKE3 hash of the JSON string.
pub fn calculate_hash<R: HLCRecord>(record: &R) -> Result<String> {
    let data = record.data_for_hashing();
    let json_string = serde_json::to_string(&data).context("Failed to serialize data to JSON")?;

    let mut hasher = Hasher::new();
    hasher.update(json_string.as_bytes());
    let hash_bytes = hasher.finalize();
    Ok(hash_bytes.to_hex().to_string())
}

/// Fetches data created or updated before or at a given HLC, paginated.
///
/// Retrieves records from the database that have an 'updated_at_hlc' value
/// less than or equal to the specified HLC. Results are paginated.
///
/// # Type Parameters
///
/// *   `R`: Must implement the `HLCRecord` trait and be convertible to `ActiveModelTrait`.
///
/// # Arguments
///
/// *   `db`: A database connection.
/// *   `hlc`: The HLC timestamp to compare against (inclusive).
/// *   `page`: The page number to retrieve (starting from 0).
/// *   `page_size`: The number of items per page.
/// *   `include_full_data`: A boolean indicating whether to include full data in the response (currently not used in query).
///
/// # Returns
///
/// A `Result` containing a vector of `HLCRecord` items or an error.
pub async fn get_data_before_hlc<R>(
    db: &DatabaseConnection,
    hlc: &HLC,
    page: u64,
    page_size: u64,
) -> Result<Vec<R::Model>>
where
    R: HLCModel + EntityTrait + Sync,
    <R as EntityTrait>::Model: Sync,
{
    let column = R::updated_at_time_column();

    let paginator = R::find()
        .filter(R::lte(hlc)?)
        .order_by_asc(column)
        .paginate(db, page_size);

    let results = paginator.fetch_page(page).await?;

    Ok(results)
}

/// Fetches data created or updated after a given HLC, paginated.
///
/// Retrieves records from the database that have an 'updated_at_hlc' value
/// greater than the specified HLC. Results are paginated.
///
/// # Type Parameters
///
/// *   `R`: Must implement the `HLCRecord` trait and be convertible to `ActiveModelTrait`.
///
/// # Arguments
///
/// *   `db`: A database connection.
/// *   `hlc`: The HLC timestamp to compare against (exclusive).
/// *   `page`: The page number to retrieve (starting from 0).
/// *   `page_size`: The number of items per page.
/// *   `include_full_data`: A boolean indicating whether to include full data in the response (currently not used in query).
///
/// # Returns
///
/// A `Result` containing a vector of `HLCRecord` items or an error.
pub async fn get_data_after_hlc<R>(
    db: &DatabaseConnection,
    hlc: &HLC,
    page: u64,
    page_size: u64,
) -> Result<Vec<R::Model>>
where
    R: HLCModel + EntityTrait + Sync,
    <R as EntityTrait>::Model: Sync,
{
    let column = R::updated_at_time_column();

    let paginator = R::find()
        .filter(R::gt(hlc)?)
        .order_by_asc(column)
        .paginate(db, page_size);

    let results = paginator.fetch_page(page).await?;

    Ok(results)
}

/// Fetches data created or updated within a specified HLC range.
///
/// Retrieves records from the database that have an 'updated_at_hlc' value
/// within the range defined by `start_hlc` and `end_hlc` (inclusive).
///
/// # Type Parameters
///
/// *   `R`: Must implement the `HLCRecord` trait and be convertible to `ActiveModelTrait`.
///
/// # Arguments
///
/// *   `db`: A database connection.
/// *   `start_hlc`: The starting HLC timestamp of the range (inclusive).
/// *   `end_hlc`: The ending HLC timestamp of the range (inclusive).
/// *   `include_full_data`: A boolean indicating whether to include full data in the response (currently not used in query).
///
/// # Returns
///
/// A `Result` containing a vector of `HLCRecord` items or an error.
pub async fn get_data_in_hlc_range<R>(
    db: &DatabaseConnection,
    start_hlc: &HLC,
    end_hlc: &HLC,
    page: u64,
    page_size: u64,
) -> Result<Vec<R::Model>>
where
    R: HLCModel + EntityTrait + Sync,
    <R as EntityTrait>::Model: Sync,
{
    let column = R::updated_at_time_column();

    let paginator = R::find()
        .filter(R::between(start_hlc, end_hlc)?)
        .order_by_asc(column)
        .paginate(db, page_size);

    let results = paginator.fetch_page(page).await?;

    Ok(results)
}
