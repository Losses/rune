use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use axum::{
    routing::{get, post},
    serve, Router,
};
use chrono::Utc;
use sea_orm::{
    prelude::Decimal, ActiveModelTrait, ActiveValue, ColumnTrait, ConnectOptions, ConnectionTrait,
    Database, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, Schema,
};
use serde::{Deserialize, Serialize};
use sync::core::SyncTableMetadata;
use tokio::{net::TcpListener, task::JoinHandle};
use uuid::Uuid;

use ::database::{
    entities::{
        albums, artists, genres, media_cover_art, media_file_albums, media_file_artists,
        media_file_genres, media_files, sync_record,
    },
    sync::{
        chunking::{
            apply_remote_changes_handler, get_node_id_handler, get_remote_chunks_handler,
            get_remote_last_sync_hlc_handler, get_remote_records_in_hlc_range_handler,
            get_remote_sub_chunks_handler, AppState, GetRemoteSubChunksPayload,
        },
        foreign_keys::RuneForeignKeyResolver,
        setup_and_run_sync, utils as sync_utils,
    },
};
use ::sync::{
    chunking::{ChunkingOptions, DataChunk},
    core::{RemoteDataSource, RemoteRecordsWithPayload, SyncOperation},
    hlc::{HLCModel, HLCRecord, SyncTaskContext, HLC},
    // Presuming TableSyncResult and SyncTableMetadata are exposed from here or a sub-module like sync_scheduler
    sync_scheduler::TableSyncResult, // Added this for clarity, adjust path if needed
};

#[derive(Debug)]
struct RemoteHttpDataSource {
    base_url: String,
    client: reqwest::Client,
}

impl RemoteHttpDataSource {
    fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            client: reqwest::Client::new(),
        }
    }

    fn build_url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

#[async_trait::async_trait]
impl RemoteDataSource for RemoteHttpDataSource {
    async fn get_remote_node_id(&self) -> Result<Uuid> {
        let url = self.build_url("/node-id");
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        let node_id: Uuid = resp.json().await?;
        Ok(node_id)
    }

    async fn get_remote_chunks<E>(
        &self,
        table_name: &str,
        after_hlc: Option<&HLC>,
    ) -> Result<Vec<DataChunk>>
    where
        E: HLCModel + EntityTrait + Send + Sync,
        E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize,
    {
        let url = self.build_url(&format!("/tables/{}/chunks", table_name));
        let mut query_params = Vec::new();
        if let Some(hlc) = after_hlc {
            query_params.push(("after_hlc_ts", hlc.timestamp.to_string()));
            query_params.push(("after_hlc_ver", hlc.version.to_string()));
            query_params.push(("after_hlc_nid", hlc.node_id.to_string()));
        }
        let resp = self
            .client
            .get(&url)
            .query(&query_params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    async fn get_remote_sub_chunks<E>(
        &self,
        table_name: &str,
        parent_chunk: &DataChunk,
        sub_chunk_size: u64,
    ) -> Result<Vec<DataChunk>>
    where
        E: HLCModel + EntityTrait + Send + Sync,
        E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize,
    {
        let url = self.build_url(&format!("/tables/{}/sub-chunks", table_name));
        let payload = GetRemoteSubChunksPayload {
            parent_chunk: parent_chunk.clone(),
            sub_chunk_size,
        };
        let resp = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    async fn get_remote_records_in_hlc_range<E>(
        &self,
        table_name: &str,
        start_hlc: &HLC,
        end_hlc: &HLC,
    ) -> Result<RemoteRecordsWithPayload<E::Model>>
    where
        E: HLCModel + EntityTrait + Send + Sync,
        E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize,
    {
        let url = self.build_url(&format!("/tables/{}/records", table_name));
        let query_params = [
            ("start_hlc_ts", start_hlc.timestamp.to_string()),
            ("start_hlc_ver", start_hlc.version.to_string()),
            ("start_hlc_nid", start_hlc.node_id.to_string()),
            ("end_hlc_ts", end_hlc.timestamp.to_string()),
            ("end_hlc_ver", end_hlc.version.to_string()),
            ("end_hlc_nid", end_hlc.node_id.to_string()),
        ];
        let resp = self
            .client
            .get(&url)
            .query(&query_params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    async fn apply_remote_changes<E>(
        &self,
        table_name: &str,
        operations: Vec<SyncOperation<E::Model>>,
    ) -> Result<HLC>
    where
        E: HLCModel + EntityTrait + Send + Sync,
        E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize,
    {
        let url = self.build_url(&format!("/tables/{}/changes", table_name));
        let resp = self
            .client
            .post(&url)
            .json(&operations)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    async fn get_remote_last_sync_hlc(
        &self,
        table_name: &str,
        local_node_id: Uuid,
    ) -> Result<Option<HLC>> {
        let url = self.build_url(&format!(
            "/tables/{}/last-sync-hlc/{}",
            table_name, local_node_id
        ));
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }
}

async fn setup_db(is_server: bool) -> Result<DatabaseConnection> {
    let db_id = if is_server { "server" } else { "client" };
    // Unique DB name per test execution to avoid conflicts with shared in-memory DBs
    let db_name = format!(
        "file:memdb_sync_test_{}_{}?mode=memory&cache=shared",
        db_id,
        Uuid::new_v4()
    );

    let mut opt = ConnectOptions::new(db_name);
    opt.sqlx_logging(false) // Disable verbose SQL logging from sea-orm unless debugging
        .acquire_timeout(Duration::from_secs(10)); // Longer timeout for CI

    let db = Database::connect(opt).await?;
    setup_schema_for_db(&db).await?;
    Ok(db)
}

async fn setup_schema_for_db(db: &DatabaseConnection) -> Result<()> {
    let schema = Schema::new(db.get_database_backend());
    let builder = db.get_database_backend();

    macro_rules! create_table {
        ($entity:path) => {
            db.execute(builder.build(schema.create_table_from_entity($entity).if_not_exists()))
                .await
                .with_context(|| format!("Failed to create table for {:?}", stringify!($entity)))?;
        };
    }

    create_table!(albums::Entity);
    create_table!(artists::Entity);
    create_table!(genres::Entity);
    create_table!(media_cover_art::Entity);
    create_table!(media_files::Entity);
    create_table!(media_file_albums::Entity);
    create_table!(media_file_artists::Entity);
    create_table!(media_file_genres::Entity);
    create_table!(sync_record::Entity);

    Ok(())
}

struct TestServer {
    addr: SocketAddr,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    handle: JoinHandle<Result<()>>,
    node_id: Uuid,
}

async fn start_server(db: DatabaseConnection) -> Result<TestServer> {
    let server_node_id = Uuid::new_v4();
    let app_state = Arc::new(AppState {
        db,
        node_id: server_node_id,
        fk_resolver: Arc::new(RuneForeignKeyResolver),
        default_chunking_options: ChunkingOptions::default(server_node_id),
    });

    let app = Router::new()
        .route("/node-id", get(get_node_id_handler))
        .route("/tables/:table_name/chunks", get(get_remote_chunks_handler))
        .route(
            "/tables/:table_name/sub-chunks",
            post(get_remote_sub_chunks_handler),
        )
        .route(
            "/tables/:table_name/records",
            get(get_remote_records_in_hlc_range_handler),
        )
        .route(
            "/tables/:table_name/changes",
            post(apply_remote_changes_handler),
        )
        .route(
            "/tables/:table_name/last-sync-hlc/:client_node_id",
            get(get_remote_last_sync_hlc_handler),
        )
        .with_state(app_state.clone());

    let port = portpicker::pick_unused_port().context("No free ports")?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind to {}", addr))?;

    let handle = tokio::spawn(async move {
        serve(listener, app.into_make_service())
            .with_graceful_shutdown(async {
                shutdown_rx.await.ok();
                println!("Graceful shutdown signal received, server shutting down.");
            })
            .await
            .context("Axum server error")
    });

    // Brief pause to ensure server starts
    tokio::time::sleep(Duration::from_millis(200)).await;

    Ok(TestServer {
        addr,
        shutdown_tx,
        handle,
        node_id: server_node_id,
    })
}

#[tokio::test]
async fn test_initial_sync_empty_databases() -> Result<()> {
    let _ = env_logger::try_init();

    let server_db = setup_db(true).await.context("Server DB setup failed")?;
    let client_db = setup_db(false).await.context("Client DB setup failed")?;

    let test_server = start_server(server_db.clone())
        .await
        .context("Server start failed")?;
    let client_node_id = Uuid::new_v4();

    let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", test_server.addr));
    let hlc_task_context = SyncTaskContext::new(client_node_id);

    let results: Vec<TableSyncResult> = setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await
    .context("Sync execution failed")?;

    for result in results {
        assert!(
            result.is_success(),
            "Sync job for table '{}' failed: {:?}",
            result.table_name_str(),
            result.get_error()
        );
        let metadata = result.unwrap_metadata();
        // We can assert the table name is correctly reported.
        // The exact value of last_sync_hlc for an empty sync depends on initialization logic.
        // For example, it might be the client's HLC at the start of the sync for that table.
        // Or a minimal HLC like HLC::default() or HLC::new(client_node_id) if it was just initialized.
        // For now, we'll just check that the table name is one of the expected synced tables.
        let known_tables = ["albums", "media_cover_art", "media_files", "sync_record"]; // Add other tables if they are part of the default sync
        assert!(
            known_tables.contains(&metadata.table_name.as_str()),
            "Unexpected table in sync metadata: {}",
            metadata.table_name
        );
        // Not asserting specific last_sync_hlc here as its initial value might vary.
    }

    assert_eq!(albums::Entity::find().count(&client_db).await?, 0);
    assert_eq!(albums::Entity::find().count(&server_db).await?, 0);

    test_server.shutdown_tx.send(()).ok();
    test_server.handle.await??;
    Ok(())
}

#[tokio::test]
async fn test_client_inserts_album_synced_to_server() -> Result<()> {
    let _ = env_logger::try_init();

    let server_db = setup_db(true).await?;
    let client_db = setup_db(false).await?;
    let test_server = start_server(server_db.clone()).await?;
    let client_node_id = Uuid::new_v4();
    let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", test_server.addr));
    let hlc_task_context = SyncTaskContext::new(client_node_id);

    // 1. Client inserts data
    let album_creation_hlc = HLC::new(client_node_id);
    let new_album_pk_id = 1;
    let new_album_hlc_uuid = Uuid::new_v4().to_string();

    let new_album = albums::ActiveModel {
        id: ActiveValue::Set(new_album_pk_id),
        name: ActiveValue::Set("Client Test Album".to_string()),
        group: ActiveValue::Set("Test Group".to_string()),
        hlc_uuid: ActiveValue::Set(new_album_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(album_creation_hlc.timestamp.to_string()),
        created_at_hlc_ver: ActiveValue::Set(album_creation_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(album_creation_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(album_creation_hlc.timestamp.to_string()),
        updated_at_hlc_ver: ActiveValue::Set(album_creation_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(album_creation_hlc.node_id.to_string()),
    };
    new_album.insert(&client_db).await?;

    // 2. Run sync
    let results: Vec<TableSyncResult> = setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    // 3. Assertions
    // r is &TableSyncResult
    let albums_job_result_item_ref: &TableSyncResult = results
        .iter()
        .find(|r| {
            // r is &TableSyncResult
            r.get_metadata().is_some_and(|s| s.table_name == "albums")
        })
        .expect("Albums job result not found"); // Use expect for better error

    // Use new helper method. Panics if albums_job_result_item_ref is Failure.
    let albums_job_metadata: &SyncTableMetadata = albums_job_result_item_ref.metadata_ref();

    assert_eq!(albums_job_metadata.table_name, "albums");
    assert_eq!(
        albums_job_metadata.last_sync_hlc, album_creation_hlc,
        "Last sync HLC should match the HLC of the synced client album"
    );

    let server_album = albums::Entity::find_by_id(new_album_pk_id)
        .one(&server_db)
        .await?
        .context("Album not found on server")?;

    assert_eq!(server_album.name, "Client Test Album");
    assert_eq!(server_album.group, "Test Group");
    assert_eq!(server_album.hlc_uuid, new_album_hlc_uuid);
    assert_eq!(
        server_album.updated_at_hlc_ts,
        album_creation_hlc.timestamp.to_string()
    );
    assert_eq!(
        server_album.updated_at_hlc_ver,
        album_creation_hlc.version as i32
    );
    assert_eq!(
        server_album.updated_at_hlc_nid,
        album_creation_hlc.node_id.to_string()
    );

    test_server.shutdown_tx.send(()).ok();
    test_server.handle.await??;
    Ok(())
}

#[tokio::test]
async fn test_server_inserts_album_synced_to_client() -> Result<()> {
    let _ = env_logger::try_init();

    let server_db = setup_db(true).await?;
    let client_db = setup_db(false).await?;
    let test_server = start_server(server_db.clone()).await?;
    let client_node_id = Uuid::new_v4();
    let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", test_server.addr));
    let hlc_task_context = SyncTaskContext::new(client_node_id);

    // 1. Server inserts data
    let album_creation_hlc = HLC::new(test_server.node_id);
    let new_album_pk_id = 2;
    let new_album_hlc_uuid = Uuid::new_v4().to_string();

    let new_album_server = albums::ActiveModel {
        id: ActiveValue::Set(new_album_pk_id),
        name: ActiveValue::Set("Server Test Album".to_string()),
        group: ActiveValue::Set("Server Group".to_string()),
        hlc_uuid: ActiveValue::Set(new_album_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(album_creation_hlc.timestamp.to_string()),
        created_at_hlc_ver: ActiveValue::Set(album_creation_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(album_creation_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(album_creation_hlc.timestamp.to_string()),
        updated_at_hlc_ver: ActiveValue::Set(album_creation_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(album_creation_hlc.node_id.to_string()),
    };
    new_album_server.insert(&server_db).await?;

    // 2. Run sync
    let results: Vec<TableSyncResult> = setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    // 3. Assertions
    let albums_job_result_item_ref: &TableSyncResult = results
        .iter()
        .find(|r| r.get_metadata().is_some_and(|s| s.table_name == "albums"))
        .expect("Albums job result not found");
    let albums_job_metadata: &SyncTableMetadata = albums_job_result_item_ref.metadata_ref();

    assert_eq!(albums_job_metadata.table_name, "albums");
    assert_eq!(
        albums_job_metadata.last_sync_hlc, album_creation_hlc,
        "Last sync HLC should match the HLC of the synced server album"
    );

    let client_album = albums::Entity::find_by_id(new_album_pk_id)
        .one(&client_db)
        .await?
        .context("Album not found on client")?;

    assert_eq!(client_album.name, "Server Test Album");
    assert_eq!(client_album.group, "Server Group");
    assert_eq!(client_album.hlc_uuid, new_album_hlc_uuid);
    assert_eq!(
        client_album.updated_at_hlc_ts,
        album_creation_hlc.timestamp.to_string()
    );
    assert_eq!(
        client_album.updated_at_hlc_ver,
        album_creation_hlc.version as i32
    );
    assert_eq!(
        client_album.updated_at_hlc_nid,
        album_creation_hlc.node_id.to_string()
    );

    test_server.shutdown_tx.send(()).ok();
    test_server.handle.await??;
    Ok(())
}

#[tokio::test]
async fn test_bidirectional_sync_different_albums() -> Result<()> {
    let _ = env_logger::try_init();

    let server_db = setup_db(true).await?;
    let client_db = setup_db(false).await?;
    let test_server = start_server(server_db.clone()).await?;
    let client_node_id = Uuid::new_v4();
    let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", test_server.addr));
    let hlc_task_context = SyncTaskContext::new(client_node_id);

    // Client inserts Album C
    let album_creation_hlc = HLC::new(client_node_id);
    let album_c_pk_id = 3;
    let album_c_hlc_uuid = Uuid::new_v4().to_string();
    albums::ActiveModel {
        id: ActiveValue::Set(album_c_pk_id),
        name: ActiveValue::Set("Album C (from Client)".to_string()),
        group: ActiveValue::Set("Client Group C".to_string()),
        hlc_uuid: ActiveValue::Set(album_c_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(album_creation_hlc.timestamp.to_string()),
        created_at_hlc_ver: ActiveValue::Set(album_creation_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(album_creation_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(album_creation_hlc.timestamp.to_string()),
        updated_at_hlc_ver: ActiveValue::Set(album_creation_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(album_creation_hlc.node_id.to_string()),
    }
    .insert(&client_db)
    .await?;

    // Server inserts Album S
    let server_hlc = HLC::new(test_server.node_id);
    let album_s_pk_id = 4;
    let album_s_hlc_uuid = Uuid::new_v4().to_string();
    albums::ActiveModel {
        id: ActiveValue::Set(album_s_pk_id),
        name: ActiveValue::Set("Album S (from Server)".to_string()),
        group: ActiveValue::Set("Server Group S".to_string()),
        hlc_uuid: ActiveValue::Set(album_s_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(server_hlc.timestamp.to_string()),
        created_at_hlc_ver: ActiveValue::Set(server_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(server_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(server_hlc.timestamp.to_string()),
        updated_at_hlc_ver: ActiveValue::Set(server_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(server_hlc.node_id.to_string()),
    }
    .insert(&server_db)
    .await?;

    // Run sync
    let results: Vec<TableSyncResult> = setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    let albums_job_result_item_ref: &TableSyncResult = results
        .iter()
        .find(|r| {
            // r is &TableSyncResult
            r.get_metadata().is_some_and(|s| s.table_name == "albums")
        })
        .expect("Albums job result not found");
    let albums_job_metadata: &SyncTableMetadata = albums_job_result_item_ref.metadata_ref();

    assert_eq!(albums_job_metadata.table_name, "albums");
    assert_eq!(
        albums_job_metadata.last_sync_hlc, album_creation_hlc,
        "Last sync HLC should match the HLC of the synced server album"
    );

    assert!(albums::Entity::find_by_id(album_c_pk_id)
        .one(&client_db)
        .await?
        .is_some());
    assert!(albums::Entity::find_by_id(album_s_pk_id)
        .one(&client_db)
        .await?
        .is_some());
    assert_eq!(albums::Entity::find().count(&client_db).await?, 2);

    assert!(albums::Entity::find_by_id(album_c_pk_id)
        .one(&server_db)
        .await?
        .is_some());
    assert!(albums::Entity::find_by_id(album_s_pk_id)
        .one(&server_db)
        .await?
        .is_some());
    assert_eq!(albums::Entity::find().count(&server_db).await?, 2);

    test_server.shutdown_tx.send(()).ok();
    test_server.handle.await??;
    Ok(())
}

#[tokio::test]
async fn test_sync_media_files_with_cover_art_fk() -> Result<()> {
    let _ = env_logger::try_init();

    let server_db = setup_db(true).await?;
    let client_db = setup_db(false).await?;
    let test_server = start_server(server_db.clone()).await?;
    let client_node_id = Uuid::new_v4();
    let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", test_server.addr));
    let hlc_task_context = SyncTaskContext::new(client_node_id);

    let mut client_hlc = HLC::new(client_node_id);

    // Client: Insert CoverArt CA1, MediaFile MF1 -> CA1
    let ca1_pk_id = 1;
    let ca1_hlc_uuid = Uuid::new_v4().to_string();
    let ca1_client = media_cover_art::ActiveModel {
        id: ActiveValue::Set(ca1_pk_id),
        file_hash: ActiveValue::Set("ca1_hash_client".to_string()),
        binary: ActiveValue::Set(vec![1, 1, 1]),
        primary_color: ActiveValue::Set(Some(0xAAAAAA)),
        hlc_uuid: ActiveValue::Set(ca1_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(client_hlc.timestamp.to_string()),
        created_at_hlc_ver: ActiveValue::Set(client_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(client_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(client_hlc.timestamp.to_string()),
        updated_at_hlc_ver: ActiveValue::Set(client_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(client_hlc.node_id.to_string()),
    };
    ca1_client.insert(&client_db).await?;
    client_hlc.increment();

    let mf1_pk_id = 10;
    let mf1_hlc_uuid = Uuid::new_v4().to_string();
    let mf1_client = media_files::ActiveModel {
        id: ActiveValue::Set(mf1_pk_id),
        file_name: ActiveValue::Set("client_song_1".to_string()),
        directory: ActiveValue::Set("/music/client/".to_string()),
        extension: ActiveValue::Set("mp3".to_string()),
        file_hash: ActiveValue::Set("mf1_hash_client".to_string()),
        last_modified: ActiveValue::Set(Utc::now().to_rfc3339()),
        cover_art_id: ActiveValue::Set(Some(ca1_pk_id)),
        sample_rate: ActiveValue::Set(44100),
        duration: ActiveValue::Set(Decimal::new(180, 0)), // 180s
        hlc_uuid: ActiveValue::Set(mf1_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(client_hlc.timestamp.to_string()),
        created_at_hlc_ver: ActiveValue::Set(client_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(client_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(client_hlc.timestamp.to_string()),
        updated_at_hlc_ver: ActiveValue::Set(client_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(client_hlc.node_id.to_string()),
    };
    mf1_client.insert(&client_db).await?;

    // First sync (Client -> Server)
    let _results1: Vec<TableSyncResult> = setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    let server_ca1 = media_cover_art::Entity::find()
        .filter(media_cover_art::Column::HlcUuid.eq(ca1_hlc_uuid.clone()))
        .one(&server_db)
        .await?
        .context("CA1 not on server")?;
    assert_eq!(server_ca1.file_hash, "ca1_hash_client");
    assert_eq!(server_ca1.binary, vec![1, 1, 1]);
    assert_eq!(server_ca1.primary_color, Some(0xAAAAAA));

    let server_mf1 = media_files::Entity::find()
        .filter(media_files::Column::HlcUuid.eq(mf1_hlc_uuid.clone()))
        .one(&server_db)
        .await?
        .context("MF1 not on server")?;
    assert_eq!(server_mf1.file_name, "client_song_1");
    assert_eq!(server_mf1.directory, "/music/client/");
    assert_eq!(server_mf1.cover_art_id, Some(server_ca1.id)); // FK check

    // Server: Insert CoverArt CA2, MediaFile MF2 -> CA2
    let mut server_hlc = HLC::new(test_server.node_id);
    let ca2_pk_id = 2;
    let ca2_hlc_uuid = Uuid::new_v4().to_string();
    let ca2_server = media_cover_art::ActiveModel {
        id: ActiveValue::Set(ca2_pk_id),
        file_hash: ActiveValue::Set("ca2_hash_server".to_string()),
        binary: ActiveValue::Set(vec![2, 2, 2]),
        primary_color: ActiveValue::Set(Some(0xBBBBBB)),
        hlc_uuid: ActiveValue::Set(ca2_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(server_hlc.timestamp.to_string()),
        created_at_hlc_ver: ActiveValue::Set(server_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(server_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(server_hlc.timestamp.to_string()),
        updated_at_hlc_ver: ActiveValue::Set(server_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(server_hlc.node_id.to_string()),
    };
    ca2_server.insert(&server_db).await?;
    server_hlc.increment();

    let mf2_pk_id = 20;
    let mf2_hlc_uuid = Uuid::new_v4().to_string();
    let mf2_server = media_files::ActiveModel {
        id: ActiveValue::Set(mf2_pk_id),
        file_name: ActiveValue::Set("server_song_2".to_string()),
        directory: ActiveValue::Set("/audio/server/".to_string()),
        extension: ActiveValue::Set("ogg".to_string()),
        file_hash: ActiveValue::Set("mf2_hash_server".to_string()),
        last_modified: ActiveValue::Set(Utc::now().to_rfc3339()),
        cover_art_id: ActiveValue::Set(Some(ca2_pk_id)),
        sample_rate: ActiveValue::Set(48000),
        duration: ActiveValue::Set(Decimal::new(2405, 1)), // 240.5s
        hlc_uuid: ActiveValue::Set(mf2_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(server_hlc.timestamp.to_string()),
        created_at_hlc_ver: ActiveValue::Set(server_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(server_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(server_hlc.timestamp.to_string()),
        updated_at_hlc_ver: ActiveValue::Set(server_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(server_hlc.node_id.to_string()),
    };
    mf2_server.insert(&server_db).await?;

    // Second sync (Server -> Client, and client also checks for updates)
    let _results_2: Vec<TableSyncResult> = setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    let client_ca2 = media_cover_art::Entity::find()
        .filter(media_cover_art::Column::HlcUuid.eq(ca2_hlc_uuid.clone()))
        .one(&client_db)
        .await?
        .context("CA2 not on client")?;
    assert_eq!(client_ca2.file_hash, "ca2_hash_server");
    assert_eq!(client_ca2.binary, vec![2, 2, 2]);
    assert_eq!(client_ca2.primary_color, Some(0xBBBBBB));

    let client_mf2 = media_files::Entity::find()
        .filter(media_files::Column::HlcUuid.eq(mf2_hlc_uuid.clone()))
        .one(&client_db)
        .await?
        .context("MF2 not on client")?;
    assert_eq!(client_mf2.file_name, "server_song_2");
    assert_eq!(client_mf2.directory, "/audio/server/");
    assert_eq!(client_mf2.sample_rate, 48000);
    assert_eq!(client_mf2.duration, Decimal::new(2405, 1));
    assert_eq!(client_mf2.cover_art_id, Some(client_ca2.id)); // FK check

    // Verify counts
    assert_eq!(media_cover_art::Entity::find().count(&client_db).await?, 2);
    assert_eq!(media_files::Entity::find().count(&client_db).await?, 2);
    assert_eq!(media_cover_art::Entity::find().count(&server_db).await?, 2);
    assert_eq!(media_files::Entity::find().count(&server_db).await?, 2);

    test_server.shutdown_tx.send(()).ok();
    test_server.handle.await??;
    Ok(())
}

#[tokio::test]
async fn test_get_remote_last_sync_hlc() -> Result<()> {
    let _ = env_logger::try_init();

    let server_db = setup_db(true).await?;
    let client_db = setup_db(false).await?;
    let test_server = start_server(server_db.clone()).await?;
    let client_node_id = Uuid::new_v4();
    let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", test_server.addr));
    let hlc_task_context = SyncTaskContext::new(client_node_id);

    let last_hlc_before_sync = remote_data_source
        .get_remote_last_sync_hlc("albums", client_node_id)
        .await?;
    assert!(last_hlc_before_sync.is_none());

    let album_creation_hlc = HLC::new(client_node_id);
    let album_pk_id = 5;
    let album_hlc_uuid = Uuid::new_v4().to_string();
    let album_name = "Album for Last Sync HLC Test".to_string();
    albums::ActiveModel {
        id: ActiveValue::Set(album_pk_id),
        name: ActiveValue::Set(album_name.clone()),
        group: ActiveValue::Set("Last Sync Group".to_string()),
        hlc_uuid: ActiveValue::Set(album_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(album_creation_hlc.timestamp.to_string()),
        created_at_hlc_ver: ActiveValue::Set(album_creation_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(album_creation_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(album_creation_hlc.timestamp.to_string()),
        updated_at_hlc_ver: ActiveValue::Set(album_creation_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(album_creation_hlc.node_id.to_string()),
    }
    .insert(&client_db)
    .await?;

    let results: Vec<TableSyncResult> = setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    let albums_job_metadata_owned: SyncTableMetadata = results
        .into_iter()
        .find(|r: &TableSyncResult| r.get_metadata().is_some_and(|s| s.table_name == "albums"))
        .expect("Albums job result not found") // This returns TableSyncResult
        .unwrap_metadata(); // Consumes TableSyncResult -> SyncTableMetadata

    let last_hlc_after_sync_opt = remote_data_source
        .get_remote_last_sync_hlc("albums", client_node_id)
        .await?;

    assert_eq!(
        last_hlc_after_sync_opt, Some(albums_job_metadata_owned.last_sync_hlc), // This is now Some(HLC) == Option<HLC>
        "Last sync HLC from server API does not match HLC from client's sync metadata after client sent data"
    );

    let server_album = albums::Entity::find()
        .filter(albums::Column::HlcUuid.eq(album_hlc_uuid))
        .one(&server_db)
        .await?
        .context("Album not found on server after sync")?;
    assert_eq!(server_album.name, album_name);

    let expected_hlc_from_op = HLC {
        timestamp: album_creation_hlc.timestamp,
        version: album_creation_hlc.version,
        node_id: album_creation_hlc.node_id,
    };

    let server_sync_record = sync_record::Entity::find()
        .filter(sync_record::Column::TableName.eq("albums"))
        .filter(sync_record::Column::ClientNodeId.eq(client_node_id))
        .one(&server_db)
        .await?
        .context("sync_record not found on server")?;

    let server_stored_hlc = sync_utils::parse_hlc(
        &server_sync_record.last_sync_hlc_ts,
        server_sync_record.last_sync_hlc_ver,
        &server_sync_record.last_sync_hlc_nid,
    )?;

    assert_eq!(
        Some(server_stored_hlc.clone()),
        last_hlc_after_sync_opt, // Compare Option<HLC> with Option<HLC>
        "Server's stored HLC in sync_record table mismatches API response"
    );
    assert!(
        server_stored_hlc >= expected_hlc_from_op,
        "Server's stored HLC should be >= HLC of the operation. Server: {}, Op: {}",
        server_stored_hlc,
        expected_hlc_from_op
    );

    test_server.shutdown_tx.send(()).ok();
    test_server.handle.await??;
    Ok(())
}

// TODO: Add more tests:
// - Updates (client updates, server updates)
// - Deletes (client deletes, server deletes)
// - Sync for junction tables (media_file_albums, media_file_artists, media_file_genres)
//   ensuring FKs are correct (e.g. media_file_albums.track_number).
// - More complex bidirectional scenarios (e.g. client updates X, server updates Y, then sync).
// - Conflict scenarios (if your HLC logic handles them, e.g., both update same record).
// - Test chunking and sub-chunking more directly if specific behaviors need validation beyond successful sync.
// - Test error conditions (e.g., server down during a call, malformed data).
