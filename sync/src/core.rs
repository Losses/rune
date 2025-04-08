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
//! # use std::str::FromStr;
//!
//! # // Mock structures to make the example compile standalone
//! # use sea_orm::{entity::prelude::*, ConnectionTrait, DbErr, DeleteResult, ExecResult, InsertResult, UpdateResult, ActiveModelBehavior, ActiveValue, Set, Unchanged, QueryFilter, Condition, IntoActiveModel};
//! # use serde::{Serialize, Deserialize};
//! # use uuid::Uuid;
//! # use anyhow::{Result, Context, anyhow};
//! #
//! # use sync::hlc::{HLC, HLCRecord, HLCModel};
//! # use sync::core::{PrimaryKeyFromStr};
//!
//! #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
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
//! // 1. HLCRecord Implementation (for Model)
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
//! // Boilerplate SeaORM ActiveModelBehavior
//! impl ActiveModelBehavior for ActiveModel {}
//!
//! // 2. HLCModel Implementation (for Entity)
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
//! // 3. PrimaryKeyFromStr Implementation (for Entity::PrimaryKey)
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

use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::hash::Hash;

use anyhow::{anyhow, Context, Result};
#[cfg(not(test))]
use log::{debug, error, info, warn};
#[cfg(test)]
use std::{println as info, println as warn, println as error, println as debug};

use sea_orm::entity::prelude::*;
use sea_orm::{
    ActiveModelBehavior, DatabaseConnection, EntityTrait, IntoActiveModel, Iterable,
    PrimaryKeyTrait, QueryFilter, TransactionTrait, Value,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::chunking::{break_data_chunk, generate_data_chunks, ChunkingOptions, DataChunk};
use crate::hlc::{HLCModel, HLCQuery, HLCRecord, SyncTaskContext, HLC};

/// If a chunk pair has differing hashes, but the maximum record count
/// in either chunk is below or equal to this threshold, fetch individual records directly
/// instead of breaking the chunk down further.
const COMPARISON_THRESHOLD: u64 = 50;

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
        E: HLCModel + EntityTrait + Send + Sync,
        E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize;

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
        E: HLCModel + EntityTrait + Send + Sync,
        E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize;

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
        E: HLCModel + EntityTrait + Send + Sync,
        E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize;

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
        E: HLCModel + EntityTrait + Send + Sync,
        E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize;

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

    // 1. Fetch Initial Chunks
    // Fetch local and remote chunk metadata for data modified *after* the last sync HLC.
    let sync_start_hlc = metadata.last_sync_hlc.clone();
    let local_chunks_fut = generate_data_chunks::<E>(
        context.db,
        &context.chunking_options,
        Some(sync_start_hlc.clone()),
    );
    let remote_chunks_fut = context
        .remote_source
        .get_remote_chunks::<E>(table_name, Some(&sync_start_hlc));

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

    // 2. Reconcile Chunk Differences Recursively/Iteratively
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

    // 3. Merge and Compare Individual Records
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

    // 4. Conflict Resolution and Operation Generation
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

    // 5. Apply Changes Transactionally
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

    // 6. Finalize and Update Metadata
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

// Helper Functions

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
                let id_str = model.unique_id();
                debug!("Local TXN: Updating record ID {}", id_str);

                // 1. Convert the incoming model to ActiveModel.
                //    This ActiveModel might contain an incorrect primary key value
                //    inherited from the `model` (e.g., if it came from remote).
                let mut am_from_model: E::ActiveModel = model.into_active_model().reset_all();

                // 2. Reset the primary key field(s) in the ActiveModel.
                //    This sets the PK fields to `NotSet`, ensuring that the `set`
                //    operation below does not attempt to modify the primary key itself.
                //    The filter condition will target the correct row.
                //    Requires E::PrimaryKey: Into<E::Column> bound.
                for pk_col in E::PrimaryKey::iter() {
                    am_from_model.reset(pk_col.into_column());
                }

                // 3. Perform the update using `update_many` (even for a single row).
                //    Filter by the logical unique ID (`sync_id` in tests).
                //    The `set` method applies only the fields that are `Set`
                //    in `am_from_model` (which now excludes the PKs).
                let update_result = E::update_many()
                    .set(am_from_model) // Apply the changes (PK field is NotSet)
                    .filter(E::unique_id_column().eq(id_str.clone())) // Use ColumnTrait::eq
                    .exec(&txn)
                    .await
                    .with_context(|| {
                        format!("Failed to update local record with unique ID {}", id_str)
                    })?;

                // 4. Check if any row was actually updated.
                if update_result.rows_affected == 0 {
                    // This implies the record with the given unique_id didn't exist locally.
                    // This could happen if a remote delete won a conflict resolution
                    // against a local update, but the delete hasn't been processed yet,
                    // or indicates some other inconsistency.
                    warn!(
                        "Local TXN: Update operation for unique ID {} affected 0 rows. Record might not exist locally or was already deleted.",
                        id_str
                    );
                } else if update_result.rows_affected > 1 {
                    // This should ideally not happen if unique_id_column has a unique constraint.
                    warn!(
                        "Local TXN: Update operation for unique ID {} affected {} rows. Expected 1.",
                        id_str, update_result.rows_affected
                    );
                } else {
                    debug!(
                        "Local TXN: Successfully updated 1 row for unique ID {}",
                        id_str
                    );
                }
            }
            SyncOperation::DeleteLocal(id_str) => {
                debug!("Local TXN: Deleting record ID {}", id_str);
                let delete_result = E::delete_many()
                    // Filter using the unique_id column and the string ID directly
                    .filter(E::unique_id_column().eq(id_str.clone()))
                    .exec(&txn)
                    .await
                    .with_context(|| format!("Failed to delete local record {}", id_str))?;

                // Use delete_many with filter, consistent with update
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

// Trait for Primary Key Parsing

/// Trait required for SeaORM Entities' PrimaryKey types.
/// Enables parsing the string unique ID from `HLCRecord::unique_id()` back
/// into the concrete `ValueType` needed for SeaORM database operations.
/// **Users must implement this for each Entity's PrimaryKey.**
pub trait PrimaryKeyFromStr<T>: PrimaryKeyTrait {
    /// Parses a string representation into the primary key's value type.
    /// `T` should be `Self::ValueType`.
    fn read_key(s: &str) -> Result<T>;
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fmt::Debug;
    use std::sync::Arc;

    use sea_orm::{
        ConnectionTrait, Database, DbBackend, DbConn, NotSet, PrimaryKeyTrait, QueryOrder, Schema,
        Set,
    };
    use serde::{Deserialize, Serialize};
    use tokio::sync::Mutex as TokioMutex; // Use Tokio Mutex for async mocking
    use uuid::Uuid;

    use super::*;
    use crate::chunking::{calculate_chunk_hash, ChunkingOptions, DataChunk};
    use crate::core::PrimaryKeyFromStr;
    use crate::hlc::{hlc_timestamp_millis_to_rfc3339, HLCModel, HLCRecord, SyncTaskContext, HLC};

    mod test_entity {
        use std::str::FromStr;

        use anyhow::{anyhow, Result};
        use sea_orm::{
            ActiveModelBehavior, DeriveEntityModel, DerivePrimaryKey, DeriveRelation, EnumIter,
        };
        use serde::{Deserialize, Serialize};
        use uuid::Uuid;

        use crate::hlc::{HLCModel, HLCRecord, HLC};

        use super::*;

        #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
        #[sea_orm(table_name = "test_items")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = true)]
            pub id: i32,
            #[sea_orm(unique)]
            pub sync_id: String,
            pub name: String,
            pub value: Option<i32>,
            #[sea_orm(column_type = "Text")]
            pub created_at_hlc_ts: String,
            pub created_at_hlc_ct: i32,
            pub created_at_hlc_id: Uuid,
            #[sea_orm(column_type = "Text")]
            pub updated_at_hlc_ts: String,
            pub updated_at_hlc_ct: i32,
            pub updated_at_hlc_id: Uuid,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}
        impl ActiveModelBehavior for ActiveModel {}

        impl HLCRecord for Model {
            fn created_at_hlc(&self) -> Option<HLC> {
                match chrono::DateTime::parse_from_rfc3339(&self.created_at_hlc_ts) {
                    Ok(dt) => Some(HLC {
                        timestamp: dt.timestamp_millis() as u64,
                        version: self.created_at_hlc_ct as u32,
                        node_id: self.created_at_hlc_id,
                    }),
                    Err(e) => {
                        eprintln!(
                            "Error parsing created_at HLC timestamp {}: {}",
                            self.created_at_hlc_ts, e
                        );
                        None
                    }
                }
            }

            fn updated_at_hlc(&self) -> Option<HLC> {
                match chrono::DateTime::parse_from_rfc3339(&self.updated_at_hlc_ts) {
                    Ok(dt) => Some(HLC {
                        timestamp: dt.timestamp_millis() as u64,
                        version: self.updated_at_hlc_ct as u32,
                        node_id: self.updated_at_hlc_id,
                    }),
                    Err(e) => {
                        eprintln!(
                            "Error parsing updated_at HLC timestamp {}: {}",
                            self.updated_at_hlc_ts, e
                        );
                        None
                    }
                }
            }

            fn unique_id(&self) -> String {
                self.sync_id.clone()
            }

            fn data_for_hashing(&self) -> serde_json::Value {
                serde_json::json!({
                    "sync_id": self.sync_id,
                    "name": self.name,
                    "value": self.value,
                    // Omit created/updated HLC fields
                })
            }
        }

        impl HLCModel for Entity {
            fn updated_at_time_column() -> Self::Column {
                Column::UpdatedAtHlcTs
            }
            fn updated_at_version_column() -> Self::Column {
                Column::UpdatedAtHlcCt
            }
            fn unique_id_column() -> Self::Column {
                Column::SyncId
            }
        }

        // Implement PrimaryKeyFromStr for the Entity's PrimaryKey
        impl PrimaryKeyFromStr<<Self as PrimaryKeyTrait>::ValueType> for PrimaryKey
        where
            i32: FromStr,
            <i32 as FromStr>::Err: std::error::Error + Send + Sync + 'static,
        {
            fn read_key(s: &str) -> Result<<Self as PrimaryKeyTrait>::ValueType> {
                s.parse::<i32>() // Parse as i32
                    .map_err(|e| {
                        anyhow!(e)
                            .context(format!("Failed to parse primary key string '{}' as i32", s))
                    })
            }
        }
    }

    #[derive(Debug, Clone)]
    struct MockRemoteDataSource {
        node_id: Uuid,
        remote_data: Arc<TokioMutex<HashMap<String, Model>>>,
        remote_chunks: Arc<TokioMutex<Vec<DataChunk>>>,
        applied_ops: Arc<TokioMutex<Vec<SyncOperation<Model>>>>,
        fail_on_apply: bool,
        fail_on_get_records: bool,
        fail_on_get_chunks: bool,
        fail_on_get_sub_chunks: bool,
        sub_chunk_requests: Arc<TokioMutex<Vec<(DataChunk, u64)>>>,
        get_records_calls: Arc<TokioMutex<Vec<(HLC, HLC)>>>, // Track get_records calls
    }

    impl MockRemoteDataSource {
        fn new(node_id: Uuid) -> Self {
            MockRemoteDataSource {
                node_id,
                remote_data: Arc::new(TokioMutex::new(HashMap::new())),
                remote_chunks: Arc::new(TokioMutex::new(Vec::new())),
                applied_ops: Arc::new(TokioMutex::new(Vec::new())),
                fail_on_apply: false,
                fail_on_get_records: false,
                fail_on_get_chunks: false,
                fail_on_get_sub_chunks: false,
                sub_chunk_requests: Arc::new(TokioMutex::new(Vec::new())),
                get_records_calls: Arc::new(TokioMutex::new(Vec::new())),
            }
        }

        async fn set_remote_data(&self, data: Vec<Model>) {
            let mut guard = self.remote_data.lock().await;
            guard.clear();
            for item in data {
                guard.insert(item.sync_id.clone(), item);
            }
        }

        async fn set_remote_chunks(&self, chunks: Vec<DataChunk>) {
            let mut guard = self.remote_chunks.lock().await;
            *guard = chunks;
        }

        async fn get_applied_ops(&self) -> Vec<SyncOperation<Model>> {
            self.applied_ops.lock().await.clone()
        }

        async fn get_sub_chunk_requests(&self) -> Vec<(DataChunk, u64)> {
            self.sub_chunk_requests.lock().await.clone()
        }

        async fn get_records_call_ranges(&self) -> Vec<(HLC, HLC)> {
            self.get_records_calls.lock().await.clone()
        }

        // Helper to clear mock state between tests if needed
        #[allow(dead_code)]
        async fn clear(&mut self) {
            self.remote_data.lock().await.clear();
            self.remote_chunks.lock().await.clear();
            self.applied_ops.lock().await.clear();
            self.sub_chunk_requests.lock().await.clear();
            self.get_records_calls.lock().await.clear();
            self.fail_on_apply = false;
            self.fail_on_get_records = false;
            self.fail_on_get_chunks = false;
            self.fail_on_get_sub_chunks = false;
        }
    }

    #[async_trait::async_trait]
    impl RemoteDataSource for MockRemoteDataSource {
        async fn get_remote_node_id(&self) -> Result<Uuid> {
            Ok(self.node_id)
        }

        async fn get_remote_chunks<E>(
            &self,
            _table_name: &str,
            after_hlc: Option<&HLC>,
        ) -> Result<Vec<DataChunk>>
        where
            E: HLCModel + EntityTrait + Send + Sync,
            E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize + Debug,
        {
            if self.fail_on_get_chunks {
                return Err(anyhow!("Simulated failure getting remote chunks"));
            }
            let guard = self.remote_chunks.lock().await;
            let filtered_chunks = match after_hlc {
                Some(start) => guard
                    .iter()
                    .filter(|c| &c.start_hlc > start) // Compare &HLC with &HLC
                    .cloned()
                    .collect(),
                None => guard.clone(),
            };
            let mut sorted = filtered_chunks;
            sorted.sort_by(|a, b| a.start_hlc.cmp(&b.start_hlc));
            Ok(sorted)
        }

        async fn get_remote_sub_chunks<E>(
            &self,
            _table_name: &str,
            parent_chunk: &DataChunk,
            sub_chunk_size: u64,
        ) -> Result<Vec<DataChunk>>
        where
            E: HLCModel + EntityTrait + Send + Sync,
            E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize + Debug,
        {
            // Record the request
            self.sub_chunk_requests
                .lock()
                .await
                .push((parent_chunk.clone(), sub_chunk_size));

            if self.fail_on_get_sub_chunks {
                return Err(anyhow!("Simulated failure getting remote sub-chunks"));
            }
            // Simulate breakdown based on actual remote data
            let data_guard = self.remote_data.lock().await;
            let mut records_in_range: Vec<Model> = data_guard
                .values()
                .filter_map(|m| {
                    m.updated_at_hlc().and_then(|hlc| {
                        // Inclusive range check
                        if hlc >= parent_chunk.start_hlc && hlc <= parent_chunk.end_hlc {
                            Some(m.clone())
                        } else {
                            None
                        }
                    })
                })
                .collect();
            records_in_range.sort_by_key(|m| m.updated_at_hlc()); // Sort by HLC

            let mut sub_chunks = Vec::new();
            if records_in_range.is_empty() {
                return Ok(sub_chunks);
            }

            // Generate chunks based on the requested size
            for chunk_slice in records_in_range.chunks(sub_chunk_size as usize) {
                if chunk_slice.is_empty() {
                    continue;
                }
                let first_hlc = chunk_slice.first().unwrap().updated_at_hlc().unwrap();
                let last_hlc = chunk_slice.last().unwrap().updated_at_hlc().unwrap();
                let count = chunk_slice.len() as u64;
                // Calculate hash based on the concrete Model type
                let hash = calculate_chunk_hash::<Model>(chunk_slice)
                    .context("Failed to calculate sub-chunk hash")?;

                sub_chunks.push(DataChunk {
                    start_hlc: first_hlc,
                    end_hlc: last_hlc,
                    count,
                    chunk_hash: hash,
                });
            }
            Ok(sub_chunks)
        }

        async fn get_remote_records_in_hlc_range<E>(
            &self,
            _table_name: &str,
            start_hlc: &HLC,
            end_hlc: &HLC,
        ) -> Result<Vec<E::Model>>
        where
            E: HLCModel + EntityTrait + Send + Sync,
            E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize + Debug, // Added Debug
        {
            // Record the call
            self.get_records_calls
                .lock()
                .await
                .push((start_hlc.clone(), end_hlc.clone()));

            if self.fail_on_get_records {
                return Err(anyhow!("Simulated failure getting remote records"));
            }
            let guard = self.remote_data.lock().await;
            let mut records: Vec<Model> = guard // Use concrete Model type
                .values()
                .filter_map(|m| {
                    m.updated_at_hlc().and_then(|hlc| {
                        // Inclusive range check
                        if hlc >= *start_hlc && hlc <= *end_hlc {
                            Some(m.clone())
                        } else {
                            None
                        }
                    })
                })
                .collect();
            records.sort_by_key(|m| m.updated_at_hlc()); // Ensure sorted order

            // Convert Vec<Model> to Vec<E::Model> using Serde for safety
            let mut result_vec = Vec::new();
            for model in records {
                let json_val =
                    serde_json::to_value(&model).context("Failed to serialize mock record")?;
                let e_model: E::Model = serde_json::from_value(json_val)
                    .context("Failed to deserialize mock record into target type")?;
                result_vec.push(e_model);
            }
            Ok(result_vec)
        }

        async fn apply_remote_changes<E>(
            &self,
            _table_name: &str,
            operations: Vec<SyncOperation<E::Model>>,
        ) -> Result<HLC>
        where
            E: HLCModel + EntityTrait + Send + Sync,
            E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize + Debug, // Added Debug
        {
            if self.fail_on_apply {
                return Err(anyhow!("Simulated remote apply failure"));
            }

            let mut data_guard = self.remote_data.lock().await;
            let mut ops_guard = self.applied_ops.lock().await;
            let mut max_hlc = HLC::new(self.node_id); // Start with a base HLC

            for op in operations {
                // Convert E::Model to concrete Model for storage/logging in mock
                let op_model: SyncOperation<Model> = match op {
                    SyncOperation::InsertRemote(m) => {
                        // Separate arm for InsertRemote
                        let json_val = serde_json::to_value(&m)
                            .context("Serialize failed in InsertRemote arm")?;
                        let model: Model = serde_json::from_value(json_val)
                            .context("Deserialize failed in InsertRemote arm")?;
                        SyncOperation::InsertRemote(model)
                    }
                    SyncOperation::UpdateRemote(m) => {
                        // Separate arm for UpdateRemote
                        let json_val = serde_json::to_value(&m)
                            .context("Serialize failed in UpdateRemote arm")?;
                        let model: Model = serde_json::from_value(json_val)
                            .context("Deserialize failed in UpdateRemote arm")?;
                        SyncOperation::UpdateRemote(model)
                    }
                    SyncOperation::DeleteRemote(sync_id) => SyncOperation::DeleteRemote(sync_id),
                    // These shouldn't normally be passed here, but handle for completeness/logging
                    SyncOperation::InsertLocal(m) => SyncOperation::InsertLocal(
                        serde_json::from_value(serde_json::to_value(&m)?)?,
                    ),
                    SyncOperation::UpdateLocal(m) => SyncOperation::UpdateLocal(
                        serde_json::from_value(serde_json::to_value(&m)?)?,
                    ),
                    SyncOperation::DeleteLocal(pk_str) => SyncOperation::DeleteLocal(pk_str),
                    SyncOperation::NoOp(sync_id) => SyncOperation::NoOp(sync_id),
                };

                ops_guard.push(op_model.clone()); // Store the concrete operation

                // Simulate applying the change to the mock's data store
                match op_model {
                    SyncOperation::InsertRemote(model) | SyncOperation::UpdateRemote(model) => {
                        if let Some(hlc) = model.updated_at_hlc() {
                            if hlc > max_hlc {
                                max_hlc = hlc; // Use clone here? No, hlc is owned/moved or copied
                            }
                        }
                        data_guard.insert(model.sync_id.clone(), model);
                    }
                    SyncOperation::DeleteRemote(sync_id) => {
                        data_guard.remove(&sync_id);
                        // Deletes don't typically contribute to max_hlc unless tracked via tombstone HLC
                    }
                    // Ignore local ops or NoOps for data modification and max_hlc
                    _ => {}
                }
            }
            // If no operations modified data, return the node's base HLC, otherwise return max_hlc found
            if ops_guard.iter().any(|op| {
                matches!(
                    op,
                    SyncOperation::InsertRemote(_)
                        | SyncOperation::UpdateRemote(_)
                        | SyncOperation::DeleteRemote(_)
                )
            }) {
                Ok(max_hlc)
            } else {
                Ok(HLC::new(self.node_id)) // Return base HLC if no real ops
            }
        }

        async fn get_remote_last_sync_hlc(
            &self,
            _table_name: &str,
            _local_node_id: Uuid,
        ) -> Result<Option<HLC>> {
            Ok(None)
        }
    }

    use test_entity::{ActiveModel, Column, Entity, Model}; // Ensure PrimaryKey is imported
    async fn setup_db() -> Result<DbConn> {
        let db = Database::connect("sqlite::memory:").await?;
        let schema = Schema::new(DbBackend::Sqlite);
        let stmt = schema.create_table_from_entity(Entity); // Use test_entity::Entity
        db.execute(db.get_database_backend().build(&stmt)).await?;
        Ok(db)
    }

    fn hlc(ts: u64, v: u32, node_str: &str) -> HLC {
        HLC {
            timestamp: ts,
            version: v,
            node_id: Uuid::parse_str(node_str).unwrap(),
        }
    }

    async fn insert_test_record(
        db: &DbConn,
        sync_id: &str,
        name: &str,
        val: Option<i32>,
        created_hlc: &HLC,
        updated_hlc: &HLC,
    ) -> Result<Model> {
        let created_ts_str = hlc_timestamp_millis_to_rfc3339(created_hlc.timestamp)?;
        let updated_ts_str = hlc_timestamp_millis_to_rfc3339(updated_hlc.timestamp)?;

        let model = ActiveModel {
            id: NotSet,
            sync_id: Set(sync_id.to_string()),
            name: Set(name.to_string()),
            value: Set(val),
            created_at_hlc_ts: Set(created_ts_str.clone()),
            created_at_hlc_ct: Set(created_hlc.version as i32),
            created_at_hlc_id: Set(created_hlc.node_id),
            updated_at_hlc_ts: Set(updated_ts_str.clone()),
            updated_at_hlc_ct: Set(updated_hlc.version as i32),
            updated_at_hlc_id: Set(updated_hlc.node_id),
        };
        Ok(Entity::insert(model).exec_with_returning(db).await?) // Use test_entity::Entity
    }

    const LOCAL_NODE_STR: &str = "11111111-1111-1111-1111-111111111111";
    const REMOTE_NODE_STR: &str = "22222222-2222-2222-2222-222222222222";
    const BASE_TS: u64 = 1700000000000;

    #[tokio::test]
    async fn test_synchronize_table_no_changes() -> Result<()> {
        // ... (existing test code)
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?;
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let remote_source = MockRemoteDataSource::new(remote_node_id);

        let start_hlc = hlc(BASE_TS - 2000, 0, LOCAL_NODE_STR); // Start really early
        let data_hlc = hlc(BASE_TS - 1000, 0, LOCAL_NODE_STR); // Data after initial, before 'now'

        // Insert identical record locally and remotely
        let record =
            insert_test_record(&db, "sync_nochange", "Same", Some(1), &data_hlc, &data_hlc).await?;
        remote_source.set_remote_data(vec![record.clone()]).await;

        let hlc_context = SyncTaskContext::new(local_node_id);
        let options = ChunkingOptions {
            // Use small chunks to ensure chunking happens
            min_size: 1,
            max_size: 1,
            alpha: 0.0,
            node_id: local_node_id,
        };
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options.clone(),
            sync_direction: SyncDirection::Bidirectional,
            hlc_context: &hlc_context,
        };
        let initial_metadata = SyncTableMetadata {
            table_name: "test_items".to_string(),
            last_sync_hlc: start_hlc.clone(),
        };

        // Generate local chunks and set them as remote chunks for the mock
        let local_chunks = generate_data_chunks::<Entity>(&db, &options, Some(start_hlc)).await?;
        remote_source.set_remote_chunks(local_chunks.clone()).await;

        assert!(
            !local_chunks.is_empty(),
            "Should have generated at least one chunk"
        );

        let final_metadata =
            synchronize_table::<Entity, _>(&context, "test_items", &initial_metadata).await?;

        // Assertions
        let applied_ops = remote_source.get_applied_ops().await;
        assert!(
            applied_ops
                .iter()
                .all(|op| matches!(op, SyncOperation::NoOp(_))),
            "No real operations should have been applied remotely"
        ); // Should be NoOp

        let get_records_calls = remote_source.get_records_call_ranges().await;
        assert!(
            get_records_calls.is_empty(),
            "Should not have fetched records when chunks match"
        );

        let local_final_data = Entity::find().all(&db).await?;
        assert_eq!(local_final_data.len(), 1); // Data remains
        assert_eq!(final_metadata.last_sync_hlc, data_hlc); // HLC advances to the latest data HLC

        Ok(())
    }

    #[tokio::test]
    async fn test_synchronize_table_local_only_insert_bidirectional() -> Result<()> {
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?;
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let remote_source = MockRemoteDataSource::new(remote_node_id);

        let start_hlc = hlc(BASE_TS, 0, LOCAL_NODE_STR);
        let hlc1 = hlc(BASE_TS + 100, 0, LOCAL_NODE_STR);

        // Insert local record *after* starting sync
        insert_test_record(&db, "sync_local1", "NewLocal", Some(1), &hlc1, &hlc1).await?;

        let hlc_context = SyncTaskContext::new(local_node_id);
        let options = ChunkingOptions {
            min_size: 1,
            max_size: 1,
            alpha: 0.0,
            node_id: local_node_id,
        };
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options,
            sync_direction: SyncDirection::Bidirectional,
            hlc_context: &hlc_context,
        };
        let initial_metadata = SyncTableMetadata {
            table_name: "test_items".to_string(),
            last_sync_hlc: start_hlc.clone(),
        };
        // Remote has no chunks initially
        remote_source.set_remote_chunks(vec![]).await;

        let final_metadata =
            synchronize_table::<Entity, _>(&context, "test_items", &initial_metadata).await?;

        let applied_ops = remote_source.get_applied_ops().await;
        assert_eq!(applied_ops.len(), 1);
        match &applied_ops[0] {
            SyncOperation::InsertRemote(model) => {
                assert_eq!(model.sync_id, "sync_local1");
                assert_eq!(model.name, "NewLocal");
                assert_eq!(model.updated_at_hlc().unwrap(), hlc1);
            }
            _ => panic!("Expected InsertRemote operation"),
        }

        let remote_data_guard = remote_source.remote_data.lock().await;
        assert_eq!(remote_data_guard.len(), 1);
        assert!(remote_data_guard.contains_key("sync_local1"));

        let local_final_data = Entity::find().all(&db).await?;
        assert_eq!(local_final_data.len(), 1); // Local data remains
        assert_eq!(final_metadata.last_sync_hlc, hlc1);

        Ok(())
    }

    #[tokio::test]
    async fn test_synchronize_table_remote_only_insert_bidirectional() -> Result<()> {
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?;
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let remote_source = MockRemoteDataSource::new(remote_node_id);

        let start_hlc = hlc(BASE_TS, 0, LOCAL_NODE_STR);
        let hlc1 = hlc(BASE_TS + 100, 0, REMOTE_NODE_STR);
        let remote_record = Model {
            id: 999, // Mock PK, doesn't matter for remote state
            sync_id: "sync_remote1".to_string(),
            name: "NewRemote".to_string(),
            value: Some(2),
            created_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc1.timestamp)?,
            created_at_hlc_ct: hlc1.version as i32,
            created_at_hlc_id: hlc1.node_id,
            updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc1.timestamp)?,
            updated_at_hlc_ct: hlc1.version as i32,
            updated_at_hlc_id: hlc1.node_id,
        };
        remote_source
            .set_remote_data(vec![remote_record.clone()])
            .await;
        let remote_chunk = DataChunk {
            start_hlc: hlc1.clone(),
            end_hlc: hlc1.clone(),
            count: 1,
            chunk_hash: calculate_chunk_hash(&[remote_record])?,
        };
        remote_source.set_remote_chunks(vec![remote_chunk]).await;

        let hlc_context = SyncTaskContext::new(local_node_id);
        let options = ChunkingOptions {
            min_size: 1,
            max_size: 1,
            alpha: 0.0,
            node_id: local_node_id,
        };
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options,
            sync_direction: SyncDirection::Bidirectional,
            hlc_context: &hlc_context,
        };
        let initial_metadata = SyncTableMetadata {
            table_name: "test_items".to_string(),
            last_sync_hlc: start_hlc.clone(),
        };

        let final_metadata =
            synchronize_table::<Entity, _>(&context, "test_items", &initial_metadata).await?;

        let applied_ops = remote_source.get_applied_ops().await;
        assert!(
            applied_ops.is_empty()
                || applied_ops
                    .iter()
                    .all(|op| matches!(op, SyncOperation::NoOp(_))),
            "No real ops should be sent to remote"
        );

        let local_final_data = Entity::find().all(&db).await?;
        assert_eq!(local_final_data.len(), 1);
        assert_eq!(local_final_data[0].sync_id, "sync_remote1");
        assert_eq!(local_final_data[0].name, "NewRemote");
        assert_eq!(local_final_data[0].updated_at_hlc().unwrap(), hlc1);
        assert_eq!(final_metadata.last_sync_hlc, hlc1);

        Ok(())
    }

    #[tokio::test]
    async fn test_synchronize_table_local_wins_conflict() -> Result<()> {
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?;
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let remote_source = MockRemoteDataSource::new(remote_node_id);

        let start_hlc = hlc(BASE_TS, 0, LOCAL_NODE_STR);
        let hlc_remote_old = hlc(BASE_TS + 100, 0, REMOTE_NODE_STR);
        let hlc_local_new = hlc(BASE_TS + 200, 0, LOCAL_NODE_STR); // Local has higher HLC

        // Initial state: Both have the record, but local is newer
        let _local_initial = insert_test_record(
            &db,
            "sync_conflict1",
            "LocalWin",
            Some(1),
            &hlc_remote_old,
            &hlc_local_new,
        )
        .await?; // Use remote HLC for creation, local for update
        let remote_record = Model {
            id: 998, // Mock PK
            sync_id: "sync_conflict1".to_string(),
            name: "RemoteOld".to_string(),
            value: Some(99),
            created_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_remote_old.timestamp)?,
            created_at_hlc_ct: hlc_remote_old.version as i32,
            created_at_hlc_id: hlc_remote_old.node_id,
            updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_remote_old.timestamp)?, // Older HLC
            updated_at_hlc_ct: hlc_remote_old.version as i32,
            updated_at_hlc_id: hlc_remote_old.node_id,
        };
        remote_source
            .set_remote_data(vec![remote_record.clone()])
            .await;

        // Setup chunks - need one local, one remote covering the HLCs, with different hashes
        let options = ChunkingOptions {
            min_size: 1,
            max_size: 1,
            alpha: 0.0,
            node_id: local_node_id,
        };
        let local_chunks =
            generate_data_chunks::<Entity>(&db, &options, Some(start_hlc.clone())).await?;
        let remote_chunk = DataChunk {
            start_hlc: hlc_remote_old.clone(), // Assume remote chunk covers its update time
            end_hlc: hlc_remote_old.clone(),
            count: 1,
            chunk_hash: calculate_chunk_hash(&[remote_record])?,
        };
        remote_source.set_remote_chunks(vec![remote_chunk]).await;
        // Ensure local chunk covers its HLC too
        assert!(
            local_chunks
                .iter()
                .any(|c| c.start_hlc <= hlc_local_new && c.end_hlc >= hlc_local_new),
            "Local chunk should cover the new HLC"
        );
        assert_ne!(
            local_chunks[0].chunk_hash,
            remote_source.remote_chunks.lock().await[0].chunk_hash,
            "Chunk hashes must differ"
        );

        let hlc_context = SyncTaskContext::new(local_node_id);
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options,
            sync_direction: SyncDirection::Bidirectional,
            hlc_context: &hlc_context,
        };
        let initial_metadata = SyncTableMetadata {
            table_name: "test_items".to_string(),
            last_sync_hlc: start_hlc.clone(),
        };

        let final_metadata =
            synchronize_table::<Entity, _>(&context, "test_items", &initial_metadata).await?;

        let applied_ops = remote_source.get_applied_ops().await;
        assert_eq!(applied_ops.len(), 1);
        match &applied_ops[0] {
            SyncOperation::UpdateRemote(model) => {
                assert_eq!(model.sync_id, "sync_conflict1");
                assert_eq!(model.name, "LocalWin");
                assert_eq!(model.updated_at_hlc().unwrap(), hlc_local_new);
            }
            op => panic!("Expected UpdateRemote operation, got {:?}", op),
        }

        let remote_data_guard = remote_source.remote_data.lock().await;
        assert_eq!(remote_data_guard.len(), 1);
        assert_eq!(
            remote_data_guard.get("sync_conflict1").unwrap().name,
            "LocalWin"
        );

        let local_final_data = Entity::find().all(&db).await?;
        assert_eq!(local_final_data.len(), 1);
        assert_eq!(local_final_data[0].name, "LocalWin"); // Local data remains the winner
        assert_eq!(final_metadata.last_sync_hlc, hlc_local_new); // Max HLC encountered

        Ok(())
    }

    #[tokio::test]
    async fn test_synchronize_table_remote_wins_conflict() -> Result<()> {
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?;
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let remote_source = MockRemoteDataSource::new(remote_node_id);

        let start_hlc = hlc(BASE_TS, 0, LOCAL_NODE_STR);
        let hlc_local_old = hlc(BASE_TS + 100, 0, LOCAL_NODE_STR);
        let hlc_remote_new = hlc(BASE_TS + 200, 0, REMOTE_NODE_STR); // Remote has higher HLC

        // Initial state: Both have the record, but remote is newer
        let _local_initial = insert_test_record(
            &db,
            "sync_conflict2",
            "LocalOld",
            Some(1),
            &hlc_local_old,
            &hlc_local_old,
        )
        .await?;
        let remote_record = Model {
            id: 997, // Mock PK
            sync_id: "sync_conflict2".to_string(),
            name: "RemoteWin".to_string(),
            value: Some(100),
            created_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_local_old.timestamp)?, // Assume same creation HLC for simplicity
            created_at_hlc_ct: hlc_local_old.version as i32,
            created_at_hlc_id: hlc_local_old.node_id,
            updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_remote_new.timestamp)?, // Newer HLC
            updated_at_hlc_ct: hlc_remote_new.version as i32,
            updated_at_hlc_id: hlc_remote_new.node_id,
        };
        remote_source
            .set_remote_data(vec![remote_record.clone()])
            .await;

        // Setup chunks
        let options = ChunkingOptions {
            min_size: 1,
            max_size: 1,
            alpha: 0.0,
            node_id: local_node_id,
        };
        let local_chunks =
            generate_data_chunks::<Entity>(&db, &options, Some(start_hlc.clone())).await?;
        let remote_chunk = DataChunk {
            start_hlc: hlc_remote_new.clone(), // Chunk covers the new HLC
            end_hlc: hlc_remote_new.clone(),
            count: 1,
            chunk_hash: calculate_chunk_hash(&[remote_record.clone()])?,
        };
        remote_source.set_remote_chunks(vec![remote_chunk]).await;
        assert!(!local_chunks.is_empty());
        assert_ne!(
            local_chunks[0].chunk_hash,
            remote_source.remote_chunks.lock().await[0].chunk_hash
        );

        let hlc_context = SyncTaskContext::new(local_node_id);
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options,
            sync_direction: SyncDirection::Bidirectional,
            hlc_context: &hlc_context,
        };
        let initial_metadata = SyncTableMetadata {
            table_name: "test_items".to_string(),
            last_sync_hlc: start_hlc.clone(),
        };

        let final_metadata =
            synchronize_table::<Entity, _>(&context, "test_items", &initial_metadata).await?;

        let applied_ops = remote_source.get_applied_ops().await;
        assert!(
            applied_ops.is_empty()
                || applied_ops
                    .iter()
                    .all(|op| matches!(op, SyncOperation::NoOp(_))),
            "No real ops should be sent to remote"
        ); // Remote already has winner

        let local_final_data = Entity::find().all(&db).await?;
        assert_eq!(local_final_data.len(), 1);
        assert_eq!(local_final_data[0].name, "RemoteWin"); // Local data updated
        assert_eq!(
            local_final_data[0].updated_at_hlc().unwrap(),
            hlc_remote_new
        );
        assert_eq!(final_metadata.last_sync_hlc, hlc_remote_new); // Max HLC encountered

        Ok(())
    }

    #[tokio::test]
    async fn test_synchronize_table_tie_break_local_wins() -> Result<()> {
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?; // Smaller UUID
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let remote_source = MockRemoteDataSource::new(remote_node_id);

        let start_hlc = hlc(BASE_TS, 0, LOCAL_NODE_STR);
        let common_hlc = hlc(BASE_TS + 100, 0, &local_node_id.to_string()); // Base HLC ts/v
        let hlc_local_tie = HLC {
            node_id: local_node_id,
            ..common_hlc
        };
        let hlc_remote_tie = HLC {
            node_id: remote_node_id,
            ..common_hlc
        }; // Same ts/v, different node

        // Initial state: Both updated to the same HLC ts/v, but with different node IDs and data
        let _local_initial = insert_test_record(
            &db,
            "sync_tie1",
            "LocalTie",
            Some(1),
            &common_hlc,
            &hlc_local_tie,
        )
        .await?;
        let remote_record = Model {
            id: 996, // Mock PK
            sync_id: "sync_tie1".to_string(),
            name: "RemoteTie".to_string(),
            value: Some(2),
            created_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(common_hlc.timestamp)?,
            created_at_hlc_ct: common_hlc.version as i32,
            created_at_hlc_id: common_hlc.node_id,
            updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_remote_tie.timestamp)?, // Same ts
            updated_at_hlc_ct: hlc_remote_tie.version as i32,                              // Same v
            updated_at_hlc_id: hlc_remote_tie.node_id, // Different node
        };
        remote_source
            .set_remote_data(vec![remote_record.clone()])
            .await;

        // Setup chunks - need differing hashes
        let options = ChunkingOptions {
            min_size: 1,
            max_size: 1,
            alpha: 0.0,
            node_id: local_node_id,
        };
        let local_chunks =
            generate_data_chunks::<Entity>(&db, &options, Some(start_hlc.clone())).await?;
        let remote_chunk = DataChunk {
            start_hlc: hlc_remote_tie.clone(), // Chunk HLC is specific to node
            end_hlc: hlc_remote_tie.clone(),
            count: 1,
            chunk_hash: calculate_chunk_hash(&[remote_record])?,
        };
        remote_source.set_remote_chunks(vec![remote_chunk]).await;
        assert!(!local_chunks.is_empty());
        // The start/end HLCs will differ due to node_id, so align_and_queue_chunks will likely FetchRange.
        // Let's ensure the test doesn't rely on specific chunk alignment logic details.

        let hlc_context = SyncTaskContext::new(local_node_id);
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options,
            sync_direction: SyncDirection::Bidirectional,
            hlc_context: &hlc_context,
        };
        let initial_metadata = SyncTableMetadata {
            table_name: "test_items".to_string(),
            last_sync_hlc: start_hlc.clone(),
        };

        let final_metadata =
            synchronize_table::<Entity, _>(&context, "test_items", &initial_metadata).await?;

        let applied_ops = remote_source.get_applied_ops().await;
        assert_eq!(applied_ops.len(), 1);
        match &applied_ops[0] {
            SyncOperation::UpdateRemote(model) => {
                assert_eq!(model.sync_id, "sync_tie1");
                assert_eq!(model.name, "LocalTie"); // Local wins tie-break
                assert_eq!(model.updated_at_hlc().unwrap(), hlc_local_tie);
            }
            op => panic!("Expected UpdateRemote operation, got {:?}", op),
        }

        let remote_data_guard = remote_source.remote_data.lock().await;
        assert_eq!(remote_data_guard.get("sync_tie1").unwrap().name, "LocalTie");
        let local_final_data = Entity::find().all(&db).await?;
        assert_eq!(local_final_data[0].name, "LocalTie");

        // Max HLC encountered should be the one with the higher node ID when ts/v are equal
        assert_eq!(final_metadata.last_sync_hlc, hlc_remote_tie);

        Ok(())
    }

    #[tokio::test]
    async fn test_apply_local_changes_commit() -> Result<()> {
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?;
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let remote_source = MockRemoteDataSource::new(remote_node_id);

        let hlc_c = hlc(BASE_TS, 0, LOCAL_NODE_STR); // Creation
        let hlc_i = hlc(BASE_TS + 1, 0, REMOTE_NODE_STR); // Insert comes from "remote" conceptually
        let hlc_u = hlc(BASE_TS + 2, 0, REMOTE_NODE_STR); // Update comes from "remote"

        let initial_record_u =
            insert_test_record(&db, "sync_u", "UpdateMe", Some(1), &hlc_c, &hlc_c).await?;
        let initial_record_d =
            insert_test_record(&db, "sync_d", "DeleteMe", Some(2), &hlc_c, &hlc_c).await?;

        let ops = vec![
            SyncOperation::InsertLocal(Model {
                id: 0, // Placeholder
                sync_id: "sync_i".to_string(),
                name: "Inserted".to_string(),
                value: Some(10),
                created_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_i.timestamp)?,
                created_at_hlc_ct: hlc_i.version as i32,
                created_at_hlc_id: hlc_i.node_id,
                updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_i.timestamp)?,
                updated_at_hlc_ct: hlc_i.version as i32,
                updated_at_hlc_id: hlc_i.node_id,
            }),
            SyncOperation::UpdateLocal(Model {
                id: initial_record_u.id, // Use actual PK
                sync_id: "sync_u".to_string(),
                name: "Updated".to_string(),
                value: Some(11),
                // Preserve original creation HLC fields
                created_at_hlc_ts: initial_record_u.created_at_hlc_ts.clone(),
                created_at_hlc_ct: initial_record_u.created_at_hlc_ct,
                created_at_hlc_id: initial_record_u.created_at_hlc_id,
                // Set new update HLC fields
                updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_u.timestamp)?,
                updated_at_hlc_ct: hlc_u.version as i32,
                updated_at_hlc_id: hlc_u.node_id,
            }),
            // Pass the sync_id for deletion, apply_local_changes uses it with unique_id_column
            SyncOperation::DeleteLocal(initial_record_d.sync_id),
            SyncOperation::NoOp("sync_noop".to_string()), // Should be ignored
        ];

        let hlc_context = SyncTaskContext::new(local_node_id);
        let options = ChunkingOptions::default(local_node_id);
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options,
            sync_direction: SyncDirection::Bidirectional, // Doesn't affect apply_local_changes
            hlc_context: &hlc_context,
        };

        apply_local_changes::<Entity>(&context, ops).await?;

        // Verify DB state after commit
        let final_data = Entity::find().order_by_asc(Column::SyncId).all(&db).await?; // Order by sync_id for consistent results
        assert_eq!(final_data.len(), 2); // sync_i and sync_u remain

        println!("FINAL DATA: {:#?}", final_data);

        assert_eq!(final_data[0].sync_id, "sync_i");
        assert_eq!(final_data[0].name, "Inserted");
        assert_eq!(final_data[0].updated_at_hlc().unwrap(), hlc_i);

        assert_eq!(final_data[1].sync_id, "sync_u");
        assert_eq!(final_data[1].name, "Updated");
        assert_eq!(final_data[1].value, Some(11));
        assert_eq!(final_data[1].updated_at_hlc().unwrap(), hlc_u);

        Ok(())
    }

    #[tokio::test]
    async fn test_apply_local_changes_rollback() -> Result<()> {
        // ... (existing test code)
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?;
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let remote_source = MockRemoteDataSource::new(remote_node_id);

        let hlc_initial = hlc(BASE_TS, 0, LOCAL_NODE_STR);
        let hlc_update_try = hlc(BASE_TS + 1, 0, LOCAL_NODE_STR);
        let hlc_insert_fail = hlc(BASE_TS + 2, 0, LOCAL_NODE_STR);

        // Insert initial record
        let initial = insert_test_record(
            &db,
            "sync_dup",
            "Initial",
            Some(0),
            &hlc_initial,
            &hlc_initial,
        )
        .await?;

        let ops = vec![
            SyncOperation::UpdateLocal(Model {
                id: initial.id,
                sync_id: "sync_dup".to_string(),
                name: "UpdateTry".to_string(), // This update should be rolled back
                value: Some(1),
                created_at_hlc_ts: initial.created_at_hlc_ts.clone(),
                created_at_hlc_ct: initial.created_at_hlc_ct,
                created_at_hlc_id: initial.created_at_hlc_id,
                updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_update_try.timestamp)?,
                updated_at_hlc_ct: hlc_update_try.version as i32,
                updated_at_hlc_id: hlc_update_try.node_id,
            }),
            SyncOperation::InsertLocal(Model {
                // This insert will fail (duplicate unique sync_id)
                id: 0,                           // Placeholder
                sync_id: "sync_dup".to_string(), // Duplicate unique key
                name: "InsertedFail".to_string(),
                value: Some(10),
                created_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_insert_fail.timestamp)?,
                created_at_hlc_ct: hlc_insert_fail.version as i32,
                created_at_hlc_id: hlc_insert_fail.node_id,
                updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_insert_fail.timestamp)?,
                updated_at_hlc_ct: hlc_insert_fail.version as i32,
                updated_at_hlc_id: hlc_insert_fail.node_id,
            }),
        ];

        let hlc_context = SyncTaskContext::new(local_node_id);
        let options = ChunkingOptions::default(local_node_id);
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options,
            sync_direction: SyncDirection::Bidirectional,
            hlc_context: &hlc_context,
        };

        let result = apply_local_changes::<Entity>(&context, ops).await;
        assert!(
            result.is_err(),
            "Expected transaction to fail due to unique constraint violation"
        );
        eprintln!("Rollback Error: {:?}", result.err().unwrap()); // Log error for debugging

        // Verify DB state after expected rollback
        let final_data = Entity::find().all(&db).await?;
        assert_eq!(final_data.len(), 1); // Only the initial record should exist
        assert_eq!(final_data[0].sync_id, "sync_dup");
        assert_eq!(final_data[0].name, "Initial"); // Name should NOT be "UpdateTry"
        assert_eq!(final_data[0].updated_at_hlc().unwrap(), hlc_initial); // HLC should be the initial one

        Ok(())
    }

    #[tokio::test]
    async fn test_synchronize_table_chunk_hash_mismatch_fetch() -> Result<()> {
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?;
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let remote_source = MockRemoteDataSource::new(remote_node_id);

        let start_hlc = hlc(BASE_TS, 0, LOCAL_NODE_STR);
        let common_hlc = hlc(BASE_TS + 100, 0, LOCAL_NODE_STR); // Define the HLC to be used by both

        insert_test_record(
            &db,
            "fetch_rec", // Unique ID for the record
            "LocalData", // Local version's data
            Some(1),
            &common_hlc, // Use common_hlc for creation
            &common_hlc, // Use common_hlc for update
        )
        .await?;

        // Define the remote record with the same sync_id and HLC, but different data
        let remote_record = Model {
            id: 995,                          // Mock PK
            sync_id: "fetch_rec".to_string(), // Same unique ID
            name: "RemoteData".to_string(),   // Different data -> different hash
            value: Some(2),
            // Use the same HLC components as the local record
            created_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(common_hlc.timestamp)?,
            created_at_hlc_ct: common_hlc.version as i32,
            created_at_hlc_id: common_hlc.node_id,
            updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(common_hlc.timestamp)?,
            updated_at_hlc_ct: common_hlc.version as i32,
            updated_at_hlc_id: common_hlc.node_id,
        };
        remote_source
            .set_remote_data(vec![remote_record.clone()])
            .await;

        // Setup chunks (count=1, below threshold)
        let options = ChunkingOptions {
            min_size: 1,
            max_size: 1, // Ensure single record chunk
            alpha: 0.0,
            node_id: local_node_id,
        };
        let local_chunks =
            generate_data_chunks::<Entity>(&db, &options, Some(start_hlc.clone())).await?;
        let remote_chunk = DataChunk {
            start_hlc: common_hlc.clone(), // Chunk covers the record's HLC
            end_hlc: common_hlc.clone(),
            count: 1,
            chunk_hash: calculate_chunk_hash(&[remote_record.clone()])?, // Hash based on remote data
        };
        remote_source
            .set_remote_chunks(vec![remote_chunk.clone()])
            .await;

        // Assertions on setup
        assert_eq!(local_chunks.len(), 1, "Should generate one local chunk");
        assert_eq!(local_chunks[0].count, 1);
        assert!(local_chunks[0].count <= COMPARISON_THRESHOLD);
        assert_eq!(
            local_chunks[0].start_hlc, common_hlc,
            "Local chunk start should match record HLC"
        );
        assert_eq!(
            remote_chunk.start_hlc, common_hlc,
            "Remote chunk start should match record HLC"
        );
        assert_eq!(
            local_chunks[0].start_hlc, remote_chunk.start_hlc,
            "Chunks should align"
        ); // Verify alignment
        assert_eq!(local_chunks[0].end_hlc, remote_chunk.end_hlc);
        assert_ne!(
            local_chunks[0].chunk_hash, remote_chunk.chunk_hash,
            "Chunk hashes must differ"
        ); // Verify hash mismatch

        let hlc_context = SyncTaskContext::new(local_node_id);
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options,
            sync_direction: SyncDirection::Bidirectional,
            hlc_context: &hlc_context,
        };
        let initial_metadata = SyncTableMetadata {
            table_name: "test_items".to_string(),
            last_sync_hlc: start_hlc.clone(),
        };

        let final_metadata =
            synchronize_table::<Entity, _>(&context, "test_items", &initial_metadata).await?;

        // ... rest of the assertions from the original test ...
        let get_records_calls = remote_source.get_records_call_ranges().await;
        assert_eq!(
            get_records_calls.len(),
            1,
            "Should have called get_remote_records_in_hlc_range once"
        );
        assert_eq!(get_records_calls[0].0, common_hlc);
        assert_eq!(get_records_calls[0].1, common_hlc);

        let applied_ops = remote_source.get_applied_ops().await;
        assert_eq!(applied_ops.len(), 1);
        match &applied_ops[0] {
            SyncOperation::UpdateRemote(model) => {
                assert_eq!(model.sync_id, "fetch_rec");
                assert_eq!(model.name, "LocalData");
            }
            op => panic!("Expected UpdateRemote, got {:?}", op),
        }

        let local_data = Entity::find().all(context.db).await?;
        assert_eq!(local_data[0].name, "LocalData");

        assert_eq!(final_metadata.last_sync_hlc, common_hlc);

        Ok(())
    }

    #[tokio::test]
    async fn test_synchronize_table_chunk_hash_mismatch_breakdown() -> Result<()> {
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?;
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let remote_source = MockRemoteDataSource::new(remote_node_id);

        let start_hlc = hlc(BASE_TS, 0, LOCAL_NODE_STR);
        let mut current_hlc = hlc(BASE_TS + 100, 0, LOCAL_NODE_STR); // HLC *before* first record

        // Create > COMPARISON_THRESHOLD records locally and remotely with differing data
        let record_count = COMPARISON_THRESHOLD + 5;
        let mut local_records = Vec::new();
        let mut remote_records = Vec::new();
        let mut first_record_hlc = None;

        for i in 0..record_count {
            let sync_id = format!("break_rec_{}", i);
            current_hlc.increment(); // Increment *before* use for the current record

            if i == 0 {
                first_record_hlc = Some(current_hlc.clone());
            }

            // Local record
            let local = insert_test_record(
                &db,
                &sync_id,
                &format!("Local_{}", i),
                Some(i as i32),
                &current_hlc, // Use the incremented HLC
                &current_hlc, // Use the incremented HLC
            )
            .await?;
            local_records.push(local.clone());

            // Remote record (different data, same HLC for test simplicity)
            let remote = Model {
                id: 1000 + i as i32, // Mock PK
                sync_id: sync_id.clone(),
                name: format!("Remote_{}", i),
                value: Some(i as i32 * 10),
                created_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(current_hlc.timestamp)?,
                created_at_hlc_ct: current_hlc.version as i32,
                created_at_hlc_id: current_hlc.node_id,
                updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(current_hlc.timestamp)?,
                updated_at_hlc_ct: current_hlc.version as i32,
                updated_at_hlc_id: current_hlc.node_id,
            };
            remote_records.push(remote.clone());
        }
        let chunk_hlc_end = current_hlc.clone(); // HLC of the last record
        let chunk_hlc_start = first_record_hlc.expect("Should have inserted at least one record"); // <--- Use the actual first HLC

        remote_source.set_remote_data(remote_records.clone()).await;

        // Setup chunks (count > threshold)
        let options = ChunkingOptions {
            min_size: record_count, // Ensure one chunk initially
            max_size: record_count * 2,
            alpha: 0.0,
            node_id: local_node_id,
        };
        let local_chunks =
            generate_data_chunks::<Entity>(&db, &options, Some(start_hlc.clone())).await?;
        let remote_chunk = DataChunk {
            start_hlc: chunk_hlc_start.clone(), // <--- Use correct start HLC
            end_hlc: chunk_hlc_end.clone(),
            count: record_count,
            chunk_hash: calculate_chunk_hash(&remote_records)?, // Different hash
        };
        remote_source
            .set_remote_chunks(vec![remote_chunk.clone()])
            .await;

        // Assertions
        assert_eq!(local_chunks.len(), 1);
        assert_eq!(local_chunks[0].count, record_count);
        assert!(local_chunks[0].count > COMPARISON_THRESHOLD);

        // Verify chunk alignment and hash difference
        assert_eq!(
            local_chunks[0].start_hlc,
            chunk_hlc_start, // Compare local start to the actual first record HLC
            "Local chunk start HLC ({:?}) should match the HLC of the first record ({:?})",
            local_chunks[0].start_hlc,
            chunk_hlc_start
        );
        assert_eq!(
            local_chunks[0].start_hlc,
            remote_chunk.start_hlc, // Compare local and remote start
            "Ensure local ({:?}) and remote ({:?}) chunk start HLCs align",
            local_chunks[0].start_hlc,
            remote_chunk.start_hlc
        );
        assert_eq!(local_chunks[0].end_hlc, remote_chunk.end_hlc);
        assert_ne!(local_chunks[0].chunk_hash, remote_chunk.chunk_hash);

        let hlc_context = SyncTaskContext::new(local_node_id);
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options,
            sync_direction: SyncDirection::Bidirectional,
            hlc_context: &hlc_context,
        };
        let initial_metadata = SyncTableMetadata {
            table_name: "test_items".to_string(),
            last_sync_hlc: start_hlc.clone(),
        };

        let final_metadata =
            synchronize_table::<Entity, _>(&context, "test_items", &initial_metadata).await?;

        // Assertions
        let sub_chunk_requests = remote_source.get_sub_chunk_requests().await;
        assert_eq!(
            sub_chunk_requests.len(),
            1,
            "Should have requested sub-chunks once"
        );
        assert_eq!(
            sub_chunk_requests[0].0, local_chunks[0],
            "Sub-chunk request should be for the mismatched local chunk"
        ); // Parent chunk matches local
        assert_eq!(
            sub_chunk_requests[0].1, COMPARISON_THRESHOLD,
            "Sub-chunk size should match threshold"
        ); // Target size is threshold

        // Since sub-chunks will also likely mismatch, record fetching will happen eventually
        let get_records_calls = remote_source.get_records_call_ranges().await;
        assert!(
            !get_records_calls.is_empty(),
            "Should have eventually fetched records for sub-chunks"
        );

        // Conflict resolution (tie-break, local wins)
        let applied_ops = remote_source.get_applied_ops().await;
        assert_eq!(
            applied_ops.len(),
            record_count as usize,
            "Should have one update op per record"
        );
        for i in 0..record_count {
            match &applied_ops[i as usize] {
                SyncOperation::UpdateRemote(model) => {
                    assert!(model.sync_id.starts_with("break_rec_"));
                    assert!(model.name.starts_with("Local_")); // Local data pushed
                }
                op => panic!("Expected UpdateRemote, got {:?}", op),
            }
        }

        assert_eq!(final_metadata.last_sync_hlc, chunk_hlc_end);

        Ok(())
    }

    #[tokio::test]
    async fn test_synchronize_table_misaligned_chunks() -> Result<()> {
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?;
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let remote_source = MockRemoteDataSource::new(remote_node_id);

        let start_hlc = hlc(BASE_TS, 0, LOCAL_NODE_STR);
        let hlc1 = hlc(BASE_TS + 100, 0, LOCAL_NODE_STR);
        let hlc2 = hlc(BASE_TS + 200, 0, LOCAL_NODE_STR);
        let hlc3 = hlc(BASE_TS + 300, 0, REMOTE_NODE_STR);
        let hlc4 = hlc(BASE_TS + 400, 0, REMOTE_NODE_STR);

        // Local: Record L1@hlc1, L2@hlc2 -> Chunk [hlc1-hlc2]
        insert_test_record(&db, "mis_l1", "L1", Some(1), &hlc1, &hlc1).await?;
        insert_test_record(&db, "mis_l2", "L2", Some(2), &hlc2, &hlc2).await?;

        // Remote: Record R1@hlc3, R2@hlc4 -> Chunk [hlc3-hlc4]
        let r1 = Model {
            id: 994,
            sync_id: "mis_r1".to_string(),
            name: "R1".to_string(),
            value: Some(3),
            created_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc3.timestamp)?,
            created_at_hlc_ct: hlc3.version as i32,
            created_at_hlc_id: hlc3.node_id,
            updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc3.timestamp)?,
            updated_at_hlc_ct: hlc3.version as i32,
            updated_at_hlc_id: hlc3.node_id,
        };
        let r2 = Model {
            id: 993,
            sync_id: "mis_r2".to_string(),
            name: "R2".to_string(),
            value: Some(4),
            created_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc4.timestamp)?,
            created_at_hlc_ct: hlc4.version as i32,
            created_at_hlc_id: hlc4.node_id,
            updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc4.timestamp)?,
            updated_at_hlc_ct: hlc4.version as i32,
            updated_at_hlc_id: hlc4.node_id,
        };
        remote_source
            .set_remote_data(vec![r1.clone(), r2.clone()])
            .await;

        // Setup chunks
        let options = ChunkingOptions {
            min_size: 2,
            max_size: 10,
            alpha: 0.0,
            node_id: local_node_id,
        }; // Allow slightly larger chunks
        let local_chunks =
            generate_data_chunks::<Entity>(&db, &options, Some(start_hlc.clone())).await?;
        let remote_chunk = DataChunk {
            start_hlc: hlc3.clone(), // Starts later than local chunk
            end_hlc: hlc4.clone(),
            count: 2,
            chunk_hash: calculate_chunk_hash(&[r1.clone(), r2.clone()])?,
        };
        remote_source
            .set_remote_chunks(vec![remote_chunk.clone()])
            .await;

        assert_eq!(local_chunks.len(), 1);
        assert_eq!(local_chunks[0].start_hlc, hlc1);
        assert_eq!(local_chunks[0].end_hlc, hlc2);
        assert!(local_chunks[0].start_hlc < remote_chunk.start_hlc); // Verify misalignment

        let hlc_context = SyncTaskContext::new(local_node_id);
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options,
            sync_direction: SyncDirection::Bidirectional,
            hlc_context: &hlc_context,
        };
        let initial_metadata = SyncTableMetadata {
            table_name: "test_items".to_string(),
            last_sync_hlc: start_hlc.clone(),
        };

        let final_metadata =
            synchronize_table::<Entity, _>(&context, "test_items", &initial_metadata).await?;

        // Assertions
        // align_and_queue_chunks should queue FetchRange for the misaligned parts
        let get_records_calls = remote_source.get_records_call_ranges().await;
        // Expect FetchRange for local chunk's range [hlc1-hlc2] and remote chunk's range [hlc3-hlc4]
        assert!(
            get_records_calls
                .iter()
                .any(|(s, e)| s == &hlc1 && e == &hlc2),
            "Should have fetched for local range"
        );
        assert!(
            get_records_calls
                .iter()
                .any(|(s, e)| s == &hlc3 && e == &hlc4),
            "Should have fetched for remote range"
        );
        // Depending on queue order, might be 2 calls or potentially merged ranges if logic changes. Check for minimum expected calls.
        assert!(
            get_records_calls.len() >= 2,
            "Expected at least two record fetch calls due to misalignment"
        );

        // Check final state: Local inserts remotely, remote inserts locally
        let applied_ops = remote_source.get_applied_ops().await;
        let inserted_remotely: Vec<_> = applied_ops
            .iter()
            .filter_map(|op| match op {
                SyncOperation::InsertRemote(m) => Some(m.sync_id.clone()),
                _ => None,
            })
            .collect();
        assert!(inserted_remotely.contains(&"mis_l1".to_string()));
        assert!(inserted_remotely.contains(&"mis_l2".to_string()));
        assert_eq!(inserted_remotely.len(), 2);

        let local_data = Entity::find()
            .order_by_asc(Column::SyncId)
            .all(context.db)
            .await?;
        assert_eq!(local_data.len(), 4); // L1, L2, R1, R2
        assert_eq!(local_data[0].sync_id, "mis_l1");
        assert_eq!(local_data[1].sync_id, "mis_l2");
        assert_eq!(local_data[2].sync_id, "mis_r1");
        assert_eq!(local_data[3].sync_id, "mis_r2");

        assert_eq!(final_metadata.last_sync_hlc, hlc4); // Max HLC encountered

        Ok(())
    }

    #[tokio::test]
    async fn test_synchronize_table_pull_only() -> Result<()> {
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?;
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let remote_source = MockRemoteDataSource::new(remote_node_id);

        let start_hlc = hlc(BASE_TS, 0, LOCAL_NODE_STR);
        let hlc_local_old = hlc(BASE_TS + 100, 0, LOCAL_NODE_STR);
        let hlc_remote_insert = hlc(BASE_TS + 150, 0, REMOTE_NODE_STR);
        let hlc_remote_update = hlc(BASE_TS + 200, 0, REMOTE_NODE_STR); // Remote wins update HLC

        // Local has one old record
        let _l_old = insert_test_record(
            &db,
            "pull_rec",
            "LocalOld",
            Some(1),
            &hlc_local_old,
            &hlc_local_old,
        )
        .await?;

        // Remote has a new record and an update for the existing one
        let r_new = Model {
            id: 992,
            sync_id: "pull_new".to_string(),
            name: "RemoteNew".to_string(),
            value: Some(2),
            created_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_remote_insert.timestamp)?,
            created_at_hlc_ct: hlc_remote_insert.version as i32,
            created_at_hlc_id: hlc_remote_insert.node_id,
            updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_remote_insert.timestamp)?,
            updated_at_hlc_ct: hlc_remote_insert.version as i32,
            updated_at_hlc_id: hlc_remote_insert.node_id,
        };
        let r_update = Model {
            id: 991,
            sync_id: "pull_rec".to_string(),
            name: "RemoteUpdateWins".to_string(),
            value: Some(3),
            created_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_local_old.timestamp)?,
            created_at_hlc_ct: hlc_local_old.version as i32,
            created_at_hlc_id: hlc_local_old.node_id,
            updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_remote_update.timestamp)?,
            updated_at_hlc_ct: hlc_remote_update.version as i32,
            updated_at_hlc_id: hlc_remote_update.node_id,
        };
        remote_source
            .set_remote_data(vec![r_new.clone(), r_update.clone()])
            .await;

        // Setup remote chunks
        let options = ChunkingOptions {
            min_size: 1,
            max_size: 10,
            alpha: 0.0,
            node_id: local_node_id,
        };
        let remote_chunk1 = DataChunk {
            start_hlc: hlc_remote_insert.clone(),
            end_hlc: hlc_remote_insert.clone(),
            count: 1,
            chunk_hash: calculate_chunk_hash(&[r_new.clone()])?,
        };
        let remote_chunk2 = DataChunk {
            start_hlc: hlc_remote_update.clone(),
            end_hlc: hlc_remote_update.clone(),
            count: 1,
            chunk_hash: calculate_chunk_hash(&[r_update.clone()])?,
        };
        remote_source
            .set_remote_chunks(vec![remote_chunk1, remote_chunk2])
            .await;

        let hlc_context = SyncTaskContext::new(local_node_id);
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options,
            sync_direction: SyncDirection::Pull,
            hlc_context: &hlc_context,
        }; // PULL direction
        let initial_metadata = SyncTableMetadata {
            table_name: "test_items".to_string(),
            last_sync_hlc: start_hlc.clone(),
        };

        let final_metadata =
            synchronize_table::<Entity, _>(&context, "test_items", &initial_metadata).await?;

        // Assertions
        let applied_ops = remote_source.get_applied_ops().await;
        assert!(
            applied_ops.is_empty()
                || applied_ops
                    .iter()
                    .all(|op| matches!(op, SyncOperation::NoOp(_))),
            "No operations should have been sent to remote in Pull mode"
        );

        let local_data = Entity::find()
            .order_by_asc(Column::SyncId)
            .all(context.db)
            .await?;
        assert_eq!(local_data.len(), 2);

        assert_eq!(local_data[0].sync_id, "pull_new"); // New record inserted locally
        assert_eq!(local_data[0].name, "RemoteNew");
        assert_eq!(local_data[0].updated_at_hlc().unwrap(), hlc_remote_insert);

        assert_eq!(local_data[1].sync_id, "pull_rec");
        assert_eq!(local_data[1].name, "RemoteUpdateWins"); // Existing record updated locally
        assert_eq!(local_data[1].updated_at_hlc().unwrap(), hlc_remote_update);

        assert_eq!(final_metadata.last_sync_hlc, hlc_remote_update); // Max HLC encountered

        Ok(())
    }

    #[tokio::test]
    async fn test_synchronize_table_push_only() -> Result<()> {
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?;
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let remote_source = MockRemoteDataSource::new(remote_node_id);

        let start_hlc = hlc(BASE_TS, 0, LOCAL_NODE_STR);
        let hlc_remote_old = hlc(BASE_TS + 100, 0, REMOTE_NODE_STR);
        let hlc_local_insert = hlc(BASE_TS + 150, 0, LOCAL_NODE_STR);
        let hlc_local_update = hlc(BASE_TS + 200, 0, LOCAL_NODE_STR); // Local wins update HLC

        // Local has a new record and an update for the existing one
        let _l_new = insert_test_record(
            &db,
            "push_new",
            "LocalNew",
            Some(1),
            &hlc_local_insert,
            &hlc_local_insert,
        )
        .await?;
        insert_test_record(
            &db,
            "push_rec",
            "LocalUpdateWins",
            Some(2),
            &hlc_remote_old,
            &hlc_local_update,
        )
        .await?; // Created with remote HLC, updated locally

        // Remote has one old record
        let r_old = Model {
            id: 990,
            sync_id: "push_rec".to_string(),
            name: "RemoteOld".to_string(),
            value: Some(99),
            created_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_remote_old.timestamp)?,
            created_at_hlc_ct: hlc_remote_old.version as i32,
            created_at_hlc_id: hlc_remote_old.node_id,
            updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(hlc_remote_old.timestamp)?,
            updated_at_hlc_ct: hlc_remote_old.version as i32,
            updated_at_hlc_id: hlc_remote_old.node_id,
        };
        remote_source.set_remote_data(vec![r_old.clone()]).await;

        // Setup remote chunks
        let options = ChunkingOptions {
            min_size: 1,
            max_size: 10,
            alpha: 0.0,
            node_id: local_node_id,
        };
        let remote_chunk = DataChunk {
            start_hlc: hlc_remote_old.clone(),
            end_hlc: hlc_remote_old.clone(),
            count: 1,
            chunk_hash: calculate_chunk_hash(&[r_old.clone()])?,
        };
        remote_source.set_remote_chunks(vec![remote_chunk]).await;

        let hlc_context = SyncTaskContext::new(local_node_id);
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options,
            sync_direction: SyncDirection::Push,
            hlc_context: &hlc_context,
        }; // PUSH direction
        let initial_metadata = SyncTableMetadata {
            table_name: "test_items".to_string(),
            last_sync_hlc: start_hlc.clone(),
        };

        let final_metadata =
            synchronize_table::<Entity, _>(&context, "test_items", &initial_metadata).await?;

        // Assertions
        let applied_ops = remote_source.get_applied_ops().await;
        assert_eq!(applied_ops.len(), 2); // InsertNew + UpdateExisting

        let mut ops_map = HashMap::new();
        for op in applied_ops {
            match op {
                SyncOperation::InsertRemote(m) => {
                    ops_map.insert("insert", m);
                }
                SyncOperation::UpdateRemote(m) => {
                    ops_map.insert("update", m);
                }
                _ => {}
            }
        }

        assert!(ops_map.contains_key("insert"));
        assert_eq!(ops_map["insert"].sync_id, "push_new");
        assert_eq!(ops_map["insert"].name, "LocalNew");

        assert!(ops_map.contains_key("update"));
        assert_eq!(ops_map["update"].sync_id, "push_rec");
        assert_eq!(ops_map["update"].name, "LocalUpdateWins");
        assert_eq!(
            ops_map["update"].updated_at_hlc().unwrap(),
            hlc_local_update
        );

        // Verify local DB state is unchanged by Pull/Bi operations
        let local_data = Entity::find()
            .order_by_asc(Column::SyncId)
            .all(context.db)
            .await?;
        assert_eq!(local_data.len(), 2);
        assert_eq!(local_data[0].sync_id, "push_new"); // Local state remains as initially set up
        assert_eq!(local_data[1].sync_id, "push_rec");
        assert_eq!(local_data[1].name, "LocalUpdateWins");

        assert_eq!(final_metadata.last_sync_hlc, hlc_local_update); // Max HLC encountered

        Ok(())
    }

    #[tokio::test]
    async fn test_error_getting_remote_chunks() -> Result<()> {
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?;
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let mut remote_source = MockRemoteDataSource::new(remote_node_id);
        remote_source.fail_on_get_chunks = true; // Simulate failure

        let start_hlc = hlc(BASE_TS, 0, LOCAL_NODE_STR);
        let hlc_context = SyncTaskContext::new(local_node_id);
        let options = ChunkingOptions::default(local_node_id);
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options,
            sync_direction: SyncDirection::Bidirectional,
            hlc_context: &hlc_context,
        };
        let initial_metadata = SyncTableMetadata {
            table_name: "test_items".to_string(),
            last_sync_hlc: start_hlc.clone(),
        };

        let result =
            synchronize_table::<Entity, _>(&context, "test_items", &initial_metadata).await;

        assert!(result.is_err());
        let error = result.err().unwrap(); // Get the anyhow::Error
        let error_string = error.to_string();
        eprintln!("Actual error string (get_remote_chunks): {}", error_string);

        assert!(error_string.contains("Failed to fetch remote chunks for table 'test_items'"));
        assert!(error
            .root_cause()
            .to_string()
            .contains("Simulated failure getting remote chunks"));

        Ok(())
    }

    #[tokio::test]
    async fn test_error_getting_remote_records() -> Result<()> {
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?;
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let mut remote_source = MockRemoteDataSource::new(remote_node_id);
        remote_source.fail_on_get_records = true; // Simulate failure

        let start_hlc = hlc(BASE_TS, 0, LOCAL_NODE_STR);
        let data_hlc = hlc(BASE_TS + 100, 0, LOCAL_NODE_STR);

        // Setup scenario requiring record fetch (hash mismatch, count=1)
        let local_record_model =
            insert_test_record(&db, "fail_rec", "Local", Some(1), &data_hlc, &data_hlc).await?;
        let remote_record = Model {
            id: 989,
            sync_id: "fail_rec".to_string(),
            name: "Remote".to_string(),
            value: Some(2),
            created_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(data_hlc.timestamp)?,
            created_at_hlc_ct: data_hlc.version as i32,
            created_at_hlc_id: data_hlc.node_id,
            updated_at_hlc_ts: hlc_timestamp_millis_to_rfc3339(data_hlc.timestamp)?,
            updated_at_hlc_ct: data_hlc.version as i32,
            updated_at_hlc_id: data_hlc.node_id,
        };
        remote_source
            .set_remote_data(vec![remote_record.clone()])
            .await;

        let options = ChunkingOptions {
            min_size: 1,
            max_size: 1,
            alpha: 0.0,
            node_id: local_node_id,
        };
        // Generate local chunk based on the inserted record
        let local_chunks =
            generate_data_chunks::<Entity>(&db, &options, Some(start_hlc.clone())).await?;
        let remote_chunk = DataChunk {
            start_hlc: data_hlc.clone(),
            end_hlc: data_hlc.clone(),
            count: 1,
            chunk_hash: calculate_chunk_hash(&[remote_record])?,
        };
        remote_source
            .set_remote_chunks(vec![remote_chunk.clone()])
            .await;

        // Ensure chunk hashes differ (critical for triggering fetch)
        let local_hash = calculate_chunk_hash(&[local_record_model])?;
        assert_ne!(
            local_hash, remote_chunk.chunk_hash,
            "Chunk hashes must differ to trigger record fetch"
        );
        // Ensure chunks align correctly for the hash mismatch path
        assert_eq!(local_chunks[0].start_hlc, remote_chunk.start_hlc);
        assert_eq!(local_chunks[0].end_hlc, remote_chunk.end_hlc);

        let hlc_context = SyncTaskContext::new(local_node_id);
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options,
            sync_direction: SyncDirection::Bidirectional,
            hlc_context: &hlc_context,
        };
        let initial_metadata = SyncTableMetadata {
            table_name: "test_items".to_string(),
            last_sync_hlc: start_hlc.clone(),
        };

        let result =
            synchronize_table::<Entity, _>(&context, "test_items", &initial_metadata).await;

        assert!(result.is_err());
        let error = result.err().unwrap();
        let error_string = error.to_string();
        eprintln!("Actual error string (get_remote_records): {}", error_string);
        assert!(error_string.contains("Failed to fetch remote records for range"));
        assert!(error
            .root_cause()
            .to_string()
            .contains("Simulated failure getting remote records"));

        Ok(())
    }

    #[tokio::test]
    async fn test_error_applying_remote_changes() -> Result<()> {
        let db = setup_db().await?;
        let local_node_id = Uuid::parse_str(LOCAL_NODE_STR)?;
        let remote_node_id = Uuid::parse_str(REMOTE_NODE_STR)?;
        let mut remote_source = MockRemoteDataSource::new(remote_node_id);
        remote_source.fail_on_apply = true; // Simulate failure

        let start_hlc = hlc(BASE_TS, 0, LOCAL_NODE_STR);
        let insert_hlc = hlc(BASE_TS + 100, 0, LOCAL_NODE_STR);

        // Setup scenario requiring remote apply (local insert, bidirectional)
        insert_test_record(
            &db,
            "fail_apply",
            "LocalNew",
            Some(1),
            &insert_hlc,
            &insert_hlc,
        )
        .await?;
        // Remote starts empty
        remote_source.set_remote_data(vec![]).await;
        remote_source.set_remote_chunks(vec![]).await;

        let hlc_context = SyncTaskContext::new(local_node_id);
        let options = ChunkingOptions {
            min_size: 1,
            max_size: 1,
            alpha: 0.0,
            node_id: local_node_id,
        };
        let context = SyncContext {
            db: &db,
            local_node_id,
            remote_source: &remote_source,
            chunking_options: options,
            sync_direction: SyncDirection::Bidirectional,
            hlc_context: &hlc_context,
        };
        let initial_metadata = SyncTableMetadata {
            table_name: "test_items".to_string(),
            last_sync_hlc: start_hlc.clone(),
        };

        let result =
            synchronize_table::<Entity, _>(&context, "test_items", &initial_metadata).await;

        assert!(result.is_err());
        let error = result.err().unwrap();
        let error_string = error.to_string();
        eprintln!(
            "Actual error string (apply_remote_changes): {}",
            error_string
        );
        assert!(
            error_string.contains("Sync failed for table 'test_items' during changes application") // Check context
        );
        assert!(error
            .root_cause()
            .to_string()
            .contains("Simulated remote apply failure")); // Check root cause

        Ok(())
    }
}
