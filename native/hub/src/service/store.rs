//! What a streaming service's library keeps on disk across restarts, so it answers from it at
//! once, before its service does.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::io::{ErrorKind, Write};
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The version of the files' format. A file of another version is ignored, and replaced by the
/// next write of its value.
const FORMAT: u32 = 1;

/// How long a write waits for more, so a burst of changes is written once.
const COALESCE: Duration = Duration::from_secs(1);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Values kept in one folder, a versioned JSON file each, by name, for the account they were read
/// for. A file that cannot be read, or that another account's values were written to, is
/// ignored, so a store never gives one account what was read for another, also when the files of
/// that one could not be deleted or are written after the account changed. Writes are made on a
/// thread of their own after a short wait, so a burst of them is written once, and each replaces
/// its file whole. The folder is made by the first write, in a parent that must exist already:
/// once the library's folder is deleted with its record, nothing is written back into it.
///
/// Clones are handles to the same folder.
#[derive(Clone)]
pub(crate) struct ListStore(Arc<Inner>);

/// How many times a store was cleared when a read began. What the read found is saved under it,
/// and dropped when the store was cleared since, as it may belong to another account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StoreGeneration(u64);

struct Inner {
    dir: PathBuf,
    state: Mutex<State>,
    /// Held while a file is written or the folder deleted, so a deletion never meets a write
    /// half done.
    io: Mutex<()>,
}

#[derive(Default)]
struct State {
    generation: u64,
    /// The account the values are kept for, as `account_key` gives it.
    account: Option<String>,
    /// The writes not made yet, by name: only the last of each is made.
    pending: HashMap<String, Pending>,
    /// Whether the writing thread runs.
    writing: bool,
}

struct Pending {
    generation: u64,
    encode: Box<dyn FnOnce() -> serde_json::Result<Vec<u8>> + Send>,
}

#[derive(Serialize)]
struct Versioned<'a, T> {
    version: u32,
    account: Option<&'a str>,
    value: &'a T,
}

#[derive(Deserialize)]
struct Stored<T> {
    version: u32,
    #[serde(default)]
    account: Option<String>,
    value: T,
}

/// What a store keeps of an account: a digest of it, so its files name no account. None for a
/// service whose accounts the binding cannot tell apart.
fn account_key(account: Option<&str>) -> Option<String> {
    account.map(|account| blake3::hash(account.as_bytes()).to_hex().to_string())
}

impl fmt::Debug for ListStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListStore")
            .field("dir", &self.0.dir)
            .finish_non_exhaustive()
    }
}

impl ListStore {
    /// A store of the folder `dir`, which its first write makes, for `account`.
    pub(crate) fn new(dir: PathBuf, account: Option<&str>) -> Self {
        Self(Arc::new(Inner {
            dir,
            state: Mutex::new(State {
                account: account_key(account),
                ..State::default()
            }),
            io: Mutex::new(()),
        }))
    }

    /// The generation to take before a read whose result is saved.
    pub(crate) fn generation(&self) -> StoreGeneration {
        StoreGeneration(lock(&self.0.state).generation)
    }

    /// The value saved as `name` for the store's account, or `None` when there is none, or it
    /// cannot be read, is of another version or was saved for another account. It reads the file,
    /// so it blocks.
    pub(crate) fn load<T: DeserializeOwned>(&self, name: &str) -> Option<T> {
        let bytes = match fs::read(self.0.path(name)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return None,
            Err(error) => {
                log::warn!("Could not read the kept {name}: {error}");
                return None;
            }
        };
        match serde_json::from_slice::<Stored<T>>(&bytes) {
            Ok(stored) if stored.version == FORMAT => {
                if stored.account == lock(&self.0.state).account {
                    Some(stored.value)
                } else {
                    log::info!("Ignored the kept {name} of another account");
                    None
                }
            }
            Ok(stored) => {
                log::info!("Ignored the kept {name} of format {}", stored.version);
                None
            }
            Err(error) => {
                log::warn!("Ignored the kept {name}, which cannot be read: {error}");
                None
            }
        }
    }

    /// Saves `value` as `name` soon, unless the store was cleared since `generation`. A later
    /// save of the same name before the write replaces this one.
    pub(crate) fn save<T>(&self, generation: StoreGeneration, name: &str, value: Arc<T>)
    where
        T: Serialize + Send + Sync + 'static,
    {
        let mut state = lock(&self.0.state);
        if state.generation != generation.0 {
            return;
        }
        // The generation names the account too, as only a clear changes it.
        let account = state.account.clone();
        let encode = Box::new(move || {
            serde_json::to_vec(&Versioned {
                version: FORMAT,
                account: account.as_deref(),
                value: &*value,
            })
        });
        state.pending.insert(
            name.to_owned(),
            Pending {
                generation: generation.0,
                encode,
            },
        );
        if state.writing {
            return;
        }
        let inner = Arc::clone(&self.0);
        let spawned = thread::Builder::new()
            .name("service-lists".into())
            .spawn(move || inner.write_pending());
        match spawned {
            Ok(_) => state.writing = true,
            Err(error) => log::warn!("Could not start writing the kept lists: {error}"),
        }
    }

    /// Deletes every saved value, drops the saves not written yet, keeps values for `account`
    /// from now on, and gives the new generation, under which saves are accepted again. It waits
    /// for a write being made, so nothing is left of it either. A file it cannot delete stays
    /// another account's, which the store does not give.
    pub(crate) fn clear(&self, account: Option<&str>) -> StoreGeneration {
        // The folder first, so a save under the new generation is written after the deletion.
        let _io = lock(&self.0.io);
        let mut state = lock(&self.0.state);
        state.generation += 1;
        state.account = account_key(account);
        state.pending.clear();
        match fs::remove_dir_all(&self.0.dir) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => log::warn!("Could not delete the kept lists: {error}"),
        }
        StoreGeneration(state.generation)
    }
}

impl Inner {
    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }

    /// The writing thread: waits for a burst of saves to end, writes them, and stops once
    /// nothing is left to write.
    fn write_pending(self: Arc<Self>) {
        loop {
            thread::sleep(COALESCE);
            let pending = {
                let mut state = lock(&self.state);
                if state.pending.is_empty() {
                    state.writing = false;
                    return;
                }
                mem::take(&mut state.pending)
            };
            for (name, pending) in pending {
                self.write(&name, pending);
            }
        }
    }

    fn write(&self, name: &str, pending: Pending) {
        let bytes = match (pending.encode)() {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("Could not encode the {name} to keep: {error}");
                return;
            }
        };
        let _io = lock(&self.io);
        // Checked while the folder cannot be deleted: a clear either came before and drops
        // this write, or comes after and deletes its file.
        if lock(&self.state).generation != pending.generation {
            return;
        }
        match write_atomically(&self.dir, &self.path(name), &bytes) {
            Ok(()) => {}
            // The library's folder went with its record.
            Err(error) if error.kind() == ErrorKind::NotFound => {
                log::debug!("Did not keep the {name}, as its library's folder is gone");
            }
            Err(error) => log::warn!("Could not keep the {name}: {error}"),
        }
    }
}

/// Writes `bytes` to a file beside `path` and moves it into place, so `path` holds the old value
/// or the new one, never a part of either. Each write has a file of its own, as another store of
/// the same library, such as one a binding closed a moment ago left writing, may write the same
/// name at once.
fn write_atomically(dir: &Path, path: &Path, bytes: &[u8]) -> io::Result<()> {
    create_private_dir(dir)?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = dir.join(format!(".{name}.{}.tmp", Uuid::new_v4().simple()));
    let written = (|| {
        let mut file = create_private_file(&temporary)?;
        file.write_all(bytes)?;
        fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// Makes the folder, readable by its owner only: what it keeps is the user's library. Its parent
/// is not made.
pub(crate) fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(dir) {
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
        created => created,
    }
}

/// Makes a file readable by its owner only.
pub(crate) fn create_private_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    use tempfile::TempDir;

    use super::{COALESCE, ListStore};

    fn names(value: &str) -> Arc<Vec<String>> {
        Arc::new(vec![value.to_owned()])
    }

    /// Waits until `read` finds what the store's writing thread wrote.
    fn written(read: &ListStore, name: &str) -> Option<Vec<String>> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let found = read.load::<Vec<String>>(name);
            if found.is_some() || Instant::now() > deadline {
                return found;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits for longer than a write waits, so a write that was to be made is made.
    fn after_a_write() {
        thread::sleep(COALESCE * 3);
    }

    #[test]
    fn saves_are_written_whole_and_read_back_by_another_store() {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("lists");
        let store = ListStore::new(dir.clone(), None);
        let generation = store.generation();
        store.save(generation, "names", names("a"));
        store.save(generation, "names", names("b"));
        let read = ListStore::new(dir.clone(), None);
        assert_eq!(written(&read, "names"), Some(vec!["b".to_owned()]));
        // Only the file itself is left.
        let files: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(files, ["names.json"]);

        // A save taken before a clear is not written after it.
        store.save(generation, "names", names("c"));
        store.clear(None);
        assert!(!dir.exists());
        store.save(generation, "names", names("d"));
        after_a_write();
        assert!(!dir.exists());
        assert_eq!(read.load::<Vec<String>>("names"), None);
    }

    #[test]
    fn a_clear_gives_the_generation_saves_are_accepted_under_from_then_on() {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("lists");
        let store = ListStore::new(dir.clone(), None);
        let before = store.generation();
        let generation = store.clear(None);
        assert_ne!(generation, before);
        assert_eq!(store.generation(), generation);
        store.save(generation, "names", names("a"));
        assert_eq!(written(&store, "names"), Some(vec!["a".to_owned()]));
    }

    #[cfg(unix)]
    #[test]
    fn the_folder_and_its_files_are_private_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;

        let root = TempDir::new().unwrap();
        let dir = root.path().join("lists");
        let store = ListStore::new(dir.clone(), None);
        store.save(store.generation(), "names", names("a"));
        assert!(written(&store, "names").is_some());

        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("names.json")), 0o600);
    }

    #[test]
    fn saves_are_written_after_the_last_handle_is_dropped() {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("lists");
        let store = ListStore::new(dir.clone(), None);
        store.save(store.generation(), "names", names("a"));
        drop(store);

        let read = ListStore::new(dir, None);
        assert_eq!(written(&read, "names"), Some(vec!["a".to_owned()]));
    }

    #[test]
    fn nothing_is_written_once_the_library_folder_is_gone() {
        let root = TempDir::new().unwrap();
        let library = root.path().join("library");
        fs::create_dir_all(&library).unwrap();
        let store = ListStore::new(library.join("lists"), None);
        store.save(store.generation(), "names", names("a"));
        // The record and its folder are deleted before the write is made.
        fs::remove_dir_all(&library).unwrap();
        after_a_write();
        assert!(!library.exists());
    }

    #[test]
    fn a_file_of_another_version_or_shape_is_ignored() {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("lists");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("old.json"), r#"{"version":0,"value":["a"]}"#).unwrap();
        fs::write(dir.join("broken.json"), "{").unwrap();
        let store = ListStore::new(dir.clone(), None);
        assert_eq!(store.load::<Vec<String>>("old"), None);
        assert_eq!(store.load::<Vec<String>>("broken"), None);
        assert_eq!(store.load::<Vec<String>>("missing"), None);
        store.clear(None);
        assert!(!dir.exists());
    }
}
