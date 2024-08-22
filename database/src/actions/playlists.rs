use std::collections::HashSet;

use sea_orm::prelude::*;

use crate::entities::{media_file_playlists, playlists};
use crate::get_groups;

use super::utils::CountByFirstLetter;

impl CountByFirstLetter for playlists::Entity {
    fn group_column() -> Self::Column {
        playlists::Column::Group
    }

    fn id_column() -> Self::Column {
        playlists::Column::Id
    }
}

get_groups!(
    get_playlists_groups,
    playlists,
    media_file_playlists,
    PlaylistId
);
