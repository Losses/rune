//! Generic SeaORM persistence for caller-defined synchronization models.
pub use causal_store::CausalReplica as OrmReplica;

pub mod causal_candidates;
pub mod causal_capabilities;
pub mod causal_compaction;
pub mod causal_gc;
pub mod causal_leases;
pub mod causal_manifests;
pub mod causal_observed;
pub mod causal_progress;
pub mod causal_schema;
pub mod causal_snapshot;
pub mod causal_source;
pub mod causal_store;
pub mod causal_writer;
pub mod maintenance;

pub mod registry;
pub mod row_policy;

pub mod derived_effect;
pub mod owned_list;

/// Recognize SQLite write-lock contention from a SeaORM/SQLx error chain.
/// Driver codes 5 and 6 are SQLITE_BUSY and SQLITE_LOCKED.
pub fn is_storage_busy(error: &anyhow::Error) -> bool {
    use sea_orm::{DbErr, RuntimeErr, sqlx::Error as SqlxError};
    use std::ops::Deref;

    error.chain().filter_map(|cause| cause.downcast_ref::<DbErr>()).any(|db| {
        let sqlx_error = match db {
            DbErr::Exec(RuntimeErr::SqlxError(error))
            | DbErr::Query(RuntimeErr::SqlxError(error)) => Some(error.deref()),
            _ => None,
        };
        matches!(sqlx_error, Some(SqlxError::Database(driver_error))
            if driver_error.code().and_then(|code| code.parse::<i32>().ok()).is_some_and(|code| matches!(code, 5 | 6)))
    })
}
