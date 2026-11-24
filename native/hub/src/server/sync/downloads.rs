//! Network execution for device-local download rules.
use super::blobs::download_track_blob;
use anyhow::{Context, Result, ensure};
use database::{
    actions::{
        downloads::{
            DownloadReport, download_job, prepare_download, record_download_failure,
            release_obsolete_reservations, retry_is_due, rule_queries, validate_queries,
        },
        file::{attach_download, catalog_track, is_materialized},
        mixes::query_mix_media_files_with_seed,
    },
    connection::RecommendationDbConnection,
    entities::sync_download_rules,
    sync::{LibrarySyncStore, model::RuneModel},
};
use fsio::FsIo;
use http_request::sync::MetadataPeer;
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder};
use std::time::Duration;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};
use sync::SyncPeer;
use tokio::time as tokio_time;
use tokio::time::Instant;
use uuid::Uuid;

pub struct DownloadAudioContext<'a> {
    pub node_id: Uuid,
    pub remote: &'a MetadataPeer<RuneModel>,
    pub notify: Arc<dyn Fn() + Send + Sync>,
    pub playback_queue: &'a [i32],
}

/// Keeps rule selections and attempted file IDs for the current download sweep.
#[derive(Default)]
pub struct DownloadProgress {
    rules: Vec<sync_download_rules::Model>,
    queries: HashMap<String, Vec<(String, String)>>,
    random_seeds: HashMap<String, u64>,
    attempted: HashSet<i32>,
}

/// Describes why a bounded download pass returned.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum DownloadPassOutcome {
    /// The sweep ran to completion.
    #[default]
    Complete,
    /// The deadline prevented another transfer from starting.
    Deadline,
    /// The pass used its inspection and transfer budget.
    AttemptLimit,
    /// A rule query failed and the worker should retry with backoff.
    RuleQueryFailed,
}

/// The counts and stopping reason for one pass through the sweep.
#[derive(Debug, Default)]
pub struct DownloadAudioPass {
    /// Download, inspection, and rule counts and any limit reached in this pass.
    pub report: DownloadReport,
    /// Disposition for the worker, with rule-query failures taking precedence.
    pub outcome: DownloadPassOutcome,
}

impl DownloadAudioPass {
    fn finish(
        report: DownloadReport,
        outcome: DownloadPassOutcome,
        rule_query_failed: bool,
    ) -> Self {
        let outcome = if rule_query_failed {
            DownloadPassOutcome::RuleQueryFailed
        } else {
            outcome
        };
        Self { report, outcome }
    }
}

/// Evaluate enabled rules and fetch only missing matching audio from this peer.
pub async fn download_requested_audio(
    db: DatabaseConnection,
    recommend_db: &RecommendationDbConnection,
    fsio: Arc<FsIo>,
    root: PathBuf,
    node_id: Uuid,
    remote: &MetadataPeer<RuneModel>,
    notify: Arc<dyn Fn() + Send + Sync>,
) -> Result<DownloadReport> {
    let context = DownloadAudioContext {
        node_id,
        remote,
        notify,
        playback_queue: &[],
    };
    download_requested_audio_until(
        db,
        recommend_db,
        fsio,
        root,
        context,
        Instant::now() + Duration::from_secs(60),
    )
    .await
}

/// Run one bounded pass, allowing an in-flight transfer to finish.
pub async fn download_requested_audio_until(
    db: DatabaseConnection,
    recommend_db: &RecommendationDbConnection,
    fsio: Arc<FsIo>,
    root: PathBuf,
    context: DownloadAudioContext<'_>,
    deadline: tokio_time::Instant,
) -> Result<DownloadReport> {
    download_audio_pass(
        db,
        recommend_db,
        fsio,
        root,
        context,
        deadline,
        &mut DownloadProgress::default(),
    )
    .await
    .map(|pass| pass.report)
}

/// Resume a bounded pass, skipping files already attempted in this sweep.
pub async fn download_audio_pass(
    db: DatabaseConnection,
    recommend_db: &RecommendationDbConnection,
    fsio: Arc<FsIo>,
    root: PathBuf,
    context: DownloadAudioContext<'_>,
    deadline: Instant,
    progress: &mut DownloadProgress,
) -> Result<DownloadAudioPass> {
    release_obsolete_reservations(&db).await?;
    let rules = sync_download_rules::Entity::find()
        .filter(sync_download_rules::Column::Enabled.eq(true))
        .order_by_asc(sync_download_rules::Column::Id)
        .all(&db)
        .await?;
    if rules != progress.rules {
        *progress = DownloadProgress {
            rules,
            ..DownloadProgress::default()
        };
    }
    if progress.rules.is_empty() {
        return Ok(DownloadAudioPass::default());
    }
    let server = context.remote.base_url();
    let client = context.remote.client();
    let identity = context.remote.identity().await?;
    ensure!(
        !identity.namespace.is_nil() && !identity.replica.is_nil() && !identity.generation.is_nil(),
        "invalid peer identity"
    );
    let local =
        LibrarySyncStore::open(db.clone(), context.node_id, Some(identity.namespace)).await?;
    let mut report = DownloadReport::default();
    let mut rule_query_failed = false;
    let mut selected = Vec::new();
    let mut next_queries = progress.queries.clone();
    let mut changed = false;
    for rule in &progress.rules {
        let queries = match rule_queries(&db, rule).await {
            Ok(queries) => queries,
            Err(error) => {
                log::warn!("Could not evaluate offline rule {}: {error:#}", rule.id);
                report.failed_rules += 1;
                if error.downcast_ref::<serde_json::Error>().is_none() {
                    rule_query_failed = true;
                }
                if error.downcast_ref::<DbErr>().is_none() {
                    next_queries.remove(&rule.id);
                }
                continue;
            }
        };
        if next_queries.get(&rule.id) != Some(&queries) {
            changed = true;
            next_queries.insert(rule.id.clone(), queries.clone());
        }
        if queries.is_empty() {
            continue;
        }
        if let Err(error) = validate_queries(&queries) {
            log::warn!("Skipping invalid offline rule: {error}");
            report.failed_rules += 1;
            continue;
        }
        selected.push((rule.id.as_str(), queries));
    }
    // Publish resolved queries and invalidate their dependent state together.
    progress.queries = next_queries;
    if changed {
        progress.attempted.clear();
        progress.random_seeds.clear();
    }

    const PAGE: usize = 100;
    const MAX_ATTEMPTS: usize = 128;
    let attempted = &mut progress.attempted;
    let attempted_before = attempted.len();
    for (rule_id, queries) in selected {
        let random_seed = queries
            .iter()
            .any(|(operator, _)| operator == "lib::random")
            .then(|| {
                *progress
                    .random_seeds
                    .entry(rule_id.to_owned())
                    .or_insert_with(rand::random)
            });
        let mut cursor = 0usize;
        loop {
            let files = match query_mix_media_files_with_seed(
                &db,
                recommend_db,
                queries.clone(),
                cursor,
                PAGE,
                context.playback_queue,
                random_seed,
            )
            .await
            {
                Ok(files) => files,
                Err(error) => {
                    log::warn!("Could not query offline rule {rule_id}: {error:#}");
                    report.failed_rules += 1;
                    rule_query_failed = true;
                    break;
                }
            };
            let count = files.len();
            if count == 0 {
                break;
            }
            for file in files {
                let file_id = file.id;
                if attempted.contains(&file_id) {
                    continue;
                }
                // Inspection failures use the same budget as transfers.
                if attempted.len() - attempted_before >= MAX_ATTEMPTS {
                    return Ok(DownloadAudioPass::finish(
                        report,
                        DownloadPassOutcome::AttemptLimit,
                        rule_query_failed,
                    ));
                }
                let materialized = match is_materialized(&db, &fsio, &root, file_id).await {
                    Ok(value) => value,
                    Err(error) => {
                        log::warn!("Could not inspect file {}: {error:#}", file_id);
                        report.failed += 1;
                        attempted.insert(file_id);
                        continue;
                    }
                };
                if materialized {
                    continue;
                }
                let (track_id, blob, extension) = match catalog_track(&db, file_id).await {
                    Ok(Some(track)) => track,
                    Ok(None) => continue,
                    Err(error) => {
                        log::warn!("Could not look up file {}: {error:#}", file_id);
                        report.failed += 1;
                        attempted.insert(file_id);
                        continue;
                    }
                };
                let job = match download_job(&db, track_id, &blob.digest).await {
                    Ok(job) => job,
                    Err(error) => {
                        log::warn!("Could not look up download for file {}: {error:#}", file_id);
                        report.failed += 1;
                        attempted.insert(file_id);
                        continue;
                    }
                };
                if job.is_some_and(|job| !retry_is_due(&job, server)) {
                    report.pending += 1;
                    continue;
                }
                if Instant::now() >= deadline {
                    report.deadline_reached = true;
                    return Ok(DownloadAudioPass::finish(
                        report,
                        DownloadPassOutcome::Deadline,
                        rule_query_failed,
                    ));
                }
                report.requested += 1;
                let transfer = async {
                    // Keep the path reserved until attachment commits, including
                    // cancellation and failures after publication of verified bytes.
                    prepare_download(&db, &fsio, &root, track_id, &blob, &extension).await?;
                    let path = download_track_blob(
                        &fsio, client, server, &root, track_id, &blob, &extension,
                    )
                    .await?;
                    #[cfg(feature = "sync-crash-probe")]
                    super::fault_hook::stop("scanned");
                    attach_download(
                        &db,
                        &local.record_store(),
                        &fsio,
                        &root,
                        track_id,
                        &blob,
                        &path,
                    )
                    .await?;
                    #[cfg(feature = "sync-crash-probe")]
                    super::fault_hook::stop("attached");
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                attempted.insert(file_id);
                match transfer {
                    Ok(()) => {
                        report.downloaded += 1;
                        (context.notify)();
                    }
                    Err(error) => {
                        log::warn!("Download failed for file {}: {error:#}", file_id);
                        record_download_failure(&db, track_id, &blob, server, &error)
                            .await
                            .with_context(|| {
                                format!("could not record download failure for file {file_id}")
                            })?;
                        report.failed += 1;
                        report.pending += 1;
                    }
                }
            }
            cursor = cursor
                .checked_add(count)
                .ok_or_else(|| anyhow::anyhow!("offline query cursor overflow"))?;
            if count < PAGE {
                break;
            }
        }
    }
    if !rule_query_failed {
        *progress = DownloadProgress::default();
    }
    Ok(DownloadAudioPass::finish(
        report,
        DownloadPassOutcome::Complete,
        rule_query_failed,
    ))
}
