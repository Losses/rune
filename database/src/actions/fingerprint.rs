mod source;
pub use source::{
    ComputedFingerprint, ComputedSimilarity, SourceStatus, assign_computed_fingerprint,
    assign_computed_similarity, assign_duplicate_mark, current_fingerprint,
    default_similarity_configuration, fingerprint_status, similarity_status,
    unavailable_fingerprints, unavailable_similarities,
};

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use fsio::FsIo;
use log::info;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub use tag_editor::music_brainz::fingerprint::{Configuration, Segment};
use tag_editor::music_brainz::fingerprint::{
    calc_fingerprint, calculate_similarity_score, get_track_duration_in_secs, match_fingerprints,
};

use crate::entities::prelude::{MediaFileFingerprint, MediaFileSimilarity, MediaFiles};
use crate::entities::{media_file_fingerprint, media_file_similarity, media_files};

fn compute_single_fingerprint(
    fsio: &FsIo,
    lib_path: &Path,
    file: &media_files::Model,
    config: &Configuration,
    cancel_token: Option<CancellationToken>,
) -> Result<(Vec<u32>, Duration)> {
    let file_path = lib_path.join(&file.directory).join(&file.file_name);

    info!("Computing fingerprint for: {}", file.file_name);

    if let Some(token) = &cancel_token
        && token.is_cancelled()
    {
        return Err(anyhow!("Operation cancelled"));
    }

    anyhow::ensure!(
        file.local_available,
        "fingerprint source is not locally available"
    );
    let expected = match (&file.content_hash, file.content_size) {
        (Some(hash), Some(size)) if size >= 0 => crate::sync::model::BlobRef {
            digest: hash.clone(),
            size: size as u64,
        },
        _ => bail!("fingerprint source has no verified content identity"),
    };
    anyhow::ensure!(
        crate::actions::file::hash_file(fsio, &file_path)? == expected,
        "fingerprint source changed before computation"
    );
    let result = calc_fingerprint(fsio, &file_path, config)
        .with_context(|| format!("compute fingerprint for: {}", file_path.display()))?;
    anyhow::ensure!(
        crate::actions::file::hash_file(fsio, &file_path)? == expected,
        "fingerprint source changed during computation"
    );
    Ok(result)
}

pub async fn has_fingerprint(main_db: &DatabaseConnection, file_id: i32) -> Result<bool> {
    let Some(file) = MediaFiles::find_by_id(file_id).one(main_db).await? else {
        return Ok(false);
    };
    Ok(media_file_fingerprint::Entity::find()
        .filter(media_file_fingerprint::Column::TrackUuid.eq(file.hlc_uuid))
        .exists(main_db)
        .await?)
}

pub async fn get_fingerprint_count(main_db: &DatabaseConnection) -> Result<u64> {
    Ok(media_file_fingerprint::Entity::find()
        .count(main_db)
        .await?)
}

pub fn bytes_to_u32s(bytes: Vec<u8>) -> Result<Vec<u32>> {
    if !bytes.len().is_multiple_of(4) {
        bail!("The length of the input byte vector must be a multiple of 4.".to_string());
    }

    let mut u32s = Vec::new();
    for chunk in bytes.chunks(4) {
        // Use try_into to convert the byte slice to a [u8; 4] array
        let byte_array: [u8; 4] = match chunk.try_into() {
            Ok(arr) => arr,
            Err(_) => {
                // Theoretically, chunks_exact guarantees a length of 4, so this error should not occur
                bail!("Internal error: byte chunk is not 4 bytes.".to_string());
            }
        };

        // Create u32 from little-endian byte array
        let u32_value = u32::from_le_bytes(byte_array);
        u32s.push(u32_value);
    }

    Ok(u32s)
}

fn group_similar_files(similarities: &[media_file_similarity::Model]) -> Vec<Vec<String>> {
    let mut adjacency_list: HashMap<String, Vec<String>> = HashMap::new();

    // Build an adjacency list for our similarity graph
    for similarity in similarities {
        adjacency_list
            .entry(similarity.first_track_uuid.clone())
            .or_default()
            .push(similarity.second_track_uuid.clone());
        adjacency_list
            .entry(similarity.second_track_uuid.clone())
            .or_default()
            .push(similarity.first_track_uuid.clone());
    }

    // Use a set to track visited nodes during our search
    let mut visited = HashSet::new();
    let mut groups = Vec::new();

    // Perform a depth-first search to find connected components (groups)
    for file_id in adjacency_list.keys().cloned() {
        if visited.contains(&file_id) {
            continue;
        }

        let mut group = Vec::new();
        let mut stack = vec![file_id.clone()];
        visited.insert(file_id);

        while let Some(current_id) = stack.pop() {
            group.push(current_id.clone());

            if let Some(neighbors) = adjacency_list.get(&current_id) {
                for neighbor in neighbors.iter().cloned() {
                    if !visited.contains(&neighbor) {
                        visited.insert(neighbor.clone());
                        stack.push(neighbor);
                    }
                }
            }
        }

        if group.len() > 1 {
            groups.push(group);
        }
    }

    groups
}

// Function to get all duplicated files
pub async fn get_duplicate_files(db: &DatabaseConnection) -> Result<Vec<media_files::Model>> {
    let mut duplicates = Vec::new();
    for file in MediaFiles::find().all(db).await? {
        if let Some(row) = crate::actions::fingerprint::current_fingerprint(db, &file).await?
            && row.is_duplicated == 1
            && crate::actions::fingerprint::fingerprint_status(&row, &file)
                == crate::actions::fingerprint::SourceStatus::Verified
        {
            duplicates.push(file);
        }
    }
    Ok(duplicates)
}

// Function to reset duplicate marks
/// Compute new fingerprints and commit their source rows with assignments.
pub async fn compute_file_fingerprints<F>(
    fsio: Arc<FsIo>,
    db: &DatabaseConnection,
    root: &Path,
    batch_size: usize,
    progress: F,
    cancel: Option<CancellationToken>,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<usize>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    anyhow::ensure!(batch_size > 0, "batch size must be positive");
    let query = MediaFiles::find().filter(media_files::Column::LocalAvailable.eq(true));
    let total = query.clone().count(db).await? as usize;
    let mut cursor = 0;
    let mut processed = 0;
    loop {
        let rows = query
            .clone()
            .filter(media_files::Column::Id.gt(cursor))
            .order_by_asc(media_files::Column::Id)
            .limit(batch_size as u64)
            .all(db)
            .await?;
        if rows.is_empty() {
            break;
        }
        for file in rows {
            cursor = file.id;
            if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                return Ok(processed);
            }
            if let Some(current) =
                crate::actions::fingerprint::current_fingerprint(db, &file).await?
                && crate::actions::fingerprint::fingerprint_status(&current, &file)
                    == crate::actions::fingerprint::SourceStatus::Verified
            {
                processed += 1;
                progress(processed, total);
                continue;
            }
            let path_for_hash = root.join(&file.directory).join(&file.file_name);
            let expected = match (&file.content_hash, file.content_size) {
                (Some(hash), Some(size)) if size >= 0 => crate::sync::model::BlobRef {
                    digest: hash.clone(),
                    size: size as u64,
                },
                _ => {
                    log::warn!("fingerprint source identity unavailable: {}", file.id);
                    processed += 1;
                    progress(processed, total);
                    continue;
                }
            };
            if crate::actions::file::hash_file(&fsio, &path_for_hash)
                .ok()
                .as_ref()
                != Some(&expected)
            {
                log::warn!(
                    "fingerprint source content changed before computation: {}",
                    file.id
                );
                processed += 1;
                progress(processed, total);
                continue;
            }
            let fs = fsio.clone();
            let path = root.to_path_buf();
            let observed = file.clone();
            let token = cancel.clone();
            let result = tokio::task::spawn_blocking(move || {
                compute_single_fingerprint(
                    fs.as_ref(),
                    &path,
                    &observed,
                    &Configuration::default(),
                    token,
                )
            })
            .await?;
            match result {
                Ok((fingerprint, _)) => {
                    if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                        return Ok(processed);
                    }
                    if crate::actions::file::hash_file(&fsio, &path_for_hash)
                        .ok()
                        .as_ref()
                        != Some(&expected)
                    {
                        log::warn!(
                            "fingerprint source content changed during computation: {}",
                            file.id
                        );
                        processed += 1;
                        progress(processed, total);
                        continue;
                    }
                    let bytes: Vec<u8> =
                        fingerprint.into_iter().flat_map(u32::to_le_bytes).collect();
                    crate::actions::fingerprint::assign_computed_fingerprint(
                        db,
                        store,
                        &file,
                        ComputedFingerprint {
                            algorithm: "chromaprint".into(),
                            configuration: "rusty-chromaprint-0.3.0/default".into(),
                            format_version: 1,
                            bytes,
                            is_duplicated: 0,
                        },
                    )
                    .await?;
                }
                Err(error) => {
                    log::warn!("Fingerprint computation failed for {}: {error:#}", file.id)
                }
            }
            processed += 1;
            progress(processed, total);
        }
    }
    Ok(processed)
}

/// Compare verified fingerprints and author ordered similarity assignments.
pub async fn compare_all_pairs<F>(
    db: &DatabaseConnection,
    progress: F,
    config: &Configuration,
    cancel: Option<Arc<CancellationToken>>,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<()>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    use crate::actions::fingerprint::{SourceStatus, fingerprint_status};
    let files = MediaFiles::find()
        .filter(media_files::Column::LocalAvailable.eq(true))
        .all(db)
        .await?;
    let mut eligible = Vec::new();
    for file in files {
        let Some(row) = crate::actions::fingerprint::current_fingerprint(db, &file).await? else {
            continue;
        };
        if fingerprint_status(&row, &file) == SourceStatus::Verified {
            eligible.push((file, bytes_to_u32s(row.fingerprint)?));
        }
    }
    eligible.sort_by_key(|(file, _)| file.hlc_uuid.clone());
    let total = eligible
        .len()
        .saturating_mul(eligible.len().saturating_sub(1))
        / 2;
    let mut done = 0;
    let configuration = format!(
        "rusty-chromaprint-0.3.0/{}",
        blake3::hash(format!("{config:?}").as_bytes()).to_hex()
    );
    for first in 0..eligible.len() {
        for second in first + 1..eligible.len() {
            if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                return Ok(());
            }
            let (file1, fp1) = &eligible[first];
            let (file2, fp2) = &eligible[second];
            let segments = match_fingerprints(fp1, fp2, config)?;
            let score = calculate_similarity_score(
                &segments,
                get_track_duration_in_secs(fp1, config)
                    .max(get_track_duration_in_secs(fp2, config)),
                config,
            );
            crate::actions::fingerprint::assign_computed_similarity(
                db,
                store,
                file1,
                file2,
                ComputedSimilarity {
                    first_fingerprint: &fp1
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                        .collect::<Vec<_>>(),
                    second_fingerprint: &fp2
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                        .collect::<Vec<_>>(),
                    algorithm: "chromaprint-match".into(),
                    configuration: configuration.clone(),
                    format_version: 1,
                    score,
                },
            )
            .await?;
            done += 1;
            progress(done, total);
        }
    }
    Ok(())
}

/// Recompute shared duplicate marks from similarity rows with verified source observations.
pub async fn mark_duplicate_files<F>(
    db: &DatabaseConnection,
    threshold: f32,
    progress: F,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<usize>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    let rows = MediaFileSimilarity::find()
        .filter(media_file_similarity::Column::Similarity.gte(threshold))
        .all(db)
        .await?;
    let mut verified = Vec::new();
    for row in rows {
        let Some(first) = MediaFiles::find()
            .filter(media_files::Column::HlcUuid.eq(row.first_track_uuid.clone()))
            .one(db)
            .await?
        else {
            continue;
        };
        let Some(second) = MediaFiles::find()
            .filter(media_files::Column::HlcUuid.eq(row.second_track_uuid.clone()))
            .one(db)
            .await?
        else {
            continue;
        };
        if crate::actions::fingerprint::similarity_status(&row, &first, &second)
            != crate::actions::fingerprint::SourceStatus::Verified
        {
            continue;
        }
        let first_fingerprint =
            crate::actions::fingerprint::current_fingerprint(db, &first).await?;
        let second_fingerprint =
            crate::actions::fingerprint::current_fingerprint(db, &second).await?;
        let (Some(first_fingerprint), Some(second_fingerprint)) =
            (first_fingerprint, second_fingerprint)
        else {
            continue;
        };
        if crate::actions::fingerprint::fingerprint_status(&first_fingerprint, &first)
            != crate::actions::fingerprint::SourceStatus::Verified
            || crate::actions::fingerprint::fingerprint_status(&second_fingerprint, &second)
                != crate::actions::fingerprint::SourceStatus::Verified
        {
            continue;
        }
        verified.push(row);
    }
    let groups = group_similar_files(&verified);
    let total = groups.len();
    let mut marked = 0;
    for (index, group) in groups.into_iter().enumerate() {
        let files = MediaFiles::find()
            .filter(media_files::Column::HlcUuid.is_in(group))
            .all(db)
            .await?;
        let mut identities = HashSet::new();
        for file in &files {
            let id = Uuid::parse_str(&file.hlc_uuid)?;
            anyhow::ensure!(
                !id.is_nil() && identities.insert(id),
                "ambiguous source identities in duplicate group"
            );
        }
        let keep = files
            .iter()
            .max_by(|a, b| {
                a.sample_rate
                    .cmp(&b.sample_rate)
                    .then_with(|| b.hlc_uuid.cmp(&a.hlc_uuid))
            })
            .context("empty duplicate group")?
            .id;
        for file in files {
            let mark = i32::from(file.id != keep);
            crate::actions::fingerprint::assign_duplicate_mark(db, store, file.id, mark).await?;
            marked += mark as usize;
        }
        progress(index + 1, total);
    }
    Ok(marked)
}

/// Reset shared duplicate marks using source-row assignments.
pub async fn reset_duplicate_marks<F>(
    db: &DatabaseConnection,
    progress: F,
    store: &crate::sync::SeaOrmSyncStore,
) -> Result<usize>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    use crate::sync::model::{Collection, ObjectKey, author_source_on};
    use sea_orm::TransactionTrait;
    const BATCH_SIZE: u64 = 64;
    let total = MediaFileFingerprint::find()
        .filter(media_file_fingerprint::Column::IsDuplicated.eq(1))
        .count(db)
        .await? as usize;
    progress(0, total);
    let mut reset = 0;
    let mut cursor = 0;
    loop {
        let tx = db.begin().await?;
        store.lock(&tx).await?;
        let rows = MediaFileFingerprint::find()
            .filter(media_file_fingerprint::Column::IsDuplicated.eq(1))
            .filter(media_file_fingerprint::Column::Id.gt(cursor))
            .order_by_asc(media_file_fingerprint::Column::Id)
            .limit(BATCH_SIZE)
            .all(&tx)
            .await?;
        if rows.is_empty() {
            tx.commit().await?;
            break;
        }
        for row in &rows {
            let file_id = Uuid::parse_str(&row.track_uuid)?;
            let id = Uuid::parse_str(&row.hlc_uuid)?;
            anyhow::ensure!(
                id == migration::catalog_identity::fingerprint(
                    file_id,
                    row.algorithm.as_deref(),
                    row.configuration.as_deref(),
                    row.format_version
                ),
                "fingerprint source identity requires correction"
            );
            let mut update: media_file_fingerprint::ActiveModel = row.clone().into();
            update.is_duplicated = ActiveValue::Set(0);
            update.update(&tx).await?;
            author_source_on(
                &tx,
                store,
                ObjectKey {
                    collection: Collection::Fingerprints,
                    id,
                },
            )
            .await?;
        }
        cursor = rows.last().unwrap().id;
        tx.commit().await?;
        reset += rows.len();
        progress(reset, total);
    }
    Ok(reset)
}
use sea_orm::SelectExt;
