use std::sync::Arc;

use anyhow::{Context, Result};

use ::database::{
    actions::stats::{get_liked, set_liked},
    connection::MainDbConnection,
};
use ::playback::player::PlayingItem;

use crate::{
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
    Signal,
};

impl ParamsExtractor for SetLikedRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for SetLikedRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = SetLikedResponse;

    async fn handle(
        &self,
        (main_db,): Self::Params,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        if let Some(item) = &request.item {
            let parsed_item: PlayingItem = item.clone().into();

            let response = match parsed_item {
                PlayingItem::InLibrary(file_id) => {
                    set_liked(&main_db, file_id, request.liked)
                        .await
                        .with_context(|| {
                            format!(
                                "Failed to set liked: file_id={}, liked={}",
                                file_id, request.liked
                            )
                        })?;

                    SetLikedResponse {
                        item: Some(item.clone()),
                        liked: request.liked,
                        success: true,
                    }
                }
                PlayingItem::IndependentFile(_) => SetLikedResponse {
                    item: Some(item.clone()),
                    liked: false,
                    success: false,
                },
                PlayingItem::Unknown => SetLikedResponse {
                    item: Some(item.clone()),
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
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for GetLikedRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = GetLikedResponse;

    async fn handle(
        &self,
        (main_db,): Self::Params,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        if let Some(item) = &request.item {
            let parsed_item: PlayingItem = item.clone().into();

            let response = match parsed_item {
                PlayingItem::InLibrary(file_id) => {
                    let liked = get_liked(&main_db, file_id)
                        .await
                        .with_context(|| format!("Failed to get liked: file_id={}", file_id))?;

                    GetLikedResponse {
                        item: Some(item.clone()),
                        liked,
                    }
                }
                PlayingItem::IndependentFile(_) => GetLikedResponse {
                    item: Some(item.clone()),
                    liked: false,
                },
                PlayingItem::Unknown => GetLikedResponse {
                    item: Some(item.clone()),
                    liked: false,
                },
            };

            return Ok(Some(response));
        }

        Ok(None)
    }
}
