use anyhow::{Context, Result};

use ::database::actions::{
    file::file_ids_by_uuid,
    stats::{get_liked, set_liked},
};
use ::database::error::ExpectedError;
use ::database::library_registry::LibraryAccess;
use ::playback::player::PlayingItem;

use std::{path::Path, sync::Arc};

use crate::{
    Session, Signal,
    handlers::playback::{playing_item_from_request, reject_remote_independent_item},
    library_pool::{LibraryHandle, LibraryPool},
    messages::*,
    node::NodeContext,
    request_error::ExpectedRequestError,
    subsonic_binding::SubsonicBinding,
    subsonic_translate::{get_liked_request, set_liked_request},
    utils::{GlobalParams, ParamsExtractor, require_library, subsonic_binding_for_playback},
};
use ::database::library_registry::{LibraryRecordId, LibraryRegistry, ProtocolAdapter};

type LikedParams = (Arc<NodeContext>, Arc<LibraryPool>, Option<LibraryHandle>);

fn liked_params(all_params: &GlobalParams) -> LikedParams {
    (
        all_params.node.clone(),
        all_params.library_pool.clone(),
        all_params.library.clone(),
    )
}

async fn item_subsonic_binding(
    node: &Arc<NodeContext>,
    item: &PlayingItemRef,
) -> Result<Option<Arc<SubsonicBinding>>> {
    let PlayingItemRef::LibraryTrack { library_id, .. } = item else {
        return Ok(None);
    };
    let library_id = uuid::Uuid::parse_str(library_id).map_err(|error| {
        ExpectedRequestError::invalid_input(format!("Invalid library id: {error}"))
    })?;
    let registry = LibraryRegistry::load(Path::new(&*node.config_path))?;
    let Ok(record) = registry.record(LibraryRecordId(library_id)) else {
        return Ok(None);
    };
    if !matches!(&record.protocol, ProtocolAdapter::SubsonicTranslate { .. }) {
        return Ok(None);
    }
    if let Some(binding) = node.subsonic_binding(library_id)? {
        return Ok(Some(binding));
    }
    subsonic_binding_for_playback(node, library_id)
        .await
        .map(Some)
}

async fn item_library(pool: &Arc<LibraryPool>, item: &PlayingItemRef) -> Result<LibraryHandle> {
    let PlayingItemRef::LibraryTrack { library_id, .. } = item else {
        anyhow::bail!("Liked state requires a library track");
    };
    let library_id = uuid::Uuid::parse_str(library_id).map_err(|error| {
        ExpectedRequestError::invalid_input(format!("Invalid library id: {error}"))
    })?;
    pool.acquire_for_request(library_id).await
}

impl ParamsExtractor for SetLikedRequest {
    type Params = LikedParams;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        liked_params(all_params)
    }
}

impl Signal for SetLikedRequest {
    type Params = LikedParams;
    type Response = SetLikedResponse;

    async fn handle(
        &self,
        (node, pool, ui_library): Self::Params,
        session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        if let Some(item) = &request.item {
            reject_remote_independent_item(session.as_ref(), item)?;
            if let Some(binding) = item_subsonic_binding(&node, item).await? {
                return set_liked_request(&binding, request).await;
            }
            let library = match item {
                PlayingItemRef::LibraryTrack { .. } => item_library(&pool, item).await?,
                PlayingItemRef::IndependentFile { .. } => require_library(ui_library)?,
            };
            let parsed_item = playing_item_from_request(item.clone(), Some(&library)).await?;

            let PlayingItem::LibraryTrack { library_id, track } = parsed_item else {
                return Err(ExpectedRequestError::invalid_input(
                    "only library tracks have a liked state",
                )
                .into());
            };
            if library_id != library.library_id {
                return Err(ExpectedRequestError::invalid_input(
                    "request library does not match the item's library",
                )
                .into());
            }
            let file_uuid = uuid::Uuid::parse_str(&track).map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid track id: {error}"))
            })?;
            if library.descriptor.access_level() == Some(LibraryAccess::Read) {
                return Err(ExpectedRequestError::Unavailable {
                    what: "write access to liked state",
                }
                .into());
            }
            let file_id = file_ids_by_uuid(&library.main_db, &[file_uuid])
                .await?
                .get(&file_uuid)
                .copied()
                .ok_or(ExpectedError::NotFound { what: "track" })?;
            set_liked(
                &library.main_db,
                &library.library_sync.record_store(),
                file_id,
                request.liked,
            )
            .await
            .with_context(|| {
                format!(
                    "Failed to set liked: file_id={}, liked={}",
                    file_id, request.liked
                )
            })?;

            let response = SetLikedResponse {
                call_id: None,
                item: item.clone(),
                liked: request.liked,
            };

            return Ok(Some(response));
        }

        Err(ExpectedRequestError::invalid_input("a liked-state request needs an item").into())
    }
}

impl ParamsExtractor for GetLikedRequest {
    type Params = LikedParams;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        liked_params(all_params)
    }
}

impl Signal for GetLikedRequest {
    type Params = LikedParams;
    type Response = GetLikedResponse;

    async fn handle(
        &self,
        (node, pool, ui_library): Self::Params,
        session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        if let Some(item) = &request.item {
            reject_remote_independent_item(session.as_ref(), item)?;
            if let Some(binding) = item_subsonic_binding(&node, item).await? {
                return get_liked_request(&binding, request).await;
            }
            let library = match item {
                PlayingItemRef::LibraryTrack { .. } => item_library(&pool, item).await?,
                PlayingItemRef::IndependentFile { .. } => require_library(ui_library)?,
            };
            let parsed_item = playing_item_from_request(item.clone(), Some(&library)).await?;

            let response = match parsed_item {
                PlayingItem::LibraryTrack { library_id, track }
                    if library_id == library.library_id
                        && let Ok(file_uuid) = uuid::Uuid::parse_str(&track) =>
                {
                    let Some(file_id) = file_ids_by_uuid(&library.main_db, &[file_uuid])
                        .await?
                        .get(&file_uuid)
                        .copied()
                    else {
                        return Ok(Some(GetLikedResponse {
                            call_id: None,
                            item: item.clone(),
                            liked: false,
                        }));
                    };
                    let liked = get_liked(&*library.main_db, file_id)
                        .await
                        .with_context(|| format!("Failed to get liked: file_id={file_id}"))?;

                    GetLikedResponse {
                        call_id: None,
                        item: item.clone(),
                        liked,
                    }
                }
                PlayingItem::IndependentFile(_) => GetLikedResponse {
                    call_id: None,
                    item: item.clone(),
                    liked: false,
                },
                PlayingItem::LibraryTrack { .. } | PlayingItem::Online(_, _) => GetLikedResponse {
                    call_id: None,
                    item: item.clone(),
                    liked: false,
                },
                PlayingItem::Unknown => GetLikedResponse {
                    call_id: None,
                    item: item.clone(),
                    liked: false,
                },
            };

            return Ok(Some(response));
        }

        Err(ExpectedRequestError::invalid_input("a liked-state request needs an item").into())
    }
}
