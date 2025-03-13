use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use log::info;
use sea_orm::{
    ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QuerySelect,
};
use tokio_util::sync::CancellationToken;

use tag_editor::music_brainz::fingerprint::{calc_fingerprint, Configuration};

use crate::entities::{media_file_fingerprint, media_files};
use crate::parallel_media_files_processing;

pub async fn compute_file_fingerprints<F>(
    main_db: &DatabaseConnection,
    lib_path: &Path,
    batch_size: usize,
    progress_callback: F,
    cancel_token: Option<CancellationToken>,
) -> Result<usize>
where
    F: Fn(usize, usize) + Send + Sync + 'static,
{
    let progress_callback = Arc::new(progress_callback);

    info!(
        "Starting audio fingerprint computation with batch size: {}",
        batch_size
    );

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

    parallel_media_files_processing!(
        main_db,
        batch_size,
        progress_callback,
        cancel_token,
        cursor_query,
        lib_path,
        move |file, lib_path, cancel_token| {
            compute_single_fingerprint(file, lib_path, &Configuration::default(), cancel_token)
        },
        |db, file: media_files::Model, fingerprint_result: Result<(Vec<u32>, _)>| async move {
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
                        ..Default::default()
                    };

                    match media_file_fingerprint::Entity::insert(model).exec(db).await {
                        Ok(_) => debug!("Inserted fingerprint for file: {}", file.id),
                        Err(e) => error!("Failed to insert fingerprint: {}", e),
                    }
                }
                Err(e) => error!("Failed to compute fingerprint: {}", e),
            }
        }
    )
}

fn compute_single_fingerprint(
    file: &media_files::Model,
    lib_path: &Path,
    config: &Configuration,
    cancel_token: Option<CancellationToken>,
) -> Result<(Vec<u32>, std::time::Duration)> {
    let file_path = lib_path.join(&file.directory).join(&file.file_name);

    if let Some(token) = &cancel_token {
        if token.is_cancelled() {
            return Err(anyhow::anyhow!("Operation cancelled"));
        }
    }

    calc_fingerprint(&file_path, config)
        .with_context(|| format!("Failed to compute fingerprint for: {}", file_path.display()))
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
