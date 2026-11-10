//! In-process read-only WebDAV fixture shared by filesystem and hub tests.
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

#[derive(Clone)]
pub struct WebDavFixture {
    pub endpoint: String,
    writes: Arc<Mutex<Vec<String>>>,
    task: Arc<JoinHandle<()>>,
}

impl WebDavFixture {
    pub async fn start(files: Vec<(String, Vec<u8>)>) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let files = Arc::new(files.into_iter().collect::<HashMap<_, _>>());
        let writes = Arc::new(Mutex::new(Vec::new()));
        let server_files = files.clone();
        let server_writes = writes.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let files = server_files.clone();
                let writes = server_writes.clone();
                tokio::spawn(async move {
                    serve_webdav(stream, files, writes).await;
                });
            }
        });
        Ok(Self {
            endpoint: format!("http://{address}"),
            writes,
            task: Arc::new(task),
        })
    }

    pub fn write_requests(&self) -> Vec<String> {
        self.writes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
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
    task: Arc<JoinHandle<()>>,
}

impl S3Fixture {
    pub async fn start(files: Vec<(String, Vec<u8>)>) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let files = Arc::new(files.into_iter().collect::<HashMap<_, _>>());
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let files = files.clone();
                tokio::spawn(async move {
                    serve_s3(stream, files).await;
                });
            }
        });
        Ok(Self {
            endpoint: format!("http://{address}"),
            task: Arc::new(task),
        })
    }
}

impl Drop for S3Fixture {
    fn drop(&mut self) {
        if Arc::strong_count(&self.task) == 1 {
            self.task.abort();
        }
    }
}

async fn serve_s3(stream: TcpStream, files: Arc<HashMap<String, Vec<u8>>>) {
    let (method, path, headers, mut stream) = {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line).await.is_err() {
            return;
        }
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_owned();
        let path = parts.next().unwrap_or_default().to_owned();
        let mut headers = HashMap::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await.is_err() || line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
            }
        }
        (method, path, headers, reader.into_inner())
    };
    let (status, content_type, extra, body) = if method == "GET" && path.contains("list-type=2") {
        let requested_prefix = path
            .split("prefix=")
            .nth(1)
            .and_then(|value| value.split('&').next())
            .unwrap_or("library/");
        let prefix = requested_prefix.replace("%2F", "/").replace("%2f", "/");
        let prefix = prefix.trim_start_matches('/');
        let mut contents = String::new();
        let mut prefixes = BTreeSet::new();
        for (name, bytes) in files.iter() {
            let key = format!("library/{name}");
            let Some(remainder) = key.strip_prefix(prefix) else {
                continue;
            };
            if let Some((directory, _)) = remainder.split_once('/') {
                prefixes.insert(format!("{prefix}{directory}/"));
            } else {
                contents.push_str(&format!("<Contents><Key>{key}</Key><LastModified>2025-01-01T00:00:00Z</LastModified><ETag>&quot;fixture&quot;</ETag><Size>{}</Size><StorageClass>STANDARD</StorageClass></Contents>", bytes.len()));
            }
        }
        let common_prefixes = prefixes
            .into_iter()
            .map(|prefix| format!("<CommonPrefixes><Prefix>{prefix}</Prefix></CommonPrefixes>"))
            .collect::<String>();
        let xml = format!("<?xml version=\"1.0\"?><ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>bucket</Name><Prefix>{prefix}</Prefix><IsTruncated>false</IsTruncated>{common_prefixes}{contents}</ListBucketResult>");
        (200, "application/xml", String::new(), xml.into_bytes())
    } else {
        let name = path.rsplit("/library/").next().unwrap_or_default();
        match files.get(name) {
            Some(_bytes) if method == "HEAD" => (
                200,
                "application/octet-stream",
                "Last-Modified: Wed, 01 Jan 2025 00:00:00 GMT\r\n".to_owned(),
                Vec::new(),
            ),
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
                let mut extra = "Last-Modified: Wed, 01 Jan 2025 00:00:00 GMT\r\n".to_owned();
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
        _ => "Not Found",
    };
    let length = if method == "HEAD" {
        files
            .get(path.rsplit("/library/").next().unwrap_or_default())
            .map_or(0, Vec::len)
    } else {
        body.len()
    };
    let response = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\n{extra}Connection: close\r\n\r\n");
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.write_all(&body).await;
    let _ = stream.shutdown().await;
}

async fn serve_webdav(
    stream: TcpStream,
    files: Arc<HashMap<String, Vec<u8>>>,
    writes: Arc<Mutex<Vec<String>>>,
) {
    let (method, path, headers, mut stream) = {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line).await.is_err() {
            return;
        }
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_owned();
        let path = parts.next().unwrap_or_default().to_owned();
        let mut headers = HashMap::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await.is_err() || line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
            }
        }
        let body_length = headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        if body_length > 0 {
            let mut body = vec![0; body_length];
            if reader.read_exact(&mut body).await.is_err() {
                return;
            }
        }
        (method, path, headers, reader.into_inner())
    };
    if matches!(
        method.as_str(),
        "PUT" | "MKCOL" | "DELETE" | "MOVE" | "COPY" | "PROPPATCH"
    ) {
        writes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(method.clone());
    }
    let (status, content_type, extra, body) = if !path.starts_with("/dav/library/") {
        (404, "text/plain", String::new(), Vec::new())
    } else if headers.get("authorization").map(String::as_str) != Some("Basic dXNlcjpwYXNz") {
        (401, "text/plain", String::new(), Vec::new())
    } else if method == "PROPFIND" {
        let directory = path
            .trim_start_matches("/dav/library/")
            .trim_end_matches('/');
        let prefix = if directory.is_empty() {
            String::new()
        } else {
            format!("{directory}/")
        };
        let entries = if files.contains_key(directory) {
            files
                .get(directory)
                .map(|bytes| vec![dav_entry(&path, false, bytes.len())])
                .unwrap_or_default()
        } else {
            let mut entries = vec![dav_entry(&path, true, 0)];
            let mut directories = BTreeSet::new();
            for (name, bytes) in files.iter() {
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
                    ));
                }
            }
            entries.extend(
                directories
                    .into_iter()
                    .map(|child| dav_entry(&format!("/dav/library/{prefix}{child}/"), true, 0)),
            );
            entries
        };
        let xml = format!(
            "<?xml version=\"1.0\"?><D:multistatus xmlns:D=\"DAV:\">{}</D:multistatus>",
            entries.join("")
        );
        (207, "application/xml", String::new(), xml.into_bytes())
    } else if method == "GET" || method == "HEAD" {
        let name = path.trim_start_matches("/dav/library/");
        match files.get(name) {
            Some(_bytes) if method == "HEAD" => {
                (200, "application/octet-stream", String::new(), Vec::new())
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
                let extra = bounds.map_or_else(String::new, |(start, end)| {
                    format!("Content-Range: bytes {start}-{end}/{}\r\n", bytes.len())
                });
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
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Method Not Allowed",
    };
    let length = if method == "HEAD" {
        files
            .get(path.trim_start_matches("/dav/library/"))
            .map_or(0, Vec::len)
    } else {
        body.len()
    };
    let response = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\n{extra}Connection: close\r\n\r\n");
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.write_all(&body).await;
    let _ = stream.shutdown().await;
}

fn dav_entry(path: &str, directory: bool, size: usize) -> String {
    let resource = if directory { "<D:collection/>" } else { "" };
    format!("<D:response><D:href>{path}</D:href><D:propstat><D:prop><D:resourcetype>{resource}</D:resourcetype><D:getcontentlength>{size}</D:getcontentlength><D:getlastmodified>Wed, 01 Jan 2025 00:00:00 GMT</D:getlastmodified></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>")
}
