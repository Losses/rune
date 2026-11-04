//! Adapters for using [`reqwest`] with `stream-download`

use std::str::FromStr;
use std::sync::LazyLock;

use bytes::Bytes;
use futures_util::Stream;
use reqwest::header::{self, AsHeaderName, HeaderMap};
use tracing::warn;

use super::{DecodeError, RANGE_HEADER_KEY, format_range_header_bytes};
use crate::http::{Client, ClientResponse, ResponseHeaders};

impl ResponseHeaders for HeaderMap {
    fn header(&self, name: &str) -> Option<&str> {
        get_header_str(self, name)
    }
}

fn get_header_str<K: AsHeaderName>(headers: &HeaderMap, key: K) -> Option<&str> {
    headers.get(key).and_then(|val| {
        val.to_str()
            .inspect_err(|e| warn!("error converting header value: {e:?}"))
            .ok()
    })
}

/// Error returned when making an HTTP call
#[derive(thiserror::Error, Debug)]
#[error("Failed to fetch: {source}")]
pub struct FetchError {
    #[source]
    source: reqwest::Error,
    // Box to prevent large error variant warnings
    response: Box<reqwest::Response>,
}

impl FetchError {
    /// Error source.
    pub fn source(&self) -> &reqwest::Error {
        &self.source
    }

    /// Http response.
    pub fn response(&self) -> &reqwest::Response {
        &self.response
    }
}

impl DecodeError for FetchError {
    async fn decode_error(self) -> String {
        match self.response.text().await {
            Ok(text) => format!("{}: {text}", self.source),
            Err(e) => format!("{}. Error decoding response: {e}", self.source),
        }
    }
}

impl ClientResponse for reqwest::Response {
    type ResponseError = FetchError;
    type StreamError = reqwest::Error;
    type Headers = HeaderMap;

    fn content_length(&self) -> Option<u64> {
        get_header_str(self.headers(), header::CONTENT_LENGTH).and_then(|content_length| {
            u64::from_str(content_length)
                .inspect_err(|e| warn!("invalid content length value: {e:?}"))
                .ok()
        })
    }

    fn content_type(&self) -> Option<&str> {
        get_header_str(self.headers(), header::CONTENT_TYPE)
    }

    fn headers(&self) -> Self::Headers {
        self.headers().clone()
    }

    fn into_result(self) -> Result<Self, Self::ResponseError> {
        if let Err(error) = self.error_for_status_ref() {
            Err(FetchError {
                source: error,
                response: Box::new(self),
            })
        } else {
            Ok(self)
        }
    }

    fn stream(
        self,
    ) -> Box<dyn Stream<Item = Result<Bytes, Self::StreamError>> + Unpin + Send + Sync> {
        Box::new(self.bytes_stream())
    }
}

// per reqwest's docs, it's advisable to create a single client and reuse it
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

impl Client for reqwest::Client {
    type Url = reqwest::Url;
    type Response = reqwest::Response;
    type Error = reqwest::Error;
    type Headers = HeaderMap;

    fn create() -> Self {
        CLIENT.clone()
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
