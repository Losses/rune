//! The folder a streaming service's library keeps on this device, for its covers and for what it
//! read from its service.

use std::fs;
use std::io;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use tokio::fs as tokio_fs;
use uuid::Uuid;

use crate::service::covers::cover_cache_root;

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

/// Deletes the folder of the service library `library_id`, with everything it kept, and the
/// covers it kept before it had a folder, when they were never moved.
pub(crate) async fn delete_service_library_folder(
    config_dir: &Path,
    library_id: Uuid,
) -> io::Result<()> {
    for folder in [
        service_library_folder(config_dir, library_id),
        old_covers_folder(config_dir, library_id),
    ] {
        match tokio_fs::remove_dir_all(folder).await {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use uuid::Uuid;

    use super::{
        delete_service_library_folder, old_covers_folder, prepare_service_library_folder,
        service_library_folder,
    };
    use crate::service::covers::cover_cache_root;

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

        delete_service_library_folder(root.path(), library_id)
            .await
            .unwrap();
        assert!(!folder.exists());
        assert!(!old.exists());
        // Nothing left to delete is no failure.
        delete_service_library_folder(root.path(), library_id)
            .await
            .unwrap();
    }
}
