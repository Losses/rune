# FsIo: Filesystem I/O Abstraction Layer

## Overview

`fsio` is a Rust crate that provides a unified file I/O abstraction layer designed for cross-platform applications. Its primary goal is to offer a consistent API for filesystem operations across standard operating systems (like Linux, macOS, Windows) and Android, where file access is restricted and must go through the Storage Access Framework (SAF).

On Android, `fsio` leverages the `ndk-saf` crate and maintains a SQLite database inside the SAF tree (`.rune/.android-fs.db`) to cache file metadata and content URIs, enabling efficient path-based access without repeatedly traversing the SAF tree.

## Features

- **Consistent API**: A single `FileIo` trait for all platforms.
- **Android SAF Support**: Transparently handles file I/O on Android through `ndk-saf`.
- **Caching on Android**: SQLite cache maps paths to content URIs and metadata (`is_dir`, `size`, `filename`) for fast, IPC-free lookups.
- **Read-through self-healing**: Cache misses are resolved live against the SAF tree and cached; stale entries are evicted and retried once on failure.
- **Standard Fallback**: Uses standard filesystem calls on non-Android platforms.
- **Self-test**: `self_test::run_fs_self_test` runs layered probes (L2-L7) against any `FsIo` backend for on-device diagnostics.

## Usage

The main entry point is the `FsIo` struct, which derefs to `dyn FileIo`.

### Initialization (Standard Platforms)

```rust
use fsio::FsIo;

let fs = FsIo::new();
```

### Initialization (Android)

```rust,ignore
// db_path is tree-relative; the cache DB lives inside the SAF tree.
let fs = FsIo::new(Path::new(".rune/.android-fs.db"), root_uri)?;
```

`FsIo::new` is synchronous and never panics on a nested Tokio runtime. The cache is built on first use (empty database) and reused afterwards; call `refresh_cache()` (a `FileIo` trait method, no-op on other backends) to force a full rescan, e.g. before a library scan.

## API Reference

See `src/lib.rs` for the full `FileIo` trait. Notes:

- `open`/`open_async` are both synchronous under the hood; write-style modes (`w`/`a`/`t`) create missing files (via SAF `create_file` on Android).
- `walk_dir`/`read_dir` on Android are served from the cache with zero provider IPC; they reflect the tree as of the last `refresh_cache` plus any changes made through this `FsIo` instance or healed on access.
- `modified_time` returns seconds since the UNIX epoch (via `fstat` on the SAF fd on Android).
- `canonicalize_path` on Android returns the real `/mnt/user/...` path via `/proc/self/fd` readlink — useful for display, but that path is **not** directly readable under scoped storage; open files through `FsIo` instead.

## Testing

`cargo test -p fsio` runs the conformance suite (`tests/conformance.rs`) against `StdFsIo` and `NoOpFsIo`, plus a host-side self-test smoke run.
