use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use http_body_util::Empty;
use hyper::body::Bytes;
use hyper::Uri;
use hyper_util::rt::TokioIo;
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
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use toml;
use webpki_roots::TLS_SERVER_ROOTS;
use x509_parser::parse_x509_certificate;

use crate::ssl::calculate_base85_fingerprint;

type CertInfo = Option<(Vec<u8>, String)>;

#[derive(Error, Debug)]
pub enum CertValidatorError {
    #[error("The specified path is not a directory")]
    NotADirectory,

    #[error("Invalid path: cannot convert to string")]
    InvalidPath,

    #[error("Failed to create directory: {0}")]
    DirectoryCreation(#[from] std::io::Error),

    #[error("Failed to serialize/deserialize report: {0}")]
    Serialization(String),

    #[error("Failed to parse certificate: {0}")]
    CertificateParsing(String),

    #[error("Invalid server name format")]
    InvalidServerName,

    #[error("Certificate fingerprint mismatch")]
    FingerprintMismatch,

    #[error("Unknown server")]
    UnknownServer,

    #[error("TLS error: {0}")]
    TlsError(#[from] RustlsError),
}

#[derive(Debug, Clone)]
pub struct CertValidator {
    inner_verifier: Arc<WebPkiServerVerifier>,
    report_path: PathBuf,
    host_to_fingerprint: Arc<Mutex<HashMap<String, String>>>, // host -> fingerprint
    fingerprint_to_hosts: Arc<Mutex<HashMap<String, Vec<String>>>>, // fingerprint -> hosts
}

#[derive(Debug, Serialize, Deserialize)]
struct FingerprintReport {
    entries: HashMap<String, Vec<String>>, // fingerprint -> list of hosts
}

impl CertValidator {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, CertValidatorError> {
        let path = path.as_ref();

        if !path.exists() {
            fs::create_dir_all(path).map_err(CertValidatorError::DirectoryCreation)?;
        } else if !path.is_dir() {
            return Err(CertValidatorError::NotADirectory);
        }

        let report_path = path.join(".known-clients");

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

        let (host_to_fingerprint, fingerprint_to_hosts) = if report_path.exists() {
            let data = std::fs::read_to_string(&report_path)?;
            let report: FingerprintReport = toml::from_str(&data)
                .map_err(|e| CertValidatorError::Serialization(e.to_string()))?;
            let mut host_map = HashMap::new();
            let mut fp_map = HashMap::new();
            for (fp, hosts) in report.entries {
                for host in &hosts {
                    host_map.insert(host.clone(), fp.clone());
                }
                fp_map.insert(fp, hosts);
            }
            (host_map, fp_map)
        } else {
            (HashMap::new(), HashMap::new())
        };

        Ok(Self {
            inner_verifier,
            report_path,
            host_to_fingerprint: Arc::new(Mutex::new(host_to_fingerprint)),
            fingerprint_to_hosts: Arc::new(Mutex::new(fingerprint_to_hosts)),
        })
    }

    fn save_report(&self) -> Result<(), CertValidatorError> {
        let fp_map = self.fingerprint_to_hosts.lock().unwrap().clone();
        let report = FingerprintReport { entries: fp_map };

        let data = toml::to_string(&report)
            .map_err(|e| CertValidatorError::Serialization(e.to_string()))?;
        std::fs::write(&self.report_path, data).map_err(CertValidatorError::DirectoryCreation)?;
        Ok(())
    }

    pub fn replace_hosts_for_fingerprint(
        &self,
        fingerprint: &str,
        new_hosts: Vec<String>,
    ) -> Result<(), CertValidatorError> {
        let mut host_map = self.host_to_fingerprint.lock().unwrap();
        let mut fp_map = self.fingerprint_to_hosts.lock().unwrap();

        if let Some(old_hosts) = fp_map.remove(fingerprint) {
            for host in old_hosts {
                host_map.remove(&host);
            }
        }

        let mut dedup_hosts = Vec::new();
        for host in new_hosts {
            if !host_map.contains_key(&host) {
                host_map.insert(host.clone(), fingerprint.to_string());
                dedup_hosts.push(host);
            }
        }

        fp_map.insert(fingerprint.to_string(), dedup_hosts);
        self.save_report()
    }

    pub fn remove_fingerprint(&self, fingerprint: &str) -> Result<(), CertValidatorError> {
        let mut host_map = self.host_to_fingerprint.lock().unwrap();
        let mut fp_map = self.fingerprint_to_hosts.lock().unwrap();

        if let Some(hosts) = fp_map.remove(fingerprint) {
            for host in hosts {
                host_map.remove(&host);
            }
        }
        self.save_report()
    }

    pub fn get_hosts_for_fingerprint(&self, fingerprint: &str) -> Vec<String> {
        self.fingerprint_to_hosts
            .lock()
            .unwrap()
            .get(fingerprint)
            .cloned()
            .unwrap_or_default()
    }

    pub fn add_trusted_domains<I, D, F>(
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
        let mut host_map = self.host_to_fingerprint.lock().unwrap();
        let mut fp_map = self.fingerprint_to_hosts.lock().unwrap();

        for domain in domains.into_iter() {
            let domain = domain.as_ref().to_string();

            if let Some(old_fp) = host_map.remove(&domain) {
                if let Some(hosts) = fp_map.get_mut(&old_fp) {
                    hosts.retain(|h| h != &domain);
                    if hosts.is_empty() {
                        fp_map.remove(&old_fp);
                    }
                }
            }

            host_map.insert(domain.clone(), fingerprint.clone());

            fp_map.entry(fingerprint.clone()).or_default().push(domain);
        }

        self.save_report()
    }

    pub fn into_client_config(self) -> ClientConfig {
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(self))
            .with_no_client_auth()
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
        self.inner_verifier.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        )?;

        let (_, cert) = parse_x509_certificate(end_entity.as_ref())
            .map_err(|e| RustlsError::General(e.to_string()))?;
        let public_key_der = cert.public_key().raw;

        let fingerprint = calculate_base85_fingerprint(public_key_der)
            .map_err(|e| RustlsError::General(e.to_string()))?;

        let server_name_str = match server_name {
            ServerName::DnsName(dns) => dns.as_ref().to_string(),
            _ => return Err(RustlsError::General("Invalid server name".into())),
        };

        let host_to_fp = self.host_to_fingerprint.lock().unwrap();
        match host_to_fp.get(&server_name_str) {
            Some(existing) if existing != &fingerprint => Err(RustlsError::General(
                "Certificate fingerprint mismatch".into(),
            )),
            None => Err(RustlsError::General("Unknown server".into())),
            Some(_) => Ok(ServerCertVerified::assertion()),
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
    pub cert: Vec<u8>,
    pub fingerprint: String,
}

pub async fn fetch_server_certificate(url: &str) -> Result<ServerCert, CertValidatorError> {
    let uri = url
        .parse::<hyper::Uri>()
        .map_err(|e| CertValidatorError::Serialization(e.to_string()))?;
    let host = uri
        .host()
        .ok_or(CertValidatorError::InvalidServerName)?
        .to_string();
    let port = uri.port_u16().unwrap_or(443);

    let cert_info = Arc::new(Mutex::new(None));
    let verifier = TempCertVerifier {
        cert_info: Arc::clone(&cert_info),
    };

    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();

    let connector = TlsConnector::from(Arc::new(config));

    let tcp_stream = tokio::net::TcpStream::connect((host.clone(), port))
        .await
        .map_err(CertValidatorError::DirectoryCreation)?;

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
        .body(Empty::<Bytes>::new())
        .map_err(|e| CertValidatorError::Serialization(e.to_string()))?;

    let _ = sender.send_request(request).await;

    let guard = cert_info.lock().unwrap();
    guard
        .clone()
        .map(|(public_key, fingerprint)| ServerCert {
            cert: public_key,
            fingerprint,
        })
        .ok_or(CertValidatorError::CertificateParsing(
            "No certificate captured".into(),
        ))
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
        let (_, cert) = parse_x509_certificate(end_entity.as_ref())
            .map_err(|e| RustlsError::General(e.to_string()))?;

        let public_key = cert.public_key().raw.to_vec();
        let fingerprint = calculate_base85_fingerprint(&public_key)
            .map_err(|e| RustlsError::General(e.to_string()))?;

        *self.cert_info.lock().unwrap() = Some((public_key, fingerprint));
        Ok(ServerCertVerified::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::RSA_PKCS1_SHA256]
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

pub fn trust_server(
    validator: &CertValidator,
    domains: Vec<String>,
    ips: Vec<String>,
    fingerprint: &str,
) -> Result<(), CertValidatorError> {
    let mut all_entries = Vec::with_capacity(domains.len() + ips.len());
    all_entries.extend(domains);
    all_entries.extend(ips);

    validator.add_trusted_domains(all_entries, fingerprint)
}

pub async fn try_connect(host: &str, config: ClientConfig) -> Result<String> {
    let uri = host.parse::<Uri>()?;
    let host_str = uri.host().unwrap().to_string();
    let sni = ServerName::try_from(host_str.clone())?;

    let connector = TlsConnector::from(Arc::new(config));
    let tcp = TcpStream::connect((host_str.as_str(), uri.port_u16().unwrap_or(443))).await?;
    let _ = connector.connect(sni, tcp).await?;

    Ok(host.to_string())
}
