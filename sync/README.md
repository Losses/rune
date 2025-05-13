# Sync: Simplified Data Synchronization

`sync` is a Rust library designed for efficient and understandable data synchronization between devices, specifically tailored for the Rune player ecosystem. Unlike complex traditional synchronization protocols, `sync` prioritizes simplicity and clarity by focusing on core timestamping and conflict resolution mechanisms, intentionally omitting certain data intricacies.

The core of the system relies on Hybrid Logical Clocks (HLC) for ordering events across distributed nodes and an adaptive chunking mechanism for efficient data comparison.

## Core Concepts & Design

This library implements a synchronization system based on the following principles:

### 1. Clock System (Based on HLC)

The foundation of ordering and conflict resolution is the Hybrid Logical Clock (HLC).

- **Clock Type**: Hybrid Logical Clock (HLC), combining the strengths of physical and logical clocks.
- **HLC Structure**: Each HLC timestamp consists of:
  - `Physical Time Component`: Unix millisecond timestamp (UTC).
  - `Logical Counter`: A 32-bit unsigned integer to differentiate multiple operations within the same millisecond.
  - `Node ID`: A 16-byte unique identifier (UUID v4) ensuring global uniqueness across nodes.
- **Physical Clock Calibration**:
  - Uses Cristian's Algorithm for calibration against a master reference node.
  - Requires multiple samples (at least 5) to calculate clock offset, using the median value.
  - Each node maintains only its offset relative to the master node.
  - **Master Node Failure**: Synchronization pauses if the master node becomes unavailable, awaiting recovery or manual intervention.
  - **Offset Validity**: An offset threshold (500ms) is enforced. If a newly calculated offset differs from the current one by more than this threshold, an emergency re-calibration (10 samples with consistency check) is triggered.
- **HLC Construction & Updates**:
  - **Sending**: When sending data/messages, a node constructs an HLC based on its current physical time and logical counter, appending its `Node ID`.
  - **Receiving**: When receiving data/messages, the recipient applies its calibrated clock offset to the physical time component of the incoming HLC before comparing or updating its local HLC, ensuring consistent time ordering.
- **Clock Skew Protection**:
  - **Backward Jumps**: If the current physical time is detected to be earlier than the previously recorded physical time:
    - If the difference > 1 second, an error is reported to the client, requiring manual intervention to fix the system clock.
    - If the difference ≤ 1 second, the local time is gradually adjusted forward (max 100ms per adjustment) to catch up.
  - **Timezone**: The physical time component of HLC always uses UTC to prevent timezone-related inconsistencies.

### 2. Data Model

The library assumes a simple data model for synchronized entities:

- **Required Data Attributes**:
  - `entity_key`: A unique identifier for the data record within the business logic scope (defined by the application's table structure).
  - `modified_hlc`: The HLC timestamp when the record was last modified.
  - `created_hlc`: The HLC timestamp when the record was created. This value is set only on insertion and **must not** be changed afterward.
- **Per-Table Node Storage**: Each node needs to store the following metadata _for each table_ being synchronized:
  - `last_sync_time`: The HLC timestamp indicating the point up to which synchronization was last successfully completed.
  - `last_sync_id`: The maximum record ID (or similar marker) processed during the last sync (can optimize fetching new data).
  - `node_id`: The unique UUID v4 identifier for the current node.

### 3. Chunking Algorithm

To efficiently compare large datasets, data is divided into chunks using an adaptive algorithm.

- **Principles**:
  - Define a minimum (`min_size`) and maximum (`max_size`) chunk size.
  - Use smaller chunks for recent data (more likely to change) and larger chunks for older data, up to `max_size`.
- **Algorithm**: Exponential Decay Formula:
  ```
  window_size = min_size * (1 + α)^ceil(age_factor)
  ```
  - `α` (alpha) is a decay factor. Recommended presets based on data volatility (e.g., 0.3 for frequently changing data, 0.6 for stable data) balance efficiency and granularity.
  - `age_factor` relates to how old the data is (e.g., time since last modification).
  - Maximum chunk size is suggested to be around 10,000 records.
- **Hashing**: BLAKE3 algorithm is used to compute the hash of each individual data record and chunks.

### 4. Synchronization

The synchronization process involves several steps:

- **Pre-Sync Preparation**: Before exchanging data, nodes must exchange HLC information and perform clock calibration to establish a consistent time baseline.
- **Data Comparison & Conflict Resolution**:
  - **Conflict Types & Strategies**:
    - **Create-Create**: Both nodes created the same record (same `entity_key`). Keep the record with the higher `modified_hlc`. If HLCs are identical, keep the one from the node with the lexicographically smaller `Node ID`.
    - **Update-Update**: Both nodes updated the same record. Keep the version with the higher `modified_hlc`.
    - **Update-Delete**: One node updated, another deleted the record.
      - If Delete HLC ≥ Update HLC: Perform the delete.
      - If Delete HLC < Update HLC: Keep the update.
    - **Metadata Conflict**: (Potentially related to sync state or other metadata). Prioritize based on criteria like version vector coverage (if used), latest modification time, or node priority (if defined). _Note: The spec mentions this but details might depend on specific metadata being tracked._
- **Historical Data Synchronization**:
  - Compare chunk hashes.
  - If chunk hashes differ, recursively split the chunk (or compare records directly if small enough).
  - Compare individual records based primarily on their `modified_hlc` for total ordering. The spec mentions comparing `updated_at` if "version numbers" are identical, which might serve as a fallback under specific (potentially rare) HLC collision scenarios.
- **4. Optimization**:
  - **Per-Table Timestamps**: Maintain separate `last_sync_time` for each table to narrow the scope of data needing comparison in subsequent syncs.
  - **Transactional Writes**: All database writes resulting from a sync operation should occur within a single transaction. If the transaction fails, it's rolled back. If successful, the `last_sync_time` (and `last_sync_id`) are updated. This ensures atomicity and data consistency.

### 5. Error Recovery Mechanism

The system includes mechanisms to recover from interruptions during synchronization:

- **Sync Checkpoints**: After successfully processing each chunk (comparing and resolving conflicts), a checkpoint is recorded. This checkpoint includes the identifier of the processed chunk and should be persisted reliably (e.g., to disk or a database) and potentially include a hash for validation.
- **Interruption Recovery**: If synchronization is interrupted (e.g., network loss, crash), it can resume from the last successfully recorded checkpoint, avoiding redundant processing of already synchronized chunks.
