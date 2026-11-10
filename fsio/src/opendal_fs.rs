use std::collections::HashSet;
use std::fmt;
use std::io::{Error as IoError, ErrorKind as IoErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use opendal::blocking::{Operator, Reader};
use opendal::services;
use opendal::{ErrorKind, HttpTransporter, OperationContext, Operator as AsyncOperator};
use opendal_http_transport_reqwest::ReqwestTransport;
use rustls::{ClientConfig, RootCertStore};
use tokio::runtime::Handle;

use crate::{FileIo, FileIoError, FileStream, FsNode};

/// Credentials and location for a read-only OpenDAL filesystem.
#[derive(Clone)]
pub enum OpenDalConfig {
    WebDav {
        endpoint: String,
        root: String,
        username: String,
        password: String,
    },
    S3 {
        endpoint: String,
        bucket: String,
        region: String,
        root: String,
        access_key_id: String,
        secret_access_key: String,
    },
}

impl fmt::Debug for OpenDalConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WebDav {
                endpoint,
                root,
                username,
                ..
            } => f
                .debug_struct("WebDav")
                .field("endpoint", endpoint)
                .field("root", root)
                .field("username", username)
                .field("password", &"[REDACTED]")
                .finish(),
            Self::S3 {
                endpoint,
                bucket,
                region,
                root,
                ..
            } => f
                .debug_struct("S3")
                .field("endpoint", endpoint)
                .field("bucket", bucket)
                .field("region", region)
                .field("root", root)
                .field("access_key_id", &"[REDACTED]")
                .field("secret_access_key", &"[REDACTED]")
                .finish(),
        }
    }
}

/// Read-only remote filesystem. Sync calls use OpenDAL's blocking facade on the
/// supplied runtime; callers must construct with a live Tokio handle. Calls from
/// Tokio workers should be moved to `spawn_blocking` so they never block a worker.
#[derive(Clone)]
pub(crate) struct OpenDalFsIo {
    operator: Operator,
    handle: Handle,
}

impl OpenDalFsIo {
    pub(crate) fn new(config: OpenDalConfig, handle: Handle) -> Result<Self, FileIoError> {
        let http_client = {
            let _guard = handle.enter();
            // Same trust roots as the rest of the workspace (reqwest `rustls-tls` uses
            // webpki-roots); a platform verifier would need JNI setup on Android.
            let roots = RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            };
            let tls_config = ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            reqwest::Client::builder()
                .tls_backend_preconfigured(tls_config)
                .build()
                .map_err(|_| remote_error("configure"))?
        };
        let http_transport = HttpTransporter::new(ReqwestTransport::new(http_client));
        let op = match config {
            OpenDalConfig::WebDav {
                endpoint,
                root,
                username,
                password,
            } => {
                let builder = services::Webdav::default()
                    .endpoint(&endpoint)
                    .root(&root)
                    .username(&username)
                    .password(&password);
                let _guard = handle.enter();
                AsyncOperator::new(builder)
                    .map_err(|_| remote_error("configure"))?
                    .with_context(OperationContext::new().with_http_transport(http_transport))
            }
            OpenDalConfig::S3 {
                endpoint,
                bucket,
                region,
                root,
                access_key_id,
                secret_access_key,
            } => {
                let builder = services::S3::default()
                    .endpoint(&endpoint)
                    .bucket(&bucket)
                    .region(&region)
                    .root(&root)
                    .access_key_id(&access_key_id)
                    .secret_access_key(&secret_access_key)
                    .disable_config_load();
                let _guard = handle.enter();
                AsyncOperator::new(builder)
                    .map_err(|_| remote_error("configure"))?
                    .with_context(OperationContext::new().with_http_transport(http_transport))
            }
        };
        let _guard = handle.enter();
        let operator = Operator::new(op).map_err(|_| remote_error("configure"))?;
        Ok(Self { operator, handle })
    }

    fn enter(&self) -> tokio::runtime::EnterGuard<'_> {
        self.handle.enter()
    }

    fn key(&self, path: &Path) -> Result<String, FileIoError> {
        let mut parts = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(value) => {
                    parts.push(value.to_str().ok_or(FileIoError::InvalidPath)?)
                }
                Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) | Component::ParentDir => {
                    return Err(FileIoError::InvalidPath)
                }
            }
        }
        let relative = parts.join("/");
        Ok(relative)
    }

    fn run<T>(
        &self,
        operation: &str,
        f: impl FnOnce(&Operator) -> opendal::Result<T>,
    ) -> Result<T, FileIoError> {
        let _guard = self.enter();
        f(&self.operator).map_err(|error| map_open_error(&error, operation))
    }

    fn node(&self, key: &str, path: PathBuf, filename: String) -> Result<FsNode, FileIoError> {
        let meta = self.run("stat", |op| op.stat(key))?;
        Ok(FsNode {
            filename,
            path,
            raw_path: path_string(key),
            is_dir: meta.is_dir(),
            is_file: meta.is_file(),
            size: meta.content_length(),
        })
    }

    fn listed_node(entry: &opendal::Entry) -> FsNode {
        let key = entry.path();
        let metadata = entry.metadata();
        let filename = key
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string();
        FsNode {
            filename,
            path: PathBuf::from(key),
            raw_path: path_string(key),
            is_dir: metadata.is_dir(),
            is_file: metadata.is_file(),
            size: metadata.content_length(),
        }
    }

    fn directory_exists(&self, key: &str) -> Result<bool, FileIoError> {
        let prefix = directory_key(key);
        self.run("list", |op| op.list(&prefix))
            .map(|entries| !entries.is_empty())
    }

    fn metadata_node(
        &self,
        key: &str,
        path: PathBuf,
        filename: String,
    ) -> Result<FsNode, FileIoError> {
        match self.node(key, path.clone(), filename.clone()) {
            Ok(node) => Ok(node),
            Err(FileIoError::PathNotFound(_)) if self.directory_exists(key)? => Ok(FsNode {
                filename,
                path,
                raw_path: path_string(key),
                is_dir: true,
                is_file: false,
                size: 0,
            }),
            Err(error) => Err(error),
        }
    }

    fn list_recursive(
        &self,
        key: &str,
        visited: &mut HashSet<String>,
        nodes: &mut Vec<FsNode>,
    ) -> Result<(), FileIoError> {
        let key = directory_key(key);
        if !visited.insert(key.clone()) {
            return Ok(());
        }
        let entries = self.run("list", |op| op.list(&key))?;
        let current = key.trim_matches('/');
        let mut seen = HashSet::new();
        for entry in entries {
            let entry_key = entry.path().to_string();
            let normalized = entry_key.trim_matches('/');
            if normalized == current || !seen.insert(normalized.to_string()) {
                continue;
            }
            let node = Self::listed_node(&entry);
            let directory = node.is_dir;
            nodes.push(node);
            if directory {
                self.list_recursive(&entry_key, visited, nodes)?;
            }
        }
        Ok(())
    }

    fn list_one(&self, key: &str) -> Result<Vec<FsNode>, FileIoError> {
        let key = directory_key(key);
        let entries = self.run("list", |op| op.list(&key))?;
        let current = key.trim_matches('/');
        let mut seen = HashSet::new();
        let mut nodes = Vec::new();
        for entry in entries {
            let entry_key = entry.path().to_string();
            let normalized = entry_key.trim_matches('/');
            if normalized == current || !seen.insert(normalized.to_string()) {
                continue;
            }
            nodes.push(Self::listed_node(&entry));
        }
        Ok(nodes)
    }
}

fn directory_key(key: &str) -> String {
    if key.is_empty() || key.ends_with('/') {
        key.to_string()
    } else {
        format!("{key}/")
    }
}

fn path_string(key: &str) -> String {
    key.to_string()
}

fn remote_error(operation: &str) -> FileIoError {
    FileIoError::Remote(format!("{operation} failed"))
}

pub fn classify_open_error(error: &opendal::Error) -> OpenDalErrorKind {
    if error.kind() == ErrorKind::PermissionDenied || error.to_string().contains("401") {
        OpenDalErrorKind::Authentication
    } else if matches!(error.kind(), ErrorKind::NotFound) {
        OpenDalErrorKind::NotFound
    } else {
        OpenDalErrorKind::Unreachable
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenDalErrorKind {
    Authentication,
    NotFound,
    Unreachable,
}

fn map_open_error(error: &opendal::Error, operation: &str) -> FileIoError {
    match classify_open_error(error) {
        OpenDalErrorKind::Authentication => FileIoError::PermissionDenied(operation.to_string()),
        OpenDalErrorKind::NotFound => FileIoError::PathNotFound(operation.to_string()),
        OpenDalErrorKind::Unreachable => remote_error(operation),
    }
}

struct RemoteFile {
    reader: Reader,
    position: u64,
    length: u64,
}

impl Read for RemoteFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() || self.position >= self.length {
            return Ok(0);
        }
        let end = self
            .position
            .saturating_add(buf.len().min(64 * 1024) as u64)
            .min(self.length);
        let bytes = self
            .reader
            .read(self.position..end)
            .map_err(|_| IoError::other("remote read failed"))?;
        let bytes = bytes.to_vec();
        let count = bytes.len().min(buf.len());
        buf[..count].copy_from_slice(&bytes[..count]);
        self.position += count as u64;
        Ok(count)
    }
}
impl Seek for RemoteFile {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let next = match pos {
            SeekFrom::Start(offset) => offset as i128,
            SeekFrom::Current(offset) => self.position as i128 + offset as i128,
            SeekFrom::End(offset) => self.length as i128 + offset as i128,
        };
        if next < 0 || next > u64::MAX as i128 {
            return Err(IoError::new(
                std::io::ErrorKind::InvalidInput,
                "invalid seek",
            ));
        }
        self.position = next as u64;
        Ok(self.position)
    }
}
impl Write for RemoteFile {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(IoError::new(IoErrorKind::Unsupported, "read-only"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl FileStream for RemoteFile {
    fn sync_all(&mut self) -> Result<(), FileIoError> {
        Err(FileIoError::NotSupported("sync remote file".into()))
    }
}

#[async_trait]
impl FileIo for OpenDalFsIo {
    fn name(&self) -> &'static str {
        "OpenDAL"
    }

    fn open(&self, path: &Path, open_mode: &str) -> Result<Box<dyn FileStream>, FileIoError> {
        if open_mode.is_empty() || open_mode != "r" {
            return Err(FileIoError::NotSupported("remote write access".into()));
        }
        let key = self.key(path)?;
        let (reader, length) = self.run("open", |op| {
            Ok((op.reader(&key)?, op.stat(&key)?.content_length()))
        })?;
        Ok(Box::new(RemoteFile {
            reader,
            position: 0,
            length,
        }))
    }

    async fn open_async(
        &self,
        path: &Path,
        open_mode: &str,
    ) -> Result<Box<dyn FileStream>, FileIoError> {
        let this = self.clone();
        let path = path.to_path_buf();
        let mode = open_mode.to_string();
        tokio::task::spawn_blocking(move || this.open(&path, &mode))
            .await
            .map_err(|_| remote_error("open"))?
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>, FileIoError> {
        let key = self.key(path)?;
        self.run("read", |op| op.read(&key)).map(|b| b.to_vec())
    }
    fn read_to_string(&self, path: &Path) -> Result<String, FileIoError> {
        String::from_utf8(self.read(path)?).map_err(|_| FileIoError::InvalidPath)
    }
    async fn write(&self, _path: &Path, _contents: &[u8]) -> Result<(), FileIoError> {
        Err(FileIoError::NotSupported("remote write".into()))
    }
    async fn write_string(&self, _path: &Path, _contents: &str) -> Result<(), FileIoError> {
        Err(FileIoError::NotSupported("remote write".into()))
    }
    async fn create_dir(&self, _parent: &Path, _name: &str) -> Result<PathBuf, FileIoError> {
        Err(FileIoError::NotSupported("remote create directory".into()))
    }
    fn create_dir_all(&self, _path: &Path) -> Result<(), FileIoError> {
        Err(FileIoError::NotSupported("remote create directory".into()))
    }

    async fn read_dir(&self, path: &Path) -> Result<Vec<FsNode>, FileIoError> {
        let this = self.clone();
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let key = this.key(&path)?;
            this.list_one(&key)
        })
        .await
        .map_err(|_| remote_error("list"))?
    }
    async fn remove_file(&self, _path: &Path) -> Result<(), FileIoError> {
        Err(FileIoError::NotSupported("remote delete".into()))
    }
    async fn rename_file(&self, _from: &Path, _to: &Path) -> Result<(), FileIoError> {
        Err(FileIoError::NotSupported("remote rename".into()))
    }
    async fn remove_dir_all(&self, _path: &Path) -> Result<(), FileIoError> {
        Err(FileIoError::NotSupported("remote delete".into()))
    }

    fn walk_dir(&self, path: &Path, _follow_links: bool) -> Result<Vec<FsNode>, FileIoError> {
        let key = self.key(path)?;
        let mut nodes = Vec::new();
        self.list_recursive(&key, &mut HashSet::new(), &mut nodes)?;
        Ok(nodes)
    }

    fn exists(&self, path: &Path) -> Result<bool, FileIoError> {
        let key = self.key(path)?;
        if self.run("exists", |op| op.exists(&key))? {
            Ok(true)
        } else {
            self.directory_exists(&key)
        }
    }
    async fn is_file(&self, path: &Path) -> Result<bool, FileIoError> {
        let this = self.clone();
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let key = this.key(&path)?;
            this.metadata_node(&key, PathBuf::from(&key), String::new())
                .map(|node| node.is_file)
        })
        .await
        .map_err(|_| remote_error("stat"))?
    }
    async fn is_dir(&self, path: &Path) -> Result<bool, FileIoError> {
        let this = self.clone();
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let key = this.key(&path)?;
            this.metadata_node(&key, PathBuf::from(&key), String::new())
                .map(|node| node.is_dir)
        })
        .await
        .map_err(|_| remote_error("stat"))?
    }
    fn canonicalize_path(&self, path: &Path) -> Result<PathBuf, FileIoError> {
        let key = self.key(path)?;
        self.metadata_node(&key, PathBuf::from(&key), String::new())?;
        Ok(PathBuf::from(key))
    }
    fn canonicalize_path_str(&self, path: &str) -> Result<PathBuf, FileIoError> {
        self.canonicalize_path(Path::new(path))
    }
    fn canonicalize(&self, path: &Path) -> Result<FsNode, FileIoError> {
        let key = self.key(path)?;
        let filename = path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or_default()
            .to_string();
        self.metadata_node(&key, PathBuf::from(&key), filename)
    }
    fn canonicalize_str(&self, path: &str) -> Result<FsNode, FileIoError> {
        self.canonicalize(Path::new(path))
    }
    fn modified_time(&self, path: &Path) -> Result<u64, FileIoError> {
        let key = self.key(path)?;
        self.run("stat", |op| op.stat(&key)).map(|meta| {
            meta.last_modified()
                .map(SystemTime::from)
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or_default()
        })
    }
    async fn ensure_file(&self, path: &Path) -> Result<FsNode, FileIoError> {
        let this = self.clone();
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || this.canonicalize(&path))
            .await
            .map_err(|_| remote_error("stat"))?
    }
    async fn ensure_directory(&self, path: &Path) -> Result<FsNode, FileIoError> {
        let this = self.clone();
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || this.canonicalize(&path))
            .await
            .map_err(|_| remote_error("stat"))?
    }
}
