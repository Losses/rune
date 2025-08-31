use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use http_body_util::combinators::UnsyncBoxBody;
use hyper::{Response, body::Incoming};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

pub use http_body_util::{BodyExt, Empty, Full};
pub use hyper::{Method, Request, StatusCode, Uri, body::Bytes};
pub use rustls::ClientConfig;

pub async fn create_https_client(
    host: String,
    port: u16,
    config: Arc<ClientConfig>,
) -> Result<hyper::client::conn::http1::SendRequest<UnsyncBoxBody<Bytes, anyhow::Error>>> {
    let host = host.to_string();
    let tcp_stream = TcpStream::connect((host.clone(), port))
        .await
        .with_context(|| format!("Failed to connect to {host}:{port}"))?;

    let server_name =
        ServerName::try_from(host.clone()).map_err(|_| anyhow!("Invalid server name: {}", host))?;

    let connector = TlsConnector::from(config);
    let tls_stream = connector
        .connect(server_name, tcp_stream)
        .await
        .with_context(|| format!("TLS handshake failed for {host}:{port}"))?;

    let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls_stream))
        .await
        .with_context(|| format!("HTTP handshake failed for {host}:{port}"))?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {e}");
        }
    });

    Ok(sender)
}

pub async fn send_http_request<B>(
    sender: &mut hyper::client::conn::http1::SendRequest<UnsyncBoxBody<Bytes, anyhow::Error>>,
    req: Request<B>,
) -> Result<Response<Incoming>>
where
    B: http_body::Body<Data = Bytes> + Send + 'static,
    B::Error: Into<anyhow::Error>,
{
    let req = req.map(|body| {
        body.map_err(|e| anyhow::anyhow!(e)).boxed_unsync() as UnsyncBoxBody<Bytes, anyhow::Error>
    });

    let res: Response<Incoming> = sender
        .send_request(req)
        .await
        .context("Failed to send request")?;

    Ok(res)
}
