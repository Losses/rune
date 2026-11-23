use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "music_metadata_sync_edits")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub source_uuid: String,
    pub kind: String,
    pub owner_uuid: String,
    pub edition_uuid: Option<String>,
    pub target_uuid: Option<String>,
    pub label: Option<String>,
    pub generation: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
