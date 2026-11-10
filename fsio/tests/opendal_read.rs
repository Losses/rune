use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

use fsio::{FileIoError, FsIo, OpenDalConfig};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::task::{spawn_blocking, JoinHandle};

const AUDIO: &[u8] = b"0123456789abcdef";
const DEADLINE: Duration = Duration::from_secs(10);

struct FixtureServer {
    endpoint: String,
    task: JoinHandle<()>,
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn start_server() -> FixtureServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move { serve(stream).await });
        }
    });
    FixtureServer {
        endpoint: format!("http://{address}"),
        task,
    }
}

async fn serve(stream: TcpStream) {
    let (method, path, headers, stream) = {
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        if reader.read_line(&mut request).await.is_err() {
            return;
        }
        let mut parts = request.split_whitespace();
        let method = parts.next().unwrap_or_default().to_string();
        let path = parts.next().unwrap_or_default().to_string();
        let mut headers = HashMap::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await.is_err() || line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
            }
        }
        let length = headers
            .get("content-length")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        if length > 0 {
            let mut body = vec![0; length];
            if reader.read_exact(&mut body).await.is_err() {
                return;
            }
        }
        (method, path, headers, reader.into_inner())
    };
    let (status, extra, body) = respond(&method, &path, &headers);
    let reason = match status {
        200 => "OK",
        206 => "Partial Content",
        207 => "Multi-Status",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "OK",
    };
    let length_header = extra
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
        .map(str::to_string);
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\n{}Connection: close\r\n{extra}\r\n",
        length_header
            .map(|line| format!("{line}\r\n"))
            .unwrap_or_else(|| format!("Content-Length: {}\r\n", body.len()))
    );
    let mut stream = stream;
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.write_all(&body).await;
    let _ = stream.shutdown().await;
}

fn respond(method: &str, path: &str, headers: &HashMap<String, String>) -> (u16, String, Vec<u8>) {
    if path.starts_with("/dav/") {
        if headers.get("authorization").map(String::as_str) != Some("Basic dXNlcjpwYXNz") {
            return (401, String::new(), Vec::new());
        }
        if method == "PROPFIND" {
            let base = "/dav/library/";
            let nested = format!("{base}nested/");
            let file = format!("{nested}audio.mp3");
            let entries = if path.ends_with("audio.mp3") {
                vec![dav_entry(&file, false, AUDIO.len())]
            } else if path.contains("nested") {
                vec![
                    dav_entry(&nested, true, 0),
                    dav_entry(&file, false, AUDIO.len()),
                ]
            } else {
                vec![dav_entry(base, true, 0), dav_entry(&nested, true, 0)]
            };
            let xml = format!(
                "<?xml version=\"1.0\"?><D:multistatus xmlns:D=\"DAV:\">{}</D:multistatus>",
                entries.join("")
            );
            return (
                207,
                "Content-Type: application/xml\r\n".into(),
                xml.into_bytes(),
            );
        }
        if path.ends_with("audio.mp3") && (method == "GET" || method == "HEAD") {
            return object_response(method, headers, "text/plain");
        }
        return (404, String::new(), Vec::new());
    }

    if method == "GET" && path.contains("list-type=2") {
        let nested = path.contains("nested");
        let prefixes = if nested {
            String::new()
        } else {
            "<CommonPrefixes><Prefix>library/nested/</Prefix></CommonPrefixes>".into()
        };
        let contents = if nested {
            format!("<Contents><Key>library/nested/audio.mp3</Key><LastModified>2025-01-01T00:00:00Z</LastModified><ETag>&quot;tag&quot;</ETag><Size>{}</Size><StorageClass>STANDARD</StorageClass></Contents>", AUDIO.len())
        } else {
            String::new()
        };
        let xml = format!("<?xml version=\"1.0\"?><ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>bucket</Name><Prefix>library/</Prefix><IsTruncated>false</IsTruncated>{prefixes}{contents}</ListBucketResult>");
        return (
            200,
            "Content-Type: application/xml\r\n".into(),
            xml.into_bytes(),
        );
    }
    if path.ends_with("audio.mp3") && (method == "GET" || method == "HEAD") {
        return object_response(method, headers, "application/octet-stream");
    }
    (404, String::new(), Vec::new())
}

fn dav_entry(path: &str, directory: bool, size: usize) -> String {
    let resource = if directory { "<D:collection/>" } else { "" };
    format!("<D:response><D:href>{path}</D:href><D:propstat><D:prop><D:resourcetype>{resource}</D:resourcetype><D:getcontentlength>{size}</D:getcontentlength><D:getlastmodified>Sun, 06 Nov 1994 08:49:37 GMT</D:getlastmodified></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>")
}

fn object_response(
    method: &str,
    headers: &HashMap<String, String>,
    content_type: &str,
) -> (u16, String, Vec<u8>) {
    let modified = "Last-Modified: Wed, 01 Jan 2025 00:00:00 GMT\r\n";
    if method == "HEAD" {
        return (
            200,
            format!(
                "Content-Type: {content_type}\r\nContent-Length: {}\r\n{modified}",
                AUDIO.len()
            ),
            Vec::new(),
        );
    }
    if let Some(range) = headers
        .get("range")
        .and_then(|value| value.strip_prefix("bytes="))
    {
        let mut bounds = range.split('-');
        let start = bounds
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0)
            .min(AUDIO.len());
        let end = bounds
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(AUDIO.len() - 1)
            .min(AUDIO.len() - 1);
        let bytes = AUDIO[start..=end].to_vec();
        return (206, format!("Content-Type: {content_type}\r\n{modified}Content-Range: bytes {start}-{end}/{}\r\n", AUDIO.len()), bytes);
    }
    (
        200,
        format!("Content-Type: {content_type}\r\n{modified}"),
        AUDIO.to_vec(),
    )
}

fn webdav_config(server: &FixtureServer, password: &str) -> OpenDalConfig {
    OpenDalConfig::WebDav {
        endpoint: server.endpoint.clone(),
        root: "/dav/library/".into(),
        username: "user".into(),
        password: password.into(),
    }
}

fn s3_config(server: &FixtureServer) -> OpenDalConfig {
    OpenDalConfig::S3 {
        endpoint: server.endpoint.clone(),
        bucket: "bucket".into(),
        region: "us-east-1".into(),
        root: "/library/".into(),
        access_key_id: "access".into(),
        secret_access_key: "secret".into(),
    }
}

async fn assert_read_contract(config: OpenDalConfig) {
    let fs = FsIo::new_opendal(config, Handle::current()).unwrap();
    let blocking_fs = fs.clone();
    spawn_blocking(move || {
        let files = blocking_fs.walk_dir(Path::new(""), false).unwrap();
        let audio = files
            .iter()
            .find(|node| node.filename == "audio.mp3")
            .unwrap();
        assert!(audio.is_file, "{files:#?}");
        assert_eq!(audio.size, AUDIO.len() as u64);
        assert_eq!(
            blocking_fs.read(Path::new("nested/audio.mp3")).unwrap(),
            AUDIO
        );
        let mut stream = blocking_fs.open(Path::new(&audio.raw_path), "r").unwrap();
        stream.seek(SeekFrom::Start(7)).unwrap();
        let mut slice = [0; 4];
        stream.read_exact(&mut slice).unwrap();
        assert_eq!(&slice, &AUDIO[7..11]);
        assert!(
            blocking_fs
                .modified_time(Path::new("nested/audio.mp3"))
                .unwrap()
                > 0
        );
        assert!(blocking_fs.exists(Path::new("nested/audio.mp3")).unwrap());
        assert_eq!(
            blocking_fs
                .canonicalize_path(Path::new("nested/audio.mp3"))
                .unwrap(),
            Path::new("nested/audio.mp3")
        );
        assert!(
            blocking_fs
                .canonicalize(Path::new("nested/audio.mp3"))
                .unwrap()
                .is_file
        );
    })
    .await
    .unwrap();
    assert!(fs.is_file(Path::new("nested/audio.mp3")).await.unwrap());
    assert!(fs.is_dir(Path::new("nested")).await.unwrap());
    let children = fs.read_dir(Path::new("nested")).await.unwrap();
    assert_eq!(children.len(), 1, "{children:#?}");
    assert_eq!(children[0].filename, "audio.mp3");
}

#[tokio::test]
async fn webdav_read_and_seek_contract() {
    tokio::time::timeout(DEADLINE, async {
        let server = start_server().await;
        assert_read_contract(webdav_config(&server, "pass")).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn s3_read_and_seek_contract() {
    tokio::time::timeout(DEADLINE, async {
        let server = start_server().await;
        assert_read_contract(s3_config(&server)).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn wrong_webdav_password_is_redacted_permission_error() {
    tokio::time::timeout(DEADLINE, async {
        let server = start_server().await;
        let fs =
            FsIo::new_opendal(webdav_config(&server, "wrong-secret"), Handle::current()).unwrap();
        let error = spawn_blocking(move || fs.walk_dir(Path::new(""), false).unwrap_err())
            .await
            .unwrap();
        assert!(
            matches!(error, FileIoError::PermissionDenied(_)),
            "{error:?}"
        );
        assert!(!error.to_string().contains("wrong-secret"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn write_attempts_return_not_supported() {
    tokio::time::timeout(DEADLINE, async {
        let server = start_server().await;
        let fs = FsIo::new_opendal(s3_config(&server), Handle::current()).unwrap();
        assert!(matches!(
            fs.write(Path::new("x"), b"x").await,
            Err(FileIoError::NotSupported(_))
        ));
        assert!(matches!(
            fs.write_string(Path::new("x"), "x").await,
            Err(FileIoError::NotSupported(_))
        ));
        assert!(matches!(
            fs.open(Path::new("x"), "w"),
            Err(FileIoError::NotSupported(_))
        ));
        assert!(matches!(
            fs.create_dir(Path::new(""), "new").await,
            Err(FileIoError::NotSupported(_))
        ));
        assert!(matches!(
            fs.create_dir_all(Path::new("new")),
            Err(FileIoError::NotSupported(_))
        ));
        assert!(matches!(
            fs.remove_file(Path::new("x")).await,
            Err(FileIoError::NotSupported(_))
        ));
        assert!(matches!(
            fs.remove_dir_all(Path::new("x")).await,
            Err(FileIoError::NotSupported(_))
        ));
        assert!(matches!(
            fs.rename_file(Path::new("x"), Path::new("y")).await,
            Err(FileIoError::NotSupported(_))
        ));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn config_debug_redacts_both_secrets() {
    tokio::time::timeout(DEADLINE, async {
        let webdav = format!(
            "{:?}",
            OpenDalConfig::WebDav {
                endpoint: "http://localhost".into(),
                root: "/".into(),
                username: "user".into(),
                password: "dav-secret".into(),
            }
        );
        let s3 = format!(
            "{:?}",
            OpenDalConfig::S3 {
                endpoint: "http://localhost".into(),
                bucket: "bucket".into(),
                region: "region".into(),
                root: "/".into(),
                access_key_id: "access-key".into(),
                secret_access_key: "s3-secret".into(),
            }
        );
        assert!(!webdav.contains("dav-secret"));
        assert!(!s3.contains("s3-secret"));
        assert!(webdav.contains("REDACTED"));
        assert!(s3.contains("REDACTED"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sync_read_from_spawn_blocking_completes() {
    tokio::time::timeout(DEADLINE, async {
        let server = start_server().await;
        let fs = FsIo::new_opendal(s3_config(&server), Handle::current()).unwrap();
        let result = spawn_blocking(move || fs.read(Path::new("nested/audio.mp3")))
            .await
            .unwrap();
        assert_eq!(result.unwrap(), AUDIO);
    })
    .await
    .unwrap();
}
