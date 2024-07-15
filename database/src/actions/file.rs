use sea_orm::entity::prelude::*;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::entities::media_files;

pub async fn get_files_by_ids(
    db: &DatabaseConnection,
    ids: &[i32],
) -> Result<Vec<media_files::Model>, Box<dyn std::error::Error>> {
    let files = media_files::Entity::find()
        .filter(media_files::Column::Id.is_in(ids.to_vec()))
        .all(db)
        .await?;
    Ok(files)
}
