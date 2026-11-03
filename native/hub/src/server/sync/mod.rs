pub mod blobs;
pub mod client;
pub mod downloads;
#[cfg(feature = "sync-crash-probe")]
mod fault_hook;
pub mod http;

use axum::{
    Json,
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use database::sync::model::RuneModel;
use std::sync::Arc;
use uuid::Uuid;

pub fn library_router(
    library: http::SharedLibrary,
    notify: Arc<dyn Fn() + Send + Sync>,
) -> axum::Router {
    http_request::sync::router_with_provider::<RuneModel>(Arc::new(move || {
        let library = library.clone();
        let notify = notify.clone();
        Box::pin(async move {
            Arc::new(library.0.read().await.endpoint().with_notification(notify))
                as Arc<dyn http_request::sync::MetadataStore<RuneModel>>
        })
    }))
}

/// The paired service serves exactly one open library.
pub(super) async fn require_library(
    State(served): State<Uuid>,
    Path(path): Path<LibraryPath>,
    request: Request,
    next: Next,
) -> Response {
    if path.library_id.parse::<Uuid>().ok() != Some(served) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "library_not_served"})),
        )
            .into_response();
    }
    next.run(request).await
}

#[derive(serde::Deserialize)]
pub(super) struct LibraryPath {
    library_id: String,
}

pub(super) fn library_base(base: &str, library_id: Uuid) -> String {
    format!("{}/libraries/{library_id}", base.trim_end_matches('/'))
}
