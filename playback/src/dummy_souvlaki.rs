//! Souvlaki's types for Android and iOS, where souvlaki is not built. The media controls
//! themselves are `android_media_session`'s and `ios_now_playing`'s.

use std::time::Duration;

/// The metadata of a media item.
#[derive(Clone, Debug, Default)]
pub struct MediaMetadata<'a> {
    pub title: Option<&'a str>,
    pub album: Option<&'a str>,
    pub artist: Option<&'a str>,
    pub cover_url: Option<&'a str>,
    pub duration: Option<Duration>,
}

/// The status of media playback.
#[derive(Clone, Debug)]
pub enum MediaPlayback {
    Stopped,
    Paused { progress: Option<MediaPosition> },
    Playing { progress: Option<MediaPosition> },
}

#[derive(Debug, Clone, Copy)]
pub struct MediaPosition(pub Duration);

#[derive(Clone, Debug)]
pub enum MediaControlEvent {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    Seek(SeekDirection),
    SetPosition(MediaPosition),
}

#[derive(Clone, Debug)]
pub enum SeekDirection {
    Forward,
    Backward,
}

#[allow(dead_code)]
pub struct PlatformConfig<'a> {
    pub dbus_name: &'a str,
    pub display_name: &'a str,
    pub hwnd: Option<()>,
}

/// A platform-specific error.
#[derive(Debug)]
pub struct Error;
