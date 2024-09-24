use std::sync::Arc;

use anyhow::{Context, Result};
use rinf::DartSignal;

use database::{
    actions::stats::{get_liked, set_liked},
    connection::MainDbConnection,
};

use crate::{GetLikedRequest, GetLikedResponse, SetLikedRequest, SetLikedResponse};

pub async fn set_liked_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<SetLikedRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    set_liked(&main_db, request.file_id, request.liked)
        .await
        .with_context(|| {
            format!(
                "Failed to set liked: file_id={}, liked={}",
                request.file_id, request.liked
            )
        })?;
    SetLikedResponse {
        file_id: request.file_id,
        liked: request.liked,
        success: true,
    }
    .send_signal_to_dart();

    Ok(())
}

pub async fn get_liked_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetLikedRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let liked = get_liked(&main_db, request.file_id)
        .await
        .with_context(|| format!("Failed to get liked: file_id={}", request.file_id))?;

    GetLikedResponse {
        file_id: request.file_id,
        liked,
    }
    .send_signal_to_dart();

    Ok(())
}
