//! Android: the media session that the lock screen, quick settings, headset and Bluetooth buttons
//! and other apps control, with its notification, and the audio focus, in place of souvlaki's
//! `MediaControls`. Souvlaki's other types are `dummy_souvlaki`'s, so the hub drives this the way
//! it drives souvlaki elsewhere. The session lives in a service of the app
//! (`android/app/src/main/kotlin/ci/not/rune/PlaybackService.kt`), which this drives through JNI.
//!
//! The state is Rust's, as on iOS: the metadata, the playback, and what audio focus asks of the
//! player. Every write hands the service all of it, together with whether the service is to run
//! and whether in the foreground, so the service keeps nothing that a write does not give it. It
//! runs in the foreground while the player plays, and while it is paused only until it is to play
//! again once audio focus comes back, so that playing again needs no foreground start from the
//! background, which Android 12 and later refuse. Paused otherwise, it runs in the background and
//! keeps its notification. Stopped, it does not run.
//!
//! The hub reports playback about ten times a second, and the session moves the position on by
//! itself, as the Now Playing center does on iOS, so a report is written under the same rules
//! ([`ShownPlayback`]), and also whenever the player's state changes.
//!
//! Audio focus. The service asks for it when playback starts. A transient loss pauses the player,
//! and the gain that ends it plays again, but only if that loss paused it. A loss for good pauses
//! the player. A loss that only ducks does nothing, as the system lowers the volume itself.
//! Headphones that go away pause the player, by a pause command from the service.
//!
//! A platform host that plays the current track asks for audio focus with a player of its own,
//! and follows its changes itself. Audio focus belongs to one request at a time, so while the
//! hub says that a host plays the track, the service gives up the focus it holds and asks for
//! none, and a change of focus pauses and plays nothing. Once Rune's own player plays again, the
//! service asks for the focus as before. The session, its notification, the foreground and
//! headphones that go away stay as they are.
//!
//! Commands from the session, its notification and audio focus reach the attached handler as the
//! `MediaControlEvent`s of the other platforms. What they lead to is decided in plain Rust, built
//! on every platform for its tests: only the JNI calls are Android's.
//!
//! Threads. The hub calls `MediaControls` from its own threads, and the service calls back on the
//! main thread. The state sits behind a lock, and every write is decided and handed to the
//! service under it, so writes reach the service in the order they were decided: the service
//! posts each one to the main thread, which runs them in order.

use std::time::{Duration, Instant};

#[cfg(any(target_os = "android", target_os = "ios"))]
use crate::dummy_souvlaki::MediaControlEvent;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use souvlaki::MediaControlEvent;

use crate::ios_now_playing::ShownPlayback;
use crate::{MediaMetadata, MediaPlayback, MediaPosition};

// The commands of the service, `PlaybackService.COMMAND_*`.
const COMMAND_PLAY: i32 = 0;
const COMMAND_PAUSE: i32 = 1;
const COMMAND_NEXT: i32 = 2;
const COMMAND_PREVIOUS: i32 = 3;
const COMMAND_STOP: i32 = 4;
const COMMAND_SEEK_TO: i32 = 5;

// `AudioManager.AUDIOFOCUS_*`, which the service passes on as they are.
const AUDIOFOCUS_GAIN: i32 = 1;
const AUDIOFOCUS_LOSS: i32 = -1;
const AUDIOFOCUS_LOSS_TRANSIENT: i32 = -2;

/// The player's state, as the session shows it. The values are `PlaybackService.STATE_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Stopped = 0,
    Paused = 1,
    Playing = 2,
}

/// Whether the service runs, and in the foreground. The values are `PlaybackService.SERVICE_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Service {
    Stopped = 0,
    Background = 1,
    Foreground = 2,
}

/// What the service does about audio focus. The values are `PlaybackService.FOCUS_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    /// Keeps what it holds: Rune's own player does not play.
    Keep = 0,
    /// Asks for it, unless it holds it: Rune's own player plays.
    Request = 1,
    /// Gives up what it holds, and asks for none: a platform host plays the track.
    Abandon = 2,
}

/// Whether playback that audio focus paused is to go on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resume {
    /// Nothing waits for audio focus.
    No,
    /// A transient loss of audio focus paused the player, which plays again once the focus comes
    /// back.
    OnGain,
    /// The focus came back and the player was asked to play, which it has not reported yet.
    Asked,
}

/// One write: all that the session shows, and whether the service runs.
#[derive(Debug)]
struct Info {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    duration: Option<Duration>,
    /// The cover, as a path to an image file.
    artwork: Option<String>,
    state: State,
    service: Service,
    focus: Focus,
    /// The position at the moment of the write, from which the session moves it on at `rate`.
    elapsed: Duration,
    rate: f64,
}

/// What a command or a change of audio focus leads to: the event for the hub, and a write.
type Outcome = (Option<MediaControlEvent>, Option<Info>);

/// What the session shows and what audio focus asks of the player, from which every write is
/// made.
struct Session {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    duration: Option<Duration>,
    artwork: Option<String>,
    playback: ShownPlayback,
    state: State,
    resume: Resume,
    /// Whether a platform host plays the current track, with a player of its own.
    hosted: bool,
}

impl Session {
    fn new(now: Instant) -> Self {
        Self {
            title: None,
            artist: None,
            album: None,
            duration: None,
            artwork: None,
            playback: ShownPlayback::new(now),
            state: State::Stopped,
            resume: Resume::No,
            hosted: false,
        }
    }

    fn service(&self) -> Service {
        match self.state {
            State::Stopped => Service::Stopped,
            State::Paused if self.resume == Resume::No => Service::Background,
            State::Paused | State::Playing => Service::Foreground,
        }
    }

    fn focus(&self) -> Focus {
        if self.hosted {
            Focus::Abandon
        } else if self.state == State::Playing {
            Focus::Request
        } else {
            Focus::Keep
        }
    }

    /// The write of what the session shows from `now` on.
    fn info(&mut self, now: Instant) -> Info {
        let (rate, elapsed) = self.playback.write_at(now);
        Info {
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            duration: self.duration,
            artwork: self.artwork.clone(),
            state: self.state,
            service: self.service(),
            focus: self.focus(),
            elapsed,
            rate,
        }
    }

    /// A new item, which is always written. `cover_url` is a plain path to an image file on
    /// Android.
    fn set_metadata(&mut self, metadata: &MediaMetadata<'_>, now: Instant) -> Info {
        self.title = metadata.title.map(str::to_owned);
        self.artist = metadata.artist.map(str::to_owned);
        self.album = metadata.album.map(str::to_owned);
        self.duration = metadata.duration;
        self.artwork = metadata.cover_url.map(str::to_owned);
        self.playback.new_item(now);
        self.info(now)
    }

    /// Takes a report of the player's playback at `now`, and returns the write it needs, if any.
    fn report(&mut self, playback: &MediaPlayback, now: Instant) -> Option<Info> {
        let state = match playback {
            MediaPlayback::Playing { .. } => State::Playing,
            MediaPlayback::Paused { .. } => State::Paused,
            MediaPlayback::Stopped => State::Stopped,
        };
        let moved = self.playback.report(playback, now);
        // Playback that starts, whether audio focus asked for it or not, or that stops, waits
        // for nothing. A report that the player still plays is not a start: it may have been
        // made before the pause that a loss of focus asked for.
        if (state == State::Playing && self.state != State::Playing) || state == State::Stopped {
            self.resume = Resume::No;
        }
        let changed = state != self.state;
        self.state = state;
        (moved || changed).then(|| self.info(now))
    }

    /// A command from the session, its notification or the service, with the position in
    /// milliseconds that a seek goes to.
    fn command(&mut self, command: i32, position_ms: i64, now: Instant) -> Outcome {
        let event = match command {
            COMMAND_PLAY => MediaControlEvent::Play,
            COMMAND_PAUSE => MediaControlEvent::Pause,
            COMMAND_NEXT => MediaControlEvent::Next,
            COMMAND_PREVIOUS => MediaControlEvent::Previous,
            COMMAND_STOP => MediaControlEvent::Stop,
            COMMAND_SEEK_TO => MediaControlEvent::SetPosition(MediaPosition(
                Duration::from_millis(u64::try_from(position_ms).unwrap_or(0)),
            )),
            _ => return (None, None),
        };
        // A pause or a stop is the user's own, or the headphones', so what audio focus paused
        // stays paused.
        let write = match command {
            COMMAND_PAUSE | COMMAND_STOP => self.set_resume(Resume::No, now),
            _ => None,
        };
        (Some(event), write)
    }

    /// A change of audio focus, `AudioManager.AUDIOFOCUS_*`. While a host plays the track, the
    /// change is its player's to follow, as when the host's own request took the focus from the
    /// service.
    fn focus_changed(&mut self, change: i32, now: Instant) -> Outcome {
        if self.hosted {
            return (None, None);
        }
        let playing = self.state == State::Playing || self.resume == Resume::Asked;
        let (resume, event) = match change {
            AUDIOFOCUS_LOSS_TRANSIENT if playing => {
                (Resume::OnGain, Some(MediaControlEvent::Pause))
            }
            AUDIOFOCUS_LOSS => (Resume::No, playing.then_some(MediaControlEvent::Pause)),
            AUDIOFOCUS_GAIN if self.resume == Resume::OnGain => {
                (Resume::Asked, Some(MediaControlEvent::Play))
            }
            // A loss that only ducks, which the system does itself, or one while nothing plays.
            _ => return (None, None),
        };
        (event, self.set_resume(resume, now))
    }

    /// Whether a platform host plays the current track, and the write it needs if that changes.
    /// A host's track waits for no audio focus of the service's.
    fn set_hosted(&mut self, hosted: bool, now: Instant) -> Option<Info> {
        if hosted == self.hosted {
            return None;
        }
        self.hosted = hosted;
        if hosted {
            self.resume = Resume::No;
        }
        Some(self.info(now))
    }

    /// Sets what waits for audio focus, and returns the write it needs if the service changes.
    fn set_resume(&mut self, resume: Resume, now: Instant) -> Option<Info> {
        let service = self.service();
        self.resume = resume;
        (self.service() != service).then(|| self.info(now))
    }

    /// The controls let go of the session: the service stops.
    fn stop(&mut self, now: Instant) -> Info {
        self.state = State::Stopped;
        self.resume = Resume::No;
        self.info(now)
    }
}

#[cfg(target_os = "android")]
pub(crate) use android::MediaControls;

#[cfg(target_os = "android")]
mod android {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
    use std::time::{Duration, Instant};

    use jni::errors::{Error as JniError, Result as JniResult};
    use jni::objects::{GlobalRef, JClass, JObject, JValue};
    use jni::sys::{jint, jlong};
    use jni::{JNIEnv, JavaVM};
    use log::{debug, error};

    use super::{Info, Outcome, Session};
    use crate::android_utils;
    use crate::dummy_souvlaki::{
        Error, MediaControlEvent, MediaMetadata, MediaPlayback, PlatformConfig,
    };

    /// The service's class, by its binary name.
    const SERVICE_CLASS: &str = "ci.not.rune.PlaybackService";
    /// `PlaybackService.update(context, title, artist, album, durationMs, artwork, state,
    /// service, positionMs, speed, focus)`.
    const UPDATE: (&str, &str) = (
        "update",
        "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;JLjava/lang/String;IIJFI)V",
    );

    /// What the commands are sent to.
    type Handler = Box<dyn Fn(MediaControlEvent) + Send>;

    /// What a `MediaControls` shares with the service's callbacks.
    struct Shared {
        session: Mutex<Session>,
        handler: Mutex<Option<Handler>>,
    }

    /// The controls that the service's callbacks reach: the ones attached last. Only they write
    /// to the service.
    static ATTACHED: Mutex<Option<Arc<Shared>>> = Mutex::new(None);

    /// A handle to the media session, with the API of souvlaki's `MediaControls`.
    pub struct MediaControls {
        shared: Arc<Shared>,
    }

    impl MediaControls {
        pub fn new(_config: PlatformConfig<'_>) -> Result<Self, Error> {
            Ok(Self {
                shared: Arc::new(Shared {
                    session: Mutex::new(Session::new(Instant::now())),
                    handler: Mutex::new(None),
                }),
            })
        }

        /// Sends the commands of the session, its notification and audio focus to
        /// `event_handler`, and lets these controls write to the service.
        pub fn attach<F>(&mut self, event_handler: F) -> Result<(), Error>
        where
            F: Fn(MediaControlEvent) + Send + 'static,
        {
            *lock(&self.shared.handler) = Some(Box::new(event_handler));
            *lock(&ATTACHED) = Some(Arc::clone(&self.shared));
            Ok(())
        }

        /// The commands reach no handler any more, and the service that these controls wrote to
        /// stops.
        pub fn detach(&mut self) -> Result<(), Error> {
            *lock(&self.shared.handler) = None;
            let was_attached = {
                let mut attached = lock(&ATTACHED);
                let ours = is(&attached, &self.shared);
                if ours {
                    *attached = None;
                }
                ours
            };
            if was_attached {
                let mut session = lock(&self.shared.session);
                let info = session.stop(Instant::now());
                update(&info);
            }
            Ok(())
        }

        /// Shows the player's playback, if the session does not show it already.
        pub fn set_playback(&mut self, playback: MediaPlayback) -> Result<(), Error> {
            let mut session = lock(&self.shared.session);
            if let Some(info) = session.report(&playback, Instant::now()) {
                self.write(&info);
            }
            Ok(())
        }

        /// Shows a new item. The service loads its cover.
        pub fn set_metadata(&mut self, metadata: MediaMetadata<'_>) -> Result<(), Error> {
            let mut session = lock(&self.shared.session);
            let info = session.set_metadata(&metadata, Instant::now());
            self.write(&info);
            Ok(())
        }

        /// Whether a platform host plays the current track, whose player asks for audio focus
        /// itself.
        pub fn set_hosted_audio(&mut self, hosted: bool) -> Result<(), Error> {
            let mut session = lock(&self.shared.session);
            if let Some(info) = session.set_hosted(hosted, Instant::now()) {
                self.write(&info);
            }
            Ok(())
        }

        /// Hands `info` to the service if these controls are attached. The caller holds the
        /// session's lock.
        fn write(&self, info: &Info) {
            if is(&lock(&ATTACHED), &self.shared) {
                update(info);
            }
        }
    }

    impl Drop for MediaControls {
        fn drop(&mut self) {
            let _ = self.detach();
        }
    }

    fn is(attached: &Option<Arc<Shared>>, shared: &Arc<Shared>) -> bool {
        attached
            .as_ref()
            .is_some_and(|attached| Arc::ptr_eq(attached, shared))
    }

    fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `PlaybackService.nativeCommand`: a command from the session, its notification or the
    /// service, with the position in milliseconds that a seek goes to.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_ci_not_rune_PlaybackService_nativeCommand(
        _env: JNIEnv,
        _service: JObject,
        command: jint,
        position_ms: jlong,
    ) {
        callback("command", |session, now| {
            session.command(command, position_ms, now)
        });
    }

    /// `PlaybackService.nativeAudioFocusChange`: a change of audio focus,
    /// `AudioManager.AUDIOFOCUS_*`.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_ci_not_rune_PlaybackService_nativeAudioFocusChange(
        _env: JNIEnv,
        _service: JObject,
        change: jint,
    ) {
        callback("audio focus change", |session, now| {
            session.focus_changed(change, now)
        });
    }

    /// Decides what a callback of the service leads to from the attached controls' session,
    /// writes to the service under its lock, and sends the event, if any, to their handler.
    fn callback(what: &str, decide: impl FnOnce(&mut Session, Instant) -> Outcome) {
        // A panic must not unwind into the JVM.
        let result = catch_unwind(AssertUnwindSafe(|| {
            let Some(shared) = lock(&ATTACHED).clone() else {
                debug!("No media controls are attached for the {what}");
                return;
            };
            let event = {
                let mut session = lock(&shared.session);
                let (event, info) = decide(&mut session, Instant::now());
                if let Some(info) = info {
                    update(&info);
                }
                event
            };
            if let Some(event) = event
                && let Some(handler) = lock(&shared.handler).as_ref()
            {
                debug!("Media session {what}: {event:?}");
                handler(event);
            }
        }));
        if result.is_err() {
            error!("The media session's {what} panicked");
        }
    }

    /// The JVM and the service's class, found the first time a write needs them.
    struct Java {
        vm: JavaVM,
        class: GlobalRef,
    }

    fn java() -> Option<&'static Java> {
        static JAVA: OnceLock<Option<Java>> = OnceLock::new();
        JAVA.get_or_init(|| match find_java() {
            Ok(java) => Some(java),
            Err(e) => {
                error!("The media session's service cannot be reached: {e}");
                None
            }
        })
        .as_ref()
    }

    fn find_java() -> JniResult<Java> {
        let vm = android_utils::get_jvm().ok_or(JniError::NullPtr("No JVM was loaded"))?;
        // SAFETY: `JNI_OnLoad` stored the process's JVM.
        let vm = unsafe { JavaVM::from_raw(vm.cast()) }?;
        let mut env = vm.attach_current_thread_permanently()?;
        // The app's class loader, which `JNI_OnLoad` keeps, finds the app's classes from any
        // thread. This thread stays attached, so the local reference it returns is this frame's.
        let class = env.with_local_frame(4, |env| {
            let class = ndk_saf::find_class(SERVICE_CLASS)?;
            env.new_global_ref(class)
        })?;
        Ok(Java { vm, class })
    }

    /// `PlaybackService.update`, which hands `info` to the main thread. The calling thread stays
    /// attached to the JVM, and the call runs in a local frame of its own, so that no local
    /// reference outlives it.
    fn update(info: &Info) {
        let (Some(java), Some(context)) = (java(), android_utils::application_context()) else {
            return;
        };
        let mut env = match java.vm.attach_current_thread_permanently() {
            Ok(env) => env,
            Err(e) => {
                error!("Failed to attach to the JVM for the media session: {e}");
                return;
            }
        };
        let result = env.with_local_frame(8, |env| -> JniResult<()> {
            let title = string(env, info.title.as_deref())?;
            let artist = string(env, info.artist.as_deref())?;
            let album = string(env, info.album.as_deref())?;
            let artwork = string(env, info.artwork.as_deref())?;
            let class: &JClass = java.class.as_obj().into();
            env.call_static_method(
                class,
                UPDATE.0,
                UPDATE.1,
                &[
                    JValue::Object(context.as_obj()),
                    JValue::Object(&title),
                    JValue::Object(&artist),
                    JValue::Object(&album),
                    JValue::Long(millis(info.duration)),
                    JValue::Object(&artwork),
                    JValue::Int(info.state as jint),
                    JValue::Int(info.service as jint),
                    JValue::Long(millis(Some(info.elapsed))),
                    JValue::Float(info.rate as f32),
                    JValue::Int(info.focus as jint),
                ],
            )?;
            Ok(())
        });
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        if let Err(e) = result {
            error!("Failed to update the media session: {e}");
        }
    }

    /// A Java string, or null for none.
    fn string<'local>(env: &mut JNIEnv<'local>, value: Option<&str>) -> JniResult<JObject<'local>> {
        match value {
            Some(value) => Ok(env.new_string(value)?.into()),
            None => Ok(JObject::null()),
        }
    }

    /// Milliseconds, or -1 for none.
    fn millis(duration: Option<Duration>) -> jlong {
        duration.map_or(-1, |duration| {
            jlong::try_from(duration.as_millis()).unwrap_or(jlong::MAX)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK`.
    const AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK: i32 = -3;

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

    /// A session whose player plays from `start`.
    fn playing_session(start: Instant) -> Session {
        let mut session = Session::new(start);
        assert!(session.report(&playing(0.0), start).is_some());
        session
    }

    fn service_of(info: Option<Info>) -> Option<Service> {
        info.map(|info| info.service)
    }

    /// Whether `outcome` sends no event and writes nothing.
    fn nothing(outcome: Outcome) -> bool {
        outcome.0.is_none() && outcome.1.is_none()
    }

    #[test]
    fn commands_are_the_events_of_the_other_platforms() {
        let now = Instant::now();
        let mut session = Session::new(now);
        let event =
            |session: &mut Session, command, position| session.command(command, position, now).0;
        assert!(matches!(
            event(&mut session, COMMAND_PLAY, 0),
            Some(MediaControlEvent::Play)
        ));
        assert!(matches!(
            event(&mut session, COMMAND_PAUSE, 0),
            Some(MediaControlEvent::Pause)
        ));
        assert!(matches!(
            event(&mut session, COMMAND_NEXT, 0),
            Some(MediaControlEvent::Next)
        ));
        assert!(matches!(
            event(&mut session, COMMAND_PREVIOUS, 0),
            Some(MediaControlEvent::Previous)
        ));
        assert!(matches!(
            event(&mut session, COMMAND_STOP, 0),
            Some(MediaControlEvent::Stop)
        ));
        assert!(matches!(
            event(&mut session, COMMAND_SEEK_TO, 63_959),
            Some(MediaControlEvent::SetPosition(MediaPosition(position)))
                if position == Duration::from_millis(63_959)
        ));
        assert!(matches!(
            event(&mut session, COMMAND_SEEK_TO, -5),
            Some(MediaControlEvent::SetPosition(MediaPosition(position))) if position.is_zero()
        ));
        assert!(event(&mut session, 99, 0).is_none());
    }

    #[test]
    fn the_service_runs_in_the_foreground_only_while_the_player_plays() {
        let start = Instant::now();
        let mut session = Session::new(start);
        let info = session.report(&playing(0.0), start).expect("written");
        assert_eq!(
            (info.state, info.service),
            (State::Playing, Service::Foreground)
        );
        let info = session
            .report(&paused(1.0), after(start, 1.0))
            .expect("written");
        assert_eq!(
            (info.state, info.service),
            (State::Paused, Service::Background)
        );
        let info = session
            .report(&MediaPlayback::Stopped, after(start, 2.0))
            .expect("written");
        assert_eq!(
            (info.state, info.service),
            (State::Stopped, Service::Stopped)
        );
    }

    #[test]
    fn every_change_of_state_is_written_even_where_the_position_shows_none() {
        let start = Instant::now();
        let mut session = Session::new(start);
        assert!(session.report(&paused(10.0), start).is_some());
        // A stopped player keeps the shown position where it was, and is still written.
        assert!(
            session
                .report(&MediaPlayback::Stopped, after(start, 1.0))
                .is_some()
        );
        assert!(
            session
                .report(&MediaPlayback::Stopped, after(start, 1.1))
                .is_none()
        );
        assert!(session.report(&paused(10.0), after(start, 2.0)).is_some());
    }

    #[test]
    fn reports_that_follow_the_shown_position_are_not_written() {
        let start = Instant::now();
        let mut session = playing_session(start);
        for tenth in 1..=50 {
            let seconds = f64::from(tenth) / 10.0;
            assert!(
                session
                    .report(&playing(seconds), after(start, seconds))
                    .is_none()
            );
        }
        let seek = session
            .report(&playing(60.0), after(start, 5.1))
            .expect("written");
        assert_eq!((seek.elapsed, seek.rate), (Duration::from_secs(60), 1.0));
    }

    #[test]
    fn new_metadata_is_written_with_all_that_the_session_shows() {
        let start = Instant::now();
        let mut session = playing_session(start);
        let metadata = MediaMetadata {
            title: Some("Title"),
            album: Some("Album"),
            artist: Some("Artist"),
            cover_url: Some("/data/cover.png"),
            duration: Some(Duration::from_secs(200)),
        };
        let info = session.set_metadata(&metadata, after(start, 3.0));
        assert_eq!(info.title.as_deref(), Some("Title"));
        assert_eq!(info.artist.as_deref(), Some("Artist"));
        assert_eq!(info.album.as_deref(), Some("Album"));
        assert_eq!(info.duration, Some(Duration::from_secs(200)));
        assert_eq!(info.artwork.as_deref(), Some("/data/cover.png"));
        assert_eq!(
            (info.state, info.service),
            (State::Playing, Service::Foreground)
        );
        // A new item starts from its beginning, and the next report is written whatever it says.
        assert_eq!((info.elapsed, info.rate), (Duration::ZERO, 1.0));
        assert!(session.report(&playing(0.05), after(start, 3.05)).is_some());
    }

    #[test]
    fn a_transient_loss_of_focus_pauses_and_its_end_plays_again() {
        let start = Instant::now();
        let mut session = playing_session(start);
        let (event, write) = session.focus_changed(AUDIOFOCUS_LOSS_TRANSIENT, start);
        assert!(matches!(event, Some(MediaControlEvent::Pause)));
        assert!(write.is_none());
        // The service stays in the foreground while the player waits for the focus.
        assert_eq!(
            service_of(session.report(&paused(1.0), after(start, 1.0))),
            Some(Service::Foreground)
        );
        let (event, write) = session.focus_changed(AUDIOFOCUS_GAIN, after(start, 5.0));
        assert!(matches!(event, Some(MediaControlEvent::Play)));
        assert!(write.is_none());
        assert_eq!(
            service_of(session.report(&playing(1.0), after(start, 5.1))),
            Some(Service::Foreground)
        );
        // Paused again by the user, the service leaves the foreground, and a gain plays nothing.
        assert_eq!(
            service_of(session.report(&paused(2.0), after(start, 6.0))),
            Some(Service::Background)
        );
        assert!(
            session
                .focus_changed(AUDIOFOCUS_GAIN, after(start, 7.0))
                .0
                .is_none()
        );
    }

    #[test]
    fn a_transient_loss_while_paused_plays_nothing_once_it_ends() {
        let start = Instant::now();
        let mut session = Session::new(start);
        assert!(session.report(&paused(1.0), start).is_some());
        assert!(nothing(
            session.focus_changed(AUDIOFOCUS_LOSS_TRANSIENT, start)
        ));
        assert!(nothing(
            session.focus_changed(AUDIOFOCUS_GAIN, after(start, 1.0))
        ));
    }

    #[test]
    fn a_report_that_the_player_still_plays_keeps_waiting_for_the_focus() {
        let start = Instant::now();
        let mut session = playing_session(start);
        assert!(
            session
                .focus_changed(AUDIOFOCUS_LOSS_TRANSIENT, start)
                .0
                .is_some()
        );
        // Made before the pause took effect.
        assert!(session.report(&playing(0.1), after(start, 0.1)).is_none());
        assert!(session.report(&paused(0.2), after(start, 0.2)).is_some());
        assert!(matches!(
            session.focus_changed(AUDIOFOCUS_GAIN, after(start, 3.0)).0,
            Some(MediaControlEvent::Play)
        ));
    }

    #[test]
    fn playback_the_user_starts_during_a_transient_loss_waits_for_nothing() {
        let start = Instant::now();
        let mut session = playing_session(start);
        assert!(
            session
                .focus_changed(AUDIOFOCUS_LOSS_TRANSIENT, start)
                .0
                .is_some()
        );
        assert!(session.report(&paused(0.2), after(start, 0.2)).is_some());
        assert!(session.report(&playing(0.2), after(start, 1.0)).is_some());
        assert!(session.report(&paused(0.5), after(start, 1.3)).is_some());
        assert!(nothing(
            session.focus_changed(AUDIOFOCUS_GAIN, after(start, 2.0))
        ));
    }

    #[test]
    fn a_pause_during_a_transient_loss_keeps_the_player_paused() {
        let start = Instant::now();
        let mut session = playing_session(start);
        assert!(
            session
                .focus_changed(AUDIOFOCUS_LOSS_TRANSIENT, start)
                .0
                .is_some()
        );
        assert!(session.report(&paused(0.2), after(start, 0.2)).is_some());
        let (event, write) = session.command(COMMAND_PAUSE, 0, after(start, 1.0));
        assert!(matches!(event, Some(MediaControlEvent::Pause)));
        assert_eq!(service_of(write), Some(Service::Background));
        assert!(nothing(
            session.focus_changed(AUDIOFOCUS_GAIN, after(start, 2.0))
        ));
    }

    #[test]
    fn a_loss_of_focus_for_good_pauses_and_nothing_plays_again() {
        let start = Instant::now();
        let mut session = playing_session(start);
        let (event, write) = session.focus_changed(AUDIOFOCUS_LOSS, start);
        assert!(matches!(event, Some(MediaControlEvent::Pause)));
        assert!(write.is_none());
        assert_eq!(
            service_of(session.report(&paused(0.2), after(start, 0.2))),
            Some(Service::Background)
        );
        assert!(nothing(
            session.focus_changed(AUDIOFOCUS_GAIN, after(start, 1.0))
        ));
    }

    #[test]
    fn a_loss_for_good_after_a_transient_one_lets_the_service_leave_the_foreground() {
        let start = Instant::now();
        let mut session = playing_session(start);
        assert!(
            session
                .focus_changed(AUDIOFOCUS_LOSS_TRANSIENT, start)
                .0
                .is_some()
        );
        assert!(session.report(&paused(0.2), after(start, 0.2)).is_some());
        let (event, write) = session.focus_changed(AUDIOFOCUS_LOSS, after(start, 1.0));
        assert!(event.is_none());
        assert_eq!(service_of(write), Some(Service::Background));
        assert!(nothing(
            session.focus_changed(AUDIOFOCUS_GAIN, after(start, 2.0))
        ));
    }

    #[test]
    fn a_transient_loss_after_the_player_was_asked_to_play_again_pauses_it() {
        let start = Instant::now();
        let mut session = playing_session(start);
        assert!(
            session
                .focus_changed(AUDIOFOCUS_LOSS_TRANSIENT, start)
                .0
                .is_some()
        );
        assert!(session.report(&paused(0.2), after(start, 0.2)).is_some());
        assert!(
            session
                .focus_changed(AUDIOFOCUS_GAIN, after(start, 1.0))
                .0
                .is_some()
        );
        // The play is still on its way.
        assert!(matches!(
            session
                .focus_changed(AUDIOFOCUS_LOSS_TRANSIENT, after(start, 1.05))
                .0,
            Some(MediaControlEvent::Pause)
        ));
    }

    #[test]
    fn a_loss_that_only_ducks_does_nothing() {
        let start = Instant::now();
        let mut session = playing_session(start);
        assert!(nothing(
            session.focus_changed(AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK, start)
        ));
        assert!(nothing(
            session.focus_changed(AUDIOFOCUS_GAIN, after(start, 1.0))
        ));
    }

    #[test]
    fn a_stop_ends_the_wait_for_the_focus() {
        let start = Instant::now();
        let mut session = playing_session(start);
        assert!(
            session
                .focus_changed(AUDIOFOCUS_LOSS_TRANSIENT, start)
                .0
                .is_some()
        );
        assert_eq!(
            service_of(session.report(&MediaPlayback::Stopped, after(start, 0.2))),
            Some(Service::Stopped)
        );
        assert!(nothing(
            session.focus_changed(AUDIOFOCUS_GAIN, after(start, 1.0))
        ));
    }

    #[test]
    fn letting_go_stops_the_service() {
        let start = Instant::now();
        let mut session = playing_session(start);
        let info = session.stop(after(start, 1.0));
        assert_eq!(
            (info.state, info.service),
            (State::Stopped, Service::Stopped)
        );
        // Playback that starts again afterwards is written.
        assert!(session.report(&playing(1.0), after(start, 2.0)).is_some());
    }

    #[test]
    fn rune_asks_for_focus_only_while_its_own_player_plays() {
        let start = Instant::now();
        let mut session = Session::new(start);
        let focus_of = |info: Option<Info>| info.map(|info| info.focus);
        assert_eq!(
            focus_of(session.report(&playing(0.0), start)),
            Some(Focus::Request)
        );
        assert_eq!(
            focus_of(session.report(&paused(1.0), after(start, 1.0))),
            Some(Focus::Keep)
        );
        assert_eq!(
            focus_of(session.report(&MediaPlayback::Stopped, after(start, 2.0))),
            Some(Focus::Keep)
        );
    }

    #[test]
    fn a_hosts_track_gives_up_the_focus_and_asks_for_none() {
        let start = Instant::now();
        let mut session = playing_session(start);
        let info = session
            .set_hosted(true, after(start, 1.0))
            .expect("written");
        assert_eq!(
            (info.state, info.service, info.focus),
            (State::Playing, Service::Foreground, Focus::Abandon)
        );
        // Every write while the host plays says so, and the service runs in the foreground while
        // the host's track plays, as for Rune's own.
        let info = session
            .report(&paused(2.0), after(start, 2.0))
            .expect("written");
        assert_eq!(
            (info.service, info.focus),
            (Service::Background, Focus::Abandon)
        );
        let info = session
            .report(&playing(2.0), after(start, 3.0))
            .expect("written");
        assert_eq!(
            (info.service, info.focus),
            (Service::Foreground, Focus::Abandon)
        );
        let metadata = MediaMetadata {
            title: Some("Next"),
            ..MediaMetadata::default()
        };
        assert_eq!(
            session.set_metadata(&metadata, after(start, 4.0)).focus,
            Focus::Abandon
        );
        assert!(session.set_hosted(true, after(start, 5.0)).is_none());
    }

    #[test]
    fn changes_of_focus_pause_and_play_nothing_while_a_host_plays() {
        let start = Instant::now();
        let mut session = playing_session(start);
        assert!(session.set_hosted(true, start).is_some());
        // As when the host's own request takes the focus from the service, and more after it.
        for change in [
            AUDIOFOCUS_LOSS,
            AUDIOFOCUS_LOSS_TRANSIENT,
            AUDIOFOCUS_GAIN,
            AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK,
        ] {
            assert!(nothing(session.focus_changed(change, after(start, 1.0))));
        }
        assert_eq!(session.state, State::Playing);
    }

    #[test]
    fn a_host_that_takes_over_ends_the_wait_for_the_focus() {
        let start = Instant::now();
        let mut session = playing_session(start);
        assert!(
            session
                .focus_changed(AUDIOFOCUS_LOSS_TRANSIENT, start)
                .0
                .is_some()
        );
        assert_eq!(
            service_of(session.report(&paused(0.2), after(start, 0.2))),
            Some(Service::Foreground)
        );
        let info = session
            .set_hosted(true, after(start, 1.0))
            .expect("written");
        assert_eq!(
            (info.service, info.focus),
            (Service::Background, Focus::Abandon)
        );
        assert!(nothing(
            session.focus_changed(AUDIOFOCUS_GAIN, after(start, 2.0))
        ));
    }

    #[test]
    fn rune_asks_for_the_focus_again_once_its_own_track_plays() {
        let start = Instant::now();
        let mut session = playing_session(start);
        assert!(session.set_hosted(true, start).is_some());
        // The host lets go, and Rune's own track loads paused, then plays.
        assert!(session.report(&paused(1.0), after(start, 1.0)).is_some());
        let info = session
            .set_hosted(false, after(start, 1.5))
            .expect("written");
        assert_eq!(info.focus, Focus::Keep);
        let info = session
            .report(&playing(0.0), after(start, 2.0))
            .expect("written");
        assert_eq!(info.focus, Focus::Request);
        // Changes of focus act on Rune's own player again.
        assert!(matches!(
            session
                .focus_changed(AUDIOFOCUS_LOSS_TRANSIENT, after(start, 3.0))
                .0,
            Some(MediaControlEvent::Pause)
        ));
    }
}
