#![cfg(not(target_os = "android"))]

use fsio::FsIo;

#[tokio::test]
async fn verified_file_can_be_published_by_same_directory_rename() {
    let root = tempfile::tempdir().unwrap();
    let fs = FsIo::new();
    let partial = root.path().join("audio.part");
    let published = root.path().join("audio.ogg");
    fs.write(&partial, b"verified content").await.unwrap();
    fs.open(&partial, "rw").unwrap().sync_all().unwrap();
    fs.rename_file(&partial, &published).await.unwrap();
    assert!(!fs.exists(&partial).unwrap());
    assert_eq!(fs.read(&published).unwrap(), b"verified content");
    let other = root.path().join("other");
    fs.create_dir_all(&other).unwrap();
    assert!(fs
        .rename_file(&published, &other.join("audio.ogg"))
        .await
        .is_err());
    assert!(fs
        .rename_file(&other, &root.path().join("moved"))
        .await
        .is_err());
    assert_eq!(fs.read(&published).unwrap(), b"verified content");
}
