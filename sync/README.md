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

The synchronization process involves several steps, with the logic fundamentally pivoting on the `last_sync_time` of the two nodes. This approach avoids the need for a complex tombstone system for handling deletions.

- **Pre-Sync Preparation**: Before exchanging data, nodes must exchange HLC information and perform clock calibration to establish a consistent time baseline.
- **Two-Phase Data Reconciliation**: The core logic is split into two phases based on the `last_sync_time` for the table being synchronized. Comparison starts by comparing chunk hashes; if they differ, the system drills down to individual record comparison.

  - **Phase 1: Historical Data Reconciliation (Intersection Logic)**

    - **Scope**: Applies to all data with a `modified_hlc` _before_ the `last_sync_time`.
    - **Logic**: The nodes enforce consistency by taking the **intersection** of their datasets. For any given record in this time range, if it exists on one node but not the other, it is **deleted** from the node where it exists.
    - **Purpose**: This ensures that historical data is identical on both nodes, cleaning up any discrepancies from previous partial syncs or out-of-band modifications. This implicit deletion mechanism makes a tombstone system unnecessary.

  - **Phase 2: Recent Data Synchronization (Union Logic)**
    - **Scope**: Applies to all data with a `modified_hlc` _at or after_ the `last_sync_time`.
    - **Logic**: The nodes merge recent changes by taking the **union** of their datasets. If a record exists on one node but not the other, it is **inserted** on the node where it is missing.
    - **Conflict Resolution**: If a record (`entity_key`) exists on both nodes within this time range, a conflict occurs and is resolved as follows:
      - **Update-Update / Create-Create**: Both nodes have a version of the same record. The conflict is resolved by keeping the version with the higher `modified_hlc`. If the HLCs are identical, the record from the node with the lexicographically smaller `Node ID` is chosen to ensure deterministic resolution.

- **4.1. Optimization & Atomicity**
  - **Per-Table Timestamps**: Maintain a separate `last_sync_time` for each table to narrow the scope of data needing comparison in subsequent syncs.
  - **Transactional Writes**: All database writes (inserts, updates, deletes) resulting from a sync operation must occur within a single transaction. If the transaction fails, it is rolled back. If successful, the `last_sync_time` is updated. This ensures atomicity and data consistency.

### 5. Error Recovery Mechanism

The system includes mechanisms to recover from interruptions during synchronization:

- **Sync Checkpoints**: After successfully processing each chunk (comparing and resolving conflicts), a checkpoint is recorded. This checkpoint includes the identifier of the processed chunk and should be persisted reliably (e.g., to disk or a database) and potentially include a hash for validation.
- **Interruption Recovery**: If synchronization is interrupted (e.g., network loss, crash), it can resume from the last successfully recorded checkpoint, avoiding redundant processing of already synchronized chunks.
