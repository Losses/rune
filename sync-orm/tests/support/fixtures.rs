#![allow(dead_code)]

use anyhow::Result;
use sea_orm::{Database, DatabaseConnection};
use sync_orm::causal_schema;

pub async fn fresh_database() -> Result<DatabaseConnection> {
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    Ok(db)
}
