//! The portable album assignment supplied by the sync protocol for a track.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

use super::media_files::{Column as MediaFilesColumn, Entity as MediaFilesEntity};
use super::portable_albums::{Column as AlbumsColumn, Entity as AlbumsEntity};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "music_metadata_track_album_sources")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique)]
    pub track_uuid: String,
    pub track_number: Option<i32>,
    pub album_uuid: String,
    #[sea_orm(column_type = "Text")]
    pub source_uuid: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "AlbumsEntity",
        from = "Column::AlbumUuid",
        to = "AlbumsColumn::SourceUuid"
    )]
    Albums,
    #[sea_orm(
        belongs_to = "MediaFilesEntity",
        from = "Column::TrackUuid",
        to = "MediaFilesColumn::SourceUuid"
    )]
    MediaFiles,
}

impl Related<super::portable_albums::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Albums.def()
    }
}

impl Related<super::media_files::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MediaFiles.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
