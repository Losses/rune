#[path = "support/causal_library.rs"]
mod support;

use anyhow::Result;
use database::sync::model::RuneModel;
use database::sync::model::*;
use http_request::sync::router;
type MetadataPeer = http_request::sync::MetadataPeer<RuneModel>;
use support::{SqlLibrary, key};
use sync::{self as p, SyncPeer};
use uuid::Uuid;

#[tokio::test]
async fn http_transfers_both_directions_and_rejects_large_requests() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let object = key(Collection::Playlists, Uuid::new_v4());
    source
        .peer
        .write_local(object.clone(), support::playlist("source"))
        .await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let app = router(source.peer.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let remote = MetadataPeer::new(&url, reqwest::Client::new());
    let scope = Scope {
        collections: [Collection::Playlists].into_iter().collect(),
    };
    support::drive(
        &*target.peer,
        &*target.peer,
        &*target.peer,
        &remote,
        &remote,
        scope.clone(),
    )
    .await?;
    assert!(target.peer.get(&object).await?.is_some());
    let reverse = key(Collection::Playlists, Uuid::new_v4());
    target
        .peer
        .write_local(reverse.clone(), support::playlist("target"))
        .await?;
    let source_peer = p::StorePeer(target.peer.clone());
    support::drive(
        &remote,
        &remote,
        &remote,
        &source_peer,
        &*target.peer,
        scope.clone(),
    )
    .await?;
    assert!(source.peer.get(&reverse).await?.is_some());
    target.peer.write_local(reverse.clone(), None).await?;
    support::drive(
        &remote,
        &remote,
        &remote,
        &source_peer,
        &*target.peer,
        scope,
    )
    .await?;
    assert!(source.peer.get(&reverse).await?.is_none());
    let status = reqwest::Client::new()
        .post(format!("{url}/sync/metadata"))
        .header("content-type", "application/json")
        .body(" ".repeat(9 * 1024 * 1024 + 1))
        .send()
        .await?
        .status();
    assert_eq!(status, reqwest::StatusCode::PAYLOAD_TOO_LARGE);
    server.abort();
    Ok(())
}

#[tokio::test]
async fn rejects_large_responses_with_and_without_content_length() -> Result<()> {
    use axum::{Router, body::Body, routing::post};
    let app = Router::new()
        .route(
            "/length/sync/metadata",
            post(|| async { "x".repeat(32 * 1024 * 1024 + 1) }),
        )
        .route(
            "/stream/sync/metadata",
            post(|| async {
                Body::from_stream(futures::stream::iter(
                    (0..33).map(|_| Ok::<_, std::io::Error>(vec![b'x'; 1024 * 1024])),
                ))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    for path in ["length", "stream"] {
        let remote = MetadataPeer::new(format!("http://{address}/{path}"), reqwest::Client::new());
        assert!(matches!(
            SyncPeer::identity(&remote).await,
            Err(p::SyncError::LogicalLimit)
        ));
    }
    server.abort();
    Ok(())
}

#[tokio::test]
async fn connection_failure_keeps_checkpoint() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = support::memory(namespace, Uuid::new_v4())?;
    let target = support::memory(namespace, Uuid::new_v4())?;
    source
        .write_local(
            key(Collection::Playlists, Uuid::new_v4()),
            support::playlist("one"),
        )
        .await?;
    let scope = Scope {
        collections: [Collection::Playlists].into_iter().collect(),
    };
    let identity = source.identity().await?;
    let checkpoint = p::CheckpointKey {
        namespace,
        source_replica: identity.replica,
        source_generation: identity.generation,
        scope_id: p::scope_id(&scope)?,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let app = router(source);
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let remote = MetadataPeer::new(&url, reqwest::Client::new());
    support::drive(
        &*target,
        &*target,
        &*target,
        &remote,
        &remote,
        scope.clone(),
    )
    .await?;
    let before = target.checkpoint(&checkpoint).await?;
    assert_eq!(before.as_ref().unwrap().sequence, 1);
    server.abort();
    let _ = server.await;
    let remote = MetadataPeer::new(&url, reqwest::Client::new());
    assert!(
        support::drive(&*target, &*target, &*target, &remote, &remote, scope)
            .await
            .is_err()
    );
    assert_eq!(target.checkpoint(&checkpoint).await?, before);
    Ok(())
}
