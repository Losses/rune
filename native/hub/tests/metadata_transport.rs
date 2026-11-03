//! Actual HTTP transport preserves recovery errors without exposing local commands.

use anyhow::Result;
use axum::{
    Json, Router,
    http::{StatusCode, header},
    routing::post,
};
use database::sync::model::RuneModel;
type MetadataPeer = http_request::sync::MetadataPeer<RuneModel>;
use sync::protocol as p;
use uuid::Uuid;

#[tokio::test]
async fn typed_recovery_errors_survive_http_and_unknown_operations_are_not_sent() -> Result<()> {
    let expected = p::SyncError::NeedsReconciliation(p::HistoryStatus::BelowFloor {
        floor: p::ReceiveProgress {
            sequence: 9,
            anchor: [7; 32],
        },
    });
    let error = expected.clone();
    let app = Router::new().route(
        "/sync/metadata",
        post(move |Json(request): Json<serde_json::Value>| {
            let error = error.clone();
            async move {
                assert_eq!(request["operation"], "history");
                assert!(request["progress"].is_null());
                assert_eq!(request["key"]["scope_id"], "03".repeat(32));
                Json(p::Result::<p::HistoryStatus>::Err(error))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let peer = MetadataPeer::new(base, reqwest::Client::new());
    let key = p::CheckpointKey {
        namespace: Uuid::new_v4(),
        source_replica: Uuid::new_v4(),
        source_generation: Uuid::new_v4(),
        scope_id: [3; 32],
    };
    let result = p::SyncPeer::history_status(&peer, &key, None).await;
    server.abort();
    let _ = server.await;
    assert_eq!(result, Err(expected));
    Ok(())
}

#[tokio::test]
async fn unauthorized_http_is_typed_without_a_successful_empty_head() -> Result<()> {
    let app = Router::new().route(
        "/sync/metadata",
        post(|| async {
            (
                StatusCode::UNAUTHORIZED,
                [(header::CONTENT_TYPE, "text/plain")],
                "expired",
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let peer = MetadataPeer::new(base, reqwest::Client::new());
    let result = p::SyncPeer::head(&peer).await;
    server.abort();
    let _ = server.await;
    assert_eq!(result, Err(p::SyncError::Unauthorized));
    Ok(())
}
