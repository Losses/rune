use std::slice::from_ref;
use std::sync::Arc;

use anyhow::{Context, Result};
use database::{
    actions::{
        cover_art::bake_cover_art_by_media_files, file::file_ids_by_uuid,
        mixes::query_mix_media_files, stats::increase_played_through,
    },
    playing_item::{PlayingItemMetadataSummary, dispatcher::PlayingItemActionDispatcher},
};
use playback::player::PlayingItem;
use rinf::DartSignal;
use serde::{Serialize, de::DeserializeOwned};
use visualizer::VisualizerInputs;

use crate::{
    Signal, TypedRequest,
    backends::remote::RendererLink,
    library_pool::LibraryHandle,
    macros::{BodyLibraries, RequestScope, RequestSlots},
    messages::{CastLibraryState, RemoveDownloadRuleRequest, SetDownloadRuleRequest},
    node::NodeContext,
    request_error::ExpectedRequestError,
    rpc::{Slot, TargetRequest},
    service::{ServiceBinding, ServicePlayer, item_slots},
    utils::{GlobalParams, ParamsExtractor},
    visualizer::RANDOM_COVERS,
};
use uuid::Uuid;

/// The implementation currently connected to the UI as its library.
#[derive(Clone)]
pub(crate) enum OpenLibrary {
    Closed,
    Local,
    RunePeer {
        reconnecting: Option<String>,
        link: Option<Arc<RendererLink>>,
    },
    Service(Arc<dyn ServiceBinding>),
}

/// The implementation currently connected to the UI as its player.
#[derive(Clone)]
pub(crate) enum CurrentPlayer {
    Local,
    RunePeer {
        link: Option<Arc<RendererLink>>,
        reconnecting: Option<String>,
    },
    /// The own player of the open service library, which its binding controls.
    ServicePlayer(Arc<dyn ServicePlayer>),
}

/// Refuses to send another device anything that names a library kept on this device, such as a
/// streaming service account signed in here: no other device holds that library, so none can
/// play or read it. It is the rule the websocket filter applies to what this device
/// broadcasts, and like it, it counts a library as kept when the policy cannot tell.
/// `visit_libraries` visits the libraries a request names. Items to load onto the other
/// device's player are refused as a cast that is not possible, anything else as unavailable.
pub(crate) fn refuse_libraries_kept_here<'a>(
    node: &NodeContext,
    loads_player: bool,
    visit_libraries: impl FnOnce(&mut dyn FnMut(&'a str)),
) -> Result<()> {
    let mut kept = None;
    visit_libraries(&mut |library_id: &'a str| {
        if kept.is_none() && library_kept_here(node, library_id) {
            kept = Some(library_id);
        }
    });
    match kept {
        Some(library_id) if loads_player => Err(ExpectedRequestError::CastNotPossible {
            library_id: library_id.to_owned(),
            state: CastLibraryState::NotHeld,
        }
        .into()),
        Some(_) => Err(ExpectedRequestError::Unavailable {
            what: "library kept on this device",
        }
        .into()),
        None => Ok(()),
    }
}

/// Whether the library of `library_id`, as it appears on the wire, is kept on this device.
fn library_kept_here(node: &NodeContext, library_id: &str) -> bool {
    Uuid::parse_str(library_id)
        .is_ok_and(|library_id| node.access_policy.is_device_only(library_id))
}

/// Whether `request` names a library and every library it names is kept on this device.
fn names_only_libraries_kept_here<R: TargetRequest>(node: &NodeContext, request: &R) -> bool {
    let (mut named, mut all_kept) = (false, true);
    request.visit_forwarded_libraries(&mut |library_id| {
        named = true;
        all_kept &= library_kept_here(node, library_id);
    });
    named && all_kept
}

/// A copy of `request` without the items it lists of libraries kept on this device, when it
/// lists any: it names them only to skip or select them, and the other device has no use for
/// them. None when it lists none, so the request goes as it is.
fn without_items_kept_here<R>(node: &NodeContext, request: &R) -> Result<Option<R>>
where
    R: TargetRequest + DartSignal + Serialize,
{
    let kept = |library_id: &str| library_kept_here(node, library_id);
    if !request.forwarded_lists_kept(&kept) {
        return Ok(None);
    }
    let mut copy: R = rinf::deserialize(&rinf::serialize(request)?)?;
    copy.leave_kept_out_of_forwarded(&kept);
    Ok(Some(copy))
}

/// Executes requests for one concrete library or player implementation.
#[derive(Clone)]
pub(crate) enum Implementation {
    Local,
    /// This device, for the requests it answers itself while a library that declares its own
    /// requests is open in its UI. The requests that work on that library's own data follow what
    /// it declares for them (see `RequestSlots::following_library`).
    LocalBeside(Box<RequestSlots>),
    RunePeer {
        link: Option<Arc<RendererLink>>,
        reconnecting: Option<String>,
    },
    Service(Arc<dyn ServiceBinding>),
}

impl Implementation {
    pub(crate) async fn visualizer_covers(
        &self,
        item: &PlayingItem,
        node: &NodeContext,
    ) -> Result<(Option<String>, Vec<String>)> {
        match self {
            Self::Service(binding) => {
                let PlayingItem::LibraryTrack { track, .. } = item else {
                    anyhow::bail!("Service visualizer item is not a library track");
                };
                binding.visualizer_covers(track).await
            }
            _ => {
                let PlayingItem::LibraryTrack { library_id, .. } = item else {
                    anyhow::bail!("Visualizer item does not belong to a library");
                };
                let library = node.library_pool.acquire(*library_id).await?;
                let (_, playing) = self.item_metadata(item, true, &library).await?;
                let queue = node.session_queue_for_library(*library_id).await?;
                let files = query_mix_media_files(
                    &library.main_db,
                    &library.recommend_db,
                    vec![
                        ("lib::random".to_owned(), RANDOM_COVERS.to_string()),
                        ("filter::with_cover_art".to_owned(), "true".to_owned()),
                    ],
                    0,
                    RANDOM_COVERS,
                    &queue,
                )
                .await?;
                let ids = files.iter().map(|file| file.id).collect::<Vec<_>>();
                let paths =
                    bake_cover_art_by_media_files(&library.fsio, &library.main_db, files).await?;
                Ok((
                    playing,
                    ids.into_iter()
                        .filter_map(|id| paths.get(&id))
                        .filter(|path| !path.is_empty())
                        .cloned()
                        .collect(),
                ))
            }
        }
    }

    pub(crate) async fn item_metadata(
        &self,
        item: &PlayingItem,
        with_cover: bool,
        library: &LibraryHandle,
    ) -> Result<(PlayingItemMetadataSummary, Option<String>)> {
        match self {
            Self::Service(binding) => {
                let PlayingItem::LibraryTrack { track, .. } = item else {
                    anyhow::bail!("Service library item is not a library track");
                };
                binding.item_metadata(track, with_cover).await
            }
            _ => {
                let dispatcher = PlayingItemActionDispatcher::new();
                let items = from_ref(item);
                let metadata = dispatcher
                    .get_metadata_summary(&library.fsio, &library.main_db, items)
                    .await?
                    .into_iter()
                    .next()
                    .context("Track metadata unavailable")?;
                let cover = if with_cover {
                    dispatcher
                        .bake_cover_art(&library.fsio, &library.media_root, &library.main_db, items)
                        .await?
                        .remove(item)
                } else {
                    None
                };
                Ok((metadata, cover))
            }
        }
    }

    pub(crate) async fn track_started(&self, item: &PlayingItem, start_time: u64) {
        if let (Self::Service(binding), PlayingItem::LibraryTrack { track, .. }) = (self, item) {
            binding.track_started(track, start_time).await;
        }
    }

    pub(crate) async fn track_played_through(&self, item: &PlayingItem) {
        if let (Self::Service(binding), PlayingItem::LibraryTrack { track, .. }) = (self, item) {
            binding.track_played_through(track).await;
        }
    }

    pub(crate) async fn record_played(
        &self,
        item: &PlayingItem,
        library: &LibraryHandle,
    ) -> Result<()> {
        // A service counts the plays of its own tracks, which have no local statistics.
        let Self::Service(_) = self else {
            if let PlayingItem::LibraryTrack { track, .. } = item {
                let file_uuid =
                    Uuid::parse_str(track).context("Invalid Rune library track UUID")?;
                let ids = file_ids_by_uuid(&library.main_db, &[file_uuid]).await?;
                let id = ids
                    .get(&file_uuid)
                    .context("Playing track no longer exists in its library")?;
                let store = library.library_sync.record_store();
                increase_played_through(&library.main_db, &store, *id, store.author()).await?;
            }
            return Ok(());
        };
        Ok(())
    }

    pub(crate) fn slots(&self) -> RequestSlots {
        match self {
            Self::Local => RequestSlots::local(),
            Self::LocalBeside(library) => RequestSlots::local().following_library(library),
            Self::RunePeer {
                link: Some(link), ..
            } => {
                let mut slots = RequestSlots::rune();
                slots.narrow_to_minor(link.peer_version().minor);
                slots
            }
            Self::RunePeer { .. } => RequestSlots::rune(),
            Self::Service(binding) => binding.slots(),
        }
    }

    /// How this implementation answers the requests routed by item about the items of its own
    /// library. The slots of other requests mean nothing here. An open service library leaves
    /// those requests to the library of each item named, while its own items have what the
    /// service declares for them.
    pub(crate) fn item_slots(&self) -> RequestSlots {
        match self {
            Self::Service(binding) => item_slots(binding.service()),
            // A Rune library answers about its items as it answers as the open library: here,
            // or through the device that holds it. The session resolves each item it plays to
            // the library this device holds of it, so a peer's slots apply only to an item a
            // peer answers for.
            Self::Local | Self::LocalBeside(_) | Self::RunePeer { .. } => self.slots(),
        }
    }

    pub(crate) async fn run<R>(
        &self,
        params: &GlobalParams,
        request: &R,
    ) -> Result<Option<<R as TypedRequest>::Response>>
    where
        R: TargetRequest
            + Signal<
                Response = <R as TypedRequest>::Response,
                Params = <R as ParamsExtractor>::Params,
            > + ParamsExtractor
            + DartSignal
            + Serialize,
        <R as TypedRequest>::Response: DeserializeOwned,
    {
        let mut slot = R::slot(&self.slots());
        let mut answered_here = false;
        // An item of a library kept here is answered here, by the binding of its own library,
        // as when a library of this device is open, even while the remote library reconnects.
        if slot == Slot::Forward
            && R::ROUTED_BY_ITEM
            && names_only_libraries_kept_here(&params.node, request)
        {
            slot = Slot::Native;
            answered_here = true;
        }

        if !answered_here {
            match self {
                Self::RunePeer {
                    reconnecting: Some(alias),
                    ..
                } => {
                    return Err(ExpectedRequestError::Reconnecting {
                        alias: alias.clone(),
                    }
                    .into());
                }
                Self::RunePeer { link: None, .. } => {
                    return Err(ExpectedRequestError::Unavailable {
                        what: "remote UI library",
                    }
                    .into());
                }
                _ => {}
            }
        }

        match slot {
            Slot::Native => {
                request
                    .handle(request.extract_params(params), None, request)
                    .await
            }
            Slot::Forward => {
                let Self::RunePeer {
                    link: Some(link), ..
                } = self
                else {
                    anyhow::bail!("Forward slot has no renderer link");
                };
                // Only a request whose body names items for a player loads them onto one.
                let loads_player = matches!(R::BODY_LIBRARIES, BodyLibraries::PlayerReads);
                refuse_libraries_kept_here(&params.node, loads_player, |visit| {
                    request.visit_forwarded_libraries(visit)
                })?;
                let copy = without_items_kept_here(&params.node, request)?;
                let request = copy.as_ref().unwrap_or(request);
                let response = if R::SCOPE == RequestScope::Library {
                    link.call_for_current_library(request).await?
                } else {
                    link.call(request).await?
                };
                Ok(Some(response))
            }
            Slot::Translate => {
                let Self::Service(binding) = self else {
                    anyhow::bail!("Translate slot has no service binding");
                };
                request.translate(binding.as_ref()).await
            }
            Slot::Unsupported { reason } => Err(ExpectedRequestError::Unsupported {
                request: R::wire_name(),
                reason,
            }
            .into()),
        }
    }

    /// Whether requests run on this device's own player and libraries.
    pub(crate) fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }
}

impl OpenLibrary {
    pub(crate) fn implementation(&self) -> Implementation {
        match self {
            Self::Closed | Self::Local => Implementation::Local,
            Self::RunePeer { link, reconnecting } => Implementation::RunePeer {
                link: link.as_ref().map(Arc::clone),
                reconnecting: reconnecting.clone(),
            },
            Self::Service(binding) => Implementation::Service(Arc::clone(binding)),
        }
    }

    /// This device, for the requests it answers itself, beside what the open library declares.
    /// A Rune peer's library still leaves them all to this device.
    pub(crate) fn node_implementation(&self) -> Implementation {
        match self {
            Self::Closed | Self::Local | Self::RunePeer { .. } => Implementation::Local,
            Self::Service(binding) => Implementation::LocalBeside(Box::new(binding.slots())),
        }
    }
}

impl RequestSlots {
    /// This device's own requests beside an open library that declares `library`. The offline
    /// selection is kept in the open library's own data, so where that library does not support
    /// it, neither does this device. Every other request stays this device's own.
    fn following_library(mut self, library: &RequestSlots) -> Self {
        self.withdraw_where_unsupported::<SetDownloadRuleRequest>(library);
        self.withdraw_where_unsupported::<RemoveDownloadRuleRequest>(library);
        self
    }

    fn withdraw_where_unsupported<R: TargetRequest>(&mut self, library: &RequestSlots) {
        if let slot @ Slot::Unsupported { .. } = R::slot(library) {
            self.set::<R>(slot);
        }
    }
}

#[cfg(test)]
mod following_library_tests {
    use super::*;
    use crate::{messages::ListLocalLibrariesRequest, rpc::UnsupportedReason};

    #[test]
    fn this_device_withdraws_only_the_offline_selection_a_library_does_not_support() {
        let unsupported = Slot::Unsupported {
            reason: UnsupportedReason::NotImplemented,
        };
        let mut library = RequestSlots::local();
        library.set::<SetDownloadRuleRequest>(unsupported);
        library.set::<ListLocalLibrariesRequest>(unsupported);

        let slots = RequestSlots::local().following_library(&library);

        assert_eq!(SetDownloadRuleRequest::slot(&slots), unsupported);
        assert_eq!(RemoveDownloadRuleRequest::slot(&slots), Slot::Native);
        assert_eq!(ListLocalLibrariesRequest::slot(&slots), Slot::Native);
    }
}

impl CurrentPlayer {
    pub(crate) fn visualizer_inputs(&self) -> VisualizerInputs {
        match self {
            Self::Local => VisualizerInputs::Full,
            Self::RunePeer { .. } => VisualizerInputs::Full,
            Self::ServicePlayer(player) => player.visualizer_inputs(),
        }
    }

    pub(crate) fn implementation(&self) -> Implementation {
        match self {
            Self::Local => Implementation::Local,
            Self::RunePeer {
                link, reconnecting, ..
            } => Implementation::RunePeer {
                link: link.as_ref().map(Arc::clone),
                reconnecting: reconnecting.clone(),
            },
            Self::ServicePlayer(player) => Implementation::Service(Arc::clone(player) as _),
        }
    }

    pub(crate) fn suppresses_local_session_events(&self) -> bool {
        match self {
            Self::Local => false,
            Self::RunePeer { .. } | Self::ServicePlayer(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CurrentPlayer;
    use visualizer::VisualizerInputs;

    #[test]
    fn local_and_rune_players_feed_the_full_visualizer() {
        assert_eq!(
            CurrentPlayer::Local.visualizer_inputs(),
            VisualizerInputs::Full
        );
        let player = CurrentPlayer::RunePeer {
            link: None,
            reconnecting: None,
        };
        assert_eq!(player.visualizer_inputs(), VisualizerInputs::Full);
    }
}
