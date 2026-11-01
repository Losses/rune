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
}

static PENDING: LazyLock<Pending> = LazyLock::new(|| {
    let (sender, receiver) = mpsc::channel(QUEUE);
    Pending {
        sender,
        receiver: Mutex::new(Some(receiver)),
    }
});

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
    // A full queue (the library is not open yet, or the database is slow) drops the entry.
    let _ = PENDING.sender.try_send(Entry {
        level,
        domain: format!("visualizer::{operation}"),
        detail,
    });
}

/// The database the writer inserts into: the library open now.
static TARGET: Mutex<Option<Arc<MainDbConnection>>> = Mutex::new(None);

/// Write the entries to `main_db` from now on, the ones that waited first. Every call replaces
/// the database, so that entries go to the library open now; only the first call starts the
/// writer, which needs the runtime.
pub fn write_to(main_db: Arc<MainDbConnection>) {
    *TARGET.lock().unwrap_or_else(|e| e.into_inner()) = Some(main_db);
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
            if is_repeat(&mut recent, &entry.domain, &entry.detail, Instant::now()) {
                continue;
            }
            let main_db = TARGET.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let Some(main_db) = main_db else {
                continue;
            };
            if let Err(e) = insert_log(&*main_db, entry.level, entry.domain, entry.detail).await {
                error!("Failed to log a visualizer entry: {e:#?}");
            }
        }
    });
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
