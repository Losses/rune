mod internal;
mod sfx_internal;
mod realtime_fft;
mod simple_channel;

pub mod strategies;
pub mod output_stream;
pub mod controller;
pub mod player;
pub mod sfx_player;

#[cfg(target_os = "android")]
mod dummy_souvlaki;

#[cfg(not(target_os = "android"))]
pub use souvlaki::{MediaMetadata, MediaPlayback, MediaPosition};

#[cfg(target_os = "android")]
pub use dummy_souvlaki::{MediaMetadata, MediaPlayback, MediaPosition};

pub use internal::{PlayerCommand, PlayerEvent};

#[cfg(target_os = "android")]
pub mod android_utils;
