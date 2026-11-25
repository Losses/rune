//! What a streaming service's library keeps on this device of what the user makes in Rune, such
//! as Rune's own playlists, beside what its service holds. Each kind of record is one small file
//! in the library's records folder, named after the kind. They are never synced and never served
//! to other devices: a service library stays on the device that signed it in.
//!
//! Unlike the lists the library reads from its service (see `lists`), the records are the user's
//! own: a change is written before it answers, a sign-out keeps them, and a file that cannot be
//! read is reported and left as it is, never replaced.

use std::collections::HashSet;
use std::fmt::Display;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use tokio::sync::Mutex;
use tokio::task;
use uuid::Uuid;

use crate::request_error::ExpectedRequestError;
use crate::service::folder::service_library_folder;
use crate::service::store::{create_private_dir, create_private_file};

/// The folder in which the service library `library_id` keeps its records.
pub(crate) fn device_records_folder(config_dir: &Path, library_id: Uuid) -> PathBuf {
    service_library_folder(config_dir, library_id).join("rune")
}

/// One kind of record a store keeps.
pub(crate) trait StoredRecord:
    Clone + Serialize + DeserializeOwned + Send + Sync + 'static
{
    /// The field of the file that holds the records.
    const FIELD: &'static str;
    /// The version of the file's format. A file of another version is reported, not read.
    const FORMAT: u32;
    /// What the records are, as a failure names them.
    const WHAT: &'static str;

    /// Names the record for good. The ids the UI knows it by are the binding's.
    fn id(&self) -> &str;
}

/// The file as it is read: its format, and its other fields, among them the records.
#[derive(Deserialize)]
struct StoreFile {
    format: u32,
    #[serde(flatten)]
    fields: serde_json::Map<String, serde_json::Value>,
}

/// The file as it is written: its format, then the records under their field.
struct StoreFileRef<'a, T> {
    records: &'a [T],
}

impl<T: StoredRecord> Serialize for StoreFileRef<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut file = serializer.serialize_map(Some(2))?;
        file.serialize_entry("format", &T::FORMAT)?;
        file.serialize_entry(T::FIELD, self.records)?;
        file.end()
    }
}

/// The file could not be read as the records it should hold. It is left as it is, so nothing it
/// holds is lost; the records cannot be used until it is mended or removed.
#[derive(Debug, thiserror::Error)]
#[error("The {what} of this library could not be read from {path}: {detail}")]
pub(crate) struct UnreadableStore {
    what: &'static str,
    path: String,
    detail: String,
}

enum State<T> {
    /// The file is not to be read yet: whose it is, is still being told (see `held`).
    Held,
    /// The file was not read yet, or could not be read the last time.
    Unread,
    Read(Arc<Vec<T>>),
    /// The library record is being removed; nothing may be written any more.
    Closed,
}

/// The records of one kind of one service library. The file is read when they are first asked
/// for, and every change replaces it whole, one change at a time.
pub(crate) struct DeviceStore<T> {
    inner: Arc<Inner<T>>,
}

struct Inner<T> {
    path: PathBuf,
    state: Mutex<State<T>>,
    /// Whether `listed` logged that the file cannot be read, which it does once until the file
    /// can be read again.
    unreadable_logged: AtomicBool,
}

fn closed(what: &'static str) -> anyhow::Error {
    ExpectedRequestError::Unavailable { what }.into()
}

impl<T: StoredRecord> Inner<T> {
    fn unreadable(&self, detail: impl Display) -> anyhow::Error {
        UnreadableStore {
            what: T::WHAT,
            path: self.path.display().to_string(),
            detail: detail.to_string(),
        }
        .into()
    }

    /// The records the file holds; none when there is no file yet.
    async fn read_file(&self) -> Result<Vec<T>> {
        let path = self.path.clone();
        let bytes = match task::spawn_blocking(move || std::fs::read(path)).await? {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(self.unreadable(error)),
        };
        let mut file: StoreFile =
            serde_json::from_slice(&bytes).map_err(|error| self.unreadable(error))?;
        if file.format != T::FORMAT {
            return Err(self.unreadable(format_args!("format {} is not known", file.format)));
        }
        let records = file
            .fields
            .remove(T::FIELD)
            .ok_or_else(|| self.unreadable(format_args!("it has no {}", T::FIELD)))?;
        let records: Vec<T> =
            serde_json::from_value(records).map_err(|error| self.unreadable(error))?;
        let mut ids = HashSet::new();
        if let Some(record) = records.iter().find(|record| !ids.insert(record.id())) {
            return Err(self.unreadable(format_args!("two records have the id {}", record.id())));
        }
        Ok(records)
    }

    /// The records as they are, read from the file the first time.
    async fn loaded(&self, state: &mut State<T>) -> Result<Arc<Vec<T>>> {
        match state {
            State::Read(records) => Ok(Arc::clone(records)),
            State::Held => Ok(Arc::default()),
            State::Closed => Err(closed(T::WHAT)),
            State::Unread => {
                let records = Arc::new(self.read_file().await?);
                // A write cut short, as when the app quit during it, left its new file behind.
                let path = self.path.clone();
                task::spawn_blocking(move || remove_leftovers(&path)).await?;
                *state = State::Read(Arc::clone(&records));
                Ok(records)
            }
        }
    }
}

impl<T: StoredRecord> DeviceStore<T> {
    /// The records kept in the file at `path`, in the records folder of a service library.
    pub(crate) fn new(path: PathBuf) -> Self {
        Self::with_state(path, State::Unread)
    }

    /// A store whose file is not read until `release`: the records folder may still be replaced
    /// by one a removed library left. Meanwhile it lists no records and refuses every change.
    pub(crate) fn held(path: PathBuf) -> Self {
        Self::with_state(path, State::Held)
    }

    fn with_state(path: PathBuf, state: State<T>) -> Self {
        Self {
            inner: Arc::new(Inner {
                path,
                state: Mutex::new(state),
                unreadable_logged: AtomicBool::new(false),
            }),
        }
    }

    /// Reads the file from now on, as it is then, unless the store was closed.
    pub(crate) async fn release(&self) {
        let mut state = self.inner.state.lock().await;
        if matches!(*state, State::Held) {
            *state = State::Unread;
        }
    }

    /// Every record, in the order they were made. A file that cannot be read fails with
    /// `UnreadableStore`, which names it.
    pub(crate) async fn list(&self) -> Result<Arc<Vec<T>>> {
        let mut state = self.inner.state.lock().await;
        self.inner.loaded(&mut state).await
    }

    /// The records to list beside the service's own. A file that cannot be read lists none, so
    /// the service's still show; it is logged once, until the file can be read again, and
    /// everything that uses one of the records still fails with it.
    pub(crate) async fn listed(&self) -> Result<Arc<Vec<T>>> {
        match self.list().await {
            Ok(records) => {
                self.inner.unreadable_logged.store(false, Ordering::Relaxed);
                Ok(records)
            }
            Err(error) if error.downcast_ref::<UnreadableStore>().is_some() => {
                if !self.inner.unreadable_logged.swap(true, Ordering::Relaxed) {
                    log::warn!("{error:#}; they are not listed until it is mended or removed");
                }
                Ok(Arc::default())
            }
            Err(error) => Err(error),
        }
    }

    /// Applies `change` to a copy of the records and writes it to the file; only once it is
    /// written does the copy take their place. Changes wait for each other. The change runs on a
    /// task of its own, so it ends as it began even when the caller stops waiting: the file and
    /// the records kept in memory never differ, and `close` waits for it.
    pub(crate) async fn change<A: Send + 'static>(
        &self,
        change: impl FnOnce(&mut Vec<T>) -> Result<A> + Send + 'static,
    ) -> Result<A> {
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            let mut state = inner.state.lock().await;
            if matches!(*state, State::Held) {
                return Err(closed(T::WHAT));
            }
            let mut records = (*inner.loaded(&mut state).await?).clone();
            let answer = change(&mut records)?;
            let bytes = serde_json::to_vec_pretty(&StoreFileRef { records: &records })?;
            let path = inner.path.clone();
            task::spawn_blocking(move || write_atomically(&path, &bytes))
                .await?
                .with_context(|| {
                    format!("Could not save the {} to {}", T::WHAT, inner.path.display())
                })?;
            *state = State::Read(Arc::new(records));
            Ok(answer)
        })
        .await?
    }

    /// Writes `bytes` to the file at `path` beside the records' file, as a note that goes with
    /// them, unless the store was closed or is held.
    pub(crate) async fn write_beside(&self, path: PathBuf, bytes: Vec<u8>) -> Result<()> {
        let state = self.inner.state.lock().await;
        if matches!(*state, State::Closed | State::Held) {
            return Err(closed(T::WHAT));
        }
        let written = path.clone();
        task::spawn_blocking(move || write_atomically(&written, &bytes))
            .await?
            .with_context(|| format!("Could not write {}", path.display()))?;
        drop(state);
        Ok(())
    }

    /// Waits for a change being written, then refuses every later one: the library record is
    /// being removed.
    pub(crate) async fn close(&self) {
        *self.inner.state.lock().await = State::Closed;
    }
}

/// The records of kind `T` the file at `path` holds, read in place as well as it can be; none
/// when it cannot be read. For what tells whose a records folder is, where nothing waits.
pub(crate) fn read_records<T: StoredRecord>(path: &Path) -> Vec<T> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    serde_json::from_slice::<StoreFile>(&bytes)
        .ok()
        .filter(|file| file.format == T::FORMAT)
        .and_then(|mut file| file.fields.remove(T::FIELD))
        .and_then(|records| serde_json::from_value(records).ok())
        .unwrap_or_default()
}

/// The prefix of the temporary files a write of `path` makes beside it.
fn temporary_prefix(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!(".{name}.")
}

/// Removes the temporary files that writes of `path` cut short left beside it. What cannot be
/// removed is only logged: nothing reads them.
fn remove_leftovers(path: &Path) {
    let Some(dir) = path.parent() else {
        return;
    };
    let prefix = temporary_prefix(path);
    let Ok(listing) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in listing.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix)
            && name.ends_with(".tmp")
            && let Err(error) = std::fs::remove_file(entry.path())
        {
            log::warn!("Could not remove the unfinished file {name}: {error}");
        }
    }
}

/// Writes `bytes` to a file beside `path` and moves it into place, so that `path` holds either the
/// old records or the new ones, never a part of them. The folder is made private to its owner, in
/// a parent that must exist already: once the library's folder is deleted with its record,
/// nothing is written back into it. Once the file is in place the write has happened: a folder
/// that cannot be synced after it is only logged.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidInput, "no folder"))?;
    create_private_dir(dir)?;
    let temporary = dir.join(format!("{}{}.tmp", temporary_prefix(path), Uuid::new_v4()));
    let written = (|| {
        let mut file = create_private_file(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
        return written;
    }
    #[cfg(unix)]
    if let Err(error) = std::fs::File::open(dir).and_then(|dir| dir.sync_all()) {
        log::warn!(
            "Could not sync the folder of {} after saving it: {error}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use futures::FutureExt;

    use super::*;

    /// A record of the kind the tests keep.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct Note {
        id: String,
        text: String,
    }

    impl StoredRecord for Note {
        const FIELD: &'static str = "notes";
        const FORMAT: u32 = 1;
        const WHAT: &'static str = "notes";

        fn id(&self) -> &str {
            &self.id
        }
    }

    fn note(id: &str, text: &str) -> Note {
        Note {
            id: id.to_owned(),
            text: text.to_owned(),
        }
    }

    /// The notes of a new library in `config_dir`, whose folder exists as its binding makes it.
    fn store(config_dir: &Path) -> (PathBuf, DeviceStore<Note>) {
        let library = service_library_folder(config_dir, Uuid::new_v4());
        std::fs::create_dir_all(&library).unwrap();
        let path = library.join("rune").join("notes.json");
        (path.clone(), DeviceStore::new(path))
    }

    /// Keeps `note` after the others.
    async fn add(store: &DeviceStore<Note>, note: Note) -> Result<()> {
        store
            .change(move |notes| {
                notes.push(note);
                Ok(())
            })
            .await
    }

    #[test]
    fn the_records_folder_is_in_the_folder_of_its_library() {
        let library = Uuid::new_v4();
        assert_eq!(
            device_records_folder(Path::new("config"), library),
            service_library_folder(Path::new("config"), library).join("rune")
        );
    }

    #[tokio::test]
    async fn changes_are_written_and_read_back_by_another_store() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (path, notes) = store(dir.path());
        // Nothing is written until something changes.
        assert!(notes.list().await?.is_empty());
        assert!(!path.exists());

        add(&notes, note("a", "First")).await?;
        add(&notes, note("b", "Second")).await?;
        let answer = notes
            .change(|notes| {
                notes[0].text = "Changed".to_owned();
                Ok(notes.len())
            })
            .await?;
        assert_eq!(answer, 2);
        // A change that fails is not kept, and nothing is written for it.
        let written = std::fs::read(&path)?;
        let refused = notes.change(|notes| -> Result<()> {
            notes.clear();
            anyhow::bail!("refused")
        });
        assert!(refused.await.is_err());
        assert_eq!(std::fs::read(&path)?, written);

        let again = DeviceStore::<Note>::new(path.clone());
        assert_eq!(
            *again.list().await?,
            [note("a", "Changed"), note("b", "Second")]
        );
        assert_eq!(*again.list().await?, *notes.list().await?);
        // No temporary file is left beside it.
        assert_eq!(std::fs::read_dir(path.parent().unwrap())?.count(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn a_file_that_cannot_be_read_is_reported_and_left_as_it_is() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for bad in [
            &b"{ not json"[..],
            br#"{"format": 2, "notes": []}"#,
            br#"{"format": 1, "others": []}"#,
            br#"{"format": 1, "notes": [
                {"id": "a", "text": "A"},
                {"id": "a", "text": "B"}
            ]}"#,
        ] {
            let (path, notes) = store(dir.path());
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, bad)?;
            let error = notes.list().await.unwrap_err();
            assert!(
                error.downcast_ref::<UnreadableStore>().is_some(),
                "{error:#}"
            );
            assert!(error.to_string().contains(&path.display().to_string()));
            // Beside the service's records, none are listed meanwhile, every time.
            assert!(notes.listed().await?.is_empty());
            assert!(notes.listed().await?.is_empty());
            assert!(add(&notes, note("b", "New")).await.is_err());
            assert_eq!(std::fs::read(&path)?, bad);
            // Once the file is mended or removed, the records are read again.
            std::fs::remove_file(&path)?;
            assert!(notes.listed().await?.is_empty());
            add(&notes, note("b", "New")).await?;
            assert_eq!(*notes.listed().await?, [note("b", "New")]);
        }
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_change_that_cannot_be_written_is_not_kept() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir()?;
        let (path, notes) = store(dir.path());
        add(&notes, note("a", "Kept")).await?;
        let folder = path.parent().unwrap();
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o500))?;
        let refused = add(&notes, note("b", "Refused")).await;
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))?;
        assert!(refused.is_err());
        assert_eq!(*notes.list().await?, [note("a", "Kept")]);
        assert_eq!(
            *DeviceStore::<Note>::new(path).list().await?,
            [note("a", "Kept")]
        );
        Ok(())
    }

    #[tokio::test]
    async fn nothing_is_written_once_the_folder_of_the_library_is_gone() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (path, notes) = store(dir.path());
        std::fs::remove_dir(path.parent().unwrap().parent().unwrap())?;
        assert!(add(&notes, note("a", "Late")).await.is_err());
        assert!(notes.list().await?.is_empty());
        assert!(!path.parent().unwrap().parent().unwrap().exists());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_change_whose_caller_stops_waiting_is_still_written_and_kept() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (path, notes) = store(dir.path());
        // The store is held as a change holds it, so the change cannot finish before its caller
        // stops waiting.
        let held = notes.inner.state.lock().await;
        // Polled once, then dropped.
        assert!(add(&notes, note("a", "A")).now_or_never().is_none());
        assert!(!path.exists());
        drop(held);
        crate::test_wait::wait_until("the change to be written", || async {
            Ok((notes.list().await?.len() == 1).then_some(()))
        })
        .await?;
        assert_eq!(*notes.list().await?, [note("a", "A")]);
        assert_eq!(
            *DeviceStore::<Note>::new(path).list().await?,
            [note("a", "A")]
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn changes_made_at_once_are_all_kept() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (path, notes) = store(dir.path());
        let notes = Arc::new(notes);
        let adds = (0..20).map(|n| {
            let notes = Arc::clone(&notes);
            tokio::spawn(async move { add(&notes, note(&n.to_string(), "Note")).await })
        });
        for added in futures::future::join_all(adds).await {
            added??;
        }
        assert_eq!(notes.list().await?.len(), 20);
        assert_eq!(DeviceStore::<Note>::new(path).list().await?.len(), 20);
        Ok(())
    }

    #[tokio::test]
    async fn leftovers_of_cut_short_writes_go_on_load() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (path, notes) = store(dir.path());
        let folder = path.parent().unwrap().to_owned();
        std::fs::create_dir_all(&folder)?;
        let leftover = folder.join(format!(".notes.json.{}.tmp", Uuid::new_v4()));
        let other = folder.join(format!(".others.json.{}.tmp", Uuid::new_v4()));
        std::fs::write(&leftover, b"{")?;
        std::fs::write(&other, b"{")?;
        notes.list().await?;
        assert!(!leftover.exists());
        // Another kind's are its own.
        assert!(other.exists());
        Ok(())
    }

    #[tokio::test]
    async fn a_held_store_lists_none_and_writes_nothing_until_it_is_released() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (path, notes) = store(dir.path());
        add(&notes, note("a", "Kept")).await?;
        let held = DeviceStore::<Note>::held(path.clone());
        let beside = path.with_extension("note");
        assert!(held.list().await?.is_empty());
        assert!(add(&held, note("b", "Refused")).await.is_err());
        assert!(held.write_beside(beside.clone(), Vec::new()).await.is_err());
        assert!(!beside.exists());
        held.release().await;
        assert_eq!(*held.list().await?, [note("a", "Kept")]);
        held.write_beside(beside.clone(), b"noted".to_vec()).await?;
        assert_eq!(std::fs::read(&beside)?, b"noted");
        // Read in place, the file holds what the store keeps; one that cannot be read holds
        // none.
        assert_eq!(read_records::<Note>(&path), [note("a", "Kept")]);
        assert!(read_records::<Note>(&beside).is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn a_closed_store_refuses_changes_and_keeps_its_file() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (path, notes) = store(dir.path());
        add(&notes, note("a", "Kept")).await?;
        notes.close().await;
        let refused = add(&notes, note("b", "Late")).await.unwrap_err();
        assert!(matches!(
            refused.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unavailable { .. })
        ));
        assert!(
            notes
                .write_beside(path.with_extension("note"), Vec::new())
                .await
                .is_err()
        );
        // Released after it was closed, it stays closed.
        notes.release().await;
        assert!(notes.list().await.is_err());
        assert_eq!(
            *DeviceStore::<Note>::new(path).list().await?,
            [note("a", "Kept")]
        );
        Ok(())
    }
}
