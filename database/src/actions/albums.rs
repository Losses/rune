use std::collections::HashSet;

use anyhow::Result;
use async_trait::async_trait;
use sea_orm::prelude::*;

use crate::actions::collection::CollectionQuery;
use crate::collection_query;
use crate::connection::MainDbConnection;
use crate::entities::{albums, media_file_albums};

use super::collection::CollectionQueryType;
use super::utils::CollectionDefinition;

impl CollectionDefinition for albums::Entity {
    fn group_column() -> Self::Column {
        albums::Column::Group
    }

    fn id_column() -> Self::Column {
        albums::Column::Id
    }
}

collection_query!(
    albums,
    CollectionQueryType::Album,
    "lib::album".to_owned(),
    media_file_albums,
    AlbumId
);
