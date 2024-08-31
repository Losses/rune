use std::collections::HashSet;

use sea_orm::prelude::*;

use crate::entities::{albums, media_file_albums};
use crate::{get_all_ids, get_by_id, get_by_ids, get_groups};

use super::utils::CountByFirstLetter;

impl CountByFirstLetter for albums::Entity {
    fn group_column() -> Self::Column {
        albums::Column::Group
    }

    fn id_column() -> Self::Column {
        albums::Column::Id
    }
}

get_groups!(get_albums_groups, albums, media_file_albums, AlbumId);
get_all_ids!(get_media_file_ids_of_album, media_file_albums, AlbumId);
get_by_ids!(get_albums_by_ids, albums);
get_by_id!(get_album_by_id, albums);
