use anyhow::Result;
use log::warn;
use sea_orm::entity::prelude::*;
use sea_orm::sea_query::Condition;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect, QueryTrait};

use crate::connection::RecommendationDbConnection;
use crate::entities::{
    media_file_albums, media_file_artists, media_file_playlists, media_file_stats, media_files,
};

use super::analysis::get_centralized_analysis_result;
use super::file::get_files_by_ids;
use super::recommendation::get_recommendation_by_parameter;

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

fn apply_sorting(
    query: Select<media_files::Entity>,
    column: media_files::Column,
    asc: Option<bool>,
) -> Select<media_files::Entity> {
    if let Some(asc) = asc {
        println!("Applying sorting: {:#?}", column);
        query.order_by(column, if asc { Order::Asc } else { Order::Desc })
    } else {
        query
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

    for (operator, parameter) in queries {
        match operator.as_str() {
            "lib::artist" => {
                if let Some(id) = parse_parameter::<i32>(&parameter, &operator) {
                    artist_ids.push(id)
                }
            }
            "lib::album" => {
                if let Some(id) = parse_parameter::<i32>(&parameter, &operator) {
                    album_ids.push(id)
                }
            }
            "lib::playlist" => {
                if let Some(id) = parse_parameter::<i32>(&parameter, &operator) {
                    playlist_ids.push(id)
                }
            }
            "lib::track" => {
                if let Some(id) = parse_parameter::<i32>(&parameter, &operator) {
                    track_ids.push(id)
                }
            }
            "directory.deep" => directories_deep.push(parameter.clone()),
            "directory.shallow" => directories_shallow.push(parameter.clone()),
            "sort::last_modified" => {
                sort_last_modified_asc = parse_parameter::<bool>(&parameter, &operator)
            }
            "sort::duration" => sort_duration_asc = parse_parameter::<bool>(&parameter, &operator),
            "sort::playedthrough" => {
                sort_playedthrough_asc = parse_parameter::<bool>(&parameter, &operator)
            }
            "sort::skipped" => sort_skipped_asc = parse_parameter::<bool>(&parameter, &operator),
            "filter::liked" => filter_liked = parse_parameter::<bool>(&parameter, &operator),
            "pipe::limit" => pipe_limit = parse_parameter::<u64>(&parameter, &operator),
            "pipe::recommend" => pipe_recommend = parse_parameter::<i32>(&parameter, &operator),
            _ => warn!("Unknown operator: {}", operator),
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
    if !artist_ids.is_empty() {
        let artist_subquery = media_file_artists::Entity::find()
            .select_only()
            .filter(media_file_artists::Column::ArtistId.is_in(artist_ids))
            .column(media_file_artists::Column::MediaFileId)
            .into_query();

        or_condition =
            or_condition.add(Expr::col(media_files::Column::Id).in_subquery(artist_subquery));
    }

    // Filter by album_ids if provided
    if !album_ids.is_empty() {
        let album_subquery = media_file_albums::Entity::find()
            .select_only()
            .filter(media_file_albums::Column::AlbumId.is_in(album_ids))
            .column(media_file_albums::Column::MediaFileId)
            .into_query();

        or_condition =
            or_condition.add(Expr::col(media_files::Column::Id).in_subquery(album_subquery));
    }

    // Filter by playlist_ids if provided
    if !playlist_ids.is_empty() {
        let playlist_subquery = media_file_playlists::Entity::find()
            .select_only()
            .filter(media_file_playlists::Column::PlaylistId.is_in(playlist_ids))
            .column(media_file_playlists::Column::MediaFileId)
            .into_query();

        or_condition =
            or_condition.add(Expr::col(media_files::Column::Id).in_subquery(playlist_subquery));
    }

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

    if let Some(recommend_n) = pipe_recommend {
        query = apply_sorting(
            query,
            media_files::Column::LastModified,
            sort_last_modified_asc,
        );
        query = apply_sorting(query, media_files::Column::Duration, sort_duration_asc);

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

        let virtual_point = get_centralized_analysis_result(main_db, file_ids).await?;

        let virtual_point: [f32; 61] = virtual_point.into();

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

    if let Some(asc) = sort_last_modified_asc {
        cursor_by = query.clone().cursor_by(media_files::Column::LastModified);

        final_asc = asc;
    }

    if let Some(asc) = sort_duration_asc {
        cursor_by = query.clone().cursor_by(media_files::Column::Duration);

        final_asc = asc;
    }

    if let Some(asc) = sort_playedthrough_asc {
        cursor_by = query
            .clone()
            .cursor_by(media_file_stats::Column::PlayedThrough);

        final_asc = asc;
    }

    if let Some(asc) = sort_skipped_asc {
        cursor_by = query.clone().cursor_by(media_file_stats::Column::Skipped);

        final_asc = asc;
    }

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
        cursor_by.desc().before(cursor as i32).first(final_page_size)
    })
    .all(main_db)
    .await?;

    Ok(media_files)
}
