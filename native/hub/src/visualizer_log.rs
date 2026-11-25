//! The visualizer on the app's Log page, like the other parts of the app: its errors always, and
//! what it runs on (the GPU, how frames reach the screen) while the user turns the diagnostics
//! on. An entry's domain is `visualizer::<operation>`, whose last part the page shows as its
//! title, and its detail is the error or the diagnostic.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use ::database::actions::logging::{LogLevel, insert_log};
use ::database::connection::MainDbConnection;
use log::error;
use tokio::sync::mpsc;

/// How many entries wait for the database at most (it opens with the library); more are dropped.
const QUEUE: usize = 256;

/// The same entry again within this time is not written again: an error that repeats every frame
/// shows once.
const REPEAT: Duration = Duration::from_secs(60);

static DIAGNOSTICS: AtomicBool = AtomicBool::new(false);

struct Entry {
    level: LogLevel,
    domain: String,
    detail: String,
}

struct Pending {
    sender: mpsc::Sender<Entry>,
    /// Taken by the task that writes, once.
    receiver: Mutex<Option<mpsc::Receiver<Entry>>>,
    /// Whether the library open now keeps no log on this device, as a streaming service library
    /// does. What the visualizer reports meanwhile belongs to no library's log, so it is dropped
    /// instead of waiting for the next library, whether or not the writer runs yet.
    detached: AtomicBool,
}

impl Pending {
    fn new() -> Self {
        let (sender, receiver) = mpsc::channel(QUEUE);
        Self {
            sender,
            receiver: Mutex::new(Some(receiver)),
            detached: AtomicBool::new(false),
        }
    }

    fn push(&self, entry: Entry) {
        if self.detached.load(Ordering::Relaxed) {
            return;
        }
        // A full queue (the library is not open yet, or the database is slow) drops the entry.
        let _ = self.sender.try_send(entry);
    }
}

static PENDING: LazyLock<Pending> = LazyLock::new(Pending::new);

/// An error of the visualizer while doing `operation` (`load_scene`, `gpu`, ...).
pub fn error(operation: &str, detail: String) {
    queue(LogLevel::Error, operation, detail);
}

/// A diagnostic of `operation`, written while the user turned the diagnostics on.
pub fn diagnostic(operation: &str, detail: String) {
    if DIAGNOSTICS.load(Ordering::Relaxed) {
        queue(LogLevel::Info, operation, detail);
    }
}

pub fn set_diagnostics(enabled: bool) {
    DIAGNOSTICS.store(enabled, Ordering::Relaxed);
}

fn queue(level: LogLevel, operation: &str, detail: String) {
    PENDING.push(Entry {
        level,
        domain: format!("visualizer::{operation}"),
        detail,
    });
}

/// The database the writer inserts into: the library open now.
static TARGET: Mutex<Option<Arc<MainDbConnection>>> = Mutex::new(None);

/// Stop writing to the database of the library open before, for a library that keeps no log on
/// this device. Entries reported until `write_to` names a database again are dropped.
pub fn detach() {
    PENDING.detached.store(true, Ordering::Relaxed);
    *TARGET.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Write the entries to `main_db` from now on, the ones that waited first. Every call replaces
/// the database, so that entries go to the library open now; only the first call starts the
/// writer, which needs the runtime.
pub fn write_to(main_db: Arc<MainDbConnection>) {
    *TARGET.lock().unwrap_or_else(|e| e.into_inner()) = Some(main_db);
    PENDING.detached.store(false, Ordering::Relaxed);
    let receiver = PENDING
        .receiver
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    let Some(mut receiver) = receiver else {
        return;
    };
    tokio::spawn(async move {
        let mut recent: HashMap<(String, String), Instant> = HashMap::new();
        while let Some(entry) = receiver.recv().await {
            let target = TARGET.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let Some(main_db) = destination(&mut recent, &entry, target, Instant::now()) else {
                continue;
            };
            if let Err(e) = insert_log(&*main_db, entry.level, entry.domain, entry.detail).await {
                error!("Failed to log a visualizer entry: {e:#?}");
            }
        }
    });
}

/// The database `target` that `entry` is written to, or none when it is not written: while no
/// database is attached, or when the same entry was written within [`REPEAT`]. Only an entry that
/// is written counts as a repeat, so one dropped for want of a database is written once a library
/// that keeps a log is open again.
fn destination<T>(
    recent: &mut HashMap<(String, String), Instant>,
    entry: &Entry,
    target: Option<T>,
    now: Instant,
) -> Option<T> {
    let target = target?;
    (!is_repeat(recent, &entry.domain, &entry.detail, now)).then_some(target)
}

/// Whether the entry was written within [`REPEAT`] before `now`; if not, remember it as written
/// now. Entries older than the window are forgotten, so the memory stays small.
fn is_repeat(
    recent: &mut HashMap<(String, String), Instant>,
    domain: &str,
    detail: &str,
    now: Instant,
) -> bool {
    recent.retain(|_, at| now.saturating_duration_since(*at) < REPEAT);
    let key = (domain.to_string(), detail.to_string());
    if recent.contains_key(&key) {
        return true;
    }
    recent.insert(key, now);
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(detail: &str) -> Entry {
        Entry {
            level: LogLevel::Error,
            domain: "visualizer::gpu".to_owned(),
            detail: detail.to_owned(),
        }
    }

    #[test]
    fn an_entry_dropped_for_want_of_a_database_does_not_count_as_written() {
        let mut recent = HashMap::new();
        let now = Instant::now();
        assert_eq!(
            destination(&mut recent, &entry("lost"), None::<&str>, now),
            None
        );
        // Once a library that keeps a log is open, the same entry is written, and only then is it
        // a repeat.
        assert_eq!(
            destination(&mut recent, &entry("lost"), Some("library"), now),
            Some("library")
        );
        assert_eq!(
            destination(&mut recent, &entry("lost"), Some("library"), now),
            None
        );
    }

    #[test]
    fn entries_wait_for_the_first_library_but_not_through_one_that_keeps_no_log() {
        let pending = Pending::new();
        let mut receiver = pending.receiver.lock().unwrap().take().unwrap();
        // Before a library is open, an entry waits for the first one's database.
        pending.push(entry("before"));
        assert_eq!(receiver.try_recv().unwrap().detail, "before");
        // While a library that keeps no log is open, an entry is dropped instead of waiting for
        // the next library.
        pending.detached.store(true, Ordering::Relaxed);
        pending.push(entry("while detached"));
        assert!(receiver.try_recv().is_err());
        pending.detached.store(false, Ordering::Relaxed);
        pending.push(entry("after"));
        assert_eq!(receiver.try_recv().unwrap().detail, "after");
    }

    #[test]
    fn an_entry_is_written_once_per_window_even_when_two_alternate() {
        let mut recent = HashMap::new();
        let start = Instant::now();
        assert!(!is_repeat(&mut recent, "visualizer::gpu", "a", start));
        assert!(!is_repeat(&mut recent, "visualizer::gpu", "b", start));
        assert!(is_repeat(&mut recent, "visualizer::gpu", "a", start));
        assert!(is_repeat(&mut recent, "visualizer::gpu", "b", start));
        let later = start + REPEAT;
        assert!(!is_repeat(&mut recent, "visualizer::gpu", "a", later));
        assert_eq!(recent.len(), 1);
    }
}
