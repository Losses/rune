//! Whether a service library may look its songs' lyrics up at an online lyrics database. The
//! user decides it per library on this device, and it is off until they turn it on, as a lookup
//! tells the database which songs are played.

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::service::folder::service_library_folder;

/// The file that says whether the lookups are allowed, in the folder of the kept lyrics.
const CONSENT_FILE: &str = "consent.json";

#[derive(Serialize, Deserialize)]
struct Consent {
    enabled: bool,
}

/// Where the service library `library_id` keeps the lyrics it looked up and whether it may. It
/// is in the library's folder, so it goes with the library's record, but beside its lists, not
/// among them: lyrics belong to no account, so neither a sign-out nor another account clears
/// them.
pub(crate) fn kept_lyrics_folder(config_dir: &Path, library_id: Uuid) -> PathBuf {
    service_library_folder(config_dir, library_id).join("lyrics")
}

fn consent_path(config_dir: &Path, library_id: Uuid) -> PathBuf {
    kept_lyrics_folder(config_dir, library_id).join(CONSENT_FILE)
}

/// Whether the library `library_id` may look lyrics up online. A library that was never asked
/// may not, nor one whose answer cannot be read. It reads a file, so it blocks.
pub(crate) fn allowed(config_dir: &Path, library_id: Uuid) -> bool {
    let bytes = match std::fs::read(consent_path(config_dir, library_id)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return false,
        Err(error) => {
            log::warn!("Could not read whether lyrics may be looked up online: {error}");
            return false;
        }
    };
    match serde_json::from_slice::<Consent>(&bytes) {
        Ok(consent) => consent.enabled,
        Err(error) => {
            log::warn!("Ignored whether lyrics may be looked up online: {error}");
            false
        }
    }
}

/// Keeps whether the library `library_id` may look lyrics up online, replacing the file whole.
/// It writes a file, so it blocks.
pub(crate) fn set_allowed(
    config_dir: &Path,
    library_id: Uuid,
    enabled: bool,
) -> std::io::Result<()> {
    let path = consent_path(config_dir, library_id);
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let bytes = serde_json::to_vec(&Consent { enabled })?;
    let temporary = path.with_extension("json.tmp");
    let written = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, &path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::{allowed, consent_path, set_allowed};
    use uuid::Uuid;

    #[test]
    fn lookups_are_off_until_allowed_and_an_unreadable_answer_allows_none() {
        let dir = tempfile::tempdir().unwrap();
        let library = Uuid::new_v4();
        assert!(!allowed(dir.path(), library));
        set_allowed(dir.path(), library, true).unwrap();
        assert!(allowed(dir.path(), library));
        // Each library has its own answer.
        assert!(!allowed(dir.path(), Uuid::new_v4()));
        set_allowed(dir.path(), library, false).unwrap();
        assert!(!allowed(dir.path(), library));
        set_allowed(dir.path(), library, true).unwrap();
        std::fs::write(consent_path(dir.path(), library), b"not json").unwrap();
        assert!(!allowed(dir.path(), library));
    }
}
