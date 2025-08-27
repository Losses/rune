use std::fmt;
use std::str::FromStr;

use anyhow::{Result, anyhow};
use chrono::Utc;
use sea_orm::ActiveValue;
use sea_orm::QueryOrder;
use sea_orm::prelude::*;

use crate::entities::log;

use super::utils::DatabaseExecutor;

/// Enum representing log levels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
    Debug,
}

impl FromStr for LogLevel {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "info" => Ok(LogLevel::Info),
            "warning" => Ok(LogLevel::Warning),
            "error" => Ok(LogLevel::Error),
            "debug" => Ok(LogLevel::Debug),
            _ => Err(anyhow!("Invalid log level")),
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level_str = match self {
            LogLevel::Info => "INFO",
            LogLevel::Warning => "WARNING",
            LogLevel::Error => "ERROR",
            LogLevel::Debug => "DEBUG",
        };
        write!(f, "{level_str}")
    }
}

/// Insert a new log entry.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `level` - The log level.
/// * `domain` - The domain or category of the log.
/// * `detail` - The detailed log message.
///
/// # Returns
/// * `Result<Model>` - The inserted log model or an error.
pub async fn insert_log<E>(
    main_db: &E,
    level: LogLevel,
    domain: String,
    detail: String,
) -> Result<log::Model>
where
    E: DatabaseExecutor + sea_orm::ConnectionTrait,
{
    let new_log = log::ActiveModel {
        date: ActiveValue::Set(Utc::now()),
        level: ActiveValue::Set(level.to_string()),
        domain: ActiveValue::Set(domain),
        detail: ActiveValue::Set(detail),
        ..Default::default()
    };

    let inserted_log = new_log.insert(main_db).await?;
    Ok(inserted_log)
}

/// Clear all log entries.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
///
/// # Returns
/// * `Result<()>` - An empty result indicating success or an error.
pub async fn clear_logs(main_db: &DatabaseConnection) -> Result<()> {
    log::Entity::delete_many().exec(main_db).await?;
    Ok(())
}

/// Delete a log entry by ID.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `log_id` - The ID of the log to delete.
///
/// # Returns
/// * `Result<()>` - An empty result indicating success or an error.
pub async fn delete_log(main_db: &DatabaseConnection, log_id: i32) -> Result<()> {
    let log_to_delete = log::Entity::find_by_id(log_id).one(main_db).await?;

    if let Some(log) = log_to_delete {
        log.delete(main_db).await?;
    }

    Ok(())
}

/// List log entries with pagination.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `cursor` - The starting point for pagination (0-based index).
/// * `page_size` - The number of logs to retrieve per page.
///
/// # Returns
/// * `Result<Vec<log::Model>>` - A vector of log models or an error.
pub async fn list_log(
    main_db: &DatabaseConnection,
    cursor: u64,
    page_size: u64,
) -> Result<Vec<log::Model>> {
    let paginator = log::Entity::find()
        .order_by_desc(log::Column::Date)
        .paginate(main_db, page_size);

    let logs = paginator.fetch_page(cursor).await?;
    Ok(logs)
}
