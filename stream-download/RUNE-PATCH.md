# Rune changes to stream-download 0.24.5

This is stream-download 0.24.5 from crates.io, with its original licenses, used through
`[patch.crates-io]` in the workspace `Cargo.toml`.

- `reqwest` is 0.12 instead of 0.13, so the workspace keeps one reqwest version. The
  feature names follow 0.12 (`rustls-tls`; the 0.13-only `system-proxy` default is
  dropped). The `reqwest-middleware` feature still expects reqwest 0.13 and is unused.
  The `reqwest-retry` dev-dependency, used only by doc examples, is dropped.

- Seeks (`src/source/mod.rs`, `handle_seek`): a queued seek to bytes already downloaded,
  or to the end of the stream, acknowledges the waiting reader instead of looking for a
  missing range (upstream 0.24.5 leaves the reader asleep until another chunk arrives;
  0.22.4 also panicked there). With `prefetch_bytes(0)` a seek keeps prefetch off, so the reader wakes as soon as
  its range arrives instead of waiting for earlier missing ranges.
- Downloader exit (`Drop for Source`, and a drop guard on the download task in
  `src/lib.rs`): if the downloader stops without finishing, including by panic or task
  drop, the download is marked failed and blocked readers wake with an I/O error instead of
  waiting forever.
- Memory storage reads (`src/storage/memory.rs`): the storage records the ranges a writer
  stored, and a read stops at the end of the written range at the reader's position. A
  count of all bytes written let a reader past a partial write, a writer seek or an
  overwrite read preallocated zeros as data, and a read after seeking past the end of the
  buffer panicked. A seek before the start is an `InvalidInput` error instead of wrapping.
  Tests: `src/storage/memory_tests.rs`; on unpatched 0.24.5 all but the sequential control
  fail.
- Bounded storage seeks (`src/storage/bounded.rs`): a seek moves only the reader's or writer's
  absolute position. A forward seek moved a position without adding to a separate count of
  bytes read or written, so a later backward seek could subtract more than that count and
  underflow (a fresh reader seeking to 8 and back to 0 panicked in a debug build). The counts
  are replaced by the set of positions whose ring slots hold what the writer stored for them,
  which only writes change: a write adds its positions and drops every other position whose
  slot it reused. A read ends at the end of the stored range at the reader's position, so it
  never returns a byte that was not written for that position, and a position more than a ring
  below the furthest write gets the "too far behind" error. A writer behind the reader can
  still reuse the slots of unread bytes, as upstream; those positions then read as nothing
  instead of the gap's bytes. A wrapped read no longer underflows while the writer is behind
  the reader. A seek before the start of the stream, or past what a `usize` holds, and a write
  past `usize::MAX` are `InvalidInput` errors instead of wrapping. Tests:
  `src/storage/bounded_tests.rs`; on unpatched 0.24.5 all but the wrapped-ring writer-seek test
  fail.
- Process exit status (`src/process/mod.rs`, `ProcessStream::on_finish`, and
  `Source::download` and `finish_or_find_next_gap` in `src/source/mod.rs`): a command that
  closes its output and exits unsuccessfully fails the download, so the reader gets an I/O
  error instead of a normal end of stream. The downloader announces the end of the stream
  only after `on_finish` returns, so a reader cannot see EOF before the exit status is known,
  and a cancel while `on_finish` waits calls it again as a cancellation, which kills the
  command. With `cancel_on_drop(true)` that includes dropping the stream before the command
  exits. Tests:
  `src/process/exit_status_tests.rs` (`--features process`); on unpatched 0.24.5 the
  nonzero-exit and cancel-after-EOF tests fail.
- Pipeline children (`src/process/command_builder.rs` and `SpawnedCommand` and `ProcessStream`
  in `src/process/mod.rs`): the builder keeps every intermediate `Child` it starts instead of
  dropping the handle after connecting its output. `ProcessStream::on_finish` kills and waits
  for the intermediate children once the last command exits or the stream is cancelled, and
  `CommandBuilder::spawn` kills and waits for the children it already started when a later
  command cannot be spawned. Dropping a spawned command or process stream does the same for
  any child that has not been reaped yet. The reaper runs on a thread of its own rather than
  the shared blocking pool, so readers that hold every blocking slot while waiting for
  completion cannot stall it, and its completion receiver is kept across a cancelled
  `on_finish` call so completion and stderr cleanup wait for the same reaper next time. Tests:
  `src/process/pipeline_tests.rs` (`--features process`, which adds `libc` as a dev-dependency
  to observe reaping with `waitpid`). The failed-spawn test runs in a process of its own and
  requires `waitpid(-1, WNOHANG)` to report `ECHILD`, so both a running child and a zombie
  fail it; another test holds the only blocking-pool slot. On unpatched 0.24.5 the completion,
  cancellation and spawn-failure tests fail because the intermediate child keeps running, and
  the blocking-pool test times out.
- Adaptive storage capacity (`src/storage/adaptive.rs`, `src/storage/mod.rs`, and
  `from_create_stream` in `src/lib.rs`): a stream of unknown length uses a ring of the buffer
  size, but the provider reported no capacity, so a read or seek larger than the ring was not
  rejected and waited forever for space the writer could not free. The provider now reports its
  capacity for the stream's content length (new `StorageProvider::max_capacity_for`, which
  defaults to `max_capacity`): the buffer size for unknown lengths, otherwise the capacity of the
  fixed or variable provider that the length selects, so finite streams keep reading buffers
  larger than the buffer size. Tests: `src/storage/adaptive_tests.rs`; on unpatched 0.24.5 the
  oversized read and seek tests stall.

Regression tests: `src/source/lifecycle_tests.rs` (on unpatched 0.24.5 the cached-seek,
zero-prefetch and downloader-panic tests fail; the two gap tests guard the 0.22.4 panic), run from this directory with
`cargo test --lib source::lifecycle_tests --target-dir ../target`.

Remove this copy once upstream releases the fixes below and the workspace moves to
reqwest 0.13.
