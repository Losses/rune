use anyhow::{Context, Result};

use ::database::actions::{
    file::file_ids_by_uuid,
    stats::{get_liked, set_liked},
};
use ::playback::player::PlayingItem;

use crate::{
    Session, Signal,
    handlers::playback::{playing_item_from_request, reject_remote_independent_item},
    library_pool::LibraryHandle,
    messages::*,
    utils::{GlobalParams, ParamsExtractor, require_library},
};

impl ParamsExtractor for SetLikedRequest {
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

impl Signal for SetLikedRequest {
    type Params = Option<LibraryHandle>;
    type Response = SetLikedResponse;

    async fn handle(
        &self,
        library: Self::Params,
        session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let library = require_library(library)?;

        if let Some(item) = &request.item {
            reject_remote_independent_item(session.as_ref(), item)?;
            let parsed_item = playing_item_from_request(item.clone(), Some(&library)).await?;

            let response = match parsed_item {
                PlayingItem::InLibrary(identity) if identity.library_id == library.library_id => {
                    let file_id = file_ids_by_uuid(&library.main_db, &[identity.file_uuid])
                        .await?
                        .get(&identity.file_uuid)
                        .copied();
                    let Some(file_id) = file_id else {
                        return Ok(Some(SetLikedResponse {
                            item: item.clone(),
                            liked: false,
                            success: false,
                        }));
                    };
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

                    SetLikedResponse {
                        item: item.clone(),
                        liked: request.liked,
                        success: true,
                    }
                }
                PlayingItem::IndependentFile(_) => SetLikedResponse {
                    item: item.clone(),
                    liked: false,
                    success: false,
                },
                PlayingItem::InLibrary(_) | PlayingItem::Online(_, _) => SetLikedResponse {
                    item: item.clone(),
                    liked: false,
                    success: false,
                },
                PlayingItem::Unknown => SetLikedResponse {
                    item: item.clone(),
                    liked: false,
                    success: false,
                },
            };

            return Ok(Some(response));
        }

        Ok(None)
    }
}

impl ParamsExtractor for GetLikedRequest {
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

impl Signal for GetLikedRequest {
    type Params = Option<LibraryHandle>;
    type Response = GetLikedResponse;

    async fn handle(
        &self,
        library: Self::Params,
        session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let library = require_library(library)?;

        if let Some(item) = &request.item {
            reject_remote_independent_item(session.as_ref(), item)?;
            let parsed_item = playing_item_from_request(item.clone(), Some(&library)).await?;

            let response = match parsed_item {
                PlayingItem::InLibrary(identity) if identity.library_id == library.library_id => {
                    let Some(file_id) = file_ids_by_uuid(&library.main_db, &[identity.file_uuid])
                        .await?
                        .get(&identity.file_uuid)
                        .copied()
                    else {
                        return Ok(Some(GetLikedResponse {
                            item: item.clone(),
                            liked: false,
                        }));
                    };
                    let liked = get_liked(&*library.main_db, file_id)
                        .await
                        .with_context(|| format!("Failed to get liked: file_id={file_id}"))?;

                    GetLikedResponse {
                        item: item.clone(),
                        liked,
                    }
                }
                PlayingItem::IndependentFile(_) => GetLikedResponse {
                    item: item.clone(),
                    liked: false,
                },
                PlayingItem::InLibrary(_) | PlayingItem::Online(_, _) => GetLikedResponse {
                    item: item.clone(),
                    liked: false,
                },
                PlayingItem::Unknown => GetLikedResponse {
                    item: item.clone(),
                    liked: false,
                },
            };

            return Ok(Some(response));
        }

        Ok(None)
    }
}
