use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use axum::{
    routing::{get, post},
    serve, Router,
};
use chrono::Utc;
use sea_orm::{
    prelude::Decimal, ActiveModelTrait, ActiveValue, ColumnTrait, ConnectOptions, Database,
    DatabaseConnection, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
};
use tokio::{net::TcpListener, task::JoinHandle};
use uuid::Uuid;

use ::database::{
    connection::initialize_db,
    entities::{albums, media_cover_art, media_files, sync_record},
    sync::{
        chunking::{
            apply_remote_changes_handler, get_node_id_handler, get_remote_chunks_handler,
            get_remote_last_sync_hlc_handler, get_remote_records_in_hlc_range_handler,
            get_remote_sub_chunks_handler, AppState,
        },
        data_source::RemoteHttpDataSource,
        foreign_keys::RuneForeignKeyResolver,
        setup_and_run_sync, utils as sync_utils,
    },
};
use ::sync::{
    chunking::ChunkingOptions,
    core::{RemoteDataSource, SyncTableMetadata},
    hlc::{SyncTaskContext, HLC},
    sync_scheduler::TableSyncResult,
};

async fn setup_db(is_server: bool, node_id: &str) -> Result<DatabaseConnection> {
    let side = if is_server { "server" } else { "client" };
    println!("Setting up database for the {} side", side);

    // Use a named, shared, in-memory SQLite database.
    // Each test run gets a unique DB name to prevent interference.
    // The `cache=shared` is crucial.
    let db_name = format!("test-db-{}-{}", side, Uuid::new_v4());
    let db_url = format!("sqlite:file:{}?mode=memory&cache=shared", db_name);

    println!("Initializing shared in-memory DB at: {}", db_url);

    let mut opt = ConnectOptions::new(&db_url);

    opt.sqlx_logging(true)
        .sqlx_logging_level(log::LevelFilter::Trace)
        .acquire_timeout(Duration::from_secs(10));

    let db = Database::connect(opt).await?;
    initialize_db(&db, node_id).await?;
    Ok(db)
}

pub struct TestServer {
    addr: SocketAddr,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    handle: JoinHandle<Result<()>>,
    pub node_id: Uuid,
    hlc_context: Arc<SyncTaskContext>,
}

async fn start_server(db: DatabaseConnection) -> Result<TestServer> {
    let server_node_id = Uuid::new_v4();

    let hlc_context = Arc::new(SyncTaskContext::new(server_node_id));

    let app_state = Arc::new(AppState {
        db,
        node_id: server_node_id,
        fk_resolver: Arc::new(RuneForeignKeyResolver),
        default_chunking_options: ChunkingOptions::default(server_node_id),
        hlc_context: hlc_context.clone(),
    });

    let app = Router::new()
        .route("/node-id", get(get_node_id_handler))
        .route(
            "/tables/{table_name}/chunks",
            get(get_remote_chunks_handler),
        )
        .route(
            "/tables/{table_name}/sub-chunks",
            post(get_remote_sub_chunks_handler),
        )
        .route(
            "/tables/{table_name}/records",
            get(get_remote_records_in_hlc_range_handler),
        )
        .route(
            "/tables/{table_name}/changes",
            post(apply_remote_changes_handler),
        )
        .route(
            "/tables/{table_name}/last-sync-hlc/{client_node_id}",
            get(get_remote_last_sync_hlc_handler),
        )
        .with_state(app_state.clone());

    let port = portpicker::pick_unused_port().context("No free ports")?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let listener = TcpListener::bind(addr).await?;

    let handle = tokio::spawn(async move {
        serve(listener, app.into_make_service())
            .with_graceful_shutdown(async {
                shutdown_rx.await.ok();
            })
            .await
            .context("Axum server error")
    });

    tokio::time::sleep(Duration::from_millis(200)).await;

    Ok(TestServer {
        addr,
        shutdown_tx,
        handle,
        node_id: server_node_id,
        hlc_context,
    })
}

#[tokio::test]
async fn test_initial_sync_empty_databases() -> Result<()> {
    let _ = env_logger::try_init();

    let server_db = setup_db(true, "").await.context("Server DB setup failed")?;
    let client_db = setup_db(false, "")
        .await
        .context("Client DB setup failed")?;

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
        let known_tables = [
            "albums",
            "artists",
            "genres",
            "media_cover_art",
            "media_files",
            "media_file_albums",
            "media_file_artists",
            "media_file_genres",
        ];
        assert!(
            known_tables.contains(&metadata.table_name.as_str()),
            "Unexpected table in sync metadata: {}",
            metadata.table_name
        );
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

    let server_db = setup_db(true, "").await?;
    let client_db = setup_db(false, "").await?;
    let test_server = start_server(server_db.clone()).await?;
    let client_node_id = Uuid::new_v4();
    let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", test_server.addr));
    let hlc_task_context = SyncTaskContext::new(client_node_id);

    // 1. Client inserts data
    // Use the HLC context to generate a realistic timestamp that will be > the initial sync HLC.
    let album_creation_hlc = hlc_task_context.generate_hlc();
    println!("[TEST] Creating album with HLC: {}", album_creation_hlc); // Add a log for good measure
    let new_album_pk_id = 1;
    let new_album_hlc_uuid = Uuid::new_v4().to_string();

    let new_album = albums::ActiveModel {
        id: ActiveValue::Set(new_album_pk_id),
        name: ActiveValue::Set("Client Test Album".to_string()),
        group: ActiveValue::Set("Test Group".to_string()),
        hlc_uuid: ActiveValue::Set(new_album_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(album_creation_hlc.to_rfc3339()?),
        created_at_hlc_ver: ActiveValue::Set(album_creation_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(album_creation_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(album_creation_hlc.to_rfc3339()?),
        updated_at_hlc_ver: ActiveValue::Set(album_creation_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(album_creation_hlc.node_id.to_string()),
    };
    new_album.insert(&client_db).await?;

    // FOR DEBUG PURPOSE
    let all_server_albums = albums::Entity::find().all(&client_db).await?;
    println!("**CLIENT_DB: {:#?}", all_server_albums);

    tokio::time::sleep(Duration::from_millis(5)).await;

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
    assert!(
        albums_job_metadata.last_sync_hlc >= album_creation_hlc,
        "Last sync HLC ({}) should be >= the HLC of the synced client album ({})",
        albums_job_metadata.last_sync_hlc,
        album_creation_hlc
    );

    let server_album = albums::Entity::find()
        .filter(albums::Column::HlcUuid.eq(new_album_hlc_uuid.clone()))
        .one(&server_db)
        .await?
        .context("Album not found on server by HLC_UUID")?;

    assert_eq!(server_album.name, "Client Test Album");
    assert_eq!(server_album.hlc_uuid, new_album_hlc_uuid);

    test_server.shutdown_tx.send(()).ok();
    test_server.handle.await??;
    Ok(())
}

#[tokio::test]
async fn test_server_inserts_album_synced_to_client() -> Result<()> {
    let _ = env_logger::try_init();

    let server_db = setup_db(true, "").await?;
    let client_db = setup_db(false, "").await?;
    let test_server = start_server(server_db.clone()).await?;
    let client_node_id = Uuid::new_v4();
    let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", test_server.addr));
    let hlc_task_context = SyncTaskContext::new(client_node_id);

    let album_creation_hlc = test_server.hlc_context.generate_hlc();
    let new_album_pk_id = 2;
    let new_album_hlc_uuid = Uuid::new_v4().to_string();

    albums::ActiveModel {
        id: ActiveValue::Set(new_album_pk_id),
        name: ActiveValue::Set("Server Test Album".to_string()),
        group: ActiveValue::Set("Server Group".to_string()),
        hlc_uuid: ActiveValue::Set(new_album_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(album_creation_hlc.to_rfc3339()?),
        created_at_hlc_ver: ActiveValue::Set(album_creation_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(album_creation_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(album_creation_hlc.to_rfc3339()?),
        updated_at_hlc_ver: ActiveValue::Set(album_creation_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(album_creation_hlc.node_id.to_string()),
    }
    .insert(&server_db)
    .await?;

    tokio::time::sleep(Duration::from_millis(10)).await;

    let results: Vec<TableSyncResult> = setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    let albums_job_metadata = results
        .iter()
        .find(|r| r.get_metadata().is_some_and(|s| s.table_name == "albums"))
        .expect("Albums job result not found")
        .metadata_ref();

    assert_eq!(albums_job_metadata.table_name, "albums");
    assert!(
        albums_job_metadata.last_sync_hlc >= album_creation_hlc,
        "Last sync HLC ({}) should be >= the HLC of the synced server album ({})",
        albums_job_metadata.last_sync_hlc,
        album_creation_hlc
    );

    let client_album = albums::Entity::find_by_id(new_album_pk_id)
        .one(&client_db)
        .await?
        .context("Album not found on client")?;

    assert_eq!(client_album.name, "Server Test Album");
    assert_eq!(client_album.hlc_uuid, new_album_hlc_uuid);

    test_server.shutdown_tx.send(()).ok();
    test_server.handle.await??;
    Ok(())
}

#[tokio::test]
async fn test_bidirectional_sync_different_albums() -> Result<()> {
    let _ = env_logger::try_init();

    let server_db = setup_db(true, "").await?;
    let client_db = setup_db(false, "").await?;
    let test_server = start_server(server_db.clone()).await?;
    let client_node_id = Uuid::new_v4();
    let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", test_server.addr));
    let hlc_task_context = SyncTaskContext::new(client_node_id);

    let client_hlc = hlc_task_context.generate_hlc();
    let album_c_pk_id = 3;
    let album_c_hlc_uuid = Uuid::new_v4().to_string();
    albums::ActiveModel {
        id: ActiveValue::Set(album_c_pk_id),
        name: ActiveValue::Set("Album C (from Client)".to_string()),
        group: ActiveValue::Set("Client Group".to_string()),
        hlc_uuid: ActiveValue::Set(album_c_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(client_hlc.to_rfc3339()?),
        created_at_hlc_ver: ActiveValue::Set(client_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(client_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(client_hlc.to_rfc3339()?),
        updated_at_hlc_ver: ActiveValue::Set(client_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(client_hlc.node_id.to_string()),
    }
    .insert(&client_db)
    .await?;

    let server_hlc = test_server.hlc_context.generate_hlc();
    let album_s_pk_id = 4;
    let album_s_hlc_uuid = Uuid::new_v4().to_string();
    albums::ActiveModel {
        id: ActiveValue::Set(album_s_pk_id),
        name: ActiveValue::Set("Album S (from Server)".to_string()),
        group: ActiveValue::Set("Server Group".to_string()),
        hlc_uuid: ActiveValue::Set(album_s_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(server_hlc.to_rfc3339()?),
        created_at_hlc_ver: ActiveValue::Set(server_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(server_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(server_hlc.to_rfc3339()?),
        updated_at_hlc_ver: ActiveValue::Set(server_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(server_hlc.node_id.to_string()),
    }
    .insert(&server_db)
    .await?;

    tokio::time::sleep(Duration::from_millis(10)).await;

    let results: Vec<TableSyncResult> = setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    let albums_job_metadata = results
        .iter()
        .find(|r| r.get_metadata().is_some_and(|s| s.table_name == "albums"))
        .expect("Albums job result not found")
        .metadata_ref();

    let max_creation_hlc = std::cmp::max(client_hlc, server_hlc);
    assert!(
        albums_job_metadata.last_sync_hlc >= max_creation_hlc,
        "Last sync HLC ({}) should be >= the max HLC of all created records ({})",
        albums_job_metadata.last_sync_hlc,
        max_creation_hlc
    );

    assert_eq!(albums::Entity::find().count(&client_db).await?, 2);
    assert_eq!(albums::Entity::find().count(&server_db).await?, 2);

    test_server.shutdown_tx.send(()).ok();
    test_server.handle.await??;
    Ok(())
}

#[tokio::test]
async fn test_sync_media_files_with_cover_art_fk() -> Result<()> {
    let _ = env_logger::try_init();

    let server_db = setup_db(true, "").await?;
    let client_db = setup_db(false, "").await?;
    let test_server = start_server(server_db.clone()).await?;
    let client_node_id = Uuid::new_v4();
    let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", test_server.addr));
    let hlc_task_context = SyncTaskContext::new(client_node_id);

    // Client: Insert CoverArt CA1, MediaFile MF1 -> CA1
    let ca1_hlc = hlc_task_context.generate_hlc();
    let ca1_pk_id = 1;
    let ca1_hlc_uuid = Uuid::new_v4().to_string();
    let ca1_client = media_cover_art::ActiveModel {
        id: ActiveValue::Set(ca1_pk_id),
        file_hash: ActiveValue::Set("ca1_hash_client".to_string()),
        binary: ActiveValue::Set(vec![1, 1, 1]),
        primary_color: ActiveValue::Set(Some(0xAAAAAA)),
        hlc_uuid: ActiveValue::Set(ca1_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(ca1_hlc.to_rfc3339()?),
        created_at_hlc_ver: ActiveValue::Set(ca1_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(ca1_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(ca1_hlc.to_rfc3339()?),
        updated_at_hlc_ver: ActiveValue::Set(ca1_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(ca1_hlc.node_id.to_string()),
    };
    ca1_client.insert(&client_db).await?;

    let mf1_hlc = hlc_task_context.generate_hlc();
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
        duration: ActiveValue::Set(Decimal::new(180, 0)),
        hlc_uuid: ActiveValue::Set(mf1_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(mf1_hlc.to_rfc3339()?),
        created_at_hlc_ver: ActiveValue::Set(mf1_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(mf1_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(mf1_hlc.to_rfc3339()?),
        updated_at_hlc_ver: ActiveValue::Set(mf1_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(mf1_hlc.node_id.to_string()),
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
    let ca2_hlc = test_server.hlc_context.generate_hlc();
    let ca2_pk_id = 2;
    let ca2_hlc_uuid = Uuid::new_v4().to_string();
    let ca2_server = media_cover_art::ActiveModel {
        id: ActiveValue::Set(ca2_pk_id),
        file_hash: ActiveValue::Set("ca2_hash_server".to_string()),
        binary: ActiveValue::Set(vec![2, 2, 2]),
        primary_color: ActiveValue::Set(Some(0xBBBBBB)),
        hlc_uuid: ActiveValue::Set(ca2_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(ca2_hlc.to_rfc3339()?),
        created_at_hlc_ver: ActiveValue::Set(ca2_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(ca2_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(ca2_hlc.to_rfc3339()?),
        updated_at_hlc_ver: ActiveValue::Set(ca2_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(ca2_hlc.node_id.to_string()),
    };
    ca2_server.insert(&server_db).await?;

    let mf2_hlc = test_server.hlc_context.generate_hlc();
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
        duration: ActiveValue::Set(Decimal::new(2405, 1)),
        hlc_uuid: ActiveValue::Set(mf2_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(mf2_hlc.to_rfc3339()?),
        created_at_hlc_ver: ActiveValue::Set(mf2_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(mf2_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(mf2_hlc.to_rfc3339()?),
        updated_at_hlc_ver: ActiveValue::Set(mf2_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(mf2_hlc.node_id.to_string()),
    };
    mf2_server.insert(&server_db).await?;

    // Second sync (Server -> Client)
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

    let server_db = setup_db(true, "").await?;
    let client_db = setup_db(false, "").await?;
    let test_server = start_server(server_db.clone()).await?;
    let client_node_id = Uuid::new_v4();
    let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", test_server.addr));
    let hlc_task_context = SyncTaskContext::new(client_node_id);

    // Initial check
    let last_hlc_before_sync = remote_data_source
        .get_remote_last_sync_hlc("albums", client_node_id)
        .await?;
    assert!(last_hlc_before_sync.is_none());

    let album_creation_hlc = hlc_task_context.generate_hlc();
    let album_pk_id = 5;
    let album_hlc_uuid = Uuid::new_v4().to_string();
    let album_name = "Album for Last Sync HLC Test".to_string();
    albums::ActiveModel {
        id: ActiveValue::Set(album_pk_id),
        name: ActiveValue::Set(album_name.clone()),
        group: ActiveValue::Set("Last Sync Group".to_string()),
        hlc_uuid: ActiveValue::Set(album_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(album_creation_hlc.to_rfc3339()?),
        created_at_hlc_ver: ActiveValue::Set(album_creation_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(album_creation_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(album_creation_hlc.to_rfc3339()?),
        updated_at_hlc_ver: ActiveValue::Set(album_creation_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(album_creation_hlc.node_id.to_string()),
    }
    .insert(&client_db)
    .await?;

    // Run sync
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
        .expect("Albums job result not found")
        .unwrap_metadata();

    let last_hlc_after_sync_opt = remote_data_source
        .get_remote_last_sync_hlc("albums", client_node_id)
        .await?;

    let server_reported_hlc = last_hlc_after_sync_opt
        .clone()
        .context("Server API did not return a last_sync_hlc for the client")?;

    let client_final_hlc = albums_job_metadata_owned.last_sync_hlc;

    assert!(client_final_hlc >= server_reported_hlc);

    let server_album = albums::Entity::find()
        .filter(albums::Column::HlcUuid.eq(album_hlc_uuid))
        .one(&server_db)
        .await?
        .context("Album not found on server after sync")?;
    assert_eq!(server_album.name, album_name);

    let expected_hlc_from_op = HLC {
        timestamp_ms: album_creation_hlc.timestamp_ms,
        version: album_creation_hlc.version,
        node_id: album_creation_hlc.node_id,
    };

    let server_sync_record = sync_record::Entity::find()
        .filter(sync_record::Column::TableName.eq("albums"))
        .filter(sync_record::Column::ClientNodeId.eq(client_node_id.to_string()))
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

#[tokio::test]
async fn test_client_updates_album_synced_to_server() -> Result<()> {
    let _ = env_logger::try_init();

    // 1. Setup
    let server_db = setup_db(true, "").await?;
    let client_db = setup_db(false, "").await?;
    let test_server = start_server(server_db.clone()).await?;
    let client_node_id = Uuid::new_v4();
    let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", test_server.addr));
    let hlc_task_context = SyncTaskContext::new(client_node_id);

    // 2. Create initial record and sync it
    let initial_hlc = hlc_task_context.generate_hlc();
    let album_pk_id = 101;
    let album_hlc_uuid = Uuid::new_v4().to_string();
    albums::ActiveModel {
        id: ActiveValue::Set(album_pk_id),
        name: ActiveValue::Set("Initial Album Name".to_string()),
        group: ActiveValue::Set("Initial Group".to_string()),
        hlc_uuid: ActiveValue::Set(album_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(initial_hlc.to_rfc3339()?),
        created_at_hlc_ver: ActiveValue::Set(initial_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(initial_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(initial_hlc.to_rfc3339()?),
        updated_at_hlc_ver: ActiveValue::Set(initial_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(initial_hlc.node_id.to_string()),
    }
    .insert(&client_db)
    .await?;

    setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    // 3. Client updates the record
    let client_update_hlc = hlc_task_context.generate_hlc();
    // Fetch the existing record, convert it to an ActiveModel, then modify and save.
    // This pattern is more robust and consistent with other passing tests.
    let mut album_to_update = albums::Entity::find_by_id(album_pk_id)
        .one(&client_db)
        .await?
        .context("Failed to find album on client before updating")?
        .into_active_model();

    album_to_update.name = ActiveValue::Set("Updated by Client".to_string());
    album_to_update.updated_at_hlc_ts = ActiveValue::Set(client_update_hlc.to_rfc3339()?);
    album_to_update.updated_at_hlc_ver = ActiveValue::Set(client_update_hlc.version as i32);
    album_to_update.updated_at_hlc_nid = ActiveValue::Set(client_update_hlc.node_id.to_string());

    album_to_update.update(&client_db).await?;

    // 4. Run sync again
    setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    // 5. Assertions
    let server_album = albums::Entity::find_by_id(album_pk_id)
        .one(&server_db)
        .await?
        .context("Album not found on server after update sync")?;

    assert_eq!(server_album.name, "Updated by Client");
    assert_eq!(
        server_album.updated_at_hlc_ts,
        client_update_hlc.to_rfc3339()?
    );
    assert_eq!(
        server_album.updated_at_hlc_ver,
        client_update_hlc.version as i32
    );
    assert_eq!(
        server_album.updated_at_hlc_nid,
        client_update_hlc.node_id.to_string()
    );

    test_server.shutdown_tx.send(()).ok();
    test_server.handle.await??;
    Ok(())
}

#[tokio::test]
async fn test_server_updates_album_synced_to_client() -> Result<()> {
    let _ = env_logger::try_init();

    // 1. Setup
    let server_db = setup_db(true, "").await?;
    let client_db = setup_db(false, "").await?;
    let test_server = start_server(server_db.clone()).await?;
    let client_node_id = Uuid::new_v4();
    let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", test_server.addr));
    let hlc_task_context = SyncTaskContext::new(client_node_id);

    // 2. Create initial record on server and sync it
    let initial_hlc = test_server.hlc_context.generate_hlc();
    let album_pk_id = 102;
    let album_hlc_uuid = Uuid::new_v4().to_string();
    albums::ActiveModel {
        id: ActiveValue::Set(album_pk_id),
        name: ActiveValue::Set("Original Server Album".to_string()),
        group: ActiveValue::Set("Original Group".to_string()),
        hlc_uuid: ActiveValue::Set(album_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(initial_hlc.to_rfc3339()?),
        created_at_hlc_ver: ActiveValue::Set(initial_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(initial_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(initial_hlc.to_rfc3339()?),
        updated_at_hlc_ver: ActiveValue::Set(initial_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(initial_hlc.node_id.to_string()),
    }
    .insert(&server_db)
    .await?;

    setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    // 3. Server updates the record
    let server_update_hlc = test_server.hlc_context.generate_hlc();
    let mut album_to_update = albums::Entity::find_by_id(album_pk_id)
        .one(&server_db)
        .await?
        .context("Failed to find album on server before updating")?
        .into_active_model();

    album_to_update.name = ActiveValue::Set("Updated by Server".to_string());
    album_to_update.updated_at_hlc_ts = ActiveValue::Set(server_update_hlc.to_rfc3339()?);
    album_to_update.updated_at_hlc_ver = ActiveValue::Set(server_update_hlc.version as i32);
    album_to_update.updated_at_hlc_nid = ActiveValue::Set(server_update_hlc.node_id.to_string());

    album_to_update.update(&server_db).await?;

    // 4. Run sync again
    setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    // 5. Assertions
    let client_album = albums::Entity::find_by_id(album_pk_id)
        .one(&client_db)
        .await?
        .context("Album not found on client after update sync")?;

    assert_eq!(client_album.name, "Updated by Server");
    assert_eq!(
        client_album.updated_at_hlc_ts,
        server_update_hlc.to_rfc3339()?
    );

    test_server.shutdown_tx.send(()).ok();
    test_server.handle.await??;
    Ok(())
}

#[tokio::test]
async fn test_conflict_resolution_client_wins() -> Result<()> {
    let _ = env_logger::try_init();

    // 1. Setup
    let server_db = setup_db(true, "").await?;
    let client_db = setup_db(false, "").await?;
    let test_server = start_server(server_db.clone()).await?;
    let client_node_id = Uuid::new_v4();
    let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", test_server.addr));
    let hlc_task_context = SyncTaskContext::new(client_node_id);

    // 2. Create initial record and sync it
    let initial_hlc = hlc_task_context.generate_hlc();
    let album_pk_id = 103;
    let album_hlc_uuid = Uuid::new_v4().to_string();
    albums::ActiveModel {
        id: ActiveValue::Set(album_pk_id),
        name: ActiveValue::Set("Conflict Candidate".to_string()),
        group: ActiveValue::Set("Conflict Group".to_string()),
        hlc_uuid: ActiveValue::Set(album_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(initial_hlc.to_rfc3339()?),
        created_at_hlc_ver: ActiveValue::Set(initial_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(initial_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(initial_hlc.to_rfc3339()?),
        updated_at_hlc_ver: ActiveValue::Set(initial_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(initial_hlc.node_id.to_string()),
    }
    .insert(&client_db)
    .await?;
    setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    // 3. Server makes an update (the "older" one)
    let server_update_hlc = test_server.hlc_context.generate_hlc();
    let mut server_album_am = albums::Entity::find_by_id(album_pk_id)
        .one(&server_db)
        .await?
        .unwrap()
        .into_active_model();
    server_album_am.name = ActiveValue::Set("Server's Update (should lose)".to_string());
    server_album_am.updated_at_hlc_ts = ActiveValue::Set(server_update_hlc.to_rfc3339()?);
    server_album_am.updated_at_hlc_ver = ActiveValue::Set(server_update_hlc.version as i32);
    server_album_am.updated_at_hlc_nid = ActiveValue::Set(server_update_hlc.node_id.to_string());
    server_album_am.update(&server_db).await?;

    // Ensure client HLC is newer
    tokio::time::sleep(Duration::from_millis(5)).await;

    // 4. Client makes an update (the "newer" one)
    let client_update_hlc = hlc_task_context.generate_hlc();
    let mut client_album_am = albums::Entity::find_by_id(album_pk_id)
        .one(&client_db)
        .await?
        .unwrap()
        .into_active_model();
    client_album_am.name = ActiveValue::Set("Client's Update (should win)".to_string());
    client_album_am.updated_at_hlc_ts = ActiveValue::Set(client_update_hlc.to_rfc3339()?);
    client_album_am.updated_at_hlc_ver = ActiveValue::Set(client_update_hlc.version as i32);
    client_album_am.updated_at_hlc_nid = ActiveValue::Set(client_update_hlc.node_id.to_string());
    client_album_am.update(&client_db).await?;
    assert!(
        client_update_hlc > server_update_hlc,
        "Test setup failed: client HLC was not greater than server HLC"
    );

    // 5. Run sync
    setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    // 6. Assertions: Client's update should be on both databases
    let final_server_album = albums::Entity::find_by_id(album_pk_id)
        .one(&server_db)
        .await?
        .unwrap();
    let final_client_album = albums::Entity::find_by_id(album_pk_id)
        .one(&client_db)
        .await?
        .unwrap();

    assert_eq!(final_server_album.name, "Client's Update (should win)");
    assert_eq!(
        final_server_album.updated_at_hlc_nid,
        client_update_hlc.node_id.to_string()
    );

    assert_eq!(final_client_album.name, "Client's Update (should win)");
    assert_eq!(
        final_client_album.updated_at_hlc_nid,
        client_update_hlc.node_id.to_string()
    );

    test_server.shutdown_tx.send(()).ok();
    test_server.handle.await??;
    Ok(())
}

#[tokio::test]
async fn test_conflict_resolution_server_wins() -> Result<()> {
    let _ = env_logger::try_init();

    // 1. Setup
    let server_db = setup_db(true, "").await?;
    let client_db = setup_db(false, "").await?;
    let test_server = start_server(server_db.clone()).await?;
    let client_node_id = Uuid::new_v4();
    let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", test_server.addr));
    let hlc_task_context = SyncTaskContext::new(client_node_id);

    // 2. Create initial record and sync it
    let initial_hlc = hlc_task_context.generate_hlc();
    let album_pk_id = 104;
    let album_hlc_uuid = Uuid::new_v4().to_string();
    albums::ActiveModel {
        id: ActiveValue::Set(album_pk_id),
        name: ActiveValue::Set("Conflict Candidate".to_string()),
        group: ActiveValue::Set("Conflict Group".to_string()),
        hlc_uuid: ActiveValue::Set(album_hlc_uuid.clone()),
        created_at_hlc_ts: ActiveValue::Set(initial_hlc.to_rfc3339()?),
        created_at_hlc_ver: ActiveValue::Set(initial_hlc.version as i32),
        created_at_hlc_nid: ActiveValue::Set(initial_hlc.node_id.to_string()),
        updated_at_hlc_ts: ActiveValue::Set(initial_hlc.to_rfc3339()?),
        updated_at_hlc_ver: ActiveValue::Set(initial_hlc.version as i32),
        updated_at_hlc_nid: ActiveValue::Set(initial_hlc.node_id.to_string()),
    }
    .insert(&client_db)
    .await?;
    setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    // 3. Client makes an update (the "older" one)
    let client_update_hlc = hlc_task_context.generate_hlc();
    let mut client_album_am = albums::Entity::find_by_id(album_pk_id)
        .one(&client_db)
        .await?
        .unwrap()
        .into_active_model();
    client_album_am.name = ActiveValue::Set("Client's Update (should lose)".to_string());
    client_album_am.updated_at_hlc_ts = ActiveValue::Set(client_update_hlc.to_rfc3339()?);
    client_album_am.updated_at_hlc_ver = ActiveValue::Set(client_update_hlc.version as i32);
    client_album_am.updated_at_hlc_nid = ActiveValue::Set(client_update_hlc.node_id.to_string());
    client_album_am.update(&client_db).await?;

    // Ensure server HLC is newer
    tokio::time::sleep(Duration::from_millis(5)).await;

    // 4. Server makes an update (the "newer" one)
    let server_update_hlc = test_server.hlc_context.generate_hlc();
    let mut server_album_am = albums::Entity::find_by_id(album_pk_id)
        .one(&server_db)
        .await?
        .unwrap()
        .into_active_model();
    server_album_am.name = ActiveValue::Set("Server's Update (should win)".to_string());
    server_album_am.updated_at_hlc_ts = ActiveValue::Set(server_update_hlc.to_rfc3339()?);
    server_album_am.updated_at_hlc_ver = ActiveValue::Set(server_update_hlc.version as i32);
    server_album_am.updated_at_hlc_nid = ActiveValue::Set(server_update_hlc.node_id.to_string());
    server_album_am.update(&server_db).await?;
    assert!(
        server_update_hlc > client_update_hlc,
        "Test setup failed: server HLC was not greater than client HLC"
    );

    // 5. Run sync
    setup_and_run_sync(
        &client_db,
        client_node_id,
        &remote_data_source,
        &hlc_task_context,
    )
    .await?;

    // 6. Assertions: Server's update should be on both databases
    let final_server_album = albums::Entity::find_by_id(album_pk_id)
        .one(&server_db)
        .await?
        .unwrap();
    let final_client_album = albums::Entity::find_by_id(album_pk_id)
        .one(&client_db)
        .await?
        .unwrap();

    assert_eq!(final_server_album.name, "Server's Update (should win)");
    assert_eq!(
        final_server_album.updated_at_hlc_nid,
        server_update_hlc.node_id.to_string()
    );

    assert_eq!(final_client_album.name, "Server's Update (should win)");
    assert_eq!(
        final_client_album.updated_at_hlc_nid,
        server_update_hlc.node_id.to_string()
    );

    test_server.shutdown_tx.send(()).ok();
    test_server.handle.await??;
    Ok(())
}

// // TODO: Add more tests:
// // - Deletes (client deletes, server deletes)
// // - Sync for junction tables (media_file_albums, media_file_artists, media_file_genres)
// //   ensuring FKs are correct (e.g. media_file_albums.track_number).
// // - More complex bidirectional scenarios (e.g. client updates X, server updates Y, then sync).
// // - Conflict scenarios (if your HLC logic handles them, e.g., both update same record).
// // - Test chunking and sub-chunking more directly if specific behaviors need validation beyond successful sync.
// // - Test error conditions (e.g., server down during a call, malformed data).
