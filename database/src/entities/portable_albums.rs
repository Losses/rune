//! Synchronized album identities and their original numeric IDs.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

use super::track_album_sources::Entity as MediaFileAlbumsEntity;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "music_metadata_albums")]
pub struct Model {
    #[sea_orm(primary_key, column_name = "legacy_id")]
    pub id: i32,
    #[sea_orm(column_name = "portable_title", column_type = "Text", unique)]
    pub name: String,
    #[sea_orm(column_name = "portable_group", column_type = "Text")]
    pub group: String,
    #[sea_orm(column_name = "id", column_type = "Text")]
    pub source_uuid: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "MediaFileAlbumsEntity")]
    MediaFileAlbums,
}

impl Related<super::track_album_sources::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MediaFileAlbums.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
