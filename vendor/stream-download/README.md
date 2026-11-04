# stream-download

[![crates.io](https://img.shields.io/crates/v/stream-download.svg?logo=rust)](https://crates.io/crates/stream-download)
[![docs.rs](https://img.shields.io/docsrs/stream-download?logo=rust)](https://docs.rs/stream-download)
![license](https://img.shields.io/badge/License-MIT%20or%20Apache%202-green.svg)
[![CI](https://github.com/aschey/stream-download-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/aschey/stream-download-rs/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/aschey/stream-download-rs/branch/main/graph/badge.svg?token=Wx7OgIb0qa)](https://app.codecov.io/gh/aschey/stream-download-rs)
![GitHub repo size](https://img.shields.io/github/repo-size/aschey/stream-download-rs)
![Lines of Code](https://aschey.tech/tokei/github/aschey/stream-download-rs)

[stream-download](https://github.com/aschey/stream-download-rs) is a library for
streaming content from a remote location and using it as a
[`read`](https://doc.rust-lang.org/stable/std/io/trait.Read.html) and
[`seek`](https://doc.rust-lang.org/stable/std/io/trait.Seek.html)-able source.
This is primarily useful for audio or video sources, but this library does not
make any assumptions about the content type of the data being streamed.

The requested content is downloaded in the background, allowing read and seek
operations to progress before the download is finished. Seek operations may
cause the stream to be restarted from the requested position if the download is
still in progress.

This library makes heavy use of the adapter pattern to allow for pluggable
transports and storage implementations.

**See the [blog post](https://aschey.tech/blog/streaming) for more info on the
approach used here.**

## Installation

```sh
cargo add stream-download
```

## Feature Flags

- `http` - adds an HTTP-based implementation of the
  [`SourceStream`](https://docs.rs/stream-download/latest/stream_download/source/trait.SourceStream.html)
  trait (enabled by default).
- `reqwest` - enables streaming content over http using
  [reqwest](https://crates.io/crates/reqwest) (enabled by default).
- `reqwest-rustls` - enables reqwest's `rustls` feature. Also enables the
  `reqwest` feature (enabled by default).
- `reqwest-native-tls` - enables reqwest's `native-tls` feature. Also enables
  the `reqwest` feature.
- `reqwest-middleware` - enables integration with
  [`reqwest-middleware`](https://crates.io/crates/reqwest-middleware). Can be
  used to add retry policies and additional observability. Also enables the
  `reqwest` feature.
- `async-read` - adds a `SourceStream` implementation for any type implementing
  [`AsyncRead`](https://docs.rs/tokio/latest/tokio/io/trait.AsyncRead.html).
- `process` - adds a `SourceStream` implementation for external processes. Also
  enables the `async-read` feature.
- `temp-storage` - adds a temporary file-based storage backend (enabled by
  default).
- `registry` - adds a method for routing inputs to different stream handlers
  based on a set of rules.

**NOTE**: The default features in this crate also enable `reqwest`'s default features. If
you need to disable some of these, disable the default features from this crate
and enable the
[features](https://docs.rs/reqwest/latest/reqwest/#optional-features) you need.

## Usage

```rust,no_run
use std::error::Error;
use std::io;
use std::io::Read;
use std::result::Result;

use stream_download::source::DecodeError;
use stream_download::storage::temp::TempStorageProvider;
use stream_download::{Settings, StreamDownload};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut reader = match StreamDownload::new_http(
        "https://some-cool-url.com/some-file.mp3".parse()?,
        TempStorageProvider::new(),
        Settings::default(),
    )
    .await
    {
        Ok(reader) => reader,
        Err(e) => Err(e.decode_error().await)?,
    };

    tokio::task::spawn_blocking(move || {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        Ok::<_, io::Error>(())
    })
    .await??;

    Ok(())
}
```

## Examples

See
[examples](https://github.com/aschey/stream-download-rs/tree/main/examples/examples).

## Transports

Transports implement the
[`SourceStream`](https://docs.rs/stream-download/latest/stream_download/source/trait.SourceStream.html)
trait. A few types of transports are provided out of the box:

- [`http`](https://docs.rs/stream-download/latest/stream_download/http) for
  typical HTTP-based sources.
- [`async_read`](https://docs.rs/stream-download/latest/stream_download/async_read)
  for any source implementing
  [`AsyncRead`](https://docs.rs/tokio/latest/tokio/io/trait.AsyncRead.html).
- [`process`](https://docs.rs/stream-download/latest/stream_download/process)
  for reading data from
  [an external process](https://docs.rs/tokio/latest/tokio/process/index.html).

Only `http` is enabled by default. You can provide a custom transport by
implementing `SourceStream` yourself.

### OpenDAL

If you need to support other types of transports, you may wish to use
[OpenDAL](https://crates.io/crates/opendal). An adapter is provided as a
[separate crate](https://crates.io/crates/stream-download-opendal).

## Streams with Unknown Length

Resources such as standalone songs or videos have a finite length that is used
to support certain seeking functionality. Live streams or those that otherwise
don't have a known length are still supported, but attempting to seek from the
end of the stream will return an error. This may cause issues with certain audio
or video libraries that attempt to perform such seek operations. If it's
necessary to explicitly check for an infinite stream, you can check the stream's
content length ahead of time.

```rust,no_run
use std::error::Error;
use std::io;
use std::io::Read;
use std::result::Result;

use stream_download::http::HttpStream;
use stream_download::http::reqwest::Client;
use stream_download::source::DecodeError;
use stream_download::source::SourceStream;
use stream_download::storage::temp::TempStorageProvider;
use stream_download::{Settings, StreamDownload};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let stream =
        HttpStream::<Client>::create("https://some-cool-url.com/some-stream".parse()?).await?;
    let content_length = stream.content_length();
    let is_infinite = content_length.is_none();
    println!("Infinite stream = {is_infinite}");

    let mut reader = match StreamDownload::from_stream(
        stream,
        TempStorageProvider::default(),
        Settings::default(),
    )
    .await
    {
        Ok(reader) => reader,
        Err(e) => Err(e.decode_error().await)?,
    };

    tokio::task::spawn_blocking(move || {
        let mut buf = [0; 256];
        reader.read_exact(&mut buf)?;
        Ok::<_, io::Error>(())
    })
    .await??;

    Ok(())
}
```

### Icecast/Shoutcast Streams

If you're using this library to handle Icecast streams or one if its
derivatives, check out the [icy-metadata](https://crates.io/crates/icy-metadata)
crate. There are examples for how to use it with `stream-download`
[in the repo](https://github.com/aschey/icy-metadata/tree/main/examples).

## Streaming from YouTube and Similar Sites

Some websites with embedded audio or video streams can be tricky to handle
directly. For these cases, it's easier to use a dedicated program such as
[yt-dlp](https://github.com/yt-dlp/yt-dlp) which can parse media from specific
websites (it supports more websites than just YouTube, despite the name). If you
enable the `process` feature, you can integrate with external programs like
`yt-dlp` that can send their output to `stdout`. Some helpers for interacting
with `yt-dlp` and `ffmpeg` (for post-processing) are also included.

See
[youtube_simple](https://github.com/aschey/stream-download-rs/blob/main/examples/examples/youtube_simple.rs)
for a simple way to stream audio from a YouTube video.

See
[yt_dlp](https://github.com/aschey/stream-download-rs/blob/main/examples/examples/yt_dlp.rs)
for a more complex example of handling different kinds of URLs with `yt-dlp`.

## Registry

Some applications may need to use multiple types of stream handlers. For
example, you may want to handle YouTube URLs with `yt-dlp`, normal HTTP URLs
with `reqwest`, and `file://` URLs with a simple
[`BufReader`](https://doc.rust-lang.org/std/io/struct.BufReader.html). You can
use a
[`registry`](https://docs.rs/stream-download/latest/stream_download/registry/index.html)
to handle these cases.

See the
[example](https://github.com/aschey/stream-download-rs/blob/main/examples/examples/registry/main.rs)
for more info.

## Storage

The
[storage](https://docs.rs/stream-download/latest/stream_download/storage/index.html)
module provides ways to customize how the stream is cached locally.
Pre-configured implementations are available for memory and temporary file-based
storage. Typically you'll want to use temporary file-based storage to prevent
using too much memory, but memory-based storage may be preferable if you know
the stream size is small or you need to run your application on a read-only
filesystem.

```rust,no_run
use std::error::Error;
use std::io::Read;
use std::result::Result;

use stream_download::source::DecodeError;
use stream_download::storage::memory::MemoryStorageProvider;
use stream_download::{Settings, StreamDownload};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut reader = match StreamDownload::new_http(
        "https://some-cool-url.com/some-file.mp3".parse()?,
        // buffer will be stored in memory instead of on disk
        MemoryStorageProvider,
        Settings::default(),
    )
    .await
    {
        Ok(reader) => reader,
        Err(e) => Err(e.decode_error().await)?,
    };

    Ok(())
}
```

### Bounded Storage

When using infinite streams which don't need to support seeking, it usually
isn't desirable to let the underlying cache grow indefinitely if the stream may
be running for a while. For these cases, you may want to use
[bounded storage](https://docs.rs/stream-download/latest/stream_download/storage/bounded/index.html).
Bounded storage uses a circular buffer which will overwrite the oldest contents
once it fills up. If the reader falls too far behind the writer, the writer will
pause so the reader can catch up.

```rust,no_run
use std::error::Error;
use std::io::Read;
use std::num::NonZeroUsize;
use std::result::Result;

use stream_download::source::DecodeError;
use stream_download::storage::bounded::BoundedStorageProvider;
use stream_download::storage::memory::MemoryStorageProvider;
use stream_download::{Settings, StreamDownload};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut reader = match StreamDownload::new_http(
        "https://some-cool-url.com/some-file.mp3".parse()?,
        // use bounded storage to keep the underlying size from growing indefinitely
        BoundedStorageProvider::new(
            // you can use any other kind of storage provider here
            MemoryStorageProvider,
            // be liberal with the buffer size, you need to make sure it holds enough space to
            // prevent any out-of-bounds reads
            NonZeroUsize::new(512 * 1024).unwrap(),
        ),
        Settings::default(),
    )
    .await
    {
        Ok(reader) => reader,
        Err(e) => Err(e.decode_error().await)?,
    };

    Ok(())
}
```

### Adaptive Storage

When you need to support both finite and infinite streams, you may want to use
[adaptive storage](https://docs.rs/stream-download/latest/stream_download/storage/adaptive/index.html).
This is a convenience wrapper that will use bounded storage when the stream has
no content length and unbounded storage when the stream does return a content
length.

```rust,no_run
use std::error::Error;
use std::io::Read;
use std::num::NonZeroUsize;
use std::result::Result;

use stream_download::source::DecodeError;
use stream_download::storage::adaptive::AdaptiveStorageProvider;
use stream_download::storage::temp::TempStorageProvider;
use stream_download::{Settings, StreamDownload};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut reader = match StreamDownload::new_http(
        "https://some-cool-url.com/some-file.mp3".parse()?,
        // use adaptive storage to keep the underlying size from growing indefinitely
        // when the content type is not known
        AdaptiveStorageProvider::new(
            // you can use any other kind of storage provider here
            TempStorageProvider::default(),
            // be liberal with the buffer size, you need to make sure it holds enough space to
            // prevent any out-of-bounds reads
            NonZeroUsize::new(512 * 1024).unwrap(),
        ),
        Settings::default(),
    )
    .await
    {
        Ok(reader) => reader,
        Err(e) => return Err(e.decode_error().await)?,
    };

    Ok(())
}
```

## Handling Errors and Reconnects

Some automatic support is available for retrying stalled streams. See the docs
for
[the `StreamDownload` struct](https://docs.rs/stream-download/latest/stream_download/struct.StreamDownload.html)
for more details.

If using `reqwest-middleware`, a retry policy can be used to handle transient
server errors. See
[retry_middleware](https://github.com/aschey/stream-download-rs/blob/main/examples/examples/retry_middleware.rs)
for an example of adding retry middleware.

## Authentication and Other Customization

It's possible to customize your HTTP requests if you need to perform
authentication or change other settings.

See
[client_options](https://github.com/aschey/stream-download-rs/blob/main/examples/examples/client_options.rs)
for customizing the HTTP client builder.

See
[custom_client](https://github.com/aschey/stream-download-rs/blob/main/examples/examples/custom_client.rs)
for dynamically modifying each HTTP request.

## Supported Rust Versions

The MSRV is currently `1.95.0`. Since Cargo's V3 resolver supports MSRV-aware
dependencies, we do not treat an MSRV bump as a breaking change.
