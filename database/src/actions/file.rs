use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use metadata::describe::FileDescription;
use rust_decimal::prelude::ToPrimitive;
use sea_orm::entity::prelude::*;
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, Order, QueryFilter, QueryOrder, QuerySelect,
    QueryTrait,
};

use migration::{Func, SimpleExpr};

use crate::entities::media_files;
use crate::{get_by_id, get_by_ids, get_first_n};

get_by_ids!(get_files_by_ids, media_files);
get_by_id!(get_file_by_id, media_files);
get_first_n!(list_files, media_files);

pub async fn get_ordered_files_by_ids(
    main_db: &DatabaseConnection,
    file_ids: &[i32],
) -> Result<Vec<media_files::Model>> {
    let files = get_files_by_ids(main_db, file_ids).await?;

    // Create a HashMap to store file_id -> file mapping
    let file_map: HashMap<i32, media_files::Model> =
        files.into_iter().map(|file| (file.id, file)).collect();

    // Reorder files based on the order of recommendation_ids
    let ordered_files: Vec<media_files::Model> = file_ids
        .iter()
        .filter_map(|id| file_map.get(id).cloned())
        .collect();

    Ok(ordered_files)
}

pub async fn get_random_files(
    db: &DatabaseConnection,
    n: usize,
) -> Result<Vec<media_files::Model>, sea_orm::DbErr> {
    let mut query: sea_orm::sea_query::SelectStatement =
        media_files::Entity::find().as_query().to_owned();
    let select = query
        .order_by_expr(SimpleExpr::FunctionCall(Func::random()), Order::Asc)
        .limit(n as u64);
    let statement = db.get_database_backend().build(select);

    let files = media_files::Model::find_by_statement(statement)
        .all(db)
        .await?;

    Ok(files)
}

pub async fn get_file_by_path(
    db: &DatabaseConnection,
    relative_path: &Path,
) -> Result<Option<media_files::Model>, sea_orm::DbErr> {
    let directory = relative_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_str()
        .unwrap_or("")
        .to_string();
    let file_name = relative_path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new(""))
        .to_str()
        .unwrap_or("")
        .to_string();

    let file = media_files::Entity::find()
        .filter(media_files::Column::Directory.eq(directory))
        .filter(media_files::Column::FileName.eq(file_name))
        .one(db)
        .await?;

    Ok(file)
}

pub async fn get_file_id_from_path(
    db: &DatabaseConnection,
    root_path: &Path,
    file_path: &Path,
) -> Result<i32, String> {
    // Check if the file exists as an absolute path
    let absolute_path = if file_path.is_absolute() {
        file_path.to_path_buf()
    } else {
        root_path.join(file_path)
    };

    if !absolute_path.exists() {
        return Err(format!("File does not exist: {absolute_path:?}"));
    }

    let relative_path = match absolute_path.strip_prefix(root_path) {
        Ok(path) => path,
        Err(_) => {
            return Err(format!(
                "File is not within the specified library path: {absolute_path:?}"
            ));
        }
    };

    let file_info = match get_file_by_path(db, relative_path).await {
        Ok(Some(file_info)) => file_info,
        Ok(_none) => {
            return Err(format!("File is not in the database: {relative_path:?}"));
        }
        Err(e) => {
            return Err(format!("Failed to query the database: {e}"));
        }
    };

    Ok(file_info.id)
}

pub async fn get_media_files(
    db: &DatabaseConnection,
    cursor: usize,
    page_size: usize,
) -> Result<Vec<media_files::Model>, sea_orm::DbErr> {
    media_files::Entity::find()
        .cursor_by(media_files::Column::Id)
        .after(cursor as i32)
        .first(page_size as u64)
        .all(db)
        .await
}

pub async fn get_reverse_listed_media_files(
    main_db: &DatabaseConnection,
    cursor: usize,
    page_size: usize,
) -> Result<Vec<media_files::Model>, sea_orm::DbErr> {
    media_files::Entity::find()
        .order_by(media_files::Column::Id, Order::Desc)
        .offset(cursor as u64)
        .limit(page_size as u64)
        .all(main_db)
        .await
}

pub async fn get_file_ids_by_descriptions(
    db: &DatabaseConnection,
    descriptions: &[Option<FileDescription>],
) -> Result<Vec<i32>, DbErr> {
    if descriptions.is_empty() {
        return Ok(vec![]);
    }

    let mut conditions = sea_orm::Condition::any();

    for description in descriptions {
        match description {
            Some(x) => {
                conditions = conditions.add(
                    media_files::Column::Directory
                        .eq(x.directory.clone())
                        .and(media_files::Column::FileName.eq(x.file_name.clone())),
                );
            }
            _none => {}
        }
    }

    let file_entries = media_files::Entity::find()
        .filter(conditions)
        .all(db)
        .await?;

    let file_ids = file_entries.into_iter().map(|entry| entry.id).collect();

    Ok(file_ids)
}

pub async fn get_duration_by_file_id(
    db: &DatabaseConnection,
    file_id: i32,
) -> Result<f64, sea_orm::DbErr> {
    let analysis_entry: Option<media_files::Model> = media_files::Entity::find()
        .filter(media_files::Column::Id.eq(file_id))
        .one(db)
        .await?;

    if let Some(entry) = analysis_entry {
        Ok(entry.duration.to_f64().unwrap())
    } else {
        Err(sea_orm::DbErr::RecordNotFound(
            "Analysis record not found".to_string(),
        ))
    }
}
