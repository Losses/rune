use std::io::{Read, Write};
use std::path::Path;

use fsio::{FileIoError, FsIo};

fn std_fsio_in(temp: &tempfile::TempDir) -> (FsIo, std::path::PathBuf) {
    (FsIo::new(), temp.path().to_path_buf())
}

#[tokio::test]
async fn std_write_and_read_roundtrip() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);
    let file = root.join("hello.bin");

    fsio.write(&file, b"hello world").await.unwrap();
    assert_eq!(fsio.read(&file).unwrap(), b"hello world");

    fsio.write_string(&file, "文本内容").await.unwrap();
    assert_eq!(fsio.read_to_string(&file).unwrap(), "文本内容");
}

#[tokio::test]
async fn std_create_dir_and_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);

    let dir = fsio.create_dir(&root, "sub").await.unwrap();
    assert!(fsio.exists(&dir).unwrap());
    assert!(fsio.is_dir(&dir).await.unwrap());
    assert!(!fsio.is_file(&dir).await.unwrap());

    let nested = root.join("a/b/c");
    fsio.create_dir_all(&nested).unwrap();
    assert!(fsio.is_dir(&nested).await.unwrap());

    assert!(!fsio.exists(&root.join("missing")).unwrap());
}

#[tokio::test]
async fn std_read_dir_lists_entries() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);

    fsio.write(&root.join("a.txt"), b"aaa").await.unwrap();
    fsio.create_dir(&root, "d").await.unwrap();

    let mut entries = fsio.read_dir(&root).await.unwrap();
    entries.sort_by(|a, b| a.filename.cmp(&b.filename));

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].filename, "a.txt");
    assert!(entries[0].is_file);
    assert!(!entries[0].is_dir);
    assert_eq!(entries[0].size, 3);
    assert_eq!(entries[1].filename, "d");
    assert!(entries[1].is_dir);
}

#[tokio::test]
async fn std_open_modes() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);
    let file = root.join("modes.bin");

    {
        let mut w = fsio.open(&file, "w").unwrap();
        w.write_all(b"abc").unwrap();
    }
    {
        let mut a = fsio.open(&file, "a").unwrap();
        a.write_all(b"def").unwrap();
    }
    {
        let mut r = fsio.open(&file, "r").unwrap();
        let mut buf = Vec::new();
        r.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"abcdef");
    }
    {
        let mut w = fsio.open_async(&file, "wt").await.unwrap();
        w.write_all(b"xy").unwrap();
    }
    assert_eq!(fsio.read(&file).unwrap(), b"xy");
}

#[tokio::test]
async fn std_walk_dir_finds_nested_files() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);

    fsio.create_dir_all(&root.join("x/y")).unwrap();
    fsio.write(&root.join("x/y/f1"), b"1").await.unwrap();
    fsio.write(&root.join("f2"), b"2").await.unwrap();

    let nodes = fsio.walk_dir(&root, false).unwrap();
    let names: Vec<_> = nodes.iter().map(|n| n.filename.as_str()).collect();
    assert!(names.contains(&"x"));
    assert!(names.contains(&"y"));
    assert!(names.contains(&"f1"));
    assert!(names.contains(&"f2"));
}

#[tokio::test]
async fn std_ensure_file_and_directory_are_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);

    let file_path = root.join("deep/nested/file.txt");
    let node = fsio.ensure_file(&file_path).await.unwrap();
    assert!(node.is_file);
    let node_again = fsio.ensure_file(&file_path).await.unwrap();
    assert_eq!(node.path, node_again.path);

    let dir_path = root.join("deep/dir");
    let dir = fsio.ensure_directory(&dir_path).await.unwrap();
    assert!(dir.is_dir);
    fsio.ensure_directory(&dir_path).await.unwrap();
}

#[tokio::test]
async fn std_remove_file_and_dir_all() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);

    let dir = root.join("gone");
    fsio.create_dir_all(&dir).unwrap();
    fsio.write(&dir.join("f"), b"x").await.unwrap();

    fsio.remove_file(&dir.join("f")).await.unwrap();
    assert!(!fsio.exists(&dir.join("f")).unwrap());

    fsio.remove_dir_all(&dir).await.unwrap();
    assert!(!fsio.exists(&dir).unwrap());
}

#[tokio::test]
async fn std_canonicalize_returns_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);
    let file = root.join("c.txt");
    fsio.write(&file, b"content").await.unwrap();

    let canonical = fsio.canonicalize_path(&file).unwrap();
    assert!(canonical.is_absolute());
    assert!(canonical.exists());

    let node = fsio.canonicalize(&file).unwrap();
    assert_eq!(node.filename, "c.txt");
    assert!(node.is_file);
    assert_eq!(node.size, 7);

    let node_str = fsio.canonicalize_str(file.to_str().unwrap()).unwrap();
    assert_eq!(node_str.filename, "c.txt");
}

#[tokio::test]
async fn std_errors_on_missing_paths() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);
    let missing = root.join("nope");

    assert!(matches!(fsio.read(&missing), Err(FileIoError::Io(_))));
    assert!(matches!(
        fsio.read_to_string(&missing),
        Err(FileIoError::Io(_))
    ));
    assert!(fsio.open(&missing, "r").is_err());
    assert!(fsio.is_file(&missing).await.is_err());
    assert!(fsio.is_dir(&missing).await.is_err());
    assert!(!fsio.exists(&missing).unwrap());
}

// NoOpFsIo is the remote-mode stub: every operation succeeds and returns
// empty values. These tests pin that contract.

#[tokio::test]
async fn noop_stub_contract() {
    let fsio = FsIo::new_noop();
    let path = Path::new("anything/file.txt");

    assert_eq!(fsio.name(), "NoOp");
    fsio.write(path, b"data").await.unwrap();
    fsio.write_string(path, "data").await.unwrap();
    assert!(fsio.read(path).unwrap().is_empty());
    assert!(fsio.read_to_string(path).unwrap().is_empty());
    assert!(!fsio.exists(path).unwrap());
    assert!(!fsio.is_file(path).await.unwrap());
    assert!(!fsio.is_dir(path).await.unwrap());
    assert!(fsio.read_dir(path).await.unwrap().is_empty());
    assert!(fsio.walk_dir(path, false).unwrap().is_empty());

    fsio.create_dir_all(path).unwrap();
    fsio.remove_file(path).await.unwrap();
    fsio.remove_dir_all(path).await.unwrap();

    let mut stream = fsio.open(path, "rw").unwrap();
    stream.write_all(b"ignored").unwrap();

    let node = fsio.ensure_file(path).await.unwrap();
    assert_eq!(node.filename, "file.txt");
    assert!(!node.is_file);
    assert!(!node.is_dir);
}

#[tokio::test]
async fn std_fs_self_test_all_layers() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);

    fsio.write(&root.join("probe.txt"), b"probe").await.unwrap();

    let mut progress = Vec::new();
    let results = fsio::self_test::run_fs_self_test(&fsio, &root, |r| {
        progress.push(r.clone());
    })
    .await;

    assert_eq!(results.len(), 6);
    assert_eq!(progress.len(), 6);
    for result in &results {
        assert!(
            result.ok || result.skipped,
            "{} failed: {}",
            result.layer,
            result.detail
        );
    }

    let direct_read = results
        .iter()
        .find(|r| r.layer.contains("L6"))
        .expect("L6 result missing");
    assert!(
        direct_read.ok && !direct_read.skipped,
        "L6 should read probe.txt directly: {}",
        direct_read.detail
    );

    assert!(!fsio.exists(&root.join(".rune/.selftest")).unwrap());
}
