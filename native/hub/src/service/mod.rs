//! What the hub needs from a streaming service's library, whichever service it is.

pub(crate) mod covers;
pub(crate) mod folder;
pub(crate) mod home;
pub(crate) mod ids;
pub(crate) mod lists;
pub(crate) mod mixes;
pub(crate) mod positional_occurrences;
pub(crate) mod reach;
pub(crate) mod registry;
pub(crate) mod store;

use std::sync::Arc;

use anyhow::Result;
use database::credentials::Credential;
use database::library_registry::ServiceKind;
use database::playing_item::PlayingItemMetadataSummary;
use futures::future::BoxFuture;
use playback::player::ServiceTarget;
use tokio::sync::watch;
use uuid::Uuid;
use visualizer::VisualizerInputs;

use crate::apple_music_binding::{APPLE_MUSIC_HOST, hosted_target};
use crate::macros::{RequestSlots, RequestTranslator};

pub(crate) type ServiceFuture<'a, T> = BoxFuture<'a, Result<T>>;

/// How `service` answers the requests routed by item about its own items, whichever library is
/// open, as its request table declares. The slots of other requests mean nothing here.
pub(crate) fn item_slots(service: ServiceKind) -> RequestSlots {
    match service {
        ServiceKind::AppleMusic => RequestSlots::apple_music_items(),
        ServiceKind::Subsonic => RequestSlots::subsonic_items(),
    }
}

/// The family of platform hosts that play the tracks of `service` on this device and report its
/// network, as the host names itself. None for a service that plays through none.
pub(crate) fn platform_host(service: ServiceKind) -> Option<&'static str> {
    match service {
        ServiceKind::AppleMusic => Some(APPLE_MUSIC_HOST),
        ServiceKind::Subsonic => None,
    }
}

/// How this device plays a track of `service` while it holds no binding for its library, as for
/// a queue restored before the library signs in. A service whose platform host plays its tracks
/// by their keys alone needs none. None for a service whose tracks need one.
pub(crate) fn playback_source_without_binding(
    service: ServiceKind,
    track: &str,
) -> Option<Result<ServiceTarget>> {
    match service {
        ServiceKind::AppleMusic => Some(hosted_target(track)),
        ServiceKind::Subsonic => None,
    }
}

/// One open connection to a streaming service's library. Catalogue requests in a `Translate`
/// slot go to its `RequestTranslator` methods, and everything else the hub asks of a service
/// goes through the methods below. Tracks are named by the ids the service gives them.
pub(crate) trait ServiceBinding: RequestTranslator + Send + Sync {
    /// The library id its tracks carry, which is the id of its library record.
    fn library_id(&self) -> Uuid;

    /// How each request runs against this service.
    fn slots(&self) -> RequestSlots;

    /// The service the library belongs to, which declares what its items support.
    fn service(&self) -> ServiceKind;

    /// Whether the binding holds a sign-in, without which the service answers nothing about
    /// its items.
    fn signed_in(&self) -> bool {
        true
    }

    /// Whether which of its tracks play depends on this device being online: offline, only
    /// the tracks the device stores play, and the rest are unavailable. The library says so
    /// itself. The default is that connectivity changes nothing.
    fn availability_follows_connectivity(&self) -> bool {
        false
    }

    /// The device went online or offline, or the tracks it stores changed. Called only for a
    /// library that declares `availability_follows_connectivity`, which reloads its lists.
    fn reach_changed(&self, _reach: &reach::Reach) {}

    /// The metadata of a track and, when `with_cover` is set, its cover if the cover is cached
    /// already.
    fn item_metadata<'a>(
        &'a self,
        track: &'a str,
        with_cover: bool,
    ) -> ServiceFuture<'a, (PlayingItemMetadataSummary, Option<String>)>;

    /// The metadata of a track from what the binding already holds, without asking the service.
    /// None when it has not seen the track.
    fn cached_summary(&self, _track: &str) -> Option<PlayingItemMetadataSummary> {
        None
    }

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

    /// How this device plays a track: from audio it decodes, or through a platform host that
    /// the service declares for it.
    fn playback_source<'a>(&'a self, track: &'a str) -> ServiceFuture<'a, ServiceTarget>;

    /// This device started playing a track at `start_time`, in milliseconds since the epoch.
    fn track_started<'a>(&'a self, _track: &'a str, _start_time: u64) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// This device played a track to its end.
    fn track_played_through<'a>(&'a self, _track: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Reads together what the binding does not hold yet of `tracks`, so they can be named
    /// without a request each, as the queued tracks of a library that just signed in. The
    /// default reads nothing.
    fn read_tracks(&self, _tracks: Vec<String>) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    /// The UI opened this library, so the binding may start reading what the UI will ask for.
    fn opened(&self) {}

    /// The service's own player, when the library's tracks play there rather than on this
    /// device. None when this device plays them.
    fn own_player(self: Arc<Self>) -> Option<Arc<dyn ServicePlayer>> {
        None
    }

    /// Signs in to the service again with `credential`, the one its library record keeps from
    /// now on, without the library being opened again. A credential of another account clears the
    /// lists the binding keeps, on disk too; one of the same account keeps them. A service that
    /// does not sign in with a kept credential has nothing to renew.
    fn renew_sign_in(&self, _credential: &Credential) {}

    /// Stops the background work done for this library while it was open in the UI. The hub
    /// calls it only when it closes the UI's library: the UI closes, another UI attaches, or
    /// the node shuts down. Opening another library in its place just drops the binding, so a
    /// binding must also stop that work when it is dropped.
    fn close(&self) {}
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
