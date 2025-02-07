use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::ring::default_provider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::VerifierBuilderError;
use rustls::{ClientConfig, Error as RustlsError, RootCertStore, SignatureScheme};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use webpki_roots::TLS_SERVER_ROOTS;
use x509_parser::parse_x509_certificate;

use crate::ssl::calculate_base85_fingerprint;

#[derive(Error, Debug)]
pub enum CertValidatorError {
    #[error("The specified path is not a directory")]
    NotADirectory,

    #[error("Invalid path: cannot convert to string")]
    InvalidPath,

    #[error("Failed to create directory: {0}")]
    DirectoryCreation(#[from] std::io::Error),

    #[error("Failed to serialize/deserialize report: {0}")]
    Serialization(#[from] bincode::Error),

    #[error("Failed to parse certificate: {0}")]
    CertificateParsing(String),

    #[error("Invalid server name format")]
    InvalidServerName,

    #[error("Certificate fingerprint mismatch")]
    FingerprintMismatch,

    #[error("Unknown server and not in learn mode")]
    UnknownServer,

    #[error("TLS error: {0}")]
    TlsError(#[from] RustlsError),
}

#[derive(Debug, Clone)]
pub struct CertValidator {
    inner_verifier: Arc<WebPkiServerVerifier>,
    report_path: PathBuf,
    fingerprints: Arc<Mutex<HashMap<String, String>>>,
    learn_mode: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct FingerprintReport {
    entries: HashMap<String, String>,
}

impl CertValidator {
    pub fn new<P: AsRef<Path>>(path: P, learn_mode: bool) -> Result<Self, CertValidatorError> {
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

        let report_path = report_path.to_path_buf();
        let fingerprints = if report_path.exists() {
            let data =
                std::fs::read(&report_path).map_err(CertValidatorError::DirectoryCreation)?;
            let report: FingerprintReport = bincode::deserialize(&data)?;
            report.entries
        } else {
            HashMap::new()
        };

        Ok(Self {
            inner_verifier,
            report_path,
            fingerprints: Arc::new(Mutex::new(fingerprints)),
            learn_mode,
        })
    }

    fn save_report(&self) -> Result<(), CertValidatorError> {
        let fingerprints = self.fingerprints.lock().unwrap().clone();
        let report = FingerprintReport {
            entries: fingerprints,
        };
        let data = bincode::serialize(&report)?;
        std::fs::write(&self.report_path, data).map_err(CertValidatorError::DirectoryCreation)?;
        Ok(())
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

        let mut fingerprints = self.fingerprints.lock().unwrap();
        match fingerprints.get(&server_name_str) {
            Some(existing) if existing != &fingerprint => Err(RustlsError::General(
                "Certificate fingerprint mismatch".into(),
            )),
            None if self.learn_mode => {
                fingerprints.insert(server_name_str, fingerprint);
                self.save_report()
                    .map_err(|e| RustlsError::General(e.to_string()))?;
                Ok(ServerCertVerified::assertion())
            }
            None => Err(RustlsError::General(
                "Unknown server and not in learn mode".into(),
            )),
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
