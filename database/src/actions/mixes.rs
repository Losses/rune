use anyhow::Result;
use log::warn;
use sea_orm::entity::prelude::*;
use sea_orm::sea_query::Condition;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect, QueryTrait};

use crate::actions::analysis::get_percentile_analysis_result;
use crate::connection::RecommendationDbConnection;
use crate::entities::{
    media_file_albums, media_file_artists, media_file_playlists, media_file_stats, media_files,
};

use super::analysis::get_centralized_analysis_result;
use super::file::get_files_by_ids;
use super::recommendation::get_recommendation_by_parameter;

#[derive(Debug)]
enum QueryOperator {
    LibArtist(i32),
    LibAlbum(i32),
    LibPlaylist(i32),
    LibTrack(i32),
    LibDirectoryDeep(String),
    LibDirectoryShallow(String),
    SortLastModified(bool),
    SortDuration(bool),
    SortPlayedthrough(bool),
    SortSkipped(bool),
    FilterLiked(bool),
    PipeLimit(u64),
    PipeRecommend(i32),
    Unknown(String),
}

fn parse_parameter<T>(parameter: &str, operator: &str) -> Option<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Debug,
{
    match parameter.parse::<T>() {
        Ok(x) => Some(x),
        Err(_) => {
            warn!(
                "Unable to parse the parameter of operator: {}({})",
                operator, parameter
            );
            None
        }
    }
}

fn parse_query(query: &(String, String)) -> QueryOperator {
    let (operator, parameter) = query;
    match operator.as_str() {
        "lib::artist" => parse_parameter::<i32>(parameter, operator)
            .map(QueryOperator::LibArtist)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "lib::album" => parse_parameter::<i32>(parameter, operator)
            .map(QueryOperator::LibAlbum)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "lib::playlist" => parse_parameter::<i32>(parameter, operator)
            .map(QueryOperator::LibPlaylist)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "lib::track" => parse_parameter::<i32>(parameter, operator)
            .map(QueryOperator::LibTrack)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "lib::directory.deep" => QueryOperator::LibDirectoryDeep(parameter.clone()),
        "lib::directory.shallow" => QueryOperator::LibDirectoryShallow(parameter.clone()),
        "sort::last_modified" => parse_parameter::<bool>(parameter, operator)
            .map(QueryOperator::SortLastModified)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "sort::duration" => parse_parameter::<bool>(parameter, operator)
            .map(QueryOperator::SortDuration)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "sort::playedthrough" => parse_parameter::<bool>(parameter, operator)
            .map(QueryOperator::SortPlayedthrough)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "sort::skipped" => parse_parameter::<bool>(parameter, operator)
            .map(QueryOperator::SortSkipped)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "filter::liked" => parse_parameter::<bool>(parameter, operator)
            .map(QueryOperator::FilterLiked)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "pipe::limit" => parse_parameter::<u64>(parameter, operator)
            .map(QueryOperator::PipeLimit)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "pipe::recommend" => parse_parameter::<i32>(parameter, operator)
            .map(QueryOperator::PipeRecommend)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        _ => QueryOperator::Unknown(operator.clone()),
    }
}

fn apply_join_filter(
    query: Select<media_files::Entity>,
    filter_liked: Option<bool>,
    sort_playedthrough_asc: Option<bool>,
    sort_skipped_asc: Option<bool>,
) -> Select<media_files::Entity> {
    if filter_liked.is_some() || sort_playedthrough_asc.is_some() || sort_skipped_asc.is_some() {
        query.join(
            sea_orm::JoinType::LeftJoin,
            media_file_stats::Entity::belongs_to(media_files::Entity)
                .from(media_file_stats::Column::MediaFileId)
                .to(media_files::Column::Id)
                .into(),
        )
    } else {
        query
    }
}

// Macro to handle sorting
macro_rules! apply_sorting_macro {
    ($query:expr, $column:expr, $sort_option:expr) => {
        if let Some(asc) = $sort_option {
            $query = $query.order_by($column, if asc { Order::Asc } else { Order::Desc });
        }
    };
}

// Macro to handle cursor sorting
macro_rules! apply_cursor_sorting_macro {
    ($query:expr, $cursor_by:expr, $column:expr, $sort_option:expr, $final_asc:expr) => {
        if let Some(asc) = $sort_option {
            $cursor_by = $query.clone().cursor_by($column);
            $final_asc = asc;
        }
    };
}

// Macro to handle subquery filters
macro_rules! add_subquery_filter {
    ($or_condition:expr, $ids:expr, $entity:ty, $column:expr) => {
        if !$ids.is_empty() {
            let subquery = <$entity>::find()
                .select_only()
                .filter($column.is_in($ids))
                .column(media_file_artists::Column::MediaFileId)
                .into_query();

            $or_condition =
                $or_condition.add(Expr::col(media_files::Column::Id).in_subquery(subquery));
        }
    };
}

pub async fn query_mix_media_files(
    main_db: &DatabaseConnection,
    recommend_db: &RecommendationDbConnection,
    queries: Vec<(String, String)>,
    cursor: usize,
    page_size: usize,
) -> Result<Vec<media_files::Model>> {
    let mut artist_ids: Vec<i32> = vec![];
    let mut album_ids: Vec<i32> = vec![];
    let mut playlist_ids: Vec<i32> = vec![];
    let mut track_ids: Vec<i32> = vec![];
    let mut directories_deep: Vec<String> = vec![];
    let mut directories_shallow: Vec<String> = vec![];

    let mut sort_last_modified_asc: Option<bool> = None;
    let mut sort_duration_asc: Option<bool> = None;
    let mut sort_playedthrough_asc: Option<bool> = None;
    let mut sort_skipped_asc: Option<bool> = None;

    let mut filter_liked: Option<bool> = None;
    let mut pipe_limit: Option<u64> = None;
    let mut pipe_recommend: Option<i32> = None;

    for query in queries {
        match parse_query(&query) {
            QueryOperator::LibArtist(id) => artist_ids.push(id),
            QueryOperator::LibAlbum(id) => album_ids.push(id),
            QueryOperator::LibPlaylist(id) => playlist_ids.push(id),
            QueryOperator::LibTrack(id) => track_ids.push(id),
            QueryOperator::LibDirectoryDeep(dir) => directories_deep.push(dir),
            QueryOperator::LibDirectoryShallow(dir) => directories_shallow.push(dir),
            QueryOperator::SortLastModified(asc) => sort_last_modified_asc = Some(asc),
            QueryOperator::SortDuration(asc) => sort_duration_asc = Some(asc),
            QueryOperator::SortPlayedthrough(asc) => sort_playedthrough_asc = Some(asc),
            QueryOperator::SortSkipped(asc) => sort_skipped_asc = Some(asc),
            QueryOperator::FilterLiked(liked) => filter_liked = Some(liked),
            QueryOperator::PipeLimit(limit) => pipe_limit = Some(limit),
            QueryOperator::PipeRecommend(recommend) => pipe_recommend = Some(recommend),
            QueryOperator::Unknown(op) => warn!("Unknown operator: {}", op),
        }
    }

    if pipe_recommend.is_some() && cursor > 0 {
        return Ok([].to_vec());
    }

    // Base query for media_files
    let mut query = media_files::Entity::find();

    // Create an OR condition to hold all the subconditions
    let mut or_condition = Condition::any();

    // Filter by artist_ids if provided
    add_subquery_filter!(
        or_condition,
        artist_ids,
        media_file_artists::Entity,
        media_file_artists::Column::ArtistId
    );

    // Filter by album_ids if provided
    add_subquery_filter!(
        or_condition,
        album_ids,
        media_file_albums::Entity,
        media_file_albums::Column::AlbumId
    );

    // Filter by playlist_ids if provided
    add_subquery_filter!(
        or_condition,
        playlist_ids,
        media_file_playlists::Entity,
        media_file_playlists::Column::PlaylistId
    );

    // Filter by track_ids if provided
    if !track_ids.is_empty() {
        or_condition = or_condition.add(Expr::col(media_files::Column::Id).is_in(track_ids));
    }

    // Filter by directories if provided
    if !directories_deep.is_empty() {
        let mut dir_conditions = Condition::any();
        for dir in directories_deep {
            let dir = dir.strip_prefix('/').unwrap_or(&dir);

            dir_conditions = dir_conditions.add(
                Expr::col(media_files::Column::Directory)
                    .eq(dir)
                    .or(Expr::col(media_files::Column::Directory).like(format!("{}/%", dir))),
            );
        }
        or_condition = or_condition.add(dir_conditions);
    }

    // Filter by directories if provided
    if !directories_shallow.is_empty() {
        let mut dir_conditions = Condition::any();
        for dir in directories_shallow {
            let dir = dir.strip_prefix('/').unwrap_or(&dir);

            dir_conditions = dir_conditions.add(Expr::col(media_files::Column::Directory).eq(dir));
        }
        or_condition = or_condition.add(dir_conditions);
    }

    if let Some(liked) = filter_liked {
        query = query.filter(
            Condition::all()
                .add(or_condition)
                .add(media_file_stats::Column::Liked.eq(liked)),
        );
    } else {
        query = query.filter(or_condition);
    }

    // Join with media_file_stats table for sorting by playedthrough and skipped, and filtering by liked
    query = apply_join_filter(
        query,
        filter_liked,
        sort_playedthrough_asc,
        sort_skipped_asc,
    );

    if let Some(recommend_group) = pipe_recommend {
        apply_sorting_macro!(
            query,
            media_files::Column::LastModified,
            sort_last_modified_asc
        );
        apply_sorting_macro!(query, media_files::Column::Duration, sort_duration_asc);

        if let Some(asc) = sort_playedthrough_asc {
            query = query.order_by(
                media_file_stats::Column::PlayedThrough,
                if asc { Order::Asc } else { Order::Desc },
            );
        }

        if let Some(asc) = sort_skipped_asc {
            query = query.order_by(
                media_file_stats::Column::Skipped,
                if asc { Order::Asc } else { Order::Desc },
            );
        }

        if let Some(query_limit) = pipe_limit {
            query = query.limit(query_limit);
        }

        let file_ids = query
            .select_only()
            .column(media_files::Column::Id)
            .distinct()
            .into_tuple::<i32>()
            .all(main_db)
            .await?;

        let virtual_point: [f32; 61] = if recommend_group >= 0 {
            get_percentile_analysis_result(
                main_db,
                1.0 / (9 + 2) as f64 * (recommend_group + 1) as f64,
            )
            .await?
        } else {
            get_centralized_analysis_result(main_db, file_ids)
                .await?
                .into()
        };

        let recommend_n = if let Some(query_limit) = pipe_limit {
            query_limit
        } else {
            30
        };

        let file_ids =
            get_recommendation_by_parameter(recommend_db, virtual_point, recommend_n as usize)?
                .into_iter()
                .map(|x| x.0 as i32)
                .collect::<Vec<i32>>();

        return Ok(get_files_by_ids(main_db, &file_ids).await?);
    }

    // Use cursor pagination
    let mut cursor_by = query.clone().cursor_by(media_files::Column::Id);
    let mut final_asc = true;

    apply_cursor_sorting_macro!(
        query,
        cursor_by,
        media_files::Column::LastModified,
        sort_last_modified_asc,
        final_asc
    );
    apply_cursor_sorting_macro!(
        query,
        cursor_by,
        media_files::Column::Duration,
        sort_duration_asc,
        final_asc
    );
    apply_cursor_sorting_macro!(
        query,
        cursor_by,
        media_file_stats::Column::PlayedThrough,
        sort_playedthrough_asc,
        final_asc
    );
    apply_cursor_sorting_macro!(
        query,
        cursor_by,
        media_file_stats::Column::Skipped,
        sort_skipped_asc,
        final_asc
    );

    if let Some(limit) = pipe_limit {
        if cursor as u64 >= limit {
            return Ok(vec![]);
        }
    }

    let final_page_size = if let Some(limit) = pipe_limit {
        (limit - cursor as u64).min(page_size as u64)
    } else {
        page_size as u64
    };

    let media_files = (if final_asc {
        cursor_by.after(cursor as i32).first(final_page_size)
    } else {
        cursor_by
            .desc()
            .before(cursor as i32)
            .first(final_page_size)
    })
    .all(main_db)
    .await?;

    Ok(media_files)
}
