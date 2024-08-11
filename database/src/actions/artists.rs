use std::collections::HashSet;

use sea_orm::prelude::*;

use crate::entities::media_cover_art;
use crate::entities::{artists, media_file_artists, media_files};
use crate::generate_get_groups_fn;

use super::utils::CountByFirstLetter;

impl CountByFirstLetter for artists::Entity {
    fn group_column() -> Self::Column {
        artists::Column::Group
    }

    fn id_column() -> Self::Column {
        artists::Column::Id
    }
}

generate_get_groups_fn!(get_artists_groups, artists, media_file_artists, ArtistId);
