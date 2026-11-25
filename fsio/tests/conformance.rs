use std::io::{ErrorKind, Read, Write};
use std::path::Path;

use fsio::self_test;
use fsio::{DirEntryDetails, EntryKind, FileIoError, FsIo, FsNode, OpenMode, StorageLocation};
use std::fs;
use std::path as std_path;

fn std_fsio_in(temp: &tempfile::TempDir) -> (FsIo, std_path::PathBuf) {
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
        let mut w = fsio.open(&file, OpenMode::Write).unwrap();
        w.write_all(b"abc").unwrap();
    }
    {
        let mut a = fsio.open(&file, OpenMode::Append).unwrap();
        a.write_all(b"def").unwrap();
    }
    {
        let mut r = fsio.open(&file, OpenMode::Read).unwrap();
        let mut buf = Vec::new();
        r.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"abcdef");
    }
    {
        let mut w = fsio
            .open_async(&file, OpenMode::WriteTruncate)
            .await
            .unwrap();
        w.write_all(b"xy").unwrap();
    }
    assert_eq!(fsio.read(&file).unwrap(), b"xy");
}

#[test]
fn local_backends_report_local_storage() {
    assert_eq!(FsIo::new().storage(), StorageLocation::Local);
    assert_eq!(FsIo::new_noop().storage(), StorageLocation::Local);
}

#[test]
fn std_open_modes_keep_their_meaning() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);
    let file = root.join("kept.bin");

    // Only a read of a missing file fails: every mode that writes creates it.
    assert!(fsio.open(&file, OpenMode::Read).is_err());
    for mode in [
        OpenMode::Write,
        OpenMode::WriteTruncate,
        OpenMode::Append,
        OpenMode::ReadWrite,
    ] {
        let missing = root.join(format!("created-{}.bin", mode.as_str()));
        assert!(fsio.open(&missing, mode).is_ok(), "{mode:?} did not create");
        assert!(fsio.exists(&missing).unwrap());
    }

    fsio.open(&file, OpenMode::Write)
        .unwrap()
        .write_all(b"abc")
        .unwrap();
    // Writing without truncating keeps what follows the written bytes.
    fsio.open(&file, OpenMode::Write)
        .unwrap()
        .write_all(b"x")
        .unwrap();
    assert_eq!(fsio.read(&file).unwrap(), b"xbc");
    {
        let mut both = fsio.open(&file, OpenMode::ReadWrite).unwrap();
        let mut first = [0u8; 1];
        both.read_exact(&mut first).unwrap();
        assert_eq!(&first, b"x");
    }
    fsio.open(&file, OpenMode::WriteTruncate).unwrap();
    assert_eq!(fsio.read(&file).unwrap(), b"");
}

#[tokio::test]
async fn std_walk_dir_finds_nested_files() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);

    fsio.create_dir_all(&root.join("x/y")).unwrap();
    fsio.write(&root.join("x/y/f1"), b"1").await.unwrap();
    fsio.write(&root.join("f2"), b"2").await.unwrap();

    let nodes = fsio.walk_dir(&root, false).unwrap().nodes;
    let names: Vec<_> = nodes.iter().map(|n| n.filename.as_str()).collect();
    assert!(names.contains(&"x"));
    assert!(names.contains(&"y"));
    assert!(names.contains(&"f1"));
    assert!(names.contains(&"f2"));
}

#[tokio::test]
async fn std_walk_dir_of_an_empty_root_lists_only_the_root() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);

    let walk = fsio.walk_dir(&root, false).unwrap();
    let listed: Vec<_> = walk.nodes.iter().map(|node| node.path.as_path()).collect();
    assert_eq!(listed, [root.as_path()]);
    assert!(walk.skipped.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn std_walk_dir_errors_on_a_root_it_cannot_read() {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);
    let locked = root.join("locked");
    fsio.create_dir_all(&locked).unwrap();
    fsio.write(&locked.join("track.ogg"), b"1").await.unwrap();
    fs::set_permissions(&locked, Permissions::from_mode(0o000)).unwrap();
    let walk = fsio.walk_dir(&locked, false);
    fs::set_permissions(&locked, Permissions::from_mode(0o755)).unwrap();

    assert!(
        matches!(&walk, Err(FileIoError::Io(error)) if error.kind() == ErrorKind::PermissionDenied),
        "{walk:?}"
    );
}

#[cfg(any(windows, all(unix, not(target_vendor = "apple"))))]
#[tokio::test]
async fn std_walk_dir_reports_paths_that_are_not_unicode_apart() {
    use fsio::test_fixtures::non_unicode_name;

    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);
    fsio.write(&root.join("kept"), b"1").await.unwrap();
    let file = root.join(non_unicode_name("file-", ""));
    fsio.write(&file, b"2").await.unwrap();
    let folder = root.join(non_unicode_name("folder-", ""));
    fsio.create_dir_all(&folder).unwrap();
    fsio.write(&folder.join("inside"), b"3").await.unwrap();
    fsio.create_dir_all(&root.join("nested")).unwrap();
    let nested = root.join("nested").join(non_unicode_name("nested-", ""));
    fsio.write(&nested, b"4").await.unwrap();

    let walk = fsio.walk_dir(&root, false).unwrap();
    let mut listed: Vec<_> = walk.nodes.iter().map(|node| node.path.clone()).collect();
    listed.sort();
    assert_eq!(
        listed,
        [root.clone(), root.join("kept"), root.join("nested")]
    );
    let mut skipped = walk.skipped;
    skipped.sort();
    let mut expected = vec![file, folder, nested];
    expected.sort();
    assert_eq!(skipped, expected);
}

#[cfg(all(unix, not(target_vendor = "apple")))]
#[tokio::test]
async fn std_walk_dir_reports_a_root_link_that_is_not_unicode_alone() {
    use std::os::unix::fs::symlink;

    use fsio::test_fixtures::non_unicode_name;

    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);
    let target = root.join("target");
    fsio.create_dir_all(&target).unwrap();
    fsio.write(&target.join("inside"), b"1").await.unwrap();
    let link = root.join(non_unicode_name("link-", ""));
    symlink(&target, &link).unwrap();

    // walkdir enters a root link even when it does not follow links.
    let walk = fsio.walk_dir(&link, false).unwrap();
    assert!(walk.nodes.is_empty());
    assert_eq!(walk.skipped, [link]);
}

#[cfg(any(windows, all(unix, not(target_vendor = "apple"))))]
#[tokio::test]
async fn std_read_dir_and_canonicalize_reject_paths_that_are_not_unicode() {
    use fsio::test_fixtures::non_unicode_name;

    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);
    fsio.write(&root.join("kept"), b"1").await.unwrap();
    let file = root.join(non_unicode_name("file-", ""));
    fsio.write(&file, b"2").await.unwrap();

    let names: Vec<_> = fsio
        .read_dir(&root)
        .await
        .unwrap()
        .into_iter()
        .map(|node| node.filename)
        .collect();
    assert_eq!(names, ["kept"]);
    assert!(matches!(
        fsio.canonicalize(&file),
        Err(FileIoError::InvalidPath)
    ));
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
    assert!(fsio.open(&missing, OpenMode::Read).is_err());
    assert!(fsio.is_file(&missing).await.is_err());
    assert!(fsio.is_dir(&missing).await.is_err());
    assert!(!fsio.exists(&missing).unwrap());
    let walk = fsio.walk_dir(&missing, false);
    assert!(
        matches!(&walk, Err(FileIoError::Io(error)) if error.kind() == ErrorKind::NotFound),
        "{walk:?}"
    );
}

#[tokio::test]
async fn std_modified_time_reports_real_mtime() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);
    let file = root.join("mtime.txt");

    fsio.write(&file, b"x").await.unwrap();
    let mtime = fsio.modified_time(&file).unwrap();
    assert!(mtime > 0);

    assert!(fsio.modified_time(&root.join("missing")).is_err());

    let noop = FsIo::new_noop();
    assert_eq!(noop.modified_time(Path::new("any")).unwrap(), 0);
}

#[tokio::test]
async fn std_read_dir_details_reports_kind_size_time_and_hidden() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);
    fsio.write(&root.join("song.flac"), b"12345").await.unwrap();
    fsio.write(&root.join(".secret"), b"x").await.unwrap();
    fsio.create_dir(&root, "Albums").await.unwrap();

    // Plain paths have no document provider to name them to.
    assert_eq!(fsio.content_uri(&root.join("song.flac")).unwrap(), None);

    let mut entries = fsio.read_dir_details(&root).await.unwrap();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let summary: Vec<_> = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.kind, entry.size, entry.hidden))
        .collect();
    assert_eq!(
        summary,
        [
            (".secret", EntryKind::File, 1, cfg!(not(windows))),
            ("Albums", EntryKind::Directory, 0, false),
            ("song.flac", EntryKind::File, 5, false),
        ]
    );
    let song = &entries[2];
    assert_eq!(song.path, root.join("song.flac"));
    assert_eq!(song.modified, Some(fsio.modified_time(&song.path).unwrap()));
}

#[cfg(unix)]
#[tokio::test]
async fn std_read_dir_details_follows_links_and_keeps_broken_ones() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);
    fsio.create_dir(&root, "real").await.unwrap();
    fsio.write(&root.join("real/track.mp3"), b"abc")
        .await
        .unwrap();
    symlink(root.join("real"), root.join("linked")).unwrap();
    symlink(root.join("real/track.mp3"), root.join("linked.mp3")).unwrap();
    symlink(root.join("gone"), root.join("broken")).unwrap();

    let mut entries = fsio.read_dir_details(&root).await.unwrap();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let kinds: Vec<_> = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.kind, entry.size))
        .collect();
    assert_eq!(
        kinds,
        [
            ("broken", EntryKind::Other, 0),
            ("linked", EntryKind::Directory, 0),
            ("linked.mp3", EntryKind::File, 3),
            ("real", EntryKind::Directory, 0),
        ]
    );
}

#[tokio::test]
async fn std_read_dir_details_fails_on_a_missing_or_file_path() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);
    fsio.write(&root.join("file.txt"), b"x").await.unwrap();

    let missing = fsio.read_dir_details(&root.join("missing")).await;
    assert!(
        matches!(&missing, Err(FileIoError::Io(error)) if error.kind() == ErrorKind::NotFound),
        "{missing:?}"
    );
    assert!(fsio.read_dir_details(&root.join("file.txt")).await.is_err());
}

#[tokio::test]
async fn default_read_dir_details_derives_from_nodes() {
    let node = FsNode {
        filename: ".hidden.txt".into(),
        path: "dir/.hidden.txt".into(),
        raw_path: "dir/.hidden.txt".into(),
        is_dir: false,
        is_file: true,
        size: 9,
    };
    let details = DirEntryDetails::from_node(node);
    assert_eq!(details.kind, EntryKind::File);
    assert_eq!(details.size, 9);
    assert!(details.hidden);
    assert_eq!(details.modified, None);

    let fsio = FsIo::new_noop();
    assert!(fsio
        .read_dir_details(Path::new("any"))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn std_rename_entry_renames_files_and_folders_without_replacing() {
    let temp = tempfile::tempdir().unwrap();
    let (fsio, root) = std_fsio_in(&temp);
    fsio.write(&root.join("a.flac"), b"a").await.unwrap();
    fsio.write(&root.join("taken.flac"), b"keep").await.unwrap();
    fsio.create_dir(&root, "New folder").await.unwrap();

    fsio.rename_entry(&root.join("a.flac"), &root.join("b.flac"))
        .await
        .unwrap();
    fsio.rename_entry(&root.join("New folder"), &root.join("Live"))
        .await
        .unwrap();
    assert!(fsio.is_file(&root.join("b.flac")).await.unwrap());
    assert!(fsio.is_dir(&root.join("Live")).await.unwrap());

    let taken = fsio
        .rename_entry(&root.join("b.flac"), &root.join("taken.flac"))
        .await;
    assert!(
        matches!(&taken, Err(FileIoError::Io(error)) if error.kind() == ErrorKind::AlreadyExists),
        "{taken:?}"
    );
    assert_eq!(fsio.read(&root.join("taken.flac")).unwrap(), b"keep");

    fsio.create_dir(&root, "elsewhere").await.unwrap();
    assert!(matches!(
        fsio.rename_entry(&root.join("b.flac"), &root.join("elsewhere/b.flac"))
            .await,
        Err(FileIoError::InvalidPath)
    ));
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
    assert!(fsio.walk_dir(path, false).unwrap().nodes.is_empty());

    fsio.create_dir_all(path).unwrap();
    fsio.remove_file(path).await.unwrap();
    fsio.remove_dir_all(path).await.unwrap();

    let mut stream = fsio.open(path, OpenMode::ReadWrite).unwrap();
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
    let results = self_test::run_fs_self_test(&fsio, &root, |r| {
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
