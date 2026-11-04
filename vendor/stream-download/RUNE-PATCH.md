# Rune changes to stream-download 0.24.5

This is stream-download 0.24.5 from crates.io, with its original licenses, used through
`[patch.crates-io]` in the workspace `Cargo.toml`.

- `reqwest` is 0.12 instead of 0.13, so the workspace keeps one reqwest version. The
  feature names follow 0.12 (`rustls-tls`; the 0.13-only `system-proxy` default is
  dropped). The `reqwest-middleware` feature still expects reqwest 0.13 and is unused.
  The `reqwest-retry` dev-dependency, used only by doc examples, is dropped.

Remove this copy once upstream releases the fixes below and the workspace moves to
reqwest 0.13.
