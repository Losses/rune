use std::{fmt::Debug, sync::Arc};

use anyhow::{Context, Result, anyhow};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use chrono::DateTime;
use log::{debug, error, info, warn};
use sea_orm::{
    ActiveModelBehavior, ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection,
    EntityTrait, IntoActiveModel, Iterable, ModelTrait, PrimaryKeyToColumn, PrimaryKeyTrait,
    QueryFilter, TransactionTrait, sea_query::OnConflict,
};
use serde::{Deserialize, Serialize};
use sync::chunking::{break_data_chunk, generate_data_chunks};
use uuid::Uuid;

use crate::{
    entities::{
        albums, artists, genres, media_cover_art, media_file_albums, media_file_artists,
        media_file_fingerprint, media_file_genres, media_file_similarity, media_files, sync_record,
    },
    sync::utils::parse_hlc,
};

use ::sync::{
    chunking::{ChunkingOptions, DataChunk, SubDataChunk},
    core::{RemoteRecordsWithPayload, SyncOperation},
    foreign_key::{ActiveModelWithForeignKeyOps, ForeignKeyResolver, ModelWithForeignKeyOps},
    hlc::{HLC, HLCModel, HLCQuery, HLCRecord, SyncTaskContext},
};

use super::foreign_keys::RuneForeignKeyResolver;

// Server's application state
#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub node_id: Uuid,
    pub fk_resolver: Arc<RuneForeignKeyResolver>,
    pub default_chunking_options: ChunkingOptions,
    // Add an HLC context for the server to generate timestamps
    pub hlc_context: Arc<SyncTaskContext>,
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

pub fn parse_optional_hlc_from_parts(
    ts: Option<String>,
    v: Option<u32>,
    nid_str_opt: Option<String>,
) -> Result<Option<HLC>> {
    match (ts, v, nid_str_opt) {
        (Some(timestamp_str), Some(version), Some(nid_str)) => {
            // Parse RFC3339 timestamp and convert to milliseconds
            let timestamp_ms = DateTime::parse_from_rfc3339(&timestamp_str)
                .with_context(|| format!("Invalid RFC3339 timestamp: {timestamp_str}"))?
                .timestamp_millis() as u64;

            let node_id = Uuid::parse_str(&nid_str)
                .with_context(|| format!("Invalid HLC node_id string: {nid_str}"))?;

            Ok(Some(HLC {
                timestamp_ms,
                version,
                node_id,
            }))
        }
        (None, None, None) => Ok(None),
        _ => Err(anyhow!(
            "Incomplete HLC provided. Must provide all parts (timestamp, version, node_id) or none."
        )),
    }
}

pub async fn get_node_id_handler(State(state): State<Arc<AppState>>) -> Json<Uuid> {
    Json(state.node_id)
}

#[derive(Deserialize, Debug)]
pub struct GetRemoteChunksParams {
    pub after_hlc_ts: Option<String>,
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
        "media_file_fingerprint" => {
            generate_data_chunks::<media_file_fingerprint::Entity, _>(
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
            )));
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
        "media_file_fingerprint" => {
            break_data_chunk::<media_file_fingerprint::Entity, _>(
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
            )));
        }
    };
    let result_chunks: Vec<DataChunk> = sub_chunks_metadata.into_iter().map(|s| s.chunk).collect();
    Ok(Json(result_chunks))
}

#[derive(Deserialize, Debug)]
pub struct GetRemoteRecordsParams {
    pub start_hlc_ts: String,
    pub start_hlc_ver: u32,
    pub start_hlc_nid: String,
    pub end_hlc_ts: String,
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
    let start_ts_ms = DateTime::parse_from_rfc3339(&params.start_hlc_ts)
        .with_context(|| format!("Invalid start_hlc_ts: {}", params.start_hlc_ts))?
        .timestamp_millis() as u64;
    let end_ts_ms = DateTime::parse_from_rfc3339(&params.end_hlc_ts)
        .with_context(|| format!("Invalid end_hlc_ts: {}", params.end_hlc_ts))?
        .timestamp_millis() as u64;

    let start_hlc = HLC {
        timestamp_ms: start_ts_ms,
        version: params.start_hlc_ver,
        node_id: Uuid::parse_str(&params.start_hlc_nid)?,
    };
    let end_hlc = HLC {
        timestamp_ms: end_ts_ms,
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
        "media_file_fingerprint" => serde_json::to_value(
            fetch_records_with_fk_payloads::<media_file_fingerprint::Entity, _>(
                db,
                &start_hlc,
                &end_hlc,
                fk_resolver,
            )
            .await?,
        )?,
        "media_file_similarity" => serde_json::to_value(
            fetch_records_with_fk_payloads::<media_file_similarity::Entity, _>(
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
            )));
        }
    };
    Ok(Json(response_json).into_response())
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ApplyChangesPayload<Model: HLCRecord> {
    pub operations: Vec<SyncOperation<Model>>,
    pub client_node_id: Uuid,
    pub new_last_sync_hlc: HLC,
}

/// Generic function to process sync operations for a given entity within a transaction.
/// This version includes extensive diagnostic logging.
#[allow(clippy::needless_borrow)]
async fn process_entity_changes<'a, E, FKR>(
    txn: &'a sea_orm::DatabaseTransaction,
    body: &'a Bytes,
    fk_resolver: &'a FKR,
    table_name: &str,
) -> Result<(u64, Uuid, HLC)>
where
    E: HLCModel + EntityTrait + Send + Sync,
    E::PrimaryKey: PrimaryKeyTrait,
    <E::PrimaryKey as PrimaryKeyTrait>::ValueType: Send + Debug,
    E::Model: HLCRecord
        + ModelTrait
        + Send
        + Sync
        + for<'de> Deserialize<'de>
        + IntoActiveModel<E::ActiveModel>
        + ModelWithForeignKeyOps
        + Debug
        + Clone,
    E::ActiveModel: ActiveModelBehavior + Send + Sync + ActiveModelWithForeignKeyOps,
    FKR: ForeignKeyResolver + Send + Sync,
{
    let payload: ApplyChangesPayload<E::Model> = serde_json::from_slice(body)
        .with_context(|| format!("Failed to deserialize full payload for table {table_name}"))?;

    let mut operations_processed_count = 0;

    for op in &payload.operations {
        match op {
            SyncOperation::InsertRemote(model, fk_payload)
            | SyncOperation::UpdateRemote(model, fk_payload) => {
                let op_type = if matches!(op, SyncOperation::InsertRemote(..)) {
                    "InsertRemote"
                } else {
                    "UpdateRemote"
                };
                let unique_id = model.unique_id();

                let existing_record = E::find()
                    .filter(E::unique_id_column().eq(unique_id.clone()))
                    .one(txn)
                    .await?;

                if let Some(db_record) = &existing_record
                    && model.updated_at_hlc() <= db_record.updated_at_hlc()
                {
                    debug!(
                        "[SERVER DIAGNOSTICS] Ignoring stale {op_type} for unique_id: {unique_id}"
                    );
                    continue;
                }

                debug!("[SERVER DIAGNOSTICS] Processing {op_type} for unique_id: {unique_id}");

                // Create an active model from the incoming model, where all fields will be `Set`.
                let json_val = serde_json::to_value(model.clone())?;
                let mut active_model = E::ActiveModel::from_json(json_val)?;

                // Remap FKs before inserting or updating.
                fk_resolver
                    .remap_and_set_foreign_keys(&mut active_model, &fk_payload, txn)
                    .await?;

                if existing_record.is_some() {
                    // Reset the primary key field(s) to `NotSet`.
                    // This is crucial to prevent trying to `SET` the primary key column
                    // during the update operation.
                    for pk_col in E::PrimaryKey::iter() {
                        active_model.reset(pk_col.into_column());
                    }

                    // Use `update_many` filtered by the logical unique_id.
                    // This correctly applies only the `Set` fields from `active_model`.
                    let res = E::update_many()
                        .set(active_model)
                        .filter(E::unique_id_column().eq(unique_id.clone()))
                        .exec(txn)
                        .await?;

                    if res.rows_affected == 0 {
                        // This indicates a problem, as we found an existing record but couldn't update it.
                        return Err(anyhow!(
                            "Record with unique_id {} was found but update affected 0 rows.",
                            unique_id
                        ));
                    }
                    operations_processed_count += res.rows_affected;
                } else {
                    // Reset the primary key to ensure the database generates a new one.
                    for pk_col in E::PrimaryKey::iter() {
                        active_model.reset(pk_col.into_column());
                    }

                    // Explicitly call insert.
                    E::insert(active_model).exec_without_returning(txn).await?;
                    operations_processed_count += 1;
                }
            }
            SyncOperation::DeleteRemote(unique_id) => {
                let res = E::delete_many()
                    .filter(E::unique_id_column().eq(unique_id.clone()))
                    .exec(txn)
                    .await?;
                if res.rows_affected > 0 {
                    operations_processed_count += 1;
                } else {
                    warn!(
                        "Delete operation for table {table_name}: Record with unique_id {unique_id} not found."
                    );
                }
            }
            op => {
                debug!(
                    "Server received a non-standard or no-op client operation, ignoring: {op:?}"
                );
            }
        }
    }

    Ok((
        operations_processed_count,
        payload.client_node_id,
        payload.new_last_sync_hlc,
    ))
}

/// Applies a batch of `SyncOperation`s to the remote data source for a specific table.
pub async fn apply_remote_changes_handler(
    State(state): State<Arc<AppState>>,
    Path(table_name): Path<String>,
    body: Bytes,
) -> Result<Json<HLC>, AppError> {
    info!(
        "[SERVER] Request: apply_remote_changes for table '{}' with body: {}",
        table_name,
        String::from_utf8_lossy(&body)
    );

    let db = &state.db;
    let fk_resolver = state.fk_resolver.as_ref();

    let txn = db.begin().await.context("Failed to begin transaction")?;
    debug!("Transaction started for apply_remote_changes on table {table_name}");

    let (operations_processed_count, client_node_id, new_last_sync_hlc) = match table_name.as_str()
    {
        "albums" => {
            process_entity_changes::<albums::Entity, _>(&txn, &body, fk_resolver, &table_name)
                .await?
        }
        "artists" => {
            process_entity_changes::<artists::Entity, _>(&txn, &body, fk_resolver, &table_name)
                .await?
        }
        "genres" => {
            process_entity_changes::<genres::Entity, _>(&txn, &body, fk_resolver, &table_name)
                .await?
        }
        "media_cover_art" => {
            process_entity_changes::<media_cover_art::Entity, _>(
                &txn,
                &body,
                fk_resolver,
                &table_name,
            )
            .await?
        }
        "media_files" => {
            process_entity_changes::<media_files::Entity, _>(&txn, &body, fk_resolver, &table_name)
                .await?
        }
        "media_file_albums" => {
            process_entity_changes::<media_file_albums::Entity, _>(
                &txn,
                &body,
                fk_resolver,
                &table_name,
            )
            .await?
        }
        "media_file_artists" => {
            process_entity_changes::<media_file_artists::Entity, _>(
                &txn,
                &body,
                fk_resolver,
                &table_name,
            )
            .await?
        }
        "media_file_genres" => {
            process_entity_changes::<media_file_genres::Entity, _>(
                &txn,
                &body,
                fk_resolver,
                &table_name,
            )
            .await?
        }
        "media_file_fingerprint" => {
            process_entity_changes::<media_file_fingerprint::Entity, _>(
                &txn,
                &body,
                fk_resolver,
                &table_name,
            )
            .await?
        }
        _ => {
            txn.rollback()
                .await
                .context("Rollback failed on unsupported table")?;
            return Err(AppError(anyhow!(
                "Unsupported table name for changes: {}",
                table_name
            )));
        }
    };

    debug!(
        "Processed {operations_processed_count} operations for table '{table_name}'. Upserting sync_record for client {client_node_id} with HLC {new_last_sync_hlc}."
    );
    let sync_record_model = sync_record::ActiveModel {
        table_name: Set(table_name.clone()),
        client_node_id: Set(client_node_id.to_string()),
        last_sync_hlc_ts: Set(new_last_sync_hlc.to_rfc3339()?),
        last_sync_hlc_ver: Set(new_last_sync_hlc.version as i32),
        last_sync_hlc_nid: Set(new_last_sync_hlc.node_id.to_string()),
        ..Default::default()
    };

    sync_record::Entity::insert(sync_record_model)
        .on_conflict(
            OnConflict::columns([
                sync_record::Column::TableName,
                sync_record::Column::ClientNodeId,
            ])
            .update_columns([
                sync_record::Column::LastSyncHlcTs,
                sync_record::Column::LastSyncHlcVer,
                sync_record::Column::LastSyncHlcNid,
            ])
            .to_owned(),
        )
        .exec(&txn)
        .await
        .context("Failed to upsert sync_record")?;

    txn.commit().await.context("Failed to commit transaction")?;
    debug!("Transaction committed for apply_remote_changes on table {table_name}");

    info!(
        "apply_remote_changes for table '{table_name}' completed. Effective HLC: {new_last_sync_hlc}"
    );
    Ok(Json(new_last_sync_hlc))
}

/// Fetches the remote's perspective of the last sync HLC with the local node.
pub async fn get_remote_last_sync_hlc_handler(
    State(state): State<Arc<AppState>>,
    Path((table_name, client_node_id_str)): Path<(String, String)>,
) -> Result<Json<Option<HLC>>, AppError> {
    info!(
        "Request: get_remote_last_sync_hlc for table '{table_name}', client_node_id: {client_node_id_str}"
    );

    let client_node_id = Uuid::parse_str(&client_node_id_str)?;
    let sync_log_model = sync_record::Entity::find()
        .filter(sync_record::Column::TableName.eq(table_name.clone()))
        .filter(sync_record::Column::ClientNodeId.eq(client_node_id.to_string()))
        .one(&state.db)
        .await?;

    if let Some(log_entry) = sync_log_model {
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
