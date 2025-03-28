//! # Core Synchronization Orchestration Module
//!
//! This module provides the central logic for orchestrating data synchronization between
//! a local SeaORM database and a remote data source. It leverages Hybrid Logical Clocks (HLC)
//! for causal ordering, data chunking for efficient comparison over networks, and defines
//! abstractions for interacting with different data sources and applying changes.
//!
//! ## Key Concepts and Architecture
//!
//! 1.  **Hybrid Logical Clocks (HLC):** Synchronization relies heavily on HLC timestamps (`created_at_hlc`, `updated_at_hlc`)
//!     associated with each record. These timestamps provide a monotonic, causally-ordered sequence
//!     of events across distributed nodes, forming the basis for conflict resolution. (See `hlc.rs`)
//!
//! 2.  **Data Chunking:** To avoid comparing entire tables record by record, data is divided into
//!     chunks based on HLC ranges. An exponential decay algorithm generates smaller chunks for recent
//!     data and larger chunks for older data. Each chunk has metadata including start/end HLCs, record count,
//!     and a cryptographic hash (`chunk_hash`) of its contents. (See `chunking.rs`)
//!
//! 3.  **`RemoteDataSource` Trait:** This trait defines the essential interface for interacting with the
//!     remote peer. Users of this library **must** implement this trait to handle the specifics of
//!     network communication, data fetching (chunks, records), and applying changes transactionally
//!     on the remote side.
//!
//! 4.  **`SyncContext`:** Holds the configuration and state for a synchronization task, including the
//!     local database connection, local node ID, the `RemoteDataSource` implementation, chunking options,
//!     sync direction, and the HLC generator context.
//!
//! 5.  **`SyncDirection`:** Specifies whether to `Pull` changes from remote, `Push` changes to remote,
//!     or perform `Bidirectional` synchronization.
//!
//! 6.  **`SyncTableMetadata`:** Represents the state persisted (by the user application) for each synchronized
//!     table, primarily storing the `last_sync_hlc` – the HLC timestamp up to which the last successful
//!     synchronization occurred for that table.
//!
//! 7.  **Reconciliation Process (`synchronize_table`):**
//!     *   Fetches local and remote chunk metadata generated *after* the `last_sync_hlc`.
//!     *   Aligns chunks based on HLC ranges.
//!     *   Compares `chunk_hash` for perfectly aligned chunks. If hashes match, the chunk is skipped (optimization).
//!     *   If hashes differ or chunks misalign, they are added to a reconciliation queue.
//!     *   The queue is processed:
//!         *   Chunk pairs with differing hashes are either broken down recursively into sub-chunks (if large)
//!             or marked for direct record fetching (if small, based on `COMPARISON_THRESHOLD`).
//!         *   Misaligned ranges are marked for direct record fetching.
//!     *   Fetches required records (local and remote) for ranges marked for fetching.
//!     *   Merges all fetched/identified records needing comparison.
//!     *   Performs conflict resolution record by record based on `updated_at_hlc`:
//!         *   Higher HLC wins.
//!         *   If HLCs are equal, the record from the node with the lexicographically smaller `node_id` wins.
//!     *   Generates `SyncOperation` lists (Insert/Update/Delete/NoOp) for local and remote sides based on
//!         conflict resolution results and the `SyncDirection`.
//!     *   Applies local changes within a single database transaction (`apply_local_changes`).
//!     *   Applies remote changes via `RemoteDataSource::apply_remote_changes` (which must also be transactional).
//!     *   If both apply steps succeed, updates the `SyncTableMetadata` with the new `last_sync_hlc` (the maximum
//!         HLC encountered during the sync).
//!
//! 8.  **Transactional Integrity:** Local changes are applied within a SeaORM transaction. The `RemoteDataSource`
//!     implementation is expected to ensure transactional application of changes on the remote side. Failure in either
//!     step aborts the synchronization for that table, and `last_sync_hlc` is not updated.
//!
//! ## User Adaptation Guide: Implementing Required Traits
//!
//! To use the `synchronize_table` function, you must adapt your SeaORM entities and models
//! by implementing specific traits. This allows the generic synchronization logic to interact
//! correctly with your custom data structures and database schema.
//!
//! Your database schema **must** include columns to store the HLC components (timestamp, counter, node_id)
//! for both creation (`created_at_hlc_*`) and update (`updated_at_hlc_*`) times. Typically:
//! - Timestamp (`_ts`): `BigInt` (storing milliseconds since epoch) or `Timestamp`/`DateTime` (ensure proper conversion). The examples assume `BigInt`.
//! - Counter (`_ct`): `Integer` or `BigInt`. The examples assume `Integer`.
//! - Node ID (`_id`): `Uuid`.
//!
//! Here's how to implement the required traits for an example entity `my_table`:
//!
//! ```rust
//! # // Mock structures to make the example compile standalone
//! # use sea_orm::{entity::prelude::*, ConnectionTrait, DbErr, DeleteResult, ExecResult, InsertResult, UpdateResult, ActiveModelBehavior, ActiveValue, Set, Unchanged, QueryFilter, Condition, IntoActiveModel};
//! # use serde::{Serialize, Deserialize};
//! # use uuid::Uuid;
//! # use crate::hlc::{HLC, HLCRecord, HLCModel};
//! # use crate::core::{PrimaryKeyFromStr}; // Reference the trait from the same file/module
//! # use anyhow::{Result, Context, anyhow};
//! # use std::str::FromStr;
//!
//! #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
//! #[sea_orm(table_name = "my_table")]
//! pub struct Model {
//!     #[sea_orm(primary_key)]
//!     pub id: i32, // Example: Integer primary key
//!     pub data: String,
//!     pub other_field: Option<i64>,
//!     // HLC Fields (assuming BigInt for timestamp, Int for counter)
//!     pub created_at_hlc_ts: i64,
//!     pub created_at_hlc_ct: i32,
//!     pub created_at_hlc_id: Uuid,
//!     pub updated_at_hlc_ts: i64, // Timestamp (e.g., milliseconds)
//!     pub updated_at_hlc_ct: i32, // Counter/Version
//!     pub updated_at_hlc_id: Uuid, // Node ID
//! }
//!
//! #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
//! pub enum Relation {}
//!
//! // --- 1. HLCRecord Implementation (for Model) ---
//! /// Purpose: Provides access to HLC timestamps, a unique ID (string), and
//! ///          data representation for hashing and serialization required by the sync logic.
//! impl HLCRecord for Model {
//!     fn created_at_hlc(&self) -> Option<HLC> {
//!         // Combine the stored components into an HLC struct.
//!         // Handle potential type conversions (e.g., i64 to u64, i32 to u32).
//!         Some(HLC {
//!             timestamp: self.created_at_hlc_ts as u64,
//!             version: self.created_at_hlc_ct as u32,
//!             node_id: self.created_at_hlc_id,
//!         })
//!     }
//!
//!     fn updated_at_hlc(&self) -> Option<HLC> {
//!         // Combine the stored components into an HLC struct.
//!         Some(HLC {
//!             timestamp: self.updated_at_hlc_ts as u64,
//!             version: self.updated_at_hlc_ct as u32,
//!             node_id: self.updated_at_hlc_id,
//!         })
//!     }
//!
//!     fn unique_id(&self) -> String {
//!         // Return the primary key (or a composite key) as a stable string.
//!         self.id.to_string()
//!     }
//!
//!     fn data_for_hashing(&self) -> serde_json::Value {
//!         // Serialize the *relevant* data fields into a JSON value.
//!         // CRITICAL: Exclude the `updated_at_hlc_*` fields themselves,
//!         // as the hash should represent the content *at* that HLC time.
//!         // Including `created_at_hlc_*` might be okay if it's immutable.
//!         // Use `serde_json::json!` macro or `serde_json::to_value`.
//!         // Ensure keys are consistently ordered (serde_json usually does this).
//!         serde_json::json!({
//!             "id": self.id,
//!             "data": self.data,
//!             "other_field": self.other_field,
//!             // Include created_at HLC if it's part of the immutable identity/data
//!             // "created_at_hlc_ts": self.created_at_hlc_ts,
//!             // "created_at_hlc_ct": self.created_at_hlc_ct,
//!             // "created_at_hlc_id": self.created_at_hlc_id,
//!         })
//!     }
//!
//!     // to_summary() and full_data() can be overridden if needed,
//!     // otherwise they default to data_for_hashing().
//! }
//!
//! // --- Boilerplate SeaORM ActiveModelBehavior ---
//! impl ActiveModelBehavior for ActiveModel {}
//!
//! // --- 2. HLCModel Implementation (for Entity) ---
//! /// Purpose: Provides SeaORM Column definitions for HLC-related fields,
//! ///          allowing the sync logic to build database queries dynamically.
//! impl HLCModel for Entity {
//!     fn updated_at_time_column() -> Self::Column {
//!         // Return the Column enum variant for the HLC timestamp.
//!         // Adjust if your DB stores HLC timestamp differently (e.g., RFC3339 string).
//!         Column::UpdatedAtHlcTs
//!     }
//!
//!     fn updated_at_version_column() -> Self::Column {
//!         // Return the Column enum variant for the HLC counter.
//!         Column::UpdatedAtHlcCt
//!     }
//!
//!     fn unique_id_column() -> Self::Column {
//!         // Return the Column enum variant for the primary key.
//!         Column::Id
//!     }
//!
//!     // gt, lt, gte, lte, between methods using these columns are provided by the trait.
//!     // Ensure hlc_timestamp_millis_to_rfc3339 is used correctly if your time column
//!     // is DATETIME/TIMESTAMP requiring RFC3339 format. If it's BIGINT millis,
//!     // the HLCModel trait methods need adjustment. The current code assumes RFC3339 comparison.
//!     // *** NOTE: The provided HLCModel assumes comparison happens via RFC3339 strings.
//!     // If your `updated_at_time_column` stores milliseconds as BIGINT, you need to
//!     // modify the gt, lt, gte, lte, between methods in `hlc.rs` to compare integers directly,
//!     // removing the `hlc_timestamp_millis_to_rfc3339` conversion. ***
//! }
//!
//! // --- 3. PrimaryKeyFromStr Implementation (for Entity::PrimaryKey) ---
//! /// Purpose: Converts the string representation of the primary key (from `HLCRecord::unique_id`)
//! ///          back into the actual SeaORM primary key type (`ValueType`) required for database
//! ///          operations like update and delete.
//! impl PrimaryKeyFromStr<<Self as PrimaryKeyTrait>::ValueType> for PrimaryKey
//! where
//!     // Ensure the actual PK type (e.g., i32) implements FromStr
//!     <Self as PrimaryKeyTrait>::ValueType: FromStr,
//!     // Ensure the error type from parsing implements standard Error traits
//!     <<Self as PrimaryKeyTrait>::ValueType as FromStr>::Err: std::error::Error + Send + Sync + 'static,
//! {
//!     fn read_key(s: &str) -> Result<<Self as PrimaryKeyTrait>::ValueType> {
//!         s.parse::<<Self as PrimaryKeyTrait>::ValueType>()
//!             .map_err(|e| anyhow!(e).context(format!("Failed to parse primary key string '{}'", s)))
//!     }
//! }
//!
//! // --- 4. IntoActiveModel Implementation (for Model) ---
//! /// Purpose: Converts a `Model` instance (e.g., read from DB or received from remote)
//! ///          into an `ActiveModel` suitable for SeaORM insert or update operations.
//! impl IntoActiveModel<ActiveModel> for Model {
//!     fn into_active_model(self) -> ActiveModel {
//!         ActiveModel {
//!             // For updates, the primary key should typically be Unchanged
//!             // or Set if you know it's correct. For inserts, it's often Default or NotSet.
//!             // If using `update_many().set(active_model)`, PK is usually ignored here
//!             // and filtering happens via `.filter()`.
//!             // If using `insert()`, PK should be NotSet/Default if auto-incrementing.
//!             // Let's assume it's for an update where PK is used for filtering later:
//!             id: Unchanged(self.id),
//!             // Wrap fields to be updated/inserted in Set()
//!             data: Set(self.data),
//!             other_field: Set(self.other_field),
//!             // Include HLC fields if they need to be set/updated
//!             created_at_hlc_ts: Set(self.created_at_hlc_ts),
//!             created_at_hlc_ct: Set(self.created_at_hlc_ct),
//!             created_at_hlc_id: Set(self.created_at_hlc_id),
//!             updated_at_hlc_ts: Set(self.updated_at_hlc_ts),
//!             updated_at_hlc_ct: Set(self.updated_at_hlc_ct),
//!             updated_at_hlc_id: Set(self.updated_at_hlc_id),
//!             // Use `..Default::default()` if ActiveModel has more fields than Model,
//!             // though explicit is often better. It's important here because 'id' might
//!             // be Unchanged or Set, not covering all fields.
//!             // ..Default::default() // Or be explicit if all fields are covered
//!         }
//!     }
//! }
//!
//! // Example Usage (Conceptual - Requires RemoteDataSource impl etc.)
//! /*
//! async fn run_sync(db: &DatabaseConnection, remote: &impl RemoteDataSource, local_node_id: Uuid) -> Result<()> {
//!     let hlc_ctx = SyncTaskContext::new(local_node_id);
//!     let options = ChunkingOptions::default(local_node_id);
//!     let sync_ctx = SyncContext {
//!         db,
//!         local_node_id,
//!         remote_source: remote,
//!         chunking_options: options,
//!         sync_direction: SyncDirection::Bidirectional,
//!         hlc_context: &hlc_ctx,
//!     };
//!
//!     // Assume metadata is loaded from persistence
//!     let initial_metadata = SyncTableMetadata {
//!         table_name: "my_table".to_string(),
//!         last_sync_hlc: HLC::new(local_node_id), // Start from beginning initially
//!     };
//!
//!     let final_metadata = synchronize_table::<Entity, _>(&sync_ctx, "my_table", &initial_metadata).await?;
//!
//!     // Persist final_metadata.last_sync_hlc for the next run
//!     println!("Sync completed. New last_sync_hlc: {}", final_metadata.last_sync_hlc);
//!
//!     Ok(())
//! }
//! */
//! ```
//!
//! By implementing these traits for each entity you wish to synchronize, you provide the necessary
//! bridge between your specific data models and the generic synchronization engine. Remember to
//! also implement the `RemoteDataSource` trait to handle the communication with your specific
//! remote peer.

use crate::chunking::{
    break_data_chunk,
    generate_data_chunks,
    ChunkingOptions,
    DataChunk,
    // SubDataChunk is used internally in break_data_chunk but not directly exposed here
};
use crate::hlc::{HLCModel, HLCQuery, HLCRecord, SyncTaskContext, HLC}; // Assuming hlc.rs is in the same crate
use anyhow::{anyhow, Context, Result};
use log::{debug, error, info, warn};
use sea_orm::entity::prelude::*;
use sea_orm::{
    ActiveModelBehavior, DatabaseConnection, EntityTrait, IntoActiveModel, PrimaryKeyTrait,
    QueryFilter, TransactionTrait, Value,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::hash::Hash;
use uuid::Uuid;

// --- Constants ---
/// If a chunk pair has differing hashes, but the maximum record count
/// in either chunk is below or equal to this threshold, fetch individual records directly
/// instead of breaking the chunk down further.
const COMPARISON_THRESHOLD: u64 = 50;

// --- Enums and Structs ---

/// Defines the direction of synchronization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncDirection {
    /// Pull changes from remote to local. Local conflicts resolved by remote winning, only applies remote->local changes.
    Pull,
    /// Push changes from local to remote. Local conflicts resolved by local winning, only applies local->remote changes.
    Push,
    /// Perform bidirectional synchronization. Conflicts resolved by HLC, changes applied in both directions.
    Bidirectional,
}

/// Metadata stored per table to track synchronization progress.
/// This should be persisted by the user application for each table and remote peer pairing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncTableMetadata {
    /// The name of the table this metadata corresponds to.
    pub table_name: String,
    /// HLC timestamp indicating the point up to which synchronization
    /// with a specific remote peer was last successfully completed for this table.
    /// This should represent the maximum HLC timestamp (`updated_at_hlc`) encountered
    /// *from either side* among the records processed during the last successful sync run.
    pub last_sync_hlc: HLC,
    // node_id is implicitly the local node's ID, managed by the application.
    // The remote node_id is obtained via RemoteDataSource.
}

/// Represents an action determined during conflict resolution, to be applied
/// either locally or remotely.
#[derive(Debug, Clone)]
pub enum SyncOperation<Model: HLCRecord> {
    /// Insert a new record locally.
    InsertLocal(Model),
    /// Update an existing record locally with the provided model data.
    UpdateLocal(Model),
    /// Delete a record locally identified by its unique ID.
    /// Requires underlying support for deletes (e.g., soft deletes or actual deletion).
    DeleteLocal(String), // String is the unique_id
    /// Insert a new record remotely.
    InsertRemote(Model),
    /// Update an existing record remotely with the provided model data.
    UpdateRemote(Model),
    /// Delete a record remotely identified by its unique ID.
    DeleteRemote(String), // String is the unique_id
    /// No operation needed for this record (e.g., records are identical or sync direction prevents action).
    NoOp(String), // String is the unique_id
}

/// Abstraction for interacting with the remote data source.
/// **Users must implement this trait.**
#[async_trait::async_trait]
pub trait RemoteDataSource: Send + Sync + Debug {
    /// Gets the unique Node ID (UUID v4) of the remote data source.
    async fn get_remote_node_id(&self) -> Result<Uuid>;

    /// Fetches chunk metadata (`DataChunk`) for a specific table from the remote node.
    ///
    /// # Arguments
    /// * `table_name`: The name of the table.
    /// * `after_hlc`: If `Some`, only return chunks where `start_hlc` is strictly greater than this HLC.
    ///                If `None`, return all chunks (less common for sync).
    async fn get_remote_chunks<E>(
        &self,
        table_name: &str,
        after_hlc: Option<&HLC>,
    ) -> Result<Vec<DataChunk>>
    where
        E: HLCModel + EntityTrait + Send + Sync, // Entity constraints
        E::Model: HLCRecord + Send + Sync + Deserialize<'static> + Serialize; // Model constraints (add Deserialize/Serialize)

    /// Requests the remote node to break down a specified parent chunk into smaller sub-chunks.
    /// The remote implementation **should** perform verification (count and hash check)
    /// against its own data for the `parent_chunk` range before generating sub-chunks.
    /// Returns the metadata (`DataChunk`) of the generated sub-chunks.
    ///
    /// # Arguments
    /// * `table_name`: The name of the table.
    /// * `parent_chunk`: The metadata of the chunk to be broken down.
    /// * `sub_chunk_size`: The target number of records per sub-chunk.
    async fn get_remote_sub_chunks<E>(
        &self,
        table_name: &str,
        parent_chunk: &DataChunk,
        sub_chunk_size: u64,
    ) -> Result<Vec<DataChunk>>
    where
        E: HLCModel + EntityTrait + Send + Sync, // Entity constraints
        E::Model: HLCRecord + Send + Sync + Deserialize<'static> + Serialize; // Model constraints

    /// Fetches full records (`Model`) within a specific HLC range (inclusive)
    /// for a table from the remote node. Used when chunk hashes differ or for small ranges.
    ///
    /// # Arguments
    /// * `table_name`: The name of the table.
    /// * `start_hlc`: The inclusive start HLC of the range.
    /// * `end_hlc`: The inclusive end HLC of the range.
    async fn get_remote_records_in_hlc_range<E>(
        &self,
        table_name: &str,
        start_hlc: &HLC,
        end_hlc: &HLC,
    ) -> Result<Vec<E::Model>>
    where
        E: HLCModel + EntityTrait + Send + Sync, // Entity constraints
        E::Model: HLCRecord + Send + Sync + Deserialize<'static> + Serialize; // Model constraints

    /// Applies a batch of `SyncOperation`s (Inserts, Updates, Deletes) to the remote data source
    /// for a specific table.
    /// This operation **must be transactional** on the remote side. If any operation fails,
    /// the entire batch should be rolled back.
    /// It should return the HLC timestamp representing the state *after* successfully applying
    /// the changes, typically the maximum HLC among the applied changes or the HLC generated
    /// by the remote server during the transaction.
    ///
    /// # Arguments
    /// * `table_name`: The name of the table.
    /// * `operations`: A vector of `SyncOperation`s containing the records/IDs to modify.
    ///                 Note: Only `InsertRemote`, `UpdateRemote`, `DeleteRemote` are relevant here.
    async fn apply_remote_changes<E>(
        &self,
        table_name: &str,
        operations: Vec<SyncOperation<E::Model>>,
    ) -> Result<HLC>
    // Returns the HLC timestamp achieved remotely
    where
        E: HLCModel + EntityTrait + Send + Sync, // Entity constraints
        E::Model: HLCRecord + Send + Sync + Deserialize<'static> + Serialize; // Model constraints

    /// Optional: Fetches the remote's perspective of the last sync HLC with the local node.
    /// This might be useful for consistency checks or specific synchronization protocols,
    /// but is not strictly required by the current core logic.
    async fn get_remote_last_sync_hlc(
        &self,
        table_name: &str,
        local_node_id: Uuid,
    ) -> Result<Option<HLC>>;
}

/// Context containing configuration and state for a synchronization task instance.
#[derive(Clone)] // Requires DbConn to be cloneable (e.g., Arc<DbConn>) or use lifetimes.
pub struct SyncContext<'a, R: RemoteDataSource> {
    /// Local SeaORM database connection.
    pub db: &'a DatabaseConnection,
    /// Local Node ID (UUID v4).
    pub local_node_id: Uuid,
    /// User-provided implementation for interacting with the remote data source.
    pub remote_source: &'a R,
    /// Options for data chunking (min/max size, alpha).
    pub chunking_options: ChunkingOptions,
    /// Direction of synchronization (`Pull`, `Push`, or `Bidirectional`).
    pub sync_direction: SyncDirection,
    /// HLC generator context for generating new HLCs locally if needed (e.g., for local conflict winners).
    pub hlc_context: &'a SyncTaskContext,
}

/// Internal enum representing the state of a record during the comparison phase,
/// keyed by the record's `unique_id`.
#[derive(Debug)]
enum RecordSyncState<M: HLCRecord> {
    /// Record exists only locally within the compared range.
    LocalOnly(M),
    /// Record exists only remotely within the compared range.
    RemoteOnly(M),
    /// Record exists on both sides within the compared range. Holds both versions.
    Both(M, M), // (Local Model, Remote Model)
                // Future: Add Tombstone variants if implementing delete tracking
                // LocalTombstoneRemoteUpdate(LocalTombstone, M),
                // LocalUpdateRemoteTombstone(M, RemoteTombstone),
}

/// Internal struct representing an HLC range that requires fetching individual records
/// for detailed comparison because chunk hashes differed or alignment failed.
#[derive(Debug, Clone)]
struct ComparisonRange {
    start_hlc: HLC,
    end_hlc: HLC,
}

/// Internal enum representing an item in the reconciliation queue during chunk comparison.
#[derive(Debug)]
enum ReconciliationItem {
    /// A pair of local and remote chunks covering the exact same HLC range but with differing hashes.
    /// Needs further processing (breakdown or fetch).
    ChunkPair(DataChunk, DataChunk), // (Local Chunk, Remote Chunk)
    /// An HLC range for which individual records need to be fetched from both local and remote sources.
    FetchRange(ComparisonRange),
}

// --- Main Synchronization Logic ---

/// Performs synchronization for a single table between the local node and the remote source.
///
/// This is the main entry point for synchronizing a specific table based on its last sync state.
/// It orchestrates chunk comparison, record fetching, conflict resolution, and transactional application
/// of changes according to the specified `SyncDirection`.
///
/// # Type Parameters
/// * `E`: The SeaORM `EntityTrait` for the table being synchronized.
/// * `R`: The type implementing the `RemoteDataSource` trait.
///
/// # Constraints (where clause)
/// Ensures that the Entity, Model, ActiveModel, and PrimaryKey types satisfy all traits required
/// by the synchronization logic (HLC access, database operations, serialization, hashing, etc.).
///
/// # Arguments
/// * `context`: The `SyncContext` containing configuration, connections, and the remote source implementation.
/// * `table_name`: The string name of the table to synchronize (used for logging and remote calls).
/// * `metadata`: The current `SyncTableMetadata` for this table, containing the `last_sync_hlc`.
///
/// # Returns
/// A `Result` containing the updated `SyncTableMetadata` (with the new `last_sync_hlc`) upon successful completion,
/// or an `anyhow::Error` if synchronization fails at any step.
pub async fn synchronize_table<E, R>(
    context: &SyncContext<'_, R>,
    table_name: &str,
    metadata: &SyncTableMetadata,
) -> Result<SyncTableMetadata>
where
    // Entity must support HLC queries and standard Entity traits
    E: HLCModel + EntityTrait + Send + Sync,
    E::Column: Send + Sync, // Columns needed for ordering/filtering
    // Model must support HLC record access, basic traits, serialization, and conversion to ActiveModel
    E::Model: HLCRecord
        + Send
        + Sync
        + Debug
        + Clone
        + Serialize // Needed for apply_remote_changes potentially
        + for<'de> Deserialize<'de> // Needed for receiving remote records potentially
        + IntoActiveModel<E::ActiveModel>,
    // ActiveModel must support standard SeaORM behavior
    E::ActiveModel: ActiveModelBehavior + Send + Sync + Debug,
    // PrimaryKey must support trait operations and conversion from string ID
    E::PrimaryKey:
        PrimaryKeyTrait + PrimaryKeyFromStr<<E::PrimaryKey as PrimaryKeyTrait>::ValueType>,
    // The actual type of the PrimaryKey's value must support basic traits + conversion into SeaORM's Value type
    <E::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Eq + Hash + Clone + Send + Sync + Debug + Ord + Into<Value>,
    // RemoteDataSource implementation
    R: RemoteDataSource + Send + Sync + Debug,
{
    info!(
        "Starting sync for table '{}' ({:?}) from HLC: {}",
        table_name, context.sync_direction, metadata.last_sync_hlc
    );

    // Ensure remote node ID is obtained early for conflict resolution tie-breaking
    let remote_node_id = context
        .remote_source
        .get_remote_node_id()
        .await
        .context("Failed to get remote node ID")?;

    // --- 1. Fetch Initial Chunks ---
    // Fetch local and remote chunk metadata for data modified *after* the last sync HLC.
    let sync_start_hlc = metadata.last_sync_hlc.clone();
    let local_chunks_fut = generate_data_chunks::<E>(
        context.db,
        &context.chunking_options,
        Some(sync_start_hlc.clone()), // Pass Option<&HLC>
    );
    let remote_chunks_fut = context
        .remote_source
        .get_remote_chunks::<E>(table_name, Some(&sync_start_hlc)); // Pass Option<&HLC>

    // Execute futures concurrently
    let (local_chunks_res, remote_chunks_res) = tokio::join!(local_chunks_fut, remote_chunks_fut);

    let mut local_chunks = local_chunks_res
        .with_context(|| format!("Failed to generate local chunks for table '{}'", table_name))?;
    let mut remote_chunks = remote_chunks_res
        .with_context(|| format!("Failed to fetch remote chunks for table '{}'", table_name))?;

    // Sort chunks by start HLC for efficient alignment
    local_chunks.sort_by(|a, b| a.start_hlc.cmp(&b.start_hlc));
    remote_chunks.sort_by(|a, b| a.start_hlc.cmp(&b.start_hlc));

    debug!(
        "Table '{}': Found {} local chunks, {} remote chunks after HLC {}",
        table_name,
        local_chunks.len(),
        remote_chunks.len(),
        sync_start_hlc
    );

    // --- 2. Reconcile Chunk Differences Recursively/Iteratively ---
    // Initialize lists to store records that need detailed comparison
    let mut local_records_to_compare: Vec<E::Model> = Vec::new();
    let mut remote_records_to_compare: Vec<E::Model> = Vec::new();
    // Track the maximum HLC encountered across all processed chunks/records
    let mut max_hlc_encountered = sync_start_hlc.clone();

    // Use a queue for iterative processing of ranges/chunks needing reconciliation
    let mut reconciliation_queue: VecDeque<ReconciliationItem> = VecDeque::new();

    // Align the top-level chunks and populate the initial reconciliation queue
    align_and_queue_chunks(
        local_chunks,
        remote_chunks,
        &mut reconciliation_queue,
        &mut max_hlc_encountered,
    );

    // Process the reconciliation queue until empty
    while let Some(item) = reconciliation_queue.pop_front() {
        match item {
            ReconciliationItem::FetchRange(range) => {
                // This range requires fetching individual records
                debug!(
                    "Processing FetchRange: [{}-{}]",
                    range.start_hlc, range.end_hlc
                );
                // Ensure the end HLC of the range is considered for the overall max HLC
                update_max_hlc(&mut max_hlc_encountered, &range.end_hlc);

                // Fetch local and remote records concurrently for this range
                let local_fut =
                    fetch_local_records_in_range::<E>(context.db, &range.start_hlc, &range.end_hlc);
                let remote_fut = context.remote_source.get_remote_records_in_hlc_range::<E>(
                    table_name,
                    &range.start_hlc,
                    &range.end_hlc,
                );
                let (local_res, remote_res) = tokio::join!(local_fut, remote_fut);

                // Extend the comparison lists, propagating errors
                local_records_to_compare.extend(local_res.with_context(|| {
                    format!(
                        "Failed to fetch local records for range [{}-{}]",
                        range.start_hlc, range.end_hlc
                    )
                })?);
                remote_records_to_compare.extend(remote_res.with_context(|| {
                    format!(
                        "Failed to fetch remote records for range [{}-{}]",
                        range.start_hlc, range.end_hlc
                    )
                })?);
            }
            ReconciliationItem::ChunkPair(local_chunk, remote_chunk) => {
                // This pair represents aligned chunks with differing hashes
                debug!(
                    "Processing ChunkPair: [{}-{}] (Hash L: {:.8}, Hash R: {:.8})",
                    local_chunk.start_hlc,
                    local_chunk.end_hlc,
                    local_chunk.chunk_hash,
                    remote_chunk.chunk_hash
                );
                // Update max HLC based on the chunk boundaries
                update_max_hlc(&mut max_hlc_encountered, &local_chunk.end_hlc);
                // remote_chunk.end_hlc is the same as local_chunk.end_hlc here

                // Defensive check: If hashes somehow match now, skip.
                if local_chunk.chunk_hash == remote_chunk.chunk_hash {
                    warn!(
                        "Chunk hashes matched unexpectedly in queue processing for range [{}-{}]. Skipping.",
                        local_chunk.start_hlc, local_chunk.end_hlc
                    );
                    continue;
                }

                // Decide whether to break down further or fetch records based on count threshold
                // Use the *larger* count of the two chunks for the decision
                let max_count = std::cmp::max(local_chunk.count, remote_chunk.count);

                if max_count == 0 {
                    // Both chunks reported 0 count but hashes differed. This is weird.
                    // Hash of empty should be consistent. Fetch range to be safe.
                    warn!("ChunkPair has 0 count but differing hashes for range [{}-{}] L:'{}' R:'{}'. Fetching range.",
                          local_chunk.start_hlc, local_chunk.end_hlc,
                          local_chunk.chunk_hash, remote_chunk.chunk_hash);
                    reconciliation_queue.push_back(ReconciliationItem::FetchRange(
                        ComparisonRange {
                            start_hlc: local_chunk.start_hlc.clone(),
                            end_hlc: local_chunk.end_hlc.clone(),
                        },
                    ));
                } else if max_count <= COMPARISON_THRESHOLD {
                    // Count is small enough, fetch individual records for this range
                    debug!(
                        "Chunk count ({}) <= threshold ({}). Adding FetchRange [{}-{}].",
                        max_count, COMPARISON_THRESHOLD, local_chunk.start_hlc, local_chunk.end_hlc
                    );
                    reconciliation_queue.push_back(ReconciliationItem::FetchRange(
                        ComparisonRange {
                            start_hlc: local_chunk.start_hlc.clone(),
                            end_hlc: local_chunk.end_hlc.clone(),
                        },
                    ));
                } else {
                    // Count is too large, break down the chunk recursively
                    debug!(
                        "Chunk count ({}) > threshold ({}). Breaking down chunk [{}-{}].",
                        max_count, COMPARISON_THRESHOLD, local_chunk.start_hlc, local_chunk.end_hlc
                    );
                    // Define the target size for sub-chunks (can be tuned)
                    let sub_chunk_size = COMPARISON_THRESHOLD; // Use threshold as target size

                    // Break down local chunk (includes verification)
                    let local_subs_fut =
                        break_data_chunk::<E>(context.db, &local_chunk, sub_chunk_size);

                    // Request remote sub-chunks (remote performs its own verification)
                    // Pass the *local* chunk definition as the basis for remote breakdown request
                    let remote_subs_fut = context.remote_source.get_remote_sub_chunks::<E>(
                        table_name,
                        &local_chunk,
                        sub_chunk_size,
                    );

                    // Execute concurrently
                    let (local_subs_res, remote_subs_res) =
                        tokio::join!(local_subs_fut, remote_subs_fut);

                    // Process the results of the breakdown
                    match (local_subs_res, remote_subs_res) {
                        (Ok(local_sub_data), Ok(mut remote_subs)) => {
                            // Successfully got sub-chunks from both sides
                            // Extract DataChunk from the local SubDataChunk result
                            let mut local_subs: Vec<DataChunk> =
                                local_sub_data.into_iter().map(|s| s.chunk).collect();

                            // Sort sub-chunks from both sides for alignment
                            local_subs.sort_by(|a, b| a.start_hlc.cmp(&b.start_hlc));
                            remote_subs.sort_by(|a, b| a.start_hlc.cmp(&b.start_hlc));

                            debug!(
                                "Successfully broke down chunk [{}-{}] into {} local and {} remote sub-chunks. Aligning sub-chunks.",
                                local_chunk.start_hlc, local_chunk.end_hlc,
                                local_subs.len(), remote_subs.len()
                            );

                            // Align and queue the generated sub-chunks for further processing
                            align_and_queue_chunks(
                                local_subs,
                                remote_subs,
                                &mut reconciliation_queue,
                                &mut max_hlc_encountered, // Pass down max_hlc_encountered
                            );
                        }
                        (Err(e), _) => {
                            // Failed to break down the local chunk (verification likely failed)
                            error!("Failed to break down local chunk [{}-{}]: {:?}. Falling back to FetchRange.",
                                   local_chunk.start_hlc, local_chunk.end_hlc, e);
                            // Fallback: Fetch all records for the original parent chunk range
                            reconciliation_queue.push_back(ReconciliationItem::FetchRange(
                                ComparisonRange {
                                    start_hlc: local_chunk.start_hlc.clone(),
                                    end_hlc: local_chunk.end_hlc.clone(),
                                },
                            ));
                        }
                        (_, Err(e)) => {
                            // Failed to get sub-chunks from the remote side
                            error!("Failed to get remote sub-chunks for parent range [{}-{}]: {:?}. Falling back to FetchRange.",
                                   local_chunk.start_hlc, local_chunk.end_hlc, e);
                            // Fallback: Fetch all records for the original parent chunk range
                            reconciliation_queue.push_back(ReconciliationItem::FetchRange(
                                ComparisonRange {
                                    start_hlc: local_chunk.start_hlc.clone(), // Use local range as reference
                                    end_hlc: local_chunk.end_hlc.clone(),
                                },
                            ));
                        }
                    }
                }
            }
        }
    } // End of reconciliation queue processing loop

    debug!(
        "Finished chunk reconciliation for table '{}'. Comparing {} local and {} remote records.",
        table_name,
        local_records_to_compare.len(),
        remote_records_to_compare.len()
    );

    // --- 3. Merge and Compare Individual Records ---
    // Use a HashMap keyed by `unique_id` to efficiently merge local and remote records
    // and track their state (LocalOnly, RemoteOnly, Both).
    let mut comparison_map: HashMap<String, RecordSyncState<E::Model>> = HashMap::new();

    // Process local records first
    for local_record in local_records_to_compare {
        let key = local_record.unique_id();
        // Ensure max HLC tracks HLCs from individual records too
        if let Some(hlc) = local_record.updated_at_hlc() {
            update_max_hlc(&mut max_hlc_encountered, &hlc);
        } else {
            warn!(
                "Local record {} missing updated_at_hlc during comparison phase.",
                key
            );
            // Skip this record or handle error? Skipping for now.
            continue;
        }
        comparison_map.insert(key, RecordSyncState::LocalOnly(local_record));
    }

    // Process remote records, merging with local ones
    for remote_record in remote_records_to_compare {
        let key = remote_record.unique_id();
        // Ensure max HLC tracks HLCs from individual records too
        if let Some(hlc) = remote_record.updated_at_hlc() {
            update_max_hlc(&mut max_hlc_encountered, &hlc);
        } else {
            warn!(
                "Remote record {} missing updated_at_hlc during comparison phase.",
                key
            );
            // Skip this record or handle error? Skipping for now.
            continue;
        }
        // Check if a local version exists in the map
        match comparison_map.remove(&key) {
            Some(RecordSyncState::LocalOnly(local_record)) => {
                // Record exists on both sides, move to Both state
                comparison_map.insert(key, RecordSyncState::Both(local_record, remote_record));
            }
            None => {
                // Record only exists remotely (within the compared ranges)
                comparison_map.insert(key, RecordSyncState::RemoteOnly(remote_record));
            }
            _ => unreachable!("Invalid state reached during record merging"),
        }
    }

    // --- 4. Conflict Resolution and Operation Generation ---
    // Iterate through the merged record states and determine the appropriate SyncOperation
    // based on the state, HLC comparison, Node ID tie-breaking, and SyncDirection.
    let mut local_ops: Vec<SyncOperation<E::Model>> = Vec::new();
    let mut remote_ops: Vec<SyncOperation<E::Model>> = Vec::new();

    for (_key, state) in comparison_map {
        match state {
            RecordSyncState::LocalOnly(local_record) => {
                // Record only exists locally
                let id = local_record.unique_id();
                debug!("Conflict Resolution: Record {} is LocalOnly.", id);
                if context.sync_direction == SyncDirection::Push
                    || context.sync_direction == SyncDirection::Bidirectional
                {
                    // Push local record to remote
                    remote_ops.push(SyncOperation::InsertRemote(local_record));
                } else {
                    // Pull only, do nothing locally or remotely for this record
                    local_ops.push(SyncOperation::NoOp(id));
                }
            }
            RecordSyncState::RemoteOnly(remote_record) => {
                // Record only exists remotely
                let id = remote_record.unique_id();
                debug!("Conflict Resolution: Record {} is RemoteOnly.", id);
                if context.sync_direction == SyncDirection::Pull
                    || context.sync_direction == SyncDirection::Bidirectional
                {
                    // Pull remote record to local
                    local_ops.push(SyncOperation::InsertLocal(remote_record));
                } else {
                    // Push only, do nothing locally or remotely for this record
                    remote_ops.push(SyncOperation::NoOp(id));
                }
            }
            RecordSyncState::Both(local_record, remote_record) => {
                // Record exists on both sides, perform conflict resolution
                let id = local_record.unique_id(); // ID is the same
                let local_hlc = local_record.updated_at_hlc().ok_or_else(|| {
                    anyhow!(
                        "Local record {} missing updated_at_hlc during conflict resolution",
                        id
                    )
                })?;
                let remote_hlc = remote_record.updated_at_hlc().ok_or_else(|| {
                    anyhow!(
                        "Remote record {} missing updated_at_hlc during conflict resolution",
                        id
                    )
                })?;

                debug!(
                    "Conflict Resolution: Record {} is Both (Local HLC: {}, Remote HLC: {})",
                    id, local_hlc, remote_hlc
                );

                // Compare HLCs first
                let comparison = local_hlc.cmp(&remote_hlc);
                let (local_wins, remote_wins) = match comparison {
                    std::cmp::Ordering::Greater => {
                        debug!(" -> Local HLC is greater.");
                        (true, false) // Local wins
                    }
                    std::cmp::Ordering::Less => {
                        debug!(" -> Remote HLC is greater.");
                        (false, true) // Remote wins
                    }
                    std::cmp::Ordering::Equal => {
                        // HLCs are identical, use Node ID as tie-breaker (lexicographically smaller wins)
                        debug!(
                            " -> HLCs are equal. Tie-breaking using Node IDs (Local: {}, Remote: {}).",
                            context.local_node_id, remote_node_id
                        );
                        match context.local_node_id.cmp(&remote_node_id) {
                            std::cmp::Ordering::Less => {
                                debug!(" -> Local Node ID wins tie-breaker.");
                                (true, false)
                            }
                            std::cmp::Ordering::Equal => {
                                // Node IDs are identical? Should not happen with UUIDs.
                                // Treat as no-op or log error. Let's treat as no-op.
                                warn!(
                                "Identical HLCs and Node IDs ({}) found for record {}. Treating as NoOp.",
                                context.local_node_id, id
                            );
                                (false, false) // No winner, effectively NoOp
                            }
                            std::cmp::Ordering::Greater => {
                                debug!(" -> Remote Node ID wins tie-breaker.");
                                (false, true)
                            }
                        }
                    }
                };

                // Determine operations based on winner and sync direction
                if local_wins {
                    // Local version is the winner
                    if context.sync_direction == SyncDirection::Push
                        || context.sync_direction == SyncDirection::Bidirectional
                    {
                        // Check if remote actually needs the update (it should if local won)
                        if local_hlc > remote_hlc
                            || (local_hlc == remote_hlc && context.local_node_id < remote_node_id)
                        {
                            debug!(" -> Action: UpdateRemote with local winner.");
                            remote_ops.push(SyncOperation::UpdateRemote(local_record.clone()));
                        } else {
                            // Remote already has the winning state? Log warning.
                            warn!(
                                "Local won conflict for record {} but remote HLC ({}) was not older or tied differently. Remote NoOp.",
                                id, remote_hlc
                            );
                            remote_ops.push(SyncOperation::NoOp(id.clone()));
                        }
                    } else {
                        // Pull only, no remote action needed if local wins
                        remote_ops.push(SyncOperation::NoOp(id.clone()));
                    }
                    // Local side needs no operation as it already has the winning version
                    local_ops.push(SyncOperation::NoOp(id));
                } else if remote_wins {
                    // Remote version is the winner
                    if context.sync_direction == SyncDirection::Pull
                        || context.sync_direction == SyncDirection::Bidirectional
                    {
                        // Check if local actually needs the update (it should if remote won)
                        if remote_hlc > local_hlc
                            || (remote_hlc == local_hlc && remote_node_id < context.local_node_id)
                        {
                            debug!(" -> Action: UpdateLocal with remote winner.");
                            local_ops.push(SyncOperation::UpdateLocal(remote_record.clone()));
                        } else {
                            // Local already has the winning state? Log warning.
                            warn!(
                                "Remote won conflict for record {} but local HLC ({}) was not older or tied differently. Local NoOp.",
                                id, local_hlc
                            );
                            local_ops.push(SyncOperation::NoOp(id.clone()));
                        }
                    } else {
                        // Push only, no local action needed if remote wins
                        local_ops.push(SyncOperation::NoOp(id.clone()));
                    }
                    // Remote side needs no operation as it already has the winning version
                    remote_ops.push(SyncOperation::NoOp(id));
                } else {
                    // No winner (e.g., identical HLC and Node ID, or identical records implicitly)
                    debug!(" -> No clear winner or records identical. Action: NoOp for both.");
                    local_ops.push(SyncOperation::NoOp(id.clone()));
                    remote_ops.push(SyncOperation::NoOp(id));
                }
            } // End RecordSyncState::Both
        } // End match state
    } // End loop through comparison_map

    // --- 5. Apply Changes Transactionally ---
    // Determine the final HLC for this sync run (the highest HLC encountered)
    let final_sync_hlc = max_hlc_encountered.clone(); // Use the tracked maximum HLC

    debug!(
        "Applying changes for table '{}'. {} local ops, {} remote ops. Target HLC: {}",
        table_name,
        local_ops.len(),
        remote_ops.len(),
        final_sync_hlc
    );

    // Apply local changes first within a transaction
    let local_apply_result = apply_local_changes::<E>(context, local_ops).await;

    // Apply remote changes only if local changes succeeded and if needed by direction/ops
    let remote_apply_result = match local_apply_result {
        Ok(_) => {
            // Local changes applied successfully
            let remote_ops_to_apply: Vec<_> = remote_ops
                .into_iter()
                .filter(|op| !matches!(op, SyncOperation::NoOp(_))) // Filter out NoOps
                .collect();

            if !remote_ops_to_apply.is_empty()
                && (context.sync_direction == SyncDirection::Push
                    || context.sync_direction == SyncDirection::Bidirectional)
            {
                info!(
                    "Successfully applied local changes for table '{}'. Applying {} remote changes.",
                    table_name,
                    remote_ops_to_apply.len()
                );
                // Call the remote source to apply changes transactionally
                context
                    .remote_source
                    .apply_remote_changes::<E>(table_name, remote_ops_to_apply)
                    .await
                    .context("Failed to apply remote changes")
            } else {
                debug!(
                    "No remote changes to apply or sync direction prevents it for table '{}'.",
                    table_name
                );
                // If no remote ops needed, return the calculated final HLC as "achieved"
                Ok(final_sync_hlc.clone())
            }
        }
        Err(e) => {
            // Local changes failed, abort sync for this table
            error!(
                "Failed to apply local changes for table '{}': {:?}. Aborting sync for this table.",
                table_name, e
            );
            // Propagate the error
            Err(e).context(format!(
                "Local changes application failed for table '{}'",
                table_name
            ))
        }
    };

    // --- 6. Finalize and Update Metadata ---
    // Check the result of the remote changes application (or the placeholder Ok if skipped)
    match remote_apply_result {
        Ok(achieved_remote_hlc) => {
            // Both local and remote (if applicable) changes succeeded.
            // The new last_sync_hlc should be the maximum HLC encountered during the process.
            // We also get an `achieved_remote_hlc` which *should* ideally not exceed `final_sync_hlc`,
            // but we use `final_sync_hlc` calculated during the run as the definitive upper bound processed.
            // Consider logging if achieved_remote_hlc > final_sync_hlc, as it might indicate clock issues.
            if achieved_remote_hlc > final_sync_hlc {
                warn!("Achieved remote HLC {} is greater than calculated max encountered HLC {}. This might indicate clock skew or remote operations generating unexpected HLCs.", achieved_remote_hlc, final_sync_hlc);
                // Decide whether to use achieved_remote_hlc or final_sync_hlc. Using final_sync_hlc is safer
                // as it represents the state processed.
            }

            let new_last_sync_hlc = final_sync_hlc; // Use the max HLC encountered during the run
            info!(
                "Sync successful for table '{}'. Updating last_sync_hlc to: {}",
                table_name, new_last_sync_hlc
            );
            // Return the new metadata to be persisted by the caller
            Ok(SyncTableMetadata {
                table_name: table_name.to_string(),
                last_sync_hlc: new_last_sync_hlc,
            })
        }
        Err(e) => {
            // Remote changes failed (or local failed earlier and error propagated)
            error!(
                "Sync failed for table '{}' during remote changes application: {:?}. Metadata not updated.",
                table_name, e
            );
            // Propagate the error, indicating sync failure for this table
            Err(e).context(format!(
                "Sync failed for table '{}' during changes application",
                table_name
            ))
        }
    }
}

// --- Helper Functions ---

/// Applies a list of local `SyncOperation`s within a single database transaction.
async fn apply_local_changes<E>(
    context: &SyncContext<'_, impl RemoteDataSource>, // Use impl Trait for R
    operations: Vec<SyncOperation<E::Model>>,
) -> Result<()>
where
    // Constraints copied from synchronize_table for consistency
    E: HLCModel + EntityTrait + Send + Sync,
    E::Model: HLCRecord + Send + Sync + Debug + Clone + Serialize + IntoActiveModel<E::ActiveModel>,
    E::ActiveModel: ActiveModelBehavior + Send + Sync + Debug,
    E::PrimaryKey:
        PrimaryKeyTrait + PrimaryKeyFromStr<<E::PrimaryKey as PrimaryKeyTrait>::ValueType>,
    <E::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Eq + Hash + Clone + Send + Sync + Debug + Ord + Into<Value>,
{
    // Filter out NoOp operations early
    let ops_to_apply: Vec<_> = operations
        .into_iter()
        .filter(|op| !matches!(op, SyncOperation::NoOp(_)))
        .collect();

    if ops_to_apply.is_empty() {
        debug!("No local operations to apply.");
        return Ok(());
    }

    // Begin transaction
    let txn = context
        .db
        .begin()
        .await
        .context("Failed to begin local transaction")?;
    debug!(
        "Applying {} local operations within transaction.",
        ops_to_apply.len()
    );

    for op in ops_to_apply {
        match op {
            SyncOperation::InsertLocal(model) => {
                let id_str = model.unique_id(); // Get ID for logging before move
                debug!("Local TXN: Inserting record ID {}", id_str);
                // Convert Model to the Entity's specific ActiveModel
                let active_model: E::ActiveModel = model.into_active_model();
                // Ensure PK is NotSet or Default if auto-incrementing
                E::insert(active_model)
                    .exec(&txn)
                    .await
                    .with_context(|| format!("Failed to insert local record ID {}", id_str))?;
            }
            SyncOperation::UpdateLocal(model) => {
                let id_str = model.unique_id(); // Get ID string before moving model
                debug!("Local TXN: Updating record ID {}", id_str);

                // Convert Model to ActiveModel. Ensure PK is Unchanged or excluded if set automatically.
                let active_model: E::ActiveModel = model.into_active_model();

                // Parse the primary key string into the actual PK ValueType
                let pk_value = E::PrimaryKey::read_key(&id_str).with_context(|| {
                    format!("Failed to parse primary key '{}' for update", id_str)
                })?;

                // Use update_many with filter for safety, applying changes from active_model
                // This assumes `into_active_model` prepares the `active_model` with Set values for changed fields
                // and Unchanged/NotSet for PK.
                let res = E::update_many()
                    .set(active_model) // Apply changes defined in active_model
                    .filter(E::unique_id_column().eq(pk_value.clone())) // Filter by PK
                    .exec(&txn)
                    .await;

                // Check affected rows? SeaORM update result doesn't directly expose this easily.
                // We assume the update worked if no error occurred.
                res.with_context(|| format!("Failed to update local record {}", id_str))?;
            }
            SyncOperation::DeleteLocal(id_str) => {
                debug!("Local TXN: Deleting record ID {}", id_str);
                // Parse the primary key string
                let pk_value = E::PrimaryKey::read_key(&id_str).with_context(|| {
                    format!("Failed to parse primary key '{}' for delete", id_str)
                })?;
                // Use delete_many with filter, consistent with update
                let delete_result = E::delete_many()
                    .filter(E::unique_id_column().eq(pk_value.clone()))
                    .exec(&txn)
                    .await
                    .with_context(|| format!("Failed to delete local record {}", id_str))?;

                // Check if any row was actually deleted (optional)
                if delete_result.rows_affected == 0 {
                    warn!(
                        "Local TXN: Delete operation for ID {} affected 0 rows.",
                        id_str
                    );
                }
            }
            SyncOperation::NoOp(_) => { /* Already filtered out */ }
            // Remote operations are ignored in apply_local_changes
            SyncOperation::InsertRemote(_)
            | SyncOperation::UpdateRemote(_)
            | SyncOperation::DeleteRemote(_) => {
                unreachable!("Remote operations should not reach apply_local_changes")
            }
        }
    }

    // Commit transaction
    txn.commit()
        .await
        .context("Failed to commit local transaction")?;
    debug!("Local transaction committed successfully.");
    Ok(())
}

/// Aligns sorted lists of local and remote chunks and populates the reconciliation queue.
///
/// Compares chunks based on HLC ranges.
/// - Perfectly aligned chunks with matching hashes are skipped.
/// - Perfectly aligned chunks with differing hashes are added as `ChunkPair`.
/// - Misaligned or non-overlapping chunks result in `FetchRange` items covering the affected HLC ranges.
fn align_and_queue_chunks(
    local_chunks: Vec<DataChunk>,
    remote_chunks: Vec<DataChunk>,
    queue: &mut VecDeque<ReconciliationItem>,
    max_hlc_encountered: &mut HLC, // Pass mutable ref to update max HLC
) {
    let mut local_idx = 0;
    let mut remote_idx = 0;

    while local_idx < local_chunks.len() || remote_idx < remote_chunks.len() {
        let local_opt = local_chunks.get(local_idx);
        let remote_opt = remote_chunks.get(remote_idx);

        match (local_opt, remote_opt) {
            // Case 1: Both lists have chunks remaining
            (Some(local), Some(remote)) => {
                // Update max HLC seen so far based on chunk boundaries
                update_max_hlc(max_hlc_encountered, &local.end_hlc);
                update_max_hlc(max_hlc_encountered, &remote.end_hlc);

                // Compare chunks based on start HLC first for alignment
                match local.start_hlc.cmp(&remote.start_hlc) {
                    std::cmp::Ordering::Less => {
                        // Local chunk starts earlier -> Assume local-only range for now
                        debug!(
                            "Align: Local chunk [{}-{}] starts first. Queueing FetchRange.",
                            local.start_hlc, local.end_hlc
                        );
                        queue.push_back(ReconciliationItem::FetchRange(ComparisonRange {
                            start_hlc: local.start_hlc.clone(),
                            end_hlc: local.end_hlc.clone(),
                        }));
                        local_idx += 1; // Advance local index
                    }
                    std::cmp::Ordering::Greater => {
                        // Remote chunk starts earlier -> Assume remote-only range for now
                        debug!(
                            "Align: Remote chunk [{}-{}] starts first. Queueing FetchRange.",
                            remote.start_hlc, remote.end_hlc
                        );
                        queue.push_back(ReconciliationItem::FetchRange(ComparisonRange {
                            start_hlc: remote.start_hlc.clone(),
                            end_hlc: remote.end_hlc.clone(),
                        }));
                        remote_idx += 1; // Advance remote index
                    }
                    std::cmp::Ordering::Equal => {
                        // Start HLCs match, now compare end HLCs
                        if local.end_hlc == remote.end_hlc {
                            // Perfect alignment: Start and End HLCs match
                            if local.chunk_hash == remote.chunk_hash {
                                // Hashes match -> Chunks are identical, skip
                                debug!(
                                    "Align: Chunks perfectly aligned and hashes match for [{}-{}]. Skipping.",
                                    local.start_hlc, local.end_hlc
                                );
                            } else {
                                // Hashes differ -> Needs reconciliation
                                debug!(
                                    "Align: Chunks perfectly aligned, hashes differ for [{}-{}]. Queueing ChunkPair.",
                                    local.start_hlc, local.end_hlc
                                );
                                queue.push_back(ReconciliationItem::ChunkPair(
                                    local.clone(),
                                    remote.clone(),
                                ));
                            }
                            // Advance both indexes
                            local_idx += 1;
                            remote_idx += 1;
                        } else {
                            // Start HLCs match, but End HLCs differ -> Misalignment
                            // This indicates overlapping ranges that don't perfectly coincide.
                            // Safest approach: Fetch records for the union of the ranges.
                            let union_start = local.start_hlc.clone(); // Same start
                            let union_end =
                                std::cmp::max(local.end_hlc.clone(), remote.end_hlc.clone());
                            warn!(
                                "Align: Chunk misalignment (start match, end differ) at [{}]. L_end: {}, R_end: {}. Queueing FetchRange for union [{}-{}].",
                                union_start, local.end_hlc, remote.end_hlc, union_start, union_end
                            );
                            queue.push_back(ReconciliationItem::FetchRange(ComparisonRange {
                                start_hlc: union_start,
                                end_hlc: union_end,
                            }));
                            // Advance both indexes past the point of misalignment start
                            local_idx += 1;
                            remote_idx += 1;
                        }
                    }
                }
            }
            // Case 2: Only local chunks left
            (Some(local), None) => {
                update_max_hlc(max_hlc_encountered, &local.end_hlc);
                debug!(
                    "Align: Remaining local chunk [{}-{}]. Queueing FetchRange.",
                    local.start_hlc, local.end_hlc
                );
                queue.push_back(ReconciliationItem::FetchRange(ComparisonRange {
                    start_hlc: local.start_hlc.clone(),
                    end_hlc: local.end_hlc.clone(),
                }));
                local_idx += 1;
            }
            // Case 3: Only remote chunks left
            (None, Some(remote)) => {
                update_max_hlc(max_hlc_encountered, &remote.end_hlc);
                debug!(
                    "Align: Remaining remote chunk [{}-{}]. Queueing FetchRange.",
                    remote.start_hlc, remote.end_hlc
                );
                queue.push_back(ReconciliationItem::FetchRange(ComparisonRange {
                    start_hlc: remote.start_hlc.clone(),
                    end_hlc: remote.end_hlc.clone(),
                }));
                remote_idx += 1;
            }
            // Case 4: Both lists exhausted
            (None, None) => break, // Finished alignment
        }
    }
}

/// Helper to fetch local records strictly *after* a given HLC (exclusive).
/// Not currently used in main flow but potentially useful.
#[allow(dead_code)]
async fn fetch_local_records_after<E>(
    db: &DatabaseConnection,
    start_hlc_exclusive: &HLC,
) -> Result<Vec<E::Model>>
where
    E: HLCModel + EntityTrait + Send + Sync,
    E::Model: HLCRecord + Send + Sync,
{
    E::find()
        .filter(E::gt(start_hlc_exclusive)?) // gt is >
        .order_by_hlc_asc::<E>()
        .all(db)
        .await
        .with_context(|| {
            format!(
                "Failed to fetch local records after HLC {}",
                start_hlc_exclusive
            )
        })
}

/// Helper to fetch local records within an inclusive HLC range.
async fn fetch_local_records_in_range<E>(
    db: &DatabaseConnection,
    start_hlc: &HLC,
    end_hlc: &HLC,
) -> Result<Vec<E::Model>>
where
    E: HLCModel + EntityTrait + Send + Sync,
    E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de>, // Add Deserialize here too if needed
{
    E::find()
        .filter(E::between(start_hlc, end_hlc)?) // between is >= start AND <= end
        .order_by_hlc_asc::<E>() // Consistent ordering
        .all(db)
        .await
        .with_context(|| {
            format!(
                "Failed to fetch local records in HLC range [{}-{}]",
                start_hlc, end_hlc
            )
        })
}

/// Helper to update the maximum HLC seen so far, ensuring monotonicity.
fn update_max_hlc(current_max: &mut HLC, potentially_new: &HLC) {
    if potentially_new > current_max {
        *current_max = potentially_new.clone();
    }
}

// --- Trait for Primary Key Parsing ---

/// Trait required for SeaORM Entities' PrimaryKey types.
/// Enables parsing the string unique ID from `HLCRecord::unique_id()` back
/// into the concrete `ValueType` needed for SeaORM database operations.
/// **Users must implement this for each Entity's PrimaryKey.**
pub trait PrimaryKeyFromStr<T>: PrimaryKeyTrait {
    /// Parses a string representation into the primary key's value type.
    /// `T` should be `Self::ValueType`.
    fn read_key(s: &str) -> Result<T>;
}
