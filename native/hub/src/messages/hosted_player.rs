use rinf::{DartSignal, RustSignal};
use serde::{Deserialize, Serialize};

/// A command for the platform host on this device, as the JSON of a `HostCommand`. Never sent to
/// other devices.
#[derive(Clone, Deserialize, Serialize, RustSignal)]
pub struct HostedPlayerCommand {
    pub payload: String,
}

/// What the platform host on this device reports, as the JSON of a `HostEvent`.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct HostedPlayerEvent {
    pub payload: String,
}

/// Whether the current track of this device's own player is played by a platform host, for the UI
/// here: the volume control stands for the system volume then, unless the host has a volume of
/// its own. It says how this device plays, so it goes to the UI of this device only. It is not
/// part of `PlayingItemCapabilities`, which tells every reader what may be done with the item. It
/// is `false` as soon as another player is current, as with a renderer or a service's own player,
/// whatever this device's player holds, and while this device's player has no track.
#[derive(Clone, Deserialize, Serialize, RustSignal)]
pub struct PlayerHostChanged {
    pub hosted: bool,
    /// Whether that host has a volume of its own, which the player's volume sets, as its
    /// `hostReady` said. The volume control stays Rune's own volume then. Never `true` without
    /// `hosted`.
    pub own_volume: bool,
}

crate::implement_rinf_rust_signal_trait!(PlayerHostChanged);
