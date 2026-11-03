use anyhow::{Result, anyhow};
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
    let stream = stream_download::http::HttpStream::new(client, url.parse()?)
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
            .ok_or_else(|| std::io::Error::other("missing audio request capability"))?;
        capability.get(None).await.map_err(std::io::Error::other)
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
            .ok_or_else(|| std::io::Error::other("missing audio request capability"))?;
        capability
            .get(Some((start, end)))
            .await
            .map_err(std::io::Error::other)
    }
}

pub async fn create_stream_with_authorizer(
    request: std::sync::Arc<dyn crate::player::AudioRequest>,
    url: &str,
) -> Result<StreamDownload<TempStorageProvider>> {
    let stream =
        stream_download::http::HttpStream::new(RenewableClient(Some(request)), url.parse()?)
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
    cancellation_bridge: tokio::task::JoinHandle<()>,
}
impl ManagedStream {
    pub fn new(
        inner: StreamDownload<TempStorageProvider>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Self {
        let download = inner.cancellation_token();
        let stop = cancel.clone();
        let cancellation_bridge = tokio::spawn(async move {
            stop.cancelled().await;
            download.cancel();
        });
        Self {
            inner,
            cancel,
            cancellation_bridge,
        }
    }
    fn ensure_active(&self) -> std::io::Result<()> {
        if self.cancel.is_cancelled() {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "audio access revoked",
            ))
        } else {
            Ok(())
        }
    }
}
impl std::io::Read for ManagedStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.ensure_active()?;
        self.inner.read(buf)
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
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::oneshot;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authenticated_client_reads_before_full_response_arrives() -> Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (release, waiting) = oneshot::channel::<()>();
        let forced = std::sync::Arc::new(AtomicBool::new(false));
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
            if tokio::time::timeout(std::time::Duration::from_secs(3), waiting)
                .await
                .is_err()
            {
                forced_server.store(true, Ordering::SeqCst);
            }
            let _ = socket.write_all(&vec![9; 1024 * 1024 - 300 * 1024]).await;
        });
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, "Bearer test".parse()?);
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .no_proxy()
            .build()?;
        let mut stream = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            create_stream_with_client(client, &format!("http://{address}/audio")),
        )
        .await??;
        let (first, stream) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
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
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut managed = ManagedStream::new(stream, cancel.clone());
        cancel.cancel();
        assert_eq!(
            managed.read(&mut [0; 1]).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        drop(managed);
        server.await?;
        Ok(())
    }
}
