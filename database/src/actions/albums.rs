use std::collections::HashSet;

use sea_orm::prelude::*;

use crate::entities::{albums, media_file_albums};
use crate::get_groups;

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
