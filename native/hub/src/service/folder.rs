//! The folder a streaming service's library keeps on this device, for its covers, for what it
//! read from its service and for what the user made in Rune (see `device_records`). The records
//! outlive the library's record: a library added again may take over those a removed library
//! left.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tokio::fs as tokio_fs;
use uuid::Uuid;

use crate::service::covers::cover_cache_root;
use crate::service::device_records::device_records_folder;

/// The folder, in the configuration folder, that holds the folder of each service library.
const SERVICE_LIBRARIES: &str = "service-libraries";

/// The folder, in the configuration folder, where each Subsonic library kept its covers before
/// every service library had a folder of its own.
const OLD_COVERS: &str = "subsonic-covers";

/// The folder of the service library `library_id`, named by its library id.
pub(crate) fn service_library_folder(config_dir: &Path, library_id: Uuid) -> PathBuf {
    config_dir
        .join(SERVICE_LIBRARIES)
        .join(library_id.to_string())
}

/// Where the service library `library_id` kept its covers before it had a folder of its own.
fn old_covers_folder(config_dir: &Path, library_id: Uuid) -> PathBuf {
    config_dir.join(OLD_COVERS).join(library_id.to_string())
}

/// Makes the folder of the service library `library_id`, and moves the covers it kept before it
/// had one into it, unless its folder holds covers already. The move renames the old folder, so
/// the covers are either all in the old place or all in the new one. A move that fails is logged
/// and leaves the old folder as it was, and the covers are fetched again.
pub(crate) fn prepare_service_library_folder(config_dir: &Path, library_id: Uuid) -> PathBuf {
    let folder = service_library_folder(config_dir, library_id);
    if let Err(error) = fs::create_dir_all(&folder) {
        log::warn!("Could not make the folder of service library {library_id}: {error}");
        return folder;
    }
    let old = old_covers_folder(config_dir, library_id);
    let covers = cover_cache_root(config_dir, library_id);
    if old.is_dir() && !covers.exists() {
        match fs::rename(&old, &covers) {
            Ok(()) => {
                // The old parent goes once the last library has moved out of it.
                let _ = fs::remove_dir(config_dir.join(OLD_COVERS));
            }
            // Another binding of the library moved them first.
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                log::warn!("Could not move the covers of service library {library_id}: {error}");
            }
        }
    }
    folder
}

/// Deletes what the service library `library_id` kept on this device as its record is removed:
/// its folder with everything it read from its service, and the covers it kept before it had a
/// folder, when they were never moved. Its records folder stays when `keep_records` says so, as
/// what the user made in Rune outlives the record; then only the records folder is left in the
/// library's folder. A reset of the app deletes it too (see
/// `delete_left_service_library_folders`).
pub(crate) async fn delete_service_library_folder(
    config_dir: &Path,
    library_id: Uuid,
    keep_records: bool,
) -> io::Result<()> {
    let folder = service_library_folder(config_dir, library_id);
    let records = device_records_folder(config_dir, library_id);
    if keep_records && tokio_fs::try_exists(&records).await? {
        let mut entries = tokio_fs::read_dir(&folder).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path == records {
                continue;
            }
            if entry.file_type().await?.is_dir() {
                remove_if_there(tokio_fs::remove_dir_all(path).await)?;
            } else {
                remove_if_there(tokio_fs::remove_file(path).await)?;
            }
        }
    } else {
        remove_if_there(tokio_fs::remove_dir_all(folder).await)?;
    }
    remove_if_there(tokio_fs::remove_dir_all(old_covers_folder(config_dir, library_id)).await)
}

/// Deletes the folders the service libraries removed before left on this device, which hold
/// their records (see `delete_service_library_folder`): those of the libraries that `known` does
/// not name. A reset of the app deletes them with the libraries it removes, with anything a
/// removal could not delete beside the records. A folder of service libraries that cannot be
/// listed fails it, as nothing could be deleted. A folder that cannot be deleted does not keep
/// the others: each is deleted, and the first failure is answered after.
pub(crate) async fn delete_left_service_library_folders(
    config_dir: &Path,
    known: HashSet<Uuid>,
) -> io::Result<()> {
    let root = config_dir.to_owned();
    let left = tokio::task::spawn_blocking(move || scan_left(&root, Uuid::nil(), &known))
        .await
        .map_err(io::Error::other)??;
    let mut outcome = Ok(());
    for removed in left {
        let removal = remove_if_there(
            tokio_fs::remove_dir_all(service_library_folder(config_dir, removed)).await,
        );
        outcome = outcome.and(removal);
    }
    outcome
}

/// A removal that found nothing to remove is no failure.
fn remove_if_there(removed: io::Result<()>) -> io::Result<()> {
    match removed {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        removed => removed,
    }
}

/// Whether the service library `library_id` has a records folder of its own.
pub(crate) fn has_device_records(config_dir: &Path, library_id: Uuid) -> bool {
    device_records_folder(config_dir, library_id).is_dir()
}

/// The service libraries other than `library_id` and those `known` names that left a records
/// folder, as removed libraries do, the one written to last first. A folder that cannot be read
/// is none.
pub(crate) fn left_device_records(
    config_dir: &Path,
    library_id: Uuid,
    known: &HashSet<Uuid>,
) -> Vec<Uuid> {
    scan_left(config_dir, library_id, known).unwrap_or_default()
}

/// The libraries `left_device_records` answers, or the error of a folder of service libraries
/// that is there but cannot be read, to its last entry; one that is not there holds none.
fn scan_left(config_dir: &Path, library_id: Uuid, known: &HashSet<Uuid>) -> io::Result<Vec<Uuid>> {
    let listing = match fs::read_dir(config_dir.join(SERVICE_LIBRARIES)) {
        Ok(listing) => listing,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let entries = listing.collect::<io::Result<Vec<_>>>()?;
    let mut left: Vec<(SystemTime, Uuid)> = entries
        .into_iter()
        .filter_map(|entry| Uuid::parse_str(&entry.file_name().to_string_lossy()).ok())
        .filter(|removed| *removed != library_id && !known.contains(removed))
        .filter_map(|removed| {
            let written = last_written(&device_records_folder(config_dir, removed))?;
            Some((written, removed))
        })
        .collect();
    left.sort_unstable_by(|left, right| right.cmp(left));
    Ok(left.into_iter().map(|(_, removed)| removed).collect())
}

/// When the records in `folder` were written last; None when it is no folder.
fn last_written(folder: &Path) -> Option<SystemTime> {
    let own = fs::metadata(folder)
        .ok()
        .filter(|metadata| metadata.is_dir())?
        .modified()
        .ok()?;
    let files = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.metadata().ok()?.modified().ok());
    files.chain([own]).max()
}

/// Takes over for the service library `library_id` the records folder that the removed library
/// `removed` left, in one rename of the folder: the records are the removed library's or this
/// one's, never both or neither, so a take-over cut short leaves nothing to finish, as moving the
/// files one by one would, which needed a note of the take-over to go on with. A library that
/// has records of its own keeps them, and the removed library's stay as they are. The removed
/// library's folder, empty then, goes.
pub(crate) fn take_over_device_records(
    config_dir: &Path,
    library_id: Uuid,
    removed: Uuid,
) -> io::Result<()> {
    let records = device_records_folder(config_dir, library_id);
    if records.exists() {
        log::warn!(
            "Kept the records the removed service library {removed} left: the library {library_id} has its own"
        );
        return Ok(());
    }
    fs::create_dir_all(service_library_folder(config_dir, library_id))?;
    fs::rename(device_records_folder(config_dir, removed), &records)?;
    #[cfg(unix)]
    if let Some(parent) = records.parent()
        && let Err(error) = fs::File::open(parent).and_then(|parent| parent.sync_all())
    {
        log::warn!("Could not sync the folder of service library {library_id}: {error}");
    }
    if let Err(error) = fs::remove_dir(service_library_folder(config_dir, removed)) {
        log::warn!("Could not remove the folder of the removed service library {removed}: {error}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;

    use tempfile::TempDir;
    use uuid::Uuid;

    use super::{
        delete_left_service_library_folders, delete_service_library_folder, has_device_records,
        left_device_records, old_covers_folder, prepare_service_library_folder,
        service_library_folder, take_over_device_records,
    };
    use crate::service::covers::cover_cache_root;
    use crate::service::device_records::device_records_folder;

    /// Writes a record file of the library `library_id` holding `content`.
    fn write_records(root: &std::path::Path, library_id: Uuid, content: &str) {
        let records = device_records_folder(root, library_id);
        fs::create_dir_all(&records).unwrap();
        fs::write(records.join("playlists.json"), content).unwrap();
    }

    fn read_records(root: &std::path::Path, library_id: Uuid) -> Option<String> {
        fs::read_to_string(device_records_folder(root, library_id).join("playlists.json")).ok()
    }

    #[test]
    fn the_covers_kept_before_move_into_the_library_folder_once() {
        let root = TempDir::new().unwrap();
        let (moved, other) = (Uuid::new_v4(), Uuid::new_v4());
        for library_id in [moved, other] {
            let old = old_covers_folder(root.path(), library_id);
            fs::create_dir_all(&old).unwrap();
            fs::write(old.join("cover.img"), b"cover").unwrap();
            fs::write(old.join("cover.color"), b"digest 7").unwrap();
        }

        let folder = prepare_service_library_folder(root.path(), moved);
        assert_eq!(folder, service_library_folder(root.path(), moved));
        let covers = cover_cache_root(root.path(), moved);
        assert_eq!(covers, folder.join("covers"));
        assert_eq!(fs::read(covers.join("cover.img")).unwrap(), b"cover");
        assert_eq!(fs::read(covers.join("cover.color")).unwrap(), b"digest 7");
        assert!(!old_covers_folder(root.path(), moved).exists());
        // The other library has not moved yet, so the old parent stays for it.
        assert!(old_covers_folder(root.path(), other).is_dir());

        // Prepared again, the folder keeps its covers.
        prepare_service_library_folder(root.path(), moved);
        assert_eq!(fs::read(covers.join("cover.img")).unwrap(), b"cover");

        // The last library moved out of the old parent removes it.
        prepare_service_library_folder(root.path(), other);
        assert!(!root.path().join("subsonic-covers").exists());
        assert!(
            cover_cache_root(root.path(), other)
                .join("cover.img")
                .is_file()
        );
    }

    #[test]
    fn covers_in_the_library_folder_are_not_replaced_by_old_ones() {
        let root = TempDir::new().unwrap();
        let library_id = Uuid::new_v4();
        let covers = cover_cache_root(root.path(), library_id);
        fs::create_dir_all(&covers).unwrap();
        fs::write(covers.join("cover.img"), b"new").unwrap();
        let old = old_covers_folder(root.path(), library_id);
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("cover.img"), b"old").unwrap();

        prepare_service_library_folder(root.path(), library_id);
        assert_eq!(fs::read(covers.join("cover.img")).unwrap(), b"new");
        assert_eq!(fs::read(old.join("cover.img")).unwrap(), b"old");
    }

    #[tokio::test]
    async fn a_reset_deletes_the_records_removed_libraries_left_and_nothing_else() {
        let root = TempDir::new().unwrap();
        let (left, known, partly) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        for library_id in [left, known, partly] {
            write_records(root.path(), library_id, "kept");
        }
        // A removal that could not delete everything left covers beside the records; a library
        // still registered keeps its covers.
        for library_id in [partly, known] {
            fs::create_dir_all(cover_cache_root(root.path(), library_id)).unwrap();
        }
        delete_left_service_library_folders(root.path(), HashSet::from([known]))
            .await
            .unwrap();
        assert!(!service_library_folder(root.path(), left).exists());
        assert!(!service_library_folder(root.path(), partly).exists());
        assert_eq!(read_records(root.path(), known).as_deref(), Some("kept"));
        assert!(cover_cache_root(root.path(), known).is_dir());
        // With nothing left, there is nothing to do.
        delete_left_service_library_folders(root.path(), HashSet::from([known]))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_reset_that_cannot_read_the_service_libraries_says_so() {
        let root = TempDir::new().unwrap();
        // None yet is nothing to delete.
        delete_left_service_library_folders(root.path(), HashSet::new())
            .await
            .unwrap();
        // One that is there but cannot be listed is no empty one.
        fs::write(root.path().join("service-libraries"), b"").unwrap();
        assert!(
            delete_left_service_library_folders(root.path(), HashSet::new())
                .await
                .is_err()
        );
        assert!(left_device_records(root.path(), Uuid::new_v4(), &HashSet::new()).is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_reset_deletes_every_folder_left_though_one_cannot_be_deleted() {
        use std::os::unix::fs::PermissionsExt;

        let root = TempDir::new().unwrap();
        let (stuck, older) = (Uuid::new_v4(), Uuid::new_v4());
        // The older records were written a minute ago, so the stuck folder comes first.
        write_records(root.path(), older, "older");
        let written = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        let records = device_records_folder(root.path(), older);
        for path in [records.join("playlists.json"), records] {
            open_to_set_times(&path).set_modified(written).unwrap();
        }
        // A records folder whose files cannot be deleted.
        write_records(root.path(), stuck, "stuck");
        let locked = device_records_folder(root.path(), stuck);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
        if fs::write(locked.join("probe"), b"").is_ok() {
            // Permissions do not hold this user back, so nothing here fails to be deleted.
            return;
        }
        assert_eq!(
            left_device_records(root.path(), Uuid::nil(), &HashSet::new()),
            [stuck, older]
        );

        let swept = delete_left_service_library_folders(root.path(), HashSet::new()).await;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(swept.is_err());
        assert!(!service_library_folder(root.path(), older).exists());
        assert_eq!(read_records(root.path(), stuck).as_deref(), Some("stuck"));
    }

    #[tokio::test]
    async fn removing_a_service_record_deletes_its_folder_and_its_old_covers() {
        let root = TempDir::new().unwrap();
        let library_id = Uuid::new_v4();
        let folder = prepare_service_library_folder(root.path(), library_id);
        let covers = cover_cache_root(root.path(), library_id);
        fs::create_dir_all(&covers).unwrap();
        fs::write(covers.join("cover.img"), b"cover").unwrap();
        fs::create_dir_all(folder.join("lists")).unwrap();
        fs::write(folder.join("lists").join("songs.json"), b"{}").unwrap();
        let old = old_covers_folder(root.path(), library_id);
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("cover.img"), b"cover").unwrap();

        delete_service_library_folder(root.path(), library_id, true)
            .await
            .unwrap();
        // A library without records leaves nothing, whether they were to be kept or not.
        assert!(!folder.exists());
        assert!(!old.exists());
        // Nothing left to delete is no failure.
        delete_service_library_folder(root.path(), library_id, false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_removed_library_keeps_its_records_unless_they_go_with_it() {
        let root = TempDir::new().unwrap();
        let (kept, reset) = (Uuid::new_v4(), Uuid::new_v4());
        for library_id in [kept, reset] {
            let folder = prepare_service_library_folder(root.path(), library_id);
            fs::create_dir_all(folder.join("lists")).unwrap();
            fs::write(folder.join("lists").join("songs.json"), b"{}").unwrap();
            fs::write(folder.join("loose"), b"").unwrap();
            write_records(root.path(), library_id, "mine");
        }
        delete_service_library_folder(root.path(), kept, true)
            .await
            .unwrap();
        let folder = service_library_folder(root.path(), kept);
        let left: Vec<_> = fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["rune"]);
        assert_eq!(read_records(root.path(), kept).as_deref(), Some("mine"));
        assert!(has_device_records(root.path(), kept));
        // A reset deletes them with the rest.
        delete_service_library_folder(root.path(), reset, false)
            .await
            .unwrap();
        assert!(!service_library_folder(root.path(), reset).exists());
        assert!(!has_device_records(root.path(), reset));
    }

    /// Opens the file or folder at `path` so its times can be set. Windows opens a folder only
    /// with backup semantics, and sets times only through a handle that may write attributes.
    fn open_to_set_times(path: &std::path::Path) -> fs::File {
        let mut options = fs::OpenOptions::new();
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            options
                .access_mode(FILE_WRITE_ATTRIBUTES)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
        }
        #[cfg(not(windows))]
        options.read(true);
        options.open(path).unwrap()
    }

    #[test]
    fn the_records_removed_libraries_left_are_found_newest_first_and_taken_over_whole() {
        let root = TempDir::new().unwrap();
        let (older, newer, known, own, new) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        // The older records were written a minute ago.
        write_records(root.path(), older, "older");
        let written = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        let records = device_records_folder(root.path(), older);
        for path in [records.join("playlists.json"), records] {
            open_to_set_times(&path).set_modified(written).unwrap();
        }
        write_records(root.path(), newer, "newer");
        write_records(root.path(), known, "known");
        write_records(root.path(), own, "own");
        // A folder without records, as of a library that never made any, left none.
        prepare_service_library_folder(root.path(), Uuid::new_v4());
        let registered = HashSet::from([known, new, own]);
        assert_eq!(
            left_device_records(root.path(), new, &registered),
            [newer, older]
        );

        take_over_device_records(root.path(), new, older).unwrap();
        assert_eq!(read_records(root.path(), new).as_deref(), Some("older"));
        assert!(!service_library_folder(root.path(), older).exists());
        assert_eq!(left_device_records(root.path(), new, &registered), [newer]);
        // Records the library has are never replaced: the removed library's stay.
        take_over_device_records(root.path(), own, newer).unwrap();
        assert_eq!(read_records(root.path(), own).as_deref(), Some("own"));
        assert_eq!(read_records(root.path(), newer).as_deref(), Some("newer"));
    }
}
