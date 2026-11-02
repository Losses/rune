# Sync

This is the specification of the neutral synchronization engine. **Must** states a required behavior; conformance requires the acceptance cases below. The selected merge is observed-remove with retained concurrent live candidates and exact causal context. The public record, receive, snapshot and coordinator APIs use this causal state representation.

`sync` replicates caller-defined records. It owns deterministic merge, bounded journal exchange, state reconciliation, and receive progress. The caller owns application entities, storage, discovery, authentication, filesystem access and file-transfer policy. Storage adapters and application integration are specified in [Synchronization integration](../documents/sync-integration.md).

An object's absence is meaningful only together with exact observation knowledge and complete coverage of the stated range. Immutable comparison views carry that evidence as synchronization state. Adapters choose how to retain those views.

## Model and identity

The caller implements `SyncModel` with a collection type, payload type, and structural validation. Reference resolution belongs to the caller's binding. Model validation is a pure, deterministic function of its inputs.

The optional `validate_revision` hook validates relationships between a value and its original revision author. It runs for both locally authored and received records. A relay preserves the original revision and its author.

Each model supplies a stable model identifier, schema identifier, and stable nonempty collection tags. Tags are unique within the model, limited to 128 UTF-8 bytes, and independent of Rust enum declaration order. The serialization of a collection uses its tag. All peers in one namespace must agree on the model/schema before exchanging data. Validate compatibility before changing state.

The persistent identities have distinct meanings:

- `namespace`: the shared dataset selected by the caller.
- `replica`: the database instance publishing state; independent of an authentication credential.
- `Revision.actor`: the caller's original revision author, preserved through relay.
- `writer_incarnation`: a unique counter-allocation lifetime producing operation dots.
- `generation`: a contiguous publication history with its own sequence numbers.
- `ObjectKey = (collection_tag, object_uuid)`: an enduring logical object identity.

Canonical state is keyed by namespace and object key; authentication identifies permitted callers. `Revision.actor` denotes the original author. Writer incarnations identify counter allocators. Each existing candidate retains its original author.

A live candidate contains schema, namespace, object key, dot `(writer_incarnation, counter)`, revision `(counter, author_id)` and a payload. An assignment replaces the live candidates observed at its key and creates one fresh candidate. A removal consumes the observed live dots and retains their observation context. Concurrent unobserved assignments survive. Explicit re-add uses a fresh dot under the same stable key. A caller wanting narrower observation than the command transaction sees must supply the exact observed dots.

An empty lookup reports current absence. Distinguishing a previously removed key from an unseen key requires retained keyed history. After that history is reclaimed, the causal context still records observed operations, while their former key bindings may have been reclaimed.

For example, a caller can define a notes model with a stable `notes` collection tag and a `Note { text: String }` payload. Its validator can reject text above its declared bound. `Record<Notes>`, `Scope<Notes>`, and `MemorySyncStore<Notes>` then use that caller-defined model directly. A non-music model must remain part of conformance testing.

## Encoding, revisions, and merge

A candidate is limited to 64 KiB in its compact serialized representation. Envelope fields reject unknown fields and unsupported schema/protocol identifiers. Namespace, replica, author, writer incarnation, generation, and object UUIDs must be non-nil. Counters and journal positions are unsigned values bounded by `i64::MAX`.

Canonical candidate encoding is compact UTF-8 JSON with recursively sorted object keys, preserved array order, and the string/number encoding of the reference canonical encoder. A model normalizes semantically unordered lists. Portable fields are explicitly selected by the model; SQL IDs, paths and incidental ORM fields remain local. Serialization failure returns an error. Codec fixtures include Unicode, escaping, integer bounds, optional fields, dots, contexts and admitted numeric formats. Other implementations must reproduce these exact bytes.

For display, select the maximum tuple among surviving candidates of the same key:

```text
(revision.counter, author_uuid_bytes, canonical_candidate_digest, dot_bytes)
```

The digest is domain-separated BLAKE3 over the complete immutable candidate. UUIDs compare as 16 bytes, counters as unsigned integers, and digests as 32 bytes. Display selection preserves every surviving concurrent candidate. One dot identifying different keys, revisions or values is rejected as invalid history.

A local command joins candidate/context changes, its original-author revision and publication history in one transaction. Receiving candidates observes their logical revision counters even when their displayed values lose. Observation advances the local logical clock while preserving received revisions. Overflow returns `ClockExhausted` and rolls back the mutation; reads and reconciliation remain available. A fresh writer incarnation preserves the logical revision order.

For each collection, let `L` contain live `(key,dot,candidate)` entries and `C0` contain exact individually observed dots. Every live dot belongs to `C0`. A writer context consists of a contiguous prefix and sorted disjoint nonadjacent exception intervals above that prefix. For example, receiving counters 1 and 3 produces prefix 1 and exception interval [3,3]; counter 2 remains unobserved. Normalization and union preserve exact membership.

Accepted range contexts form `Q`. Effective observation at key `k` is `K(k,d) = C0.contains(d) || any(q.range.contains(k) && q.context.contains(d))`. For two compatible complete states, union effective observation pointwise. Keep a candidate present on both sides; keep a candidate present only on one side exactly when the other side's effective context does not contain its dot at that key. This observed-remove join is commutative, associative and idempotent under unique immutable dots and complete range evidence. The [observed-remove set construction](https://arxiv.org/abs/1210.3368) motivates the exact-context representation; partial range publication also requires the snapshot and coverage requirements.

To merge a complete source interval, evaluate the receiver's old effective context, perform that join only inside the interval, add every incoming live dot to `C0` even when suppressed, and retain the source context qualified by the interval. Merge against current receiver state, including edits since comparison started. Validate all input before committing. Context-only changes and hidden concurrent candidates are state changes that must be durable and relayable; an exact duplicate preserves the publication head.

Normalize overlapping contexts by an ordered endpoint sweep into disjoint segments with pointwise union; coalesce adjacent equal contexts. Contexts stay key-qualified. Different segment contexts retain their individual key bounds even after full source coverage. A context may be promoted only when proven valid over the entire collection root.

## Caller interfaces and read behavior

Retain the `SyncStore<M>` and `SyncPeer<M>` boundaries. Their identity, head, checkpoint, object lookup, and page-read methods are reads of committed state. Source import, content hashing, replica initialization, discovery and projection are separate explicit operations. Opening/creating a store is an explicit lifecycle operation.

The store supplies atomic local authoring and atomic page application. The peer supplies remote metadata operations through a transport chosen by the caller. `StorePeer<M>` adapts a local store for testing and in-process sessions. Reverse synchronization uses the same directional operation with endpoints exchanged; orchestration and peer scheduling are caller responsibilities.

`Record<M> { key, value: Option<Payload>, revision }` is a command receipt or projected view. Canonical storage uses live candidates and exact observation context. `get` returns a live projected record or `None`; `candidates(key, after_dot, limits)` exposes all surviving candidates. `write_local` observes every current candidate; `write_observed` receives an exact dot set. `Candidate<M>` always contains a payload. A `ProjectionBatch` contains exact before/after selected candidates, including a lower revision or absence when the previous winner is removed. Hidden changes still publish history.

The model's `validate(key, value)` and `validate_revision(key, value, revision)` run for every decoded candidate before suppression. An explicit removal calls validation with `None`. For every key identified by a current or incoming candidate, an empty resulting live set also calls validation with `None`, even if both displayed values were absent because the incoming value was suppressed. The caller uses these hooks to enforce owner-bearing state. Classifying a context dot by object policy requires evidence of its key binding.

Atomic store methods accept explicit page, apply and staging limits. Concrete durable adapters expose methods accepting their own database transaction. The caller joins its projection to the accepted-state transaction. Projection runs only during an accepted mutation.

Snapshot comparison is an optional source/sink capability. Its principal asynchronous methods are:

```rust
// Source: all expensive operations return Ready(value) or Incomplete(cursor).
open_snapshot(request_id, scope, work_limits, resume) -> WorkOutcome<SnapshotDescriptor<M>>
summarize(snapshot, range, work_limits, resume) -> WorkOutcome<RangeSummary<M>>
split(snapshot, range, work_limits, resume) -> WorkOutcome<Vec<KeyRange<M>>>
leaf(snapshot, range, after: Option<CandidateCursor<M>>, page_limits) -> LeafPage<M>
context_page(snapshot, range, after: ContextCursor, page_limits) -> ContextPage<M>
read_fragment(snapshot, manifest, ordinal, max_bytes) -> Fragment
release_request(request_id) -> ()
close_snapshot(snapshot) -> ()
cancel_snapshot_build(cursor) -> ()

// Sink: only a complete validated logical range may mutate accepted state.
begin_reconciliation(request_id, remote_descriptor, local_descriptor) -> SessionId
cancel_reconciliation(session) -> ()
discard_range(session, range) -> ()
coverage(session, ledger_revision, after, page_limits) -> CoveragePage<M>
split_range(session, parent, children: Vec<RangeSummary<M>>) -> CoverageProgress
accept_equal_range(session, remote_summary) -> CoverageProgress
context_progress(session, range) -> ContextProgress
leaf_progress(session, range) -> LeafProgress<M>
stage_context_page(session, page) -> ContextProgress
stage_leaf_page(session, page) -> LeafProgress<M>
commit_complete_range(session, range, apply_limits) -> (CoverageProgress, ProjectionBatch<M>)
normalize_step(session, work_limits, resume) -> WorkOutcome<CoverageProgress>
complete_reconciliation(session) -> ReceiveProgress
```

`CandidateCursor` contains `(key,dot)` in stable collection-tag, object-UUID, writer-UUID and unsigned counter order. A single key's concurrent candidates can span pages while retaining a unique continuation position. A context cursor contains a segment index and next boundary; an initial cursor and an unbounded endpoint are distinct states. A work cursor binds the operation, generation and strictly advancing position. Expired or incompatible cursors return `StaleWork`.

`begin_reconciliation` binds both immutable descriptors. `split_range` replaces an outstanding interval with source child summaries after validating snapshot identity and complete disjoint coverage. The ledger retains each child commitment before staging. The coordinator obtains child commitments from the same trusted immutable source view and rejects replacements incompatible with staged or accepted work. `accept_equal_range` compares the remote summary with the sink's own pinned snapshot of that interval; a Boolean from a caller is insufficient. Coverage pages are bound to a ledger revision. If that revision changes, a page read must restart rather than skip intervals. Accepted causal knowledge survives source-view closure and expiry.

## Journal, scope, and continuity

Every state-changing merge gets a new local journal sequence. Sequence is independent of the record revision: a late low-revision record on another object must remain publishable through an intermediate replica.

Journal state includes hidden concurrent candidates and context-only changes. A keyed delta includes exact removed dots and fresh immutable candidates. A complete-range event includes its interval, exact context and complete source-live evidence, or a retained immutable reference that supplies that evidence. Removal requires both exact observation context and complete source-live evidence, since an observed dot may still be live at the source. Every dependency needed to replay an acknowledged event must remain available through retained history or a complete recovery view. References are content-identified and subject to the same model, scope and range validation.

Immutable history must reconstruct all live candidates and qualified contexts at head `S`. Selecting the latest scalar payload per key is insufficient. An adapter may journal complete post-merge live sets per changed key together with immutable context events, or use immutable dot-add/remove history with equivalent indexed reconstruction. It must verify that reconstruction equals replay, including range-only events. History layout is a storage-adapter responsibility.

A scope is a nonempty set of collection tags. Its identifier is BLAKE3 of a domain-separated encoding of model ID, schema, and sorted tags. The identifier is computed from the actual scope definition. Expanding or changing scope creates independent receive progress. Narrowing scope preserves stored objects. Arbitrary application query predicates are outside the initial scope capability.

Receive progress is keyed by `(namespace, source_replica, source_generation, scope_id)`. It stores both the scanned sequence and the source's continuity anchor at that sequence. The source maintains a prefix digest:

```text
H0 = H(LP("sync.history.initial") || namespace || replica || generation || LP(model) || LP(schema))
Hn = H(LP("sync.history.link") || H(n-1) || U64(sequence) || LP(collection_tag) || event_digest)
```

Variable-length fields in hash inputs are length-prefixed; tags separate uses of the hash. The journal contains every accepted causal change represented by the prefix, including hidden candidates and context-only changes. Compaction retains the anchor at its declared history floor. Continuity anchors detect disagreement with previously observed history. Authorization and truthful complete state reports remain separate caller and trusted-peer assumptions.

`checkpoint` returns `Option<ReceiveProgress>`: no checkpoint is distinct from an initialized sequence zero. First contact, a new generation or a new scope requires snapshot reconciliation even when the journal head is zero, because canonical state may predate that publication history. A caller without snapshot support may start incrementally only from a verified empty genesis. Before incremental transfer, the peer reports its generation, head, retained floor, and anchor at the receiver's checkpoint. A changed generation, anchor mismatch, head below the checkpoint, or checkpoint below the retained floor returns `NeedsReconciliation`. Regrowing a rolled-back journal beyond an old numeric head must not bypass this check.

## Bounded page exchange and atomic application

A directional session captures a fixed source head `through`. Default page limits are 128 scanned entries and 1 MiB serialized bytes; hard limits are 1,024 entries and 4 MiB. The byte bound includes the envelope and continuity fields. Defaults may be lowered by the caller, not raised above hard bounds.

`next` is the last scanned sequence, including entries excluded by scope. `done` is true exactly when `next == through`. Later source writes wait for a subsequent tail session. Every nonfinal page advances its scanned position, including a page whose entries are all excluded by scope. In-scope records are transferred in full through pages or bounded fragments. If the first logical event cannot fit, return `FragmentNeeded` binding the exact request, sequence, collection, event digest and manifest. Earlier complete links can form a preceding page. Page batching and fragment transfer have separate byte allowances: the coordinator uses `pages.max_bytes` for pages and `stages.max_fragment_bytes` for fragment responses. Both are capped by the remaining request/response allowance of a bounded pass. An already published fragment uses the fragment allowance independently of the page budget. If even one bounded fragment envelope cannot fit that allowance, return `LimitTooSmall` with the required size.

Each page identifies namespace, model/schema, source replica/generation, scope definition, `after`, `through`, `next`, completion, anchor at `after`, anchor at `next`, and one `ScanLink` for every contiguous scanned sequence, including out-of-scope entries. Each link contains sequence, collection tag and event digest; in-scope links also carry the event or its manifest. Recompute every anchor transition. Returned sequences cover `(after, next]` exactly; all candidates and context dependencies match namespace and scope. Validate the full page before changing persistent state.

For a new advancing page, `after` and its anchor must equal the receiver's committed progress. A page entirely at or below already committed progress is an idempotent replay and returns that progress without new authorship. A gap or a partially overlapping advancing page returns `RetryAt(committed_progress)`; the caller rereads from that point. Every advancing interval requires verified continuity. Incompatible history returns `NeedsReconciliation`.

One receive transaction must include record merges, observed clock, relay entries, and receive progress. A composed adapter must also include its necessary projection or durable pending work. Commit precedes acknowledgement. Any validation/write failure rolls back all these changes. Lost acknowledgement after commit is handled by replay. Notifications and external work start after commit.

Byte accounting must be linear in records plus one final encoded-size validation, rather than repeatedly serializing the growing page. Reads use indexed key/sequence traversal; memory is bounded by the requested page plus bounded encoding buffers.

An event or context exceeding a page uses a manifest with content digest, total bytes, element count and bounded fragments identified by manifest and ordinal. Stage fragments separately from accepted state, with count, ordinal, offset, digest and total-size validation. Duplicate identical fragments are harmless; conflicting content is an error. Missing fragments leave the logical input incomplete. Validate complete manifest and logical content before the event's atomic merge. The receive checkpoint remains before the unfinished event; fragment progress is independently resumable while its bounded stage exists. Uncommitted receiver stages may expire on restart; expiry leaves the checkpoint unchanged. Resource admission limits total staged bytes, concurrent stages and logical merge work. Rejection or cancellation releases uncommitted stages and preserves accepted state. Quota admission preserves all accepted context. A caller may configure finite supported concurrency and logical-operation limits, but authoring must reject a mutation exceeding those limits before changing business data.

## Immutable state snapshots

A snapshot descriptor identifies namespace, model/schema, source replica/generation, scope definition, snapshot UUID, source head `S`, continuity anchor at `S`, root range summary, and expiry. It names immutable live candidates plus exact effective contexts. Every summary, context fragment, split and leaf page is bound to it. A partially reconciled source exports its durably accepted qualified contexts independently of the original peer's availability.

A leaf page identifies its snapshot, exact interval, exclusive input `(key,dot)` cursor, last scanned candidate cursor, completion flag and ordered complete candidates. Its next cursor is strictly greater than the input for a nonfinal page and remains inside the interval. The final page explicitly accounts for the remaining suffix, including an empty suffix. The sink validates descriptor, interval and cursor against its outstanding work before accepting coverage; a page from another interval cannot finish this one.

A source may implement an immutable database view or materialize the scoped records under one consistent database transaction. Construction may scan the dataset using bounded buffers and short transactions committed before network exchange. Durable materialization publishes a descriptor only after construction commits; incomplete construction is not serviceable. Reuse an unchanged valid snapshot instead of rebuilding it for every idle session. Resource admission, cancellation, storage quota and expiry are explicit caller configuration.

Expiration returns `SnapshotExpired`. Serving requires a valid descriptor and its captured rows and hashes. Accepted candidates and complete-range contexts remain committed, but incomplete coverage cannot advance a source checkpoint. Accepted range context survives transport-session and remote-snapshot expiry. A new snapshot starts new coverage and merges with existing effective context. A valid lease pins repeatable state and the required journal tail after `S`. Forced revocation persists expiry before releasing history; a missing tail requires a new snapshot.

The memory implementation provides a pinned immutable copy for conformance testing. Durable adapters must survive a restart with either the same valid snapshot/coverage or an explicit expiry; neither behavior may be mistaken for completed reconciliation.

## Adaptive ranges and aggregate hashes

State comparison is ordered by `(stable_collection_tag_bytes, object_uuid_bytes)`. Ranges are half-open `[lower, upper)` with explicit unbounded endpoints. A root range covers every key admitted by the scope. Cursor comparison uses the complete unique key order. Edits preserve the object key.

A `RangeSummary` contains the exact interval, candidate count, canonical candidate/context byte totals, and an aggregate hash. Hash a domain tag, model/schema, scope ID, interval bounds, ordered candidate digests and the exact normalized effective-context partition. Context-only differences matter even when displayed values are identical. Equivalent context layouts must normalize to the same logical intervals and sets before hashing. Snapshot UUID, local paths, local SQL IDs and local publication head are excluded: equal logical states compare equal across storage layouts.

Peers compare exactly the same intervals. A peer with more records/bytes in a differing interval proposes a split; the other peer summarizes those exact child intervals. Validate strictly increasing boundaries and a disjoint partition covering the complete parent interval. Empty child ranges remain explicit. Object absence is established from complete evidence for the same interval on both peers.

The default leaf target is at most 128 records and 1 MiB canonical bytes on each side. Above it, choose a pivot near the median canonical-byte position of the larger snapshot interval, with a record-count median as fallback. The pivot must strictly advance. If no interior key exists, read the bounded leaf directly; a single large valid record is not recursively split into an invalid record fragment. Leaf reads may require multiple byte-bounded pages.

Equal summaries skip transfer. Differing complete leaves exchange live candidates and effective context in both directions. Absence removes a local candidate only when complete source range evidence observes that exact dot and excludes it from source-live candidates. That evidence becomes available after complete range assembly. Context boundaries participate in splitting; a leaf spanning multiple contexts carries their complete exact partition. Count/byte targets can adapt within limits. HLC age may influence scheduling. Exact coverage, stable keys and observed dots determine completeness, identity and removal.

Each round permits at most 1,024 summary/split requests, 64 MiB of transferred data, and subdivision depth 64 by default. Reaching a budget returns `Incomplete` with a resumable cursor. Limits are explicit configurable work bounds. Requests are processed through a bounded work queue rather than an unbounded recursive call stack.

## Coverage, resumption, and transition to the journal

The receiver maintains a bounded reconciliation ledger identified by both descriptors and scope. Its ledger starts with every scoped root outstanding. Splitting replaces an interval with a validated complete partition. A leaf becomes eligible only after all candidate pages, complete context partition, dependencies and the terminal boundary validate. Merge against the receiver's current state, join projection and relay history, then commit accepted state. Only after that commit mark the interval complete in the ledger. A crash between these steps causes safe replay.

`ReceiverLedger<M>` supplies this staging and coverage validation to memory and durable stores. It contains uncommitted fragments, descriptors, page cursors, pending/completed ranges and bounded retry results; accepted candidates, contexts, history and checkpoints remain in the store. Preparing a complete input returns validated evidence and a ledger-revision admission token. The interval remains pending through preparation. Finalization follows the store's successful atomic apply; replay after an interrupted finalization must preserve accepted state and history.

Receiver coverage and uncommitted buffers may expire on restart. Already accepted range state and observation knowledge remain durable and exportable. Source reconstruction leases must remain valid across every process sharing the storage, or be explicitly revoked before their retained history is reclaimed. Each database transaction finishes before a remote request begins.

An equal-summary interval completes only against the receiver's own pinned comparison snapshot. The receiver retains that view or restarts comparison. `complete_reconciliation` takes the session lock and checks full scoped coverage with no outstanding leaf or incomplete logical input. It publishes equivalent context normalization and checkpoint `(S,anchor_at_S)` with successful completion metadata in one durable transaction. Prepared normalization cannot drop another session's contexts or promote heterogeneous intervals to collection-wide knowledge. A crash before commit has no completed checkpoint; after commit, losing the in-memory ledger does not invalidate the committed result.

Progress advances monotonically. If another session has already advanced the same source/generation/scope beyond `S`, completion retains that newer checkpoint and validates its continuity. The caller serializes advancing sessions for a checkpoint key; concurrency races return `RetryAt`.

After snapshot completion, drain the source journal after `S` to a new fixed head. Records written after the snapshots, including relayed merges caused by reconciliation itself, may require another fair round. If the tail is no longer retained, start a new snapshot instead of skipping the gap. Losing snapshot records still observe counters; duplicate records preserve the journal head.

A resumable session can span multiple bounded caller passes. `Coordinator::start_pass` resets only pass accounting; `pass_usage` reports that accounting while preserving the session and accepted progress. Every endpoint operation, including failed attempts and retries, consumes the pass's request and encoded-byte allowances. The coordinator yields before admitting an operation that cannot fit the remaining allowance. Per-operation row, mutation and subdivision limits still apply. The caller retains the coordinator when starting another pass.

## Retention and recovery

Deletion requires explicit intent and exact observed-dot evidence. Contexts retain removed-operation knowledge as exact writer intervals. Storage cost includes live concurrency, writers, gaps, qualified range segments and retained history.

Consider A and B after an exchange with a high progress timestamp. C then delivers an older object to A that B has never observed. At the next A/B comparison, that object's age and B's current absence cannot distinguish this history from one where B previously observed and removed it. An age/intersection rule lacks the observation evidence needed to choose between insertion and removal. Exact dot membership and complete source-live evidence supply that distinction even when delivery contains gaps.

Merging adjacent context intervals and coalescing equal neighboring key segments preserves information. Forgetting a writer's context is different and can resurrect stale candidates. Context retirement requires explicit authority over every replica allowed to return. With unrestricted offline return, retain necessary context. Actor retirement needs an explicit caller membership/fencing policy and a complete rebootstrap rule for excluded stale replicas. Retirement is an explicit caller operation.

Journal compaction requires a declared history floor, its anchor and a serviceable complete recovery view containing all live candidates and effective range contexts. Pinned views protect reconstruction and tails. An unavailable fallback makes compaction ineligible. A design retaining its entire journal must report that growth separately from compact context sizes; small synthetic context measurements do not establish small database storage.

A restored database enters caller-controlled recovery before new revisions are authored. The caller must establish a fresh publication generation where continuity is lost, reconcile with surviving state, observe the surviving clock, and preserve any queued user intentions. Rotating an actor or generation alone cannot reconstruct missing records or make obsolete counters win. A fully rolled-back image cannot detect its own rollback solely from fields in that image; an explicit restore workflow or external/peer continuity evidence is required.

Exclusive database movement and simultaneous cloning are different operations. A clone requires its own replica/publication history before concurrent use. It retains original record revisions while establishing its own history. Exact application workflows belong to the integration specification.

Dot allocation uses a fresh unpredictable writer incarnation for each independently writable handle unless an external exclusive nonrollback counter provides an equivalent guarantee. Clones of one live handle share its allocator. Every new operation uses a fresh dot; committed and uncertain allocations remain consumed, including after rollback. Reopening with a fresh incarnation preserves the caller's original author identity and accepted state. This policy has actor-churn cost and does not itself repair rolled-back publication history or application counters.

## Errors and cancellation

The public result set must distinguish `InvalidRecord`, `InvalidPage`, `IncompatibleModel`, `WrongNamespace`, `RetryAt`, `NeedsReconciliation`, `SnapshotExpired`, `LimitTooSmall`, `ClockExhausted`, `ResourceBusy`, `Cancelled`, and storage/transport failure. Failures retain their typed error result.

Cancellation before commit preserves the previous receive progress. Cancellation after commit leaves replayable durable progress. Snapshot range completion follows the same rule. Reuse of an interrupted transport stream is the caller's transport decision; the engine resumes from committed progress. Storage adapters serialize conflicting writers or roll back and return a retryable error.

A range that exceeds apply or work admission discards only its uncommitted assembly, subdivides under the same immutable source view and retries. Already accepted ranges remain. Multiple candidates at one key count as one projected key, with their complete byte and mutation costs. An unsplittable oversized key fails explicitly. An oversized journal event leaves its checkpoint unchanged and falls back to complete-state comparison. `RetryAt` reloads committed progress; generation changes refresh identity and the checkpoint key. Invalid or corrupt evidence remains an error.

Transport failure retains the current operation and request UUID for retry. `release_request` idempotently releases a source build or ready view by its initial request UUID, including when its first response was lost. `cancel_reconciliation` releases receiver coverage and stages while preserving accepted state. Coordinator abort and recovery perform bounded cleanup per advance and retain unacknowledged cleanup IDs. Known closed or expired operations accept cleanup. New operations use fresh request IDs after release or expiry; work-incarnation validation binds cursors to their original build. Asynchronous cleanup is explicit rather than relying on destruction of an object.

## Convergence and acceptance

For a fixed model, namespace and scope, convergence requires authoring eventually to stop, fair delivery of all accepted live/context state over permitted links, and sufficient admitted resources. Under unique immutable dots, exact context and complete stated coverage, observed-remove join is associative, commutative and idempotent; replicas receiving the same finite inputs reach equivalent live candidates and effective contexts. Deterministic projection then selects the same displayed candidates. Permanent partitions, omitted context-only relay events and incompatible scopes do not meet these assumptions. This algebra does not by itself prove fragments, history reconstruction or transactions. Replays stop growing journals and clocks. A previously unused pair may establish progress without changing state; its immediate repeat must be empty.

The neutral conformance suite must prove:

- Merge algebra, concurrent unseen update/removal, explicit re-add, maximum-counter behavior, unknown-field rejection, and exact codec/hash fixtures.
- Atomic invalid-page rejection, committed-prefix restart, lost acknowledgement, filtered empty-page progress, byte limits, and no skipped late low-revision objects.
- Stable key ordering and identical hashes despite different local representations, partition boundaries, and more tied timestamps than fit a page.
- Empty, nested, overlapping and malformed range partitions; complete coverage; resource-budget interruption; snapshot expiry; and mutation between every comparison step.
- Detection of divergent history after numeric head regrowth; recovery below a retained floor; scope changes; and retained causal knowledge after interrupted range exchange and source loss.
- Read-only identity/head/page calls and no application callbacks during them.
- Same-namespace multi-hop convergence and steady state over chain, ring, star and bridged clusters, followed by all-pairs progress catch-up.

Exact interval operations are checked against an explicit-dot oracle, including gaps, boundary counters, overlapping range merges and atomic collision rejection. Multi-hop cases compare all surviving candidates and effective contexts as well as selected values. After graph convergence, obsolete incremental assignments delivered through a previously unused pair must not create new accepted state or history. Tests also cover two keys with an apply limit of one, multiple concurrent candidates at one key, repeated cancellation after lost initial replies, small pass budgets and coverage envelopes too small to fit.

Retention measurements distinguish logical context bytes from database overhead and retained history. The workload specification includes at least 100,000 create/remove operations with 2, 8 and 32 writers, for both compact and deliberately gapped delivery. Reports identify writer churn, live candidates and range partitions; a compact prefix result is not a bound for arbitrary gaps. Storage-adapter measurements use the actual persistence implementation separately.

Run the neutral engine tests with `cargo test -p sync --offline`. Application-level convergence, ORM transactions, content durability, runtime environments and measurements are specified in [Synchronization integration](../documents/sync-integration.md); core record equality alone does not establish them.

## Fixed encoding and resource admission

The engine defines a fixed encoding. Candidate JSON has exactly `model`, `namespace`, `key`, `dot`, `revision` and `payload`. `key` contains stable `collection` and `id`; `dot` contains `writer_incarnation` and `counter`; `revision` contains `counter` and `actor`. UUIDs are lowercase hyphenated text. Integers use shortest unsigned decimal; optional fields are explicit null; binary digests use lowercase even-length hexadecimal. Reject duplicate JSON keys, nonfinite numbers, unsupported numeric forms and unknown envelope fields. Recursively sort object keys by UTF-8 bytes; preserve arrays. Model/schema/tags are nonempty and at most 128 bytes, with at most 256 collections in a scope.

A keyed event contains `kind:"key"`, `key`, `author`, `removed` and `added`. Removed dots are unique and sorted; additions are unique, ordered by candidate cursor and match the event key. A range event contains `kind:"range"`, `scope`, `range`, `context_partition`, `source`, `range_summary` and `live_manifest`. Its original normalized source scope is preserved when relayed and is used to verify the range commitment; relays retain it even when their local journal scope differs. Its dependencies establish both complete live state and exact observation partition. A context contains sorted writers, each with a prefix and disjoint nonadjacent exception intervals. Validation rejects alternative noncanonical representations before hashing.

Let `H` be BLAKE3, `LP(x)` a four-byte big-endian byte length followed by `x`, and `U64` an eight-byte unsigned big-endian number. Hash domains are exact UTF-8 strings:

- Candidate: `H(LP("sync.candidate") || LP(canonical_candidate_json))`.
- Event: `H(LP("sync.event") || LP(canonical_event_json))`.
- Scope: `H(LP("sync.scope") || LP(model) || LP(schema) || U64(count) || concat(LP(tag)))`, with tags in byte order.
- Context: `H(LP("sync.context") || LP(canonical_context_json))`.
- Manifest content: `H(LP("sync.manifest.content") || LP(complete_encoded_content))`.
- Manifest: `H(LP("sync.manifest") || LP(kind) || U64(bytes) || U64(items) || U64(fragments) || content_digest)`.
- Fragment: `H(LP("sync.fragment") || manifest_digest || U64(ordinal) || U64(offset) || LP(bytes))`.
- Range context commitment: `H(LP("sync.range.context") || LP(model) || LP(schema) || LP(bounds) || U64(segment_count) || concat(LP(segment_bounds) || context_digest))`. This hashes normalized logical contexts and excludes fragment layout and manifest identities.
- Range: `H(LP("sync.range") || LP(model) || LP(schema) || scope_digest || LP(bounds) || U64(candidate_count) || U64(candidate_bytes) || U64(segment_count) || concat(LP(segment_bounds) || context_digest) || concat(candidate_digest))`. Context segments and candidates are in canonical order. The snapshot identifier is checked in the envelope and excluded from this logical digest.

A manifest kind is `key_event`, `range_event`, `context`, `context_partition`, `context_root` or `leaf`. A context root contains the exact scope digest and one partition reference per scoped collection, sorted by stable tag. Its JSON is `{scope, partitions:[{collection,partition}]}`. Missing, duplicate, extra or wrong-kind children are invalid. A decoder receives the expected scope definition separately from the digest. Its serialized fields are kind, manifest digest, content digest, encoded bytes, item count and fragment count; the identity hash excludes its own digest field. Dependencies must be complete and bounded before accepted publication. Byte fixtures cover the complete encodings and hash inputs, including Unicode, escaping, empty roots and maximum supported counters.

The default logical event or context limit is 8 MiB, with a 64 MiB hard maximum. Count the root reference, transitive dependencies and decoded items. Logical item defaults are 65,536 with a 1,048,576 hard maximum. A manifest has at most 65,536 fragments. Staging defaults to 64 MiB and four stages per store; hard maxima are 512 MiB and sixteen stages. Page and fragment limits are defined in [bounded page exchange](#bounded-page-exchange-and-atomic-application).

Resident admission includes pending/completed coverage, retry receipts, source request caches and decoded working data. Admission enforces count and byte limits in addition to expiry. Shared context bodies, fragments and decoded values are deduplicated by digest or charged for their expanded resident size before allocation. Coverage limits include the complete serialized envelope, including an empty result. Evicting an expired completed retry result may require a restart; an active interval remains retained until completion or explicit cancellation.

An apply accumulates at most 128 projected keys and 4 MiB of before/after candidate encoding by default, with hard maxima of 1,024 keys and 16 MiB. Mutation work is separately bounded. A context-only input can change large local payloads, so input bytes alone are not an apply bound. Split a complete range before acceptance when necessary. A journal event exceeding receiver apply limits leaves its checkpoint unchanged and requests a serviceable snapshot. An unsplittable oversized key is an explicit limit error. Admission must also ensure the resulting state can be exported and compacted under supported bounds while retaining its complete accepted context.

Round defaults additionally bound scanned rows at 1,000,000 and mutations at 65,536; hard maxima are 16,000,000 and 1,048,576. Request and byte hard maxima are 16,384 and 1 GiB. Subdivision depth is at most 64. Source leases default to ten minutes with a 24-hour maximum. Callers can lower supported work limits; exceeding them returns resumable incomplete work or a specific error. `LogicalLimit`, `StageQuota`, `WorkLimit`, `ConflictingDot`, `ConflictingFragment`, `MissingDependency`, `IncompleteCoverage` and `StaleWork` are distinct from transport failure.
