use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;
use reqwest::header::HeaderMap;
use reqwest_middleware::Middleware;

use super::{Client, RANGE_HEADER_KEY, format_range_header_bytes};

static DEFAULT_MIDDLEWARE: LazyLock<Mutex<Vec<Arc<dyn reqwest_middleware::Middleware>>>> =
    LazyLock::new(|| Mutex::new([].into()));

pub(crate) fn add_default_middleware<M>(middleware: M)
where
    M: Middleware,
{
    DEFAULT_MIDDLEWARE.lock().push(Arc::new(middleware));
}

impl Client for reqwest_middleware::ClientWithMiddleware {
    type Url = reqwest::Url;
    type Response = reqwest::Response;
    type Error = reqwest_middleware::Error;
    type Headers = HeaderMap;

    fn create() -> Self {
        Self::new(
            reqwest::Client::create(),
            DEFAULT_MIDDLEWARE.lock().clone().into_boxed_slice(),
        )
    }

    async fn get(&self, url: &Self::Url) -> Result<Self::Response, Self::Error> {
        self.get(url.clone()).send().await
    }

    async fn get_range(
        &self,
        url: &Self::Url,
        start: u64,
        end: Option<u64>,
    ) -> Result<Self::Response, Self::Error> {
        self.get(url.clone())
            .header(RANGE_HEADER_KEY, format_range_header_bytes(start, end))
            .send()
            .await
    }
}
