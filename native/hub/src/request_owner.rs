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
    macros::{RequestScope, RequestSlots},
    node::NodeContext,
    request_error::ExpectedRequestError,
    rpc::{Slot, TargetRequest},
    subsonic_binding::SubsonicBinding,
    utils::{GlobalParams, ParamsExtractor},
    visualizer::RANDOM_COVERS,
};

/// The implementation currently connected to the UI as its library.
#[derive(Clone)]
pub(crate) enum OpenLibrary {
    Closed,
    Local,
    RunePeer {
        reconnecting: Option<String>,
        link: Option<Arc<RendererLink>>,
    },
    Subsonic(Arc<SubsonicBinding>),
}

/// The implementation currently connected to the UI as its player.
#[derive(Clone)]
pub(crate) enum CurrentPlayer {
    Local,
    RunePeer {
        link: Option<Arc<RendererLink>>,
        attached: bool,
        reconnecting: Option<String>,
    },
    SubsonicJukebox(Arc<SubsonicBinding>),
}

/// Executes requests for one concrete library or player implementation.
#[derive(Clone)]
pub(crate) enum Implementation {
    Local,
    RunePeer {
        link: Option<Arc<RendererLink>>,
        reconnecting: Option<String>,
    },
    Subsonic(Arc<SubsonicBinding>),
    #[cfg(test)]
    TestOverride {
        implementation: Box<Implementation>,
        slots: RequestSlots,
    },
}

impl Implementation {
    pub(crate) async fn visualizer_covers(
        &self,
        item: &PlayingItem,
        node: &NodeContext,
    ) -> Result<(Option<String>, Vec<String>)> {
        match self.execution_implementation() {
            Self::Subsonic(binding) => binding.visualizer_covers(item).await,
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
        match self.execution_implementation() {
            Self::Subsonic(binding) => {
                let PlayingItem::LibraryTrack { library_id, track } = item else {
                    anyhow::bail!("Subsonic library item is not a library track");
                };
                binding.item_metadata(*library_id, track, with_cover).await
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
        if let (Self::Subsonic(binding), PlayingItem::LibraryTrack { track, .. }) =
            (self.execution_implementation(), item)
        {
            binding.report_track_started_at(track, start_time).await;
        }
    }

    pub(crate) async fn track_played_through(&self, item: &PlayingItem) {
        if let (Self::Subsonic(binding), PlayingItem::LibraryTrack { track, .. }) =
            (self.execution_implementation(), item)
        {
            binding.report_track_played(track).await;
        }
    }

    pub(crate) async fn record_played(
        &self,
        item: &PlayingItem,
        library: &LibraryHandle,
    ) -> Result<()> {
        let Self::Subsonic(_) = self.execution_implementation() else {
            if let PlayingItem::LibraryTrack { track, .. } = item {
                let file_uuid =
                    uuid::Uuid::parse_str(track).context("Invalid Rune library track UUID")?;
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
            Self::RunePeer {
                link: Some(link), ..
            } => {
                let mut slots = RequestSlots::rune();
                slots.narrow_to_minor(link.peer_version().minor);
                slots
            }
            Self::RunePeer { .. } => RequestSlots::rune(),
            Self::Subsonic(binding) => RequestSlots::subsonic(&binding.probe),
            #[cfg(test)]
            Self::TestOverride { slots, .. } => *slots,
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
        match self.execution_implementation() {
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

        match R::slot(&self.slots()) {
            Slot::Native => {
                request
                    .handle(request.extract_params(params), None, request)
                    .await
            }
            Slot::Forward => {
                let Self::RunePeer {
                    link: Some(link), ..
                } = self.execution_implementation()
                else {
                    anyhow::bail!("Forward slot has no renderer link");
                };
                let response = if R::SCOPE == RequestScope::Library {
                    link.call_for_current_library(request).await?
                } else {
                    link.call(request).await?
                };
                Ok(Some(response))
            }
            Slot::Translate => {
                let Self::Subsonic(binding) = self.execution_implementation() else {
                    anyhow::bail!("Translate slot has no Subsonic binding");
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

    fn execution_implementation(&self) -> &Self {
        match self {
            #[cfg(test)]
            Self::TestOverride { implementation, .. } => implementation.execution_implementation(),
            implementation => implementation,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_slots(self, edit: impl FnOnce(&mut RequestSlots)) -> Self {
        let mut slots = self.slots();
        edit(&mut slots);
        Self::TestOverride {
            implementation: Box::new(self),
            slots,
        }
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
            Self::Subsonic(binding) => Implementation::Subsonic(Arc::clone(binding)),
        }
    }
}

impl CurrentPlayer {
    pub(crate) fn visualizer_inputs(&self) -> VisualizerInputs {
        match self {
            Self::Local => VisualizerInputs::Full,
            Self::RunePeer { .. } => VisualizerInputs::Full,
            Self::SubsonicJukebox(binding) => {
                if binding.probe.supports_rune_visualizer_feed() {
                    VisualizerInputs::Full
                } else {
                    VisualizerInputs::None
                }
            }
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
            Self::SubsonicJukebox(binding) => Implementation::Subsonic(Arc::clone(binding)),
        }
    }

    pub(crate) fn suppresses_local_session_events(&self) -> bool {
        matches!(self, Self::RunePeer { .. })
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
            attached: true,
            reconnecting: None,
        };
        assert_eq!(player.visualizer_inputs(), VisualizerInputs::Full);
    }
}
