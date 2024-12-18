use std::sync::Arc;

use anyhow::{Context, Result};
use rinf::DartSignal;

use database::{
    actions::stats::{get_liked, set_liked},
    connection::MainDbConnection,
};
use playback::player::PlayingItem;

use crate::{GetLikedRequest, GetLikedResponse, SetLikedRequest, SetLikedResponse};

pub async fn set_liked_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<SetLikedRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    if let Some(item) = request.item {
        let parsed_item: PlayingItem = item.clone().into();

        match parsed_item {
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
                    item: Some(item),
                    liked: request.liked,
                    success: true,
                }
                .send_signal_to_dart();
            }
            PlayingItem::IndependentFile(_) => {
                SetLikedResponse {
                    item: Some(item),
                    liked: false,
                    success: false,
                }
                .send_signal_to_dart();
            }
            PlayingItem::Unknown => {
                SetLikedResponse {
                    item: Some(item),
                    liked: false,
                    success: false,
                }
                .send_signal_to_dart();
            }
        }
    }

    Ok(())
}

pub async fn get_liked_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetLikedRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    if let Some(item) = request.item {
        let parsed_item: PlayingItem = item.clone().into();

        match parsed_item {
            PlayingItem::InLibrary(file_id) => {
                let liked = get_liked(&main_db, file_id)
                    .await
                    .with_context(|| format!("Failed to get liked: file_id={}", file_id))?;

                GetLikedResponse {
                    item: Some(item),
                    liked,
                }
                .send_signal_to_dart();
            }
            PlayingItem::IndependentFile(_) => {
                GetLikedResponse {
                    item: Some(item),
                    liked: false,
                }
                .send_signal_to_dart();
            }
            PlayingItem::Unknown => {
                GetLikedResponse {
                    item: Some(item),
                    liked: false,
                }
                .send_signal_to_dart();
            }
        }
    }

    Ok(())
}
