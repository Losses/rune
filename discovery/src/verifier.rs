use std::{
    collections::HashMap,
    io::Error as IoError,
    path::Path,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

use anyhow::{anyhow, bail, Result};
use http_body_util::Empty;
use hyper::{body::Bytes, http::Error as HttpError, Uri};
use hyper_util::rt::TokioIo;
use pem::Pem;
use rustls::{
    client::{
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        ClientConfig, WebPkiServerVerifier,
    },
    crypto::ring::default_provider,
    pki_types::{CertificateDer, ServerName, UnixTime},
    server::VerifierBuilderError,
    Error as RustlsError, RootCertStore, SignatureScheme,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{net::TcpStream, sync::broadcast};
use tokio_rustls::TlsConnector;
use webpki_roots::TLS_SERVER_ROOTS;
use x509_parser::parse_x509_certificate;

use crate::persistent::{PersistenceError, PersistentDataManager};
use crate::{ssl::calculate_base85_fingerprint, utils::server_name_to_string};

/// Represents certificate information as an optional tuple of (certificate, fingerprint)
type CertInfo = Option<(String, String)>;

#[derive(Error, Debug)]
pub enum CertValidatorError {
    #[error("Persistence error: {0}")]
    Persistence(#[from] PersistenceError),
    #[error("Certificate parsing error: {0}")]
    CertificateParsing(String),
    #[error("Invalid server name format")]
    InvalidServerName,
    #[error("Certificate fingerprint mismatch")]
    FingerprintMismatch,
    #[error("Unknown server")]
    UnknownServer,
    #[error("TLS error: {0}")]
    TlsError(#[from] RustlsError),
    #[error("Unable to initialize the crypto provider")]
    CryptoProviderInitialize,
    #[error("IO error: {0}")]
    Io(#[from] IoError),
    #[error("HTTP error: {0}")]
    Http(#[from] HttpError),
}

/// A certificate validator that implements custom certificate verification logic
/// with support for fingerprint-based trust and caching
#[derive(Debug, Clone)]
pub struct CertValidator {
    /// The underlying WebPKI verifier for standard certificate validation
    inner_verifier: Arc<WebPkiServerVerifier>,
    /// Persistent storage for fingerprint-to-host mappings
    storage: Arc<PersistentDataManager<FingerprintReport>>,
    /// In-memory cache of fingerprint-to-host mappings for faster lookups
    cached_entries: Arc<RwLock<HashMap<String, Vec<String>>>>,
}

/// Structure to store fingerprint-to-host mappings in persistent storage
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FingerprintReport {
    /// Maps fingerprints to lists of trusted host names
    pub entries: HashMap<String, Vec<String>>, // fingerprint -> list of hosts
}

impl CertValidator {
    pub async fn new<P: AsRef<Path>>(path: P) -> Result<Self, CertValidatorError> {
        let storage_path = path.as_ref().join(".known-servers");
        let storage: Arc<PersistentDataManager<FingerprintReport>> =
            Arc::new(PersistentDataManager::new(storage_path)?);

        // Initialize root certificate store with system roots
        let mut root_store = RootCertStore::empty();
        root_store.extend(TLS_SERVER_ROOTS.iter().cloned());

        let inner_verifier = WebPkiServerVerifier::builder_with_provider(
            Arc::new(root_store),
            Arc::new(default_provider()),
        )
        .build()
        .map_err(|e: VerifierBuilderError| {
            CertValidatorError::TlsError(RustlsError::General(e.to_string()))
        })?;

        // Initialize cache with initial data from storage
        let initial_entries = storage.read().await.entries.clone();
        let cached_entries = Arc::new(RwLock::new(initial_entries));

        // Set up background task to monitor storage updates
        let storage_clone = storage.clone();
        let cached_clone = cached_entries.clone();
        tokio::spawn(async move {
            let mut subscriber = storage_clone.subscribe();
            let mut debouncer = tokio::time::interval(Duration::from_millis(100));
            let mut pending_update = false;

            loop {
                tokio::select! {
                    _ = subscriber.recv() => {
                        pending_update = true;
                    }
                    _ = debouncer.tick() => {
                        if pending_update {
                            let data = storage_clone.read().await;
                            // Fix: Use write() to get a mutable reference and directly assign
                            if let Ok(mut cache) = cached_clone.write() {
                                cache.clone_from(&data.entries);
                            }
                            pending_update = false;
                        }
                    }
                }
            }
        });

        Ok(Self {
            inner_verifier,
            storage,
            cached_entries,
        })
    }

    pub async fn add_trusted_domains<I, D, F>(
        &self,
        domains: I,
        fingerprint: F,
    ) -> Result<(), CertValidatorError>
    where
        I: IntoIterator<Item = D>,
        D: AsRef<str>,
        F: AsRef<str>,
    {
        let fingerprint = fingerprint.as_ref().to_string();
        let domains: Vec<String> = domains
            .into_iter()
            .map(|d| d.as_ref().to_string())
            .collect();

        self.storage
            .update(|mut report| async move {
                for domain in &domains {
                    report
                        .entries
                        .entry(fingerprint.clone())
                        .or_default()
                        .push(domain.clone());
                }

                if let Some(hosts) = report.entries.get_mut(&fingerprint) {
                    hosts.sort();
                    hosts.dedup();
                }

                Ok((report, ()))
            })
            .await
    }

    pub async fn replace_hosts_for_fingerprint(
        &self,
        fingerprint: &str,
        new_hosts: Vec<String>,
    ) -> Result<(), CertValidatorError> {
        self.storage
            .update(|mut report| async move {
                report.entries.insert(fingerprint.to_string(), new_hosts);
                Ok((report, ()))
            })
            .await
    }

    pub async fn remove_fingerprint(&self, fingerprint: &str) -> Result<(), CertValidatorError> {
        self.storage
            .update(|mut report| async move {
                report.entries.remove(fingerprint);
                Ok((report, ()))
            })
            .await
    }

    pub async fn get_hosts_for_fingerprint(&self, fingerprint: &str) -> Vec<String> {
        self.storage
            .read()
            .await
            .entries
            .get(fingerprint)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn list_trusted_fingerprints(&self) -> Vec<String> {
        self.storage.read().await.entries.keys().cloned().collect()
    }

    pub async fn find_fingerprints_by_host(&self, host: &str) -> Vec<String> {
        self.storage
            .read()
            .await
            .entries
            .iter()
            .filter(|(_, hosts)| hosts.contains(&host.to_string()))
            .map(|(fp, _)| fp.clone())
            .collect()
    }

    pub async fn verify_certificate(
        &self,
        cert: &CertificateDer<'_>,
        server_name: &str,
    ) -> Result<(), CertValidatorError> {
        let (_, raw_cert) = parse_x509_certificate(cert.as_ref())
            .map_err(|e| CertValidatorError::CertificateParsing(e.to_string()))?;

        let fingerprint = calculate_base85_fingerprint(raw_cert.public_key().raw)
            .map_err(|e| CertValidatorError::CertificateParsing(e.to_string()))?;

        let valid = self
            .storage
            .read()
            .await
            .entries
            .get(&fingerprint)
            .map(|hosts| hosts.contains(&server_name.to_string()))
            .unwrap_or(false);

        if valid {
            Ok(())
        } else {
            Err(CertValidatorError::FingerprintMismatch)
        }
    }

    pub fn into_client_config(self) -> ClientConfig {
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(self))
            .with_no_client_auth()
    }

    pub fn fingerprints(&self) -> Result<HashMap<String, Vec<String>>> {
        self.cached_entries
            .read()
            .map_err(|e| anyhow!("Unable to read fingerprints: {}", e))
            .map(|guard| guard.clone())
    }

    pub fn subscribe_changes(&self) -> broadcast::Receiver<FingerprintReport> {
        self.storage.subscribe()
    }
}

impl ServerCertVerifier for CertValidator {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        let server_name_str = server_name_to_string(server_name)?;

        let (_, cert) = parse_x509_certificate(end_entity.as_ref())
            .map_err(|e| RustlsError::General(e.to_string()))?;
        let public_key_der = cert.public_key().raw;
        let fingerprint = calculate_base85_fingerprint(public_key_der)
            .map_err(|e| RustlsError::General(e.to_string()))?;

        let entries = self
            .cached_entries
            .read()
            .map_err(|_| RustlsError::General("Failed to acquire the lock".into()))?;
        let is_trusted = entries
            .get(&fingerprint)
            .map(|hosts| hosts.contains(&server_name_str))
            .unwrap_or(false);

        if is_trusted {
            Ok(ServerCertVerified::assertion())
        } else {
            self.inner_verifier.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            )
        }
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner_verifier.supported_verify_schemes()
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.inner_verifier
            .verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.inner_verifier
            .verify_tls13_signature(message, cert, dss)
    }
}

pub struct ServerCert {
    pub cert: String,
    pub fingerprint: String,
}

pub async fn fetch_server_certificate(url: &str) -> Result<ServerCert, CertValidatorError> {
    let uri = url
        .parse::<hyper::Uri>()
        .map_err(|_| CertValidatorError::InvalidServerName)?;
    let host = uri
        .host()
        .ok_or(CertValidatorError::InvalidServerName)?
        .to_string();
    let port = uri.port_u16().unwrap_or(7863);

    let cert_info = Arc::new(Mutex::new(None));
    let verifier = TempCertVerifier {
        cert_info: Arc::clone(&cert_info),
    };

    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();

    let connector = TlsConnector::from(Arc::new(config));

    let tcp_stream = tokio::net::TcpStream::connect((host.clone(), port)).await?;

    let server_name =
        ServerName::try_from(host).map_err(|_| CertValidatorError::InvalidServerName)?;

    let tls_stream = connector
        .connect(server_name, tcp_stream)
        .await
        .map_err(|e| CertValidatorError::TlsError(RustlsError::General(e.to_string())))?;

    let io = TokioIo::new(tls_stream);

    let (mut sender, connection) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|e| CertValidatorError::TlsError(RustlsError::General(e.to_string())))?;

    tokio::spawn(async move {
        if let Err(err) = connection.await {
            eprintln!("Connection error: {}", err);
        }
    });

    let request = hyper::Request::builder()
        .uri(uri)
        .method("GET")
        .body(Empty::<Bytes>::new())?;

    let _ = sender.send_request(request).await;

    let guard = cert_info.lock().unwrap();
    guard
        .clone()
        .map(|(cert, fingerprint)| ServerCert { cert, fingerprint })
        .ok_or(CertValidatorError::CertificateParsing(
            "No certificate captured".into(),
        ))
}

pub fn parse_certificate(x: &str) -> Result<(String, String)> {
    let pem = pem::parse(x).map_err(|e| RustlsError::General(e.to_string()))?;
    if pem.tag() != "CERTIFICATE" {
        bail!(format!("Expected CERTIFICATE PEM, got {}", pem.tag()))
    }
    let der = pem.contents();

    let (_, raw_cert) =
        parse_x509_certificate(der).map_err(|e| RustlsError::General(e.to_string()))?;

    let cert_pem = Pem::new("CERTIFICATE".to_string(), der.to_vec());
    let cert = pem::encode(&cert_pem);

    let fingerprint = calculate_base85_fingerprint(raw_cert.public_key().raw)
        .map_err(|e| RustlsError::General(e.to_string()))?;

    Ok((cert, fingerprint))
}

#[derive(Debug)]
struct TempCertVerifier {
    cert_info: Arc<Mutex<CertInfo>>,
}

impl ServerCertVerifier for TempCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        let (_, raw_cert) = parse_x509_certificate(end_entity.as_ref())
            .map_err(|e| RustlsError::General(e.to_string()))?;

        let cert = Pem::new("CERTIFICATE".to_string(), raw_cert.public_key().raw);
        let cert = pem::encode(&cert);

        let fingerprint = calculate_base85_fingerprint(raw_cert.public_key().raw)
            .map_err(|e| RustlsError::General(e.to_string()))?;

        *self.cert_info.lock().unwrap() = Some((cert, fingerprint));
        Ok(ServerCertVerified::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
        ]
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }
}

pub async fn trust_server(
    validator: &CertValidator,
    domains: Vec<String>,
    ips: Vec<String>,
    fingerprint: &str,
) -> Result<(), CertValidatorError> {
    let mut all_entries = Vec::with_capacity(domains.len() + ips.len());
    all_entries.extend(domains);
    all_entries.extend(ips);

    validator
        .add_trusted_domains(all_entries, fingerprint)
        .await
}

pub async fn try_connect(host: &str, config: ClientConfig) -> Result<String> {
    let uri = host.parse::<Uri>()?;
    let host_str = uri.host().unwrap().to_string();
    let sni = ServerName::try_from(host_str.clone())?;

    let connector = TlsConnector::from(Arc::new(config));
    let tcp = TcpStream::connect((host_str.as_str(), uri.port_u16().unwrap_or(7863))).await?;
    let _ = connector.connect(sni, tcp).await?;

    Ok(host.to_string())
}
