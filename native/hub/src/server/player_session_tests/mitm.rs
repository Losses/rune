//! A TLS-terminating test proxy for the handoff rollback tests. It presents the target server's
//! own certificate to clients, so the pinned-certificate trust path is exactly the production
//! one, and it relays every byte unchanged unless armed. Arming makes the first take-back
//! request frame the fault injection point, chosen by content rather than by timing: the cut
//! either swallows the request before it reaches the server, or forwards it whole and then
//! holds the response direction, so the apply can complete server-side while its answer never
//! passes to the source.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{Context, Result, anyhow};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, ServerConfig};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio_rustls::{TlsAcceptor, TlsConnector};

/// The largest websocket frame the scanner will buffer. Real request frames on these
/// connections are small; anything bigger is relayed without scanning so a non-websocket
/// connection through the same proxy is never held up by a random payload prefix.
const MAX_SCANNED_FRAME: usize = 64 * 1024;

/// How the armed cut treats the first take-back request it sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CutKind {
    /// The request never reaches the server: the target cannot have applied the rollback.
    BeforeApply,
    /// The request reaches the server whole, but the response direction is held from that
    /// moment on: the target applies the rollback and the source never learns it.
    AfterApply,
}

struct ConnCtl {
    kill: AtomicBool,
    killed: Notify,
    hold_responses: AtomicBool,
}

struct MitmState {
    armed: StdMutex<Option<CutKind>>,
    fired_conn: StdMutex<Option<Arc<ConnCtl>>>,
    fired: Notify,
}

/// A content-aware TLS MITM in front of one Rune server. Tests arm it, wait for the cut to
/// fire, prove server-side state, and then kill the cut connection.
pub(super) struct MitmCut {
    pub(super) listen_addr: SocketAddr,
    state: Arc<MitmState>,
    accept: tokio::task::JoinHandle<()>,
}

impl MitmCut {
    /// `cert_pem`/`key_pem` are the upstream server's own identity: clients see exactly the
    /// certificate they pinned, so the production trust path is unchanged.
    pub(super) async fn new(
        target_addr: SocketAddr,
        cert_pem: &str,
        key_pem: &str,
    ) -> Result<Self> {
        let certs: Vec<CertificateDer<'static>> = pem::parse_many(cert_pem.as_bytes())
            .context("parse server certificate pem")?
            .into_iter()
            .filter(|block| block.tag() == "CERTIFICATE")
            .map(|block| CertificateDer::from(block.into_contents()))
            .collect();
        if certs.is_empty() {
            return Err(anyhow!("server certificate pem has no CERTIFICATE block"));
        }
        let key_block = pem::parse(key_pem.as_bytes()).context("parse server key pem")?;
        let key = match key_block.tag() {
            "PRIVATE KEY" => PrivateKeyDer::Pkcs8(key_block.into_contents().into()),
            "RSA PRIVATE KEY" => PrivateKeyDer::Pkcs1(key_block.into_contents().into()),
            "EC PRIVATE KEY" => PrivateKeyDer::Sec1(key_block.into_contents().into()),
            tag => return Err(anyhow!("unsupported private key pem tag {tag}")),
        };
        let mut server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .context("build mitm server config")?;
        // The scanning relay speaks HTTP/1.1 only: offering nothing else keeps clients on it.
        server_config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let client_config = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCertificate))
            .with_no_client_auth();
        let acceptor = TlsAcceptor::from(Arc::new(server_config));
        let connector = TlsConnector::from(Arc::new(client_config));

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let listen_addr = listener.local_addr()?;
        let state = Arc::new(MitmState {
            armed: StdMutex::new(None),
            fired_conn: StdMutex::new(None),
            fired: Notify::new(),
        });
        let accept = tokio::spawn({
            let state = state.clone();
            async move {
                while let Ok((client, _)) = listener.accept().await {
                    let _ = client.set_nodelay(true);
                    let acceptor = acceptor.clone();
                    let connector = connector.clone();
                    let state = state.clone();
                    tokio::spawn(async move {
                        let _ =
                            relay_connection(client, target_addr, acceptor, connector, state).await;
                    });
                }
            }
        });
        Ok(Self {
            listen_addr,
            state,
            accept,
        })
    }

    /// The next take-back request frame through any connection is the fault point.
    pub(super) fn arm(&self, kind: CutKind) {
        *self.state.armed.lock().expect("mitm armed lock") = Some(kind);
    }

    /// Resolves once the armed cut has fired. For [`CutKind::BeforeApply`] this means the
    /// request was swallowed before reaching the server; for [`CutKind::AfterApply`] it means
    /// the whole request was forwarded and the response direction is already held.
    pub(super) async fn wait_fired(&self) {
        self.state.fired.notified().await;
    }

    /// Ends the connection the cut fired on, in both directions.
    pub(super) fn kill_fired_connection(&self) {
        let ctl = self
            .state
            .fired_conn
            .lock()
            .expect("mitm fired_conn lock")
            .clone();
        if let Some(ctl) = ctl {
            ctl.kill.store(true, Ordering::SeqCst);
            ctl.killed.notify_one();
        }
    }
}

impl Drop for MitmCut {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

/// Consumes the one-shot armed cut. `Some(kind)` exactly once per arming.
fn try_fire(state: &MitmState) -> Option<CutKind> {
    state.armed.lock().expect("mitm armed lock").take()
}

fn note_fired(state: &MitmState, ctl: &Arc<ConnCtl>) {
    *state.fired_conn.lock().expect("mitm fired_conn lock") = Some(ctl.clone());
    state.fired.notify_one();
}

async fn relay_connection(
    client: TcpStream,
    target_addr: SocketAddr,
    acceptor: TlsAcceptor,
    connector: TlsConnector,
    state: Arc<MitmState>,
) -> Result<()> {
    let server_tcp = TcpStream::connect(target_addr).await?;
    let _ = server_tcp.set_nodelay(true);
    let downstream = acceptor.accept(client);
    let upstream = connector.connect(
        ServerName::try_from("localhost").expect("static server name"),
        server_tcp,
    );
    let (downstream, upstream) = tokio::join!(downstream, upstream);
    if downstream.is_err() {
    }
    if upstream.is_err() {
    }
    let downstream = downstream.context("mitm downstream tls")?;
    let upstream = upstream.context("mitm upstream tls")?;

    let ctl = Arc::new(ConnCtl {
        kill: AtomicBool::new(false),
        killed: Notify::new(),
        hold_responses: AtomicBool::new(false),
    });

    let (client_reader, client_writer) = tokio::io::split(downstream);
    let (server_reader, server_writer) = tokio::io::split(upstream);
    let requests = pump_requests(client_reader, server_writer, ctl.clone(), state);
    let responses = pump_responses(server_reader, client_writer, ctl.clone());
    tokio::select! {
        _ = requests => {},
        _ = responses => {},
    }
    ctl.kill.store(true, Ordering::SeqCst);
    ctl.killed.notify_one();
    Ok(())
}

/// Client to server: relay everything, scanning websocket binary message starts for the
/// take-back request.
async fn pump_requests<R, W>(mut reader: R, mut writer: W, ctl: Arc<ConnCtl>, state: Arc<MitmState>)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let needle = take_back_needle();
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 16384];
    // Plain HTTP exchanges and the websocket share the server's port. Until a request line
    // asks for the upgrade the connection is keep-alive HTTP: requests come one after another,
    // each its headers then exactly content-length bytes of body, and the relay forwards both
    // untouched. The upgrade request itself has no body; after its headers the connection is
    // websocket and every byte belongs to a frame.
    let mut headers_done = false;
    let mut http_body_remaining = 0usize;
    let mut websocket = false;
    let mut scanning = true;
    let mut message = MessageScan::default();
    loop {
        if ctl.kill.load(Ordering::SeqCst) {
            return;
        }
        let read = match reader.read(&mut chunk).await {
            Ok(0) => return,
            Err(_) => return,
            Ok(read) => read,
        };
        buf.extend_from_slice(&chunk[..read]);
        loop {
            if !headers_done {
                match find_subslice(&buf, b"\r\n\r\n") {
                    Some(pos) => {
                        let end = pos + 4;
                        if writer.write_all(&buf[..end]).await.is_err() {
                            return;
                        }
                        if writer.flush().await.is_err() {
                            return;
                        }
                        let headers = String::from_utf8_lossy(&buf[..end]).into_owned();
                        let upgraded = headers.lines().any(|line| {
                            let lower = line.to_ascii_lowercase();
                            lower.starts_with("upgrade:") && lower.contains("websocket")
                        });
                        if upgraded {
                            websocket = true;
                        } else {
                            http_body_remaining = headers
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    if name.trim().eq_ignore_ascii_case("content-length") {
                                        value.trim().parse::<usize>().ok()
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or(0);
                        }
                        buf.drain(..end);
                        headers_done = true;
                    }
                    None if buf.len() > MAX_SCANNED_FRAME => {
                        // Not a header-terminated connection after all: relay it raw.
                        scanning = false;
                        if writer.write_all(&buf).await.is_err() {
                            return;
                        }
                        buf.clear();
                        break;
                    }
                    None => break,
                }
                continue;
            }
            if !scanning {
                if writer.write_all(&buf).await.is_err() {
                    return;
                }
                let _ = writer.flush().await;
                buf.clear();
                break;
            }
            if !websocket {
                if http_body_remaining == 0 {
                    // The next request's headers start here.
                    headers_done = false;
                    continue;
                }
                let relay = buf.len().min(http_body_remaining);
                if writer.write_all(&buf[..relay]).await.is_err() {
                    return;
                }
                let _ = writer.flush().await;
                buf.drain(..relay);
                http_body_remaining -= relay;
                if http_body_remaining == 0 {
                    headers_done = false;
                }
                if buf.is_empty() {
                    break;
                }
                continue;
            }
            let Some(frame) = WsFrame::parse(&buf) else {
                break;
            };
            if frame.total_len > MAX_SCANNED_FRAME {
                scanning = false;
                continue;
            }
            let bytes: Vec<u8> = buf[..frame.total_len].to_vec();
            buf.drain(..frame.total_len);
            if message.completes_take_back_start(&frame, &bytes, &needle) {
                match try_fire(&state) {
                    Some(CutKind::BeforeApply) => {
                        // The frame carrying the request is never forwarded: the server cannot
                        // have applied it, and the connection dies around the hole.
                        ctl.kill.store(true, Ordering::SeqCst);
                        ctl.killed.notify_one();
                        note_fired(&state, &ctl);
                        return;
                    }
                    Some(CutKind::AfterApply) => {
                        // Hold the response direction first: any byte the response pump reads
                        // after this store is checked against it before forwarding, and the
                        // take-back's own answer cannot exist before the request passes.
                        ctl.hold_responses.store(true, Ordering::SeqCst);
                        if writer.write_all(&bytes).await.is_err() {
                            return;
                        }
                        note_fired(&state, &ctl);
                    }
                    None => {
                        if writer.write_all(&bytes).await.is_err() {
                            return;
                        }
                    }
                }
                continue;
            }
            if writer.write_all(&bytes).await.is_err() {
                return;
            }
            let _ = writer.flush().await;
        }
    }
}

/// Server to client: relay everything until the response direction is held, then stop reading.
/// Bytes already in flight stay in the kernel buffers and never pass, and the connection is
/// torn down before the hold can matter to anything else.
async fn pump_responses<R, W>(mut reader: R, mut writer: W, ctl: Arc<ConnCtl>)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut chunk = [0u8; 16384];
    loop {
        if ctl.kill.load(Ordering::SeqCst) {
            return;
        }
        if ctl.hold_responses.load(Ordering::SeqCst) {
            ctl.killed.notified().await;
            return;
        }
        let read = tokio::select! {
            _ = ctl.killed.notified() => return,
            read = reader.read(&mut chunk) => match read {
                Ok(0) => return,
                Err(_) => return,
                Ok(read) => read,
            },
        };
        if ctl.hold_responses.load(Ordering::SeqCst) {
            ctl.killed.notified().await;
            return;
        }
        if writer.write_all(&chunk[..read]).await.is_err() {
            return;
        }
        let _ = writer.flush().await;
    }
}

fn take_back_needle() -> Vec<u8> {
    let name = b"TakeBackHandoffBlockRequest";
    let mut needle = vec![name.len() as u8];
    needle.extend_from_slice(name);
    needle
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

struct WsFrame {
    total_len: usize,
    fin: bool,
    opcode: u8,
    payload_offset: usize,
    payload_len: usize,
    mask: Option<[u8; 4]>,
}

impl WsFrame {
    fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < 2 {
            return None;
        }
        let fin = buf[0] & 0x80 != 0;
        let opcode = buf[0] & 0x0f;
        let masked = buf[1] & 0x80 != 0;
        let short_len = (buf[1] & 0x7f) as usize;
        let (payload_len, mut offset) = match short_len {
            len @ 0..=125 => (len, 2),
            126 => {
                if buf.len() < 4 {
                    return None;
                }
                (u16::from_be_bytes([buf[2], buf[3]]) as usize, 4)
            }
            _ => {
                if buf.len() < 10 {
                    return None;
                }
                (u64::from_be_bytes(buf[2..10].try_into().ok()?) as usize, 10)
            }
        };
        let mask = if masked {
            if buf.len() < offset + 4 {
                return None;
            }
            let key = [buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]];
            offset += 4;
            Some(key)
        } else {
            None
        };
        let total_len = offset.checked_add(payload_len)?;
        if buf.len() < total_len {
            return None;
        }
        Some(Self {
            total_len,
            fin,
            opcode,
            payload_offset: offset,
            payload_len,
            mask,
        })
    }

    fn unmasked_payload(&self, bytes: &[u8]) -> Vec<u8> {
        let payload = &bytes[self.payload_offset..self.payload_offset + self.payload_len];
        match self.mask {
            None => payload.to_vec(),
            Some(key) => payload
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ key[index % 4])
                .collect(),
        }
    }
}

/// Tracks the start of each binary message so the first `needle` bytes of its payload can be
/// matched. Only the message start is ever examined; control frames interleaved inside a
/// fragmented message are ignored.
#[derive(Default)]
struct MessageScan {
    in_message: bool,
    undecided: bool,
    prefix: Vec<u8>,
}

impl MessageScan {
    /// True exactly once, when a binary message start matches the encoded take-back request
    /// name. Fragmented messages are decided from their leading fragments.
    fn completes_take_back_start(&mut self, frame: &WsFrame, bytes: &[u8], needle: &[u8]) -> bool {
        const BINARY: u8 = 2;
        const CONTINUATION: u8 = 0;
        if frame.opcode == BINARY && !self.in_message {
            self.in_message = true;
            self.undecided = true;
            self.prefix.clear();
        }
        if !self.in_message {
            return false;
        }
        if self.undecided && matches!(frame.opcode, BINARY | CONTINUATION) {
            let payload = frame.unmasked_payload(bytes);
            let remaining = needle.len().saturating_sub(self.prefix.len());
            self.prefix
                .extend_from_slice(&payload[..payload.len().min(remaining)]);
            if self.prefix.len() >= needle.len() || frame.fin {
                self.undecided = false;
                if self.prefix.starts_with(needle) {
                    self.in_message = false;
                    return true;
                }
            }
        }
        if frame.fin && matches!(frame.opcode, BINARY | CONTINUATION) {
            self.in_message = false;
        }
        false
    }
}

#[derive(Debug)]
struct AcceptAnyCertificate;

impl ServerCertVerifier for AcceptAnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod mitm_self_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// The relay must pass a plain HTTPS GET end to end: a minimal upstream answers it, and
    /// the client reads the answer through the proxy.
    #[tokio::test]
    async fn relays_a_plain_https_exchange() {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};

        // Self-signed identity for the upstream.
        let key_pair = rcgen::KeyPair::generate().expect("key pair");
        let cert = rcgen::CertificateParams::new(vec!["localhost".to_owned()])
            .expect("cert params")
            .self_signed(&key_pair)
            .expect("self signed");
        let cert_pem = cert.pem();
        let key_pem = key_pair.serialize_pem();

        // Upstream: accept one TLS connection, read the request, answer a fixed response.
        let certs = vec![CertificateDer::from(cert.der().to_vec())];
        let key = PrivateKeyDer::Pkcs8(key_pair.serialize_der().into());
        let mut upstream_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .expect("upstream config");
        upstream_config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let upstream_acceptor = TlsAcceptor::from(Arc::new(upstream_config));
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let upstream_addr = upstream_listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let (tcp, _) = upstream_listener.accept().await.expect("accept");
            let mut tls = upstream_acceptor.accept(tcp).await.expect("upstream tls");
            let mut buf = vec![0u8; 4096];
            let read = tls.read(&mut buf).await.expect("upstream read");
            tls.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\nhello")
                .await
                .expect("upstream write");
            let _ = tls.flush().await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        });

        let cut = MitmCut::new(upstream_addr, &cert_pem, &key_pem).await.expect("mitm");

        // Client: plain rustls, trusting nothing, speaking HTTP/1.1.
        let client_config = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCertificate))
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(client_config));
        let tcp = TcpStream::connect(cut.listen_addr).await.expect("connect");
        let mut tls = connector
            .connect(ServerName::try_from("localhost").expect("name"), tcp)
            .await
            .expect("client tls");
        tls.write_all(b"GET /sync/challenge HTTP/1.1\r\nhost: x\r\n\r\n")
            .await
            .expect("client write");
        let _ = tls.flush().await;
        let mut response = Vec::new();
        let read = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tls.read_to_end(&mut response),
        )
        .await;
        assert!(
            response.windows(5).any(|window| window == b"hello"),
            "the response must pass the proxy: {}",
            String::from_utf8_lossy(&response)
        );
    }
}

