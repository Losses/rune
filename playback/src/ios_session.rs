//! iOS: the audio session. The player owns it, and nothing else in the app sets it up: its
//! category, its activation, and what its notifications mean for playback all live here.
//!
//! [`configure`] gives the session the playback category. The player calls it when it is made,
//! so that any audio the app plays has the category from the start. With `mix_with_others` the
//! category carries the mix-with-others option, so that other apps keep playing alongside. The
//! cost is that the system no longer takes the app for the Now Playing app, so the lock screen,
//! Control Center and headphone buttons do not control it. The player calls it again whenever
//! the setting changes.
//!
//! [`activate`] sets the category again, with the same option, and makes the session active. A
//! reset of the media services drops the category, so it is set every time. Activation can block
//! for a while, so the player calls it on its own thread, right before an output stream opens.
//!
//! [`observe`] subscribes to `AVAudioSession`'s interruption, route change and media services
//! reset notifications, and turns each into a [`PlayerCommand::Output`] for the player. No queue
//! is given, so the notification center calls a block on whichever thread posts the
//! notification. A block only reads the notification and sends into the player's command
//! channel, which takes sends from any thread. The [`Observers`] it returns removes the blocks
//! when dropped, on any thread, since the notification center is thread-safe.
//!
//! The translation from a notification's values to an [`OutputEvent`] is plain Rust, built on
//! every platform for its tests.
//!
//! [`PlayerCommand::Output`]: crate::PlayerCommand::Output

use crate::internal::OutputEvent;

// `AVAudioSessionInterruptionType`.
const INTERRUPTION_BEGAN: usize = 1;
const INTERRUPTION_ENDED: usize = 0;
// `AVAudioSessionInterruptionOptions`.
const INTERRUPTION_SHOULD_RESUME: usize = 1;
// `AVAudioSessionInterruptionReason`.
const INTERRUPTION_REASON_APP_WAS_SUSPENDED: usize = 1;
// `AVAudioSessionRouteChangeReason`.
const ROUTE_NEW_DEVICE_AVAILABLE: usize = 1;
const ROUTE_OLD_DEVICE_UNAVAILABLE: usize = 2;

/// What an interruption means for the output, from the notification's type, options and reason,
/// and whether it says that the app was suspended.
///
/// The system deactivates the session of an app that it suspends, and iOS 15 tells the app so
/// once it runs again, as the start of an interruption. That news comes late and no end follows
/// it. The app may be playing again by then, woken by a remote command, so it means nothing for
/// the output. iOS 16 and later no longer send it.
fn interruption_event(
    kind: usize,
    options: usize,
    reason: usize,
    was_suspended: bool,
) -> Option<OutputEvent> {
    match kind {
        INTERRUPTION_BEGAN if was_suspended || reason == INTERRUPTION_REASON_APP_WAS_SUSPENDED => {
            None
        }
        INTERRUPTION_BEGAN => Some(OutputEvent::Interrupted),
        INTERRUPTION_ENDED => Some(OutputEvent::InterruptionEnded {
            resume: options & INTERRUPTION_SHOULD_RESUME != 0,
        }),
        _ => None,
    }
}

/// What a route change means for the output, from its reason. The other reasons, such as a new
/// category, an override or a new configuration of the same route, leave the device that plays.
fn route_change_event(reason: usize) -> Option<OutputEvent> {
    match reason {
        ROUTE_NEW_DEVICE_AVAILABLE => Some(OutputEvent::DeviceAdded),
        ROUTE_OLD_DEVICE_UNAVAILABLE => Some(OutputEvent::DeviceLost),
        _ => None,
    }
}

/// What a reset of the media services means for the output. Every stream and the session's
/// settings are gone, as when the device that played went away: playback pauses where it was,
/// and the next play opens a new stream, which sets the session up again. Nothing resumes on its
/// own.
fn media_services_reset_event() -> OutputEvent {
    OutputEvent::DeviceLost
}

#[cfg(target_os = "ios")]
pub(crate) use ios::{Observers, activate, configure, deactivate, observe};

#[cfg(target_os = "ios")]
mod ios {
    use std::ptr::NonNull;

    use block2::RcBlock;
    use log::error;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, ProtocolObject};
    use objc2_avf_audio::{
        AVAudioSession, AVAudioSessionCategoryOptions, AVAudioSessionCategoryPlayback,
        AVAudioSessionInterruptionNotification, AVAudioSessionInterruptionOptionKey,
        AVAudioSessionInterruptionOptions, AVAudioSessionInterruptionReasonKey,
        AVAudioSessionInterruptionType, AVAudioSessionInterruptionTypeKey,
        AVAudioSessionMediaServicesWereResetNotification, AVAudioSessionModeDefault,
        AVAudioSessionRouteChangeNotification, AVAudioSessionRouteChangeReason,
        AVAudioSessionRouteChangeReasonKey,
    };
    use objc2_foundation::{
        NSNotification, NSNotificationCenter, NSNotificationName, NSNumber, NSObjectProtocol,
        NSString,
    };
    use tokio::sync::mpsc::UnboundedSender;

    use super::{
        INTERRUPTION_BEGAN, INTERRUPTION_ENDED, INTERRUPTION_REASON_APP_WAS_SUSPENDED,
        INTERRUPTION_SHOULD_RESUME, ROUTE_NEW_DEVICE_AVAILABLE, ROUTE_OLD_DEVICE_UNAVAILABLE,
        interruption_event, media_services_reset_event, route_change_event,
    };
    use crate::internal::{OutputEvent, PlayerCommand};

    // The values above are the SDK's.
    const _: () = {
        assert!(INTERRUPTION_BEGAN == AVAudioSessionInterruptionType::Began.0);
        assert!(INTERRUPTION_ENDED == AVAudioSessionInterruptionType::Ended.0);
        assert!(
            INTERRUPTION_SHOULD_RESUME == AVAudioSessionInterruptionOptions::ShouldResume.bits()
        );
        assert!(
            ROUTE_NEW_DEVICE_AVAILABLE == AVAudioSessionRouteChangeReason::NewDeviceAvailable.0
        );
        assert!(
            ROUTE_OLD_DEVICE_UNAVAILABLE == AVAudioSessionRouteChangeReason::OldDeviceUnavailable.0
        );
    };
    // The SDK marks the reason deprecated, as iOS 16 and later no longer give it.
    #[allow(deprecated)]
    const _: () = assert!(
        INTERRUPTION_REASON_APP_WAS_SUSPENDED
            == objc2_avf_audio::AVAudioSessionInterruptionReason::AppWasSuspended.0
    );

    /// Gives the session the playback category, as Rune only plays. The category sends music to
    /// the speaker rather than the receiver and to Bluetooth headphones in full quality, and
    /// keeps it playing in the background. With `mix_with_others`, other apps' audio goes on
    /// beside it.
    pub(crate) fn configure(mix_with_others: bool) {
        // SAFETY: these are constants of the AVFAudio framework, which is linked.
        let (category, mode) =
            unsafe { (AVAudioSessionCategoryPlayback, AVAudioSessionModeDefault) };
        let (Some(category), Some(mode)) = (category, mode) else {
            error!("The playback category or the default mode is missing from the SDK");
            return;
        };
        let options = if mix_with_others {
            AVAudioSessionCategoryOptions::MixWithOthers
        } else {
            AVAudioSessionCategoryOptions::empty()
        };
        // SAFETY: `sharedInstance` returns the process's session, which takes
        // `setCategory:mode:options:error:` from any thread.
        let result = unsafe {
            AVAudioSession::sharedInstance().setCategory_mode_options_error(category, mode, options)
        };
        if let Err(error) = result {
            error!(
                "Failed to set the audio session's category: {}",
                error.localizedDescription()
            );
        }
    }

    /// Sets the category again and makes the session active, so that an output stream can
    /// start. An interruption leaves the session inactive. Returns whether it is active, which
    /// it cannot be during a call, for one.
    pub(crate) fn activate(mix_with_others: bool) -> bool {
        configure(mix_with_others);
        // SAFETY: `sharedInstance` returns the process's session, which takes `setActive:error:`
        // from any thread.
        let result = unsafe { AVAudioSession::sharedInstance().setActive_error(true) };
        match result {
            Ok(()) => true,
            Err(error) => {
                error!(
                    "Failed to activate the audio session: {}",
                    error.localizedDescription()
                );
                false
            }
        }
    }

    /// Makes the session inactive. It only succeeds while no output of the app runs.
    pub(crate) fn deactivate() {
        // SAFETY: as in `activate`.
        let result = unsafe { AVAudioSession::sharedInstance().setActive_error(false) };
        if let Err(error) = result {
            error!(
                "Failed to deactivate the audio session: {}",
                error.localizedDescription()
            );
        }
    }

    /// The notification blocks that feed the player. Dropping it removes them.
    pub(crate) struct Observers(Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>);

    // SAFETY: the tokens are only handed back to the notification center's `removeObserver:` and
    // released when this is dropped. The notification center is thread-safe (objc2 marks it
    // `Send` and `Sync`), releasing an object is too, and nothing else uses the tokens.
    unsafe impl Send for Observers {}

    impl Drop for Observers {
        fn drop(&mut self) {
            let center = NSNotificationCenter::defaultCenter();
            for token in &self.0 {
                // SAFETY: the token is one this center returned from
                // `addObserverForName:object:queue:usingBlock:`.
                unsafe { center.removeObserver(token.as_ref()) };
            }
        }
    }

    /// Sends the player an [`OutputEvent`] for each notification of the audio session.
    pub(crate) fn observe(commands: UnboundedSender<PlayerCommand>) -> Observers {
        // SAFETY: these are constants of the AVFAudio framework, which is linked.
        let names = unsafe {
            [
                (
                    AVAudioSessionInterruptionNotification,
                    interruption as fn(&NSNotification) -> Option<OutputEvent>,
                ),
                (AVAudioSessionRouteChangeNotification, route_change),
                (
                    AVAudioSessionMediaServicesWereResetNotification,
                    media_services_reset,
                ),
            ]
        };
        let center = NSNotificationCenter::defaultCenter();
        let mut tokens = Vec::with_capacity(names.len());
        for (name, translate) in names {
            let Some(name) = name else {
                error!("An audio session notification is missing from the SDK");
                continue;
            };
            tokens.push(add_observer(&center, name, commands.clone(), translate));
        }
        Observers(tokens)
    }

    fn add_observer(
        center: &NSNotificationCenter,
        name: &NSNotificationName,
        commands: UnboundedSender<PlayerCommand>,
        translate: fn(&NSNotification) -> Option<OutputEvent>,
    ) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
        let block = RcBlock::new(move |notification: NonNull<NSNotification>| {
            // SAFETY: the center passes a live notification for the length of the call.
            let notification = unsafe { notification.as_ref() };
            if let Some(event) = translate(notification)
                && commands.send(PlayerCommand::Output(event)).is_err()
            {
                error!("The player is gone, so the audio output event {event:?} is dropped");
            }
        });
        // SAFETY: no sender filters the notification. Without a queue the block runs on the
        // posting thread, which is sound because it only sends into a channel that takes sends
        // from any thread.
        unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block) }
    }

    fn interruption(notification: &NSNotification) -> Option<OutputEvent> {
        // SAFETY: these are constants of the AVFAudio framework, which is linked. The key that
        // says the app was suspended is deprecated, but iOS 15 may still set it.
        #[allow(deprecated)]
        let (kind_key, options_key, reason_key, suspended_key) = unsafe {
            (
                AVAudioSessionInterruptionTypeKey,
                AVAudioSessionInterruptionOptionKey,
                AVAudioSessionInterruptionReasonKey,
                objc2_avf_audio::AVAudioSessionInterruptionWasSuspendedKey,
            )
        };
        let kind = number(notification, kind_key)?;
        // Only the end of an interruption carries options, and only its start a reason.
        let options = number(notification, options_key).unwrap_or(0);
        let reason = number(notification, reason_key).unwrap_or(0);
        let was_suspended = number(notification, suspended_key).is_some_and(|value| value != 0);
        interruption_event(kind, options, reason, was_suspended)
    }

    fn route_change(notification: &NSNotification) -> Option<OutputEvent> {
        // SAFETY: a constant of the AVFAudio framework, which is linked.
        let reason_key = unsafe { AVAudioSessionRouteChangeReasonKey };
        route_change_event(number(notification, reason_key)?)
    }

    fn media_services_reset(_: &NSNotification) -> Option<OutputEvent> {
        Some(media_services_reset_event())
    }

    /// The unsigned number the notification's user info holds under `key`.
    fn number(notification: &NSNotification, key: Option<&NSString>) -> Option<usize> {
        let key: &AnyObject = key?.as_ref();
        let value = notification.userInfo()?.objectForKey(key)?;
        let number = value.downcast::<NSNumber>().ok()?;
        Some(number.unsignedIntegerValue())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interruption_begins_and_ends() {
        assert_eq!(
            interruption_event(INTERRUPTION_BEGAN, 0, 0, false),
            Some(OutputEvent::Interrupted)
        );
        assert_eq!(
            interruption_event(INTERRUPTION_ENDED, INTERRUPTION_SHOULD_RESUME, 0, false),
            Some(OutputEvent::InterruptionEnded { resume: true })
        );
        assert_eq!(
            interruption_event(INTERRUPTION_ENDED, 0, 0, false),
            Some(OutputEvent::InterruptionEnded { resume: false })
        );
        assert_eq!(interruption_event(7, 0, 0, false), None);
    }

    #[test]
    fn the_late_news_that_the_app_was_suspended_is_no_interruption() {
        assert_eq!(
            interruption_event(
                INTERRUPTION_BEGAN,
                0,
                INTERRUPTION_REASON_APP_WAS_SUSPENDED,
                false
            ),
            None
        );
        assert_eq!(interruption_event(INTERRUPTION_BEGAN, 0, 0, true), None);
        // Other reasons, such as a muted microphone or a route that went away, still interrupt.
        for reason in [2, 4] {
            assert_eq!(
                interruption_event(INTERRUPTION_BEGAN, 0, reason, false),
                Some(OutputEvent::Interrupted),
                "reason {reason}"
            );
        }
    }

    #[test]
    fn only_a_device_that_comes_or_goes_is_a_route_change_for_the_output() {
        assert_eq!(
            route_change_event(ROUTE_NEW_DEVICE_AVAILABLE),
            Some(OutputEvent::DeviceAdded)
        );
        assert_eq!(
            route_change_event(ROUTE_OLD_DEVICE_UNAVAILABLE),
            Some(OutputEvent::DeviceLost)
        );
        // Unknown, a category change, an override, waking from sleep, no suitable route and a
        // new configuration of the same route.
        for reason in [0, 3, 4, 6, 7, 8] {
            assert_eq!(route_change_event(reason), None, "reason {reason}");
        }
    }

    #[test]
    fn a_media_services_reset_does_not_resume_on_its_own() {
        assert_eq!(media_services_reset_event(), OutputEvent::DeviceLost);
    }
}
