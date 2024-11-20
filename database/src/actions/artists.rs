use std::collections::HashSet;

use anyhow::Result;
use async_trait::async_trait;
use sea_orm::prelude::*;

use crate::actions::collection::{CollectionQuery, CollectionQueryType};
use crate::collection_query;
use crate::connection::MainDbConnection;
use crate::entities::{artists, media_file_artists};

use super::utils::CollectionDefinition;

impl CollectionDefinition for artists::Entity {
    fn group_column() -> Self::Column {
        artists::Column::Group
    }

    fn id_column() -> Self::Column {
        artists::Column::Id
    }
}

collection_query!(
    artists,
    CollectionQueryType::Artist,
    "lib::artist".to_owned(),
    media_file_artists,
    ArtistId
);
