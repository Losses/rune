use anyhow::Result;
use log::warn;
use sea_orm::entity::prelude::*;
use sea_orm::sea_query::Condition;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect, QueryTrait};

use crate::entities::{media_file_albums, media_file_artists, media_file_playlists, media_files};

fn parse_and_push<T>(parameter: &str, operator: &str, vec: &mut Vec<T>)
where
    T: std::str::FromStr,
    T::Err: std::fmt::Debug,
{
    match parameter.parse::<T>() {
        Ok(x) => vec.push(x),
        Err(_) => warn!(
            "Unable to parse the parameter of operator: {}({})",
            operator, parameter
        ),
    }
}

pub async fn query_mix_media_files(
    db: &DatabaseConnection,
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

    for (operator, parameter) in queries {
        match operator.as_str() {
            "lib::artist" => parse_and_push(&parameter, &operator, &mut artist_ids),
            "lib::album" => parse_and_push(&parameter, &operator, &mut album_ids),
            "lib::playlist" => parse_and_push(&parameter, &operator, &mut playlist_ids),
            "lib::track" => parse_and_push(&parameter, &operator, &mut track_ids),
            "directory.deep" => directories_deep.push(parameter.clone()),
            "directory.shallow" => directories_shallow.push(parameter.clone()),
            _ => warn!("Unknown operator: {}", operator),
        }
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

    // Apply the OR condition to the query
    query = query.filter(or_condition);

    // Use cursor pagination
    let mut cursor_by_id = query.cursor_by(media_files::Column::Id);

    // Retrieve the specified number of rows
    let media_files = cursor_by_id
        .after(cursor as i32)
        .first(page_size as u64)
        .all(db)
        .await?;

    Ok(media_files)
}
