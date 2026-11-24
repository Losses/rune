//! Lists kept on disk across restarts, so a library answers from them at once, before Apple
//! does and before it is signed in.

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

/// The version of the files' format. A file of another version is ignored, and replaced by
/// the next write of its list.
const FORMAT: u32 = 1;

/// How long a write waits for more, so a burst of changes is written once.
const COALESCE: Duration = Duration::from_secs(1);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Lists kept in one folder of their own, a versioned JSON file each. A file that cannot be
/// read is ignored. Writes are made on a thread of their own after a short wait, so a burst
/// of them is written once, and each replaces its file whole.
///
/// Clones are handles to the same folder.
#[derive(Clone)]
pub struct ListStore(Arc<Inner>);

/// How many times a store was cleared when a read began. What the read found is saved under
/// it, and dropped when the store was cleared since, as it may belong to another account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreGeneration(u64);

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
    /// The writes not made yet, by name: only the last of each is made.
    pending: HashMap<String, Pending>,
    /// Whether the writing thread runs.
    writing: bool,
    /// Whether every later write is refused.
    closed: bool,
}

struct Pending {
    generation: u64,
    encode: Box<dyn FnOnce() -> serde_json::Result<Vec<u8>> + Send>,
}

#[derive(Serialize)]
struct Versioned<'a, T> {
    version: u32,
    value: &'a T,
}

#[derive(Deserialize)]
struct Stored<T> {
    version: u32,
    value: T,
}

impl fmt::Debug for ListStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListStore")
            .field("dir", &self.0.dir)
            .finish_non_exhaustive()
    }
}

impl ListStore {
    /// A store of the folder `dir`, which it makes when it first writes.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self(Arc::new(Inner {
            dir: dir.into(),
            state: Mutex::default(),
            io: Mutex::new(()),
        }))
    }

    /// The generation to take before a read whose result is saved.
    pub fn generation(&self) -> StoreGeneration {
        StoreGeneration(lock(&self.0.state).generation)
    }

    /// The list saved as `name`, or `None` when there is none, or it cannot be read or is
    /// of another version. It reads the file, so it blocks.
    pub fn load<T: DeserializeOwned>(&self, name: &str) -> Option<T> {
        let path = self.0.path(name);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return None,
            Err(error) => {
                log::warn!("Could not read the kept Apple Music {name}: {error}");
                return None;
            }
        };
        match serde_json::from_slice::<Stored<T>>(&bytes) {
            Ok(stored) if stored.version == FORMAT => Some(stored.value),
            Ok(stored) => {
                log::info!(
                    "Ignored the kept Apple Music {name} of format {}",
                    stored.version
                );
                None
            }
            Err(error) => {
                log::warn!("Ignored the kept Apple Music {name}, which cannot be read: {error}");
                None
            }
        }
    }

    /// Saves `value` as `name` soon, unless the store was cleared since `generation` or is
    /// closed. A later save of the same name before the write replaces this one.
    pub fn save<T>(&self, generation: StoreGeneration, name: &str, value: Arc<T>)
    where
        T: Serialize + Send + Sync + 'static,
    {
        let mut state = lock(&self.0.state);
        if state.closed || state.generation != generation.0 {
            return;
        }
        let encode = Box::new(move || {
            serde_json::to_vec(&Versioned {
                version: FORMAT,
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
            .name("apple-music-lists".into())
            .spawn(move || inner.write_pending());
        match spawned {
            Ok(_) => state.writing = true,
            Err(error) => log::warn!("Could not start writing the Apple Music lists: {error}"),
        }
    }

    /// Deletes every saved list and drops the saves not written yet. It waits for a write
    /// being made, so nothing is left of it either.
    pub fn clear(&self) {
        self.clear_then(|| {});
    }

    /// Like [`clear`](Self::clear), and runs `then` before the new generation can be taken
    /// or a save be accepted. What a read of the new generation finds is then what `then`
    /// made, never what came before it.
    pub fn clear_then(&self, then: impl FnOnce()) {
        // The folder first, so a save under the new generation is written after the deletion.
        let _io = lock(&self.0.io);
        let mut state = lock(&self.0.state);
        state.generation += 1;
        state.pending.clear();
        match fs::remove_dir_all(&self.0.dir) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => log::warn!("Could not delete the kept Apple Music lists: {error}"),
        }
        then();
    }

    /// Refuses every later save, drops the ones not written yet and waits for a write being
    /// made: the folder is about to go.
    pub fn close(&self) {
        {
            let mut state = lock(&self.0.state);
            state.closed = true;
            state.pending.clear();
        }
        drop(lock(&self.0.io));
    }

    /// Writes the saves not written yet now, on this thread.
    #[doc(hidden)]
    pub fn flush(&self) {
        let pending = mem::take(&mut lock(&self.0.state).pending);
        for (name, pending) in pending {
            self.0.write(&name, pending);
        }
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
                log::warn!("Could not encode the Apple Music {name} to keep: {error}");
                return;
            }
        };
        let _io = lock(&self.io);
        // Checked while the folder cannot be deleted: a clear either came before and drops
        // this write, or comes after and deletes its file.
        {
            let state = lock(&self.state);
            if state.closed || state.generation != pending.generation {
                return;
            }
        }
        if let Err(error) = write_atomically(&self.dir, &self.path(name), &bytes) {
            log::warn!("Could not keep the Apple Music {name}: {error}");
        }
    }
}

/// Writes `bytes` to a file beside `path` and moves it into place, so `path` holds the old
/// list or the new one, never a part of either. Writes are made one at a time, so the file
/// beside it can have a fixed name.
fn write_atomically(dir: &Path, path: &Path, bytes: &[u8]) -> io::Result<()> {
    create_private_dir(dir)?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = dir.join(format!(".{name}.tmp"));
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

/// Makes the folder, readable by its owner only: the lists are the user's library.
fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Makes a file readable by its owner only.
fn create_private_file(path: &Path) -> io::Result<fs::File> {
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
    use super::*;
    use std::cell::RefCell;
    use std::env;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn saves_are_written_whole_and_read_back_by_another_store() {
        let dir = env::temp_dir().join(format!("apple-music-lists-{}", rand::random::<u64>()));
        let store = ListStore::new(&dir);
        let generation = store.generation();
        store.save(generation, "names", Arc::new(vec!["a".to_owned()]));
        store.save(generation, "names", Arc::new(vec!["b".to_owned()]));
        store.flush();
        let read = ListStore::new(&dir);
        assert_eq!(
            read.load::<Vec<String>>("names"),
            Some(vec!["b".to_owned()])
        );
        // Only the file itself is left.
        let files: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(files, ["names.json"]);

        // A save taken before a clear is not written after it.
        store.save(generation, "names", Arc::new(vec!["c".to_owned()]));
        store.clear();
        assert!(!dir.exists());
        store.save(generation, "names", Arc::new(vec!["d".to_owned()]));
        store.flush();
        assert!(!dir.exists());
        assert_eq!(read.load::<Vec<String>>("names"), None);

        // A closed store writes nothing more.
        store.save(store.generation(), "names", Arc::new(vec!["e".to_owned()]));
        store.close();
        store.flush();
        store.save(store.generation(), "names", Arc::new(vec!["f".to_owned()]));
        store.flush();
        assert!(!dir.exists());
    }

    #[test]
    fn a_new_generation_is_taken_only_after_what_a_clear_runs() {
        let dir = env::temp_dir().join(format!("apple-music-lists-{}", rand::random::<u64>()));
        let store = ListStore::new(&dir);
        let before = store.generation();
        let ran = AtomicBool::new(false);
        thread::scope(|scope| {
            let reader = RefCell::new(None);
            store.clear_then(|| {
                reader.replace(Some(scope.spawn(|| {
                    let generation = store.generation();
                    (generation, ran.load(Ordering::SeqCst))
                })));
                thread::sleep(Duration::from_millis(200));
                ran.store(true, Ordering::SeqCst);
            });
            let (generation, ran_before) = reader.take().unwrap().join().unwrap();
            assert_ne!(generation, before);
            assert!(ran_before);
        });
        store.clear();
    }

    #[cfg(unix)]
    #[test]
    fn the_folder_and_its_files_are_private_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;

        let dir = env::temp_dir().join(format!("apple-music-lists-{}", rand::random::<u64>()));
        let store = ListStore::new(&dir);
        store.save(store.generation(), "names", Arc::new(vec!["a".to_owned()]));
        store.flush();

        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("names.json")), 0o600);
        store.clear();
    }

    #[test]
    fn saves_are_written_after_the_last_handle_is_dropped() {
        let dir = env::temp_dir().join(format!("apple-music-lists-{}", rand::random::<u64>()));
        let store = ListStore::new(&dir);
        store.save(store.generation(), "names", Arc::new(vec!["a".to_owned()]));
        drop(store);

        let read = ListStore::new(&dir);
        for _ in 0..50 {
            if read.load::<Vec<String>>("names").is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(
            read.load::<Vec<String>>("names"),
            Some(vec!["a".to_owned()])
        );
        read.clear();
    }

    #[test]
    fn a_file_of_another_version_or_shape_is_ignored() {
        let dir = env::temp_dir().join(format!("apple-music-lists-{}", rand::random::<u64>()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("old.json"), r#"{"version":0,"value":["a"]}"#).unwrap();
        fs::write(dir.join("broken.json"), "{").unwrap();
        let store = ListStore::new(&dir);
        assert_eq!(store.load::<Vec<String>>("old"), None);
        assert_eq!(store.load::<Vec<String>>("broken"), None);
        assert_eq!(store.load::<Vec<String>>("missing"), None);
        store.clear();
        assert!(!dir.exists());
    }
}
