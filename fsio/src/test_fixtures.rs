//! In-process WebDAV and S3 fixtures shared by filesystem and hub tests.
use std::{
    collections::{BTreeSet, HashMap},
    io::{Error, ErrorKind},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};

type ReadRanges = Arc<Mutex<Vec<Option<(usize, usize)>>>>;

#[derive(Clone)]
pub struct WebDavFixture {
    pub endpoint: String,
    writes: Arc<Mutex<Vec<String>>>,
    ranges: ReadRanges,
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    #[allow(dead_code)]
    reject_reads: Arc<AtomicBool>,
    #[allow(dead_code)]
    reject_writes: Arc<AtomicBool>,
    task: Arc<JoinHandle<()>>,
}

impl WebDavFixture {
    pub async fn start(files: Vec<(String, Vec<u8>)>) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let files = Arc::new(Mutex::new(files.into_iter().collect::<HashMap<_, _>>()));
        let directories = Arc::new(Mutex::new(BTreeSet::new()));
        let writes = Arc::new(Mutex::new(Vec::new()));
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let reject_reads = Arc::new(AtomicBool::new(false));
        let reject_writes = Arc::new(AtomicBool::new(false));
        let server_state = WebDavServerState {
            files: files.clone(),
            directories: directories.clone(),
            writes: writes.clone(),
            ranges: ranges.clone(),
            reject_reads: reject_reads.clone(),
            reject_writes: reject_writes.clone(),
        };
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else {
                            break;
                        };
                        let state = server_state.clone();
                        connections.spawn(async move {
                            serve_webdav(stream, state).await;
                        });
                    }
                    Some(_) = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(Self {
            endpoint: format!("http://{address}"),
            writes,
            ranges,
            files,
            reject_reads,
            reject_writes,
            task: Arc::new(task),
        })
    }

    pub fn write_requests(&self) -> Vec<String> {
        self.writes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    #[allow(dead_code)]
    pub fn read_ranges(&self) -> Vec<Option<(usize, usize)>> {
        self.ranges
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    #[allow(dead_code)]
    pub fn set_file(&self, name: &str, bytes: Vec<u8>) {
        self.files
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(name.to_owned(), bytes);
    }

    #[allow(dead_code)]
    pub async fn start_read_only(files: Vec<(String, Vec<u8>)>) -> std::io::Result<Self> {
        let fixture = Self::start(files).await?;
        fixture.reject_writes.store(true, Ordering::SeqCst);
        Ok(fixture)
    }

    #[allow(dead_code)]
    pub fn reject_reads(&self) {
        self.reject_reads.store(true, Ordering::SeqCst);
    }

    #[allow(dead_code)]
    #[allow(dead_code)]
    pub fn reject_writes(&self) {
        self.reject_writes.store(true, Ordering::SeqCst);
    }
}

impl Drop for WebDavFixture {
    fn drop(&mut self) {
        if Arc::strong_count(&self.task) == 1 {
            self.task.abort();
        }
    }
}

#[derive(Clone)]
pub struct S3Fixture {
    pub endpoint: String,
    #[allow(dead_code)]
    writes: Arc<Mutex<Vec<String>>>,
    #[allow(dead_code)]
    conflict_next_put: Arc<AtomicBool>,
    #[allow(dead_code)]
    conflict_next_conditional_put: Arc<AtomicBool>,
    #[allow(dead_code)]
    versioned: Arc<AtomicBool>,
    #[allow(dead_code)]
    versions: Arc<Mutex<HashMap<String, Vec<Vec<u8>>>>>,
    #[allow(dead_code)]
    reject_writes: Arc<AtomicBool>,
    ranges: ReadRanges,
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    task: Arc<JoinHandle<()>>,
}

#[derive(Clone)]
struct S3ServerState {
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    writes: Arc<Mutex<Vec<String>>>,
    conflict_next_put: Arc<AtomicBool>,
    conflict_next_conditional_put: Arc<AtomicBool>,
    versioned: Arc<AtomicBool>,
    versions: Arc<Mutex<HashMap<String, Vec<Vec<u8>>>>>,
    reject_writes: Arc<AtomicBool>,
    ranges: ReadRanges,
}

impl S3Fixture {
    pub async fn start(files: Vec<(String, Vec<u8>)>) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let files = Arc::new(Mutex::new(files.into_iter().collect::<HashMap<_, _>>()));
        let writes = Arc::new(Mutex::new(Vec::new()));
        let conflict_next_put = Arc::new(AtomicBool::new(false));
        let conflict_next_conditional_put = Arc::new(AtomicBool::new(false));
        let versioned = Arc::new(AtomicBool::new(false));
        let versions = Arc::new(Mutex::new(HashMap::new()));
        let reject_writes = Arc::new(AtomicBool::new(false));
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let server_state = S3ServerState {
            files: files.clone(),
            writes: writes.clone(),
            conflict_next_put: conflict_next_put.clone(),
            conflict_next_conditional_put: conflict_next_conditional_put.clone(),
            versioned: versioned.clone(),
            versions: versions.clone(),
            reject_writes: reject_writes.clone(),
            ranges: ranges.clone(),
        };
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else {
                            break;
                        };
                        let state = server_state.clone();
                        connections.spawn(async move {
                            serve_s3(stream, state).await;
                        });
                    }
                    Some(_) = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(Self {
            endpoint: format!("http://{address}"),
            writes,
            conflict_next_put,
            conflict_next_conditional_put,
            versioned,
            versions,
            reject_writes,
            ranges,
            files,
            task: Arc::new(task),
        })
    }

    /// Make the next S3 PUT fail with 409, as when a conditional request conflicts.
    #[allow(dead_code)]
    pub fn conflict_next_put(&self) {
        self.conflict_next_put.store(true, Ordering::SeqCst);
    }

    /// Make the next conditional S3 PUT fail with 409.
    #[allow(dead_code)]
    pub fn conflict_next_conditional_put(&self) {
        self.conflict_next_conditional_put
            .store(true, Ordering::SeqCst);
    }

    #[allow(dead_code)]
    pub fn conditional_put_conflict_pending(&self) -> bool {
        self.conflict_next_conditional_put.load(Ordering::SeqCst)
    }

    #[allow(dead_code)]
    pub fn enable_versioned_overwrites(&self) {
        self.versioned.store(true, Ordering::SeqCst);
    }

    #[allow(dead_code)]
    pub fn version_count(&self, key: &str) -> usize {
        self.versions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(key)
            .map_or(0, Vec::len)
    }

    #[allow(dead_code)]
    pub fn reject_writes(&self) {
        self.reject_writes.store(true, Ordering::SeqCst);
    }

    #[allow(dead_code)]
    pub async fn start_read_only(files: Vec<(String, Vec<u8>)>) -> std::io::Result<Self> {
        let fixture = Self::start(files).await?;
        fixture.reject_writes.store(true, Ordering::SeqCst);
        Ok(fixture)
    }

    #[allow(dead_code)]
    pub fn write_requests(&self) -> Vec<String> {
        self.writes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    #[allow(dead_code)]
    pub fn read_ranges(&self) -> Vec<Option<(usize, usize)>> {
        self.ranges
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    #[allow(dead_code)]
    pub fn set_file(&self, name: &str, bytes: Vec<u8>) {
        self.files
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(name.to_owned(), bytes);
    }
}

impl Drop for S3Fixture {
    fn drop(&mut self) {
        if Arc::strong_count(&self.task) == 1 {
            self.task.abort();
        }
    }
}

async fn serve_s3(stream: TcpStream, state: S3ServerState) {
    let S3ServerState {
        files,
        writes,
        conflict_next_put,
        conflict_next_conditional_put,
        versioned,
        versions,
        reject_writes,
        ranges,
    } = state;
    let mut reader = BufReader::new(stream);
    loop {
        let (method, path, headers, request_body) = match read_request(&mut reader).await {
            Ok(Some(request)) => request,
            Ok(None) | Err(_) => return,
        };
        let files_snapshot = files
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if matches!(
            method.as_str(),
            "PUT" | "POST" | "PATCH" | "DELETE" | "MKCOL" | "MOVE" | "COPY"
        ) {
            writes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(method.clone());
        }
        let conditional_conflict = method == "PUT"
            && headers.contains_key("if-match")
            && conflict_next_conditional_put.load(Ordering::SeqCst)
            && conflict_next_conditional_put.swap(false, Ordering::SeqCst);
        let (status, content_type, extra, body) = if conditional_conflict
            || method == "PUT" && conflict_next_put.swap(false, Ordering::SeqCst)
        {
            (
                409,
                "application/xml",
                String::new(),
                b"<Error><Code>ConditionalRequestConflict</Code></Error>".to_vec(),
            )
        } else if (method == "PUT" || method == "DELETE") && reject_writes.load(Ordering::SeqCst) {
            (403, "application/xml", String::new(), Vec::new())
        } else if method == "PUT" || method == "DELETE" {
            let name = path.rsplit("/library/").next().unwrap_or_default();
            let current = files_snapshot.get(name);
            let condition_failed = headers
                .get("if-none-match")
                .is_some_and(|value| value == "*")
                && current.is_some()
                || headers.get("if-match").is_some_and(|value| {
                    current.map(|bytes| fixture_etag(bytes)) != Some(value.clone())
                });
            if condition_failed {
                (
                    412,
                    "application/xml",
                    String::new(),
                    b"<Error><Code>PreconditionFailed</Code></Error>".to_vec(),
                )
            } else if method == "PUT" {
                let tag = fixture_etag(&request_body);
                if versioned.load(Ordering::SeqCst) {
                    if let Some(previous) = current {
                        versions
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .entry(name.to_owned())
                            .or_default()
                            .push(previous.clone());
                    }
                }
                files
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .insert(name.to_owned(), request_body);
                (
                    200,
                    "application/xml",
                    format!("ETag: {tag}\r\n"),
                    format!("<PutObjectResult><ETag>{tag}</ETag></PutObjectResult>").into_bytes(),
                )
            } else {
                if files
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(name)
                    .is_some()
                {
                    (204, "text/plain", String::new(), Vec::new())
                } else {
                    (404, "text/plain", String::new(), Vec::new())
                }
            }
        } else if method == "GET" && path.contains("list-type=2") {
            let requested_prefix = path
                .split("prefix=")
                .nth(1)
                .and_then(|value| value.split('&').next())
                .unwrap_or("library/");
            let prefix = requested_prefix
                .replace("%2F", "/")
                .replace("%2f", "/")
                .replace("%3A", ":")
                .replace("%3a", ":");
            let prefix = prefix.trim_start_matches('/');
            let mut contents = String::new();
            let mut prefixes = BTreeSet::new();
            let continuation = path
                .split("continuation-token=")
                .nth(1)
                .and_then(|value| value.split('&').next())
                .unwrap_or_default()
                .replace("%2F", "/")
                .replace("%2f", "/");
            let max_keys = path
                .split("max-keys=")
                .nth(1)
                .and_then(|value| value.split('&').next())
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1000);
            let mut keys = files_snapshot
                .iter()
                .map(|(name, bytes)| (format!("library/{name}"), bytes))
                .filter(|(key, _)| key.starts_with(prefix))
                .collect::<Vec<_>>();
            keys.sort_by(|left, right| left.0.cmp(&right.0));
            if !continuation.is_empty() {
                keys.retain(|(key, _)| key > &continuation);
            }
            let truncated = keys.len() > max_keys;
            let next_token = truncated
                .then(|| {
                    keys.get(max_keys.saturating_sub(1))
                        .map(|(key, _)| key.clone())
                })
                .flatten();
            let continuation_xml = next_token
                .map(|token| format!("<NextContinuationToken>{token}</NextContinuationToken>"))
                .unwrap_or_default();
            keys.truncate(max_keys);
            let mut content_count = 0;
            for (key, bytes) in keys {
                let Some(remainder) = key.strip_prefix(prefix) else {
                    continue;
                };
                if let Some((directory, _)) = remainder.split_once('/') {
                    prefixes.insert(format!("{prefix}{directory}/"));
                } else {
                    content_count += 1;
                    contents.push_str(&format!("<Contents><Key>{key}</Key><LastModified>2025-01-01T00:00:00Z</LastModified><ETag>{}</ETag><Size>{}</Size><StorageClass>STANDARD</StorageClass></Contents>", fixture_etag(bytes), bytes.len()));
                }
            }
            let key_count = content_count + prefixes.len();
            let common_prefixes = prefixes
                .into_iter()
                .map(|prefix| format!("<CommonPrefixes><Prefix>{prefix}</Prefix></CommonPrefixes>"))
                .collect::<String>();
            let xml = format!(
            "<?xml version=\"1.0\"?><ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>bucket</Name><Prefix>{prefix}</Prefix><KeyCount>{}</KeyCount><MaxKeys>{max_keys}</MaxKeys><IsTruncated>{truncated}</IsTruncated>{continuation_xml}{common_prefixes}{contents}</ListBucketResult>",
            key_count
        );
            (200, "application/xml", String::new(), xml.into_bytes())
        } else {
            let name = path.rsplit("/library/").next().unwrap_or_default();
            match files_snapshot.get(name) {
                Some(_bytes) if method == "HEAD" => (
                    200,
                    "application/octet-stream",
                    format!(
                        "Last-Modified: Wed, 01 Jan 2025 00:00:00 GMT\r\nETag: {}\r\n",
                        fixture_etag(_bytes)
                    ),
                    Vec::new(),
                ),
                Some(bytes)
                    if method == "GET"
                        && headers
                            .get("if-none-match")
                            .is_some_and(|tag| tag == &fixture_etag(bytes)) =>
                {
                    (
                        304,
                        "application/octet-stream",
                        format!("ETag: {}\r\n", fixture_etag(bytes)),
                        Vec::new(),
                    )
                }
                Some(bytes) if method == "GET" => {
                    let range = headers
                        .get("range")
                        .and_then(|value| value.strip_prefix("bytes="));
                    let (body, range_header) = if let Some(range) = range {
                        let mut bounds = range.split('-');
                        let start = bounds
                            .next()
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(0)
                            .min(bytes.len());
                        let end = bounds
                            .next()
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(bytes.len().saturating_sub(1))
                            .min(bytes.len().saturating_sub(1));
                        (
                            bytes.get(start..=end).unwrap_or_default().to_vec(),
                            Some((start, end)),
                        )
                    } else {
                        (bytes.clone(), None)
                    };
                    ranges
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .push(range_header);
                    let mut extra = format!(
                        "Last-Modified: Wed, 01 Jan 2025 00:00:00 GMT\r\nETag: {}\r\n",
                        fixture_etag(bytes)
                    );
                    if let Some((start, end)) = range_header {
                        extra.push_str(&format!(
                            "Content-Range: bytes {start}-{end}/{}\r\n",
                            bytes.len()
                        ));
                    }
                    (
                        if range.is_some() { 206 } else { 200 },
                        "application/octet-stream",
                        extra,
                        body,
                    )
                }
                _ => (404, "text/plain", String::new(), Vec::new()),
            }
        };
        let reason = match status {
            200 => "OK",
            206 => "Partial Content",
            204 => "No Content",
            304 => "Not Modified",
            409 => "Conflict",
            412 => "Precondition Failed",
            _ => "Not Found",
        };
        let length = if method == "HEAD" {
            files_snapshot
                .get(path.rsplit("/library/").next().unwrap_or_default())
                .map_or(0, Vec::len)
        } else {
            body.len()
        };
        let close = request_wants_close(&headers);
        let connection = if close { "close" } else { "keep-alive" };
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\n{extra}Connection: {connection}\r\n\r\n"
        );
        if write_response(reader.get_mut(), response.as_bytes(), &body)
            .await
            .is_err()
        {
            return;
        }
        if close {
            let _ = reader.get_mut().shutdown().await;
            return;
        }
    }
}

#[derive(Clone)]
struct WebDavServerState {
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    directories: Arc<Mutex<BTreeSet<String>>>,
    writes: Arc<Mutex<Vec<String>>>,
    ranges: ReadRanges,
    reject_reads: Arc<AtomicBool>,
    reject_writes: Arc<AtomicBool>,
}

async fn serve_webdav(stream: TcpStream, state: WebDavServerState) {
    let WebDavServerState {
        files,
        directories,
        writes,
        ranges,
        reject_reads,
        reject_writes,
    } = state;
    let mut reader = BufReader::new(stream);
    loop {
        let (method, path, headers, request_body) = match read_request(&mut reader).await {
            Ok(Some(request)) => request,
            Ok(None) | Err(_) => return,
        };
        let files_snapshot = files
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if matches!(
            method.as_str(),
            "PUT"
                | "POST"
                | "PATCH"
                | "MKCOL"
                | "DELETE"
                | "MOVE"
                | "COPY"
                | "PROPPATCH"
                | "LOCK"
                | "UNLOCK"
        ) {
            writes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(method.clone());
        }
        let (status, content_type, extra, body) = if reject_writes.load(Ordering::SeqCst)
            && matches!(
                method.as_str(),
                "PUT"
                    | "POST"
                    | "PATCH"
                    | "MKCOL"
                    | "DELETE"
                    | "MOVE"
                    | "COPY"
                    | "PROPPATCH"
                    | "LOCK"
                    | "UNLOCK"
            ) {
            (403, "text/plain", String::new(), Vec::new())
        } else if method == "MKCOL"
            && path.starts_with("/dav/")
            && headers.get("authorization").map(String::as_str) == Some("Basic dXNlcjpwYXNz")
        {
            directories
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(path.trim_end_matches('/').to_owned());
            (201, "text/plain", String::new(), Vec::new())
        } else if !path.starts_with("/dav/library/") {
            (404, "text/plain", String::new(), Vec::new())
        } else if headers.get("authorization").map(String::as_str) != Some("Basic dXNlcjpwYXNz")
            || (method == "GET" && reject_reads.load(Ordering::SeqCst))
        {
            (401, "text/plain", String::new(), Vec::new())
        } else if method == "PUT" || method == "DELETE" {
            let name = path.trim_start_matches("/dav/library/");
            let current = files_snapshot.get(name);
            let condition_failed = headers
                .get("if-none-match")
                .is_some_and(|value| value == "*")
                && current.is_some()
                || headers.get("if-match").is_some_and(|value| {
                    current.map(|bytes| fixture_etag(bytes)) != Some(value.clone())
                });
            if condition_failed {
                (412, "text/plain", String::new(), Vec::new())
            } else if method == "PUT" {
                let tag = fixture_etag(&request_body);
                files
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .insert(name.to_owned(), request_body);
                (201, "text/plain", format!("ETag: {tag}\r\n"), Vec::new())
            } else if files
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(name)
                .is_some()
            {
                (204, "text/plain", String::new(), Vec::new())
            } else {
                (404, "text/plain", String::new(), Vec::new())
            }
        } else if method == "PROPFIND" {
            let directory = path
                .trim_start_matches("/dav/library/")
                .trim_end_matches('/');
            let prefix = if directory.is_empty() {
                String::new()
            } else {
                format!("{directory}/")
            };
            let entries = if files_snapshot.contains_key(directory) {
                files_snapshot
                    .get(directory)
                    .map(|bytes| {
                        vec![dav_entry(
                            &path,
                            false,
                            bytes.len(),
                            Some(fixture_etag(bytes)),
                        )]
                    })
                    .unwrap_or_default()
            } else {
                let has_children = files_snapshot
                    .keys()
                    .any(|name| name.strip_prefix(&prefix).is_some())
                    || directories
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .contains(path.trim_end_matches('/'));
                let mut entries = if has_children {
                    vec![dav_entry(&path, true, 0, None)]
                } else {
                    Vec::new()
                };
                let mut directories = BTreeSet::new();
                for (name, bytes) in files_snapshot.iter() {
                    let Some(remainder) = name.strip_prefix(&prefix) else {
                        continue;
                    };
                    if let Some((child, _)) = remainder.split_once('/') {
                        directories.insert(child.to_owned());
                    } else {
                        entries.push(dav_entry(
                            &format!("/dav/library/{prefix}{remainder}"),
                            false,
                            bytes.len(),
                            Some(fixture_etag(bytes)),
                        ));
                    }
                }
                entries.extend(directories.into_iter().map(|child| {
                    dav_entry(&format!("/dav/library/{prefix}{child}/"), true, 0, None)
                }));
                entries
            };
            let xml = format!(
                "<?xml version=\"1.0\"?><D:multistatus xmlns:D=\"DAV:\">{}</D:multistatus>",
                entries.join("")
            );
            (207, "application/xml", String::new(), xml.into_bytes())
        } else if method == "GET" || method == "HEAD" {
            let name = path.trim_start_matches("/dav/library/");
            match files_snapshot.get(name) {
                Some(_bytes) if method == "HEAD" => (
                    200,
                    "application/octet-stream",
                    format!("ETag: {}\r\n", fixture_etag(_bytes)),
                    Vec::new(),
                ),
                Some(bytes)
                    if method == "GET"
                        && headers
                            .get("if-none-match")
                            .is_some_and(|tag| tag == &fixture_etag(bytes)) =>
                {
                    (
                        304,
                        "application/octet-stream",
                        format!("ETag: {}\r\n", fixture_etag(bytes)),
                        Vec::new(),
                    )
                }
                Some(bytes) => {
                    let range = headers
                        .get("range")
                        .and_then(|value| value.strip_prefix("bytes="));
                    let (body, bounds) = if let Some(range) = range {
                        let mut bounds = range.split('-');
                        let start = bounds
                            .next()
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(0)
                            .min(bytes.len());
                        let end = bounds
                            .next()
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(bytes.len().saturating_sub(1))
                            .min(bytes.len().saturating_sub(1));
                        (
                            bytes.get(start..=end).unwrap_or_default().to_vec(),
                            Some((start, end)),
                        )
                    } else {
                        (bytes.clone(), None)
                    };
                    ranges
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .push(bounds);
                    let mut extra = format!("ETag: {}\r\n", fixture_etag(bytes));
                    if let Some((start, end)) = bounds {
                        extra.push_str(&format!(
                            "Content-Range: bytes {start}-{end}/{}\r\n",
                            bytes.len()
                        ));
                    }
                    (
                        if bounds.is_some() { 206 } else { 200 },
                        "application/octet-stream",
                        extra,
                        body,
                    )
                }
                None => (404, "text/plain", String::new(), Vec::new()),
            }
        } else {
            (405, "text/plain", String::new(), Vec::new())
        };
        let reason = match status {
            200 => "OK",
            206 => "Partial Content",
            207 => "Multi-Status",
            201 => "Created",
            204 => "No Content",
            304 => "Not Modified",
            401 => "Unauthorized",
            400 => "Bad Request",
            404 => "Not Found",
            412 => "Precondition Failed",
            _ => "Method Not Allowed",
        };
        let length = if method == "HEAD" {
            files_snapshot
                .get(path.trim_start_matches("/dav/library/"))
                .map_or(0, Vec::len)
        } else {
            body.len()
        };
        let close = request_wants_close(&headers);
        let connection = if close { "close" } else { "keep-alive" };
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\n{extra}Connection: {connection}\r\n\r\n"
        );
        if write_response(reader.get_mut(), response.as_bytes(), &body)
            .await
            .is_err()
        {
            return;
        }
        if close {
            let _ = reader.get_mut().shutdown().await;
            return;
        }
    }
}

async fn read_request(
    reader: &mut BufReader<TcpStream>,
) -> std::io::Result<Option<(String, String, HashMap<String, String>, Vec<u8>)>> {
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut headers = HashMap::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }

    let is_chunked = headers.get("transfer-encoding").is_some_and(|value| {
        value
            .split(',')
            .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
    });
    let body = if is_chunked {
        read_chunked_body(reader).await?
    } else if let Some(length) = headers
        .get("content-length")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|error| Error::new(ErrorKind::InvalidData, error))?
    {
        let mut body = vec![0; length];
        reader.read_exact(&mut body).await?;
        body
    } else {
        Vec::new()
    };

    Ok(Some((method, path, headers, body)))
}

async fn read_chunked_body(reader: &mut BufReader<TcpStream>) -> std::io::Result<Vec<u8>> {
    let mut buffer = [0; 8192];
    let mut body = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            return Err(Error::new(ErrorKind::UnexpectedEof, "missing chunk size"));
        }
        let size = line
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .trim_end_matches('\r')
            .parse::<usize>()
            .map_err(|error| Error::new(ErrorKind::InvalidData, error))?;
        if size == 0 {
            loop {
                line.clear();
                if reader.read_line(&mut line).await? == 0 || line == "\r\n" {
                    return Ok(body);
                }
            }
        }

        let mut remaining = size;
        while remaining > 0 {
            let count = remaining.min(buffer.len());
            reader.read_exact(&mut buffer[..count]).await?;
            body.extend_from_slice(&buffer[..count]);
            remaining -= count;
        }
        line.clear();
        if reader.read_line(&mut line).await? == 0 || line != "\r\n" {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "invalid chunk terminator",
            ));
        }
    }
}

async fn write_response(
    stream: &mut TcpStream,
    headers: &[u8],
    body: &[u8],
) -> std::io::Result<()> {
    stream.write_all(headers).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    Ok(())
}

fn request_wants_close(headers: &HashMap<String, String>) -> bool {
    headers.get("connection").is_some_and(|value| {
        value
            .split(',')
            .any(|directive| directive.trim().eq_ignore_ascii_case("close"))
    })
}

fn fixture_etag(bytes: &[u8]) -> String {
    format!("\"{}\"", blake3::hash(bytes).to_hex())
}

fn dav_entry(path: &str, directory: bool, size: usize, etag: Option<String>) -> String {
    let resource = if directory { "<D:collection/>" } else { "" };
    let etag = etag.map_or_else(String::new, |tag| format!("<D:getetag>{tag}</D:getetag>"));
    format!(
        "<D:response><D:href>{path}</D:href><D:propstat><D:prop><D:resourcetype>{resource}</D:resourcetype><D:getcontentlength>{size}</D:getcontentlength><D:getlastmodified>Wed, 01 Jan 2025 00:00:00 GMT</D:getlastmodified>{etag}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"
    )
}
