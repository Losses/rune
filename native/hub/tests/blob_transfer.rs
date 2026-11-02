use axum::{Router, routing::get};
use database::actions::file::{blob_path, hash_file};
use database::sync::model::BlobRef;
use fsio::FsIo;
use hub::server::sync::blobs::download_blob;
use std::sync::Arc;

#[tokio::test]
async fn verifies_audio_and_recovers_after_corrupt_or_interrupted_transfer() {
    let bytes = include_bytes!("../../../assets/startup_0.ogg").to_vec();
    let expected = BlobRef {
        digest: blake3::hash(&bytes).to_hex().to_string(),
        size: bytes.len() as u64,
    };
    let fixture = Arc::new(bytes);
    let serve = |bytes: Vec<u8>| async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/sync/blobs/{digest}",
            get(move || {
                let bytes = bytes.clone();
                async move { bytes }
            }),
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), task)
    };
    let dir = tempfile::tempdir().unwrap();
    let fs = FsIo::new();
    let client = reqwest::Client::new();
    let final_path = blob_path(&fs, dir.path(), &expected.digest, "ogg").unwrap();
    let mut corrupt = fixture.as_ref().clone();
    corrupt[0] ^= 1;
    let (url, task) = serve(corrupt).await;
    assert!(
        download_blob(&fs, &client, &url, dir.path(), &expected, "ogg")
            .await
            .is_err()
    );
    assert!(!final_path.exists());
    task.abort();
    let (url, task) = serve(fixture[..fixture.len() / 2].to_vec()).await;
    assert!(
        download_blob(&fs, &client, &url, dir.path(), &expected, "ogg")
            .await
            .is_err()
    );
    assert!(!final_path.exists());
    task.abort();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let first = fixture[..fixture.len() / 2].to_vec();
    let app = Router::new().route(
        "/sync/blobs/{digest}",
        get(move || {
            let first = first.clone();
            async move {
                let chunks = futures::stream::iter(vec![
                    Ok::<_, std::io::Error>(first),
                    Err(std::io::Error::other("interrupted")),
                ]);
                axum::body::Body::from_stream(chunks)
            }
        }),
    );
    let interrupted = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    assert!(
        download_blob(
            &fs,
            &client,
            &format!("http://{addr}"),
            dir.path(),
            &expected,
            "ogg"
        )
        .await
        .is_err()
    );
    assert!(!final_path.exists());
    interrupted.abort();
    let (url, task) = serve(fixture.as_ref().clone()).await;
    let path = download_blob(&fs, &client, &url, dir.path(), &expected, "ogg")
        .await
        .unwrap();
    assert_eq!(hash_file(&fs, &path).unwrap(), expected);
    assert_eq!(std::fs::read(&path).unwrap(), *fixture);
    let node = fs.canonicalize(&path).unwrap();
    metadata::reader::get_metadata(&fs, &node, None).unwrap();
    task.abort();
}
