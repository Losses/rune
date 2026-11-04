//! An HTTP implementation of the [`SourceStream`] trait.
//!
//! An implementation of the [Client] trait using [reqwest](https://docs.rs/reqwest/latest/reqwest)
//! is provided if the `reqwest` feature is enabled. If you need to customize the client object, you
//! can use [`HttpStream::new`](crate::http::HttpStream::new) to supply your own reqwest client.
//! Keep in mind that reqwest recommends creating a single client and cloning it for each new
//! connection.
//!
//! # Example
//!
//! ```no_run
//! use std::error::Error;
//! use std::result::Result;
//!
//! use stream_download::http::HttpStream;
//! use stream_download::http::reqwest::Client;
//! use stream_download::source::SourceStream;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn Error>> {
//!     let stream = HttpStream::new(
//!         Client::new(),
//!         "https://some-cool-url.com/some-file.mp3".parse()?,
//!     )
//!     .await?;
//!     let content_length = stream.content_length();
//!     Ok(())
//! }
//! ```

use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::io;
use std::pin::Pin;
use std::task::{self, Poll};
use std::time::Instant;

use bytes::Bytes;
use educe::Educe;
use futures_util::{Future, Stream};
use mediatype::MediaTypeBuf;
#[cfg(feature = "reqwest")]
pub use reqwest;
use tracing::{debug, instrument, warn};

use crate::WrapIoResult;
use crate::source::{DecodeError, SourceStream};

#[cfg(feature = "reqwest")]
pub mod reqwest_client;

#[cfg(feature = "reqwest-middleware")]
pub(crate) mod reqwest_middleware_client;

/// Wrapper trait for an HTTP client that exposes only functionality necessary for retrieving the
/// stream content. If the `reqwest` feature is enabled, this trait is implemented for
/// [reqwest::Client](https://docs.rs/reqwest/latest/reqwest/struct.Client.html).
/// This can be implemented for a custom HTTP client if desired.
pub trait Client: Send + Sync + Unpin + 'static {
    /// The HTTP URL of the remote resource.
    type Url: Display + Send + Sync + Unpin;

    /// The type that contains the HTTP response headers.
    type Headers: ResponseHeaders;

    /// The HTTP response object.
    type Response: ClientResponse<Headers = Self::Headers>;

    /// The error type returned by HTTP requests.
    type Error: Error + Send + Sync;

    /// Creates a new instance of the client.
    fn create() -> Self;

    /// Sends an HTTP GET request to the URL.
    fn get(
        &self,
        url: &Self::Url,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send;

    /// Sends an HTTP GET request to the URL utilizing the `Range` header to request a specific part
    /// of the stream.
    ///
    /// The end value should be inclusive, per the HTTP spec.
    fn get_range(
        &self,
        url: &Self::Url,
        start: u64,
        end: Option<u64>,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send;
}

/// Represents the content type HTTP response header
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentType {
    /// The top-level content type such as application, audio, video, etc.
    pub r#type: String,
    /// The specific subtype such as mpeg, mp4, ogg, etc.
    pub subtype: String,
}

/// A trait for getting a specific header value
pub trait ResponseHeaders: Send + Sync + Unpin {
    /// Get a specific header from the response.
    /// If the value is not present or it can't be decoded as a string, `None` is returned.
    fn header(&self, name: &str) -> Option<&str>;
}

/// A wrapper trait for an HTTP response that exposes only functionality necessary for retrieving
/// the stream content. If the `reqwest` feature is enabled,
/// this trait is implemented for
/// [reqwest::Response](https://docs.rs/reqwest/latest/reqwest/struct.Response.html).
/// This can be implemented for a custom HTTP response if desired.
pub trait ClientResponse: Send + Sync + Sized {
    /// Error type returned by the underlying response.
    type ResponseError: DecodeError + Send;
    /// Error type returned by the stream.
    type StreamError: Error + Send + Sync;
    /// Object containing HTTP response headers.
    type Headers: ResponseHeaders;

    /// The size of the remote resource in bytes.
    /// The result should be `None` if the stream is infinite or doesn't have a known length.
    fn content_length(&self) -> Option<u64>;

    /// The content-type response header.
    /// This should be in the standard format of `<type>/<subtype>`.
    fn content_type(&self) -> Option<&str>;

    /// Object containing HTTP response headers.
    fn headers(&self) -> Self::Headers;

    /// Turns the response into an error if the response was not successful.
    fn into_result(self) -> Result<Self, Self::ResponseError>;

    /// Converts the response into a byte stream
    fn stream(
        self,
    ) -> Box<dyn Stream<Item = Result<Bytes, Self::StreamError>> + Unpin + Send + Sync>;
}

fn fmt<T>(val: &T, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error>
where
    T: Display,
{
    write!(fmt, "{val}")
}

/// Error returned from an HTTP stream.
#[derive(thiserror::Error, Educe)]
#[educe(Debug)]
pub enum HttpStreamError<C: Client> {
    /// Failed to fetch.
    #[error("Failed to fetch: {0}")]
    FetchFailure(C::Error),
    /// Failed to get response.
    #[error("Failed to get response: {0}")]
    ResponseFailure(<<C as Client>::Response as ClientResponse>::ResponseError),
}

impl<C: Client> DecodeError for HttpStreamError<C> {
    async fn decode_error(self) -> String {
        match self {
            Self::ResponseFailure(e) => e.decode_error().await,
            this @ Self::FetchFailure(_) => this.to_string(),
        }
    }
}

/// An HTTP implementation of the [`SourceStream`] trait.
#[derive(Educe)]
#[educe(Debug)]
pub struct HttpStream<C: Client> {
    #[educe(Debug = false)]
    stream: Box<
        dyn Stream<Item = Result<Bytes, <<C as Client>::Response as ClientResponse>::StreamError>>
            + Unpin
            + Send
            + Sync,
    >,
    client: C,
    content_length: Option<u64>,
    content_type: Option<ContentType>,
    #[educe(Debug(method = "fmt"))]
    url: C::Url,
    #[educe(Debug = false)]
    headers: C::Headers,
}

impl<C: Client> HttpStream<C> {
    /// Creates a new [`HttpStream`] from a [`Client`].
    #[instrument(skip(client, url), fields(url = url.to_string()))]
    pub async fn new(
        client: C,
        url: <Self as SourceStream>::Params,
    ) -> Result<Self, HttpStreamError<C>> {
        debug!("requesting stream content");
        let request_start = Instant::now();

        let response = client
            .get(&url)
            .await
            .map_err(HttpStreamError::FetchFailure)?;
        debug!(
            duration = format!("{:?}", request_start.elapsed()),
            "request finished"
        );

        let response = response
            .into_result()
            .map_err(HttpStreamError::ResponseFailure)?;
        let content_length = response.content_length().map_or_else(
            || {
                warn!("content length header missing");
                None
            },
            |content_length| {
                debug!(content_length, "received content length");
                Some(content_length)
            },
        );

        let content_type = response.content_type().map_or_else(
            || {
                warn!("content type header missing");
                None
            },
            |content_type| {
                debug!(content_type, "received content type");
                match content_type.parse::<MediaTypeBuf>() {
                    Ok(content_type) => Some(ContentType {
                        r#type: content_type.ty().to_string(),
                        subtype: content_type.subty().to_string(),
                    }),
                    Err(e) => {
                        warn!("error parsing content type: {e:?}");
                        None
                    }
                }
            },
        );

        let headers = response.headers();
        let stream = response.stream();
        Ok(Self {
            stream: Box::new(stream),
            client,
            content_length,
            content_type,
            headers,
            url,
        })
    }

    /// The [`ContentType`] of the response stream.
    pub fn content_type(&self) -> &Option<ContentType> {
        &self.content_type
    }

    /// Get a specific header from the response.
    /// If the value is not present or it can't be decoded as a string, `None` is returned.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.header(name)
    }

    /// Object containing HTTP response headers.
    pub fn headers(&self) -> &C::Headers {
        &self.headers
    }

    fn supports_range_request(&self) -> bool {
        match self.header("Accept-Ranges") {
            Some(val) => val != "none",
            None => false,
        }
    }
}

impl<C: Client> Stream for HttpStream<C> {
    type Item = Result<Bytes, <<C as Client>::Response as ClientResponse>::StreamError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.stream).poll_next(cx)
    }
}

impl<C: Client> SourceStream for HttpStream<C> {
    type Params = C::Url;
    type StreamCreationError = HttpStreamError<C>;

    async fn create(params: Self::Params) -> Result<Self, Self::StreamCreationError> {
        Self::new(C::create(), params).await
    }

    fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    #[instrument(skip(self))]
    async fn seek_range(&mut self, start: u64, end: Option<u64>) -> io::Result<()> {
        if Some(start) == self.content_length {
            debug!(
                "attempting to seek where start is the length of the stream, returning empty \
                 stream"
            );
            self.stream = Box::new(futures_util::stream::empty());
            return Ok(());
        }

        if !self.supports_range_request() {
            warn!("Accept-Ranges header not present. Attempting seek anyway.");
        }

        debug!("sending HTTP range request");
        let request_start = Instant::now();
        let response = self
            .client
            // seek_range provides an exclusive end value, but we need it to be inclusive here
            .get_range(&self.url, start, end.map(|e| e - 1))
            .await
            .map_err(|e| io::Error::other(e.to_string()))
            .wrap_err(&format!("error sending HTTP range request to {}", self.url))?;
        debug!(
            duration = format!("{:?}", request_start.elapsed()),
            "HTTP request finished"
        );

        let response = match response.into_result() {
            Ok(response) => Ok(response),
            Err(e) => {
                let error = e.decode_error().await;
                Err(io::Error::other(error)).wrap_err(&format!(
                    "error getting HTTP range response from {}",
                    self.url
                ))
            }
        }?;
        self.stream = Box::new(response.stream());
        debug!("done seeking");
        Ok(())
    }

    async fn reconnect(&mut self, current_position: u64) -> Result<(), io::Error> {
        if self.supports_range_request() {
            self.seek_range(current_position, None).await
        } else {
            let response = self
                .client
                .get(&self.url)
                .await
                .map_err(|e| io::Error::other(e.to_string()))
                .wrap_err(&format!("error sending HTTP request to {}", self.url))?;
            self.stream = Box::new(response.stream());
            Ok(())
        }
    }

    fn supports_seek(&self) -> bool {
        true
    }
}

/// HTTP range header key
pub const RANGE_HEADER_KEY: &str = "Range";

/// Utility function to format a range header for requesting bytes.
///
/// ex: `bytes=200-400`
pub fn format_range_header_bytes(start: u64, end: Option<u64>) -> String {
    format!(
        "bytes={start}-{}",
        end.map(|e| e.to_string()).unwrap_or_default()
    )
}
