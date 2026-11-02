# Sync ORM

This package provides generic SeaORM persistence and declarative source-row bindings for the neutral [Sync engine](../sync/README.md). Rune source-row ownership and application composition belong to [the integration specification](../documents/sync-integration.md).

## Declarative source-row bindings

The adapter supports caller-owned entity policies in addition to accepted-state persistence. A policy declares one source UUID, portable columns, local columns, UUID references, insertion defaults, scalar limits, identity validation, removal behavior and original-author restrictions. The caller registers its policies once. The registry generates the collection/tag/value dispatch required by `SyncModel` and supplies initialization traversal from that single registration list.

`RowPolicy<E, M>` is the shared SeaORM executor for an entity and its generated field mapping. Ordinary callers declare fields and rules; the executor supplies insert, update and delete operations. Portable values are decoded and validated before mutation; conversions reject overflow, invalid nulls and wrong scalar types. Existing source rows are found by their unique UUID. An update writes owned portable columns and declared local adjustments. An insertion uses explicitly declared local defaults or database defaults. A removal affects the source row and its declared owned rows. The caller supplies and commits the transaction used by `project_on`. The convenience `project` method opens and commits a standalone transaction.

Queries select the primary key, source UUID, portable columns and explicitly declared `local_reads`. Local binary bodies are fetched only through declared reads. Pure owner, derived-value and local-adjustment functions receive a `RowView<E>` with typed column access; reading an unselected column is an error. Local adjustments are restricted to declared local columns.

Policy declarations produce a typed portable value and its encoder/decoder. The registry generates model dispatch from those types. The declarative macro and typed column descriptors generate the model's dispatch expressions from the registry. Registry construction checks duplicate tags/fields, column ownership, reference targets and unsupported scalar types before a store is used.

Each primary source entity has one lifecycle owner; registration of a second policy for that entity fails. Derived tables can be shared through separate complete owner predicates. Typed entity columns and generated payloads are checked by Rust; registration, schema validation and value validation check uniqueness, column/default compatibility, reference targets, nullability and scalar bounds. An incoming field omission is distinct from an explicit null, and a required local column needs a declared insertion value or database default.

UUID references are stored on source rows, including when a parent is absent. Relationship reads join the reference UUID to the parent's unique UUID. They return an unresolved result when the parent is absent. UUID reference columns allow independent delivery; parent assignment or removal preserves every child row, and queries resolve references against current parents. Integer IDs can remain a caller API detail, resolved against the current parent at command/query boundaries. The child source row retains its UUID reference throughout that lifecycle.

`row_policies!` declares each collection once. A collection provides its tag, entity, generated payload/mapping names, UUID column, portable fields, local defaults and optional limits, references, owner and identity rules. `lists` registers typed `OwnedListPolicy` values as fields of that same payload; encode, initialization and receive all use them. `effects` registers a derived-row mapping and a pure function over the selected source row and optional payload. These functions receive only their declared value inputs. The shared executor performs the bounded list diff and derived-row replacement in the caller's transaction. External I/O and notifications run after commit.

For an existing `metrics` entity with a local primary key, unique UUID column and integer count, an ordinary declaration is:

```rust
sync_orm::row_policies! { pub Metrics, Collection, Payload {
    model: "metrics", schema: "1", collections: {
        Metric => {
            tag: "metrics", entity: metrics::Entity,
            value: Metric, mapping: MetricMap,
            policy: {
                uuid: metrics::Column::Uuid,
                fields: { count: i32 => metrics::Column::Count },
                defaults: {},
                limits: { count => (0, 1000) },
            }
        }
    }
} }
```

The [row-policy tests](tests/row_policy.rs) include complete entity definitions, registration validation and joined source projection. The declaration supplies encoding and ordinary row operations; domain commands still receive the caller's transaction and author the resulting source value explicitly.

An owned list declares its owner predicate, stable item key, portable fields, local defaults and maximum item count. Matched items preserve their local primary keys and local columns. An ordered list also declares its position field and uses contiguous nonnegative positions in payload order. Reordering reserves temporary negative positions inside the transaction before assigning final positions, so swapping items works with a unique owner/position index. The declared mutation bound includes this temporary work. A removed owner deletes only its bounded owned list; independent UUID relationships are unaffected.

`RegistryProjection` admits the sum of the registered read and mutation ceilings before modifying source rows. Source traversal reserves each complete source value's read ceiling, including owned items, and measures the encoded aggregate against the byte limit. Missing UUID assignments and traversal progress remain in the caller's transaction. Tests for these generated paths are in [row_policy.rs](tests/row_policy.rs).

Owner and identity validation is also part of model validation, so it runs on all received candidates before suppression and on transitions to absence. Merge belongs to the engine. The row policy projects its exact selected value, including a lower-revision survivor or absence; authorship belongs to explicit local commands.

Initialization uses stable source keyset cursors and explicit row/byte/mutation budgets. It assigns missing UUIDs transactionally, encodes through the same field declarations, and records completion with the final checked batch. Reopening uses the committed initialization result. The generic initialization state contains progress only.

Acceptance requires real SeaORM parent, child and owner-row entities unrelated to any application. Adding a fourth ordinary entity must require only its entity and registration. Exercise preservation of local columns, different local integer IDs, unresolved UUID references, parent removal/re-addition, owner rejection, rollback after source mutation, initialization restart, lower-winner projection and large parent fan-out. These tests execute the shared row-policy projection.

## Causal storage

The SQLite layout is installed on a fresh database by `causal_schema::install_on`; existing replica tables require an explicit import. UUIDs and digests are binary keys, and collection tags use their encoded bytes. `state_seq` tracks accepted state within a namespace; journal sequence and anchor track publication continuity. The journal maps these axes. Candidate lifetimes allow a snapshot to read the live set at its pinned state.

The schema has nine relations. Each entry lists its primary key, remaining columns, purpose, writers, readers and retention:

- `sync_meta`: key `namespace`; columns `model`, `schema`, `codec`, `replica`, `generation`, `head`, `head_anchor`, `state_seq`, `revision_counter`, `history_floor`, `floor_anchor`, `floor_state_seq`, `retained_bytes`, `initialized_at`. Replica creation, authoring, merge, initialization and compaction update identity, clocks, publication and floor metadata; replica, source and maintenance operations read it. It persists for the namespace.
- `sync_candidates`: key `(namespace, collection, writer, counter)`; columns `object`, `author`, `revision`, `body`, `digest`, `retention_seq`, `live_from`, `live_until`. Authoring and merge write candidate bodies and lifetimes; current-state and snapshot readers select candidates and projection uses the winners. Live rows remain; maintenance reclaims retired or never-live rows once state pins permit.
- `sync_journal`: key `(namespace, generation, sequence)`; columns `state_seq`, `collection`, `event_digest`, `event_manifest`, `anchor_after`. Authoring and merge publish events; incremental readers and compaction read them. Bounded pruning deletes entries at or below the history floor.
- `sync_checkpoints`: key `(namespace, source_replica, source_generation, scope_digest)`; columns `normalized_scope`, `sequence`, `anchor`, `completed_at`. Receive and reconciliation update peer progress; session and incremental receive read it. It persists as peer progress.
- `sync_observed`: key `(namespace, collection, writer, first, added_seq)`; columns `last`, `removed_seq`. Authoring and merge union exact observed dots; merge and snapshot construction read current or pinned versions. Current intervals persist; maintenance reclaims retired versions when pins permit.
- `sync_manifests`: key `(namespace, digest)`; columns `kind`, `content_digest`, `encoded_bytes`, `item_count`, `fragment_count`, `metadata`. Publication writes immutable content-addressed metadata; snapshot and journal readers verify and read it. GC removes unreferenced objects under bounded work.
- `sync_chunks`: key `(namespace, manifest, ordinal)`; columns `offset`, `digest`, `bytes`. Manifest publication writes fragments; readers verify and assemble them. GC removes them with their manifest.
- `sync_manifest_edges`: key `(namespace, parent_manifest, child_manifest)`; no other columns. Manifest publication writes dependencies; readers validate them and GC checks incoming references. GC removes outgoing edges with their parent.
- `sync_leases`: key `(namespace, id)`; columns `generation`, `head`, `head_anchor`, `state_seq`, `parent_lease`, `scope`, `expires_at_ms`, `status`, `retained_root`, `descriptor`, `build_state`. Snapshot construction and lease operations write durable build progress and retained roots; source readers and maintenance inspect them. Release or expiry revokes pins; maintenance deletes revoked rows.

For a state `s`, an observed interval is visible when `added_seq <= s AND (removed_seq IS NULL OR removed_seq > s)`. Current rows have null `removed_seq` and are canonical disjoint, non-adjacent intervals per collection and writer. `insert_on` unions overlapping or adjacent intervals, charges mutations, and inserts the merged version. It deletes replaced rows created at the same state and retires older rows by setting `removed_seq`. `reclaim_step_on` deletes retired versions with `removed_seq` at or below the minimum pinned state; without a pin the current state is the bound. Current observation survives journal pruning.

## Snapshots and receipt

A durable build lease captures generation, publication head and anchor, accepted state, scope and expiry. It pins the state and journal head against reclamation and compaction, and retains the immutable root and dependencies as construction proceeds. A ready descriptor and resumable build checkpoint live in the lease; release or durable expiry removes the pin.

`SnapshotWork` version 2 advances in bounded steps through Context → Count → Hash → Publish. For each collection, Context pages observed intervals at the captured state and publishes its exact context; Count scans candidate headers at that state; Hash reads and validates candidate bodies to form the range summary. Publish retains the completed context root and publishes the descriptor. Checkpoints permit retries across transactions, and a request may execute multiple steps within its limits. Range summaries and splits also use the captured view.

The receiver keeps comparison coverage and incomplete assembly in a bounded process-local ledger. `commit_complete_range` prepares a complete leaf, reads current candidates and observation, plans the merge, and calls `apply_merge_on` when needed. Merge, source projection and journal updates commit together before the ledger marks coverage complete. Once ranges are covered, `promote_step` first handles Arrivals: candidates that became live after the target snapshot and are observed but absent from the source live set. It then applies bounded chunks of `C0_S \ C0_T`, subtracting already observed dots before merge. Projection and each promotion commit precede ledger progress. Completion advances the checkpoint after reconciliation.

## Compaction and maintenance

`compact_on` advances the floor to a retained journal event and its exact state and anchor, subject to active state and head pins. It does not construct additional context history. Bounded maintenance advances the floor, prunes journal rows through it, reclaims retired candidate and observed versions, and collects unrooted manifests and their chunks and edges. Journal references, unrevoked lease roots and incoming manifest edges keep objects live. Maintenance also revokes expired leases and removes revoked rows; its continuation is process-local and each step commits separately.

Every namespace write transaction must take `causal_leases::lock_on` as its first statement, including caller-owned transactions before reading source rows. SQLite WAL deferred transactions that read first can fail with `SQLITE_BUSY_SNAPSHOT` when later upgrading to a writer; acquiring the writer lock first serializes these writes before their reads. Schema installation uses its dedicated write transaction.
