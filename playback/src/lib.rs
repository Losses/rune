mod internal;
mod realtime_fft;
mod sfx_internal;
mod shared_source;

pub mod buffered;
pub mod controller;
pub mod output_stream;
pub mod player;
pub mod sfx_player;
pub mod strategies;
pub mod stream_utils;

#[cfg(target_os = "android")]
mod dummy_souvlaki;

#[cfg(target_os = "android")]
pub use dummy_souvlaki::{MediaMetadata, MediaPlayback, MediaPosition};

#[cfg(not(target_os = "android"))]
pub use souvlaki::{MediaMetadata, MediaPlayback, MediaPosition};

pub use internal::{PlayerCommand, PlayerEvent};

#[cfg(target_os = "android")]
pub mod android_utils;
