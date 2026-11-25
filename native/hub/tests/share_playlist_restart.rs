use anyhow::{Context, Result, bail};
use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    middleware::{self, Next},
    response::Response,
};
use database::sync as database_sync;
use database::{
    actions::playlists,
    sync::{
        model::RuneModel,
        share::{SharePullRequest, pull},
    },
    test_fixtures::causal_library::{self as fixtures, SqlLibrary},
};
use fsio::{OpenDalConfig, OpenDalObjectStore, test_fixtures::WebDavFixture};
use http_request::sync::{MetadataPeer, router};
use reqwest::Client;
use sea_orm::{Database, DatabaseConnection};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time as tokio_time;
use tokio::{
    net::TcpListener,
    sync::{Notify, oneshot},
};
use uuid::Uuid;

#[derive(Default)]
struct PromotionGate {
    steps_to_playlists: Mutex<Option<usize>>,
    reached: Notify,
    resume: Notify,
}

async fn pause_after_first_playlist_step(
    State(gate): State<Arc<PromotionGate>>,
    request: Request,
    next: Next,
) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, 9 * 1024 * 1024).await.unwrap();
    let operation: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    if !response.status().is_success() {
        return response;
    }
    let pause = {
        let mut remaining = gate.steps_to_playlists.lock().unwrap();
        match operation["operation"].as_str() {
            Some("begin_reconciliation") if remaining.is_none() => {
                let empty_collections_before_playlists = operation["remote"]["contexts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .position(|context| context["collection"] == "playlists")
                    .unwrap();
                let steps_per_empty_collection = 2;
                *remaining =
                    Some(empty_collections_before_playlists * steps_per_empty_collection + 1);
                false
            }
            Some("promote_step") => remaining.as_mut().is_some_and(|steps| {
                if *steps == 0 {
                    return false;
                }
                *steps -= 1;
                *steps == 0
            }),
            _ => false,
        }
    };
    if pause {
        gate.reached.notify_one();
        gate.resume.notified().await;
    }
    response
}

async fn playlist_names(db: &DatabaseConnection) -> Result<Vec<String>> {
    Ok(playlists::get_all_playlists(db)
        .await?
        .into_iter()
        .map(|playlist| playlist.name)
        .collect())
}

#[derive(Clone, Copy)]
enum RestartAfter {
    CompletedPull,
    StaleDelivery,
}

async fn restart_after(boundary: RestartAfter) -> Result<()> {
    let namespace = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let stale = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let receiver = SqlLibrary::new_persistent(namespace, actor).await?;
    let deleted = playlists::create_playlist(
        &source.db,
        &source.sync_store.record_store(),
        "deleted".into(),
        "test".into(),
    )
    .await?;
    fixtures::exchange(&source.peer, &stale.peer).await?;
    assert_eq!(playlist_names(&stale.db).await?, ["deleted"]);
    playlists::remove_playlist(&source.db, &source.sync_store.record_store(), deleted.id).await?;

    let dav = WebDavFixture::start(Vec::new()).await?;
    let store = Arc::new(
        OpenDalObjectStore::new(OpenDalConfig::WebDav {
            endpoint: dav.endpoint.clone(),
            root: "/dav/library/".into(),
            username: "user".into(),
            password: "pass".into(),
        })
        .map_err(|error| anyhow::anyhow!("{error:?}"))?,
    );
    source
        .sync_store
        .push_share(
            store.clone(),
            namespace,
            source.root.path().join("cache"),
            Default::default(),
        )
        .await?
        .context("missing published deletion")?;

    let cache = receiver.root.path().join("cache");
    let gate = Arc::new(PromotionGate::default());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let app = router(receiver.peer.clone()).layer(middleware::from_fn_with_state(
        gate.clone(),
        pause_after_first_playlist_step,
    ));
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    {
        let sink = MetadataPeer::<RuneModel>::new(format!("http://{address}"), Client::new());
        let endpoint = receiver.sync_store.endpoint();
        let mut primary = Box::pin(pull(SharePullRequest {
            db: &receiver.db,
            local: &endpoint,
            sink: &sink,
            local_source: &endpoint,
            store: store.clone(),
            library_id: namespace,
            scope: fixtures::scope(),
            limits: database_sync::session_limits(),
            cache_path: cache.clone(),
            capabilities: Default::default(),
        }));
        tokio::select! {
            result = &mut primary => bail!("pull ended before playlist promotion: {result:?}"),
            () = gate.reached.notified() => (),
        }
        fixtures::drive(
            &sink,
            &sink,
            &sink,
            &sync::StorePeer(stale.peer.clone()),
            &*stale.peer,
            fixtures::scope(),
        )
        .await?;
        playlists::create_playlist(
            &receiver.db,
            &receiver.sync_store.record_store(),
            "concurrent".into(),
            "test".into(),
        )
        .await?;
        match boundary {
            RestartAfter::CompletedPull => {
                gate.resume.notify_one();
                primary.await?;
            }
            RestartAfter::StaleDelivery => {
                drop(primary);
                gate.resume.notify_one();
            }
        }
    }
    let _ = stop.send(());
    server.await??;
    drop(receiver.peer);
    drop(receiver.sync_store);
    receiver.db.close().await?;

    let path = receiver.root.path().join("library.sqlite");
    let db = Database::connect(format!("sqlite://{}", path.display())).await?;
    let reopened = fixtures::fixtures::open_library(db.clone(), actor, Some(namespace)).await?;
    if matches!(boundary, RestartAfter::CompletedPull) {
        assert_eq!(playlist_names(&db).await?, ["concurrent"], "before retry");
    }
    reopened
        .pull_share(store, namespace, cache, Default::default())
        .await?;
    assert_eq!(playlist_names(&db).await?, ["concurrent"], "after retry");
    drop(reopened);
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn completed_pull_keeps_deleted_playlist_absent_after_restart() -> Result<()> {
    tokio_time::timeout(
        Duration::from_secs(120),
        restart_after(RestartAfter::CompletedPull),
    )
    .await?
}

#[tokio::test]
async fn interrupted_pull_keeps_deleted_playlist_absent_after_restart_and_retry() -> Result<()> {
    tokio_time::timeout(
        Duration::from_secs(120),
        restart_after(RestartAfter::StaleDelivery),
    )
    .await?
}
