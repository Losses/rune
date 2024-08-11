use std::collections::HashSet;

use sea_orm::prelude::*;

use crate::entities::{albums, media_cover_art, media_file_albums, media_files};
use crate::generate_get_groups_fn;

use super::utils::CountByFirstLetter;

impl CountByFirstLetter for albums::Entity {
    fn group_column() -> Self::Column {
        albums::Column::Group
    }

    fn id_column() -> Self::Column {
        albums::Column::Id
    }
}

generate_get_groups_fn!(get_albums_groups, albums, media_file_albums, AlbumId);
