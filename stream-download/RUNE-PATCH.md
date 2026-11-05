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

Regression tests: `src/source/lifecycle_tests.rs` (on unpatched 0.24.5 the cached-seek,
zero-prefetch and downloader-panic tests fail; the two gap tests guard the 0.22.4 panic), run from this directory with
`cargo test --lib source::lifecycle_tests --target-dir ../target`.

Remove this copy once upstream releases the fixes below and the workspace moves to
reqwest 0.13.
