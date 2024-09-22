use std::sync::Arc;

use anyhow::{Context, Result};
use database::{
    actions::stats::{get_liked, increase_played_through, increase_skipped, set_liked},
    connection::MainDbConnection,
};
use log::{debug, error};
use rinf::DartSignal;

use crate::{
    GetLikedRequest, GetLikedResponse, IncreasePlayedThroughRequest, IncreasePlayedThroughResponse,
    IncreaseSkippedRequest, IncreaseSkippedResponse, SetLikedRequest, SetLikedResponse,
};

pub async fn set_liked_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<SetLikedRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    debug!(
        "Setting liked: file_id={}, liked={}",
        request.file_id, request.liked
    );

    match set_liked(&main_db, request.file_id, request.liked)
        .await
        .with_context(|| "Failed to set liked")
    {
        Ok(_) => {
            SetLikedResponse {
                file_id: request.file_id,
                liked: request.liked,
                success: true,
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            SetLikedResponse {
                file_id: request.file_id,
                liked: request.liked,
                success: false,
            }
            .send_signal_to_dart();
            error!("{:?}", e);
        }
    };

    Ok(())
}
pub async fn get_liked_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetLikedRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    debug!("Setting liked: file_id={}", request.file_id);

    match get_liked(&main_db, request.file_id)
        .await
        .with_context(|| "Failed to get liked")
    {
        Ok(liked) => {
            GetLikedResponse {
                file_id: request.file_id,
                liked,
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            error!("{:?}", e);
        }
    };

    Ok(())
}

pub async fn increase_skipped_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<IncreaseSkippedRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    debug!("Increasing skipped: file_id={}", request.file_id);

    match increase_skipped(&main_db, request.file_id)
        .await
        .with_context(|| "Failed to increase skipped")
    {
        Ok(_) => {
            IncreaseSkippedResponse {
                file_id: request.file_id,
                success: true,
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            IncreaseSkippedResponse {
                file_id: request.file_id,
                success: false,
            }
            .send_signal_to_dart();
            error!("{:?}", e);
        }
    };

    Ok(())
}

pub async fn increase_played_through_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<IncreasePlayedThroughRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    debug!("Increasing played through: file_id={}", request.file_id);

    match increase_played_through(&main_db, request.file_id)
        .await
        .with_context(|| "Failed to increase skipped")
    {
        Ok(_) => {
            IncreasePlayedThroughResponse {
                file_id: request.file_id,
                success: true,
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            IncreasePlayedThroughResponse {
                file_id: request.file_id,
                success: false,
            }
            .send_signal_to_dart();
            error!("{:?}", e);
        }
    };

    Ok(())
}
