use anyhow::{Context, Result, ensure};
use blake3::Hasher;
use database::{
    actions::file::{MAX_BLOB_BYTES, blob_path, blob_path_for_track, hash_file, validate_digest},
    sync::model::BlobRef,
};
use fsio::{FsIo, OpenMode};
use std::fs;
use std::time::Duration;
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use tokio::runtime::Handle;
use tokio::task;
use tokio::time as tokio_time;
use uuid::Uuid;
struct TempFile {
    fsio: FsIo,
    path: PathBuf,
}
impl Drop for TempFile {
    fn drop(&mut self) {
        let fsio = self.fsio.clone();
        let path = self.path.clone();
        if let Ok(handle) = Handle::try_current() {
            handle.spawn(async move {
                let _ = fsio.remove_file(&path).await;
            });
        }
    }
}

pub(super) struct VerifiedBlob {
    path: PathBuf,
    temporary: Option<TempFile>,
}

impl VerifiedBlob {
    pub(super) async fn publish(self) -> Result<PathBuf> {
        if let Some(temp) = self.temporary {
            if temp.fsio.exists(&self.path)? {
                temp.fsio.remove_file(&self.path).await?;
            }
            temp.fsio
                .rename_file(&temp.path, &self.path)
                .await
                .context("could not publish downloaded blob")?;
            #[cfg(feature = "sync-crash-probe")]
            super::fault_hook::stop("published");
        }
        Ok(self.path)
    }
}

/// Download to a temporary file and publish only after full size and digest verification.
pub async fn download_blob(
    fsio: &FsIo,
    client: &reqwest::Client,
    base_url: &str,
    root: &Path,
    blob: &BlobRef,
    extension: &str,
) -> Result<PathBuf> {
    let final_path = blob_path(fsio, root, &blob.digest, extension)?;
    fetch_to_path(fsio, client, base_url, blob, final_path)
        .await?
        .publish()
        .await
}

/// Verify audio for one track before publishing it into its managed location.
pub(super) async fn fetch_track_blob(
    fsio: &FsIo,
    client: &reqwest::Client,
    base_url: &str,
    root: &Path,
    track_id: Uuid,
    blob: &BlobRef,
    extension: &str,
) -> Result<VerifiedBlob> {
    let final_path = blob_path_for_track(fsio, root, track_id, &blob.digest, extension)?;
    fetch_to_path(fsio, client, base_url, blob, final_path).await
}

async fn fetch_to_path(
    fsio: &FsIo,
    client: &reqwest::Client,
    base_url: &str,
    blob: &BlobRef,
    final_path: PathBuf,
) -> Result<VerifiedBlob> {
    validate_digest(&blob.digest)?;
    ensure!(blob.size <= MAX_BLOB_BYTES, "blob too large");
    if fsio.exists(&final_path)? {
        ensure!(
            fsio.is_file(&final_path).await?,
            "invalid existing blob path"
        );
        #[cfg(not(target_os = "android"))]
        ensure!(
            !fs::symlink_metadata(&final_path)?.file_type().is_symlink(),
            "invalid existing blob path"
        );
        let hash_fsio = fsio.clone();
        let hash_path = final_path.clone();
        if task::spawn_blocking(move || hash_file(&hash_fsio, &hash_path))
            .await
            .context("existing blob verification task failed")??
            == *blob
        {
            return Ok(VerifiedBlob {
                path: final_path,
                temporary: None,
            });
        }
    }
    let temp = TempFile {
        fsio: fsio.clone(),
        path: final_path.with_extension(format!("part-{}", Uuid::new_v4())),
    };
    let result = async {
        let mut response = tokio_time::timeout(
            Duration::from_secs(30),
            client
                .get(format!(
                    "{}/sync/blobs/{}",
                    base_url.trim_end_matches('/'),
                    blob.digest
                ))
                .send(),
        )
        .await
        .context("blob response timed out")??
        .error_for_status()?;
        if let Some(length) = response.content_length() {
            ensure!(length == blob.size, "blob size mismatch");
        }
        let mut output = fsio.open(&temp.path, OpenMode::WriteTruncate)?;
        let mut size = 0u64;
        let mut hash = Hasher::new();
        while let Some(chunk) = tokio_time::timeout(Duration::from_secs(30), response.chunk())
            .await
            .context("blob transfer stalled")??
        {
            size = size
                .checked_add(chunk.len() as u64)
                .context("blob size overflow")?;
            ensure!(size <= blob.size, "blob exceeds expected size");
            output.write_all(&chunk)?;
            hash.update(&chunk);
            #[cfg(feature = "sync-crash-probe")]
            if size < blob.size {
                output.flush()?;
                super::fault_hook::stop("partial");
            }
        }
        output.flush()?;
        output.sync_all()?;
        drop(output);
        ensure!(
            size == blob.size && hash.finalize().to_hex().as_str() == blob.digest,
            "blob verification failed"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    result?;
    Ok(VerifiedBlob {
        path: final_path,
        temporary: Some(temp),
    })
}
