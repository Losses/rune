use std::path::Path;

use anyhow::{anyhow, Context, Result};
use http_body_util::{BodyExt, Empty, Full};
use hyper::{body::Bytes, Method, Request, StatusCode, Uri};
use rustls::ClientConfig;

use discovery::request::{create_https_client, send_http_request};
use hub::{messages::*, server::utils::device::SanitizedDeviceInfo};
use serde::Serialize;

use crate::{
    cli::{OperateMode, PlaybackMode},
    connection::WSConnection,
};

pub async fn fetch_mix_queries_by_mix_id(
    mix_id: i32,
    connection: &WSConnection,
) -> Result<Vec<MixQuery>> {
    let request = FetchMixQueriesRequest { mix_id };
    let response: FetchMixQueriesResponse = connection
        .request("FetchMixQueriesRequest", request)
        .await?;
    Ok(response.result)
}

pub async fn build_query(
    collection_type: CollectionType,
    id: i32,
    connection: &WSConnection,
) -> Result<Vec<(String, String)>> {
    if collection_type == CollectionType::Mix {
        let queries = fetch_mix_queries_by_mix_id(id, connection).await?;
        Ok(queries
            .into_iter()
            .map(|q| (q.operator, q.parameter))
            .collect())
    } else {
        build_collection_query(collection_type, id)
    }
}

pub fn build_collection_query(
    collection_type: CollectionType,
    id: i32,
) -> Result<Vec<(String, String)>> {
    if collection_type == CollectionType::Mix {
        return Err(anyhow!("Not Allow"));
    }
    let operator = match collection_type {
        CollectionType::Album => "lib::album",
        CollectionType::Artist => "lib::artist",
        CollectionType::Playlist => "lib::playlist",
        CollectionType::Track => "lib::track",
        _ => return Err(anyhow!("Invalid collection type")),
    };
    Ok(vec![(operator.to_string(), id.to_string())])
}

pub fn path_to_collection_type(path: &Path) -> Option<CollectionType> {
    match path.components().nth(1)?.as_os_str().to_str()? {
        "Albums" => Some(CollectionType::Album),
        "Artists" => Some(CollectionType::Artist),
        "Playlists" => Some(CollectionType::Playlist),
        "Mixes" => Some(CollectionType::Mix),
        "Tracks" => Some(CollectionType::Track),
        _ => None,
    }
}

pub async fn fetch_collection_group_summary(
    collection_type: CollectionType,
    connection: &WSConnection,
) -> Result<CollectionGroupSummaryResponse> {
    let request = FetchCollectionGroupSummaryRequest {
        collection_type: collection_type as i32,
    };

    connection
        .request("FetchCollectionGroupSummaryRequest", request)
        .await
}

pub async fn fetch_collection_groups(
    collection_type: CollectionType,
    group_titles: Vec<String>,
    connection: &WSConnection,
) -> Result<FetchCollectionGroupsResponse> {
    let request = FetchCollectionGroupsRequest {
        collection_type: collection_type as i32,
        bake_cover_arts: false,
        group_titles,
    };

    connection
        .request("FetchCollectionGroupsRequest", request)
        .await
}

pub async fn operate_playback_with_mix_query_request(
    queries: Vec<(String, String)>,
    playback_mode: PlaybackMode,
    instant_play: bool,
    operate_mode: OperateMode,
    connection: &WSConnection,
) -> Result<OperatePlaybackWithMixQueryResponse> {
    let request = OperatePlaybackWithMixQueryRequest {
        queries: queries
            .into_iter()
            .map(|(operator, parameter)| MixQuery {
                operator,
                parameter,
            })
            .collect(),
        playback_mode: playback_mode.into(),
        hint_position: -1,
        initial_playback_item: Default::default(),
        instantly_play: instant_play,
        operate_mode: operate_mode.into(),
        fallback_playing_items: vec![],
    };

    connection
        .request("OperatePlaybackWithMixQueryRequest", request)
        .await
}

pub async fn send_mix_query_request(
    queries: Vec<(String, String)>,
    connection: &WSConnection,
) -> Result<MixQueryResponse> {
    let request = MixQueryRequest {
        queries: queries
            .into_iter()
            .map(|(operator, parameter)| MixQuery {
                operator,
                parameter,
            })
            .collect(),
        cursor: 0,
        page_size: 9999,
        bake_cover_arts: false,
    };

    connection.request("MixQueryRequest", request).await
}

pub async fn send_play_request(connection: &WSConnection) -> Result<()> {
    let request = PlayRequest {};

    connection.request_simple("PlayRequest", request).await
}

pub async fn send_pause_request(connection: &WSConnection) -> Result<()> {
    let request = PauseRequest {};

    connection.request_simple("PauseRequest", request).await
}

pub async fn send_next_request(connection: &WSConnection) -> Result<()> {
    let request = NextRequest {};

    connection.request_simple("NextRequest", request).await
}

pub async fn send_previous_request(connection: &WSConnection) -> Result<()> {
    let request = PreviousRequest {};

    connection.request_simple("PreviousRequest", request).await
}

pub async fn send_set_playback_mode_request(
    playback_mode: PlaybackMode,
    connection: &WSConnection,
) -> Result<()> {
    let request = SetPlaybackModeRequest {
        mode: playback_mode.into(),
    };

    connection
        .request_simple("SetPlaybackModeRequest", request)
        .await
}

pub async fn fetch_device_info(host: &str, config: ClientConfig) -> Result<SanitizedDeviceInfo> {
    let uri = Uri::builder()
        .scheme("https")
        .authority(format!("{}:7863", host))
        .path_and_query("/device-info")
        .build()
        .context("Invalid URL format")?;

    let mut sender = create_https_client(host.to_owned(), 7863, config)
        .await
        .context("Failed to create HTTPS client")?;

    let req = Request::builder()
        .uri(uri)
        .header("Accept", "application/json")
        .body(Empty::<Bytes>::new())
        .context("Failed to build request")?;

    let res = send_http_request(&mut sender, req)
        .await
        .context("Failed to execute request")?;

    let body = res
        .into_body()
        .collect()
        .await
        .context("Failed to read response body")?
        .to_bytes();

    let device_info: SanitizedDeviceInfo =
        serde_json::from_slice(&body).context("Failed to parse device info")?;

    Ok(device_info)
}

#[derive(Debug, Serialize)]
struct RegisterRequest {
    public_key: String,
    fingerprint: String,
    alias: String,
    device_model: String,
    device_type: String,
}

pub async fn register_device(
    host: &str,
    config: ClientConfig,
    public_key: String,
    fingerprint: String,
    alias: String,
    device_model: String,
    device_type: String,
) -> Result<()> {
    let uri = Uri::builder()
        .scheme("https")
        .authority(format!("{}:7863", host))
        .path_and_query("/register")
        .build()
        .context("Invalid URL format")?;

    let mut sender = create_https_client(host.to_owned(), 7863, config)
        .await
        .context("Failed to create HTTPS client")?;

    let register_request = RegisterRequest {
        public_key,
        fingerprint,
        alias,
        device_model,
        device_type,
    };

    let json_body =
        serde_json::to_vec(&register_request).context("Failed to serialize register request")?;

    let req = Request::builder()
        .uri(uri)
        .method(Method::POST)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(json_body)))
        .context("Failed to build request")?;

    let response = send_http_request(&mut sender, req)
        .await
        .context("Failed to execute request")?;

    let status = response.status();
    let error_body = response.into_body().collect().await?.to_bytes();

    if status != StatusCode::CREATED {
        let error_message = String::from_utf8_lossy(&error_body);
        return Err(anyhow!(
            "Registration failed with status code {}: {}",
            status,
            error_message
        ));
    }
    Ok(())
}
