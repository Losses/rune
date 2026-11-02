use sea_orm::entity::prelude::*;
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "media_file_likes")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub track_uuid: String,
    pub hlc_uuid: String,
    pub liked: bool,
    pub updated_at: String,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::media_files::Entity",
        from = "Column::TrackUuid",
        to = "super::media_files::Column::HlcUuid"
    )]
    MediaFiles,
}
impl Related<super::media_files::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MediaFiles.def()
    }
}
impl ActiveModelBehavior for ActiveModel {}
