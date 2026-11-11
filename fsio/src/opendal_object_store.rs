//! OpenDAL-backed object storage for the library's `.rune/sync/` namespace.
use std::{fmt, time::Duration};

use opendal::{services, ErrorKind, HttpTransporter, OperationContext, Operator as AsyncOperator};
use opendal_http_transport_reqwest::ReqwestTransport;
use rustls::{ClientConfig, RootCertStore};
use sync::share::{ConditionalGet, ObjectStore, ObjectTag, ObjectValue, StoreError};

use crate::OpenDalConfig;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObjectStoreCapabilities {
    pub write_probe: bool,
    pub conditional_create: bool,
    pub conditional_put: bool,
    pub conditional_get: bool,
}

#[derive(Clone)]
pub struct OpenDalObjectStore {
    operator: AsyncOperator,
}

impl fmt::Debug for OpenDalObjectStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenDalObjectStore").finish_non_exhaustive()
    }
}

impl OpenDalObjectStore {
    /// Construct a share store rooted below `<backend-root>/.rune/sync/`.
    pub fn new(config: OpenDalConfig) -> Result<Self, StoreError> {
        let http_client = reqwest::Client::builder()
            .tls_backend_preconfigured(
                ClientConfig::builder()
                    .with_root_certificates(RootCertStore {
                        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
                    })
                    .with_no_client_auth(),
            )
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| {
                StoreError::Permanent(format!("configure OpenDAL transport: {error}"))
            })?;
        let transport = HttpTransporter::new(ReqwestTransport::new(http_client));
        let operator = match config {
            OpenDalConfig::WebDav {
                endpoint,
                root,
                username,
                password,
            } => {
                let root = sync_root(&root);
                let builder = services::Webdav::default()
                    .endpoint(&endpoint)
                    .root(&root)
                    .username(&username)
                    .password(&password);
                AsyncOperator::new(builder)
                    .map_err(map_error)?
                    .with_context(OperationContext::new().with_http_transport(transport.clone()))
            }
            OpenDalConfig::S3 {
                endpoint,
                bucket,
                region,
                root,
                access_key_id,
                secret_access_key,
            } => {
                let root = sync_root(&root);
                let builder = services::S3::default()
                    .endpoint(&endpoint)
                    .bucket(&bucket)
                    .region(&region)
                    .root(&root)
                    .access_key_id(&access_key_id)
                    .secret_access_key(&secret_access_key)
                    .disable_config_load();
                AsyncOperator::new(builder)
                    .map_err(map_error)?
                    .with_context(OperationContext::new().with_http_transport(transport.clone()))
            }
        };
        Ok(Self { operator })
    }

    /// Run the required roundtrip and independent conditional-feature probes below this device's prefix.
    pub async fn probe(&self, own_prefix: &str) -> Result<ObjectStoreCapabilities, StoreError> {
        let prefix = validate_device_prefix(own_prefix)?;
        let name = format!("{prefix}.probe-{}", uuid::Uuid::new_v4());
        let bytes = b"rune object-store probe";
        let mut caps = ObjectStoreCapabilities::default();
        let probe_result = async {
            self.put(&name, bytes).await?;
            let value = self.get(&name).await?;
            if value.bytes != bytes {
                return Err(StoreError::Permanent(
                    "object-store probe returned different bytes".into(),
                ));
            }
            let listed = self.list_prefix(&prefix).await?;
            if !listed.iter().any(|key| key == &name) {
                return Err(StoreError::Permanent(
                    "object-store probe object is absent from LIST".into(),
                ));
            }
            self.delete(&name).await?;
            if !matches!(self.get(&name).await, Err(StoreError::NotFound)) {
                return Err(StoreError::Permanent(
                    "object-store probe DELETE did not remove object".into(),
                ));
            }
            Ok(())
        }
        .await;
        if let Err(error) = probe_result {
            let _ = self.delete(&name).await;
            return Err(error);
        }
        caps.write_probe = true;

        let create_key = format!("{prefix}.conditional-create-{}", uuid::Uuid::new_v4());
        if self.create(&create_key, bytes).await.is_ok() {
            caps.conditional_create = matches!(
                self.create(&create_key, bytes).await,
                Err(StoreError::PreconditionFailed)
            );
            let _ = self.delete(&create_key).await;
        }

        let match_key = format!("{prefix}.conditional-put-{}", uuid::Uuid::new_v4());
        if self.put(&match_key, bytes).await.is_ok() {
            if let Ok(original) = self.get(&match_key).await {
                if self
                    .put_if_match(&match_key, &original.tag, b"fresh")
                    .await
                    .is_ok()
                {
                    let stale_fails = matches!(
                        self.put_if_match(&match_key, &original.tag, b"stale").await,
                        Err(StoreError::PreconditionFailed)
                    );
                    caps.conditional_put = stale_fails;
                }
            }
            let _ = self.delete(&match_key).await;
        }

        let get_key = format!("{prefix}.conditional-get-{}", uuid::Uuid::new_v4());
        if self.put(&get_key, bytes).await.is_ok() {
            if let Ok(original) = self.get(&get_key).await {
                match self.get_if_none_match(&get_key, &original.tag).await {
                    Ok(ConditionalGet::NotModified) => caps.conditional_get = true,
                    Ok(ConditionalGet::Modified(_)) => caps.conditional_get = false,
                    Err(StoreError::PreconditionFailed) => caps.conditional_get = true,
                    Err(_) => {}
                }
            }
            let _ = self.delete(&get_key).await;
        }
        Ok(caps)
    }
}

fn sync_root(root: &str) -> String {
    let root = root.trim_matches('/');
    if root.is_empty() {
        ".rune/sync/".into()
    } else {
        format!("{root}/.rune/sync/")
    }
}

fn validate_device_prefix(prefix: &str) -> Result<String, StoreError> {
    let prefix = prefix.trim_matches('/');
    if !prefix.starts_with("devices/")
        || prefix
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(StoreError::Permanent(
            "probe prefix must be a device-owned devices/ prefix".into(),
        ));
    }
    Ok(format!("{prefix}/"))
}

fn object_key(key: &str) -> Result<String, StoreError> {
    if key.is_empty()
        || key.starts_with('/')
        || key
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(StoreError::Permanent("invalid object key".into()));
    }
    Ok(key.to_owned())
}

fn etag(metadata: &opendal::Metadata, _bytes: &[u8]) -> Result<ObjectTag, StoreError> {
    metadata
        .etag()
        .map(|value| ObjectTag::new(value.to_owned()))
        .ok_or_else(|| StoreError::Permanent("OpenDAL response omitted ETag".into()))
}

fn map_error(error: opendal::Error) -> StoreError {
    let detail = format!("{error:#?}");
    if detail.contains("RequestTimeTooSkewed") || error.to_string().contains("RequestTimeTooSkewed")
    {
        return StoreError::ClockSkew;
    }
    if let Some(status) = response_status(&detail) {
        return map_http_status(status, &detail, error.is_temporary());
    }
    match error.kind() {
        ErrorKind::NotFound => StoreError::NotFound,
        ErrorKind::PermissionDenied => StoreError::PermissionDenied,
        ErrorKind::ConditionNotMatch => StoreError::PreconditionFailed,
        ErrorKind::Conflict | ErrorKind::RateLimited | ErrorKind::AlreadyExists => {
            StoreError::Transient(error.to_string())
        }
        ErrorKind::Unexpected
            if error.is_temporary()
                || detail.to_ascii_lowercase().contains("timeout")
                || detail.to_ascii_lowercase().contains("timed out")
                || detail.to_ascii_lowercase().contains("connection refused")
                || detail.to_ascii_lowercase().contains("connection reset") =>
        {
            StoreError::Transient(error.to_string())
        }
        _ => StoreError::Permanent(error.to_string()),
    }
}

fn response_status(detail: &str) -> Option<u16> {
    for marker in ["HTTP/1.1 ", "HTTP/2 ", "status: ", "status=", "status "] {
        let Some((_, suffix)) = detail.split_once(marker) else {
            continue;
        };
        if let Some(code) = suffix.get(..3).and_then(|value| value.parse::<u16>().ok()) {
            return Some(code);
        }
    }
    None
}

fn map_http_status(status: u16, detail: &str, temporary: bool) -> StoreError {
    match status {
        404 => StoreError::NotFound,
        401 => StoreError::Unauthorized,
        403 => StoreError::PermissionDenied,
        412 => StoreError::PreconditionFailed,
        409 | 423 | 500..=599 => StoreError::Transient(detail.to_owned()),
        408 => StoreError::Transient(detail.to_owned()),
        400..=499 => StoreError::Permanent(detail.to_owned()),
        _ if temporary => StoreError::Transient(detail.to_owned()),
        _ => StoreError::Permanent(detail.to_owned()),
    }
}

impl ObjectStore for OpenDalObjectStore {
    fn get<'a>(&'a self, key: &'a str) -> sync::protocol::BoxFuture<'a, ObjectValue, StoreError> {
        Box::pin(async move {
            let key = object_key(key)?;
            let reader = self.operator.reader(&key).await.map_err(map_error)?;
            let bytes = reader.read(..).await.map_err(map_error)?.to_vec();
            let metadata = reader.metadata().cloned().ok_or_else(|| {
                StoreError::Permanent("OpenDAL GET response omitted metadata".into())
            })?;
            let tag = etag(&metadata, &bytes)?;
            Ok(ObjectValue { bytes, tag })
        })
    }

    fn get_if_none_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
    ) -> sync::protocol::BoxFuture<'a, ConditionalGet, StoreError> {
        Box::pin(async move {
            let key = object_key(key)?;
            let reader = match self
                .operator
                .reader_with(&key)
                .if_none_match(tag.as_str())
                .await
            {
                Ok(reader) => reader,
                Err(error) if error.kind() == ErrorKind::ConditionNotMatch => {
                    return Ok(ConditionalGet::NotModified);
                }
                Err(error) => return Err(map_error(error)),
            };
            let bytes = match reader.read(..).await {
                Ok(bytes) => bytes.to_vec(),
                Err(error) if error.kind() == ErrorKind::ConditionNotMatch => {
                    return Ok(ConditionalGet::NotModified);
                }
                Err(error) => return Err(map_error(error)),
            };
            let metadata = reader.metadata().cloned().ok_or_else(|| {
                StoreError::Permanent("OpenDAL GET response omitted metadata".into())
            })?;
            Ok(ConditionalGet::Modified(ObjectValue {
                tag: etag(&metadata, &bytes)?,
                bytes,
            }))
        })
    }

    fn put<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> sync::protocol::BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            let key = object_key(key)?;
            self.operator
                .write(&key, bytes.to_vec())
                .await
                .map_err(map_error)
                .and_then(|metadata| etag(&metadata, bytes))
        })
    }

    fn create<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> sync::protocol::BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            let key = object_key(key)?;
            self.operator
                .write_with(&key, bytes.to_vec())
                .if_not_exists(true)
                .await
                .map_err(map_error)
                .and_then(|metadata| etag(&metadata, bytes))
        })
    }

    fn put_if_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
        bytes: &'a [u8],
    ) -> sync::protocol::BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            let key = object_key(key)?;
            self.operator
                .write_with(&key, bytes.to_vec())
                .if_match(tag.as_str())
                .await
                .map_err(map_error)
                .and_then(|metadata| etag(&metadata, bytes))
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> sync::protocol::BoxFuture<'a, (), StoreError> {
        Box::pin(async move {
            let key = object_key(key)?;
            match self.operator.delete(&key).await {
                Ok(()) => Ok(()),
                Err(error) => match map_error(error) {
                    StoreError::NotFound => Ok(()),
                    error => Err(error),
                },
            }
        })
    }

    fn list_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> sync::protocol::BoxFuture<'a, Vec<String>, StoreError> {
        Box::pin(async move {
            let prefix = if prefix.is_empty() {
                String::new()
            } else {
                format!("{}/", object_key(prefix.trim_end_matches('/'))?)
            };
            self.operator
                .list(&prefix)
                .await
                .map(|entries| {
                    entries
                        .into_iter()
                        .map(|entry| entry.path().to_owned())
                        .collect()
                })
                .map_err(map_error)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{map_error, map_http_status, response_status, sync_root, validate_device_prefix};
    use opendal::{Error, ErrorKind};
    use sync::share::StoreError;

    #[test]
    fn maps_http_statuses_to_store_errors() {
        assert_eq!(map_http_status(404, "404", false), StoreError::NotFound);
        assert_eq!(
            map_http_status(412, "412", false),
            StoreError::PreconditionFailed
        );
        for status in [409, 423, 500, 503, 599, 408] {
            assert!(matches!(
                map_http_status(status, "temporary", false),
                StoreError::Transient(_)
            ));
        }
        assert_eq!(
            map_http_status(401, "unauthorized", false),
            StoreError::Unauthorized
        );
        assert_eq!(
            map_http_status(403, "permission denied", false),
            StoreError::PermissionDenied
        );
        for status in [400, 422] {
            assert!(matches!(
                map_http_status(status, "permanent", false),
                StoreError::Permanent(_)
            ));
        }
        assert!(matches!(
            map_http_status(0, "network", true),
            StoreError::Transient(_)
        ));
    }

    #[test]
    fn maps_opendal_errors_and_s3_clock_skew() {
        assert_eq!(
            map_error(Error::new(ErrorKind::NotFound, "missing")),
            StoreError::NotFound
        );
        assert_eq!(
            map_error(Error::new(ErrorKind::ConditionNotMatch, "condition")),
            StoreError::PreconditionFailed
        );
        assert_eq!(
            map_error(Error::new(ErrorKind::PermissionDenied, "permission denied")),
            StoreError::PermissionDenied
        );
        let skew = Error::new(ErrorKind::Unexpected, "RequestTimeTooSkewed");
        assert_eq!(map_error(skew), StoreError::ClockSkew);
        assert!(matches!(
            map_error(Error::new(ErrorKind::Unexpected, "connection refused")),
            StoreError::Transient(_)
        ));
        assert!(matches!(
            map_error(Error::new(ErrorKind::Unexpected, "request timed out")),
            StoreError::Transient(_)
        ));
        assert!(matches!(
            map_error(Error::new(
                ErrorKind::PermissionDenied,
                "HTTP/1.1 423 Locked"
            )),
            StoreError::Transient(_)
        ));
        assert!(matches!(
            map_error(Error::new(
                ErrorKind::Unexpected,
                "HTTP/1.1 503 unavailable"
            )),
            StoreError::Transient(_)
        ));
        assert!(matches!(
            map_error(Error::new(
                ErrorKind::Unexpected,
                "HTTP/1.1 400 Bad Request"
            )),
            StoreError::Permanent(_)
        ));
    }

    #[test]
    fn recognizes_http_status_and_scopes_share_paths() {
        assert_eq!(
            response_status("response: HTTP/1.1 412 Precondition Failed"),
            Some(412)
        );
        assert_eq!(sync_root("music"), "music/.rune/sync/");
        assert_eq!(sync_root("/"), ".rune/sync/");
        assert_eq!(
            validate_device_prefix("devices/abc.generation").unwrap(),
            "devices/abc.generation/"
        );
        assert!(validate_device_prefix("library.json").is_err());
    }
}
