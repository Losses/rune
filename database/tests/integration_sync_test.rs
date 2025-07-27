use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use axum::{
    routing::{get, post},
    serve, Router,
};
use chrono::Utc;
use sea_orm::{
    prelude::Decimal, ActiveModelTrait, ColumnTrait, ConnectOptions, Database, DatabaseConnection,
    EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, Set,
};
use tokio::{net::TcpListener, task::JoinHandle};
use uuid::Uuid;

use ::database::{
    connection::initialize_db,
    entities::{albums, media_cover_art, media_files, prelude::*},
    sync::{
        chunking::{
            apply_remote_changes_handler, get_node_id_handler, get_remote_chunks_handler,
            get_remote_last_sync_hlc_handler, get_remote_records_in_hlc_range_handler,
            get_remote_sub_chunks_handler, AppState,
        },
        data_source::RemoteHttpDataSource,
        foreign_keys::RuneForeignKeyResolver,
        setup_and_run_sync,
    },
};
use ::sync::{
    chunking::ChunkingOptions, core::RemoteDataSource, hlc::SyncTaskContext,
    sync_scheduler::TableSyncResult,
};

// Constants for Table Names
const ALBUMS_TABLE: &str = "albums";

// Test Fixture

struct TestFixture {
    server: TestServer,
    server_db: DatabaseConnection,
    client_db: DatabaseConnection,
    client_node_id: Uuid,
    client_hlc_context: Arc<SyncTaskContext>,
    remote_data_source: RemoteHttpDataSource,
}

impl TestFixture {
    async fn new() -> Result<Self> {
        let _ = env_logger::try_init();

        let server_db = setup_db(true).await.context("Server DB setup failed")?;
        let client_db = setup_db(false).await.context("Client DB setup failed")?;

        let server = start_server(server_db.clone())
            .await
            .context("Server start failed")?;
        let client_node_id = Uuid::new_v4();
        let client_hlc_context = Arc::new(SyncTaskContext::new(client_node_id));
        let remote_data_source = RemoteHttpDataSource::new(&format!("http://{}", server.addr));

        Ok(Self {
            server,
            server_db,
            client_db,
            client_node_id,
            client_hlc_context,
            remote_data_source,
        })
    }

    async fn run_sync(&self) -> Result<Vec<TableSyncResult>> {
        setup_and_run_sync(
            &self.client_db,
            self.client_node_id,
            &self.remote_data_source,
            &self.client_hlc_context,
        )
        .await
    }

    fn server_hlc_context(&self) -> &Arc<SyncTaskContext> {
        &self.server.hlc_context
    }
}

impl Drop for TestFixture {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.server.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }
}

async fn setup_db(is_server: bool) -> Result<DatabaseConnection> {
    let side = if is_server { "server" } else { "client" };
    let db_name = format!("test-db-{}-{}", side, Uuid::new_v4());
    let db_url = format!("sqlite:file:{}?mode=memory&cache=shared", db_name);

    let mut opt = ConnectOptions::new(&db_url);
    opt.sqlx_logging(false); // Disable verbose logging for cleaner test output

    let db = Database::connect(opt).await?;
    // Use a random node_id for test dbs, it's not relevant for these tests
    initialize_db(&db, &Uuid::new_v4().to_string()).await?;
    Ok(db)
}

pub struct TestServer {
    addr: SocketAddr,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    _handle: JoinHandle<Result<()>>,
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

    tokio::time::sleep(Duration::from_millis(100)).await; // Give server a moment to start

    Ok(TestServer {
        addr,
        shutdown_tx: Some(shutdown_tx),
        _handle: handle,
        node_id: server_node_id,
        hlc_context,
    })
}

// Data Seeding Helpers

async fn seed_album(
    db: &DatabaseConnection,
    hlc_context: &SyncTaskContext,
    pk_id: i32,
    name: &str,
) -> Result<albums::Model> {
    let hlc = hlc_context.generate_hlc();
    let model = albums::ActiveModel {
        id: Set(pk_id),
        name: Set(name.to_string()),
        group: Set("Test Group".to_string()),
        hlc_uuid: Set(Uuid::new_v4().to_string()),
        created_at_hlc_ts: Set(hlc.to_rfc3339()?),
        created_at_hlc_ver: Set(hlc.version as i32),
        created_at_hlc_nid: Set(hlc.node_id.to_string()),
        updated_at_hlc_ts: Set(hlc.to_rfc3339()?),
        updated_at_hlc_ver: Set(hlc.version as i32),
        updated_at_hlc_nid: Set(hlc.node_id.to_string()),
    };
    model.insert(db).await.context("Failed to seed album")
}

async fn seed_cover_art(
    db: &DatabaseConnection,
    hlc_context: &SyncTaskContext,
    pk_id: i32,
    file_hash: &str,
    binary: Vec<u8>,
) -> Result<media_cover_art::Model> {
    let hlc = hlc_context.generate_hlc();
    media_cover_art::ActiveModel {
        id: Set(pk_id),
        file_hash: Set(file_hash.to_string()),
        binary: Set(binary),
        primary_color: Set(Some(0xAAAAAA)),
        hlc_uuid: Set(Uuid::new_v4().to_string()),
        created_at_hlc_ts: Set(hlc.to_rfc3339()?),
        created_at_hlc_ver: Set(hlc.version as i32),
        created_at_hlc_nid: Set(hlc.node_id.to_string()),
        updated_at_hlc_ts: Set(hlc.to_rfc3339()?),
        updated_at_hlc_ver: Set(hlc.version as i32),
        updated_at_hlc_nid: Set(hlc.node_id.to_string()),
    }
    .insert(db)
    .await
    .context("Failed to seed cover art")
}

async fn seed_media_file(
    db: &DatabaseConnection,
    hlc_context: &SyncTaskContext,
    pk_id: i32,
    file_name: &str,
    cover_art_id: Option<i32>,
) -> Result<media_files::Model> {
    let hlc = hlc_context.generate_hlc();
    media_files::ActiveModel {
        id: Set(pk_id),
        file_name: Set(file_name.to_string()),
        directory: Set(format!("/music/{}/", file_name)),
        extension: Set("mp3".to_string()),
        file_hash: Set(format!("{}_hash", file_name)),
        last_modified: Set(Utc::now().to_rfc3339()),
        cover_art_id: Set(cover_art_id),
        sample_rate: Set(44100),
        duration: Set(Decimal::new(180, 0)),
        hlc_uuid: Set(Uuid::new_v4().to_string()),
        created_at_hlc_ts: Set(hlc.to_rfc3339()?),
        created_at_hlc_ver: Set(hlc.version as i32),
        created_at_hlc_nid: Set(hlc.node_id.to_string()),
        updated_at_hlc_ts: Set(hlc.to_rfc3339()?),
        updated_at_hlc_ver: Set(hlc.version as i32),
        updated_at_hlc_nid: Set(hlc.node_id.to_string()),
    }
    .insert(db)
    .await
    .context("Failed to seed media file")
}

// Refactored Tests

#[tokio::test]
async fn test_initial_sync_empty_databases() -> Result<()> {
    let fixture = TestFixture::new().await?;
    let results = fixture.run_sync().await.context("Sync execution failed")?;

    for result in results {
        assert!(
            result.is_success(),
            "Sync job for table '{}' failed: {:?}",
            result.table_name_str(),
            result.get_error()
        );
    }

    assert_eq!(
        Albums::find().count(&fixture.client_db).await?,
        0,
        "Client DB should have no albums"
    );
    assert_eq!(
        Albums::find().count(&fixture.server_db).await?,
        0,
        "Server DB should have no albums"
    );
    Ok(())
}

#[tokio::test]
async fn test_client_inserts_album_synced_to_server() -> Result<()> {
    let fixture = TestFixture::new().await?;

    let client_album = seed_album(
        &fixture.client_db,
        &fixture.client_hlc_context,
        1,
        "Client Album",
    )
    .await?;
    fixture.run_sync().await?;

    let server_album = Albums::find()
        .filter(albums::Column::HlcUuid.eq(client_album.hlc_uuid.clone()))
        .one(&fixture.server_db)
        .await?
        .context("Album not found on server by HLC_UUID")?;

    assert_eq!(server_album.name, client_album.name);
    assert_eq!(server_album.id, client_album.id);
    Ok(())
}

#[tokio::test]
async fn test_server_inserts_album_synced_to_client() -> Result<()> {
    let fixture = TestFixture::new().await?;

    let server_album = seed_album(
        &fixture.server_db,
        fixture.server_hlc_context(),
        2,
        "Server Album",
    )
    .await?;
    fixture.run_sync().await?;

    let client_album = Albums::find_by_id(server_album.id)
        .one(&fixture.client_db)
        .await?
        .context("Album not found on client")?;

    assert_eq!(client_album.name, server_album.name);
    assert_eq!(client_album.hlc_uuid, server_album.hlc_uuid);
    Ok(())
}

#[tokio::test]
async fn test_bidirectional_sync_different_albums() -> Result<()> {
    let fixture = TestFixture::new().await?;

    let client_album = seed_album(
        &fixture.client_db,
        &fixture.client_hlc_context,
        3,
        "Client Album",
    )
    .await?;
    let server_album = seed_album(
        &fixture.server_db,
        fixture.server_hlc_context(),
        4,
        "Server Album",
    )
    .await?;

    fixture.run_sync().await?;

    assert_eq!(
        Albums::find().count(&fixture.client_db).await?,
        2,
        "Client should have 2 albums after sync"
    );
    assert_eq!(
        Albums::find().count(&fixture.server_db).await?,
        2,
        "Server should have 2 albums after sync"
    );

    // Verify client has server's album
    let server_album_on_client = Albums::find_by_id(server_album.id)
        .one(&fixture.client_db)
        .await?
        .unwrap();
    assert_eq!(server_album_on_client.name, server_album.name);

    // Verify server has client's album
    let client_album_on_server = Albums::find_by_id(client_album.id)
        .one(&fixture.server_db)
        .await?
        .unwrap();
    assert_eq!(client_album_on_server.name, client_album.name);

    Ok(())
}

#[tokio::test]
async fn test_sync_media_files_with_cover_art_fk() -> Result<()> {
    let fixture = TestFixture::new().await?;

    // Client creates cover art and a media file linking to it
    let client_ca = seed_cover_art(
        &fixture.client_db,
        &fixture.client_hlc_context,
        1,
        "ca1_hash",
        vec![1],
    )
    .await?;
    let client_mf = seed_media_file(
        &fixture.client_db,
        &fixture.client_hlc_context,
        10,
        "client_song",
        Some(client_ca.id),
    )
    .await?;

    fixture
        .run_sync()
        .await
        .context("First sync (C->S) failed")?;

    // Verify client's data is on the server with correct FK
    let server_ca = MediaCoverArt::find()
        .filter(media_cover_art::Column::HlcUuid.eq(client_ca.hlc_uuid))
        .one(&fixture.server_db)
        .await?
        .context("Cover art not on server")?;
    let server_mf = MediaFiles::find()
        .filter(media_files::Column::HlcUuid.eq(client_mf.hlc_uuid))
        .one(&fixture.server_db)
        .await?
        .context("Media file not on server")?;
    assert_eq!(
        server_mf.cover_art_id,
        Some(server_ca.id),
        "FK from media_file to cover_art is incorrect on server"
    );

    // Server creates its own data
    let server_ca2 = seed_cover_art(
        &fixture.server_db,
        fixture.server_hlc_context(),
        2,
        "ca2_hash",
        vec![2],
    )
    .await?;
    let server_mf2 = seed_media_file(
        &fixture.server_db,
        fixture.server_hlc_context(),
        20,
        "server_song",
        Some(server_ca2.id),
    )
    .await?;

    fixture
        .run_sync()
        .await
        .context("Second sync (S->C) failed")?;

    // Verify server's data is on the client with correct FK
    let client_ca2 = MediaCoverArt::find()
        .filter(media_cover_art::Column::HlcUuid.eq(server_ca2.hlc_uuid))
        .one(&fixture.client_db)
        .await?
        .context("Server CA not on client")?;
    let client_mf2 = MediaFiles::find()
        .filter(media_files::Column::HlcUuid.eq(server_mf2.hlc_uuid))
        .one(&fixture.client_db)
        .await?
        .context("Server MF not on client")?;
    assert_eq!(
        client_mf2.cover_art_id,
        Some(client_ca2.id),
        "FK from media_file to cover_art is incorrect on client"
    );

    // Final counts
    assert_eq!(MediaCoverArt::find().count(&fixture.client_db).await?, 2);
    assert_eq!(MediaFiles::find().count(&fixture.client_db).await?, 2);
    assert_eq!(MediaCoverArt::find().count(&fixture.server_db).await?, 2);
    assert_eq!(MediaFiles::find().count(&fixture.server_db).await?, 2);

    Ok(())
}

#[tokio::test]
async fn test_get_remote_last_sync_hlc() -> Result<()> {
    let fixture = TestFixture::new().await?;

    let last_hlc_before = fixture
        .remote_data_source
        .get_remote_last_sync_hlc(ALBUMS_TABLE, fixture.client_node_id)
        .await?;
    assert!(
        last_hlc_before.is_none(),
        "Should be no last sync HLC for a new client"
    );

    seed_album(
        &fixture.client_db,
        &fixture.client_hlc_context,
        5,
        "Album for HLC test",
    )
    .await?;
    let results = fixture.run_sync().await?;

    let albums_meta = results
        .iter()
        .find(|r| r.table_name_str() == ALBUMS_TABLE)
        .unwrap()
        .metadata_ref();
    let last_hlc_after = fixture
        .remote_data_source
        .get_remote_last_sync_hlc(ALBUMS_TABLE, fixture.client_node_id)
        .await?;

    assert!(
        last_hlc_after.is_some(),
        "Server should now have a last sync HLC"
    );
    assert_eq!(
        last_hlc_after.as_ref(),
        Some(&albums_meta.last_sync_hlc),
        "Server-reported HLC should match client's final HLC"
    );

    Ok(())
}

#[tokio::test]
async fn test_client_updates_album_synced_to_server() -> Result<()> {
    let fixture = TestFixture::new().await?;

    let mut album = seed_album(
        &fixture.client_db,
        &fixture.client_hlc_context,
        101,
        "Initial Name",
    )
    .await?;
    fixture.run_sync().await?;

    // Client updates the album
    let update_hlc = fixture.client_hlc_context.generate_hlc();
    let mut album_am = album.into_active_model();
    album_am.name = Set("Updated by Client".to_string());
    album_am.updated_at_hlc_ts = Set(update_hlc.to_rfc3339()?);
    album_am.updated_at_hlc_ver = Set(update_hlc.version as i32);
    album_am.updated_at_hlc_nid = Set(update_hlc.node_id.to_string());
    album = album_am.update(&fixture.client_db).await?;

    fixture.run_sync().await?;

    let server_album = Albums::find_by_id(album.id)
        .one(&fixture.server_db)
        .await?
        .context("Album not on server after update")?;
    assert_eq!(server_album.name, "Updated by Client");
    assert_eq!(server_album.updated_at_hlc_ts, update_hlc.to_rfc3339()?);

    Ok(())
}

#[tokio::test]
async fn test_server_updates_album_synced_to_client() -> Result<()> {
    let fixture = TestFixture::new().await?;

    let mut album = seed_album(
        &fixture.server_db,
        fixture.server_hlc_context(),
        102,
        "Initial Name",
    )
    .await?;
    fixture.run_sync().await?;

    // Server updates the album
    let update_hlc = fixture.server_hlc_context().generate_hlc();
    let mut album_am = album.into_active_model();
    album_am.name = Set("Updated by Server".to_string());
    album_am.updated_at_hlc_ts = Set(update_hlc.to_rfc3339()?);
    album_am.updated_at_hlc_ver = Set(update_hlc.version as i32);
    album_am.updated_at_hlc_nid = Set(update_hlc.node_id.to_string());
    album = album_am.update(&fixture.server_db).await?;

    fixture.run_sync().await?;

    let client_album = Albums::find_by_id(album.id)
        .one(&fixture.client_db)
        .await?
        .context("Album not on client after update")?;
    assert_eq!(client_album.name, "Updated by Server");
    assert_eq!(client_album.updated_at_hlc_ts, update_hlc.to_rfc3339()?);

    Ok(())
}

#[tokio::test]
async fn test_conflict_resolution_client_wins() -> Result<()> {
    let fixture = TestFixture::new().await?;

    let album = seed_album(
        &fixture.client_db,
        &fixture.client_hlc_context,
        103,
        "Conflict Candidate",
    )
    .await?;
    fixture.run_sync().await?;

    // Server makes an "older" update
    let server_update_hlc = fixture.server_hlc_context().generate_hlc();
    let mut server_am = Albums::find_by_id(album.id)
        .one(&fixture.server_db)
        .await?
        .unwrap()
        .into_active_model();
    server_am.name = Set("Server Update (should lose)".to_string());
    server_am.updated_at_hlc_ts = Set(server_update_hlc.to_rfc3339()?);
    server_am.updated_at_hlc_ver = Set(server_update_hlc.version as i32);
    server_am.updated_at_hlc_nid = Set(server_update_hlc.node_id.to_string());
    server_am.update(&fixture.server_db).await?;

    tokio::time::sleep(Duration::from_millis(5)).await; // Ensure time progresses

    // Client makes a "newer" update
    let client_update_hlc = fixture.client_hlc_context.generate_hlc();
    let mut client_am = Albums::find_by_id(album.id)
        .one(&fixture.client_db)
        .await?
        .unwrap()
        .into_active_model();
    client_am.name = Set("Client Update (should win)".to_string());
    client_am.updated_at_hlc_ts = Set(client_update_hlc.to_rfc3339()?);
    client_am.updated_at_hlc_ver = Set(client_update_hlc.version as i32);
    client_am.updated_at_hlc_nid = Set(client_update_hlc.node_id.to_string());
    client_am.update(&fixture.client_db).await?;

    assert!(
        client_update_hlc > server_update_hlc,
        "Client HLC must be greater for this test"
    );

    fixture.run_sync().await?;

    let final_server_album = Albums::find_by_id(album.id)
        .one(&fixture.server_db)
        .await?
        .unwrap();
    let final_client_album = Albums::find_by_id(album.id)
        .one(&fixture.client_db)
        .await?
        .unwrap();

    assert_eq!(final_server_album.name, "Client Update (should win)");
    assert_eq!(final_client_album.name, "Client Update (should win)");
    assert_eq!(
        final_server_album.updated_at_hlc_nid,
        client_update_hlc.node_id.to_string()
    );

    Ok(())
}

#[tokio::test]
async fn test_conflict_resolution_server_wins() -> Result<()> {
    let fixture = TestFixture::new().await?;

    let album = seed_album(
        &fixture.client_db,
        &fixture.client_hlc_context,
        104,
        "Conflict Candidate",
    )
    .await?;
    fixture.run_sync().await?;

    // Client makes an "older" update
    let client_update_hlc = fixture.client_hlc_context.generate_hlc();
    let mut client_am = Albums::find_by_id(album.id)
        .one(&fixture.client_db)
        .await?
        .unwrap()
        .into_active_model();
    client_am.name = Set("Client Update (should lose)".to_string());
    client_am.updated_at_hlc_ts = Set(client_update_hlc.to_rfc3339()?);
    client_am.updated_at_hlc_ver = Set(client_update_hlc.version as i32);
    client_am.updated_at_hlc_nid = Set(client_update_hlc.node_id.to_string());
    client_am.update(&fixture.client_db).await?;

    tokio::time::sleep(Duration::from_millis(5)).await;

    // Server makes a "newer" update
    let server_update_hlc = fixture.server_hlc_context().generate_hlc();
    let mut server_am = Albums::find_by_id(album.id)
        .one(&fixture.server_db)
        .await?
        .unwrap()
        .into_active_model();
    server_am.name = Set("Server Update (should win)".to_string());
    server_am.updated_at_hlc_ts = Set(server_update_hlc.to_rfc3339()?);
    server_am.updated_at_hlc_ver = Set(server_update_hlc.version as i32);
    server_am.updated_at_hlc_nid = Set(server_update_hlc.node_id.to_string());
    server_am.update(&fixture.server_db).await?;

    assert!(
        server_update_hlc > client_update_hlc,
        "Server HLC must be greater for this test"
    );

    fixture.run_sync().await?;

    let final_server_album = Albums::find_by_id(album.id)
        .one(&fixture.server_db)
        .await?
        .unwrap();
    let final_client_album = Albums::find_by_id(album.id)
        .one(&fixture.client_db)
        .await?
        .unwrap();

    assert_eq!(final_server_album.name, "Server Update (should win)");
    assert_eq!(final_client_album.name, "Server Update (should win)");
    assert_eq!(
        final_server_album.updated_at_hlc_nid,
        server_update_hlc.node_id.to_string()
    );

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
