use std::path::Path;

use anyhow::{Result, anyhow};

use hub::messages::*;

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
        CollectionType::Genre => "lib::genre",
        _ => return Err(anyhow!("Invalid collection type")),
    };
    Ok(vec![(operator.to_string(), id.to_string())])
}

pub fn path_to_collection_type(path: &Path) -> Option<CollectionType> {
    let component = path.components().nth(1)?;
    let component_str = component.as_os_str().to_str()?;

    match component_str {
        "Albums" => Some(CollectionType::Album),
        "Artists" => Some(CollectionType::Artist),
        "Playlists" => Some(CollectionType::Playlist),
        "Mixes" => Some(CollectionType::Mix),
        "Tracks" => Some(CollectionType::Track),
        "Genres" => Some(CollectionType::Genre),
        _ => {
            log::warn!("path_to_collection_type: Unknown collection type '{}' from path {:?}", component_str, path);
            None
        }
    }
}

pub async fn fetch_collection_group_summary(
    collection_type: CollectionType,
    connection: &WSConnection,
) -> Result<CollectionGroupSummaryResponse> {
    let request = FetchCollectionGroupSummaryRequest { collection_type };

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
        collection_type,
        bake_cover_arts: false,
        group_titles,
    };

    connection
        .request("FetchCollectionGroupsRequest", request)
        .await
}

impl From<OperateMode> for PlaylistOperateMode {
    fn from(mode: OperateMode) -> Self {
        match mode {
            OperateMode::AppendToEnd => PlaylistOperateMode::AppendToEnd,
            OperateMode::PlayNext => PlaylistOperateMode::PlayNext,
            OperateMode::Replace => PlaylistOperateMode::Replace,
        }
    }
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
