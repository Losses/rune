use sea_orm::entity::prelude::*;
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "sync_download_jobs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub file_uuid: String,
    pub digest: String,
    pub size: i64,
    pub directory: String,
    pub file_name: String,
    pub state: String,
    pub attempts: i32,
    pub retry_at: Option<String>,
    pub retry_source: Option<String>,
    pub last_error: Option<String>,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
