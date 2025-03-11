use std::collections::HashSet;

use anyhow::Result;
use async_trait::async_trait;
use sea_orm::prelude::*;

use crate::actions::collection::{CollectionQuery, CollectionQueryType};
use crate::collection_query;
use crate::connection::MainDbConnection;
use crate::entities::{genres, media_file_genres};

use super::utils::CollectionDefinition;

impl CollectionDefinition for genres::Entity {
    fn group_column() -> Self::Column {
        genres::Column::Group
    }

    fn id_column() -> Self::Column {
        genres::Column::Id
    }
}

collection_query!(
    genres,
    CollectionQueryType::Genre,
    "lib::genre".to_owned(),
    media_file_genres,
    GenreId
);
