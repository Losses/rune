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
    cmp::{self, Ordering},
    str::FromStr,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use blake3::Hasher;
use chrono::DateTime;
use sea_orm::{
    entity::prelude::*, Condition, DatabaseConnection, DeleteResult, FromQueryResult,
    PaginatorTrait, QueryFilter, QueryOrder,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Represents a Hybrid Logical Clock (HLC).
///
/// An HLC combines a physical timestamp with a logical counter to ensure
/// monotonically increasing timestamps across a distributed system, even
/// with clock skew.
#[derive(Clone, Debug, Eq, Serialize, Deserialize)]
pub struct HLC {
    /// Physical timestamp component, in milliseconds since the Unix epoch.
    pub timestamp_ms: u64,
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
            self.timestamp_ms, self.version, self.node_id
        )
    }
}

impl PartialEq for HLC {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp_ms == other.timestamp_ms
            && self.version == other.version
            && self.node_id == other.node_id
    }
}

impl PartialOrd for HLC {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HLC {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        (self.timestamp_ms, self.version, self.node_id).cmp(&(
            other.timestamp_ms,
            other.version,
            other.node_id,
        ))
    }
}

impl HLC {
    /// Creates an initial HLC with timestamp 0 and counter 0.
    ///
    /// This is often used as a starting point or default value.
    pub fn new(node_id: Uuid) -> Self {
        HLC {
            timestamp_ms: 0,
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

        let (timestamp, counter) = match current_timestamp.cmp(&last_hlc.timestamp_ms) {
            Ordering::Greater => (current_timestamp, 0),
            Ordering::Equal => {
                if last_hlc.version == u32::MAX {
                    panic!(
                        "HLC counter overflow detected within a single millisecond. Timestamp: {}, Node: {}",
                        current_timestamp,
                        context.node_id
                    );
                }
                (current_timestamp, last_hlc.version + 1)
            }
            Ordering::Less => {
                if last_hlc.version == u32::MAX {
                    (last_hlc.timestamp_ms + 1, 0)
                } else {
                    (last_hlc.timestamp_ms, last_hlc.version + 1)
                }
            }
        };

        let new_hlc = HLC {
            timestamp_ms: timestamp,
            version: counter,
            node_id: context.node_id,
        };
        *last_hlc = new_hlc.clone();
        new_hlc
    }

    /// Increments the HLC logically by one step (version or timestamp).
    ///
    /// If the version counter is less than `u32::MAX`, it's incremented.
    /// If the version counter reaches `u32::MAX`, the timestamp is incremented
    /// by one millisecond, and the version resets to 0. This maintains the HLC ordering.
    ///
    /// **Note:** This method provides deterministic logical incrementing, primarily
    /// intended for testing or specific simulation scenarios where controlling the
    /// exact sequence of HLCs is needed, independent of the system clock used by `generate()`.
    /// For generating standard monotonic HLCs tied to physical time progression,
    /// use `HLC::generate()`. Avoid using this in production code where standard HLC
    /// generation based on physical time is required.
    pub fn increment(&mut self) {
        if self.version < u32::MAX {
            self.version += 1;
        } else {
            // Overflow: Increment timestamp and reset version
            // We assume timestamp itself won't realistically overflow u64.
            self.timestamp_ms += 1;
            self.version = 0;
        }
    }

    /// Converts the HLC timestamp to RFC3339 format.
    ///
    /// Returns a string representation of the timestamp in RFC3339 format (ISO 8601).
    /// The timestamp is interpreted as milliseconds since the Unix epoch.
    ///
    /// # Examples
    ///
    /// ```
    /// use sync::hlc::HLC;
    /// use uuid::Uuid;
    ///
    /// let hlc = HLC {
    ///     timestamp: 1640995200000, // 2022-01-01T00:00:00Z
    ///     version: 1,
    ///     node_id: Uuid::new_v4(),
    /// };
    /// assert_eq!(hlc.to_rfc3339(), "2022-01-01T00:00:00+00:00");
    /// ```
    pub fn to_rfc3339(&self) -> String {
        DateTime::from_timestamp_millis(self.timestamp_ms as i64)
            .unwrap_or_else(|| DateTime::from_timestamp(0, 0).unwrap())
            .to_rfc3339()
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
            timestamp_ms: timestamp,
            version: counter,
            node_id,
        })
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

    /// Creates a context with a specific initial HLC (useful for testing).
    #[cfg(test)]
    fn with_initial_hlc(node_id: Uuid, initial_hlc: HLC) -> Self {
        SyncTaskContext {
            node_id,
            last_hlc: Mutex::new(initial_hlc),
        }
    }

    pub fn generate_hlc(&self) -> HLC {
        HLC::generate(self)
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
#[async_trait]
pub trait HLCModel: EntityTrait + Sized + Send + Sync + 'static {
    /// Returns the SeaORM column definition for the HLC timestamp component.
    fn updated_at_time_column() -> Self::Column;

    /// Returns the SeaORM column definition for the HLC version/counter component.
    fn updated_at_version_column() -> Self::Column;

    /// Returns the SeaORM column definition for the node ID.
    fn updated_at_node_id_column() -> Self::Column;

    /// Returns the SeaORM column definition for the unique identifier.
    fn unique_id_column() -> Self::Column;

    /// Creates a SeaORM condition for records strictly greater than the given HLC.
    fn gt(hlc: &HLC) -> Result<Condition> {
        let ts_str = hlc.to_rfc3339();
        let ver_val = hlc.version as i32;
        let nid_str = hlc.node_id.to_string();

        Ok(Condition::any()
            // (ts > hlc.ts)
            .add(Self::updated_at_time_column().gt(ts_str.clone()))
            // OR (ts == hlc.ts AND ver > hlc.ver)
            .add(
                Condition::all()
                    .add(Self::updated_at_time_column().eq(ts_str.clone()))
                    .add(Self::updated_at_version_column().gt(ver_val)),
            )
            // OR (ts == hlc.ts AND ver == hlc.ver AND nid > hlc.nid)
            .add(
                Condition::all()
                    .add(Self::updated_at_time_column().eq(ts_str))
                    .add(Self::updated_at_version_column().eq(ver_val))
                    .add(Self::updated_at_node_id_column().gt(nid_str)),
            ))
    }

    /// Creates a SeaORM condition for records less than the given HLC.
    fn lt(hlc: &HLC) -> Result<Condition> {
        let ts_str = hlc.to_rfc3339();
        let ver_val = hlc.version as i32;
        let nid_str = hlc.node_id.to_string();

        Ok(Condition::any()
            // (ts < hlc.ts)
            .add(Self::updated_at_time_column().lt(ts_str.clone()))
            // OR (ts == hlc.ts AND ver < hlc.ver)
            .add(
                Condition::all()
                    .add(Self::updated_at_time_column().eq(ts_str.clone()))
                    .add(Self::updated_at_version_column().lt(ver_val)),
            )
            // OR (ts == hlc.ts AND ver == hlc.ver AND nid < hlc.nid)
            .add(
                Condition::all()
                    .add(Self::updated_at_time_column().eq(ts_str))
                    .add(Self::updated_at_version_column().eq(ver_val))
                    .add(Self::updated_at_node_id_column().lt(nid_str)),
            ))
    }

    /// Creates a SeaORM condition for records greater than or equal to the given HLC.
    fn gte(hlc: &HLC) -> Result<Condition> {
        Ok(<Self as HLCModel>::lt(hlc)?.not())
    }

    /// Creates a SeaORM condition for records less than or equal to the given HLC.
    fn lte(hlc: &HLC) -> Result<Condition> {
        Ok(<Self as HLCModel>::gt(hlc)?.not())
    }

    /// Creates a SeaORM condition for records within the given HLC range (inclusive).
    fn between(start_hlc: &HLC, end_hlc: &HLC) -> Result<Condition> {
        if start_hlc > end_hlc {
            bail!(
                "Start HLC {} must be less than or equal to End HLC {}",
                start_hlc,
                end_hlc
            );
        }

        if start_hlc == end_hlc {
            // Handle the single HLC case explicitly
            let ts_str = start_hlc.to_rfc3339();
            return Ok(Condition::all()
                .add(Self::updated_at_time_column().eq(ts_str))
                .add(Self::updated_at_version_column().eq(start_hlc.version as i32))
                .add(Self::updated_at_node_id_column().eq(start_hlc.node_id.to_string())));
        }

        // Handle the range case: >= start AND <= end
        let start_cond = Self::gte(start_hlc)?;
        let end_cond = Self::lte(end_hlc)?;

        Ok(Condition::all().add(start_cond).add(end_cond))
    }

    /// Finds a single record by its unique_id (typically sync_id).
    async fn find_by_unique_id<C>(unique_id_value: &str, db: &C) -> Result<Option<Self::Model>>
    where
        C: ConnectionTrait,
    {
        Self::find()
            .filter(Self::unique_id_column().eq(unique_id_value))
            .one(db)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to find record by unique ID: {}", e))
    }

    /// Deletes a single record by its unique_id.
    async fn delete_by_unique_id<C>(unique_id_value: &str, db: &C) -> Result<DeleteResult>
    where
        C: ConnectionTrait,
    {
        Self::delete_many()
            .filter(Self::unique_id_column().eq(unique_id_value))
            .exec(db)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete record by unique ID: {}", e))
    }
}

#[cfg(test)]
mod hlcmodel_tests {
    use crate::{
        chunking::tests::test_model_def::Entity,
        hlc::{create_hlc, HLCModel, HLC},
    };
    use anyhow::Result;
    use sea_orm::{Condition, DbBackend, EntityTrait, QueryFilter, QueryTrait, Statement, Value};
    use uuid::Uuid;

    // Helper to build SQL WHERE clause and get values for verification
    fn condition_to_sql(condition: Condition) -> (String, Vec<Value>) {
        // Use the specific entity the tests are based on
        let statement: Statement = Entity::find().filter(condition).build(DbBackend::Sqlite);

        let sql = statement.sql;
        let values = statement.values.map(|v| v.0).unwrap_or_default();

        // Extract the part after WHERE, handling potential absence of WHERE clause
        let where_clause = sql
            .split_once("WHERE")
            .map(|(_, clause)| clause.trim().to_string())
            .unwrap_or_default();

        (where_clause, values)
    }

    // Helper function to create expected values Vec<Value>
    fn expected_values(ts: &str, v: i32) -> Vec<Value> {
        vec![
            Value::from(ts.to_string()), // gt/lt ts
            Value::from(ts.to_string()), // eq ts
            Value::from(v),              // gt/lt/gte/lte v
        ]
    }

    // Helper to create expected values for between
    fn expected_between_values(
        start_ts: &str,
        start_v: i32,
        end_ts: &str,
        end_v: i32,
    ) -> Vec<Value> {
        vec![
            Value::from(start_ts.to_string()), // gte: gt start_ts
            Value::from(start_ts.to_string()), // gte: eq start_ts
            Value::from(start_v),              // gte: gte start_v
            Value::from(end_ts.to_string()),   // lte: lt end_ts
            Value::from(end_ts.to_string()),   // lte: eq end_ts
            Value::from(end_v),                // lte: lte end_v
        ]
    }

    // Helper to normalize SQL whitespace and quotes for comparison
    fn normalize_sql(sql: &str) -> String {
        // Replace different quotes with a standard one, remove extra whitespace
        sql.replace('`', "\"") // Example: handle backticks if needed
            .replace(['(', ')'], " ") // Add spaces around parentheses for splitting
            .split_whitespace()
            .collect::<Vec<&str>>()
            .join(" ")
            .replace(" . ", ".") // Correct dot spacing if split created it
            .replace(" \"", "\"") // Correct quote spacing
            .replace("\" ", "\"")
            // Add more specific normalizations if needed based on actual SeaORM output
            .trim()
            .to_string()
    }

    #[test]
    fn test_hlcmodel_gt() -> Result<()> {
        let node_id = Uuid::new_v4();
        let hlc = create_hlc(1678886400123, 5, &node_id.to_string());
        let expected_ts_str = hlc.to_rfc3339();
        let expected_version = hlc.version as i32;

        let condition = Entity::gt(&hlc)?;
        let (sql_where, values) = condition_to_sql(condition);

        // Note: SeaORM >= 0.11 might produce slightly different SQL structures (e.g. extra parens)
        // Check the actual output and adjust expected_sql accordingly, or use normalize_sql
        let expected_sql = r#"("mock_tasks"."updated_at_hlc_ts" > ?) OR ("mock_tasks"."updated_at_hlc_ts" = ? AND "mock_tasks"."updated_at_hlc_v" > ?)"#;
        let expected_vals = expected_values(&expected_ts_str, expected_version);

        println!("Generated SQL WHERE (GT): {}", sql_where);
        println!("Generated Values (GT): {:?}", values);
        println!("Expected SQL: {}", expected_sql);
        println!("Expected Values: {:?}", expected_vals);

        assert_eq!(
            normalize_sql(&sql_where),
            normalize_sql(expected_sql),
            "SQL structure mismatch"
        );
        assert_eq!(values, expected_vals, "Values mismatch");

        Ok(())
    }

    #[test]
    fn test_hlcmodel_lt() -> Result<()> {
        let node_id = Uuid::new_v4();
        let hlc = create_hlc(1678886400123, 5, &node_id.to_string());
        let expected_ts_str = hlc.to_rfc3339();
        let expected_version = hlc.version as i32;

        let condition = Entity::lt(&hlc)?;
        let (sql_where, values) = condition_to_sql(condition);

        let expected_sql = r#"("mock_tasks"."updated_at_hlc_ts" < ?) OR ("mock_tasks"."updated_at_hlc_ts" = ? AND "mock_tasks"."updated_at_hlc_v" < ?)"#;
        let expected_vals = expected_values(&expected_ts_str, expected_version);

        println!("Generated SQL WHERE (LT): {}", sql_where);
        println!("Generated Values (LT): {:?}", values);
        println!("Expected SQL: {}", expected_sql);
        println!("Expected Values: {:?}", expected_vals);

        assert_eq!(
            normalize_sql(&sql_where),
            normalize_sql(expected_sql),
            "SQL structure mismatch"
        );
        assert_eq!(values, expected_vals, "Values mismatch");

        Ok(())
    }

    #[test]
    fn test_hlcmodel_gte() -> Result<()> {
        let node_id = Uuid::new_v4();
        let hlc = create_hlc(1678886400123, 5, &node_id.to_string());
        let expected_ts_str = hlc.to_rfc3339();
        let expected_version = hlc.version as i32;

        let condition = Entity::gte(&hlc)?;
        let (sql_where, values) = condition_to_sql(condition);

        let expected_sql = r#"("mock_tasks"."updated_at_hlc_ts" > ?) OR ("mock_tasks"."updated_at_hlc_ts" = ? AND "mock_tasks"."updated_at_hlc_v" >= ?)"#;
        let expected_vals = expected_values(&expected_ts_str, expected_version);

        println!("Generated SQL WHERE (GTE): {}", sql_where);
        println!("Generated Values (GTE): {:?}", values);
        println!("Expected SQL: {}", expected_sql);
        println!("Expected Values: {:?}", expected_vals);

        assert_eq!(
            normalize_sql(&sql_where),
            normalize_sql(expected_sql),
            "SQL structure mismatch"
        );
        assert_eq!(values, expected_vals, "Values mismatch");

        Ok(())
    }

    #[test]
    fn test_hlcmodel_lte() -> Result<()> {
        let node_id = Uuid::new_v4();
        let hlc = create_hlc(1678886400123, 5, &node_id.to_string());
        let expected_ts_str = hlc.to_rfc3339();
        let expected_version = hlc.version as i32;

        let condition = Entity::lte(&hlc)?;
        let (sql_where, values) = condition_to_sql(condition);

        let expected_sql = r#"("mock_tasks"."updated_at_hlc_ts" < ?) OR ("mock_tasks"."updated_at_hlc_ts" = ? AND "mock_tasks"."updated_at_hlc_v" <= ?)"#;
        let expected_vals = expected_values(&expected_ts_str, expected_version);

        println!("Generated SQL WHERE (LTE): {}", sql_where);
        println!("Generated Values (LTE): {:?}", values);
        println!("Expected SQL: {}", expected_sql);
        println!("Expected Values: {:?}", expected_vals);

        assert_eq!(
            normalize_sql(&sql_where),
            normalize_sql(expected_sql),
            "SQL structure mismatch"
        );
        assert_eq!(values, expected_vals, "Values mismatch");

        Ok(())
    }

    #[test]
    fn test_hlcmodel_between() -> Result<()> {
        let node_id = Uuid::new_v4();
        let start_hlc = create_hlc(1678886400000, 1, &node_id.to_string());
        let end_hlc = create_hlc(1678886400123, 5, &node_id.to_string());
        let start_ts_str = start_hlc.to_rfc3339();
        let end_ts_str = end_hlc.to_rfc3339();
        let start_version = start_hlc.version as i32;
        let end_version = end_hlc.version as i32;

        let condition = Entity::between(&start_hlc, &end_hlc)?;
        let (sql_where, values) = condition_to_sql(condition);

        // GTE part: ("mock_tasks"."updated_at_hlc_ts" > ?) OR ("mock_tasks"."updated_at_hlc_ts" = ? AND "mock_tasks"."updated_at_hlc_v" >= ?)
        // LTE part: ("mock_tasks"."updated_at_hlc_ts" < ?) OR ("mock_tasks"."updated_at_hlc_ts" = ? AND "mock_tasks"."updated_at_hlc_v" <= ?)
        // Combined: ( GTE part ) AND ( LTE part )
        let expected_sql = r#"(("mock_tasks"."updated_at_hlc_ts" > ?) OR ("mock_tasks"."updated_at_hlc_ts" = ? AND "mock_tasks"."updated_at_hlc_v" >= ?)) AND (("mock_tasks"."updated_at_hlc_ts" < ?) OR ("mock_tasks"."updated_at_hlc_ts" = ? AND "mock_tasks"."updated_at_hlc_v" <= ?))"#;
        let expected_vals =
            expected_between_values(&start_ts_str, start_version, &end_ts_str, end_version);

        println!("Generated SQL WHERE (Between): {}", sql_where);
        println!("Generated Values (Between): {:?}", values);
        println!("Expected SQL: {}", expected_sql);
        println!("Expected Values: {:?}", expected_vals);

        assert_eq!(
            normalize_sql(&sql_where),
            normalize_sql(expected_sql),
            "SQL structure mismatch"
        );
        assert_eq!(values, expected_vals, "Values mismatch");

        Ok(())
    }

    #[test]
    fn test_hlcmodel_between_same_hlc() -> Result<()> {
        let node_id = Uuid::new_v4();
        let hlc = create_hlc(1678886400123, 5, &node_id.to_string());
        let ts_str = hlc.to_rfc3339();
        let version = hlc.version as i32;

        let condition = Entity::between(&hlc, &hlc)?;
        let (sql_where, values) = condition_to_sql(condition);

        let expected_sql = r#"(("mock_tasks"."updated_at_hlc_ts" > ?) OR ("mock_tasks"."updated_at_hlc_ts" = ? AND "mock_tasks"."updated_at_hlc_v" >= ?)) AND (("mock_tasks"."updated_at_hlc_ts" < ?) OR ("mock_tasks"."updated_at_hlc_ts" = ? AND "mock_tasks"."updated_at_hlc_v" <= ?))"#;
        let expected_vals = expected_between_values(&ts_str, version, &ts_str, version);

        println!("Generated SQL WHERE (Between Same HLC): {}", sql_where);
        println!("Generated Values (Between Same HLC): {:?}", values);
        println!("Expected SQL: {}", expected_sql);
        println!("Expected Values: {:?}", expected_vals);

        assert_eq!(
            normalize_sql(&sql_where),
            normalize_sql(expected_sql),
            "SQL structure mismatch"
        );
        assert_eq!(values, expected_vals, "Values mismatch");

        Ok(())
    }

    #[test]
    fn test_hlcmodel_between_invalid_range() {
        let node_id = Uuid::new_v4();
        let start_hlc = create_hlc(1678886400123, 5, &node_id.to_string());
        let end_hlc = create_hlc(1678886400000, 1, &node_id.to_string()); // End < Start

        let result = Entity::between(&start_hlc, &end_hlc);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Start HLC"));
        assert!(err_msg.contains("must be less than or equal to End HLC"));
        assert!(err_msg.contains(&start_hlc.to_string()));
        assert!(err_msg.contains(&end_hlc.to_string()));
    }

    #[test]
    fn test_hlcmodel_timestamp_conversion_error() {
        let node_id = Uuid::new_v4();
        let invalid_hlc = HLC {
            timestamp_ms: u64::MAX, // Known out-of-range value for chrono
            version: 0,
            node_id,
        };

        let result = Entity::gt(&invalid_hlc);

        assert!(result.is_err());
        let err = result.err().unwrap();
        let top_level_msg = err.to_string(); // Message potentially including context

        println!("Timestamp conversion error message: {}", top_level_msg);
        println!("Timestamp conversion error chain: {:?}", err); // Print full chain for debugging

        // Check the context message added by with_context() which is reliable
        assert!(
            top_level_msg.contains(&format!(
                "Failed to format GT timestamp for HLC {}",
                invalid_hlc
            )),
            "Error message should contain the context added in HLCModel::gt"
        );

        // Check the root cause message. This depends on the exact error from hlc_timestamp_millis_to_rfc3339
        // It might be "timestamp out of range", "value too large", etc.
        // Make the check more general or adapt to the specific message if known.
        let root_cause_msg = err.root_cause().to_string();
        println!("Root cause: {}", root_cause_msg);
        assert!(
            // Check for common keywords related to range errors
            root_cause_msg.contains("out of range")
                || root_cause_msg.contains("invalid")
                || root_cause_msg.contains("value too large"),
            "Root cause should indicate an out-of-range or invalid timestamp. Root cause: {}",
            root_cause_msg
        );
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
    E::Model: HLCRecord + FromQueryResult + Send + Sync,
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
    E::Model: HLCRecord + FromQueryResult + Send + Sync,
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
    E::Model: HLCRecord + FromQueryResult + Send + Sync,
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

#[cfg(test)]
pub fn create_hlc(ts: u64, v: u32, node_id_str: &str) -> HLC {
    HLC {
        timestamp_ms: ts,
        version: v,
        node_id: Uuid::parse_str(node_id_str).unwrap(),
    }
}

#[cfg(test)]
mod hlc_increment_tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_hlc_increment_normal() {
        let node_id = Uuid::new_v4();
        let mut hlc = HLC {
            timestamp_ms: 1000,
            version: 5,
            node_id,
        };
        hlc.increment();
        assert_eq!(hlc.timestamp_ms, 1000);
        assert_eq!(hlc.version, 6);
        assert_eq!(hlc.node_id, node_id);
    }

    #[test]
    fn test_hlc_increment_version_max() {
        let node_id = Uuid::new_v4();
        let mut hlc = HLC {
            timestamp_ms: 1000,
            version: u32::MAX - 1,
            node_id,
        };
        hlc.increment();
        assert_eq!(hlc.timestamp_ms, 1000);
        assert_eq!(hlc.version, u32::MAX);
        assert_eq!(hlc.node_id, node_id);
    }

    #[test]
    fn test_hlc_increment_overflow() {
        let node_id = Uuid::new_v4();
        let mut hlc = HLC {
            timestamp_ms: 1000,
            version: u32::MAX,
            node_id,
        };
        hlc.increment();
        assert_eq!(
            hlc.timestamp_ms, 1001,
            "Timestamp should increment on version overflow"
        );
        assert_eq!(hlc.version, 0, "Version should reset to 0 on overflow");
        assert_eq!(hlc.node_id, node_id);
    }

    #[test]
    fn test_hlc_increment_multiple_overflows() {
        let node_id = Uuid::new_v4();
        let mut hlc = HLC {
            timestamp_ms: 1000,
            version: u32::MAX - 1,
            node_id,
        };

        // Increment to MAX
        hlc.increment();
        assert_eq!(hlc.timestamp_ms, 1000);
        assert_eq!(hlc.version, u32::MAX);

        // Increment causing overflow
        hlc.increment();
        assert_eq!(hlc.timestamp_ms, 1001);
        assert_eq!(hlc.version, 0);

        // Normal increment after overflow
        hlc.increment();
        assert_eq!(hlc.timestamp_ms, 1001);
        assert_eq!(hlc.version, 1);
    }
}

#[cfg(test)]
mod hlc_generate_tests {
    use super::*;
    use std::thread::sleep;

    use chrono::Duration;

    fn get_current_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    #[test]
    fn test_generate_time_moves_forward() {
        let node_id = Uuid::new_v4();
        let initial_ts = get_current_millis();
        let initial_hlc = create_hlc(initial_ts, 5, &node_id.to_string());
        let context = SyncTaskContext::with_initial_hlc(node_id, initial_hlc.clone());

        // Wait for time to likely advance
        sleep(Duration::milliseconds(5).to_std().unwrap());

        let new_hlc = context.generate_hlc();
        let after_ts = get_current_millis(); // Timestamp might advance slightly during test

        assert!(
            new_hlc.timestamp_ms > initial_hlc.timestamp_ms,
            "New timestamp should be greater"
        );
        assert!(
            new_hlc.timestamp_ms >= initial_ts + 5,
            "New timestamp should be roughly current time"
        );
        assert!(
            new_hlc.timestamp_ms <= after_ts,
            "New timestamp should be roughly current time"
        );
        assert_eq!(new_hlc.version, 0, "Counter should reset");
        assert_eq!(new_hlc.node_id, node_id);

        // Check last_hlc was updated
        assert_eq!(*context.last_hlc.lock().unwrap(), new_hlc);
    }

    #[test]
    fn test_generate_time_stays_same() {
        let node_id = Uuid::new_v4();
        // Simulate by setting last_hlc's time to *now* just before generating
        let current_ts = get_current_millis();
        let initial_hlc = create_hlc(current_ts, 5, &node_id.to_string());
        let context = SyncTaskContext::with_initial_hlc(node_id, initial_hlc.clone());

        let new_hlc = context.generate_hlc();

        // It's possible the millisecond ticked over between get_current_millis and generate()
        if new_hlc.timestamp_ms == initial_hlc.timestamp_ms {
            assert_eq!(
                new_hlc.version,
                initial_hlc.version + 1,
                "Counter should increment if timestamp is the same"
            );
        } else {
            // If time advanced, counter should reset
            assert!(
                new_hlc.timestamp_ms > initial_hlc.timestamp_ms,
                "Timestamp advanced"
            );
            assert_eq!(
                new_hlc.version, 0,
                "Counter should reset if timestamp advanced"
            );
        }
        assert_eq!(new_hlc.node_id, node_id);
        assert_eq!(*context.last_hlc.lock().unwrap(), new_hlc);
    }

    #[test]
    fn test_generate_clock_skew() {
        let node_id = Uuid::new_v4();
        // Set last_hlc to be in the future relative to current system time
        let future_ts = get_current_millis() + 10000; // 10 seconds in the future
        let initial_hlc = create_hlc(future_ts, 5, &node_id.to_string());
        let context = SyncTaskContext::with_initial_hlc(node_id, initial_hlc.clone());

        let new_hlc = context.generate_hlc();

        assert_eq!(
            new_hlc.timestamp_ms, initial_hlc.timestamp_ms,
            "Timestamp should use the last HLC's timestamp during skew"
        );
        assert_eq!(
            new_hlc.version,
            initial_hlc.version + 1,
            "Counter should increment during skew"
        );
        assert_eq!(new_hlc.node_id, node_id);
        assert_eq!(*context.last_hlc.lock().unwrap(), new_hlc);
    }

    #[test]
    fn test_generate_time_stays_same_explicit() {
        let node_id = Uuid::new_v4();
        // Simulate by setting last_hlc's time to *now* just before generating
        let current_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let initial_version = 5u32;
        let initial_hlc = HLC {
            timestamp_ms: current_ts,
            version: initial_version,
            node_id,
        };
        let context = SyncTaskContext::with_initial_hlc(node_id, initial_hlc.clone());

        // Try to generate within the same millisecond
        let new_hlc = context.generate_hlc();

        // Check the Ordering::Equal case explicitly
        if new_hlc.timestamp_ms == initial_hlc.timestamp_ms {
            assert_eq!(
                new_hlc.version,
                initial_hlc.version + 1,
                "Counter should increment if timestamp is the same"
            );
        } else {
            // If time advanced, counter should reset (Ordering::Greater case)
            assert!(
                new_hlc.timestamp_ms > initial_hlc.timestamp_ms,
                "Timestamp advanced unexpectedly fast, test assumption failed or different code path taken."
            );
            assert_eq!(
                new_hlc.version, 0,
                "Counter should reset if timestamp advanced"
            );
        }
        assert_eq!(new_hlc.node_id, node_id);
        assert_eq!(*context.last_hlc.lock().unwrap(), new_hlc);
    }

    #[test]
    fn test_generate_clock_skew_with_counter_overflow() {
        let node_id = Uuid::new_v4();
        // Set last_hlc to be in the future relative to current system time
        let future_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 10000; // 10 seconds in the future
        let initial_hlc = HLC {
            timestamp_ms: future_ts,
            version: u32::MAX, // Set counter to MAX
            node_id,
        };
        let context = SyncTaskContext::with_initial_hlc(node_id, initial_hlc.clone());

        let new_hlc = context.generate_hlc();

        // Clock skew detected (Ordering::Less), counter was MAX.
        // Should increment timestamp and reset counter.
        assert_eq!(
            new_hlc.timestamp_ms,
            initial_hlc.timestamp_ms + 1, // Timestamp increments because counter overflowed
            "Timestamp should increment due to counter overflow during skew"
        );
        assert_eq!(
            new_hlc.version,
            0, // Counter resets
            "Counter should reset to 0 after overflow during skew"
        );
        assert_eq!(new_hlc.node_id, node_id);
        assert_eq!(*context.last_hlc.lock().unwrap(), new_hlc);
    }

    #[test]
    #[should_panic(expected = "HLC counter overflow detected")]
    #[cfg_attr(
        tarpaulin,
        ignore = "Flaky timing-dependent test under instrumentation"
    )]
    fn test_generate_counter_overflow_panic() {
        // This test is tricky because it relies on SystemTime::now() not advancing
        // between the setup and the call to generate(). It might be flaky.
        let node_id = Uuid::new_v4();
        let current_ts = get_current_millis();
        let initial_hlc = HLC {
            timestamp_ms: current_ts,
            version: u32::MAX, // Set version to max
            node_id,
        };
        let context = SyncTaskContext::with_initial_hlc(node_id, initial_hlc.clone());

        // Immediately generate. If the clock *doesn't* tick, the panic should occur.
        // If the clock *does* tick, timestamp increases, counter resets to 0, no panic.
        // The should_panic expects the panic. If it doesn't panic, the test fails.
        let _new_hlc = context.generate_hlc();
        // We might need a slight delay *after* setting initial_hlc to ensure generate()
        // sees the same timestamp, but that makes the test less reliable.
        // Let's assume for testing purposes the clock might not advance instantly.

        // Alternative: Mock SystemTime, but that requires external crates or complex setup.
        // Given the rarity of this condition, relying on the panic safeguard and the
        // deterministic `increment` test for overflow logic is often sufficient.
    }
}

#[cfg(test)]
mod hlc_from_str_tests {
    use super::*;

    #[test]
    fn test_from_str_valid() -> Result<()> {
        let node_id = Uuid::new_v4();
        let hlc_string = format!("1234567890123-0000abcd-{}", node_id);
        let hlc = HLC::from_str(&hlc_string)?;

        assert_eq!(hlc.timestamp_ms, 1234567890123);
        assert_eq!(hlc.version, 0xabcd);
        assert_eq!(hlc.node_id, node_id);
        Ok(())
    }

    #[test]
    fn test_from_str_invalid_format_parts() {
        let result = HLC::from_str("12345-abcd"); // Too few parts
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Invalid HLC string format"));
        assert!(err_msg.contains("expected 'timestamp-counterHex-node_id'"));
    }

    #[test]
    fn test_from_str_invalid_timestamp() {
        let node_id = Uuid::new_v4();
        let hlc_string = format!("not_a_number-0000abcd-{}", node_id);
        let result = HLC::from_str(&hlc_string);
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        // Check the context message
        assert!(err_msg.contains("Invalid timestamp format in HLC: 'not_a_number'"));
    }

    #[test]
    fn test_from_str_invalid_counter() {
        let node_id = Uuid::new_v4();
        let hlc_string = format!("1234567890123-not_hex-{}", node_id);
        let result = HLC::from_str(&hlc_string);
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        // Check the context message
        assert!(err_msg.contains("Invalid hex counter format in HLC: 'not_hex'"));
    }

    #[test]
    fn test_from_str_invalid_counter_format_length() {
        // Valid hex, but format might imply fixed length (though not enforced by parse)
        let node_id = Uuid::new_v4();
        let hlc_string = format!("1234567890123-abc-{}", node_id); // Shorter hex than usual
        let hlc = HLC::from_str(&hlc_string).unwrap(); // Should still parse
        assert_eq!(hlc.version, 0xabc);

        // Test hex with > 8 chars (overflows u32)
        let hlc_string_overflow = format!("1234567890123-100000000-{}", node_id);
        let result = HLC::from_str(&hlc_string_overflow);
        assert!(result.is_err()); // Error comes from u32::from_str_radix
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Invalid hex counter format in HLC: '100000000'"));
    }

    #[test]
    fn test_from_str_invalid_node_id() {
        let hlc_string = "1234567890123-0000abcd-not_a_uuid";
        let result = HLC::from_str(hlc_string);
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        // Check the context message
        assert!(err_msg.contains("Invalid node ID format in HLC: 'not_a_uuid'"));
    }
}

#[cfg(test)]
mod hlcrecord_tests {
    use sea_orm::Database;
    use serde_json::json;

    use crate::chunking::tests::{insert_task, setup_db, test_model_def::Entity};

    use super::*;

    #[derive(Clone, Debug)]
    struct MockRecord {
        id: i32,
        data: String,
        created: Option<HLC>,
        updated: Option<HLC>,
    }

    impl HLCRecord for MockRecord {
        fn created_at_hlc(&self) -> Option<HLC> {
            self.created.clone()
        }

        fn updated_at_hlc(&self) -> Option<HLC> {
            self.updated.clone()
        }

        fn unique_id(&self) -> String {
            self.id.to_string()
        }

        fn data_for_hashing(&self) -> serde_json::Value {
            // Only include data relevant for hashing
            json!({
                "id": self.id,
                "data": self.data,
                // Note: We explicitly exclude 'created' and 'updated' HLCs from the hash data
            })
        }

        // We don't override to_summary or full_data, so they use the default
    }

    #[test]
    fn test_hlcrecord_defaults() {
        let node_id = Uuid::new_v4();
        let hlc = create_hlc(100, 1, &node_id.to_string());
        let record = MockRecord {
            id: 1,
            data: "test data".to_string(),
            created: Some(hlc.clone()),
            updated: Some(hlc.clone()),
        };

        let expected_hashing_data = json!({
            "id": 1,
            "data": "test data"
        });

        // Test data_for_hashing returns the correct subset
        assert_eq!(record.data_for_hashing(), expected_hashing_data);

        // Test default to_summary uses data_for_hashing
        assert_eq!(
            record.to_summary(),
            expected_hashing_data,
            "Default to_summary should equal data_for_hashing"
        );

        // Test default full_data uses data_for_hashing
        assert_eq!(
            record.full_data(),
            expected_hashing_data,
            "Default full_data should equal data_for_hashing"
        );
    }

    #[test]
    fn test_calculate_hash() -> Result<()> {
        let node_id = Uuid::new_v4();
        let hlc = create_hlc(100, 1, &node_id.to_string());
        let record1 = MockRecord {
            id: 1,
            data: "test data".to_string(),
            created: Some(hlc.clone()),
            updated: Some(hlc.clone()),
        };
        let record2 = MockRecord {
            // Same content, different HLCs
            id: 1,
            data: "test data".to_string(),
            created: Some(hlc.clone()),
            updated: Some(create_hlc(101, 0, &node_id.to_string())),
        };
        let record3 = MockRecord {
            // Different content
            id: 2,
            data: "other data".to_string(),
            created: Some(hlc.clone()),
            updated: Some(hlc.clone()),
        };

        let hash1 = calculate_hash(&record1)?;
        let hash2 = calculate_hash(&record2)?;
        let hash3 = calculate_hash(&record3)?;

        assert_eq!(
            hash1, hash2,
            "Hashes should be the same for same content despite different HLCs"
        );
        assert_ne!(hash1, hash3, "Hashes should differ for different content");

        // Verify hash seems reasonable (BLAKE3 hex is 64 chars)
        assert_eq!(hash1.len(), 64);
        assert!(hash1.chars().all(|c| c.is_ascii_hexdigit()));

        Ok(())
    }

    #[tokio::test]
    async fn test_get_data_before_hlc() -> Result<()> {
        let db = setup_db().await?;
        let node1 = Uuid::new_v4();

        let hlc1 = create_hlc(1000, 10, &node1.to_string()); // Before pivot
        let hlc_pivot = create_hlc(2000, 5, &node1.to_string()); // Pivot
        let hlc2 = create_hlc(2000, 4, &node1.to_string()); // Before pivot (same ts, lower v)
        let hlc3 = create_hlc(3000, 0, &node1.to_string()); // After pivot

        insert_task(&db, 1, "task1", &hlc1).await?;
        insert_task(&db, 2, "task2", &hlc2).await?;
        insert_task(&db, 3, "task_pivot", &hlc_pivot).await?; // Insert pivot itself
        insert_task(&db, 4, "task3", &hlc3).await?;

        // Get data LTE hlc_pivot (page 0, size 10)
        let results = get_data_before_hlc::<Entity>(&db, &hlc_pivot, 0, 10).await?;

        // Expect results ordered DESC by HLC: pivot, hlc2, hlc1
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].id, 3); // pivot
        assert_eq!(results[1].id, 2); // hlc2
        assert_eq!(results[2].id, 1); // hlc1

        // Test pagination: Get page 1 (should be empty)
        let results_page1 = get_data_before_hlc::<Entity>(&db, &hlc_pivot, 1, 2).await?;
        assert_eq!(results_page1.len(), 1); // Page 1 contains the 3rd item (id 1)
        assert_eq!(results_page1[0].id, 1);

        // Test pagination: Get page 2 (should be empty)
        let results_page2 = get_data_before_hlc::<Entity>(&db, &hlc_pivot, 2, 2).await?;
        assert!(results_page2.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn test_get_data_after_hlc() -> Result<()> {
        let db = setup_db().await?;
        let node1 = Uuid::new_v4();

        let hlc1 = create_hlc(1000, 10, &node1.to_string()); // Before pivot
        let hlc_pivot = create_hlc(2000, 5, &node1.to_string()); // Pivot
        let hlc2 = create_hlc(2000, 6, &node1.to_string()); // After pivot (same ts, higher v)
        let hlc3 = create_hlc(3000, 0, &node1.to_string()); // After pivot

        insert_task(&db, 1, "task1", &hlc1).await?;
        insert_task(&db, 2, "task_pivot", &hlc_pivot).await?;
        insert_task(&db, 3, "task2", &hlc2).await?;
        insert_task(&db, 4, "task3", &hlc3).await?;

        // Get data GT hlc_pivot (page 0, size 10)
        let results = get_data_after_hlc::<Entity>(&db, &hlc_pivot, 0, 10).await?;

        // Expect results ordered ASC by HLC: hlc2, hlc3
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, 3); // hlc2
        assert_eq!(results[1].id, 4); // hlc3

        // Test pagination: Get page 1 (should be empty)
        let results_page1 = get_data_after_hlc::<Entity>(&db, &hlc_pivot, 1, 1).await?;
        assert_eq!(results_page1.len(), 1);
        assert_eq!(results_page1[0].id, 4); // Second item (hlc3)

        // Test pagination: Get page 2 (should be empty)
        let results_page2 = get_data_after_hlc::<Entity>(&db, &hlc_pivot, 2, 1).await?;
        assert!(results_page2.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn test_get_data_in_hlc_range() -> Result<()> {
        let db = setup_db().await?;
        let node1 = Uuid::new_v4();

        let hlc0 = create_hlc(500, 0, &node1.to_string()); // Outside range (before)
        let hlc_start = create_hlc(1000, 10, &node1.to_string()); // Range start (inclusive)
        let hlc_mid1 = create_hlc(1500, 0, &node1.to_string()); // Inside range
        let hlc_end = create_hlc(2000, 5, &node1.to_string()); // Range end (inclusive)
        let hlc_mid2 = create_hlc(2000, 4, &node1.to_string()); // Inside range (same ts as end, lower v)
        let hlc4 = create_hlc(2000, 6, &node1.to_string()); // Outside range (after end, same ts, higher v)
        let hlc5 = create_hlc(3000, 0, &node1.to_string()); // Outside range (after)

        insert_task(&db, 0, "task0", &hlc0).await?;
        insert_task(&db, 1, "task_start", &hlc_start).await?;
        insert_task(&db, 2, "task_mid1", &hlc_mid1).await?;
        insert_task(&db, 3, "task_mid2", &hlc_mid2).await?;
        insert_task(&db, 4, "task_end", &hlc_end).await?;
        insert_task(&db, 5, "task4", &hlc4).await?;
        insert_task(&db, 6, "task5", &hlc5).await?;

        // Get data between hlc_start and hlc_end (inclusive)
        let results = get_data_in_hlc_range::<Entity>(&db, &hlc_start, &hlc_end, 0, 10).await?;

        // Expect results ordered ASC by HLC: start, mid1, mid2, end
        assert_eq!(results.len(), 4);
        assert_eq!(results[0].id, 1); // start
        assert_eq!(results[1].id, 2); // mid1
        assert_eq!(results[2].id, 3); // mid2
        assert_eq!(results[3].id, 4); // end

        // Test pagination: Get page 1 with page size 2
        let results_page1 =
            get_data_in_hlc_range::<Entity>(&db, &hlc_start, &hlc_end, 1, 2).await?;
        assert_eq!(results_page1.len(), 2);
        assert_eq!(results_page1[0].id, 3); // mid2 (3rd item overall)
        assert_eq!(results_page1[1].id, 4); // end (4th item overall)

        // Test pagination: Get page 2 with page size 2 (should be empty)
        let results_page2 =
            get_data_in_hlc_range::<Entity>(&db, &hlc_start, &hlc_end, 2, 2).await?;
        assert!(results_page2.is_empty());

        // Test invalid range (start > end)
        let invalid_range_result =
            get_data_in_hlc_range::<Entity>(&db, &hlc_end, &hlc_start, 0, 10).await;
        assert!(invalid_range_result.is_err());
        let err_string = invalid_range_result.unwrap_err().to_string();
        assert!(err_string.contains("must be less than or equal to End HLC"));

        Ok(())
    }

    #[tokio::test]
    async fn test_get_data_functions_db_error() -> Result<()> {
        // Connect to an in-memory database. This connection should succeed.
        let db_invalid = Database::connect("sqlite::memory:")
            .await // Changed connection string
            .context("Failed to connect to in-memory database for error test")?;

        let node1 = Uuid::new_v4();
        let hlc = create_hlc(1000, 0, &node1.to_string());

        // Now, the query functions should fail because the table doesn't exist.
        let res_before = get_data_before_hlc::<Entity>(&db_invalid, &hlc, 0, 10).await;
        assert!(res_before.is_err(), "get_data_before_hlc should fail");
        // Check the context message added by the function itself
        assert!(
            res_before
                .unwrap_err()
                .to_string()
                .contains("Failed to fetch page for get_data_before_hlc"),
            "Error message for 'before' should contain the expected context"
        );
        // The underlying error might be "no such table", wrapped by the context.

        // Re-connect for the next test case to ensure a clean state if needed,
        // though for in-memory it might not strictly be necessary unless tables were created.
        // Or simply reuse db_invalid as the table is still missing.
        let res_after = get_data_after_hlc::<Entity>(&db_invalid, &hlc, 0, 10).await;
        assert!(res_after.is_err(), "get_data_after_hlc should fail");
        assert!(
            res_after
                .unwrap_err()
                .to_string()
                .contains("Failed to fetch page for get_data_after_hlc"),
            "Error message for 'after' should contain the expected context"
        );

        let res_range = get_data_in_hlc_range::<Entity>(&db_invalid, &hlc, &hlc, 0, 10).await;
        assert!(res_range.is_err(), "get_data_in_hlc_range should fail");
        assert!(
            res_range
                .unwrap_err()
                .to_string()
                .contains("Failed to fetch page for get_data_in_hlc_range"),
            "Error message for 'range' should contain the expected context"
        );

        // No need to remove the file as it's in-memory.
        Ok(())
    }
}
