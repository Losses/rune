pub mod blobs;
pub mod client;
pub mod downloads;
#[cfg(feature = "sync-crash-probe")]
mod fault_hook;
pub mod http;

use axum::{
    Json,
    body::Body,
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use database::sync::model::RuneModel;
use futures::StreamExt;
use tokio::sync::{Mutex, RwLock};
use tower::ServiceExt;

use crate::{library_pool::LibraryPool, messages::LibraryContentChanged, utils::GlobalParams};
use http::{BlobContext, SharedLibrary, blob_router};
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

#[derive(Clone)]
struct PoolService {
    pool: Arc<LibraryPool>,
    global: Arc<GlobalParams>,
    node_id: Uuid,
}

pub(super) fn pool_router(global: Arc<GlobalParams>, node_id: Uuid) -> axum::Router {
    axum::Router::new()
        .fallback(serve_library)
        .with_state(PoolService {
            pool: global.library_pool.clone(),
            global,
            node_id,
        })
}

pub(super) fn library_notification(
    global: &GlobalParams,
    root: &str,
) -> Arc<dyn Fn() + Send + Sync> {
    let broadcaster = global.broadcaster.clone();
    let path = root.to_owned();
    Arc::new(move || broadcaster.broadcast(&LibraryContentChanged { path: path.clone() }))
}

async fn serve_library(
    State(service): State<PoolService>,
    Path(path): Path<LibraryPath>,
    request: Request,
) -> Response {
    let library = match path.library_id.parse::<Uuid>() {
        Ok(id) => service.pool.acquire_served(id).await,
        Err(_) => Ok(None),
    };
    let library = match library {
        Ok(Some(library)) => library,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "library_not_served"})),
            )
                .into_response();
        }
        Err(error) => {
            log::warn!("Could not acquire served library: {error:#}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let notify = library_notification(&service.global, &library.media_root);
    let store = Arc::new(RwLock::new(library.library_sync.clone()));
    let router = library_router(SharedLibrary(store.clone()), notify.clone()).merge(blob_router(
        BlobContext {
            store,
            db: (*library.main_db).clone(),
            fsio: library.fsio.clone(),
            root: library.media_root.clone().into(),
            node_id: service.node_id,
            upload_lock: Arc::new(Mutex::new(())),
            notify,
        },
    ));
    let response = match router.oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    };
    let (parts, body) = response.into_parts();
    // Keep the lease until a streaming body has finished or been dropped.
    let stream = body.into_data_stream().map(move |chunk| {
        let _lease = &library;
        chunk
    });
    Response::from_parts(parts, Body::from_stream(stream))
}

#[derive(serde::Deserialize)]
pub(super) struct LibraryPath {
    library_id: String,
}

pub(super) fn library_base(base: &str, library_id: Uuid) -> String {
    format!("{}/libraries/{library_id}", base.trim_end_matches('/'))
}
