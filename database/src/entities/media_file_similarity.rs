//! `SeaORM` Entity, @generated by sea-orm-codegen 1.1.0

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "media_file_similarity")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub file_id1: i32,
    pub file_id2: i32,
    #[sea_orm(column_type = "Float")]
    pub similarity: f32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::media_files::Entity",
        from = "Column::FileId2",
        to = "super::media_files::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    MediaFiles2,
    #[sea_orm(
        belongs_to = "super::media_files::Entity",
        from = "Column::FileId1",
        to = "super::media_files::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    MediaFiles1,
}

impl ActiveModelBehavior for ActiveModel {}
