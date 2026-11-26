use hyper::{Uri, body::Bytes, client::conn::http1, http::Error as HttpError};
use std::time::{Duration, Instant};
use std::{
    collections::HashMap,
    io::Error as IoError,
    path::Path,
    sync::{Arc, Mutex, RwLock},
};

use anyhow::{Result, anyhow, bail};
use futures::{StreamExt, stream::FuturesUnordered};
use http_body_util::Empty;
use hyper_util::rt::TokioIo;
use log::warn;
use pem::Pem;
use rustls::{
    Error as RustlsError, RootCertStore, SignatureScheme,
    client::{
        ClientConfig, WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    crypto::ring::default_provider,
    pki_types::{CertificateDer, ServerName, UnixTime},
    server::VerifierBuilderError,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{net::TcpStream, sync::broadcast, time::timeout};
use tokio_rustls::TlsConnector;
use webpki_roots::TLS_SERVER_ROOTS;
use x509_parser::parse_x509_certificate;

use crate::{
    endpoint::{DEFAULT_DEVICE_PORT, DeviceAddress},
    persistent::{PersistenceError, PersistentDataManager},
    ssl::calculate_base85_fingerprint,
    utils::server_name_to_string,
};
use futures::future;
use hyper::Request;
use rustls::crypto;
use std::time as std_time;

/// Type alias for certificate information, represented as an optional tuple.
///
/// `CertInfo` is used to hold certificate data, which is optionally a tuple containing
/// the certificate in string format (PEM encoded) and its base85 fingerprint. It is used
/// in contexts where certificate retrieval might fail or the certificate information is not yet available.
type CertInfo = Option<(String, String)>;

/// A host taken from a URI, without the brackets that quote IPv6 literals. Both URI
/// hosts (`[::1]`) and plain device hosts (`::1`) are accepted, because trusted entries
/// are compared through the same form.
fn bare_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
}

/// The identity a host is trusted and matched by: the canonical form of an IP literal,
/// the host name itself otherwise. Canonicalising IP literals here, the same way
/// `DeviceAddress` does when it parses an address, keeps `::1`, `0:0:0:0:0:0:0:1` and
/// `[::1]` one identity instead of three spellings.
fn canonical_host(host: &str) -> String {
    bare_host(host)
        .parse::<std::net::IpAddr>()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| bare_host(host).to_owned())
}

/// Builds the TLS server name for a host. A bracketed IPv6 literal reaches here quoted
/// (`[::1]`) because that is how URIs carry it, while rustls expects the bare literal
/// and would treat the quoted form as a DNS name.
fn server_name_for_host(host: &str) -> Result<ServerName<'static>, CertValidatorError> {
    ServerName::try_from(bare_host(host).to_owned())
        .map_err(|_| CertValidatorError::InvalidServerName)
}

/// A trusted entry may carry a port (`host:port`); TLS only sees the host name.
fn trusted_host_matches(candidate: &str, host: &str) -> bool {
    candidate
        .parse::<DeviceAddress>()
        .is_ok_and(|address| address.host() == canonical_host(host))
}

/// Defines errors that can occur during certificate validation and management.
///
/// `CertValidatorError` is an enum that encapsulates all possible errors that might arise
/// when validating server certificates, managing trusted fingerprints, or during related operations
/// like fetching certificates or interacting with persistent storage.
#[derive(Error, Debug)]
pub enum CertValidatorError {
    /// Error originating from the persistent data storage layer.
    #[error("Persistence error: {0}")]
    Persistence(#[from] PersistenceError),
    /// Error during certificate parsing, typically when the certificate format is invalid.
    #[error("Certificate parsing error: {0}")]
    CertificateParsing(String),
    /// Error indicating an invalid server name format, often when parsing URLs or hostnames.
    #[error("Invalid server name format")]
    InvalidServerName,
    /// Error when the calculated certificate fingerprint does not match the expected or stored fingerprint.
    #[error("Certificate fingerprint mismatch")]
    FingerprintMismatch,
    /// Error indicating that the server being accessed is unknown or not trusted.
    #[error("Unknown server")]
    UnknownServer,
    /// Error originating from the rustls TLS library, indicating a TLS-related issue.
    #[error("TLS error: {0}")]
    TlsError(#[from] RustlsError),
    /// Error when the crypto provider (like Ring) fails to initialize, which is crucial for TLS operations.
    #[error("Unable to initialize the crypto provider")]
    CryptoProviderInitialize,
    /// Error related to input/output operations, such as network or file system access.
    #[error("IO error: {0}")]
    Io(#[from] IoError),
    /// Error originating from the Hyper HTTP client, indicating an HTTP-related issue during certificate fetching.
    #[error("HTTP error: {0}")]
    Http(#[from] HttpError),
}

/// Implements a certificate validator with custom verification logic, fingerprint-based trust, and caching.
///
/// `CertValidator` is responsible for verifying server certificates. It combines standard WebPKI validation
/// with a custom trust mechanism based on certificate fingerprints. It persistently stores and caches
/// mappings of fingerprints to trusted hostnames, allowing for fingerprint-based server identity verification.
/// This is useful for scenarios where traditional CA-based trust is not sufficient or practical.
#[derive(Debug, Clone)]
pub struct CertValidator {
    /// The standard WebPKI server verifier used for baseline certificate validation against root CAs.
    inner_verifier: Arc<WebPkiServerVerifier>,
    /// Persistent storage manager for saving and loading fingerprint-to-hostname mappings.
    storage: Arc<PersistentDataManager<FingerprintReport>>,
    /// In-memory cache for fingerprint-to-hostname mappings to speed up verification lookups.
    cached_entries: Arc<RwLock<HashMap<String, Vec<String>>>>,
}

/// Stores mappings of certificate fingerprints to trusted hostnames for persistent storage.
///
/// `FingerprintReport` is used to serialize and deserialize the fingerprint-to-hostname mappings
/// for persistent storage using `PersistentDataManager`. It contains a HashMap where keys are certificate
/// fingerprints (base85 encoded) and values are lists of hostnames that are trusted for that fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FingerprintReport {
    /// HashMap mapping certificate fingerprints to a list of trusted hostnames.
    pub entries: HashMap<String, Vec<String>>, // fingerprint -> list of hosts
}

impl CertValidator {
    /// Creates a new `CertValidator` instance, initializing storage, root certificate store, and cache.
    ///
    /// This asynchronous constructor sets up a `CertValidator` by initializing its persistent storage
    /// using `PersistentDataManager`, setting up a root certificate store with system-default root certificates
    /// for WebPKI verification, and initializing an in-memory cache from the persistent storage data.
    /// It also starts a background task to monitor for updates in the persistent storage and refresh the cache.
    ///
    /// # Arguments
    /// * `path` - The path to the directory where persistent data for known servers will be stored.
    ///
    /// # Returns
    /// `Result<Self, CertValidatorError>` - A `Result` containing the new `CertValidator` instance,
    ///                                        or a `CertValidatorError` if initialization fails.
    ///
    /// # Errors
    /// Returns `CertValidatorError` if:
    /// - Persistent storage initialization fails (`PersistenceError`).
    /// - Building the WebPKI server verifier fails (`TlsError`).
    pub async fn new<P: AsRef<Path>>(path: P) -> Result<Self, CertValidatorError> {
        let storage_path = path.as_ref().join(".known-servers");
        let storage: Arc<PersistentDataManager<FingerprintReport>> =
            Arc::new(PersistentDataManager::new(storage_path)?);

        // Initialize root certificate store with system roots for WebPKI verifier
        let mut root_store = RootCertStore::empty();
        root_store.extend(TLS_SERVER_ROOTS.iter().cloned());

        // Build WebPKI server verifier with the initialized root store and default crypto provider
        let inner_verifier = WebPkiServerVerifier::builder_with_provider(
            Arc::new(root_store),
            Arc::new(default_provider()),
        )
        .build()
        .map_err(|e: VerifierBuilderError| {
            CertValidatorError::TlsError(RustlsError::General(e.to_string()))
        })?;

        // Initialize cache with initial data from persistent storage
        let initial_entries = storage.read().await.entries.clone();
        let cached_entries = Arc::new(RwLock::new(initial_entries));

        // Set up background task to monitor storage updates and refresh cache
        let mut subscriber = storage.subscribe();
        let cached_clone = cached_entries.clone();
        tokio::spawn(async move {
            while let Ok(data) = subscriber.recv().await {
                if let Ok(mut cache) = cached_clone.write() {
                    *cache = data.entries.clone();
                }
            }
        });

        Ok(Self {
            inner_verifier,
            storage,
            cached_entries,
        })
    }

    /// Adds a list of trusted domains for a given certificate fingerprint.
    ///
    /// This method updates the persistent storage and in-memory cache to associate a certificate fingerprint
    /// with a list of domains that are considered trusted when presenting a certificate with this fingerprint.
    /// It ensures that domain entries are unique and sorted for each fingerprint.
    ///
    /// # Arguments
    /// * `domains` - An iterable of domain names (`AsRef<str>`) to be trusted for the given fingerprint.
    /// * `fingerprint` - The certificate fingerprint (`AsRef<str>`) for which the domains are being trusted.
    ///
    /// # Returns
    /// `Result<(), CertValidatorError>` - A `Result` indicating success or failure.
    ///
    /// # Errors
    /// Returns `CertValidatorError::Persistence` if updating the persistent storage fails.
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
        let fingerprint = fingerprint.as_ref().to_string(); // Convert fingerprint to String
        let domains: Vec<String> = domains
            .into_iter()
            .map(|d| d.as_ref().to_string()) // Convert domains to Vec<String>
            .collect();

        self.storage
            .update(|mut report| async move {
                // Update operation on persistent storage
                for domain in &domains {
                    report
                        .entries
                        .entry(fingerprint.clone()) // Get or create entry for fingerprint
                        .or_default() // Get default (empty Vec) if entry doesn't exist
                        .push(domain.clone()); // Push domain to the list of hosts for this fingerprint
                }

                // Ensure hosts are unique and sorted after adding new domains
                if let Some(hosts) = report.entries.get_mut(&fingerprint) {
                    hosts.sort(); // Sort the host list
                    hosts.dedup(); // Remove duplicate entries
                }

                Ok::<_, CertValidatorError>((report, ())) // Return updated report and success result
            })
            .await?;

        Ok(())
    }

    /// Replaces the list of trusted hosts for a given certificate fingerprint with a new list.
    ///
    /// This method completely replaces the existing list of trusted hostnames associated with a given
    /// certificate fingerprint with a new list of hostnames. It updates both persistent storage and in-memory cache.
    ///
    /// # Arguments
    /// * `fingerprint` - The certificate fingerprint for which to replace the trusted hosts.
    /// * `new_hosts` - The new list of hostnames to trust for this fingerprint.
    ///
    /// # Returns
    /// `Result<(), CertValidatorError>` - A `Result` indicating success or failure.
    ///
    /// # Errors
    /// Returns `CertValidatorError::Persistence` if updating the persistent storage fails.
    pub async fn replace_hosts_for_fingerprint(
        &self,
        fingerprint: &str,
        new_hosts: Vec<String>,
    ) -> Result<(), CertValidatorError> {
        self.storage
            .update(|mut report| async move {
                // Update operation on persistent storage
                report.entries.insert(fingerprint.to_string(), new_hosts); // Insert new hosts list for the fingerprint, replacing any existing list
                Ok((report, ())) // Return updated report and success result
            })
            .await
    }

    /// Removes a certificate fingerprint and its associated trusted hosts from the system.
    ///
    /// This method deletes a certificate fingerprint entry from both persistent storage and the in-memory cache.
    /// After this operation, the system will no longer trust any server presenting a certificate with this fingerprint
    /// based on the custom trust mechanism (though WebPKI verification might still apply).
    ///
    /// # Arguments
    /// * `fingerprint` - The certificate fingerprint to remove.
    ///
    /// # Returns
    /// `Result<(), CertValidatorError>` - A `Result` indicating success or failure.
    ///
    /// # Errors
    /// Returns `CertValidatorError::Persistence` if updating the persistent storage fails.
    pub async fn remove_fingerprint(&self, fingerprint: &str) -> Result<(), CertValidatorError> {
        self.storage
            .update(|mut report| async move {
                // Update operation on persistent storage
                report.entries.remove(fingerprint); // Remove the entry for the given fingerprint
                Ok((report, ())) // Return updated report and success result
            })
            .await
    }

    /// Retrieves the list of trusted hostnames associated with a given certificate fingerprint.
    ///
    /// This method looks up a certificate fingerprint in the persistent storage and returns the list of hostnames
    /// that are trusted for certificates with this fingerprint. If the fingerprint is not found, it returns an empty list.
    ///
    /// # Arguments
    /// * `fingerprint` - The certificate fingerprint to look up.
    ///
    /// # Returns
    /// `Vec<String>` - A vector of hostnames trusted for the given fingerprint, or an empty vector if the fingerprint is not found.
    pub async fn get_hosts_for_fingerprint(&self, fingerprint: &str) -> Vec<String> {
        self.storage
            .read()
            .await // Acquire read lock on persistent storage
            .entries
            .get(fingerprint) // Get the host list for the fingerprint
            .cloned() // Clone the list if found
            .unwrap_or_default() // Return default empty Vec if fingerprint not found
    }

    /// Lists all certificate fingerprints that are currently trusted in the system.
    ///
    /// This method retrieves all fingerprints that have associated trusted hostnames from the persistent storage.
    /// It provides a list of fingerprints for which custom trust rules are defined.
    ///
    /// # Returns
    /// `Vec<String>` - A vector of all trusted certificate fingerprints.
    pub async fn list_trusted_fingerprints(&self) -> Vec<String> {
        self.storage.read().await.entries.keys().cloned().collect() // Read entries, get keys (fingerprints), clone and collect to Vec
    }

    pub async fn clone_trusted_entries(&self) -> HashMap<String, Vec<String>> {
        self.storage.read().await.entries.clone()
    }

    /// Finds all certificate fingerprints that trust a specific hostname.
    ///
    /// This method searches through all stored fingerprint-to-hostname mappings and returns a list of fingerprints
    /// that include the given hostname in their list of trusted hosts. This is useful for determining which fingerprints
    /// are associated with a particular domain.
    ///
    /// # Arguments
    /// * `host` - The hostname to search for.
    ///
    /// # Returns
    /// `Vec<String>` - A vector of certificate fingerprints that trust the given hostname.
    pub async fn find_fingerprints_by_host(&self, host: &str) -> Vec<String> {
        let host = canonical_host(host);
        self.storage
            .read()
            .await // Acquire read lock on persistent storage
            .entries
            .iter() // Iterate over fingerprint-to-hosts entries
            .filter(|(_, hosts)| {
                hosts
                    .iter()
                    .any(|candidate| trusted_host_matches(candidate, &host))
            }) // Match a trusted host with or without its explicit port
            .map(|(fp, _)| fp.clone()) // Map filtered entries to fingerprints (keys)
            .collect() // Collect fingerprints to Vec<String>
    }

    /// Verifies a server certificate against the stored trusted fingerprints for a given server name.
    ///
    /// This method checks if a given certificate, represented by its DER encoding, is trusted for a specific
    /// server name. It calculates the certificate's fingerprint and checks if this fingerprint is associated
    /// with the server name in the persistent storage of trusted fingerprints.
    ///
    /// # Arguments
    /// * `cert` - The server certificate in DER format (`CertificateDer`).
    /// * `server_name` - The server name (hostname) for which the certificate is being verified.
    ///
    /// # Returns
    /// `Result<(), CertValidatorError>` - A `Result` indicating success (certificate is trusted) or failure.
    ///
    /// # Errors
    /// Returns `CertValidatorError::CertificateParsing` if certificate parsing fails.
    /// Returns `CertValidatorError::FingerprintMismatch` if the certificate fingerprint is not found or does not trust the server name.
    pub async fn verify_certificate(
        &self,
        cert: &CertificateDer<'_>,
        server_name: &str,
    ) -> Result<(), CertValidatorError> {
        let (_, raw_cert) = parse_x509_certificate(cert.as_ref())
            .map_err(|e| CertValidatorError::CertificateParsing(e.to_string()))?; // Parse DER certificate to X509

        let fingerprint = calculate_base85_fingerprint(raw_cert.public_key().raw)
            .map_err(|e| CertValidatorError::CertificateParsing(e.to_string()))?; // Calculate fingerprint

        let trusted_fingerprints = self.find_fingerprints_by_host(server_name).await;

        let valid = trusted_fingerprints.contains(&fingerprint);

        if valid {
            Ok(()) // Certificate is trusted based on fingerprint and server name
        } else {
            Err(CertValidatorError::FingerprintMismatch) // Fingerprint not trusted for this server name
        }
    }

    /// Converts the `CertValidator` into a `ClientConfig` for use in rustls clients.
    ///
    /// This method creates a `ClientConfig` that is configured to use this `CertValidator` for server
    /// certificate verification. The resulting `ClientConfig` can then be used to create TLS connections
    /// where certificate verification is handled by this custom validator.
    ///
    /// # Returns
    /// `ClientConfig` - A rustls `ClientConfig` configured to use this `CertValidator`.
    pub fn into_client_config(self: Arc<Self>) -> ClientConfig {
        ClientConfig::builder()
            .dangerous() // Dangerous because it bypasses normal CA verification for fingerprints
            .with_custom_certificate_verifier(self) // Set the custom certificate verifier
            .with_no_client_auth() // Disable client authentication
    }

    /// Retrieves a clone of the current fingerprint-to-hosts mappings.
    ///
    /// This method provides a snapshot of the current state of trusted fingerprints and their associated hostnames
    /// from the in-memory cache. This is useful for inspecting the currently loaded trust configuration.
    ///
    /// # Returns
    /// `Result<HashMap<String, Vec<String>>>` - A `Result` containing the HashMap of fingerprints to lists of hostnames,
    ///                                          or an error if the cache cannot be read.
    ///
    /// # Errors
    /// Returns `anyhow::Error` if the read lock on the cache cannot be acquired.
    pub fn fingerprints(&self) -> Result<HashMap<String, Vec<String>>> {
        self.cached_entries
            .read() // Acquire read lock on cache
            .map_err(|e| anyhow!("Unable to read fingerprints: {e}")) // Map lock error to anyhow Error
            .map(|guard| guard.clone()) // Clone the HashMap from the read guard
    }

    /// Subscribes to changes in the fingerprint report.
    ///
    /// This method returns a broadcast receiver that will receive updates whenever the `FingerprintReport` is modified
    /// in the persistent storage. Subscribers can use this to get notified of changes to the trusted fingerprint configuration.
    ///
    /// # Returns
    /// `broadcast::Receiver<FingerprintReport>` - A broadcast receiver for `FingerprintReport` updates.
    pub fn subscribe_changes(&self) -> broadcast::Receiver<FingerprintReport> {
        self.storage.subscribe() // Subscribe to storage changes and return the receiver
    }
}

impl ServerCertVerifier for CertValidator {
    /// Verifies a server certificate during TLS handshake, implementing the `ServerCertVerifier` trait.
    ///
    /// This is the core method for custom certificate verification. It first attempts to verify the certificate
    /// based on the stored trusted fingerprints. If a fingerprint match is found and the server name is in the
    /// trusted hosts list for that fingerprint, the verification succeeds. If not, it falls back to the standard
    /// WebPKI verification using the `inner_verifier`.
    ///
    /// # Arguments
    /// * `end_entity` - The server's end-entity certificate in DER format.
    /// * `intermediates` - Intermediate certificates provided by the server.
    /// * `server_name` - The server name being connected to.
    /// * `ocsp_response` - OCSP response (not used in this implementation).
    /// * `now` - Current time (not used in this implementation).
    ///
    /// # Returns
    /// `Result<ServerCertVerified, RustlsError>` - A `Result` indicating whether the certificate is verified.
    ///                                              `Ok(ServerCertVerified::assertion())` for successful verification,
    ///                                              or `Err(RustlsError)` for verification failure.
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        let server_name_str = server_name_to_string(server_name)?; // Convert ServerName to String

        let (_, cert) = parse_x509_certificate(end_entity.as_ref())
            .map_err(|e| RustlsError::General(e.to_string()))?; // Parse DER certificate to X509
        let public_key_der = cert.public_key().raw;
        let fingerprint = calculate_base85_fingerprint(public_key_der)
            .map_err(|e| RustlsError::General(e.to_string()))?; // Calculate fingerprint

        let entries = self
            .cached_entries
            .read()
            .map_err(|_| RustlsError::General("Failed to acquire the lock".into()))?; // Read cached fingerprint entries
        let server_host = canonical_host(&server_name_str);
        let is_trusted = entries
            .get(&fingerprint) // Get trusted hosts for the fingerprint from cache
            .map(|hosts| {
                hosts
                    .iter()
                    .any(|host| trusted_host_matches(host, &server_host))
            }) // Check trusted hosts with or without explicit ports
            .unwrap_or(false); // Default to false if fingerprint not found

        if is_trusted {
            Ok(ServerCertVerified::assertion()) // Certificate is trusted based on fingerprint
        } else {
            warn!(
                "{}({}) is not trust in the local cert list, falling back to the buildin one",
                server_name.to_str(),
                fingerprint
            );
            self.inner_verifier.verify_server_cert(
                // Fallback to WebPKI verifier if not fingerprint-trusted
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            )
        }
    }

    /// Returns the signature schemes supported by the inner verifier.
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner_verifier.supported_verify_schemes()
    }

    /// Verifies a TLS 1.2 handshake signature using the inner verifier.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.inner_verifier
            .verify_tls12_signature(message, cert, dss)
    }

    /// Verifies a TLS 1.3 handshake signature using the inner verifier.
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

/// Represents a server certificate with its PEM encoded content and fingerprint.
///
/// `ServerCert` is a simple struct to hold both the PEM encoded certificate and its base85 fingerprint.
/// It is used as a return type for functions that fetch or parse server certificates.
pub struct ServerCert {
    /// PEM encoded certificate content as a String.
    pub cert: String,
    /// Base85 encoded fingerprint of the certificate's public key.
    pub fingerprint: String,
}

/// Fetches a server's certificate from a given URL by initiating a TLS handshake.
///
/// This asynchronous function connects to a server at the given URL, initiates a TLS handshake, and captures
/// the server's certificate presented during the handshake. It uses a temporary certificate verifier (`TempCertVerifier`)
/// to intercept and store the server certificate without performing full verification.
///
/// # Arguments
/// * `url` - The URL of the server from which to fetch the certificate (e.g., "https://example.com:7863").
///
/// # Returns
/// `Result<ServerCert, CertValidatorError>` - A `Result` containing the `ServerCert` struct with the fetched certificate and fingerprint,
///                                            or a `CertValidatorError` if fetching or parsing fails.
///
/// # Errors
/// Returns `CertValidatorError` if:
/// - The URL is invalid (`InvalidServerName`).
/// - TCP connection fails (`IoError`).
/// - TLS handshake fails (`TlsError`).
/// - HTTP handshake fails (`TlsError`).
/// - Request creation fails (`HttpError`).
/// - Sending request fails (implicitly through connection error handling).
/// - Certificate parsing fails (`CertificateParsing`).
pub async fn fetch_server_certificate(url: &str) -> Result<ServerCert, CertValidatorError> {
    let uri = url
        .parse::<hyper::Uri>()
        .map_err(|_| CertValidatorError::InvalidServerName)?; // Parse URL to Uri
    let host = uri
        .host()
        .ok_or(CertValidatorError::InvalidServerName)? // Extract host from Uri
        .to_string();
    let address = DeviceAddress::new(host.clone(), uri.port_u16().unwrap_or(DEFAULT_DEVICE_PORT))
        .map_err(|_| CertValidatorError::InvalidServerName)?;

    let cert_info = Arc::new(Mutex::new(None)); // Arc Mutex to share certificate info across threads
    let verifier = TempCertVerifier {
        cert_info: Arc::clone(&cert_info), // Create TempCertVerifier with shared cert_info
    };

    let config = ClientConfig::builder()
        .dangerous() // Dangerous because it bypasses normal CA verification for fetching cert
        .with_custom_certificate_verifier(Arc::new(verifier)) // Set TempCertVerifier to capture cert
        .with_no_client_auth(); // Disable client authentication

    let connector = TlsConnector::from(Arc::new(config)); // Create TLS connector with custom config

    let tcp_stream = TcpStream::connect((address.host(), address.port())).await?;

    let server_name = server_name_for_host(&host)?; // Create ServerName from host

    let tls_stream = connector
        .connect(server_name, tcp_stream)
        .await
        .map_err(|e| CertValidatorError::TlsError(RustlsError::General(e.to_string())))?; // Establish TLS connection

    let io = TokioIo::new(tls_stream); // Convert TLS stream to TokioIo for Hyper

    let (mut sender, connection) = http1::handshake(io)
        .await
        .map_err(|e| CertValidatorError::TlsError(RustlsError::General(e.to_string())))?; // Perform HTTP/1 handshake

    tokio::spawn(async move {
        if let Err(err) = connection.await {
            eprintln!("Connection error: {}", { err }); // Spawn task to handle connection errors
        }
    });

    let request = Request::builder()
        .uri(uri) // Build GET request to the target URI
        .method("GET")
        .body(Empty::<Bytes>::new())?; // Empty body for GET request

    let _ = sender.send_request(request).await; // Send the request (response not needed for cert fetch)

    let guard = cert_info.lock().unwrap(); // Lock and access the captured certificate info
    guard
        .clone() // Clone the captured CertInfo
        .map(|(cert, fingerprint)| ServerCert { cert, fingerprint }) // Map CertInfo to ServerCert
        .ok_or(CertValidatorError::CertificateParsing(
            "No certificate captured".into(), // Return error if no certificate was captured
        ))
}

/// Parses a PEM encoded certificate string and extracts the certificate and its fingerprint.
///
/// This function takes a PEM encoded certificate as a string, parses it, and calculates the base85
/// fingerprint of the certificate's public key. It returns both the PEM encoded certificate string and its fingerprint.
///
/// # Arguments
/// * `x` - The PEM encoded certificate as a string.
///
/// # Returns
/// `Result<(String, String)>` - A `Result` containing a tuple of the PEM encoded certificate string and its fingerprint,
///                              or an error if parsing or fingerprint calculation fails.
///
/// # Errors
/// Returns `RustlsError` wrapped in `anyhow::Error` if:
/// - PEM parsing fails.
/// - The PEM is not a CERTIFICATE.
/// - X509 certificate parsing from DER content fails.
/// - Fingerprint calculation fails.
pub fn parse_certificate(x: &str) -> Result<(String, String)> {
    let pem = pem::parse(x).map_err(|e| RustlsError::General(e.to_string()))?; // Parse PEM string
    if pem.tag() != "CERTIFICATE" {
        bail!(format!("Expected CERTIFICATE PEM, got {}", pem.tag())) // Ensure PEM tag is CERTIFICATE
    }
    let der = pem.contents(); // Get DER content from PEM

    let (_, raw_cert) =
        parse_x509_certificate(der).map_err(|e| RustlsError::General(e.to_string()))?; // Parse DER to X509 certificate

    let cert_pem = Pem::new("CERTIFICATE".to_string(), der.to_vec()); // Re-create PEM for encoding
    let cert = pem::encode(&cert_pem); // Encode PEM to string

    let fingerprint = calculate_base85_fingerprint(raw_cert.public_key().raw)
        .map_err(|e| RustlsError::General(e.to_string()))?; // Calculate fingerprint

    Ok((cert, fingerprint)) // Return PEM encoded certificate and fingerprint
}

/// A temporary certificate verifier to capture and store a server's certificate during handshake.
///
/// `TempCertVerifier` is a custom `ServerCertVerifier` implementation designed specifically to intercept
/// and store the server certificate presented during a TLS handshake. It does not perform actual verification
/// but rather captures the certificate information for later use, such as fetching a server certificate.
#[derive(Debug)]
struct TempCertVerifier {
    /// Arc Mutex to store the captured certificate information (PEM and fingerprint).
    cert_info: Arc<Mutex<CertInfo>>,
}

fn verify_presented_key_signature(
    message: &[u8],
    cert: &CertificateDer<'_>,
    signature: &rustls::DigitallySignedStruct,
) -> Result<HandshakeSignatureValid, RustlsError> {
    crypto::verify_tls12_signature(
        message,
        cert,
        signature,
        &default_provider().signature_verification_algorithms,
    )
}

impl ServerCertVerifier for TempCertVerifier {
    /// Intercepts and stores the server certificate, asserting verification success without actual validation.
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        let (_, raw_cert) = parse_x509_certificate(end_entity.as_ref())
            .map_err(|e| RustlsError::General(e.to_string()))?; // Parse DER certificate to X509

        let cert = Pem::new("CERTIFICATE".to_string(), raw_cert.public_key().raw); // Create PEM from public key DER
        let cert = pem::encode(&cert); // Encode PEM to string

        let fingerprint = calculate_base85_fingerprint(raw_cert.public_key().raw)
            .map_err(|e| RustlsError::General(e.to_string()))?; // Calculate fingerprint

        *self.cert_info.lock().unwrap() = Some((cert, fingerprint)); // Store the certificate info in shared Mutex
        Ok(ServerCertVerified::assertion())
    }

    /// Returns a list of supported signature schemes for this verifier.
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
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        verify_presented_key_signature(message, cert, signature)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        crypto::verify_tls13_signature(
            message,
            cert,
            signature,
            &default_provider().signature_verification_algorithms,
        )
    }
}

/// Trusts a server by adding its domains and IPs to the trusted list for a given certificate fingerprint.
///
/// This asynchronous function takes a `CertValidator`, a list of domains, a list of IPs, and a certificate fingerprint.
/// It combines domains and IPs into a single list and then calls `add_trusted_domains` on the `CertValidator`
/// to associate these hostnames with the provided certificate fingerprint, effectively marking the server as trusted
/// for these domains and IPs when presenting a certificate with this fingerprint.
///
/// # Arguments
/// * `validator` - A reference to the `CertValidator` instance.
/// * `domains` - A vector of domain names to trust.
/// * `ips` - A vector of IP addresses to trust.
/// * `fingerprint` - The certificate fingerprint for which to trust the domains and IPs.
///
/// # Returns
/// `Result<(), CertValidatorError>` - A `Result` indicating success or failure.
///
/// # Errors
/// Returns `CertValidatorError` if `add_trusted_domains` fails, typically due to persistence issues.
pub async fn trust_server(
    validator: &CertValidator,
    domains: Vec<String>,
    ips: Vec<String>,
    fingerprint: &str,
) -> Result<(), CertValidatorError> {
    let mut all_entries = Vec::with_capacity(domains.len() + ips.len()); // Create Vec to hold all entries
    all_entries.extend(domains); // Extend with domain names
    all_entries.extend(ips); // Extend with IP addresses

    validator
        .add_trusted_domains(all_entries, fingerprint) // Add all entries as trusted domains for the fingerprint
        .await
}

/// Attempts to establish a TLS connection to a host using a provided `ClientConfig`.
///
/// This asynchronous function tries to connect to a specified host using TLS with a given `ClientConfig`.
/// It is primarily used to test connectivity and certificate validation using a specific configuration.
/// It returns the host string on successful connection, or an error if connection establishment fails.
///
/// # Arguments
/// * `host` - The host URL string to connect to (e.g., "https://example.com:7863").
/// * `config` - The rustls `ClientConfig` to use for establishing the TLS connection.
///
/// # Returns
/// `Result<String, anyhow::Error>` - A `Result` containing the host string on successful connection,
///                                   or an `anyhow::Error` if connection establishment fails.
///
/// # Errors
/// Returns `anyhow::Error` if:
/// - URL parsing fails.
/// - Server name creation fails.
/// - TCP connection fails.
/// - TLS handshake fails.
pub async fn try_connect(host: &str, config: ClientConfig) -> Result<String> {
    let uri = host.parse::<Uri>()?; // Parse host string to Uri
    let host_str = uri.host().unwrap().to_string(); // Extract host string from Uri
    let sni = server_name_for_host(&host_str)?; // Create ServerName for SNI

    let connector = TlsConnector::from(Arc::new(config)); // Create TLS connector from ClientConfig
    let address = DeviceAddress::new(host_str, uri.port_u16().unwrap_or(DEFAULT_DEVICE_PORT))?;
    let tcp = TcpStream::connect((address.host(), address.port())).await?;
    let _ = connector.connect(sni, tcp).await?; // Establish TLS connection

    Ok(host.to_string()) // Return host string on successful connection
}

/// How long one address may take to connect and finish its TLS handshake. Selection waits for
/// every address until one connects, so an address that accepts but never answers would
/// otherwise hold it up for good.
const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);

/// Selects the best host from a list of candidates by connecting to all of them in parallel
/// and returning the first one that successfully establishes a TLS connection and validates
/// the certificate.
///
/// This function attempts to connect to all provided hosts simultaneously and returns as soon
/// as one host successfully connects. It cancels all remaining connection attempts once a
/// successful connection is established.
///
/// # Arguments
/// * `hosts` - A vector of host URLs to try connecting to.
/// * `config` - The rustls `ClientConfig` to use for establishing the TLS connections.
///
/// # Returns
/// `Result<String, anyhow::Error>` - A `Result` containing the URL of the first host that
///                                   successfully connected, or an `anyhow::Error` if all
///                                   connection attempts failed.
///
/// # Errors
/// Returns `anyhow::Error` if:
/// - All connection attempts fail.
/// - No hosts are provided in the input vector.
pub async fn select_best_host(hosts: Vec<String>, config: Arc<ClientConfig>) -> Result<String> {
    let mut attempts = hosts
        .into_iter()
        .map(|host| {
            let config = (*config).clone();
            async move {
                timeout(CONNECT_ATTEMPT_TIMEOUT, try_connect(&host, config))
                    .await
                    .unwrap_or_else(|_| {
                        Err(anyhow!("no answer within {CONNECT_ATTEMPT_TIMEOUT:?}"))
                    })
                    .map_err(|e| anyhow!("Failed to connect to {}: {}", host, e))
            }
        })
        .collect::<FuturesUnordered<_>>();

    let mut first_error = None;
    while let Some(attempt) = attempts.next().await {
        match attempt {
            // Returning drops the attempts still running, which cancels them.
            Ok(host) => return Ok(host),
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    Err(match first_error {
        Some(error) => anyhow!("All connection attempts failed. First error: {}", error),
        None => anyhow!("No hosts provided"),
    })
}

/// Utility function to test multiple hosts and measure their response times.
///
/// This function connects to all provided hosts in parallel and reports their response times.
/// Unlike `select_best_host`, this function does not cancel any connection attempts and waits
/// for all of them to complete or fail.
///
/// # Arguments
/// * `hosts` - A vector of host URLs to try connecting to.
/// * `config` - The rustls `ClientConfig` to use for establishing the TLS connections.
///
/// # Returns
/// `Result<Vec<(String, std::time::Duration)>, anyhow::Error>` - A `Result` containing a vector
///                                                               of tuples with host URLs and their
///                                                               response times, sorted by response time.
///
/// # Errors
/// Returns `anyhow::Error` if no hosts are provided.
pub async fn measure_host_response_times(
    hosts: Vec<String>,
    config: ClientConfig,
) -> Result<Vec<(String, std_time::Duration)>> {
    if hosts.is_empty() {
        return Err(anyhow!("No hosts provided"));
    }

    let config = Arc::new(config);
    let mut tasks = Vec::with_capacity(hosts.len());

    for host in hosts {
        let config_clone = config.clone();
        let task = async move {
            let start = Instant::now();
            let result = try_connect(&host, (*config_clone).clone()).await;
            let duration = start.elapsed();

            (host, result, duration)
        };

        tasks.push(task);
    }

    let results = future::join_all(tasks).await;

    // Filter successful connections and sort by response time
    let mut response_times: Vec<(String, std_time::Duration)> = results
        .into_iter()
        .filter_map(|(host, result, duration)| {
            if result.is_ok() {
                Some((host, duration))
            } else {
                None
            }
        })
        .collect();

    // Sort by response time (fastest first)
    response_times.sort_by_key(|(_, duration)| *duration);

    Ok(response_times)
}

#[cfg(test)]
mod tests {
    use super::{
        CertValidator, TempCertVerifier, fetch_server_certificate, select_best_host,
        server_name_for_host, trusted_host_matches, try_connect,
    };
    use crate::ssl::generate_self_signed_cert;
    use crate::utils::server_name_to_string;
    use rustls::client::danger::ServerCertVerifier;
    use rustls::crypto::ring::default_provider;
    use rustls::{
        ClientConfig, ServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
        server::{ClientHello, ResolvesServerCert},
        sign::CertifiedKey,
    };
    use std::{
        net::Ipv6Addr,
        sync::{Arc, Mutex},
        time::{Duration, SystemTime},
    };
    use tokio::time as tokio_time;
    use tokio::{net::TcpListener, task::JoinHandle};
    use tokio_rustls::TlsAcceptor;

    #[derive(Debug)]
    struct TestCertificateResolver(Arc<CertifiedKey>);

    impl ResolvesServerCert for TestCertificateResolver {
        fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
            Some(self.0.clone())
        }
    }

    #[test]
    fn trusted_entry_with_port_matches_its_host_only() {
        assert!(trusted_host_matches("127.0.0.1", "127.0.0.1"));
        assert!(trusted_host_matches("127.0.0.1:7863", "127.0.0.1"));
        assert!(trusted_host_matches("[::1]:7863", "::1"));
        assert!(trusted_host_matches("::1", "::1"));
        assert!(trusted_host_matches(
            "[2001:db8::]:7863",
            "2001:db8:0:0:0:0:0:0"
        ));
        assert!(!trusted_host_matches("[2001:db8::1]:7863", "2001:db8::"));
        assert!(!trusted_host_matches("127.0.0.1:7863", "127.0.0.2"));
        assert!(!trusted_host_matches("127.0.0.10", "127.0.0.1"));
    }

    #[test]
    fn uri_hosts_become_their_ip_server_name() {
        for host in ["[::1]", "::1"] {
            let name = server_name_for_host(host).unwrap();
            assert!(
                matches!(name, ServerName::IpAddress(_)),
                "{host} must name an IP, not a DNS name"
            );
        }
        let name = server_name_for_host("[2001:db8::ff00:42]").unwrap();
        assert_eq!(server_name_to_string(&name).unwrap(), "2001:db8::ff00:42");
        let name = server_name_for_host("device.local").unwrap();
        assert!(matches!(name, ServerName::DnsName(_)));
        let name = server_name_for_host("192.168.1.5").unwrap();
        assert_eq!(server_name_to_string(&name).unwrap(), "192.168.1.5");
    }

    fn certified_key(certificate_pem: &str, private_key_pem: &str) -> CertifiedKey {
        let certificate = pem::parse(certificate_pem).unwrap();
        let private_key = pem::parse(private_key_pem).unwrap();
        let key = default_provider()
            .key_provider
            .load_private_key(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                private_key.contents().to_vec(),
            )))
            .unwrap();
        CertifiedKey::new(
            vec![CertificateDer::from(certificate.contents().to_vec())],
            key,
        )
    }

    /// Serves one TLS handshake with `key` on a loopback port, and returns that port.
    async fn serve_one_handshake(key: CertifiedKey) -> (u16, JoinHandle<()>) {
        let server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(TestCertificateResolver(Arc::new(key))));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let acceptor = TlsAcceptor::from(Arc::new(server_config));
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let _ = acceptor.accept(stream).await;
        });
        (port, server)
    }

    /// A loopback TLS server that accepts `accepts` handshakes with `key`, bound on IPv4 or IPv6.
    async fn serve_tls(key: CertifiedKey, bind: &str, accepts: usize) -> (u16, JoinHandle<()>) {
        let server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(TestCertificateResolver(Arc::new(key))));
        let listener = TcpListener::bind(bind).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let acceptor = TlsAcceptor::from(Arc::new(server_config));
        let server = tokio::spawn(async move {
            for _ in 0..accepts {
                let (stream, _) = listener.accept().await.unwrap();
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let _ = acceptor.accept(stream).await;
                });
            }
        });
        (port, server)
    }

    fn certificate_der(certificate_pem: &str) -> CertificateDer<'static> {
        CertificateDer::from(pem::parse(certificate_pem).unwrap().contents().to_vec())
    }

    fn unix_now() -> UnixTime {
        UnixTime::since_unix_epoch(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap(),
        )
    }

    /// The pinned-host check a TLS handshake performs: the validator's in-memory trust is seeded
    /// directly so the check does not depend on the asynchronous cache refresh.
    async fn pinned_validator(
        fingerprint: &str,
        hosts: &[&str],
    ) -> (Arc<CertValidator>, tempfile::TempDir) {
        let directory = tempfile::tempdir().unwrap();
        let validator = Arc::new(CertValidator::new(directory.path()).await.unwrap());
        validator.cached_entries.write().unwrap().insert(
            fingerprint.to_owned(),
            hosts.iter().map(|host| host.to_string()).collect(),
        );
        (validator, directory)
    }

    fn pinned_certificate_is_accepted(
        validator: &CertValidator,
        certificate: &CertificateDer<'static>,
        server_name: &ServerName<'static>,
    ) -> bool {
        validator
            .verify_server_cert(certificate, &[], server_name, &[], unix_now())
            .is_ok()
    }

    fn loopback_server_name() -> ServerName<'static> {
        ServerName::from(std::net::IpAddr::from(Ipv6Addr::LOCALHOST))
    }

    #[tokio::test]
    async fn pinned_certificate_matches_canonical_ipv6_hosts() {
        let cert = generate_self_signed_cert("localhost", "rune", "CA", 1).unwrap();
        let der = certificate_der(&cert.certificate);
        let (validator, _directory) =
            pinned_validator(&cert.public_key_fingerprint, &["::1"]).await;

        for literal in ["::1", "0:0:0:0:0:0:0:1"] {
            let address: Ipv6Addr = literal.parse().unwrap();
            let server_name = ServerName::from(std::net::IpAddr::from(address));
            assert!(
                pinned_certificate_is_accepted(&validator, &der, &server_name),
                "certificate pinned for ::1 must verify against {literal}"
            );
        }
    }

    #[tokio::test]
    async fn pinned_certificate_matches_trailing_and_interior_compressed_ipv6() {
        let cert = generate_self_signed_cert("localhost", "rune", "CA", 1).unwrap();
        let der = certificate_der(&cert.certificate);

        for (trusted, literal) in [
            ("2001:db8::", "2001:db8:0:0:0:0:0:0"),
            ("2001:db8::ff00:42", "2001:db8:0:0:0:0:ff00:42"),
            ("::", "0:0:0:0:0:0:0:0"),
        ] {
            let (validator, _directory) =
                pinned_validator(&cert.public_key_fingerprint, &[trusted]).await;
            let address: Ipv6Addr = literal.parse().unwrap();
            let server_name = ServerName::from(std::net::IpAddr::from(address));
            assert!(
                pinned_certificate_is_accepted(&validator, &der, &server_name),
                "certificate pinned for {trusted} must verify against {literal}"
            );
        }
    }

    #[tokio::test]
    async fn pinned_certificate_matching_keeps_ipv4_and_dns_controls() {
        let cert = generate_self_signed_cert("localhost", "rune", "CA", 1).unwrap();
        let der = certificate_der(&cert.certificate);
        let (validator, _directory) = pinned_validator(
            &cert.public_key_fingerprint,
            &["127.0.0.1:7863", "device.local"],
        )
        .await;

        let ipv4 = ServerName::try_from("127.0.0.1").unwrap().to_owned();
        assert!(pinned_certificate_is_accepted(&validator, &der, &ipv4));
        let dns = ServerName::try_from("device.local").unwrap().to_owned();
        assert!(pinned_certificate_is_accepted(&validator, &der, &dns));
    }

    #[tokio::test]
    async fn a_certificate_pinned_elsewhere_stays_rejected() {
        let cert = generate_self_signed_cert("localhost", "rune", "CA", 1).unwrap();
        let other = generate_self_signed_cert("other", "rune", "CA", 1).unwrap();
        let der = certificate_der(&cert.certificate);
        let (validator, _directory) =
            pinned_validator(&other.public_key_fingerprint, &["::1"]).await;

        assert!(!pinned_certificate_is_accepted(
            &validator,
            &der,
            &loopback_server_name()
        ));

        let (validator, _directory) =
            pinned_validator(&cert.public_key_fingerprint, &["2001:db8::5"]).await;
        assert!(!pinned_certificate_is_accepted(
            &validator,
            &der,
            &loopback_server_name()
        ));
    }

    /// One certificate fetch and two connections over IPv6 loopback, with the certificate
    /// pinned for its canonical `::1` host.
    #[tokio::test]
    async fn ipv6_loopback_peer_is_reached_with_its_pinned_certificate() {
        let cert = generate_self_signed_cert("localhost", "rune", "CA", 1).unwrap();
        let key = certified_key(&cert.certificate, &cert.private_key);
        let (port, server) = serve_tls(key, "[::1]:0", 3).await;

        let (validator, _directory) =
            pinned_validator(&cert.public_key_fingerprint, &["[::1]:7863"]).await;
        let config = validator.clone().into_client_config();

        let url = format!("https://[::1]:{port}/ping");
        let fetched = fetch_server_certificate(&url).await;
        let fetched = match fetched {
            Ok(fetched) => fetched,
            Err(error) => panic!("certificate fetch over bracketed IPv6 must succeed: {error}"),
        };
        assert_eq!(fetched.fingerprint, cert.public_key_fingerprint);

        for attempt in 0..2 {
            let connected = try_connect(&url, config.clone()).await;
            match connected {
                Ok(_) => {}
                Err(error) => {
                    panic!("connection {attempt} to the pinned IPv6 peer must succeed: {error}")
                }
            }
        }
        server.abort();
    }

    #[tokio::test]
    async fn ipv6_peer_with_an_unpinned_certificate_stays_rejected() {
        let cert = generate_self_signed_cert("localhost", "rune", "CA", 1).unwrap();
        let other = generate_self_signed_cert("other", "rune", "CA", 1).unwrap();
        let key = certified_key(&cert.certificate, &cert.private_key);
        let (port, server) = serve_tls(key, "[::1]:0", 1).await;

        let (validator, _directory) =
            pinned_validator(&other.public_key_fingerprint, &["::1"]).await;
        let config = validator.clone().into_client_config();

        let url = format!("https://[::1]:{port}/ping");
        let connected = try_connect(&url, config).await;
        assert!(
            connected.is_err(),
            "a certificate pinned to another key must not verify"
        );
        server.abort();
    }

    #[tokio::test]
    async fn certificate_fetch_rejects_a_server_that_does_not_hold_its_key() {
        let cert = generate_self_signed_cert("localhost", "rune", "CA", 1).unwrap();
        let other_key = generate_self_signed_cert("other", "rune", "CA", 1).unwrap();
        let (port, server) =
            serve_one_handshake(certified_key(&cert.certificate, &other_key.private_key)).await;

        let result = fetch_server_certificate(&format!("https://127.0.0.1:{port}/ping")).await;
        assert!(result.is_err());
        server.await.unwrap();
    }

    fn accept_any_certificate() -> Arc<ClientConfig> {
        Arc::new(
            ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(TempCertVerifier {
                    cert_info: Arc::new(Mutex::new(None)),
                }))
                .with_no_client_auth(),
        )
    }

    // A host whose address cannot be parsed fails before any connection is made, so it is
    // always the first attempt to finish.
    const FAILING_HOST: &str = "https://not a host/";

    #[tokio::test]
    async fn host_selection_passes_over_a_host_that_fails_first() {
        let cert = generate_self_signed_cert("localhost", "rune", "CA", 1).unwrap();
        let (port, server) =
            serve_one_handshake(certified_key(&cert.certificate, &cert.private_key)).await;
        let reachable = format!("https://127.0.0.1:{port}/");

        let selected = select_best_host(
            vec![FAILING_HOST.to_owned(), reachable.clone()],
            accept_any_certificate(),
        )
        .await;

        assert_eq!(selected.unwrap(), reachable);
        server.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn host_selection_gives_up_on_a_host_that_never_finishes_its_handshake() {
        // Connections complete in the listen backlog, but nothing answers the TLS handshake.
        let silent = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stalled = format!("https://127.0.0.1:{}/", silent.local_addr().unwrap().port());

        let selected = tokio_time::timeout(
            Duration::from_secs(60),
            select_best_host(
                vec![FAILING_HOST.to_owned(), stalled],
                accept_any_certificate(),
            ),
        )
        .await
        .expect("host selection must finish although one host never answers");

        assert!(selected.is_err());
        drop(silent);
    }

    #[tokio::test]
    async fn host_selection_fails_once_every_host_has_failed() {
        let selected =
            select_best_host(vec![FAILING_HOST.to_owned()], accept_any_certificate()).await;
        assert!(selected.is_err());
        assert!(
            select_best_host(Vec::new(), accept_any_certificate())
                .await
                .is_err()
        );
    }
}
