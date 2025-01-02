/// A dummy implementation of the media controls.
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

pub struct PlatformConfig {
    pub dbus_name: &'static str,
    pub display_name: &'static str,
    pub hwnd: Option<()>,
}

/// A platform-specific error.
#[derive(Debug)]
pub struct Error;

/// A handle to OS media controls.
pub struct MediaControls;

impl MediaControls {
    /// Create media controls with the specified config.
    pub fn new(_config: PlatformConfig) -> Result<Self, Error> {
        Ok(Self)
    }

    /// Attach the media control events to a handler.
    pub fn attach<F>(&mut self, _event_handler: F) -> Result<(), Error>
    where
        F: Fn(MediaControlEvent) + Send + 'static,
    {
        Ok(())
    }

    /// Detach the event handler.
    pub fn detach(&mut self) -> Result<(), Error> {
        Ok(())
    }

    /// Set the current playback status.
    pub fn set_playback(&mut self, _playback: MediaPlayback) -> Result<(), Error> {
        Ok(())
    }

    /// Set the metadata of the currently playing media item.
    pub fn set_metadata(&mut self, _metadata: MediaMetadata) -> Result<(), Error> {
        Ok(())
    }
}
