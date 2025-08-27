use async_trait::async_trait;
use deunicode::deunicode;
use sea_orm::prelude::*;
use sea_orm::{DatabaseConnection, DatabaseTransaction, EntityTrait};

pub trait DatabaseExecutor: Send + Sync {}

impl DatabaseExecutor for DatabaseConnection {}
impl DatabaseExecutor for DatabaseTransaction {}

pub fn first_char(s: &str) -> char {
    deunicode(s).chars().next().unwrap_or('#')
}

pub fn generate_group_name(x: &str) -> String {
    let c = first_char(x);

    if c.is_lowercase() {
        c.to_ascii_uppercase().to_string()
    } else if c.is_ascii_digit() || !c.is_alphabetic() {
        '#'.to_string()
    } else {
        c.to_string()
    }
}

#[async_trait]
pub trait CollectionDefinition: EntityTrait {
    fn group_column() -> Self::Column;
    fn id_column() -> Self::Column;
}

#[macro_export]
macro_rules! get_entity_to_cover_ids {
    ($db:expr, $entity_ids:expr, $related_entity:ident, $relation_column_name:ident, $magic_cover_art_id:expr) => {
        paste::paste! {{
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
            use std::collections::{HashMap, HashSet};
            use $crate::entities::media_files;

            // Fetch related media files for these entities
            let media_file_relations = <$related_entity::Entity>::find()
                .filter(<$related_entity::Column>::$relation_column_name.is_in($entity_ids.clone()))
                .all($db)
                .await?;

            let mut entity_to_media_file_ids: HashMap<i32, Vec<i32>> = HashMap::new();
            for relation in media_file_relations {
                entity_to_media_file_ids
                    .entry(relation.[<$relation_column_name:snake:lower>])
                    .or_default()
                    .push(relation.media_file_id);
            }

            let mut entity_to_cover_ids: HashMap<i32, HashSet<i32>> = HashMap::new();
            for (entity_id, media_file_ids) in entity_to_media_file_ids {
                let media_files = media_files::Entity::find()
                    .filter(media_files::Column::Id.is_in(media_file_ids))
                    .filter(media_files::Column::CoverArtId.ne($magic_cover_art_id))
                    .all($db)
                    .await?;

                let cover_ids = media_files
                    .into_iter()
                    .filter_map(|media_file| media_file.cover_art_id)
                    .collect::<HashSet<i32>>();

                entity_to_cover_ids.insert(entity_id, cover_ids);
            }

            Ok::<HashMap<i32, HashSet<i32>>, sea_orm::DbErr>(entity_to_cover_ids)
        }}
    };
}

#[macro_export]
macro_rules! get_cover_ids {
    ($fn_name:ident, $item_entity:ident, $related_entity:ident, $relation_column_name:ident) => {
        pub async fn $fn_name(
            db: &DatabaseConnection,
            entities: &[$item_entity::Model],
        ) -> Result<HashMap<i32, HashSet<i32>>, DbErr> {
            use $crate::get_entity_to_cover_ids;

            let entity_ids: Vec<i32> = entities.iter().map(|x| x.id).collect();
            let magic_cover_art_id = -1;

            get_entity_to_cover_ids!(
                db,
                entity_ids,
                $related_entity,
                $relation_column_name,
                magic_cover_art_id
            )
        }
    };
}

#[macro_export]
macro_rules! get_by_ids {
    ($fn_name:ident, $item_entity:ident) => {
        pub async fn $fn_name(
            db: &DatabaseConnection,
            ids: &[i32],
        ) -> Result<Vec<$item_entity::Model>, sea_orm::DbErr> {
            let items = <$item_entity::Entity>::find()
                .filter(<$item_entity::Column>::Id.is_in(ids.to_vec()))
                .all(db)
                .await?;

            Ok(items)
        }
    };
}

#[macro_export]
macro_rules! get_by_id {
    ($fn_name:ident, $item_entity:ident) => {
        pub async fn $fn_name(
            db: &DatabaseConnection,
            id: i32,
        ) -> Result<Option<$item_entity::Model>, sea_orm::DbErr> {
            let item = $item_entity::Entity::find()
                .filter($item_entity::Column::Id.eq(id))
                .one(db)
                .await?;

            Ok(item)
        }
    };
}

#[macro_export]
macro_rules! get_first_n {
    ($fn_name:ident, $item_entity:ident) => {
        pub async fn $fn_name(
            db: &DatabaseConnection,
            n: u64,
        ) -> Result<Vec<$item_entity::Model>, sea_orm::DbErr> {
            use sea_orm::QuerySelect;

            let item = $item_entity::Entity::find().limit(n).all(db).await?;

            Ok(item)
        }
    };
}

#[macro_export]
macro_rules! parallel_media_files_processing {
    (
        $main_db:expr,
        $batch_size:expr,
        $progress_callback:expr,
        $cancel_token:expr,
        $cursor_query:expr,
        $lib_path:expr,
        $fsio:expr,
        $node_id: expr,
        $process_fn:expr,
        $result_handler:expr
    ) => {{
        use async_channel;
        use log::{debug, error, info, warn};
        use std::sync::Arc;
        use tokio::sync::{Mutex, Semaphore};
        use tokio::task;

        let cursor_query_clone = $cursor_query.clone();
        let (tx, rx) = async_channel::bounded($batch_size);
        info!("Batch size: {}", $batch_size);
        let total_tasks = cursor_query_clone.count($main_db).await? as usize;
        info!("Total tasks: {}", { total_tasks });
        let processed_count = Arc::new(Mutex::new(0));

        let producer_cancel_token = $cancel_token.clone();
        let producer = {
            let cursor_query_clone = $cursor_query.clone();
            let tx = tx.clone(); // Clone sender for producer
            async move {
                let mut cursor = cursor_query_clone.cursor_by(media_files::Column::Id);
                loop {
                    if let Some(ref token) = producer_cancel_token {
                        if token.is_cancelled() {
                            info!("Cancellation requested. Exiting producer loop.");
                            // Handle the error gracefully
                            if let Err(e) = tx.send(None).await {
                                warn!("Failed to send termination signal: {:#?}", { e });
                            }
                            break;
                        }
                    }

                    let files = match cursor
                        .first($batch_size.try_into().unwrap())
                        .all($main_db)
                        .await
                    {
                        Ok(f) => f,
                        Err(e) => {
                            error!("Database error: {e:?}");
                            // Send termination signal on error
                            if let Err(e) = tx.send(None).await {
                                warn!("Failed to send termination signal: {:#?}", { e });
                            }
                            return Err(e);
                        }
                    };

                    if files.is_empty() {
                        info!("No more files to process. Exiting loop.");
                        // Handle the error gracefully
                        if let Err(e) = tx.send(None).await {
                            error!("Failed to send termination signal: {:#?}", e);
                        }
                        break;
                    }

                    for file in &files {
                        // Check cancellation before each send
                        if let Some(ref token) = producer_cancel_token {
                            if token.is_cancelled() {
                                info!("Cancellation requested during file sending.");
                                if let Err(e) = tx.send(None).await {
                                    warn!("Failed to send termination signal: {:#?}", { e });
                                }
                                return Ok(());
                            }
                        }

                        match tx.send(Some(file.clone())).await {
                            Ok(_) => {}
                            Err(e) => {
                                warn!("Failed to send file: {:#?}", { e });
                                return Ok(());
                            }
                        }
                    }

                    if let Some(last_file) = files.last() {
                        info!("Moving cursor after file ID: {}", last_file.id);
                        cursor.after(last_file.id);
                    } else {
                        break;
                    }
                }

                Ok(())
            }
        };

        let consumer_cancel_token = $cancel_token.clone();
        let semaphore = Arc::new(Semaphore::new($batch_size));
        let consumer = {
            let processed_count = Arc::clone(&processed_count);
            let progress_callback = Arc::clone(&$progress_callback);
            async move {
                let mut active_tasks = vec![];

                while let Ok(file_option) = rx.recv().await {
                    match file_option {
                        Some(file) => {
                            if let Some(ref token) = consumer_cancel_token {
                                if token.is_cancelled() {
                                    info!("Cancellation requested. Exiting consumer loop.");
                                    break;
                                }
                            }

                            let main_db = $main_db.clone();
                            let semaphore = semaphore.clone();
                            let permit = match semaphore.acquire_owned().await {
                                Ok(permit) => permit,
                                Err(e) => {
                                    error!("Failed to acquire semaphore: {e:?}");
                                    continue;
                                }
                            };

                            let lib_path = Arc::clone(&$lib_path);
                            let fsio = $fsio.clone();
                            let processed_count = Arc::clone(&processed_count);
                            let progress_callback = Arc::clone(&progress_callback);
                            let node_id_clone = $node_id.clone();
                            let process_cancel_token = consumer_cancel_token.clone();

                            let file_clone = file.clone();
                            let task = task::spawn(async move {
                                let analysis_result = task::spawn_blocking(move || {
                                    let process_fn = $process_fn;
                                    process_fn(
                                        fsio.as_ref(),
                                        &file_clone,
                                        &lib_path,
                                        process_cancel_token,
                                    )
                                    // Pass the cloned token
                                })
                                .await;

                                match analysis_result
                                    .with_context(|| "Failed to spawn analysis task")
                                {
                                    Ok(analysis_result) => {
                                        $result_handler(
                                            &main_db,
                                            file,
                                            node_id_clone,
                                            analysis_result,
                                        )
                                        .await;
                                    }
                                    Err(e) => error!("{e:?}"),
                                }

                                let mut count = processed_count.lock().await;
                                *count += 1;
                                progress_callback(*count, total_tasks);

                                drop(permit);
                            });

                            active_tasks.push(task);
                        }
                        None => {
                            info!("No files left, waiting for active tasks to complete...");
                            for task in active_tasks {
                                if let Err(e) = task.await {
                                    error!("Task join error: {e:?}");
                                }
                            }
                            break;
                        }
                    }
                }

                info!("Scanning task finished.");

                Ok::<(), sea_orm::DbErr>(())
            }
        };

        let (producer_result, consumer_result) = futures::join!(producer, consumer);

        // Handle any errors from producer or consumer
        if let Err(e) = producer_result {
            error!("Producer error: {e:?}");
        }
        if let Err(e) = consumer_result {
            error!("Consumer error: {e:?}");
        }

        info!("Total tasks executed: {}", { total_tasks });

        Ok(total_tasks)
    }};
}
