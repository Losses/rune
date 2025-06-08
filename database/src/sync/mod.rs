use std::fmt::Debug;
use std::sync::Arc;

use foreign_keys::RuneForeignKeyResolver;
use sea_orm::{DatabaseConnection, EntityName};
use sync::{
    chunking::ChunkingOptions,
    core::{RemoteDataSource, SyncContext, SyncDirection, SyncTableMetadata},
    hlc::{SyncTaskContext, HLC},
    sync_scheduler::{SyncScheduler, TableSyncJob, TableSyncResult},
};
use uuid::Uuid;

use crate::entities;

pub mod bindings;
pub mod chunking;
pub mod foreign_keys;
pub mod utils;

pub async fn setup_and_run_sync<'s, RDS: RemoteDataSource + Debug + Send + Sync + 'static>(
    db: &'s DatabaseConnection,
    local_node_id: Uuid,
    remote_data_source_ref: &'s RDS,
    hlc_task_context_ref: &'s SyncTaskContext,
) -> anyhow::Result<Vec<TableSyncResult>> {
    let sync_context = SyncContext::<'s, RDS> {
        // R in SyncContext is RDS
        db,
        local_node_id,
        remote_source: remote_data_source_ref,
        chunking_options: ChunkingOptions::default(local_node_id),
        sync_direction: SyncDirection::Bidirectional,
        hlc_context: hlc_task_context_ref,
    };

    // FKResolver is Arc'd, so it's 'static and can be cloned into closures
    let fk_resolver = Arc::new(RuneForeignKeyResolver);

    // Helper to create initial metadata
    let initial_meta = |table_name_str: &str| SyncTableMetadata {
        table_name: table_name_str.to_string(),
        last_sync_hlc: HLC::new(local_node_id),
    };

    // TODO: In a real application, we would first try to load saved metadata
    // from a persistent store. If and only if no metadata is found for a table,
    // we would then use `initial_meta` to create the default. This implementation
    // always starts from scratch, which is fine for prototype purpose.

    let jobs: Vec<TableSyncJob<RDS>> = vec![
        TableSyncJob::new::<entities::albums::Entity, _>(
            entities::albums::Entity.table_name().to_string(),
            initial_meta(entities::albums::Entity.table_name()),
            fk_resolver.clone(),
        ),
        TableSyncJob::new::<entities::artists::Entity, _>(
            entities::artists::Entity.table_name().to_string(),
            initial_meta(entities::artists::Entity.table_name()),
            fk_resolver.clone(),
        ),
        TableSyncJob::new::<entities::genres::Entity, _>(
            entities::genres::Entity.table_name().to_string(),
            initial_meta(entities::genres::Entity.table_name()),
            fk_resolver.clone(),
        ),
        TableSyncJob::new::<entities::media_files::Entity, _>(
            entities::media_files::Entity.table_name().to_string(),
            initial_meta(entities::media_files::Entity.table_name()),
            fk_resolver.clone(),
        ),
        TableSyncJob::new::<entities::media_file_albums::Entity, _>(
            entities::media_file_albums::Entity.table_name().to_string(),
            initial_meta(entities::media_file_albums::Entity.table_name()),
            fk_resolver.clone(),
        ),
        TableSyncJob::new::<entities::media_file_artists::Entity, _>(
            entities::media_file_artists::Entity
                .table_name()
                .to_string(),
            initial_meta(entities::media_file_artists::Entity.table_name()),
            fk_resolver.clone(),
        ),
        TableSyncJob::new::<entities::media_file_genres::Entity, _>(
            entities::media_file_genres::Entity.table_name().to_string(),
            initial_meta(entities::media_file_genres::Entity.table_name()),
            fk_resolver.clone(),
        ),
        TableSyncJob::new::<entities::media_cover_art::Entity, _>(
            entities::media_cover_art::Entity.table_name().to_string(),
            initial_meta(entities::media_cover_art::Entity.table_name()),
            fk_resolver.clone(),
        ),
    ];

    let scheduler = SyncScheduler::new();
    let results = scheduler.run_plan(&sync_context, jobs).await;

    Ok(results)
}
