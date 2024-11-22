use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::Result;
use sea_orm::{DatabaseConnection, DbErr};

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
