use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use urlencoding::encode;

use ::http_request::{
    BodyExt, Bytes, ClientConfig, Empty, Full, Method, Request, StatusCode, Uri,
    create_https_client, send_http_request,
};

use super::utils::device::SanitizedDeviceInfo;

pub async fn fetch_device_info(
    host: &str,
    config: Arc<ClientConfig>,
) -> Result<SanitizedDeviceInfo> {
    let uri = Uri::builder()
        .scheme("https")
        .authority(format!("{host}:7863"))
        .path_and_query("/device-info")
        .build()
        .context("Invalid URL format")?;

    let mut sender = create_https_client(host.to_owned(), 7863, config)
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
    device_type: String,
}

pub async fn register_device(
    host: &str,
    config: Arc<ClientConfig>,
    public_key: String,
    fingerprint: String,
    alias: String,
    device_model: String,
    device_type: String,
) -> Result<()> {
    let uri = Uri::builder()
        .scheme("https")
        .authority(format!("{host}:7863"))
        .path_and_query("/register")
        .build()
        .context("Invalid URL format")?;

    let mut sender = create_https_client(host.to_owned(), 7863, config)
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
    let uri = Uri::builder()
        .scheme("https")
        .authority(format!("{host}:7863"))
        .path_and_query(format!(
            "/check-fingerprint?fingerprint={}",
            encode(fingerprint)
        ))
        .build()
        .context("Invalid URL format")?;

    let mut sender = create_https_client(host.to_owned(), 7863, config)
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
