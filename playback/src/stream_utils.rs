use anyhow::{Result, anyhow};
use std::io::{self, Seek};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use stream_download::http::HttpStream;
use stream_download::{Settings, StreamDownload, storage::temp::TempStorageProvider};
use symphonia::core::io::{MediaSource, ReadOnlySource};

pub async fn create_stream_from_url(url: &str) -> Result<StreamDownload<TempStorageProvider>> {
    StreamDownload::new_http(
        url.parse()?,
        TempStorageProvider::new(),
        Settings::default(),
    )
    .await
    .map_err(|e| anyhow!(e.to_string()))
}

pub async fn create_stream_media_source_from_url(url: &str) -> Result<Box<dyn MediaSource>> {
    let reader = create_stream_from_url(url).await?;
    let source = Box::new(ReadOnlySource::new(reader));
    Ok(source)
}

/// Opens a stream using the caller's authenticated and certificate-pinned client.
pub async fn create_stream_with_client(
    client: reqwest::Client,
    url: &str,
) -> Result<StreamDownload<TempStorageProvider>> {
    let stream = HttpStream::new(client, url.parse()?)
        .await
        .map_err(|e| anyhow!(e.to_string()))?;
    StreamDownload::from_stream(
        stream,
        TempStorageProvider::new(),
        Settings::default().prefetch_bytes(0),
    )
    .await
    .map_err(|e| anyhow!(e.to_string()))
}

/// Adapts a caller capability to the downloader's initial and range requests.
#[derive(Clone)]
struct RenewableClient(Option<std::sync::Arc<dyn crate::player::AudioRequest>>);
impl stream_download::http::Client for RenewableClient {
    type Url = reqwest::Url;
    type Headers = reqwest::header::HeaderMap;
    type Response = reqwest::Response;
    type Error = std::io::Error;

    fn create() -> Self {
        Self(None)
    }
    async fn get(&self, _url: &Self::Url) -> std::io::Result<Self::Response> {
        let capability = self
            .0
            .as_ref()
            .ok_or_else(|| io::Error::other("missing audio request capability"))?;
        capability.get(None).await.map_err(io::Error::other)
    }
    async fn get_range(
        &self,
        _url: &Self::Url,
        start: u64,
        end: Option<u64>,
    ) -> std::io::Result<Self::Response> {
        let capability = self
            .0
            .as_ref()
            .ok_or_else(|| io::Error::other("missing audio request capability"))?;
        capability
            .get(Some((start, end)))
            .await
            .map_err(io::Error::other)
    }
}

pub async fn create_stream_with_authorizer(
    request: std::sync::Arc<dyn crate::player::AudioRequest>,
    url: &str,
) -> Result<StreamDownload<TempStorageProvider>> {
    let stream = HttpStream::new(RenewableClient(Some(request)), url.parse()?)
        .await
        .map_err(|e| anyhow!(e.to_string()))?;
    StreamDownload::from_stream(
        stream,
        TempStorageProvider::new(),
        Settings::default().prefetch_bytes(0),
    )
    .await
    .map_err(|e| anyhow!(e.to_string()))
}

/// Keeps a stream's lifetime tied to playback and stops reads after revocation.
pub struct ManagedStream {
    inner: StreamDownload<TempStorageProvider>,
    cancel: tokio_util::sync::CancellationToken,
    failed: Arc<AtomicBool>,
    cancellation_bridge: tokio::task::JoinHandle<()>,
}
impl ManagedStream {
    pub fn new(
        inner: StreamDownload<TempStorageProvider>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Self {
        Self::new_with_failure(inner, cancel, Arc::new(AtomicBool::new(false)))
    }
    pub fn new_with_failure(
        inner: StreamDownload<TempStorageProvider>,
        cancel: tokio_util::sync::CancellationToken,
        failed: Arc<AtomicBool>,
    ) -> Self {
        let download = inner.cancellation_token();
        let stop = cancel.clone();
        let failed_on_cancel = failed.clone();
        let cancellation_bridge = tokio::spawn(async move {
            stop.cancelled().await;
            failed_on_cancel.store(true, Ordering::Release);
            download.cancel();
        });
        Self {
            inner,
            cancel,
            failed,
            cancellation_bridge,
        }
    }
    fn ensure_active(&self) -> std::io::Result<()> {
        if self.cancel.is_cancelled() {
            self.failed.store(true, Ordering::Release);
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "audio access revoked",
            ))
        } else {
            Ok(())
        }
    }

    fn fail_on_incomplete_eof(
        &mut self,
        position: u64,
        result: std::io::Result<usize>,
    ) -> std::io::Result<usize> {
        let bytes_read = match result {
            Ok(bytes_read) => bytes_read,
            Err(error) => {
                self.failed.store(true, Ordering::Release);
                return Err(error);
            }
        };
        if bytes_read == 0
            && self
                .inner
                .content_length()
                .is_some_and(|length| position < length)
        {
            self.failed.store(true, Ordering::Release);
            return Err(io::Error::other(
                "audio stream stopped before download completed",
            ));
        }
        Ok(bytes_read)
    }
}
impl std::io::Read for ManagedStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.ensure_active()?;
        let position = self.inner.stream_position()?;
        let result = self.inner.read(buf);
        self.fail_on_incomplete_eof(position, result)
    }
}
impl std::io::Seek for ManagedStream {
    fn seek(&mut self, from: std::io::SeekFrom) -> std::io::Result<u64> {
        self.ensure_active()?;
        self.inner.seek(from)
    }
}
impl Drop for ManagedStream {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.inner.cancellation_token().cancel();
        self.cancellation_bridge.abort();
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use reqwest::header::HeaderMap;
    use std::io::ErrorKind;
    use std::io::Read;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio_util::sync::CancellationToken;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authenticated_client_reads_before_full_response_arrives() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (release, waiting) = oneshot::channel::<()>();
        let forced = Arc::new(AtomicBool::new(false));
        let forced_server = forced.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8192];
            let n = socket.read(&mut request).await.unwrap();
            assert!(
                String::from_utf8_lossy(&request[..n])
                    .to_ascii_lowercase()
                    .contains("authorization: bearer test")
            );
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            socket.write_all(&vec![7; 300 * 1024]).await.unwrap();
            if tokio::time::timeout(Duration::from_secs(3), waiting)
                .await
                .is_err()
            {
                forced_server.store(true, Ordering::SeqCst);
            }
            let _ = socket.write_all(&vec![9; 1024 * 1024 - 300 * 1024]).await;
        });
        let mut headers = HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, "Bearer test".parse()?);
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .no_proxy()
            .build()?;
        let mut stream = tokio::time::timeout(
            Duration::from_secs(5),
            create_stream_with_client(client, &format!("http://{address}/audio")),
        )
        .await??;
        let (first, stream) = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::task::spawn_blocking(move || {
                let mut bytes = [0; 4];
                stream.read_exact(&mut bytes).unwrap();
                (bytes, stream)
            }),
        )
        .await??;
        assert_eq!(first, [7; 4]);
        assert!(
            !forced.load(Ordering::SeqCst),
            "stream waited for full response"
        );
        release.send(()).unwrap();
        let cancel = CancellationToken::new();
        let mut managed = ManagedStream::new(stream, cancel.clone());
        cancel.cancel();
        assert_eq!(
            managed.read(&mut [0; 1]).unwrap_err().kind(),
            ErrorKind::PermissionDenied
        );
        drop(managed);
        server.await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn managed_reader_returns_error_when_range_download_fails() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut initial, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let _ = initial.read(&mut request).await.unwrap();
            initial
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            initial.write_all(&[1]).await.unwrap();
            drop(initial);

            let (mut range, _) = listener.accept().await.unwrap();
            let _ = range.read(&mut request).await.unwrap();
            range
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().build()?;
        let stream = tokio::time::timeout(
            Duration::from_secs(5),
            create_stream_with_client(client, &format!("http://{address}/audio")),
        )
        .await??;
        let mut managed = ManagedStream::new(stream, CancellationToken::new());
        let reader = tokio::task::spawn_blocking(move || {
            let mut output = vec![0; 1_048_576];
            managed.read_exact(&mut output)
        });
        let result = tokio::time::timeout(Duration::from_secs(5), reader).await??;
        assert!(result.is_err(), "failed range request must fail the read");
        server.await?;
        Ok(())
    }
}
