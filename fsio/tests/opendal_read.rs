use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

use fsio::{FileIoError, FsIo, OpenDalConfig};
use tokio::runtime::Handle;
use tokio::task::spawn_blocking;

#[path = "../src/test_fixtures.rs"]
mod test_fixtures;
use test_fixtures::{S3Fixture, WebDavFixture};

const AUDIO: &[u8] = b"0123456789abcdef";
const DEADLINE: Duration = Duration::from_secs(60);

fn webdav_config(server: &WebDavFixture, password: &str) -> OpenDalConfig {
    OpenDalConfig::WebDav {
        endpoint: server.endpoint.clone(),
        root: "/dav/library/".into(),
        username: "user".into(),
        password: password.into(),
    }
}

fn s3_config(server: &S3Fixture) -> OpenDalConfig {
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
        let server = WebDavFixture::start(vec![("nested/audio.mp3".into(), AUDIO.to_vec())])
            .await
            .unwrap();
        assert_read_contract(webdav_config(&server, "pass")).await;
        assert!(server.write_requests().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn s3_read_and_seek_contract() {
    tokio::time::timeout(DEADLINE, async {
        let server = S3Fixture::start(vec![("nested/audio.mp3".into(), AUDIO.to_vec())])
            .await
            .unwrap();
        assert_read_contract(s3_config(&server)).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn wrong_webdav_password_is_redacted_permission_error() {
    tokio::time::timeout(DEADLINE, async {
        let server = WebDavFixture::start(vec![("nested/audio.mp3".into(), AUDIO.to_vec())])
            .await
            .unwrap();
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
        let server = S3Fixture::start(vec![("nested/audio.mp3".into(), AUDIO.to_vec())])
            .await
            .unwrap();
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
        let server = S3Fixture::start(vec![("nested/audio.mp3".into(), AUDIO.to_vec())])
            .await
            .unwrap();
        let fs = FsIo::new_opendal(s3_config(&server), Handle::current()).unwrap();
        let result = spawn_blocking(move || fs.read(Path::new("nested/audio.mp3")))
            .await
            .unwrap();
        assert_eq!(result.unwrap(), AUDIO);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn large_remote_reads_and_end_seek_use_ranges() {
    tokio::time::timeout(DEADLINE, async {
        let bytes = vec![0x5a; 64 * 1024 * 1024];
        let webdav = WebDavFixture::start(vec![("large.bin".into(), bytes.clone())])
            .await
            .unwrap();
        let s3 = S3Fixture::start(vec![("large.bin".into(), bytes)])
            .await
            .unwrap();
        let webdav_fs =
            FsIo::new_opendal(webdav_config(&webdav, "pass"), Handle::current()).unwrap();
        let s3_fs = FsIo::new_opendal(s3_config(&s3), Handle::current()).unwrap();
        for fs in [webdav_fs, s3_fs] {
            let blocking_fs = fs.clone();
            spawn_blocking(move || {
                let mut stream = blocking_fs.open(Path::new("large.bin"), "r").unwrap();
                stream.seek(SeekFrom::End(-8)).unwrap();
                let mut tail = [0; 8];
                stream.read_exact(&mut tail).unwrap();
                assert_eq!(tail, [0x5a; 8]);
            })
            .await
            .unwrap();
        }
        for ranges in [webdav.read_ranges(), s3.read_ranges()] {
            assert!(!ranges.is_empty(), "expected ranged GETs");
            assert!(
                ranges.iter().all(Option::is_some),
                "full object GET observed: {ranges:?}"
            );
            assert!(
                ranges
                    .iter()
                    .flatten()
                    .any(|range| *range == (64 * 1024 * 1024 - 8, 64 * 1024 * 1024 - 1)),
                "unexpected end-seek ranges: {ranges:?}"
            );
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn paged_directory_listing_returns_every_entry() {
    tokio::time::timeout(DEADLINE, async {
        let files: Vec<_> = (0..11)
            .map(|index| (format!("track-{index:02}.ogg"), vec![index]))
            .collect();
        let webdav = WebDavFixture::start(files.clone()).await.unwrap();
        let s3 = S3Fixture::start(files).await.unwrap();
        for config in [webdav_config(&webdav, "pass"), s3_config(&s3)] {
            let fs = FsIo::new_opendal(config, Handle::current()).unwrap();
            let entries = fs.read_dir(Path::new("")).await.unwrap();
            assert_eq!(entries.len(), 11, "listing truncated: {entries:#?}");
        }
    })
    .await
    .unwrap();
}
