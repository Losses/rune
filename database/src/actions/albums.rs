use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sea_orm::prelude::*;

use crate::actions::collection::CollectionQuery;
use crate::actions::collection::CollectionQueryType;
use crate::actions::utils::create_count_by_first_letter;
use crate::collection_query;
use crate::connection::MainDbConnection;
use crate::entities::{albums, media_file_albums, prelude};

use super::utils::CountByFirstLetter;

impl CountByFirstLetter for albums::Entity {
    fn group_column() -> Self::Column {
        albums::Column::Group
    }

    fn id_column() -> Self::Column {
        albums::Column::Id
    }
}

collection_query!(
    albums,
    prelude::Albums,
    CollectionQueryType::Album,
    "lib::album",
    media_file_albums,
    AlbumId,
    list_albums
);
