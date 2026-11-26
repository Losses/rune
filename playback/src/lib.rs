#[cfg(any(test, target_os = "android"))]
mod android_media_session;
mod internal;
#[cfg(any(test, target_os = "ios", target_os = "android"))]
mod ios_now_playing;
#[cfg(any(test, target_os = "ios"))]
mod ios_session;
mod listen;
mod realtime_fft;
mod sfx_internal;
mod shared_source;

pub mod buffered;
pub mod controller;
pub mod decoder;
pub mod hosted;
pub mod output_stream;
pub mod pcm_tap;
pub mod player;
pub mod sfx_player;
pub mod strategies;
pub mod stream_utils;

#[cfg(any(target_os = "android", target_os = "ios"))]
mod dummy_souvlaki;

#[cfg(any(target_os = "android", target_os = "ios"))]
pub use dummy_souvlaki::{MediaMetadata, MediaPlayback, MediaPosition};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use souvlaki::{MediaMetadata, MediaPlayback, MediaPosition};

pub use internal::{InternalLog, OutputEvent, PlaybackMode, PlayerCommand, PlayerEvent};
/// What the status and event subscriptions of a [`player::Playable`] return.
pub use simple_channel::SimpleReceiver;

#[cfg(target_os = "android")]
pub mod android_utils;
