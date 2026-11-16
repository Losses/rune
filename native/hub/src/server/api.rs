use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use urlencoding::encode;

use ::http_request::{
    BodyExt, Bytes, ClientConfig, Empty, Full, Method, Request, StatusCode, Uri,
    create_https_client, send_http_request,
};

use super::utils::device::SanitizedDeviceInfo;
use crate::{messages::TrustedServerCertificate, request_error::ExpectedRequestError};
use ::discovery::server::PermissionManager;
use ::discovery::{
    client::{CertValidator, fetch_server_certificate},
    endpoint::{DeviceAddress, DeviceScheme, device_url},
    utils::DeviceType,
};

/// Remove a paired device from both trust directions.
pub async fn remove_trusted_device(
    permission_manager: &PermissionManager,
    validator: &CertValidator,
    fingerprint: &str,
) -> Result<()> {
    if permission_manager
        .list_users()
        .await
        .iter()
        .any(|user| user.fingerprint == fingerprint)
    {
        permission_manager
            .remove_user(fingerprint)
            .await
            .context("Remove trusted client")?;
    }
    validator
        .remove_fingerprint(fingerprint)
        .await
        .context("Remove trusted server")?;
    Ok(())
}

async fn verify_hosts_for_fingerprint(
    hosts: &[String],
    expected_fingerprint: &str,
) -> Result<Vec<String>> {
    let mut matched = false;
    let mut canonical_hosts = Vec::with_capacity(hosts.len());
    for host in hosts {
        let address: DeviceAddress = host
            .parse()
            .map_err(|_| ExpectedRequestError::invalid_input("Device address is malformed"))?;
        let url = address.url(DeviceScheme::Https, "/ping")?;
        canonical_hosts.push(address.to_string());
        let certificate = tokio::time::timeout(
            Duration::from_secs(5),
            fetch_server_certificate(url.as_str()),
        )
        .await;
        match certificate {
            Ok(Ok(certificate)) if certificate.fingerprint == expected_fingerprint => {
                matched = true;
            }
            Ok(Ok(_)) => return Err(ExpectedRequestError::CertificateMismatch.into()),
            Ok(Err(_)) | Err(_) => {}
        }
    }
    if matched {
        Ok(canonical_hosts)
    } else {
        Err(ExpectedRequestError::Unavailable {
            what: "certificate host",
        }
        .into())
    }
}

pub async fn add_trusted_server(
    validator: &CertValidator,
    certificate: Option<&TrustedServerCertificate>,
) -> Result<()> {
    let certificate = certificate.ok_or_else(|| {
        ExpectedRequestError::invalid_input("A certificate is required to trust a server")
    })?;
    let hosts = verify_hosts_for_fingerprint(&certificate.hosts, &certificate.fingerprint).await?;
    validator
        .add_trusted_domains(&hosts, &certificate.fingerprint)
        .await?;
    Ok(())
}

pub async fn edit_trusted_hosts(
    validator: &CertValidator,
    fingerprint: &str,
    hosts: Vec<String>,
) -> Result<()> {
    if !validator
        .list_trusted_fingerprints()
        .await
        .iter()
        .any(|trusted| trusted == fingerprint)
    {
        return Err(ExpectedRequestError::invalid_input(
            "Cannot edit hosts for an untrusted fingerprint",
        )
        .into());
    }
    let hosts = verify_hosts_for_fingerprint(&hosts, fingerprint).await?;
    validator
        .replace_hosts_for_fingerprint(fingerprint, hosts)
        .await?;
    Ok(())
}

pub async fn fetch_device_info(
    host: &str,
    config: Arc<ClientConfig>,
) -> Result<SanitizedDeviceInfo> {
    let address: DeviceAddress = host
        .parse()
        .map_err(|_| ExpectedRequestError::invalid_input("Device address is malformed"))?;
    fetch_device_info_at_port(address.host(), address.port(), config).await
}

pub async fn fetch_device_info_at_port(
    host: &str,
    port: u16,
    config: Arc<ClientConfig>,
) -> Result<SanitizedDeviceInfo> {
    let uri: Uri = device_url(host, port, DeviceScheme::Https, "/device-info")?
        .as_str()
        .parse()
        .context("Invalid URL format")?;

    let mut sender = create_https_client(host.to_owned(), port, config)
        .await
        .context("Failed to create HTTPS client")?;

    let req = Request::builder()
        .uri(uri)
        .header("Accept", "application/json")
        .body(Empty::<Bytes>::new())
        .context("Failed to build request")?;

    let res = send_http_request(&mut sender, req)
        .await
        .context("Failed to execute request")?;

    let body = res
        .into_body()
        .collect()
        .await
        .context("Failed to read response body")?
        .to_bytes();

    let device_info: SanitizedDeviceInfo =
        serde_json::from_slice(&body).context("Failed to parse device info")?;

    Ok(device_info)
}

#[derive(Debug, Serialize)]
struct RegisterRequest {
    public_key: String,
    fingerprint: String,
    alias: String,
    device_model: String,
    device_type: DeviceType,
}

#[derive(Deserialize)]
struct RegisterErrorResponse {
    code: String,
}

pub async fn register_device(
    host: &str,
    config: Arc<ClientConfig>,
    public_key: String,
    fingerprint: String,
    alias: String,
    device_model: String,
    device_type: DeviceType,
) -> Result<()> {
    let address: DeviceAddress = host
        .parse()
        .map_err(|_| ExpectedRequestError::invalid_input("Device address is malformed"))?;
    let uri: Uri = address
        .url(DeviceScheme::Https, "/register")?
        .as_str()
        .parse()
        .context("Invalid URL format")?;

    let mut sender = create_https_client(address.host().to_owned(), address.port(), config)
        .await
        .context("Failed to create HTTPS client")?;

    let register_request = RegisterRequest {
        public_key,
        fingerprint,
        alias,
        device_model,
        device_type,
    };

    let json_body =
        serde_json::to_vec(&register_request).context("Failed to serialize register request")?;

    let req = Request::builder()
        .uri(uri)
        .method(Method::POST)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(json_body)))
        .context("Failed to build request")?;

    let response = send_http_request(&mut sender, req)
        .await
        .context("Failed to execute request")?;

    let status = response.status();
    let error_body = response.into_body().collect().await?.to_bytes();

    if status != StatusCode::CREATED {
        if let Ok(error) = serde_json::from_slice::<RegisterErrorResponse>(&error_body) {
            match error.code.as_str() {
                "public_key_mismatch" => {
                    return Err(ExpectedRequestError::PublicKeyMismatch.into());
                }
                "fingerprint_conflict" => {
                    return Err(ExpectedRequestError::RegistrationConflict.into());
                }
                _ => {}
            }
        }
        let error_message = String::from_utf8_lossy(&error_body);
        return Err(anyhow!(
            "Registration failed with status code {}: {}",
            status,
            error_message
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct CheckFingerprintResponse {
    pub is_trusted: bool,
    pub status: String,
    pub message: String,
}

/// Checks if a device fingerprint is trusted by the server
pub async fn check_fingerprint(
    host: &str,
    config: Arc<ClientConfig>,
    fingerprint: &str,
) -> Result<CheckFingerprintResponse> {
    let address: DeviceAddress = host
        .parse()
        .map_err(|_| ExpectedRequestError::invalid_input("Device address is malformed"))?;
    let uri: Uri = address
        .url(
            DeviceScheme::Https,
            &format!("/check-fingerprint?fingerprint={}", encode(fingerprint)),
        )?
        .as_str()
        .parse()
        .context("Invalid URL format")?;

    let mut sender = create_https_client(address.host().to_owned(), address.port(), config)
        .await
        .context("Failed to create HTTPS client")?;

    let req = Request::builder()
        .uri(uri)
        .header("Accept", "application/json")
        .body(Empty::<Bytes>::new())
        .context("Failed to build request")?;

    let res = send_http_request(&mut sender, req)
        .await
        .context("Failed to execute request")?;

    let status = res.status();
    let body = res
        .into_body()
        .collect()
        .await
        .context("Failed to read response body")?
        .to_bytes();

    if status != StatusCode::OK {
        let error_message = String::from_utf8_lossy(&body);
        return Err(anyhow::anyhow!(
            "Fingerprint check failed with status code {}: {}",
            status,
            error_message
        ));
    }

    let response: CheckFingerprintResponse =
        serde_json::from_slice(&body).context("Failed to parse fingerprint check response")?;

    Ok(response)
}
