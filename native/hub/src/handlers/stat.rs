use anyhow::{Context, Result};

use ::database::actions::{
    file::file_ids_by_uuid,
    stats::{get_liked, set_liked},
};
use ::database::error::ExpectedError;
use ::database::library_registry::LibraryAccess;
use ::playback::player::PlayingItem;

use std::sync::Arc;

use crate::{
    Session, Signal,
    handlers::playback::{
        item_service_binding, playing_item_from_request, reject_remote_independent_item,
    },
    library_pool::{LibraryHandle, LibraryPool},
    messages::*,
    node::NodeContext,
    request_error::ExpectedRequestError,
    utils::{GlobalParams, ParamsExtractor, require_library},
};
use uuid::Uuid;

type LikedParams = (Arc<NodeContext>, Arc<LibraryPool>, Option<LibraryHandle>);

fn liked_params(all_params: &GlobalParams) -> LikedParams {
    (
        all_params.node.clone(),
        all_params.library_pool.clone(),
        all_params.library.clone(),
    )
}

/// The library of a library track. A request's own library is used when it is the track's,
/// so that a like from another device keeps the commit permit of the library it addressed.
async fn item_library(
    pool: &Arc<LibraryPool>,
    request_library: Option<&LibraryHandle>,
    item: &PlayingItemRef,
) -> Result<LibraryHandle> {
    let PlayingItemRef::LibraryTrack { library_id, .. } = item else {
        anyhow::bail!("Liked state requires a library track");
    };
    let library_id = Uuid::parse_str(library_id).map_err(|error| {
        ExpectedRequestError::invalid_input(format!("Invalid library id: {error}"))
    })?;
    match request_library.filter(|library| library.library_id == library_id) {
        Some(library) => Ok(library.clone()),
        None => pool.acquire_for_request(library_id).await,
    }
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
            if let Some(binding) = item_service_binding(&node, ui_library.as_ref(), item).await? {
                return binding.translate_set_liked_request(request).await;
            }
            let library = match item {
                PlayingItemRef::LibraryTrack { .. } => {
                    item_library(&pool, ui_library.as_ref(), item).await?
                }
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
            let file_uuid = Uuid::parse_str(&track).map_err(|error| {
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
            if let Some(binding) = item_service_binding(&node, ui_library.as_ref(), item).await? {
                return binding.translate_get_liked_request(request).await;
            }
            let library = match item {
                PlayingItemRef::LibraryTrack { .. } => {
                    item_library(&pool, ui_library.as_ref(), item).await?
                }
                PlayingItemRef::IndependentFile { .. } => require_library(ui_library)?,
            };
            let parsed_item = playing_item_from_request(item.clone(), Some(&library)).await?;

            let response = match parsed_item {
                PlayingItem::LibraryTrack { library_id, track }
                    if library_id == library.library_id
                        && let Ok(file_uuid) = Uuid::parse_str(&track) =>
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
