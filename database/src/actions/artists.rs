use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sea_orm::prelude::*;

use crate::actions::collection::{CollectionQuery, CollectionQueryType};
use crate::actions::utils::create_count_by_first_letter;
use crate::collection_query;
use crate::connection::MainDbConnection;
use crate::entities::{artists, media_file_artists, prelude};

use super::utils::CountByFirstLetter;

impl CountByFirstLetter for artists::Entity {
    fn group_column() -> Self::Column {
        artists::Column::Group
    }

    fn id_column() -> Self::Column {
        artists::Column::Id
    }
}

collection_query!(
    artists,
    prelude::Artists,
    CollectionQueryType::Artist,
    "lib::artist",
    media_file_artists,
    ArtistId,
    list_artists
);
