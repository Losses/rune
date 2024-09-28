use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::Result;
use sea_orm::QuerySelect;
use sea_orm::{DatabaseConnection, DbErr, EntityTrait, QueryOrder};

use crate::entities::{
    albums, artists, media_file_albums, media_file_artists, media_file_playlists, playlists,
};
use crate::get_cover_ids;
use crate::get_entity_to_cover_ids;

get_cover_ids!(get_album_cover_ids, albums, media_file_albums, AlbumId);
get_cover_ids!(get_artist_cover_ids, artists, media_file_artists, ArtistId);
get_cover_ids!(
    get_playlist_cover_ids,
    playlists,
    media_file_playlists,
    PlaylistId
);

pub async fn get_latest_albums_and_artists(
    db: &DatabaseConnection,
) -> Result<(Vec<albums::Model>, Vec<artists::Model>)> {
    let top_albums: Vec<albums::Model> = albums::Entity::find()
        .order_by_desc(albums::Column::Id)
        .limit(25)
        .all(db)
        .await?;

    let top_artists: Vec<artists::Model> = artists::Entity::find()
        .order_by_desc(artists::Column::Id)
        .limit(25)
        .all(db)
        .await?;

    Ok((top_albums, top_artists))
}
