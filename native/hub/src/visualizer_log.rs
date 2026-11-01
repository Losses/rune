//! The visualizer on the app's Log page, like the other parts of the app: its errors always, and
//! what it runs on (the GPU, how frames reach the screen) while the user turns the diagnostics
//! on. An entry's domain is `visualizer::<operation>`, whose last part the page shows as its
//! title, and its detail is the error or the diagnostic.

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

/// Write the entries to `main_db` from now on, the ones that waited first. Only the first call
/// starts the writer; call it within the runtime.
pub fn write_to(main_db: Arc<MainDbConnection>) {
    let receiver = PENDING
        .receiver
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    let Some(mut receiver) = receiver else {
        return;
    };
    tokio::spawn(async move {
        let mut last: Option<(String, String, Instant)> = None;
        while let Some(entry) = receiver.recv().await {
            let repeated = last.as_ref().is_some_and(|(domain, detail, at)| {
                *domain == entry.domain && *detail == entry.detail && at.elapsed() < REPEAT
            });
            if repeated {
                continue;
            }
            last = Some((entry.domain.clone(), entry.detail.clone(), Instant::now()));
            if let Err(e) = insert_log(&*main_db, entry.level, entry.domain, entry.detail).await {
                error!("Failed to log a visualizer entry: {e:#?}");
            }
        }
    });
}
