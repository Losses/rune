use std::fmt::Debug;
use std::sync::Arc;

use foreign_keys::RuneForeignKeyResolver;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityName, EntityTrait, QueryFilter, Set,
};
use sync::{
    chunking::ChunkingOptions,
    core::{RemoteDataSource, SyncContext, SyncDirection, SyncTableMetadata},
    hlc::{HLC, SyncTaskContext},
    sync_scheduler::{SyncScheduler, TableSyncJob, TableSyncResult},
};
use uuid::Uuid;

use crate::entities;
use crate::entities::sync_record;
use crate::sync::utils::{create_sync_record_active_model, get_local_last_sync_hlc};

pub mod bindings;
pub mod chunking;
pub mod data_source;
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

    let fk_resolver = Arc::new(RuneForeignKeyResolver);

    // Helper to create initial metadata
    let initial_meta = |table_name_str: String| async move {
        let last_sync_hlc = get_local_last_sync_hlc(db, &table_name_str, local_node_id)
            .await
            .unwrap_or(None)
            .unwrap_or(HLC {
                timestamp_ms: 0,
                version: 0,
                node_id: local_node_id,
            });
        SyncTableMetadata {
            table_name: table_name_str,
            last_sync_hlc,
        }
    };

    // Define the sync order based on table dependencies (topological sort).
    // Parent tables must be synced before their dependent child tables.
    let jobs: Vec<TableSyncJob<RDS>> = vec![
        // Phase 1: Parent/Independent tables
        // These tables do not have foreign keys to other synced tables, or are parents.
        TableSyncJob::new::<entities::media_cover_art::Entity, _>(
            entities::media_cover_art::Entity.table_name().to_string(),
            initial_meta(entities::media_cover_art::Entity.table_name().to_string()).await,
            fk_resolver.clone(),
        ),
        TableSyncJob::new::<entities::artists::Entity, _>(
            entities::artists::Entity.table_name().to_string(),
            initial_meta(entities::artists::Entity.table_name().to_string()).await,
            fk_resolver.clone(),
        ),
        TableSyncJob::new::<entities::genres::Entity, _>(
            entities::genres::Entity.table_name().to_string(),
            initial_meta(entities::genres::Entity.table_name().to_string()).await,
            fk_resolver.clone(),
        ),
        TableSyncJob::new::<entities::albums::Entity, _>(
            entities::albums::Entity.table_name().to_string(),
            initial_meta(entities::albums::Entity.table_name().to_string()).await,
            fk_resolver.clone(),
        ),
        // Phase 2: Child tables that depend on Phase 1 tables
        // `media_files` depends on `media_cover_art`.
        TableSyncJob::new::<entities::media_files::Entity, _>(
            entities::media_files::Entity.table_name().to_string(),
            initial_meta(entities::media_files::Entity.table_name().to_string()).await,
            fk_resolver.clone(),
        ),
        // Phase 3: Join tables that depend on Phase 1 and Phase 2 tables
        // These tables link `media_files` with `albums`, `artists`, `genres`.
        TableSyncJob::new::<entities::media_file_albums::Entity, _>(
            entities::media_file_albums::Entity.table_name().to_string(),
            initial_meta(entities::media_file_albums::Entity.table_name().to_string()).await,
            fk_resolver.clone(),
        ),
        TableSyncJob::new::<entities::media_file_artists::Entity, _>(
            entities::media_file_artists::Entity
                .table_name()
                .to_string(),
            initial_meta(
                entities::media_file_artists::Entity
                    .table_name()
                    .to_string(),
            )
            .await,
            fk_resolver.clone(),
        ),
        TableSyncJob::new::<entities::media_file_genres::Entity, _>(
            entities::media_file_genres::Entity.table_name().to_string(),
            initial_meta(entities::media_file_genres::Entity.table_name().to_string()).await,
            fk_resolver.clone(),
        ),
        TableSyncJob::new::<entities::media_file_fingerprint::Entity, _>(
            entities::media_file_fingerprint::Entity
                .table_name()
                .to_string(),
            initial_meta(
                entities::media_file_fingerprint::Entity
                    .table_name()
                    .to_string(),
            )
            .await,
            fk_resolver.clone(),
        ),
        TableSyncJob::new::<entities::media_file_similarity::Entity, _>(
            entities::media_file_similarity::Entity
                .table_name()
                .to_string(),
            initial_meta(
                entities::media_file_similarity::Entity
                    .table_name()
                    .to_string(),
            )
            .await,
            fk_resolver.clone(),
        ),
    ];

    let scheduler = SyncScheduler::new();
    let results = scheduler.run_plan(&sync_context, jobs).await;

    for result in &results {
        if let TableSyncResult::Success(metadata) = result {
            let existing_record = sync_record::Entity::find()
                .filter(sync_record::Column::TableName.eq(metadata.table_name.clone()))
                .filter(sync_record::Column::ClientNodeId.eq(local_node_id.to_string()))
                .one(db)
                .await?;

            let mut active_model = create_sync_record_active_model(
                metadata.table_name.clone(),
                local_node_id,
                &metadata.last_sync_hlc,
            )?;

            if let Some(record) = existing_record {
                active_model.id = Set(record.id);
            }

            active_model.save(db).await?;
        }
    }

    Ok(results)
}
