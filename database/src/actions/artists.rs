use std::collections::HashSet;

use sea_orm::prelude::*;

use crate::entities::{artists, media_file_artists};
use crate::{get_all_ids, get_by_id, get_by_ids, get_first_n, get_groups};

use super::utils::CountByFirstLetter;

impl CountByFirstLetter for artists::Entity {
    fn group_column() -> Self::Column {
        artists::Column::Group
    }

    fn id_column() -> Self::Column {
        artists::Column::Id
    }
}

get_groups!(get_artists_groups, artists, media_file_artists, ArtistId);
get_all_ids!(get_media_file_ids_of_artist, media_file_artists, ArtistId);
get_by_ids!(get_artists_by_ids, artists);
get_by_id!(get_artist_by_id, artists);
get_first_n!(list_artists, artists);
