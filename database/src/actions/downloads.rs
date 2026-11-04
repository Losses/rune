//! Device-local rules for keeping selected audio available offline.
use crate::actions::file::{MANAGED_AUDIO_DIRECTORY, blob_path_for_track, is_managed_directory};
use crate::actions::mixes::query_mix_media_files;
use crate::connection::RecommendationDbConnection;
use crate::sync::SeaOrmSyncStore;
use crate::sync::model::BlobRef;
use crate::{
    actions::mixes::executable_mix_queries,
    entities::{media_files, mixes, sync_download_jobs, sync_download_rules},
};
use anyhow::{Result, ensure};
use chrono::{DateTime, SecondsFormat, Utc};
use fsio::FsIo;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait,
    sea_query::OnConflict,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub fn validate_queries(queries: &[(String, String)]) -> Result<()> {
    let mut selects = false;
    for (operator, parameter) in queries {
        let valid = match operator.as_str() {
            "lib::all" => {
                let value = parameter.parse::<bool>()?;
                selects |= value;
                true
            }
            "lib::artist" | "lib::album" | "lib::genre" | "lib::playlist" | "lib::track"
            | "lib::random" => {
                selects = true;
                parameter.parse::<i32>().is_ok_and(|n| n > 0)
            }
            "lib::queue" => {
                let value = parameter.parse::<bool>()?;
                selects |= value;
                true
            }
            "lib::directory.deep" | "lib::directory.shallow" => {
                selects = true;
                !parameter.is_empty()
            }
            "filter::liked" | "filter::analyzed" | "filter::with_cover_art" => {
                selects = true;
                parameter.parse::<bool>().is_ok()
            }
            "sort::track_number"
            | "sort::last_modified"
            | "sort::duration"
            | "sort::playedthrough"
            | "sort::skipped" => parameter.parse::<bool>().is_ok(),
            "pipe::limit" => parameter.parse::<u64>().is_ok(),
            "pipe::recommend" => {
                selects = true;
                parameter.parse::<i32>().is_ok()
            }
            _ => false,
        };
        ensure!(valid, "invalid offline query operator or parameter");
    }
    ensure!(selects, "offline query has no selection");
    Ok(())
}

/// Record or disable a local offline rule. A saved mix is evaluated again each run.
pub async fn set_download_rule(
    db: &DatabaseConnection,
    queries: Vec<(String, String)>,
    mix_id: Option<i32>,
    enabled: bool,
) -> Result<String> {
    ensure!(
        mix_id.is_some() || !queries.is_empty(),
        "offline rule must select content"
    );
    ensure!(
        queries.len() <= 128
            && queries
                .iter()
                .all(|(op, value)| !op.is_empty() && op.len() <= 256 && value.len() <= 4096),
        "invalid offline rule"
    );
    if let Some(id) = mix_id {
        ensure!(id > 0, "invalid mix id");
    }
    if mix_id.is_none() {
        validate_queries(&queries)?;
    }
    let mix_uuid = if let Some(id) = mix_id {
        Some(
            mixes::Entity::find_by_id(id)
                .one(db)
                .await?
                .ok_or_else(|| anyhow::anyhow!("mix does not exist"))?
                .hlc_uuid,
        )
    } else {
        None
    };
    let stored = if mix_uuid.is_some() {
        Vec::new()
    } else {
        queries
    };
    let serialized = serde_json::to_string(&stored)?;
    let identity = serde_json::to_vec(&(&mix_uuid, &stored))?;
    let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, &identity).to_string();
    sync_download_rules::Entity::insert(sync_download_rules::ActiveModel {
        id: Set(id.clone()),
        queries: Set(serialized),
        mix_id: Set(mix_uuid),
        enabled: Set(enabled),
    })
    .on_conflict(
        OnConflict::column(sync_download_rules::Column::Id)
            .update_column(sync_download_rules::Column::Enabled)
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(id)
}

pub async fn remove_download_rule(db: &DatabaseConnection, rule_id: &str) -> Result<()> {
    sync_download_rules::Entity::delete_by_id(rule_id.to_owned())
        .exec(db)
        .await?;
    Ok(())
}

#[derive(Debug, Default)]
pub struct CleanupReport {
    pub removed: usize,
    pub delete_failures: usize,
}

pub async fn cleanup_unselected_downloads(
    db: &DatabaseConnection,
    recommend_db: &RecommendationDbConnection,
    fsio: &FsIo,
    root: &Path,
    store: &SeaOrmSyncStore,
    playback_queue: &[i32],
) -> Result<CleanupReport> {
    let rules = sync_download_rules::Entity::find()
        .filter(sync_download_rules::Column::Enabled.eq(true))
        .order_by_asc(sync_download_rules::Column::Id)
        .all(db)
        .await?;
    let mut selected = HashSet::new();
    const PAGE: usize = 100;
    for rule in rules {
        let queries = rule_queries(db, &rule).await?;
        if queries.is_empty() {
            continue;
        }
        validate_queries(&queries)?;
        let mut cursor = 0;
        loop {
            let files = query_mix_media_files(
                db,
                recommend_db,
                queries.clone(),
                cursor,
                PAGE,
                playback_queue,
            )
            .await?;
            let count = files.len();
            selected.extend(files.into_iter().map(|file| file.id));
            cursor = cursor
                .checked_add(count)
                .ok_or_else(|| anyhow::anyhow!("offline query cursor overflow"))?;
            if count < PAGE {
                break;
            }
        }
    }
    let rows = media_files::Entity::find()
        .filter(media_files::Column::LocalAvailable.eq(true))
        .all(db)
        .await?;
    let canonical_root = fsio.canonicalize_path(root)?;
    let managed = fsio.canonicalize_path(&root.join(MANAGED_AUDIO_DIRECTORY))?;
    ensure!(
        managed == canonical_root.join(MANAGED_AUDIO_DIRECTORY),
        "managed download directory escapes the library"
    );
    let mut report = CleanupReport::default();
    for row in rows {
        if selected.contains(&row.id) || !is_managed_directory(&row.directory) {
            continue;
        }
        let path = root.join(&row.directory).join(&row.file_name);
        // Only forget rows whose file provably lives inside the managed directory.
        let contained = fsio
            .canonicalize_path(&path)
            .is_ok_and(|canonical| canonical.starts_with(&managed) && canonical != managed);
        if !contained {
            log::warn!(
                "Skipping download outside the managed directory: {}",
                path.display()
            );
            report.delete_failures += 1;
            continue;
        }
        let tx = db.begin().await?;
        store.lock(&tx).await?;
        let current = media_files::Entity::find_by_id(row.id).one(&tx).await?;
        if current.as_ref() != Some(&row) {
            tx.commit().await?;
            continue;
        }
        media_files::Entity::update_many()
            .col_expr(media_files::Column::LocalAvailable, Expr::value(false))
            .col_expr(media_files::Column::Directory, Expr::value(""))
            .col_expr(media_files::Column::FileName, Expr::value(""))
            .filter(media_files::Column::Id.eq(row.id))
            .exec(&tx)
            .await?;
        sync_download_jobs::Entity::delete_many()
            .filter(sync_download_jobs::Column::FileUuid.eq(&row.hlc_uuid))
            .exec(&tx)
            .await?;
        tx.commit().await?;
        report.removed += 1;
        if let Err(error) = fsio.remove_file(&path).await {
            log::warn!(
                "Could not delete downloaded file {}: {error:#}",
                path.display()
            );
            report.delete_failures += 1;
        }
    }
    Ok(report)
}

/// Persist a download job before publishing bytes into the managed directory.
pub async fn prepare_download(
    db: &DatabaseConnection,
    fsio: &FsIo,
    root: &Path,
    track_id: Uuid,
    blob: &BlobRef,
    extension: &str,
) -> Result<PathBuf> {
    let path = blob_path_for_track(fsio, root, track_id, &blob.digest, extension)?;
    #[cfg(not(target_os = "android"))]
    let canonical_root = fsio.canonicalize_path(root)?;
    #[cfg(target_os = "android")]
    let canonical_root = root.to_path_buf();
    let relative = path.strip_prefix(&canonical_root)?;
    let directory = relative
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_string_lossy()
        .into_owned();
    let filename = relative
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("missing audio filename"))?
        .to_string_lossy()
        .into_owned();
    let job_id = Uuid::new_v5(&track_id, blob.digest.as_bytes()).to_string();
    sync_download_jobs::Entity::insert(sync_download_jobs::ActiveModel {
        id: Set(job_id),
        directory: Set(directory.clone()),
        file_name: Set(filename.clone()),
        file_uuid: Set(track_id.to_string()),
        digest: Set(blob.digest.clone()),
        size: Set(i64::try_from(blob.size)?),
        state: Set("pending".into()),
        attempts: Set(0),
        retry_at: Set(None),
        retry_source: Set(None),
        last_error: Set(None),
    })
    .on_conflict(
        OnConflict::columns([
            sync_download_jobs::Column::FileUuid,
            sync_download_jobs::Column::Digest,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    let row = sync_download_jobs::Entity::find()
        .filter(sync_download_jobs::Column::FileUuid.eq(track_id.to_string()))
        .filter(sync_download_jobs::Column::Digest.eq(&blob.digest))
        .one(db)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing download job"))?;
    ensure!(
        row.file_uuid == track_id.to_string()
            && row.digest == blob.digest
            && row.size == i64::try_from(blob.size)?
            && row.directory == directory
            && row.file_name == filename,
        "another track import owns this path"
    );
    Ok(path)
}

/// Counts requested files and those fetched or unavailable from this peer.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DownloadReport {
    pub requested: usize,
    pub downloaded: usize,
    pub pending: usize,
    pub failed_rules: usize,
    pub failed: usize,
    pub deadline_reached: bool,
}

pub async fn rule_queries(
    db: &DatabaseConnection,
    rule: &sync_download_rules::Model,
) -> Result<Vec<(String, String)>> {
    if let Some(id) = &rule.mix_id {
        let mix = mixes::Entity::find()
            .filter(mixes::Column::HlcUuid.eq(id))
            .one(db)
            .await?
            .ok_or_else(|| anyhow::anyhow!("download rule references a missing mix"))?;
        executable_mix_queries(db, mix.id).await
    } else {
        Ok(serde_json::from_str(&rule.queries)?)
    }
}

pub async fn download_job(
    db: &DatabaseConnection,
    track: Uuid,
    digest: &str,
) -> Result<Option<sync_download_jobs::Model>> {
    Ok(sync_download_jobs::Entity::find()
        .filter(sync_download_jobs::Column::FileUuid.eq(track.to_string()))
        .filter(sync_download_jobs::Column::Digest.eq(digest))
        .one(db)
        .await?)
}

pub fn retry_is_due(job: &sync_download_jobs::Model, server: &str) -> bool {
    // A different authorized source may have the missing bytes immediately.
    if job
        .retry_source
        .as_deref()
        .is_some_and(|source| source != server)
    {
        return true;
    }
    job.retry_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .is_none_or(|retry| retry <= Utc::now())
}

pub async fn record_download_failure(
    db: &DatabaseConnection,
    track: Uuid,
    blob: &BlobRef,
    server: &str,
    error: &anyhow::Error,
) -> Result<()> {
    let Some(job) = download_job(db, track, &blob.digest).await? else {
        // Preparation can fail before a destination can be reserved. The enabled
        // selection remains durable and will evaluate this item on the next pass.
        log::warn!("Could not prepare selected audio {track}: {error:#}");
        return Ok(());
    };
    let attempts = job.attempts.saturating_add(1).max(1);
    let delay = (5_i64 << attempts.min(6)).min(300);
    let retry = (Utc::now() + chrono::Duration::seconds(delay))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    sync_download_jobs::Entity::update_many()
        .col_expr(sync_download_jobs::Column::State, Expr::value("pending"))
        .col_expr(sync_download_jobs::Column::Attempts, Expr::value(attempts))
        .col_expr(sync_download_jobs::Column::RetryAt, Expr::value(retry))
        .col_expr(sync_download_jobs::Column::RetrySource, Expr::value(server))
        .col_expr(
            sync_download_jobs::Column::LastError,
            Expr::value(format!("{error:#}")),
        )
        .filter(sync_download_jobs::Column::Id.eq(job.id))
        .exec(db)
        .await?;
    Ok(())
}

pub async fn release_obsolete_reservations(db: &DatabaseConnection) -> Result<()> {
    use sea_orm::QuerySelect;
    let mut after = String::new();
    loop {
        let jobs = sync_download_jobs::Entity::find()
            .filter(sync_download_jobs::Column::Id.gt(&after))
            .order_by_asc(sync_download_jobs::Column::Id)
            .limit(128)
            .all(db)
            .await?;
        if jobs.is_empty() {
            return Ok(());
        }
        for job in jobs {
            after = job.id.clone();
            let tx = db.begin().await?;
            let file = media_files::Entity::find()
                .filter(media_files::Column::HlcUuid.eq(&job.file_uuid))
                .one(&tx)
                .await?;
            let current = file.is_some_and(|file| {
                file.content_hash.as_deref() == Some(job.digest.as_str())
                    && file.content_size == Some(job.size)
            });
            if !current {
                sync_download_jobs::Entity::delete_by_id(job.id)
                    .exec(&tx)
                    .await?;
            }
            tx.commit().await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::validate_queries;

    #[test]
    fn offline_selection_accepts_recommendation_and_rejects_unbounded_typos() {
        assert!(validate_queries(&[("pipe::recommend".into(), "-1".into())]).is_ok());
        assert!(validate_queries(&[("lib::track".into(), "12".into())]).is_ok());
        assert!(validate_queries(&[("lib::trak".into(), "12".into())]).is_err());
        assert!(validate_queries(&[("sort::duration".into(), "true".into())]).is_err());
    }
}
