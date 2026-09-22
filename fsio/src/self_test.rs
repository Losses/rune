use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

use serde::{Deserialize, Serialize};

use super::{FileIoError, FsIo};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FsSelfTestLayerResult {
    pub layer: String,
    pub ok: bool,
    pub skipped: bool,
    pub elapsed_ms: u64,
    pub detail: String,
}

pub const LAYER_ROOT_READABLE: &str = "L2 Root directory access (read_dir)";
pub const LAYER_BASIC_ROUNDTRIP: &str = "L3 Basic read/write roundtrip";
pub const LAYER_STREAMING_IO: &str = "L4 Streaming I/O (8 MiB chunked)";
pub const LAYER_WALK_DIR: &str = "L5 walk_dir consistency";
pub const LAYER_DIRECT_READ: &str = "L6 Direct playback-path read (std::fs)";
pub const LAYER_DB_WRITE_PATH: &str = "L7 Database write path";

pub const ALL_LAYERS: [&str; 6] = [
    LAYER_ROOT_READABLE,
    LAYER_BASIC_ROUNDTRIP,
    LAYER_STREAMING_IO,
    LAYER_WALK_DIR,
    LAYER_DIRECT_READ,
    LAYER_DB_WRITE_PATH,
];

pub fn skipped_layer_results(reason: &str) -> Vec<FsSelfTestLayerResult> {
    ALL_LAYERS
        .iter()
        .map(|layer| FsSelfTestLayerResult {
            layer: layer.to_string(),
            ok: false,
            skipped: true,
            elapsed_ms: 0,
            detail: reason.to_string(),
        })
        .collect()
}

enum LayerStatus {
    Done(String),
    Skipped(String),
}

type LayerOutput = Result<LayerStatus, FileIoError>;

fn self_test_dir(root: &Path) -> PathBuf {
    root.join(".rune").join(".selftest")
}

fn pattern_byte(offset: usize) -> u8 {
    (offset % 251) as u8
}

async fn read_all(fsio: &FsIo, path: &Path) -> Result<Vec<u8>, FileIoError> {
    use std::io::Read;
    let mut file = fsio.open_async(path, "r").await?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).map_err(FileIoError::Io)?;
    Ok(buffer)
}

async fn layer_root_readable(fsio: &FsIo, root: &Path) -> LayerOutput {
    let entries = fsio.read_dir(root).await?;
    let dirs = entries.iter().filter(|n| n.is_dir).count();
    let files = entries.iter().filter(|n| n.is_file).count();
    Ok(LayerStatus::Done(format!(
        "read_dir succeeded: {} entries ({} dirs, {} files)",
        entries.len(),
        dirs,
        files
    )))
}

async fn layer_basic_roundtrip(fsio: &FsIo, root: &Path) -> LayerOutput {
    let dir = self_test_dir(root);
    fsio.create_dir_all(&dir)?;

    let file_path = dir.join("roundtrip.bin");
    let expected: Vec<u8> = (0..1024).map(pattern_byte).collect();

    let result = async {
        fsio.write(&file_path, &expected).await?;

        let actual = read_all(fsio, &file_path).await?;
        if actual != expected {
            return Err(FileIoError::Saf(format!(
                "content mismatch: wrote {} bytes, read back {} bytes",
                expected.len(),
                actual.len()
            )));
        }

        if !fsio.exists(&file_path)? {
            return Err(FileIoError::Saf(
                "exists() returned false for a freshly written file".to_string(),
            ));
        }

        let listed = fsio.read_dir(&dir).await?;
        if !listed.iter().any(|n| n.path == file_path) {
            return Err(FileIoError::Saf(
                "read_dir does not list the freshly written file".to_string(),
            ));
        }

        Ok(LayerStatus::Done(format!(
            "write/read/exists/read_dir roundtrip OK ({} bytes)",
            expected.len()
        )))
    }
    .await;

    let _ = fsio.remove_file(&file_path).await;

    result
}

async fn layer_streaming_io(fsio: &FsIo, root: &Path) -> LayerOutput {
    const CHUNK_SIZE: usize = 256 * 1024;
    const CHUNK_COUNT: usize = 32;

    let dir = self_test_dir(root);
    fsio.create_dir_all(&dir)?;
    let file_path = dir.join("stream.bin");

    let result = async {
        let write_start = Instant::now();
        {
            let mut file = fsio.open_async(&file_path, "wt").await?;
            for chunk_index in 0..CHUNK_COUNT {
                let chunk: Vec<u8> = (0..CHUNK_SIZE)
                    .map(|i| pattern_byte(chunk_index * CHUNK_SIZE + i))
                    .collect();
                file.write_all(&chunk).map_err(FileIoError::Io)?;
            }
            file.flush().map_err(FileIoError::Io)?;
        }
        let write_ms = write_start.elapsed().as_millis() as u64;

        let read_start = Instant::now();
        let actual = read_all(fsio, &file_path).await?;
        let read_ms = read_start.elapsed().as_millis() as u64;

        let expected_len = CHUNK_SIZE * CHUNK_COUNT;
        if actual.len() != expected_len {
            return Err(FileIoError::Saf(format!(
                "length mismatch: wrote {} bytes, read back {} bytes",
                expected_len,
                actual.len()
            )));
        }
        if let Some(offset) = actual
            .iter()
            .enumerate()
            .find(|(i, &b)| b != pattern_byte(*i))
            .map(|(i, _)| i)
        {
            return Err(FileIoError::Saf(format!(
                "content mismatch at byte offset {offset}"
            )));
        }

        Ok(LayerStatus::Done(format!(
            "{} MiB chunked write + readback verified (write {} ms, read {} ms)",
            expected_len / (1024 * 1024),
            write_ms,
            read_ms
        )))
    }
    .await;

    let _ = fsio.remove_file(&file_path).await;

    result
}

async fn layer_walk_dir(fsio: &FsIo, root: &Path) -> LayerOutput {
    let nodes = fsio.walk_dir(root, false)?;
    if nodes.is_empty() {
        return Ok(LayerStatus::Skipped(
            "walk_dir returned 0 entries; directory appears empty".to_string(),
        ));
    }

    let files = nodes.iter().filter(|n| n.is_file).count();
    let top_level = fsio.read_dir(root).await?.len();
    Ok(LayerStatus::Done(format!(
        "walk_dir succeeded: {} entries ({} files); read_dir reports {} top-level entries",
        nodes.len(),
        files,
        top_level
    )))
}

async fn layer_direct_read(fsio: &FsIo, root: &Path) -> LayerOutput {
    let nodes = fsio.walk_dir(root, false)?;
    let Some(target) = nodes.iter().find(|n| n.is_file) else {
        return Ok(LayerStatus::Skipped(
            "no file found in the directory; nothing to read".to_string(),
        ));
    };

    let canonical = fsio.canonicalize_path(&target.path)?;
    let prefix = format!(
        "canonicalized {} -> {}",
        target.path.display(),
        canonical.display()
    );

    match std::fs::File::open(&canonical) {
        Ok(mut file) => {
            use std::io::Read;
            let mut buffer = [0u8; 4096];
            match file.read(&mut buffer) {
                Ok(n) => Ok(LayerStatus::Done(format!(
                    "{prefix}; std::fs read {n} of 4096 probe bytes"
                ))),
                Err(e) => Err(FileIoError::Saf(format!(
                    "{prefix}; std::fs read failed: {e}; canonicalized path is not directly readable; playback layer (std::fs) cannot read this file directly, fd-based playback is required"
                ))),
            }
        }
        Err(e) => Err(FileIoError::Saf(format!(
            "{prefix}; std::fs open failed: {e}; canonicalized path is not directly readable; playback layer (std::fs) cannot read this file directly, fd-based playback is required"
        ))),
    }
}

async fn layer_db_write_path(fsio: &FsIo, root: &Path) -> LayerOutput {
    let rune_dir = root.join(".rune");
    fsio.ensure_directory(&rune_dir).await?;

    let dir = self_test_dir(root);
    fsio.create_dir_all(&dir)?;

    let file_path = dir.join("db_path_probe.txt");
    let payload = "rune fs self-test db write path probe";

    let result = async {
        fsio.write_string(&file_path, payload).await?;
        let actual = fsio.read_to_string(&file_path)?;
        if actual != payload {
            return Err(FileIoError::Saf(
                "write_string/read_to_string roundtrip mismatch".to_string(),
            ));
        }
        Ok(LayerStatus::Done(
            "ensure_directory(.rune) + write_string/read_to_string roundtrip OK".to_string(),
        ))
    }
    .await;

    let _ = fsio.remove_dir_all(&dir).await;

    result
}

macro_rules! run_layer {
    ($layer:expr, $func:ident, $fsio:expr, $root:expr, $on_layer_done:expr) => {{
        let start = Instant::now();
        let output = $func($fsio, $root).await;
        let elapsed_ms = start.elapsed().as_millis() as u64;

        let result = match output {
            Ok(LayerStatus::Done(detail)) => FsSelfTestLayerResult {
                layer: $layer.to_string(),
                ok: true,
                skipped: false,
                elapsed_ms,
                detail,
            },
            Ok(LayerStatus::Skipped(detail)) => FsSelfTestLayerResult {
                layer: $layer.to_string(),
                ok: false,
                skipped: true,
                elapsed_ms,
                detail,
            },
            Err(e) => FsSelfTestLayerResult {
                layer: $layer.to_string(),
                ok: false,
                skipped: false,
                elapsed_ms,
                detail: format!("{e}"),
            },
        };

        $on_layer_done(&result);
        result
    }};
}

pub async fn run_fs_self_test<F: FnMut(&FsSelfTestLayerResult)>(
    fsio: &FsIo,
    root: &Path,
    mut on_layer_done: F,
) -> Vec<FsSelfTestLayerResult> {
    let mut results = Vec::with_capacity(ALL_LAYERS.len());

    results.push(run_layer!(
        LAYER_ROOT_READABLE,
        layer_root_readable,
        fsio,
        root,
        on_layer_done
    ));
    results.push(run_layer!(
        LAYER_BASIC_ROUNDTRIP,
        layer_basic_roundtrip,
        fsio,
        root,
        on_layer_done
    ));
    results.push(run_layer!(
        LAYER_STREAMING_IO,
        layer_streaming_io,
        fsio,
        root,
        on_layer_done
    ));
    results.push(run_layer!(
        LAYER_WALK_DIR,
        layer_walk_dir,
        fsio,
        root,
        on_layer_done
    ));
    results.push(run_layer!(
        LAYER_DIRECT_READ,
        layer_direct_read,
        fsio,
        root,
        on_layer_done
    ));
    results.push(run_layer!(
        LAYER_DB_WRITE_PATH,
        layer_db_write_path,
        fsio,
        root,
        on_layer_done
    ));

    let dir = self_test_dir(root);
    if fsio.exists(&dir).unwrap_or(false) {
        let _ = fsio.remove_dir_all(&dir).await;
    }

    results
}
