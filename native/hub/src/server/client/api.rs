use std::path::Path;

use anyhow::{anyhow, Result};

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
        page_size: 100,
        bake_cover_arts: false,
    };

    connection.request("MixQueryRequest", request).await
}
