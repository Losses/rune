mod internal;
mod sfx_internal;
mod realtime_fft;
mod output_stream;

pub mod strategies;

#[cfg(target_os = "android")]
mod dummy_souvlaki;

pub mod controller;
pub mod player;
pub mod sfx_player;

#[cfg(not(target_os = "android"))]
pub use souvlaki::{MediaMetadata, MediaPlayback, MediaPosition};

#[cfg(target_os = "android")]
pub use dummy_souvlaki::{MediaMetadata, MediaPlayback, MediaPosition};

pub use internal::{PlayerCommand, PlayerEvent};

#[cfg(target_os = "android")]
pub mod android_utils;
