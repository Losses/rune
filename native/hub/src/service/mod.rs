//! What the hub needs from a streaming service's library, whichever service it is.

#[cfg(test)]
use std::any::Any;
use std::sync::Arc;

use anyhow::Result;
use database::credentials::Credential;
use database::playing_item::PlayingItemMetadataSummary;
use futures::future::BoxFuture;
use playback::player::RemoteAudioSource;
use tokio::sync::watch;
use uuid::Uuid;
use visualizer::VisualizerInputs;

use crate::macros::{RequestSlots, RequestTranslator};

pub(crate) type ServiceFuture<'a, T> = BoxFuture<'a, Result<T>>;

/// One open connection to a streaming service's library. Catalogue requests in a `Translate`
/// slot go to its `RequestTranslator` methods, and everything else the hub asks of a service
/// goes through the methods below. Tracks are named by the ids the service gives them.
pub(crate) trait ServiceBinding: RequestTranslator + Send + Sync {
    /// The library id its tracks carry, which is the id of its library record.
    fn library_id(&self) -> Uuid;

    /// How each request runs against this service.
    fn slots(&self) -> RequestSlots;

    /// The metadata of a track and, when `with_cover` is set, its cover if the cover is cached
    /// already.
    fn item_metadata<'a>(
        &'a self,
        track: &'a str,
        with_cover: bool,
    ) -> ServiceFuture<'a, (PlayingItemMetadataSummary, Option<String>)>;

    /// The cover of a track that `item_metadata` gave without one because it was not cached
    /// yet. The cover is fetched, and its path given once it is cached. None when the track has
    /// no cover or the service does not fetch covers this way.
    fn cover_when_cached<'a>(&'a self, _track: &'a str) -> ServiceFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    /// The cover of a track and other covers of the library, for the visualizer's cover wall.
    fn visualizer_covers<'a>(
        &'a self,
        track: &'a str,
    ) -> ServiceFuture<'a, (Option<String>, Vec<String>)>;

    /// The audio of a track, which this device decodes.
    fn playback_source<'a>(&'a self, track: &'a str) -> ServiceFuture<'a, RemoteAudioSource>;

    /// This device started playing a track at `start_time`, in milliseconds since the epoch.
    fn track_started<'a>(&'a self, _track: &'a str, _start_time: u64) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// This device played a track to its end.
    fn track_played_through<'a>(&'a self, _track: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// The service's own player, when the library's tracks play there rather than on this
    /// device. None when this device plays them.
    fn own_player(self: Arc<Self>) -> Option<Arc<dyn ServicePlayer>> {
        None
    }

    /// Signs in to the service again with `credential`, the one its library record keeps from
    /// now on, without the library being opened again. A service that does not sign in with a
    /// kept credential has nothing to renew.
    fn renew_sign_in(&self, _credential: &Credential) {}

    /// Stops the background work done for this library while it was open in the UI. The hub
    /// calls it only when it closes the UI's library: the UI closes, another UI attaches, or
    /// the node shuts down. Opening another library in its place just drops the binding, so a
    /// binding must also stop that work when it is dropped.
    fn close(&self) {}

    /// The binding as its service's own type, for tests that read the state of that service.
    #[cfg(test)]
    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

/// The player of a service that plays its library's tracks itself, such as a Subsonic server's
/// jukebox. While the library is open, that player is the UI's player, and the requests made of
/// it go to its binding.
pub(crate) trait ServicePlayer: ServiceBinding {
    /// What the visualizer can use from the player right now.
    fn visualizer_inputs(&self) -> VisualizerInputs;

    /// Wakes once for every change of what the visualizer can use from the player.
    fn visualizer_input_changes(&self) -> watch::Receiver<u64>;

    /// Starts following the player, once it has become the UI's player. `ServiceBinding::close`
    /// stops it, as does dropping the binding.
    fn follow(self: Arc<Self>);
}
