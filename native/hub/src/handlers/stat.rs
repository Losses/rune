use anyhow::{Context, Result};

use ::database::actions::stats::{get_liked, set_liked};
use ::playback::player::PlayingItem;

use crate::{
    Session, Signal,
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
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let library = require_library(library)?;

        if let Some(item) = &request.item {
            let parsed_item: PlayingItem = item.clone().into();

            let response = match parsed_item {
                PlayingItem::InLibrary(file_id, _) => {
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
                PlayingItem::Online(_, _) => todo!(),
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
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let library = require_library(library)?;

        if let Some(item) = &request.item {
            let parsed_item: PlayingItem = item.clone().into();

            let response = match parsed_item {
                PlayingItem::InLibrary(file_id, _) => {
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
                PlayingItem::Online(_, _) => todo!(),
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
