//! Bounded local verification of managed audio, independent of metadata authorship.
use super::{access::materialized_path, content::MANAGED_AUDIO_DIRECTORY};
use crate::entities::media_files;
use anyhow::{Result, ensure};
use fsio::{FileStream, FsIo};
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    sea_query::Expr,
};
use std::{io::Read, path::Path};

struct Pending {
    row: media_files::Model,
    input: Box<dyn FileStream>,
    hash: blake3::Hasher,
    bytes: u64,
}

/// The cursor is local work only; restart re-verifies an unfinished file.
#[derive(Default)]
pub struct ManagedAudioVerifier {
    after: i32,
    pending: Option<Pending>,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct VerificationStep {
    pub bytes_read: u64,
    pub verified: bool,
    pub invalidated: bool,
    pub cycle_complete: bool,
}

async fn invalidate(db: &DatabaseConnection, row: &media_files::Model) -> Result<bool> {
    // An assignment or attachment committed during inspection wins over stale work.
    let result = media_files::Entity::update_many()
        .col_expr(media_files::Column::LocalAvailable, Expr::value(false))
        .filter(media_files::Column::Id.eq(row.id))
        .filter(media_files::Column::HlcUuid.eq(&row.hlc_uuid))
        .filter(media_files::Column::LocalAvailable.eq(true))
        .filter(media_files::Column::Directory.eq(&row.directory))
        .filter(media_files::Column::FileName.eq(&row.file_name))
        .filter(media_files::Column::ContentHash.eq(row.content_hash.clone()))
        .filter(media_files::Column::ContentSize.eq(row.content_size))
        .exec(db)
        .await?;
    Ok(result.rows_affected != 0)
}

impl ManagedAudioVerifier {
    /// Read at most the given budget; buffers remain 64 KiB for large objects.
    /// Only availability is changed. The normal selected-download queue repairs it.
    pub async fn step(
        &mut self,
        db: &DatabaseConnection,
        fsio: &FsIo,
        root: &Path,
        max_bytes: u64,
    ) -> Result<VerificationStep> {
        ensure!(
            (1..=16 * 1024 * 1024).contains(&max_bytes),
            "invalid managed verification budget"
        );
        let mut result = VerificationStep::default();
        if self.pending.is_none() {
            let row = media_files::Entity::find()
                .filter(media_files::Column::Id.gt(self.after))
                .filter(media_files::Column::LocalAvailable.eq(true))
                .filter(media_files::Column::ContentHash.is_not_null())
                .filter(media_files::Column::ContentSize.is_not_null())
                .filter(
                    Condition::any()
                        .add(media_files::Column::Directory.eq(MANAGED_AUDIO_DIRECTORY))
                        .add(
                            media_files::Column::Directory
                                .starts_with(format!("{MANAGED_AUDIO_DIRECTORY}/")),
                        )
                        .add(
                            media_files::Column::Directory
                                .starts_with(format!("{MANAGED_AUDIO_DIRECTORY}\\")),
                        ),
                )
                .order_by_asc(media_files::Column::Id)
                .limit(1)
                .one(db)
                .await?;
            let Some(row) = row else {
                self.after = 0;
                result.cycle_complete = true;
                return Ok(result);
            };
            self.after = row.id;
            let input = async {
                let path = materialized_path(db, fsio, root, row.id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("managed audio is missing"))?;
                Ok::<_, anyhow::Error>(fsio.open(&path, "r")?)
            }
            .await;
            match input {
                Ok(input) => {
                    self.pending = Some(Pending {
                        row,
                        input,
                        hash: blake3::Hasher::new(),
                        bytes: 0,
                    })
                }
                Err(error) => {
                    log::warn!("Managed audio {} is unavailable: {error:#}", row.hlc_uuid);
                    result.invalidated = invalidate(db, &row).await?;
                    return Ok(result);
                }
            }
        }
        let pending = self
            .pending
            .as_mut()
            .expect("verification input initialized");
        let mut buffer = [0_u8; 64 * 1024];
        while result.bytes_read < max_bytes {
            let limit = usize::try_from((max_bytes - result.bytes_read).min(buffer.len() as u64))?;
            let read = pending.input.read(&mut buffer[..limit]);
            let finished = match read {
                Ok(0) => Some(
                    pending.row.content_size == i64::try_from(pending.bytes).ok()
                        && pending.row.content_hash.as_deref()
                            == Some(pending.hash.finalize().to_hex().as_str()),
                ),
                Ok(n) => {
                    pending.bytes += n as u64;
                    result.bytes_read += n as u64;
                    pending.hash.update(&buffer[..n]);
                    if pending.bytes > pending.row.content_size.unwrap_or(-1).max(0) as u64 {
                        Some(false)
                    } else {
                        None
                    }
                }
                Err(error) => {
                    log::warn!(
                        "Managed audio {} read failed: {error}",
                        pending.row.hlc_uuid
                    );
                    Some(false)
                }
            };
            if let Some(valid) = finished {
                result.verified = valid;
                if !valid {
                    result.invalidated = invalidate(db, &pending.row).await?;
                }
                self.pending = None;
                break;
            }
            tokio::task::yield_now().await;
        }
        Ok(result)
    }
}
