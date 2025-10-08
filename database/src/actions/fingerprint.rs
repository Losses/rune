use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use fsio::FsIo;
use log::{debug, info};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, JoinType,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, RelationTrait,
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub use tag_editor::music_brainz::fingerprint::{Configuration, Segment};
use tag_editor::music_brainz::fingerprint::{
    calc_fingerprint, calculate_similarity_score, get_track_duration_in_secs, match_fingerprints,
};

use crate::entities::prelude::{MediaFileFingerprint, MediaFileSimilarity, MediaFiles};
use crate::entities::{media_file_fingerprint, media_file_similarity, media_files};
use crate::parallel_media_files_processing;

pub async fn compute_file_fingerprints<F>(
    fsio: Arc<FsIo>,
    main_db: &DatabaseConnection,
    lib_path: &Path,
    node_id: &str,
    batch_size: usize,
    progress_callback: F,
    cancel_token: Option<CancellationToken>,
) -> Result<usize>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    let progress_callback = Arc::new(progress_callback);

    info!("Starting audio fingerprint computation with batch size: {batch_size}");

    let existed_ids: Vec<i32> = media_file_fingerprint::Entity::find()
        .select_only()
        .column(media_file_fingerprint::Column::MediaFileId)
        .distinct()
        .into_tuple::<i32>()
        .all(main_db)
        .await
        .context("Failed to query existing fingerprints")?;

    let cursor_query =
        media_files::Entity::find().filter(media_files::Column::Id.is_not_in(existed_ids));

    let lib_path = Arc::new(lib_path.to_path_buf());
    let node_id = Arc::new(node_id.to_owned());

    parallel_media_files_processing!(
        main_db,
        batch_size,
        progress_callback,
        cancel_token,
        cursor_query,
        lib_path,
        fsio,
        node_id,
        move |fsio, file, lib_path, cancel_token| {
            compute_single_fingerprint(
                fsio,
                lib_path,
                file,
                &Configuration::default(),
                cancel_token,
            )
        },
        |db,
         file: media_files::Model,
         node_id: Arc<String>,
         fingerprint_result: Result<(Vec<u32>, _)>| async move {
            match fingerprint_result {
                Ok((fingerprint, _duration)) => {
                    let fingerprint_bytes = fingerprint
                        .into_iter()
                        .flat_map(|x| x.to_le_bytes())
                        .collect::<Vec<u8>>();

                    let model = media_file_fingerprint::ActiveModel {
                        media_file_id: ActiveValue::Set(file.id),
                        fingerprint: ActiveValue::Set(fingerprint_bytes),
                        is_duplicated: ActiveValue::Set(0),
                        hlc_uuid: ActiveValue::Set(
                            Uuid::new_v5(
                                &Uuid::NAMESPACE_OID,
                                format!("RUNE_FINGERPRINT::{}", file.id).as_bytes(),
                            )
                            .to_string(),
                        ),
                        created_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
                        updated_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
                        created_at_hlc_ver: ActiveValue::Set(0), // TODO: Fix this
                        updated_at_hlc_ver: ActiveValue::Set(0),
                        created_at_hlc_nid: ActiveValue::Set(node_id.to_string()),
                        updated_at_hlc_nid: ActiveValue::Set(node_id.to_string()),
                        ..Default::default()
                    };

                    match media_file_fingerprint::Entity::insert(model).exec(db).await {
                        Ok(_) => debug!("Inserted fingerprint for file: {}", file.id),
                        Err(e) => error!("Failed to insert fingerprint: {e}"),
                    }
                }
                Err(e) => error!("Failed to compute fingerprint: {e:#?}"),
            }
        }
    )
}

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

    let result = calc_fingerprint(fsio, &file_path, config)
        .with_context(|| format!("compute fingerprint for: {}", file_path.display()))?;

    Ok(result)
}

pub async fn has_fingerprint(main_db: &DatabaseConnection, file_id: i32) -> Result<bool> {
    Ok(media_file_fingerprint::Entity::find()
        .filter(media_file_fingerprint::Column::MediaFileId.eq(file_id))
        .count(main_db)
        .await?
        > 0)
}

pub async fn get_fingerprint_count(main_db: &DatabaseConnection) -> Result<u64> {
    Ok(media_file_fingerprint::Entity::find()
        .count(main_db)
        .await?)
}

pub async fn compare_all_pairs<F>(
    db: &DatabaseConnection,
    node_id: &str,
    batch_size: usize,
    progress_callback: F,
    config: &Configuration,
    cancel_token: Option<Arc<CancellationToken>>,
    page_size: u64,
) -> Result<()>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    let progress_callback = Arc::new(progress_callback);
    let mut last_id = 0;

    info!("Start comparing all tracks");

    loop {
        info!("Comparing fingerprints after: {last_id}");

        if let Some(token) = &cancel_token
            && token.is_cancelled()
        {
            return Ok(());
        }

        let files_page = MediaFiles::find()
            .filter(media_files::Column::Id.gt(last_id))
            .order_by_asc(media_files::Column::Id)
            .limit(page_size)
            .all(db)
            .await?;

        if files_page.is_empty() {
            info!("No more files to compare.");
            break;
        }

        process_page_combinations(
            db,
            node_id,
            batch_size,
            &files_page,
            config,
            cancel_token.clone(),
            Arc::clone(&progress_callback),
        )
        .await?;

        last_id = files_page.last().map(|f| f.id).unwrap_or(last_id);
    }

    Ok(())
}

async fn process_page_combinations<F>(
    db: &DatabaseConnection,
    node_id: &str,
    batch_size: usize,
    current_page: &[media_files::Model],
    config: &Configuration,
    cancel_token: Option<Arc<CancellationToken>>,
    progress_callback: Arc<F>,
) -> Result<()>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    if let Some(token) = &cancel_token
        && token.is_cancelled()
    {
        return Ok(());
    }

    info!("Processing page with {} files", current_page.len());

    let mut total_tasks = 0;
    let mut history_files_per_file = Vec::with_capacity(current_page.len());
    for (i, file1) in current_page.iter().enumerate() {
        if let Some(token) = &cancel_token
            && token.is_cancelled()
        {
            return Ok(());
        }

        let current_combinations = current_page.len() - i - 1;
        let history_files = load_history_files(db, file1.id).await?;
        let history_combinations = history_files.len();
        total_tasks += current_combinations + history_combinations;
        history_files_per_file.push(history_files);
    }

    if total_tasks == 0 {
        return Ok(());
    }

    progress_callback(0, total_tasks);

    let (tx, rx) = async_channel::bounded(1000);
    let semaphore = Arc::new(Semaphore::new(batch_size));
    let progress_counter = Arc::new(AtomicUsize::new(0));

    let producer = tokio::spawn({
        let current_page = current_page.to_vec();
        let cancel_token = cancel_token.clone();
        let history_files_per_file = history_files_per_file.clone();
        async move {
            for (i, file1) in current_page.iter().enumerate() {
                if let Some(token) = &cancel_token
                    && token.is_cancelled()
                {
                    return Ok(());
                }

                for file2 in &current_page[i + 1..] {
                    if let Some(token) = &cancel_token
                        && token.is_cancelled()
                    {
                        return Ok(());
                    }
                    tx.send((file1.id, file2.id)).await?;
                }

                let history_files = &history_files_per_file[i];
                for file2_id in history_files {
                    if let Some(token) = &cancel_token
                        && token.is_cancelled()
                    {
                        return Ok(());
                    }
                    tx.send((file1.id, *file2_id)).await?;
                }
            }
            Ok::<(), anyhow::Error>(())
        }
    });

    let consumer = tokio::spawn({
        let db = db.clone();
        let config = config.clone();
        let node_id = node_id.to_owned();
        let cancel_token = cancel_token.clone();
        let progress_callback = Arc::clone(&progress_callback);
        let progress_counter = Arc::clone(&progress_counter);
        async move {
            while let Ok((id1, id2)) = rx.recv().await {
                if let Some(token) = &cancel_token
                    && token.is_cancelled()
                {
                    return Ok(());
                }

                let _permit = semaphore.acquire().await?;

                let fp1 = load_fingerprint(db.clone(), id1).await?;
                let fp2 = load_fingerprint(db.clone(), id2).await?;

                let segments = match_fingerprints(&fp1, &fp2, &config)?;
                let similarity = calculate_similarity_score(
                    &segments,
                    get_track_duration_in_secs(&fp1, &config)
                        .max(get_track_duration_in_secs(&fp2, &config)),
                    &config,
                );

                MediaFileSimilarity::insert(media_file_similarity::ActiveModel {
                    file_id1: ActiveValue::Set(id1),
                    file_id2: ActiveValue::Set(id2),
                    similarity: ActiveValue::Set(similarity),
                    hlc_uuid: ActiveValue::Set(
                        Uuid::new_v5(
                            &Uuid::NAMESPACE_OID,
                            format!("RUNE_SIMILARITY::{id1}+{id2}").as_bytes(),
                        )
                        .to_string(),
                    ),
                    created_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
                    updated_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
                    created_at_hlc_ver: ActiveValue::Set(0), // TODO: Fix this
                    updated_at_hlc_ver: ActiveValue::Set(0),
                    created_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
                    updated_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
                    ..Default::default()
                })
                .exec(&db)
                .await?;

                let current = progress_counter.fetch_add(1, Ordering::Relaxed) + 1;
                progress_callback(current, total_tasks);
            }
            Ok::<(), anyhow::Error>(())
        }
    });

    let (p, c) = tokio::join!(producer, consumer);

    match (p, c) {
        (Ok(Ok(())), Ok(Ok(()))) => Ok(()),
        (Ok(Err(e)), _) | (_, Ok(Err(e))) => Err(e),
        (Err(e), _) => Err(anyhow::Error::from(e)),
        (_, Err(e)) => Err(anyhow::Error::from(e)),
    }
}

async fn load_fingerprint(db: DatabaseConnection, id: i32) -> Result<Vec<u32>> {
    let fingerprint = MediaFileFingerprint::find()
        .filter(media_file_fingerprint::Column::MediaFileId.eq(id))
        .one(&db)
        .await?
        .context("Fingerprint not found")?
        .fingerprint;

    bytes_to_u32s(fingerprint)
}

async fn load_history_files(db: &DatabaseConnection, current_id: i32) -> Result<Vec<i32>> {
    info!("Loading history files: {current_id}");

    let history_ids = media_files::Entity::find()
        .select_only()
        .column(media_files::Column::Id)
        .filter(media_files::Column::Id.lt(current_id))
        .order_by_asc(media_files::Column::Id)
        .into_tuple::<i32>()
        .all(db)
        .await?;

    info!("Got {} history files", history_ids.len());

    Ok(history_ids)
}

pub fn bytes_to_u32s(bytes: Vec<u8>) -> Result<Vec<u32>> {
    if !bytes.len().is_multiple_of(4) {
        bail!("The length of the input byte vector must be a multiple of 4.".to_string());
    }

    let mut u32s = Vec::new();
    for chunk in bytes.chunks_exact(4) {
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

pub async fn mark_duplicate_files<F>(
    db: &DatabaseConnection,
    similarity_threshold: f32,
    progress_callback: F,
) -> Result<usize>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    let progress_callback = Arc::new(progress_callback);

    info!("Starting duplicate detection with similarity threshold: {similarity_threshold}");

    // Step 1: Get all file similarity pairs above the threshold
    progress_callback(0, 3); // 3 main stages: getting data, grouping, marking

    let similarities = MediaFileSimilarity::find()
        .filter(media_file_similarity::Column::Similarity.gte(similarity_threshold))
        .all(db)
        .await
        .context("Failed to retrieve file similarities")?;

    if similarities.is_empty() {
        info!("No similar files found above threshold {similarity_threshold}");
        progress_callback(3, 3); // Complete all stages
        return Ok(0);
    }

    info!(
        "Found {} similar file pairs above threshold",
        similarities.len()
    );
    progress_callback(1, 3); // Completed first stage

    // Step 2: Group files into clusters of similar content
    let file_groups = group_similar_files(&similarities);
    info!("Created {} groups of similar files", file_groups.len());
    progress_callback(2, 3); // Completed second stage

    // Step 3: For each group, keep the highest sample rate file and mark others as duplicates
    let total_groups = file_groups.len();
    let progress_callback_for_marking = {
        let progress_callback = Arc::clone(&progress_callback);
        move |current: usize, _: usize| {
            // Map group progress to overall progress (from 2 to 3)
            let overall_progress = 2.0 + (current as f32 / total_groups as f32);
            progress_callback(overall_progress.floor() as usize, 3);
        }
    };

    let marked_count = mark_duplicates(db, file_groups, progress_callback_for_marking).await?;
    info!("Marked {marked_count} files as duplicates");
    progress_callback(3, 3); // Completed all stages

    Ok(marked_count)
}

fn group_similar_files(similarities: &[media_file_similarity::Model]) -> Vec<Vec<i32>> {
    let mut adjacency_list: HashMap<i32, Vec<i32>> = HashMap::new();

    // Build an adjacency list for our similarity graph
    for similarity in similarities {
        adjacency_list
            .entry(similarity.file_id1)
            .or_default()
            .push(similarity.file_id2);
        adjacency_list
            .entry(similarity.file_id2)
            .or_default()
            .push(similarity.file_id1);
    }

    // Use a set to track visited nodes during our search
    let mut visited = HashSet::new();
    let mut groups = Vec::new();

    // Perform a depth-first search to find connected components (groups)
    for &file_id in adjacency_list.keys() {
        if visited.contains(&file_id) {
            continue;
        }

        let mut group = Vec::new();
        let mut stack = vec![file_id];
        visited.insert(file_id);

        while let Some(current_id) = stack.pop() {
            group.push(current_id);

            if let Some(neighbors) = adjacency_list.get(&current_id) {
                for &neighbor in neighbors {
                    if !visited.contains(&neighbor) {
                        visited.insert(neighbor);
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

async fn mark_duplicates<F>(
    db: &DatabaseConnection,
    file_groups: Vec<Vec<i32>>,
    progress_callback: F,
) -> Result<usize>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    let mut total_marked = 0;
    let total_groups = file_groups.len();

    for (group_index, group) in file_groups.into_iter().enumerate() {
        if group.len() <= 1 {
            progress_callback(group_index + 1, total_groups);
            continue;
        }

        // Get file details for all files in this group
        let files = MediaFiles::find()
            .filter(media_files::Column::Id.is_in(group.clone()))
            .all(db)
            .await
            .context("Failed to retrieve file details")?;

        // Find the file with highest sample rate
        let keep_file = files
            .iter()
            .max_by_key(|file| file.sample_rate)
            .context("Failed to find highest sample rate file")?;

        debug!(
            "Keeping file {} with sample rate {}",
            keep_file.id, keep_file.sample_rate
        );

        // Mark all other files in the group as duplicates
        for file in files.iter().filter(|f| f.id != keep_file.id) {
            let fingerprint = MediaFileFingerprint::find()
                .filter(media_file_fingerprint::Column::MediaFileId.eq(file.id))
                .one(db)
                .await
                .context("Failed to retrieve fingerprint")?;

            if let Some(fp) = fingerprint {
                let mut fp_active: media_file_fingerprint::ActiveModel = fp.into();
                fp_active.is_duplicated = ActiveValue::Set(1); // Mark as duplicated
                fp_active
                    .update(db)
                    .await
                    .context(format!("Failed to mark file {} as duplicate", file.id))?;

                total_marked += 1;
                debug!(
                    "Marked file {} as duplicate (sample rate {})",
                    file.id, file.sample_rate
                );
            }
        }

        progress_callback(group_index + 1, total_groups);
    }

    Ok(total_marked)
}

// Function to get all duplicated files
pub async fn get_duplicate_files(db: &DatabaseConnection) -> Result<Vec<media_files::Model>> {
    let files = MediaFiles::find()
        .join(
            JoinType::InnerJoin,
            media_file_fingerprint::Relation::MediaFiles.def(),
        )
        .filter(media_file_fingerprint::Column::IsDuplicated.eq(1))
        .all(db)
        .await
        .context("Failed to retrieve duplicate files")?;

    Ok(files)
}

// Function to reset duplicate marks
pub async fn reset_duplicate_marks<F>(
    db: &DatabaseConnection,
    progress_callback: F,
) -> Result<usize>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    let fingerprints = MediaFileFingerprint::find()
        .filter(media_file_fingerprint::Column::IsDuplicated.eq(1))
        .all(db)
        .await
        .context("Failed to retrieve marked fingerprints")?;

    let total_fingerprints = fingerprints.len();
    progress_callback(0, total_fingerprints);

    let mut updated_count = 0;

    for (index, fp) in fingerprints.into_iter().enumerate() {
        let mut fp_active: media_file_fingerprint::ActiveModel = fp.into();
        fp_active.is_duplicated = ActiveValue::Set(0); // Reset duplicate mark
        fp_active
            .update(db)
            .await
            .context("Failed to reset duplicate mark")?;
        updated_count += 1;

        progress_callback(index + 1, total_fingerprints);
    }

    info!("Reset duplicate marks for {updated_count} files");
    Ok(updated_count)
}
