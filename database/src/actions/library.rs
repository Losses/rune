use sea_orm::QuerySelect;
use sea_orm::{DatabaseConnection, DbErr, EntityTrait, QueryOrder};
use std::collections::HashMap;
use std::collections::HashSet;

use crate::entities::media_file_artists;
use crate::entities::{albums, artists, media_file_albums, media_files};
use crate::get_cover_ids;
use crate::get_entity_to_cover_ids;

get_cover_ids!(
    get_album_cover_ids,
    albums::Model,
    media_file_albums::Entity,
    media_file_albums::Column
);
get_cover_ids!(
    get_artist_cover_ids,
    artists::Model,
    media_file_artists::Entity,
    media_file_artists::Column
);

pub async fn get_latest_albums_and_artists(
    db: &DatabaseConnection,
) -> Result<
    (
        Vec<(albums::Model, HashSet<i32>)>,
        Vec<(artists::Model, HashSet<i32>)>,
    ),
    DbErr,
> {
    // Step 1: Fetch the top 20 albums by ID
    let top_albums: Vec<albums::Model> = albums::Entity::find()
        .order_by_desc(albums::Column::Id)
        .limit(20)
        .all(db)
        .await?;

    // Step 2: Fetch the top 20 artists by ID
    let top_artists: Vec<artists::Model> = artists::Entity::find()
        .order_by_desc(artists::Column::Id)
        .limit(20)
        .all(db)
        .await?;

    // Step 3: Get cover IDs for top albums
    let album_cover_ids = get_album_cover_ids(db, &top_albums).await?;

    // Step 4: Get cover IDs for top artists
    let artist_cover_ids = get_artist_cover_ids(db, &top_artists).await?;

    // Step 5: Combine albums and their cover IDs
    let top_albums_with_cover_ids = top_albums
        .into_iter()
        .map(|album| {
            let cover_ids = album_cover_ids.get(&album.id).cloned().unwrap_or_default();
            (album, cover_ids)
        })
        .collect::<Vec<_>>();

    // Step 6: Combine artists and their cover IDs
    let top_artists_with_cover_ids = top_artists
        .into_iter()
        .map(|artist| {
            let cover_ids = artist_cover_ids
                .get(&artist.id)
                .cloned()
                .unwrap_or_default();
            (artist, cover_ids)
        })
        .collect::<Vec<_>>();

    Ok((top_albums_with_cover_ids, top_artists_with_cover_ids))
}
