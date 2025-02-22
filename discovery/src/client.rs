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

/// Type alias for certificate information, represented as an optional tuple.
///
/// `CertInfo` is used to hold certificate data, which is optionally a tuple containing
/// the certificate in string format (PEM encoded) and its base85 fingerprint. It is used
/// in contexts where certificate retrieval might fail or the certificate information is not yet available.
type CertInfo = Option<(String, String)>;

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
        let storage_clone = storage.clone();
        let cached_clone = cached_entries.clone();
        tokio::spawn(async move {
            let mut subscriber = storage_clone.subscribe(); // Subscribe to storage change notifications
            let mut debouncer = tokio::time::interval(Duration::from_millis(100)); // Debounce interval
            let mut pending_update = false; // Flag to indicate pending update

            loop {
                tokio::select! {
                    _ = subscriber.recv() => { // Received a storage update notification
                        pending_update = true; // Set flag to update cache on next tick
                    }
                    _ = debouncer.tick() => { // Debounce tick
                        if pending_update { // If there is a pending update
                            let data = storage_clone.read().await; // Read latest data from storage
                            // Fix: Use write() to get a mutable reference to cache and directly assign from storage
                            if let Ok(mut cache) = cached_clone.write() {
                                cache.clone_from(&data.entries); // Update cache with entries from storage
                            }
                            pending_update = false; // Reset pending update flag
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

                Ok((report, ())) // Return updated report and success result
            })
            .await
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
        self.storage
            .read()
            .await // Acquire read lock on persistent storage
            .entries
            .iter() // Iterate over fingerprint-to-hosts entries
            .filter(|(_, hosts)| hosts.contains(&host.to_string())) // Filter entries where host list contains the target host
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

        let valid = self
            .storage
            .read()
            .await // Acquire read lock on persistent storage
            .entries
            .get(&fingerprint) // Get trusted hosts for the fingerprint
            .map(|hosts| hosts.contains(&server_name.to_string())) // Check if server name is in the trusted hosts list
            .unwrap_or(false); // Default to false if fingerprint not found

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
    pub fn into_client_config(self) -> ClientConfig {
        ClientConfig::builder()
            .dangerous() // Dangerous because it bypasses normal CA verification for fingerprints
            .with_custom_certificate_verifier(Arc::new(self)) // Set the custom certificate verifier
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
            .map_err(|e| anyhow!("Unable to read fingerprints: {}", e)) // Map lock error to anyhow Error
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
        let is_trusted = entries
            .get(&fingerprint) // Get trusted hosts for the fingerprint from cache
            .map(|hosts| hosts.contains(&server_name_str)) // Check if server name is in the trusted hosts list
            .unwrap_or(false); // Default to false if fingerprint not found

        if is_trusted {
            Ok(ServerCertVerified::assertion()) // Certificate is trusted based on fingerprint
        } else {
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
    let port = uri.port_u16().unwrap_or(7863); // Extract port from Uri, default to 7863 if not specified

    let cert_info = Arc::new(Mutex::new(None)); // Arc Mutex to share certificate info across threads
    let verifier = TempCertVerifier {
        cert_info: Arc::clone(&cert_info), // Create TempCertVerifier with shared cert_info
    };

    let config = ClientConfig::builder()
        .dangerous() // Dangerous because it bypasses normal CA verification for fetching cert
        .with_custom_certificate_verifier(Arc::new(verifier)) // Set TempCertVerifier to capture cert
        .with_no_client_auth(); // Disable client authentication

    let connector = TlsConnector::from(Arc::new(config)); // Create TLS connector with custom config

    let tcp_stream = tokio::net::TcpStream::connect((host.clone(), port)).await?; // Connect to server via TCP

    let server_name =
        ServerName::try_from(host).map_err(|_| CertValidatorError::InvalidServerName)?; // Create ServerName from host

    let tls_stream = connector
        .connect(server_name, tcp_stream)
        .await
        .map_err(|e| CertValidatorError::TlsError(RustlsError::General(e.to_string())))?; // Establish TLS connection

    let io = TokioIo::new(tls_stream); // Convert TLS stream to TokioIo for Hyper

    let (mut sender, connection) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|e| CertValidatorError::TlsError(RustlsError::General(e.to_string())))?; // Perform HTTP/1 handshake

    tokio::spawn(async move {
        if let Err(err) = connection.await {
            eprintln!("Connection error: {}", err); // Spawn task to handle connection errors
        }
    });

    let request = hyper::Request::builder()
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
        Ok(ServerCertVerified::assertion()) // Assert verification success to continue handshake
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

    /// Asserts handshake signature validity for TLS 1.2 without actual verification.
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion()) // Assert signature validity without verification
    }

    /// Asserts handshake signature validity for TLS 1.3 without actual verification.
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion()) // Assert signature validity without verification
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
    let sni = ServerName::try_from(host_str.clone())?; // Create ServerName for SNI

    let connector = TlsConnector::from(Arc::new(config)); // Create TLS connector from ClientConfig
    let tcp = TcpStream::connect((host_str.as_str(), uri.port_u16().unwrap_or(7863))).await?; // Connect to host via TCP
    let _ = connector.connect(sni, tcp).await?; // Establish TLS connection

    Ok(host.to_string()) // Return host string on successful connection
}
