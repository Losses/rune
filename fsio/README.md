# FsIo: Filesystem I/O Abstraction Layer

## Overview

`fsio` is a Rust crate that provides a unified, asynchronous file I/O abstraction layer designed for cross-platform applications. Its primary goal is to offer a consistent API for filesystem operations across standard operating systems (like Linux, macOS, Windows) and Android, where file access is restricted and must go through the Storage Access Framework (SAF).

On Android, `fsio` leverages the `ndk-saf` crate and maintains a local SQLite database to cache file and directory structure, enabling efficient path-based access without repeatedly traversing the SAF tree.

## Features

- **Consistent API**: A single `FileIo` trait for all platforms.
- **Asynchronous**: All operations are `async`, built on `tokio`.
- **Android SAF Support**: Transparently handles file I/O on Android through `ndk-saf`.
- **Caching on Android**: Uses an SQLite cache for file URIs to provide fast, path-based lookups.
- **Standard Fallback**: Uses standard `tokio::fs` on non-Android platforms.
- **Easy Integration**: Designed to be a drop-in replacement for direct filesystem calls.

## Usage

The main entry point is the `FsIo` struct, which provides access to the `FileIo` trait methods.

### Initialization (Standard Platforms)

On non-Android platforms, initialization is straightforward:

```rust
use fsio::FsIo;

let fs = FsIo::new();
// You can now use fs for file operations.
```

### Initialization (Android)

On Android, you must provide a path for the cache database and a root content URI obtained from the Storage Access Framework (e.g., from an `ACTION_OPEN_DOCUMENT_TREE` intent).

```rust
use fsio::FsIo;
use std::path::Path;

#[tokio::main]
async fn main() {
    # #[cfg(target_os = "android")]
    # {
    // This is a conceptual example.
    // let db_path = Path::new("/data/data/com.yourapp/files/fs_cache.db");
    // let root_uri = "content://com.android.externalstorage.documents/tree/primary%3ADocuments";

    // let fs = FsIo::new(db_path, root_uri).await.expect("Failed to initialize FsIo");
    // The cache is automatically built on the first run.
    // To refresh the cache later, you can call:
    // fs.refresh_cache().await.expect("Failed to refresh cache");
    # }
}
```

### Example

```rust
use fsio::{FsIo, FileIo};
use std::path::Path;

#[tokio::main]
async fn main() {
    // This example runs on non-Android platforms.
    # #[cfg(not(target_os = "android"))]
    # {
    let fs = FsIo::new();
    let dir_path = Path::new("/tmp/my_app");

    if !fs.exists(dir_path).await.unwrap() {
        fs.create_dir_all(dir_path).await.expect("Failed to create directory");
    }

    let file_path = dir_path.join("data.txt");
    let content = "Hello, world!";

    fs.write(&file_path, content.as_bytes()).await.expect("Failed to write file");

    let read_content = fs.read(&file_path).await.expect("Failed to read file");

    assert_eq!(content.as_bytes(), read_content.as_slice());
    println!("File written and read successfully!");

    fs.remove_dir_all(dir_path).await.expect("Failed to clean up");
    # }
}
```

## API Reference

The core functionality is defined by the `FileIo` trait.

```rust
#[async_trait]
pub trait FileIo: Send + Sync {
    async fn open(&self, path: &Path, open_mode: &str) -> Result<std::fs::File, FileIoError>;
    async fn read(&self, path: &Path) -> Result<Vec<u8>, FileIoError>;
    async fn write(&self, path: &Path, contents: &[u8]) -> Result<(), FileIoError>;
    async fn create_dir(&self, parent: &Path, name: &str) -> Result<PathBuf, FileIoError>;
    async fn create_dir_all(&self, path: &Path) -> Result<(), FileIoError>;
    async fn read_dir(&self, path: &Path) -> Result<Vec<FsNode>, FileIoError>;
    async fn remove_file(&self, path: &Path) -> Result<(), FileIoError>;
    async fn remove_dir_all(&self, path: &Path) -> Result<(), FileIoError>;
    async fn walk_dir(&self, path: &Path, follow_links: bool) -> Result<Vec<FsNode>, FileIoError>;
    async fn exists(&self, path: &Path) -> Result<bool, FileIoError>;
    async fn is_file(&self, path: &Path) -> Result<bool, FileIoError>;
    async fn is_dir(&self, path: &Path) -> Result<bool, FileIoError>;
}
```

## Android Implementation Details

- **Database Cache**: The `AndroidFsIo` implementation relies on an SQLite database with a single table (`fs_cache`) to map filesystem paths to content URIs.
- **Cache Columns**: `path` (TEXT, PRIMARY KEY), `content_url` (TEXT), `parent` (TEXT).
- **Initialization**: When `FsIo::new` is called on Android, it initializes the database and performs an initial scan of the SAF tree from the `root_uri` to populate the cache. This can take time on the first run.
- **`walk_dir`**: On Android, this operation queries the database cache directly rather than walking the SAF tree, making it significantly faster.
- **`refresh_cache`**: You can manually trigger a full refresh of the cache by calling the `refresh_cache` method on the `AndroidFsIo` instance if you expect the underlying filesystem has changed externally.
