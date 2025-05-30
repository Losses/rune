use std::{cmp::max as cmp_max, sync::Arc};

use anyhow::{anyhow, Context, Result};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use log::{debug, error, info, warn};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel, QueryFilter,
    TransactionTrait,
};
use serde::{Deserialize, Serialize};
use sync::chunking::{break_data_chunk, generate_data_chunks};
use uuid::Uuid;

use crate::{
    entities::{
        albums, artists, genres, media_cover_art, media_file_albums, media_file_artists,
        media_file_genres, media_files, sync_record,
    },
    sync::utils::parse_hlc,
};

use ::sync::{
    chunking::{ChunkingOptions, DataChunk, SubDataChunk},
    core::{RemoteRecordsWithPayload, SyncOperation},
    foreign_key::{ForeignKeyResolver, ModelWithForeignKeyOps},
    hlc::{HLCModel, HLCQuery, HLCRecord, HLC},
};

use super::foreign_keys::RuneForeignKeyResolver;

// Server's application state
#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub node_id: Uuid, // Server's own node ID
    pub fk_resolver: Arc<RuneForeignKeyResolver>,
    pub default_chunking_options: ChunkingOptions,
}

// Custom error type for API handlers
pub struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        error!("Server error: {:?}", self.0);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}", self.0),
        )
            .into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

// Helper to parse HLC from query parameters (example)
#[derive(Deserialize, Debug)]
pub struct HlcQueryParams {
    pub timestamp: Option<u64>,
    pub version: Option<u32>,
    pub node_id: Option<String>,
}

impl HlcQueryParams {
    pub fn try_into_hlc(&self) -> Result<HLC> {
        let nid_str = self
            .node_id
            .as_deref()
            .ok_or_else(|| anyhow!("Missing HLC node_id"))?;
        Ok(HLC {
            timestamp: self
                .timestamp
                .ok_or_else(|| anyhow!("Missing HLC timestamp"))?,
            version: self.version.ok_or_else(|| anyhow!("Missing HLC version"))?,
            node_id: Uuid::parse_str(nid_str)
                .with_context(|| format!("Invalid HLC node_id string: {}", nid_str))?,
        })
    }
}

pub fn parse_optional_hlc_from_parts(
    ts: Option<u64>,
    v: Option<u32>,
    nid_str_opt: Option<String>,
) -> Result<Option<HLC>> {
    match (ts, v, nid_str_opt) {
        (Some(timestamp), Some(version), Some(nid_str)) => {
            let node_id = Uuid::parse_str(&nid_str)
                .with_context(|| format!("Invalid HLC node_id string: {}", nid_str))?;
            Ok(Some(HLC {
                timestamp,
                version,
                node_id,
            }))
        }
        (None, None, None) => Ok(None), // All parts missing, means no HLC provided
        _ => {
            // Partial HLC provided, which is an error.
            // Alternatively, you could try to default parts, but explicit is better.
            Err(anyhow!("Incomplete HLC provided. Must provide all parts (timestamp, version, node_id) or none."))
        }
    }
}

pub async fn get_node_id_handler(State(state): State<Arc<AppState>>) -> Json<Uuid> {
    Json(state.node_id)
}

#[derive(Deserialize, Debug)]
pub struct GetRemoteChunksParams {
    pub after_hlc_ts: Option<u64>,
    pub after_hlc_ver: Option<u32>,
    pub after_hlc_nid: Option<String>,
}

pub async fn get_remote_chunks_handler(
    State(state): State<Arc<AppState>>,
    Path(table_name): Path<String>,
    Query(params): Query<GetRemoteChunksParams>,
) -> Result<Json<Vec<DataChunk>>, AppError> {
    let after_hlc = parse_optional_hlc_from_parts(
        params.after_hlc_ts,
        params.after_hlc_ver,
        params.after_hlc_nid,
    )?;
    let mut options = state.default_chunking_options.clone();
    options.node_id = state.node_id;
    options.validate()?;
    let db = &state.db;
    let fk_resolver = state.fk_resolver.as_ref();

    let chunks = match table_name.as_str() {
        "albums" => {
            generate_data_chunks::<albums::Entity, _>(db, &options, after_hlc, Some(fk_resolver))
                .await?
        }
        "artists" => {
            generate_data_chunks::<artists::Entity, _>(db, &options, after_hlc, Some(fk_resolver))
                .await?
        }
        "genres" => {
            generate_data_chunks::<genres::Entity, _>(db, &options, after_hlc, Some(fk_resolver))
                .await?
        }
        "media_cover_art" => {
            generate_data_chunks::<media_cover_art::Entity, _>(
                db,
                &options,
                after_hlc,
                Some(fk_resolver),
            )
            .await?
        }
        "media_files" => {
            generate_data_chunks::<media_files::Entity, _>(
                db,
                &options,
                after_hlc,
                Some(fk_resolver),
            )
            .await?
        }
        "media_file_albums" => {
            generate_data_chunks::<media_file_albums::Entity, _>(
                db,
                &options,
                after_hlc,
                Some(fk_resolver),
            )
            .await?
        }
        "media_file_artists" => {
            generate_data_chunks::<media_file_artists::Entity, _>(
                db,
                &options,
                after_hlc,
                Some(fk_resolver),
            )
            .await?
        }
        "media_file_genres" => {
            generate_data_chunks::<media_file_genres::Entity, _>(
                db,
                &options,
                after_hlc,
                Some(fk_resolver),
            )
            .await?
        }
        _ => {
            return Err(AppError(anyhow!(
                "Unsupported table name for chunks: {}",
                table_name
            )))
        }
    };
    Ok(Json(chunks))
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GetRemoteSubChunksPayload {
    pub parent_chunk: DataChunk,
    pub sub_chunk_size: u64,
}

pub async fn get_remote_sub_chunks_handler(
    State(state): State<Arc<AppState>>,
    Path(table_name): Path<String>,
    Json(payload): Json<GetRemoteSubChunksPayload>,
) -> Result<Json<Vec<DataChunk>>, AppError> {
    let db = &state.db;
    let fk_resolver = state.fk_resolver.as_ref();
    let sub_chunks_metadata: Vec<SubDataChunk> = match table_name.as_str() {
        "albums" => {
            break_data_chunk::<albums::Entity, _>(
                db,
                &payload.parent_chunk,
                payload.sub_chunk_size,
                Some(fk_resolver),
            )
            .await?
        }
        "artists" => {
            break_data_chunk::<artists::Entity, _>(
                db,
                &payload.parent_chunk,
                payload.sub_chunk_size,
                Some(fk_resolver),
            )
            .await?
        }
        "genres" => {
            break_data_chunk::<genres::Entity, _>(
                db,
                &payload.parent_chunk,
                payload.sub_chunk_size,
                Some(fk_resolver),
            )
            .await?
        }
        "media_cover_art" => {
            break_data_chunk::<media_cover_art::Entity, _>(
                db,
                &payload.parent_chunk,
                payload.sub_chunk_size,
                Some(fk_resolver),
            )
            .await?
        }
        "media_files" => {
            break_data_chunk::<media_files::Entity, _>(
                db,
                &payload.parent_chunk,
                payload.sub_chunk_size,
                Some(fk_resolver),
            )
            .await?
        }
        "media_file_albums" => {
            break_data_chunk::<media_file_albums::Entity, _>(
                db,
                &payload.parent_chunk,
                payload.sub_chunk_size,
                Some(fk_resolver),
            )
            .await?
        }
        "media_file_artists" => {
            break_data_chunk::<media_file_artists::Entity, _>(
                db,
                &payload.parent_chunk,
                payload.sub_chunk_size,
                Some(fk_resolver),
            )
            .await?
        }
        "media_file_genres" => {
            break_data_chunk::<media_file_genres::Entity, _>(
                db,
                &payload.parent_chunk,
                payload.sub_chunk_size,
                Some(fk_resolver),
            )
            .await?
        }
        _ => {
            return Err(AppError(anyhow!(
                "Unsupported table name for sub_chunks: {}",
                table_name
            )))
        }
    };
    let result_chunks: Vec<DataChunk> = sub_chunks_metadata.into_iter().map(|s| s.chunk).collect();
    Ok(Json(result_chunks))
}

#[derive(Deserialize, Debug)]
pub struct GetRemoteRecordsParams {
    pub start_hlc_ts: u64,
    pub start_hlc_ver: u32,
    pub start_hlc_nid: String,
    pub end_hlc_ts: u64,
    pub end_hlc_ver: u32,
    pub end_hlc_nid: String,
}
pub async fn fetch_records_with_fk_payloads<E, FKR>(
    db: &DatabaseConnection,
    start_hlc: &HLC,
    end_hlc: &HLC,
    fk_resolver: &FKR,
) -> Result<RemoteRecordsWithPayload<E::Model>>
where
    E: HLCModel + EntityTrait + Send + Sync,
    E::Model:
        HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize + ModelWithForeignKeyOps,
    FKR: ForeignKeyResolver + Send + Sync,
{
    let records: Vec<E::Model> = E::find()
        .filter(E::between(start_hlc, end_hlc)?)
        .order_by_hlc_asc::<E>()
        .all(db)
        .await?;
    let mut fk_payloads = Vec::with_capacity(records.len());
    for record_model in &records {
        let payload = fk_resolver
            .extract_foreign_key_sync_ids(record_model, db)
            .await?;
        fk_payloads.push(payload);
    }
    Ok(RemoteRecordsWithPayload {
        records,
        fk_payloads,
    })
}

pub async fn get_remote_records_in_hlc_range_handler(
    State(state): State<Arc<AppState>>,
    Path(table_name): Path<String>,
    Query(params): Query<GetRemoteRecordsParams>,
) -> Result<Response, AppError> {
    let start_hlc = HLC {
        timestamp: params.start_hlc_ts,
        version: params.start_hlc_ver,
        node_id: Uuid::parse_str(&params.start_hlc_nid)?,
    };
    let end_hlc = HLC {
        timestamp: params.end_hlc_ts,
        version: params.end_hlc_ver,
        node_id: Uuid::parse_str(&params.end_hlc_nid)?,
    };
    let db = &state.db;
    let fk_resolver = state.fk_resolver.as_ref();

    let response_json = match table_name.as_str() {
        "albums" => serde_json::to_value(
            fetch_records_with_fk_payloads::<albums::Entity, _>(
                db,
                &start_hlc,
                &end_hlc,
                fk_resolver,
            )
            .await?,
        )?,
        "artists" => serde_json::to_value(
            fetch_records_with_fk_payloads::<artists::Entity, _>(
                db,
                &start_hlc,
                &end_hlc,
                fk_resolver,
            )
            .await?,
        )?,
        "genres" => serde_json::to_value(
            fetch_records_with_fk_payloads::<genres::Entity, _>(
                db,
                &start_hlc,
                &end_hlc,
                fk_resolver,
            )
            .await?,
        )?,
        "media_cover_art" => serde_json::to_value(
            fetch_records_with_fk_payloads::<media_cover_art::Entity, _>(
                db,
                &start_hlc,
                &end_hlc,
                fk_resolver,
            )
            .await?,
        )?,
        "media_files" => serde_json::to_value(
            fetch_records_with_fk_payloads::<media_files::Entity, _>(
                db,
                &start_hlc,
                &end_hlc,
                fk_resolver,
            )
            .await?,
        )?,
        "media_file_albums" => serde_json::to_value(
            fetch_records_with_fk_payloads::<media_file_albums::Entity, _>(
                db,
                &start_hlc,
                &end_hlc,
                fk_resolver,
            )
            .await?,
        )?,
        "media_file_artists" => serde_json::to_value(
            fetch_records_with_fk_payloads::<media_file_artists::Entity, _>(
                db,
                &start_hlc,
                &end_hlc,
                fk_resolver,
            )
            .await?,
        )?,
        "media_file_genres" => serde_json::to_value(
            fetch_records_with_fk_payloads::<media_file_genres::Entity, _>(
                db,
                &start_hlc,
                &end_hlc,
                fk_resolver,
            )
            .await?,
        )?,
        _ => {
            return Err(AppError(anyhow!(
                "Unsupported table name for records: {}",
                table_name
            )))
        }
    };
    Ok(Json(response_json).into_response())
}

pub fn new_server_hlc(server_node_id: Uuid) -> HLC {
    HLC::new(server_node_id)
}

/// Applies a batch of `SyncOperation`s to the remote data source for a specific table.
/// Corresponds to `RemoteDataSource::apply_remote_changes`
pub async fn apply_remote_changes_handler(
    State(state): State<Arc<AppState>>,
    Path(table_name): Path<String>,
    body: Bytes,
) -> Result<Json<HLC>, AppError> {
    info!("Request: apply_remote_changes for table '{}'", table_name);

    let db = &state.db;
    let fk_resolver = state.fk_resolver.as_ref();
    let server_node_id = state.node_id;

    let txn = db.begin().await.context("Failed to begin transaction")?;
    debug!(
        "Transaction started for apply_remote_changes on table {}",
        table_name
    );

    // Initialize with HLC::new(server_node_id) for the earliest possible HLC from this server.
    // This ensures max_hlc_in_batch always has a valid HLC with the correct node_id.
    let mut max_hlc_in_batch = HLC::new(server_node_id);
    let mut operations_processed_count = 0;
    let mut any_hlc_updated = false; // Track if any operation provided an HLC

    macro_rules! process_entity_changes {
        ($entity:ty, $active_model_ty:ty, $model_ty:ty) => {{
            type Entity = $entity;
            type Model = $model_ty;

            let operations: Vec<SyncOperation<Model>> = serde_json::from_slice(&body)
                .with_context(|| format!("Failed to deserialize operations for table {}", table_name))?;
            debug!("Deserialized {} operations for table {}", operations.len(), table_name);

            for op in operations {
                match op {
                    SyncOperation::InsertRemote(model, fk_payload) => {
                        debug!("Processing InsertRemote for sync_id: {} in table {}", model.unique_id(), table_name);

                        // Convert Model to ActiveModel.
                        // Your `ModelWithForeignKeyOps` should likely define `into_active_model_for_insert`
                        // or you rely on `model.clone().into_active_model()` if it sets HLCs.
                        // For now, let's assume `model.clone().into()` works or an ActiveModel can be constructed.
                        // And then `remap_and_set_foreign_keys` and `set_hlc_fields` are called.
                        // Let's assume `model.into_active_model()` is the way.
                        let mut active_model = model.clone().into_active_model();
                        // Or if you need a new ActiveModel and then populate:
                        // let mut active_model = ActiveModel::new();
                        // active_model.set_from_model(&model); // hypothetical helper

                        fk_resolver
                            .remap_and_set_foreign_keys(&mut active_model, &fk_payload, &txn)
                            .await
                            .with_context(|| format!("Failed to remap FKs for InsertRemote on sync_id {}", model.unique_id()))?;

                        // Set HLC fields if they are not part of the `model.into_active_model()` or if they need to be server-generated/overridden
                        // For InsertRemote, the client provides HLCs.
                        // Assume `into_active_model()` handles setting these.

                        active_model.insert(&txn).await.with_context(|| {
                            format!("Failed to insert record with sync_id {} for table {}", model.unique_id(), table_name)
                        })?;

                        if let Some(hlc) = model.updated_at_hlc() {
                            max_hlc_in_batch = cmp_max(max_hlc_in_batch.clone(), hlc);
                            any_hlc_updated = true;
                        }
                        operations_processed_count +=1;
                    }
                    SyncOperation::UpdateRemote(model, fk_payload) => {
                        debug!("Processing UpdateRemote for sync_id: {} in table {}", model.unique_id(), table_name);

                        // Create active model directly from the incoming model
                        let mut active_model = model.clone().into_active_model();

                        // Remap foreign keys using the resolver
                        fk_resolver
                            .remap_and_set_foreign_keys(&mut active_model, &fk_payload, &txn)
                            .await
                            .with_context(|| format!("Failed to remap FKs for UpdateRemote on sync_id {}", model.unique_id()))?;

                        // Perform the update
                        active_model.update(&txn).await.with_context(|| {
                            format!("Failed to update record with sync_id {} for table {}", model.unique_id(), table_name)
                        })?;

                        if let Some(hlc) = model.updated_at_hlc() {
                            max_hlc_in_batch = cmp_max(max_hlc_in_batch.clone(), hlc);
                            any_hlc_updated = true;
                        }
                        operations_processed_count +=1;
                    }
                    SyncOperation::DeleteRemote(unique_id) => {
                        // ... (no changes here, DeleteRemote was likely fine)
                        debug!("Processing DeleteRemote for sync_id: {} in table {}", unique_id, table_name);
                        let res = <Entity as HLCModel>::delete_by_unique_id(&unique_id, &txn)
                            .await
                            .with_context(|| format!("Failed to delete record with sync_id {}", unique_id))?;
                        if res.rows_affected == 0 {
                            warn!("DeleteRemote: Record with sync_id {} not found or already deleted.", unique_id);
                        } else {
                             operations_processed_count +=1;
                        }
                    }
                    _ => {
                        warn!("Received unexpected SyncOperation variant in apply_remote_changes: {:?}", op);
                    }
                }
            }
        }};
    }

    match table_name.as_str() {
        "albums" => process_entity_changes!(albums::Entity, albums::ActiveModel, albums::Model),
        "artists" => process_entity_changes!(artists::Entity, artists::ActiveModel, artists::Model),
        "genres" => process_entity_changes!(genres::Entity, genres::ActiveModel, genres::Model),
        "media_cover_art" => process_entity_changes!(
            media_cover_art::Entity,
            media_cover_art::ActiveModel,
            media_cover_art::Model
        ),
        "media_files" => process_entity_changes!(
            media_files::Entity,
            media_files::ActiveModel,
            media_files::Model
        ),
        "media_file_albums" => process_entity_changes!(
            media_file_albums::Entity,
            media_file_albums::ActiveModel,
            media_file_albums::Model
        ),
        "media_file_artists" => process_entity_changes!(
            media_file_artists::Entity,
            media_file_artists::ActiveModel,
            media_file_artists::Model
        ),
        "media_file_genres" => process_entity_changes!(
            media_file_genres::Entity,
            media_file_genres::ActiveModel,
            media_file_genres::Model
        ),
        _ => {
            txn.rollback()
                .await
                .context("Rollback failed on unsupported table")?;
            return Err(AppError(anyhow!(
                "Unsupported table name for changes: {}",
                table_name
            )));
        }
    }

    txn.commit().await.context("Failed to commit transaction")?;
    debug!(
        "Transaction committed for apply_remote_changes on table {}",
        table_name
    );

    // If operations were processed and at least one HLC was updated from them, use max_hlc_in_batch.
    // Otherwise, (e.g., only deletes, or empty batch, or no HLCs provided in ops),
    // generate a new server HLC for the transaction time.
    let result_hlc = if operations_processed_count > 0 && any_hlc_updated {
        max_hlc_in_batch
    } else {
        HLC::new(server_node_id)
    };

    info!(
        "apply_remote_changes for table '{}' completed. Effective HLC: {}",
        table_name, result_hlc
    );
    Ok(Json(result_hlc))
}

/// Fetches the remote's perspective of the last sync HLC with the local node.
/// Corresponds to `RemoteDataSource::get_remote_last_sync_hlc`
pub async fn get_remote_last_sync_hlc_handler(
    State(state): State<Arc<AppState>>,
    Path((table_name, client_node_id_str)): Path<(String, String)>,
) -> Result<Json<Option<HLC>>, AppError> {
    info!(
        "Request: get_remote_last_sync_hlc for table '{}', client_node_id: {}",
        table_name, client_node_id_str
    );

    let client_node_id = Uuid::parse_str(&client_node_id_str)
        .with_context(|| format!("Invalid client_node_id UUID string: {}", client_node_id_str))?;

    // Assuming sync_record::Entity and its HLC methods are correctly defined
    let sync_log_model = sync_record::Entity::find()
        .filter(sync_record::Column::TableName.eq(table_name.clone()))
        .filter(sync_record::Column::ClientNodeId.eq(client_node_id))
        .one(&state.db)
        .await
        .with_context(|| {
            format!(
                "DB error fetching last sync HLC for table {} and client {}",
                table_name, client_node_id
            )
        })?;

    if let Some(log_entry) = sync_log_model {
        // Assuming `log_entry` has fields `last_sync_hlc_ts`, `last_sync_hlc_v`, `last_sync_hlc_nid`
        // or a method `get_hlc()` that constructs it.
        // Let's assume direct field access for this example based on sync_record entity.
        let hlc = parse_hlc(
            &log_entry.last_sync_hlc_ts,
            log_entry.last_sync_hlc_ver,
            &log_entry.last_sync_hlc_nid,
        )?;
        Ok(Json(Some(hlc)))
    } else {
        Ok(Json(None))
    }
}
