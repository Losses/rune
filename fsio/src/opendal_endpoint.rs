use discovery::endpoint::EndpointUrl;
use opendal::raw::percent_decode_path;
use reqwest::Url;
use thiserror::Error;

use crate::OpenDalConfig;

/// A parsed endpoint cannot configure one of Rune's OpenDAL services.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum OpenDalEndpointError {
    /// The endpoint names a different kind of service.
    #[error("endpoint is not OpenDAL storage")]
    NotStorage,
    /// Rune has no storage adapter for the named OpenDAL service.
    #[error("unsupported OpenDAL service: {0}")]
    UnsupportedService(String),
    /// The storage adapter does not recognize this setting.
    #[error("unknown {service} parameter: {parameter}")]
    UnknownParameter {
        service: &'static str,
        parameter: String,
    },
    /// A setting is required and missing, or its value is invalid.
    #[error("invalid {service} parameter: {parameter}")]
    InvalidParameter {
        service: &'static str,
        parameter: &'static str,
    },
}

impl OpenDalConfig {
    /// Validate an exchange endpoint against the storage services Rune supports.
    /// Pass the account name and credential separately from the address.
    pub fn from_endpoint(
        endpoint: &EndpointUrl,
        account: String,
        secret: String,
    ) -> Result<Self, OpenDalEndpointError> {
        let EndpointUrl::OpenDal {
            service,
            location,
            params,
        } = endpoint
        else {
            return Err(OpenDalEndpointError::NotStorage);
        };
        let (service, allowed) = match service.as_str() {
            "s3" => ("s3", ["endpoint", "region"]),
            "webdav" => ("webdav", ["root", "transport"]),
            _ => return Err(OpenDalEndpointError::UnsupportedService(service.clone())),
        };
        if let Some(parameter) = params.keys().find(|key| !allowed.contains(&key.as_str())) {
            return Err(OpenDalEndpointError::UnknownParameter {
                service,
                parameter: parameter.clone(),
            });
        }
        let invalid = |parameter| OpenDalEndpointError::InvalidParameter { service, parameter };

        if service == "s3" {
            let (bucket, root) = location.split_once('/').unwrap_or((location, ""));
            let bucket = percent_decode_path(bucket);
            if bucket.is_empty() || bucket.contains(['@', ':', '/']) {
                return Err(invalid("bucket"));
            }
            let region = params
                .get("region")
                .filter(|region| !region.is_empty() && region.trim() == region.as_str())
                .ok_or_else(|| invalid("region"))?;
            let endpoint = params
                .get("endpoint")
                .map(|value| storage_http_url(value).ok_or_else(|| invalid("endpoint")))
                .transpose()?
                .unwrap_or_default();
            Ok(Self::S3 {
                endpoint,
                bucket,
                region: region.clone(),
                root: percent_decode_path(root),
                access_key_id: account,
                secret_access_key: secret,
            })
        } else {
            let transport = params.get("transport").map_or("https", String::as_str);
            if !matches!(transport, "http" | "https") {
                return Err(invalid("transport"));
            }
            let endpoint = storage_http_url(&format!("{transport}://{location}"))
                .ok_or_else(|| invalid("endpoint"))?;
            Ok(Self::WebDav {
                endpoint,
                root: params.get("root").cloned().unwrap_or_default(),
                username: account,
                password: secret,
            })
        }
    }
}

fn storage_http_url(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    // The WebDAV service appends an absolute storage path to this base.
    Some(url.as_str().trim_end_matches('/').to_owned())
}
