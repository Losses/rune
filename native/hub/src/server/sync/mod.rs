pub mod blobs;
pub mod client;
pub mod downloads;
#[cfg(feature = "sync-crash-probe")]
mod fault_hook;
pub mod http;

use database::sync::model::RuneModel;
use std::sync::Arc;

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
