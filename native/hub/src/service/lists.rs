//! One cache for the lists a streaming service's library reads from its service, such as its
//! songs, albums and artists.
//!
//! The binding declares each list with its own age limit and how it is read. A caller gets the
//! kept list at once. A list past its age limit is still given at once, and read again in the
//! background. One read of a list runs at a time, and the callers that need the list meanwhile
//! join it. What a read brings is kept on disk in the library's folder, a file for each list, so a
//! library opened again answers from its lists before its service does, and also while the
//! service cannot be reached.
//!
//! The lists keep the service's own ids. The integer ids of a response are given when a list is
//! read, so nothing kept depends on the numbers one binding handed out.
//!
//! A binding holds one sign-in. A sign-in known to be another account clears the lists, on disk
//! too, and what the reads in flight bring is not kept (see [`ServiceLists::clear`]). A sign-in
//! renewed for the same account keeps them, and so does a sign-in that is lost or expires: the
//! lists are still given, and read again once the service takes a sign-in again. Where a binding
//! cannot tell whether the account changed, it keeps them, and the first read after the sign-in
//! replaces them.

use std::fmt;
use std::io;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::fs as tokio_fs;
use tokio::sync::OnceCell;
use tokio::task;
use uuid::Uuid;

use crate::service::folder::service_library_folder;
use crate::service::store::{ListStore, StoreGeneration};

/// How long after a read in the background failed no other starts, so a service that cannot be
/// reached is not asked again for every caller.
const RETRY_AFTER: Duration = Duration::from_secs(60);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The folder in which the service library `library_id` keeps its lists.
pub(crate) fn lists_folder(config_dir: &Path, library_id: Uuid) -> PathBuf {
    service_library_folder(config_dir, library_id).join("lists")
}

/// Deletes the lists the service library `library_id` keeps, as it signs in to another account
/// while it is closed. An open library's binding clears its own (see [`ServiceLists::clear`]).
pub(crate) async fn delete_kept_lists(config_dir: &Path, library_id: Uuid) -> io::Result<()> {
    match tokio_fs::remove_dir_all(lists_folder(config_dir, library_id)).await {
        Err(error) if error.kind() != ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// A list a binding keeps: the name of its file, and how long it stays current once read.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ListSpec {
    pub(crate) name: &'static str,
    pub(crate) max_age: Duration,
}

/// Reads a list from the service.
type Read<T, E> = Arc<dyn Fn() -> BoxFuture<'static, Result<T, E>> + Send + Sync>;

/// A read in flight, which every caller that needs the list joins.
type Reading<T, E> = Shared<BoxFuture<'static, Result<Arc<T>, E>>>;

/// Hears that what the lists hold changed.
type Changed = Arc<dyn Fn() + Send + Sync>;

/// A list that can be forgotten, whatever it holds.
trait Forget: Send + Sync {
    fn forget(&self);
}

/// The lists one service library keeps, and the folder that keeps them on disk.
pub(crate) struct ServiceLists {
    store: ListStore,
    changed: Changed,
    lists: Mutex<Vec<Weak<dyn Forget>>>,
}

impl ServiceLists {
    /// The lists kept in `dir`. `changed` is called whenever a read brings a list other than the
    /// one kept, and when the lists are cleared, so what shows them can read them again. It is
    /// called on the task of the read, and must not block.
    pub(crate) fn new(dir: PathBuf, changed: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            store: ListStore::new(dir),
            changed: Arc::new(changed),
            lists: Mutex::default(),
        }
    }

    /// Declares the list `spec` names, which `read` reads from the service.
    pub(crate) fn declare<T, E>(
        &self,
        spec: ListSpec,
        read: impl Fn() -> BoxFuture<'static, Result<T, E>> + Send + Sync + 'static,
    ) -> KeptList<T, E>
    where
        T: Serialize + DeserializeOwned + PartialEq + Send + Sync + 'static,
        E: Clone + fmt::Display + Send + Sync + 'static,
    {
        let list = Arc::new(Inner {
            spec,
            read: Arc::new(read),
            store: self.store.clone(),
            changed: Arc::clone(&self.changed),
            restored: OnceCell::new(),
            state: Mutex::new(State {
                epoch: 0,
                kept: None,
                reading: None,
                failed: None,
            }),
        });
        let forget = Arc::downgrade(&list);
        lock(&self.lists).push(forget);
        KeptList(list)
    }

    /// Forgets every list, on disk too, as the account signed in has changed. What the reads in
    /// flight bring may belong to the account signed in before, so it is not kept, though the
    /// callers that wait for such a read still get its list.
    pub(crate) fn clear(&self) {
        let lists: Vec<_> = lock(&self.lists).iter().filter_map(Weak::upgrade).collect();
        // The disk first, so a list read from disk after this is never one of the old account.
        self.store.clear_then(|| {
            for list in &lists {
                list.forget();
            }
        });
        (self.changed)();
    }
}

/// One list a service library keeps, as the module says.
pub(crate) struct KeptList<T, E>(Arc<Inner<T, E>>);

struct Inner<T, E> {
    spec: ListSpec,
    read: Read<T, E>,
    store: ListStore,
    changed: Changed,
    /// Set once the list kept on disk was read, or found missing.
    restored: OnceCell<()>,
    state: Mutex<State<T, E>>,
}

struct State<T, E> {
    /// Counts the times the list was forgotten, so what a read that began before brings is not
    /// kept.
    epoch: u64,
    kept: Option<Kept<T>>,
    reading: Option<Reading<T, E>>,
    /// When the last read in the background that failed began.
    failed: Option<Instant>,
}

struct Kept<T> {
    value: Arc<T>,
    /// When the read that brought it began, or none for a list read from disk, which is old from
    /// the start.
    read_at: Option<Instant>,
}

impl<T, E> KeptList<T, E>
where
    T: Serialize + DeserializeOwned + PartialEq + Send + Sync + 'static,
    E: Clone + fmt::Display + Send + Sync + 'static,
{
    /// The list at `now`: the kept one at once while it is current, and also once it is past its
    /// age limit, when it is read again in the background. Only a list never kept waits for a
    /// read, joining one in flight. The callers that waited for a read that failed get its
    /// failure; a later caller reads again.
    pub(crate) async fn get(&self, now: Instant) -> Result<Arc<T>, E> {
        self.restore().await;
        let generation = self.0.store.generation();
        let reading = {
            let mut state = lock(&self.0.state);
            if let Some(list) = self.kept_locked(&mut state, now, generation) {
                return Ok(list);
            }
            self.reading_locked(&mut state, now, generation)
        };
        reading.await
    }

    /// The kept list at `now`, however old, as [`get`](Self::get) gives it, or `None` without a
    /// read when none is kept.
    pub(crate) async fn kept(&self, now: Instant) -> Option<Arc<T>> {
        self.restore().await;
        let generation = self.0.store.generation();
        let mut state = lock(&self.0.state);
        self.kept_locked(&mut state, now, generation)
    }

    /// The list as the service has it at `now`, never one kept from before: the read in flight
    /// is joined, else one starts. What it reads is kept.
    pub(crate) async fn current(&self, now: Instant) -> Result<Arc<T>, E> {
        self.restore().await;
        let generation = self.0.store.generation();
        let reading = self.reading_locked(&mut lock(&self.0.state), now, generation);
        reading.await
    }

    /// The kept list, however old. One past its age limit is read again in the background,
    /// unless a read runs or one failed a short while ago.
    fn kept_locked(
        &self,
        state: &mut State<T, E>,
        now: Instant,
        generation: StoreGeneration,
    ) -> Option<Arc<T>> {
        let kept = state.kept.as_ref()?;
        let list = Arc::clone(&kept.value);
        let current = kept
            .read_at
            .is_some_and(|read_at| now.saturating_duration_since(read_at) < self.0.spec.max_age);
        let retrying = state
            .failed
            .is_some_and(|failed| now.saturating_duration_since(failed) < RETRY_AFTER);
        if !current && !retrying && state.reading.is_none() {
            state.reading = Some(self.start(state.epoch, now, generation, true));
        }
        Some(list)
    }

    /// The read in flight, or a new one.
    fn reading_locked(
        &self,
        state: &mut State<T, E>,
        now: Instant,
        generation: StoreGeneration,
    ) -> Reading<T, E> {
        let epoch = state.epoch;
        state
            .reading
            .get_or_insert_with(|| self.start(epoch, now, generation, false))
            .clone()
    }

    /// Starts a read at `now`, which keeps what it brings unless the list was forgotten since
    /// `epoch`, and saves it under `generation`. `background` says that nobody waits for it, so
    /// its failure holds the next read in the background back.
    fn start(
        &self,
        epoch: u64,
        now: Instant,
        generation: StoreGeneration,
        background: bool,
    ) -> Reading<T, E> {
        let list = Arc::downgrade(&self.0);
        let read = (self.0.read)();
        let reading = async move {
            let read = read.await.map(Arc::new);
            if let Some(list) = list.upgrade() {
                list.finish(epoch, generation, now, background, &read);
            }
            read
        }
        .boxed()
        .shared();
        // Driven to its end whoever waits for it, so what it brings is kept also when the caller
        // that started it stops waiting.
        tokio::spawn(reading.clone());
        reading
    }

    /// Reads the list kept on disk into memory, once, off the runtime. A list forgotten meanwhile,
    /// or one a read brought already, keeps what it has.
    async fn restore(&self) {
        self.0
            .restored
            .get_or_init(|| async {
                let epoch = lock(&self.0.state).epoch;
                let store = self.0.store.clone();
                let name = self.0.spec.name;
                let Ok(Some(list)) = task::spawn_blocking(move || store.load::<T>(name)).await
                else {
                    return;
                };
                let mut state = lock(&self.0.state);
                if state.epoch == epoch && state.kept.is_none() {
                    state.kept = Some(Kept {
                        value: Arc::new(list),
                        read_at: None,
                    });
                }
            })
            .await;
    }
}

impl<T, E> Inner<T, E>
where
    T: Serialize + PartialEq + Send + Sync + 'static,
    E: fmt::Display,
{
    /// Keeps what a read that began at `now` under `epoch` brought, unless the list was forgotten
    /// since, and saves it under `generation`.
    fn finish(
        &self,
        epoch: u64,
        generation: StoreGeneration,
        now: Instant,
        background: bool,
        read: &Result<Arc<T>, E>,
    ) {
        let (new, changed) = {
            let mut state = lock(&self.state);
            if state.epoch != epoch {
                return;
            }
            state.reading = None;
            match read {
                Ok(list) => {
                    state.failed = None;
                    // A list the same as the kept one is on disk already.
                    let new = state.kept.as_ref().is_none_or(|kept| *kept.value != **list);
                    let changed = new && state.kept.is_some();
                    state.kept = Some(Kept {
                        value: Arc::clone(list),
                        read_at: Some(now),
                    });
                    (new, changed)
                }
                Err(error) => {
                    if background {
                        log::info!("Could not read the {} again: {error}", self.spec.name);
                        state.failed = Some(now);
                    }
                    (false, false)
                }
            }
        };
        if new && let Ok(list) = read {
            self.store
                .save(generation, self.spec.name, Arc::clone(list));
        }
        if changed {
            (self.changed)();
        }
    }
}

impl<T, E> Forget for Inner<T, E>
where
    T: Send + Sync,
    E: Send + Sync,
{
    fn forget(&self) {
        let mut state = lock(&self.state);
        state.epoch += 1;
        state.kept = None;
        state.reading = None;
        state.failed = None;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use futures::FutureExt;
    use futures::future::{BoxFuture, Shared};
    use tempfile::TempDir;
    use tokio::sync::Semaphore;

    use super::{KeptList, ListSpec, RETRY_AFTER, ServiceLists};
    use crate::test_wait::wait_until;

    const AGE: Duration = Duration::from_secs(60);

    const NUMBERS: ListSpec = ListSpec {
        name: "numbers",
        max_age: AGE,
    };

    /// What the fake service answers each read with, and how many reads it got.
    struct Service {
        reads: AtomicU32,
        /// The number the next reads bring; a read fails while it is zero.
        answer: AtomicU32,
        /// Holds every read until the test lets it end, when set.
        gate: Option<Semaphore>,
    }

    impl Service {
        fn new(answer: u32) -> Arc<Self> {
            Arc::new(Self {
                reads: AtomicU32::new(0),
                answer: AtomicU32::new(answer),
                gate: None,
            })
        }

        fn gated(answer: u32) -> Arc<Self> {
            Arc::new(Self {
                reads: AtomicU32::new(0),
                answer: AtomicU32::new(answer),
                gate: Some(Semaphore::new(0)),
            })
        }

        fn reads(&self) -> u32 {
            self.reads.load(Ordering::SeqCst)
        }

        fn answer(&self, answer: u32) {
            self.answer.store(answer, Ordering::SeqCst);
        }

        /// Lets `count` held reads end.
        fn release(&self, count: usize) {
            self.gate.as_ref().unwrap().add_permits(count);
        }

        fn read(self: &Arc<Self>) -> BoxFuture<'static, Result<u32, String>> {
            let service = Arc::clone(self);
            async move {
                service.reads.fetch_add(1, Ordering::SeqCst);
                if let Some(gate) = &service.gate {
                    gate.acquire().await.unwrap().forget();
                }
                match service.answer.load(Ordering::SeqCst) {
                    0 => Err("down".to_owned()),
                    answer => Ok(answer),
                }
            }
            .boxed()
        }
    }

    fn declare(lists: &ServiceLists, service: &Arc<Service>) -> KeptList<u32, String> {
        let service = Arc::clone(service);
        lists.declare(NUMBERS, move || service.read())
    }

    fn kept_in(root: &TempDir) -> ServiceLists {
        ServiceLists::new(root.path().join("lists"), || {})
    }

    /// Waits until `callers` wait for the read in flight, besides the list and the task that
    /// drives the read.
    async fn joined_by(list: &KeptList<u32, String>, callers: usize) {
        wait_until("the callers to join the read", || async {
            let handles = list
                .0
                .state
                .lock()
                .unwrap()
                .reading
                .as_ref()
                .and_then(Shared::strong_count);
            Ok((handles == Some(callers + 2)).then_some(()))
        })
        .await
        .unwrap();
    }

    /// Waits until the list kept at `now` is `expected`.
    async fn kept_becomes(list: &KeptList<u32, String>, now: Instant, expected: u32) {
        wait_until("the kept list", || async {
            Ok((list.kept(now).await.as_deref() == Some(&expected)).then_some(()))
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_list_past_its_age_is_given_at_once_and_read_again_in_the_background() {
        let root = TempDir::new().unwrap();
        let lists = kept_in(&root);
        let service = Service::new(1);
        let list = declare(&lists, &service);
        let start = Instant::now();

        assert_eq!(*list.get(start).await.unwrap(), 1);
        service.answer(2);
        // Current, it is given without a read.
        assert_eq!(*list.get(start + AGE / 2).await.unwrap(), 1);
        assert_eq!(service.reads(), 1);
        // Past its age, it is still given at once, and read again.
        assert_eq!(*list.get(start + AGE).await.unwrap(), 1);
        kept_becomes(&list, start + AGE, 2).await;
        assert_eq!(service.reads(), 2);
        // The list read again counts its age from its own read.
        assert_eq!(*list.get(start + AGE + AGE / 2).await.unwrap(), 2);
        assert_eq!(service.reads(), 2);
    }

    #[tokio::test]
    async fn callers_that_need_a_list_share_one_read_and_its_failure() {
        let root = TempDir::new().unwrap();
        let lists = Arc::new(kept_in(&root));
        let service = Service::gated(0);
        let list = Arc::new(declare(&lists, &service));
        let start = Instant::now();
        let callers: Vec<_> = (0..5)
            .map(|_| {
                let list = Arc::clone(&list);
                tokio::spawn(async move { list.get(start).await })
            })
            .collect();
        joined_by(&list, 5).await;
        service.release(1);
        for caller in callers {
            assert_eq!(caller.await.unwrap().unwrap_err(), "down");
        }
        assert_eq!(service.reads(), 1);

        // A caller that comes later reads again.
        service.answer(7);
        service.release(1);
        assert_eq!(*list.get(start).await.unwrap(), 7);
        assert_eq!(service.reads(), 2);
    }

    #[tokio::test]
    async fn a_failed_read_in_the_background_holds_the_next_back_and_the_list_stays() {
        let root = TempDir::new().unwrap();
        let lists = kept_in(&root);
        let service = Service::new(1);
        let list = declare(&lists, &service);
        let start = Instant::now();
        assert_eq!(*list.get(start).await.unwrap(), 1);

        service.answer(0);
        let old = start + AGE;
        assert_eq!(*list.get(old).await.unwrap(), 1);
        wait_until("the failed read", || async {
            Ok(
                (service.reads() == 2 && list.0.state.lock().unwrap().reading.is_none())
                    .then_some(()),
            )
        })
        .await
        .unwrap();
        // The old list is still given, and no read starts until a while after the failure.
        assert_eq!(*list.get(old + RETRY_AFTER / 2).await.unwrap(), 1);
        assert_eq!(service.reads(), 2);
        service.answer(3);
        assert_eq!(*list.get(old + RETRY_AFTER).await.unwrap(), 1);
        kept_becomes(&list, old + RETRY_AFTER, 3).await;
        assert_eq!(service.reads(), 3);
    }

    #[tokio::test]
    async fn the_current_list_joins_the_read_in_flight_and_is_kept() {
        let root = TempDir::new().unwrap();
        let lists = kept_in(&root);
        let service = Service::gated(1);
        let list = Arc::new(declare(&lists, &service));
        let start = Instant::now();
        let first = {
            let list = Arc::clone(&list);
            tokio::spawn(async move { list.get(start).await })
        };
        joined_by(&list, 1).await;
        let joined = {
            let list = Arc::clone(&list);
            tokio::spawn(async move { list.current(start).await })
        };
        joined_by(&list, 2).await;
        service.release(1);
        assert_eq!(*first.await.unwrap().unwrap(), 1);
        assert_eq!(*joined.await.unwrap().unwrap(), 1);
        assert_eq!(service.reads(), 1);

        // With no read in flight, the current list is read, however young the kept one is.
        service.answer(2);
        service.release(1);
        assert_eq!(*list.current(start).await.unwrap(), 2);
        assert_eq!(service.reads(), 2);
        assert_eq!(*list.get(start).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn a_list_kept_on_disk_is_given_at_once_as_old_also_while_the_service_is_down() {
        let root = TempDir::new().unwrap();
        let start = Instant::now();
        {
            let lists = kept_in(&root);
            let list = declare(&lists, &Service::new(1));
            assert_eq!(*list.get(start).await.unwrap(), 1);
        }
        let file = root.path().join("lists").join("numbers.json");
        wait_until("the kept list on disk", || async {
            Ok(file.is_file().then_some(()))
        })
        .await
        .unwrap();

        // Opened again with the service down, the list comes from disk, and the read in the
        // background fails without failing the caller.
        let lists = kept_in(&root);
        let service = Service::new(0);
        let list = declare(&lists, &service);
        assert_eq!(*list.get(start).await.unwrap(), 1);
        wait_until("the read in the background", || async {
            Ok((service.reads() == 1).then_some(()))
        })
        .await
        .unwrap();
        assert_eq!(*list.get(start).await.unwrap(), 1);

        // Once the service answers, the list read again replaces it.
        let lists = kept_in(&root);
        let service = Service::new(5);
        let list = declare(&lists, &service);
        assert_eq!(*list.get(start).await.unwrap(), 1);
        kept_becomes(&list, start, 5).await;
    }

    #[tokio::test]
    async fn clearing_forgets_the_lists_and_what_a_read_in_flight_brings() {
        let root = TempDir::new().unwrap();
        let changes = Arc::new(AtomicUsize::new(0));
        let lists = {
            let changes = Arc::clone(&changes);
            ServiceLists::new(root.path().join("lists"), move || {
                changes.fetch_add(1, Ordering::SeqCst);
            })
        };
        let service = Service::gated(1);
        let list = Arc::new(declare(&lists, &service));
        let start = Instant::now();
        service.release(1);
        assert_eq!(*list.get(start).await.unwrap(), 1);
        let file = root.path().join("lists").join("numbers.json");
        wait_until("the kept list on disk", || async {
            Ok(file.is_file().then_some(()))
        })
        .await
        .unwrap();

        // A read of the old account is in flight when another account signs in.
        service.answer(2);
        let reading = {
            let list = Arc::clone(&list);
            tokio::spawn(async move { list.current(start).await })
        };
        wait_until("the read", || async {
            Ok((service.reads() == 2).then_some(()))
        })
        .await
        .unwrap();
        lists.clear();
        assert!(!file.exists());
        assert_eq!(changes.load(Ordering::SeqCst), 1);
        // Its caller still gets its list, which is not kept.
        service.release(1);
        assert_eq!(*reading.await.unwrap().unwrap(), 2);
        assert_eq!(list.kept(start).await, None);

        // The next caller reads the list of the account signed in now.
        service.answer(3);
        service.release(1);
        assert_eq!(*list.get(start).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn only_a_read_that_brings_another_list_tells_of_a_change() {
        let root = TempDir::new().unwrap();
        let changes = Arc::new(AtomicUsize::new(0));
        let lists = {
            let changes = Arc::clone(&changes);
            ServiceLists::new(root.path().join("lists"), move || {
                changes.fetch_add(1, Ordering::SeqCst);
            })
        };
        let service = Service::new(1);
        let list = declare(&lists, &service);
        let start = Instant::now();
        // The first list is no change: its caller gets it.
        list.get(start).await.unwrap();
        list.current(start).await.unwrap();
        assert_eq!(changes.load(Ordering::SeqCst), 0);
        service.answer(2);
        list.current(start).await.unwrap();
        assert_eq!(changes.load(Ordering::SeqCst), 1);
    }
}
