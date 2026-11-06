//! iOS: the Now Playing info on the lock screen and in Control Center, and the remote commands
//! that come from there, in place of souvlaki's `MediaControls`. Souvlaki's other types are used
//! as they are, so the hub drives this the way it drives souvlaki elsewhere.
//!
//! The state that is shown, the metadata, the elapsed time, the rate and the artwork, is Rust's
//! and sits behind a lock. Every write builds the whole dictionary from it and sets it on
//! `MPNowPlayingInfoCenter`, so no write reads the center's dictionary back and none undoes
//! another. The rate is always given: without it the center assumes 1.0 and moves a paused
//! track's progress on.
//!
//! The hub reports playback about ten times a second. The center moves the elapsed time on by
//! itself at the rate it was given, so a report is only written when the rate changes, when the
//! position jumps away from where the center has moved it, as after a seek, or when new metadata
//! came. [`ShownPlayback`] decides that in plain Rust, built on every platform for its tests.
//!
//! Threads. Every MediaPlayer call happens on the main thread: registering and removing the
//! command targets, enabling and disabling the commands, making the artwork object and setting
//! the dictionary. Made from the hub's thread, which runs no run loop, they did not reliably reach
//! the system, and Control Center showed the commands disabled. The hub calls `MediaControls` from
//! that thread, and artwork loads on a thread of its own, so `MediaControls` keeps the state and
//! decides the writes on the caller's thread, under the lock, and queues each MediaPlayer call on
//! the main queue with a copy of what it needs.
//!
//! The main queue runs its work one at a time, in the order it was queued. Writes are queued while
//! the lock is held, so they are queued in the order they were decided, and the elapsed time each
//! carries is the one decided then. A call is never made right away, even on the main thread, so
//! none overtakes one queued before it, and of an attach and a detach the one made last holds.
//! What only the main thread touches, the targets and the artwork object, sits in `OnMain`.
//!
//! Images load off the main thread, as `UIImage` is sendable (`NS_SWIFT_SENDABLE` in UIKit's
//! header). A command handler only calls the hub's handler, which is `Send` but not `Sync`, under a
//! lock of its own, so it does not matter which thread the command center calls it on.

use std::time::{Duration, Instant};

use crate::MediaPlayback;

/// How far a reported position may be from where the center has moved the elapsed time before it
/// counts as a jump, such as a seek. Reports come ten times a second and the audio keeps close to
/// the wall clock, so only a jump, or playback that stalls, goes past it.
const JUMP: Duration = Duration::from_secs(1);

/// The playback the center shows: the rate, and the elapsed time at the moment it was written,
/// from which the center moves it on at that rate.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ShownPlayback {
    rate: f64,
    elapsed: Duration,
    at: Instant,
    // New metadata came since a report was last written, so the next one is written whatever it
    // says.
    stale: bool,
}

impl ShownPlayback {
    fn new(now: Instant) -> Self {
        Self {
            rate: 0.0,
            elapsed: Duration::ZERO,
            at: now,
            stale: true,
        }
    }

    /// Where the center has moved the elapsed time to by `now`.
    fn elapsed_at(&self, now: Instant) -> Duration {
        self.elapsed + now.saturating_duration_since(self.at).mul_f64(self.rate)
    }

    /// Takes a report of the player's playback at `now`, and returns whether the center needs it.
    /// One that it needs becomes what the center shows. A report without a position, or one of a
    /// stopped player, keeps the elapsed time where the center has it.
    fn report(&mut self, playback: &MediaPlayback, now: Instant) -> bool {
        let (rate, progress) = match playback {
            MediaPlayback::Playing { progress } => (1.0, progress.map(|p| p.0)),
            MediaPlayback::Paused { progress } => (0.0, progress.map(|p| p.0)),
            MediaPlayback::Stopped => (0.0, None),
        };
        let elapsed = progress.unwrap_or_else(|| self.elapsed_at(now));
        let needed =
            self.stale || rate != self.rate || self.elapsed_at(now).abs_diff(elapsed) > JUMP;
        if needed {
            *self = Self {
                rate,
                elapsed,
                at: now,
                stale: false,
            };
        }
        needed
    }

    /// A new item starts from its beginning, at the same rate, until the player reports
    /// otherwise.
    fn new_item(&mut self, now: Instant) {
        *self = Self {
            rate: self.rate,
            elapsed: Duration::ZERO,
            at: now,
            stale: true,
        };
    }

    /// The rate and elapsed time for a write at `now`. The center measures the elapsed time from
    /// the write on, so it is moved on to `now` first.
    #[cfg(target_os = "ios")]
    fn write_at(&mut self, now: Instant) -> (f64, Duration) {
        self.elapsed = self.elapsed_at(now);
        self.at = now;
        (self.rate, self.elapsed)
    }
}

#[cfg(target_os = "ios")]
pub(crate) use ios::MediaControls;

#[cfg(target_os = "ios")]
mod ios {
    use std::cell::RefCell;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::ptr::NonNull;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
    use std::thread;
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use dispatch2::DispatchQueue;
    use log::{debug, error, warn};
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{AnyThread, MainThreadMarker, msg_send};
    use objc2_core_foundation::CGSize;
    use objc2_foundation::{NSData, NSMutableDictionary, NSNumber, NSString};
    use objc2_media_player::{
        MPChangePlaybackPositionCommandEvent, MPMediaItemArtwork, MPMediaItemPropertyAlbumTitle,
        MPMediaItemPropertyArtist, MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration,
        MPMediaItemPropertyTitle, MPNowPlayingInfoCenter,
        MPNowPlayingInfoPropertyElapsedPlaybackTime, MPNowPlayingInfoPropertyPlaybackRate,
        MPRemoteCommand, MPRemoteCommandCenter, MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
    };
    use objc2_ui_kit::UIImage;
    use souvlaki::{
        Error, MediaControlEvent, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    };

    use super::ShownPlayback;

    /// What the lock screen shows. Every write hands the main thread a copy of it.
    struct State {
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        duration: Option<Duration>,
        image: Option<Retained<UIImage>>,
        // Counts the metadata, so that artwork that loads after newer metadata came is dropped.
        generation: u64,
        playback: ShownPlayback,
    }

    /// The contents of the center's dictionary at one moment, for a write on the main thread.
    struct Info {
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        duration: Option<Duration>,
        image: Option<Retained<UIImage>>,
        rate: f64,
        elapsed: Duration,
    }

    impl State {
        /// What the center shows from `now` on.
        fn info(&mut self, now: Instant) -> Info {
            let (rate, elapsed) = self.playback.write_at(now);
            Info {
                title: self.title.clone(),
                artist: self.artist.clone(),
                album: self.album.clone(),
                duration: self.duration,
                image: self.image.clone(),
                rate,
                elapsed,
            }
        }
    }

    /// What only the main thread touches: the targets that `attach` added, each with its command,
    /// and the artwork object for the image shown, so that every write hands the center the same
    /// one.
    #[derive(Default)]
    struct MainState {
        targets: Vec<(Retained<MPRemoteCommand>, Retained<AnyObject>)>,
        artwork: Option<(Retained<UIImage>, Retained<MPMediaItemArtwork>)>,
    }

    /// A [`MainState`] that may move to any thread but is only reached on the main thread.
    struct OnMain(RefCell<MainState>);

    // SAFETY: the state is only reached through `get`, which takes a `MainThreadMarker`, and a
    // drop elsewhere hands it to the main thread.
    unsafe impl Send for OnMain {}
    // SAFETY: as above.
    unsafe impl Sync for OnMain {}

    impl OnMain {
        fn get(&self, _mtm: MainThreadMarker) -> &RefCell<MainState> {
            &self.0
        }
    }

    impl Drop for OnMain {
        fn drop(&mut self) {
            if MainThreadMarker::new().is_some() {
                return;
            }
            struct AssertSend(MainState);
            // SAFETY: the state only moves to the main thread, where it is dropped.
            unsafe impl Send for AssertSend {}
            let state = AssertSend(self.0.take());
            DispatchQueue::main().exec_async(move || {
                let state = state;
                drop(state.0);
            });
        }
    }

    /// Runs `f` on the main thread, from the main queue. It is queued even when this is the main
    /// thread, so that no call overtakes one queued before it.
    fn on_main(f: impl FnOnce(MainThreadMarker) + Send + 'static) {
        DispatchQueue::main().exec_async(move || {
            // SAFETY: the main queue runs its work on the main thread.
            let mtm = unsafe { MainThreadMarker::new_unchecked() };
            // A panic must not unwind into the dispatch queue.
            if catch_unwind(AssertUnwindSafe(|| f(mtm))).is_err() {
                error!("A Now Playing call on the main thread panicked");
            }
        });
    }

    /// A handle to the lock screen's Now Playing info and remote commands, with the API of
    /// souvlaki's `MediaControls`.
    pub struct MediaControls {
        state: Arc<Mutex<State>>,
        main: Arc<OnMain>,
    }

    impl MediaControls {
        pub fn new(_config: PlatformConfig<'_>) -> Result<Self, Error> {
            let state = State {
                title: None,
                artist: None,
                album: None,
                duration: None,
                image: None,
                generation: 0,
                playback: ShownPlayback::new(Instant::now()),
            };
            Ok(Self {
                state: Arc::new(Mutex::new(state)),
                main: Arc::new(OnMain(RefCell::new(MainState::default()))),
            })
        }

        /// Sends the events of the remote commands that souvlaki enables to `event_handler`. The
        /// other commands are left as they are.
        pub fn attach<F>(&mut self, event_handler: F) -> Result<(), Error>
        where
            F: Fn(MediaControlEvent) + Send + 'static,
        {
            self.detach()?;
            let handler = Arc::new(Mutex::new(event_handler));
            let main = Arc::clone(&self.main);
            on_main(move |mtm| main.get(mtm).borrow_mut().attach(&handler));
            Ok(())
        }

        /// Removes the targets that `attach` added and disables their commands.
        pub fn detach(&mut self) -> Result<(), Error> {
            let main = Arc::clone(&self.main);
            on_main(move |mtm| main.get(mtm).borrow_mut().detach());
            Ok(())
        }

        /// Shows the player's playback, if it is not what the lock screen shows already.
        pub fn set_playback(&mut self, playback: MediaPlayback) -> Result<(), Error> {
            let now = Instant::now();
            let mut state = lock(&self.state);
            if state.playback.report(&playback, now) {
                write(&self.main, &mut state, now);
            }
            Ok(())
        }

        /// Shows a new item, without artwork until its cover has loaded on a thread of its own.
        /// `cover_url` is a plain path to an image file on iOS.
        pub fn set_metadata(&mut self, metadata: MediaMetadata<'_>) -> Result<(), Error> {
            let now = Instant::now();
            let generation = {
                let mut state = lock(&self.state);
                state.title = metadata.title.map(str::to_owned);
                state.artist = metadata.artist.map(str::to_owned);
                state.album = metadata.album.map(str::to_owned);
                state.duration = metadata.duration;
                state.image = None;
                state.generation = state.generation.wrapping_add(1);
                state.playback.new_item(now);
                write(&self.main, &mut state, now);
                state.generation
            };
            if let Some(path) = metadata.cover_url {
                load_artwork(
                    Arc::clone(&self.state),
                    Arc::clone(&self.main),
                    path.to_owned(),
                    generation,
                );
            }
            Ok(())
        }
    }

    impl Drop for MediaControls {
        fn drop(&mut self) {
            let _ = self.detach();
        }
    }

    /// Queues a write of the state at `now` for the main thread. The caller holds the state's
    /// lock, so writes are queued in the order they were decided, and the main queue runs them in
    /// that order.
    fn write(main: &Arc<OnMain>, state: &mut State, now: Instant) {
        let info = state.info(now);
        let main = Arc::clone(main);
        on_main(move |mtm| main.get(mtm).borrow_mut().write(info));
    }

    impl MainState {
        fn attach<F>(&mut self, handler: &Arc<Mutex<F>>)
        where
            F: Fn(MediaControlEvent) + Send + 'static,
        {
            // SAFETY: `sharedCommandCenter` returns the process's command center, and its command
            // getters only read.
            let (center, commands) = unsafe {
                let center = MPRemoteCommandCenter::sharedCommandCenter();
                let commands = [
                    (center.playCommand(), MediaControlEvent::Play),
                    (center.pauseCommand(), MediaControlEvent::Pause),
                    (center.togglePlayPauseCommand(), MediaControlEvent::Toggle),
                    (center.nextTrackCommand(), MediaControlEvent::Next),
                    (center.previousTrackCommand(), MediaControlEvent::Previous),
                ];
                (center, commands)
            };
            for (command, event) in commands {
                let handler = Arc::clone(handler);
                let block = RcBlock::new(move |_: NonNull<MPRemoteCommandEvent>| {
                    debug!("Remote command: {event:?}");
                    send(&handler, event.clone());
                    MPRemoteCommandHandlerStatus::Success
                });
                self.add(command, &block);
            }

            let handler = Arc::clone(handler);
            let block = RcBlock::new(move |event: NonNull<MPRemoteCommandEvent>| {
                debug!("Remote command: change the playback position");
                // SAFETY: the command passes a live event for the length of the call.
                let event = unsafe { event.as_ref() };
                match event.downcast_ref::<MPChangePlaybackPositionCommandEvent>() {
                    Some(event) => {
                        // SAFETY: `positionTime` is a read-only property of the event.
                        let seconds = unsafe { event.positionTime() };
                        match Duration::try_from_secs_f64(seconds.max(0.0)) {
                            Ok(position) => send(
                                &handler,
                                MediaControlEvent::SetPosition(MediaPosition(position)),
                            ),
                            Err(e) => warn!("The lock screen asked for position {seconds}: {e}"),
                        }
                    }
                    None => error!("The position command sent another event: {event:?}"),
                }
                MPRemoteCommandHandlerStatus::Success
            });
            // SAFETY: as above.
            let command = unsafe { center.changePlaybackPositionCommand() };
            self.add(Retained::into_super(command), &block);
        }

        /// Adds `handler` to `command` and enables it.
        fn add(
            &mut self,
            command: Retained<MPRemoteCommand>,
            handler: &RcBlock<
                dyn Fn(NonNull<MPRemoteCommandEvent>) -> MPRemoteCommandHandlerStatus,
            >,
        ) {
            // SAFETY: the command copies the block, which returns a status for every event.
            let target = unsafe { command.addTargetWithHandler(handler) };
            // SAFETY: `enabled` is a plain property of the command.
            unsafe { command.setEnabled(true) };
            self.targets.push((command, target));
        }

        fn detach(&mut self) {
            for (command, target) in self.targets.drain(..) {
                // SAFETY: the target is the one this command returned from
                // `addTargetWithHandler:`.
                unsafe {
                    command.removeTarget(Some(&target));
                    command.setEnabled(false);
                }
            }
        }

        /// Sets the whole dictionary on the center.
        fn write(&mut self, info: Info) {
            let artwork = match info.image {
                Some(image) => Some(self.artwork(image)),
                None => {
                    self.artwork = None;
                    None
                }
            };
            let dictionary = NSMutableDictionary::<NSString, AnyObject>::new();
            // SAFETY: these are constants of the MediaPlayer framework, which is linked.
            let (title, artist, album, duration, artwork_key, elapsed_key, rate_key) = unsafe {
                (
                    MPMediaItemPropertyTitle,
                    MPMediaItemPropertyArtist,
                    MPMediaItemPropertyAlbumTitle,
                    MPMediaItemPropertyPlaybackDuration,
                    MPMediaItemPropertyArtwork,
                    MPNowPlayingInfoPropertyElapsedPlaybackTime,
                    MPNowPlayingInfoPropertyPlaybackRate,
                )
            };
            for (key, value) in [
                (title, &info.title),
                (artist, &info.artist),
                (album, &info.album),
            ] {
                if let Some(value) = value {
                    dictionary.insert(key, &*NSString::from_str(value));
                }
            }
            if let Some(value) = info.duration {
                dictionary.insert(duration, &*NSNumber::new_f64(value.as_secs_f64()));
            }
            if let Some(value) = &artwork {
                dictionary.insert(artwork_key, &**value);
            }
            dictionary.insert(elapsed_key, &*NSNumber::new_f64(info.elapsed.as_secs_f64()));
            dictionary.insert(rate_key, &*NSNumber::new_f64(info.rate));
            // SAFETY: `defaultCenter` returns the process's center. The dictionary maps
            // MediaPlayer's keys to values of the types their docs give, and the center copies
            // it.
            unsafe {
                MPNowPlayingInfoCenter::defaultCenter().setNowPlayingInfo(Some(&dictionary));
            }
        }

        /// The artwork object for `image`, made by the first write that shows it.
        fn artwork(&mut self, image: Retained<UIImage>) -> Retained<MPMediaItemArtwork> {
            if let Some((shown, artwork)) = &self.artwork
                && std::ptr::eq(Retained::as_ptr(shown), Retained::as_ptr(&image))
            {
                return artwork.clone();
            }
            // SAFETY: `UIImage` is sendable, and an image's size does not change.
            let size = unsafe { image.size() };
            let shown = image.clone();
            let handler = RcBlock::new(move |_: CGSize| NonNull::from(&*image));
            // SAFETY: `initWithBoundsSize:requestHandler:` is the designated initializer (iOS 10),
            // sent to a new artwork with the image's size and a handler that returns the image,
            // which the handler owns for as long as the artwork keeps it.
            let artwork: Retained<MPMediaItemArtwork> = unsafe {
                msg_send![
                    MPMediaItemArtwork::alloc(),
                    initWithBoundsSize: size,
                    requestHandler: &*handler,
                ]
            };
            self.artwork = Some((shown, artwork.clone()));
            artwork
        }
    }

    fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
        state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn send<F: Fn(MediaControlEvent)>(handler: &Mutex<F>, event: MediaControlEvent) {
        let handler = handler.lock().unwrap_or_else(PoisonError::into_inner);
        handler(event);
    }

    /// Loads the cover at `path` on a thread of its own, and shows it if no newer metadata came
    /// meanwhile.
    fn load_artwork(state: Arc<Mutex<State>>, main: Arc<OnMain>, path: String, generation: u64) {
        let spawned = thread::Builder::new()
            .name("now-playing-artwork".into())
            .spawn(move || {
                let Some(image) = image(&path) else {
                    return;
                };
                let mut state = lock(&state);
                if state.generation == generation {
                    state.image = Some(image);
                    write(&main, &mut state, Instant::now());
                }
            });
        if let Err(e) = spawned {
            error!("Failed to start loading the Now Playing artwork: {e}");
        }
    }

    /// The image in the file at `path`, or none if the file cannot be read as an image.
    fn image(path: &str) -> Option<Retained<UIImage>> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                warn!("Failed to read the Now Playing artwork {path:?}: {e}");
                return None;
            }
        };
        let image = UIImage::imageWithData(&NSData::with_bytes(&bytes));
        if image.is_none() {
            warn!("The Now Playing artwork {path:?} is not an image");
        }
        image
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MediaPosition;

    fn playing(seconds: f64) -> MediaPlayback {
        MediaPlayback::Playing {
            progress: Some(MediaPosition(Duration::from_secs_f64(seconds))),
        }
    }

    fn paused(seconds: f64) -> MediaPlayback {
        MediaPlayback::Paused {
            progress: Some(MediaPosition(Duration::from_secs_f64(seconds))),
        }
    }

    fn after(start: Instant, seconds: f64) -> Instant {
        start + Duration::from_secs_f64(seconds)
    }

    #[test]
    fn the_first_report_is_written() {
        let start = Instant::now();
        let mut shown = ShownPlayback::new(start);
        assert!(shown.report(&paused(0.0), start));
        assert!(!shown.report(&paused(0.0), after(start, 0.1)));
    }

    #[test]
    fn reports_that_follow_the_shown_progress_are_not_written() {
        let start = Instant::now();
        let mut shown = ShownPlayback::new(start);
        assert!(shown.report(&playing(10.0), start));
        for tenth in 1..=100 {
            let seconds = f64::from(tenth) / 10.0;
            assert!(
                !shown.report(&playing(10.0 + seconds), after(start, seconds)),
                "{seconds} s"
            );
        }
    }

    #[test]
    fn pause_and_play_are_each_written_once() {
        let start = Instant::now();
        let mut shown = ShownPlayback::new(start);
        assert!(shown.report(&playing(10.0), start));
        assert!(shown.report(&paused(12.0), after(start, 2.0)));
        assert!(!shown.report(&paused(12.0), after(start, 2.1)));
        // A paused track does not move on, however long it waits.
        assert!(!shown.report(&paused(12.0), after(start, 60.0)));
        assert!(shown.report(&playing(12.0), after(start, 61.0)));
        assert!(!shown.report(&playing(12.5), after(start, 61.5)));
    }

    #[test]
    fn a_seek_is_written() {
        let start = Instant::now();
        let mut shown = ShownPlayback::new(start);
        assert!(shown.report(&playing(10.0), start));
        assert!(shown.report(&playing(60.0), after(start, 0.1)));
        assert!(!shown.report(&playing(60.1), after(start, 0.2)));
        assert!(shown.report(&playing(5.0), after(start, 0.3)));
        assert!(shown.report(&paused(5.0), after(start, 0.4)));
        assert!(shown.report(&paused(30.0), after(start, 0.5)));
    }

    #[test]
    fn playback_that_stalls_is_written_once_it_is_a_second_behind() {
        let start = Instant::now();
        let mut shown = ShownPlayback::new(start);
        assert!(shown.report(&playing(10.0), start));
        assert!(!shown.report(&playing(10.0), after(start, 0.5)));
        assert!(!shown.report(&playing(10.0), after(start, 1.0)));
        assert!(shown.report(&playing(10.0), after(start, 1.1)));
    }

    #[test]
    fn the_report_after_new_metadata_is_written() {
        let start = Instant::now();
        let mut shown = ShownPlayback::new(start);
        assert!(shown.report(&playing(100.0), start));
        shown.new_item(after(start, 1.0));
        assert_eq!(shown.elapsed_at(after(start, 1.0)), Duration::ZERO);
        assert!(shown.report(&playing(0.05), after(start, 1.05)));
        assert!(!shown.report(&playing(0.15), after(start, 1.15)));
    }

    #[test]
    fn stopped_keeps_the_elapsed_time_where_it_is() {
        let start = Instant::now();
        let mut shown = ShownPlayback::new(start);
        assert!(shown.report(&playing(10.0), start));
        assert!(shown.report(&MediaPlayback::Stopped, after(start, 2.0)));
        assert_eq!(shown.elapsed_at(after(start, 5.0)), Duration::from_secs(12));
        assert!(!shown.report(&MediaPlayback::Stopped, after(start, 6.0)));
    }

    #[test]
    fn a_report_without_a_position_keeps_the_shown_one() {
        let start = Instant::now();
        let mut shown = ShownPlayback::new(start);
        assert!(shown.report(&paused(10.0), start));
        let resumed = MediaPlayback::Playing { progress: None };
        assert!(shown.report(&resumed, after(start, 5.0)));
        assert_eq!(shown.elapsed_at(after(start, 7.0)), Duration::from_secs(12));
    }
}
