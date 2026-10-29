//! Work done on another thread while the render loop keeps drawing: a scene is built (files read,
//! images decoded, shaders compiled, which takes seconds on some drivers) without freezing the
//! picture, and the loop takes the result at the start of a frame.

use std::io;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

/// Loads running on threads of their own, of which only the newest counts: a result that comes
/// in after a newer load was started is dropped. Threads are not cancelled (a driver compile
/// cannot be), they finish and their result is ignored.
pub struct Loads<T> {
    sender: Sender<(u64, T)>,
    receiver: Receiver<(u64, T)>,
    /// The number of the newest load started, counting from 1.
    newest: u64,
    /// Whether the newest load has not been taken yet.
    pending: bool,
}

impl<T: Send + 'static> Loads<T> {
    pub fn new() -> Self {
        let (sender, receiver) = channel();
        Self {
            sender,
            receiver,
            newest: 0,
            pending: false,
        }
    }

    /// Run `work` on a new thread. Its result replaces whatever an older load is still doing.
    pub fn start(&mut self, work: impl FnOnce() -> T + Send + 'static) -> io::Result<()> {
        let number = self.newest + 1;
        let sender = self.sender.clone();
        thread::Builder::new()
            .name("visualizer-load".into())
            .spawn(move || {
                // The receiver is gone when the visualizer stopped meanwhile.
                let _ = sender.send((number, work()));
            })?;
        self.newest = number;
        self.pending = true;
        Ok(())
    }

    /// Whether the newest load is still running.
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// Forget the load that is running, if any: its result is dropped when it comes in, as if a
    /// newer load had replaced it.
    pub fn discard(&mut self) {
        if self.pending {
            self.newest += 1;
            self.pending = false;
        }
    }

    /// The result of the newest load if it has finished, once. Results of older loads that came
    /// in are dropped.
    pub fn take(&mut self) -> Option<T> {
        let mut found = None;
        while let Ok((number, result)) = self.receiver.try_recv() {
            if number == self.newest {
                found = Some(result);
            }
        }
        if found.is_some() {
            self.pending = false;
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::sync_channel;
    use std::time::{Duration, Instant};

    use super::*;

    /// Wait for the newest load of `loads`.
    fn wait<T: Send + 'static>(loads: &mut Loads<T>) -> T {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(result) = loads.take() {
                return result;
            }
            assert!(Instant::now() < until, "the load did not finish");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_finished_load_is_taken_once() {
        let mut loads = Loads::new();
        assert!(!loads.is_pending());
        loads.start(|| 7).unwrap();
        assert!(loads.is_pending());
        assert_eq!(wait(&mut loads), 7);
        assert!(!loads.is_pending());
        assert_eq!(loads.take(), None);
    }

    #[test]
    fn a_newer_load_supersedes_an_older_one_that_finishes_later() {
        let mut loads = Loads::new();
        let (release, held) = sync_channel::<()>(0);
        loads
            .start(move || {
                held.recv().unwrap();
                "old"
            })
            .unwrap();
        loads.start(|| "new").unwrap();
        assert_eq!(wait(&mut loads), "new");
        // The old load finishes now; its result is dropped and nothing is pending.
        release.send(()).unwrap();
        thread::sleep(Duration::from_millis(50));
        assert_eq!(loads.take(), None);
        assert!(!loads.is_pending());
    }

    #[test]
    fn a_discarded_load_never_shows() {
        let mut loads = Loads::new();
        let (release, held) = sync_channel::<()>(0);
        loads
            .start(move || {
                held.recv().unwrap();
                "old"
            })
            .unwrap();
        loads.discard();
        assert!(!loads.is_pending());
        release.send(()).unwrap();
        thread::sleep(Duration::from_millis(50));
        assert_eq!(loads.take(), None);
        // A load started after it works as usual.
        loads.start(|| "new").unwrap();
        assert_eq!(wait(&mut loads), "new");
    }

    #[test]
    fn an_older_result_never_shows_while_the_newer_load_runs() {
        let mut loads = Loads::new();
        let (release, held) = sync_channel::<()>(0);
        loads.start(|| "old").unwrap();
        loads
            .start(move || {
                held.recv().unwrap();
                "new"
            })
            .unwrap();
        thread::sleep(Duration::from_millis(50));
        assert_eq!(loads.take(), None);
        assert!(loads.is_pending());
        release.send(()).unwrap();
        assert_eq!(wait(&mut loads), "new");
    }
}
