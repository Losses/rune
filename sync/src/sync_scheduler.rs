use std::{
    fmt::{self, Debug},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use anyhow::Result;
#[cfg(not(test))]
use log::{error, info};
use serde::{Deserialize, Serialize};

#[cfg(test)]
use std::{println as info, println as error};

use crate::core::{self, PrimaryKeyFromStr, RemoteDataSource, SyncContext, SyncTableMetadata};
use crate::foreign_key::{
    ActiveModelWithForeignKeyOps, ForeignKeyResolver, ModelWithForeignKeyOps,
};
use crate::hlc::{HLCModel, HLCRecord};

use sea_orm::{ActiveModelBehavior, EntityTrait, IntoActiveModel, PrimaryKeyTrait, Value};
use std::hash::Hash;

type TableSyncTaskFn<R> = Box<
    dyn for<'a> Fn(
            &'a SyncContext<'a, R>,
            SyncTableMetadata,
        ) -> Pin<Box<dyn Future<Output = Result<SyncTableMetadata>> + Send + 'a>>
        + Send
        + Sync,
>;

pub struct TableSyncJob<R>
where
    R: RemoteDataSource + Send + Sync + Debug + 'static,
{
    pub table_name: String,
    pub initial_metadata: SyncTableMetadata,
    task: TableSyncTaskFn<R>,
}

impl<R: RemoteDataSource + Send + Sync + Debug + 'static> Debug for TableSyncJob<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableSyncJob")
            .field("table_name", &self.table_name)
            .field("initial_metadata", &self.initial_metadata)
            .field("task", &"<async task closure>")
            .finish()
    }
}

impl<R: RemoteDataSource + Send + Sync + Debug + 'static> TableSyncJob<R> {
    pub fn new<E, FKR>(
        table_name: String,
        initial_metadata: SyncTableMetadata,
        fk_resolver: Arc<FKR>,
    ) -> Self
    where
        E: HLCModel + EntityTrait + Send + Sync + 'static,
        E::Column: Send + Sync,
        E::Model: HLCRecord
            + Send
            + Sync
            + Debug
            + Clone
            + Serialize
            + for<'de> Deserialize<'de>
            + IntoActiveModel<E::ActiveModel>
            + ModelWithForeignKeyOps
            + 'static,
        E::ActiveModel:
            ActiveModelBehavior + Send + Sync + Debug + ActiveModelWithForeignKeyOps + 'static,
        E::PrimaryKey: PrimaryKeyTrait
            + PrimaryKeyFromStr<<E::PrimaryKey as PrimaryKeyTrait>::ValueType>
            + 'static,
        <E::PrimaryKey as PrimaryKeyTrait>::ValueType:
            Eq + Hash + Clone + Send + Sync + Debug + Ord + Into<Value> + 'static,
        FKR: ForeignKeyResolver + Send + Sync + Debug + 'static,
    {
        let task_table_name_captured = table_name.clone();
        Self {
            table_name,
            initial_metadata,
            task: Box::new(
                move |context: &SyncContext<'_, R>, metadata_arg: SyncTableMetadata| {
                    let t_name = task_table_name_captured.clone();
                    let resolver_captured = fk_resolver.clone();

                    Box::pin(async move {
                        core::synchronize_table::<E, R, FKR>(
                            // R is the concrete type from SyncContext
                            context,
                            Some(resolver_captured.as_ref()),
                            &t_name,
                            &metadata_arg,
                        )
                        .await
                    })
                },
            ),
        }
    }
}

/// Result of a single table's synchronization attempt.
#[derive(Debug)]
pub enum TableSyncResult {
    Success(SyncTableMetadata),
    Failure {
        table_name: String,
        error: anyhow::Error,
    },
}

impl TableSyncResult {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success(_))
    }

    pub fn get_metadata(&self) -> Option<&SyncTableMetadata> {
        match self {
            Self::Success(summary) => Some(summary),
            Self::Failure { .. } => None,
        }
    }

    // Equivalent to std::result::Result::err
    pub fn get_error(&self) -> Option<&anyhow::Error> {
        match self {
            Self::Success(_) => None,
            Self::Failure { error, .. } => Some(error),
        }
    }

    // Equivalent to std::result::Result::unwrap
    // Consumes self
    pub fn unwrap_metadata(self) -> SyncTableMetadata {
        // Returning TableSyncSummary
        match self {
            Self::Success(summary) => summary,
            Self::Failure { table_name, error } => {
                panic!(
                    "called unwrap_summary() on a Failure value for table '{}': {:?}",
                    table_name, error
                )
            }
        }
    }

    // Provides a reference to the summary, panics on failure.
    pub fn metadata_ref(&self) -> &SyncTableMetadata {
        // Returning TableSyncSummary
        match self {
            Self::Success(summary) => summary,
            Self::Failure { table_name, error } => {
                panic!(
                    "called summary_ref() on a Failure value for table '{}': {:?}",
                    table_name, error
                )
            }
        }
    }

    // Helper to get table_name, useful for error messages
    pub fn table_name_str(&self) -> &str {
        match self {
            Self::Success(summary) => &summary.table_name,
            Self::Failure { table_name, .. } => table_name,
        }
    }
}

/// Manages and executes a sequence of table synchronization jobs.
#[derive(Debug)]
pub struct SyncScheduler;

impl SyncScheduler {
    pub fn new() -> Self {
        Self
    }

    /// Runs a series of synchronization jobs.
    /// `R` is the concrete type of the `RemoteDataSource` implementation.
    pub async fn run_plan<R: RemoteDataSource + Send + Sync + Debug + 'static>(
        &self,
        context: &SyncContext<'_, R>, // Context has concrete R
        jobs: Vec<TableSyncJob<R>>,   // Jobs are specific to this R
    ) -> Vec<TableSyncResult> {
        let mut results = Vec::with_capacity(jobs.len());

        if jobs.is_empty() {
            info!("Sync plan is empty. Nothing to do.");
            return results;
        }

        info!("Starting sync plan with {} job(s).", jobs.len());

        for job in jobs {
            // job.initial_metadata is moved into the closure call.
            // If you need to access it after the call for some reason (e.g. original HLC), clone it before.
            let table_name_for_log = job.table_name.clone();
            let initial_hlc_for_log = job.initial_metadata.last_sync_hlc.clone();

            info!(
                "Scheduler: Starting sync for table '{}' from HLC: {}",
                table_name_for_log, initial_hlc_for_log
            );

            match (job.task)(context, job.initial_metadata).await {
                Ok(updated_metadata) => {
                    info!(
                        "Scheduler: Successfully synced table '{}'. New last_sync_hlc: {}",
                        updated_metadata.table_name, updated_metadata.last_sync_hlc
                    );
                    results.push(TableSyncResult::Success(updated_metadata));
                }
                Err(e) => {
                    error!(
                        "Scheduler: Failed to sync table '{}': {:?}",
                        table_name_for_log, e
                    );
                    results.push(TableSyncResult::Failure {
                        table_name: table_name_for_log,
                        error: e,
                    });
                }
            }
        }

        info!(
            "Sync plan execution finished. {} job(s) processed.",
            results.len()
        );
        results
    }
}

impl Default for SyncScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunking::ChunkingOptions;
    use crate::core::tests::test_entity;
    use crate::core::tests::MockRemoteDataSource;
    use crate::core::tests::NoOpForeignKeyResolver;
    use crate::core::SyncDirection;
    use crate::hlc::{SyncTaskContext, HLC};

    use anyhow::anyhow;
    use sea_orm::{ConnectionTrait, Database, DbBackend, DbConn, Schema};
    use std::sync::Arc;
    use uuid::Uuid;

    async fn setup_scheduler_test_db() -> Result<DbConn> {
        let db = Database::connect("sqlite::memory:").await?;
        let schema = Schema::new(DbBackend::Sqlite);
        db.execute(
            db.get_database_backend()
                .build(&schema.create_table_from_entity(test_entity::Entity)),
        )
        .await?;
        Ok(db)
    }

    fn create_test_sync_context<'a>(
        db: &'a DbConn,
        remote_source: &'a MockRemoteDataSource,
        hlc_context_ref: &'a SyncTaskContext,
        local_node_id: Uuid,
    ) -> SyncContext<'a, MockRemoteDataSource> {
        SyncContext {
            db,
            local_node_id,
            remote_source,
            chunking_options: ChunkingOptions::default(local_node_id),
            sync_direction: SyncDirection::Bidirectional,
            hlc_context: hlc_context_ref,
        }
    }

    #[tokio::test]
    async fn test_scheduler_empty_plan() -> Result<()> {
        let db = setup_scheduler_test_db().await?;
        let local_node_id = Uuid::new_v4();
        let remote_source = MockRemoteDataSource::new(Uuid::new_v4());
        let hlc_context = SyncTaskContext::new(local_node_id);
        let context = create_test_sync_context(&db, &remote_source, &hlc_context, local_node_id);

        let scheduler = SyncScheduler::new();
        let jobs: Vec<TableSyncJob<MockRemoteDataSource>> = Vec::new(); // Explicit R
        let report = scheduler.run_plan(&context, jobs).await;

        assert!(report.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_scheduler_single_job_success() -> Result<()> {
        let db = setup_scheduler_test_db().await?;
        let local_node_id = Uuid::new_v4();
        let remote_node_id = Uuid::new_v4();
        let remote_source = MockRemoteDataSource::new(remote_node_id);
        remote_source
            .set_remote_chunks_for_table("test_items", vec![])
            .await;

        let hlc_context = SyncTaskContext::new(local_node_id);
        let context = create_test_sync_context(&db, &remote_source, &hlc_context, local_node_id);

        let scheduler = SyncScheduler::new();
        let initial_hlc = HLC::new(local_node_id);
        let table_name = "test_items".to_string();

        let fk_resolver_arc = Arc::new(NoOpForeignKeyResolver);
        let job = TableSyncJob::<MockRemoteDataSource>::new::<
            test_entity::Entity,
            NoOpForeignKeyResolver,
        >(
            table_name.clone(),
            SyncTableMetadata {
                table_name: table_name.clone(),
                last_sync_hlc: initial_hlc.clone(),
            },
            fk_resolver_arc,
        );
        let jobs = vec![job];

        let report = scheduler.run_plan(&context, jobs).await;

        assert_eq!(report.len(), 1);
        match &report[0] {
            TableSyncResult::Success(meta) => {
                assert_eq!(meta.table_name, table_name);
                assert!(meta.last_sync_hlc >= initial_hlc);
            }
            TableSyncResult::Failure { .. } => panic!("Expected success for single job"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_scheduler_single_job_failure_from_remote_source() -> Result<()> {
        let db = setup_scheduler_test_db().await?;
        let local_node_id = Uuid::new_v4();
        let mut remote_source = MockRemoteDataSource::new(Uuid::new_v4());
        remote_source.fail_on_get_chunks = true;

        let hlc_context = SyncTaskContext::new(local_node_id);
        let context = create_test_sync_context(&db, &remote_source, &hlc_context, local_node_id);

        let scheduler = SyncScheduler::new();
        let initial_hlc = HLC::new(local_node_id);
        let table_name = "test_items_fail".to_string();

        let fk_resolver_arc = Arc::new(NoOpForeignKeyResolver);
        let job = TableSyncJob::<MockRemoteDataSource>::new::<
            test_entity::Entity,
            NoOpForeignKeyResolver,
        >(
            table_name.clone(),
            SyncTableMetadata {
                table_name: table_name.clone(),
                last_sync_hlc: initial_hlc.clone(),
            },
            fk_resolver_arc,
        );
        let jobs = vec![job];

        let report = scheduler.run_plan(&context, jobs).await;

        assert_eq!(report.len(), 1);
        match &report[0] {
            TableSyncResult::Success(_) => panic!("Expected failure"),
            TableSyncResult::Failure {
                table_name: failed_table,
                error,
            } => {
                assert_eq!(failed_table, &table_name);
                assert!(error.to_string().contains("Failed to fetch remote chunks"));
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_scheduler_multiple_jobs_one_fails() -> Result<()> {
        let db = setup_scheduler_test_db().await?;
        let local_node_id = Uuid::new_v4();
        let remote_source = MockRemoteDataSource::new(Uuid::new_v4());
        remote_source
            .set_remote_chunks_for_table("table_ok", vec![])
            .await;
        remote_source
            .set_remote_chunks_for_table("table_ok_again", vec![])
            .await;

        let hlc_context = SyncTaskContext::new(local_node_id);
        let context = create_test_sync_context(&db, &remote_source, &hlc_context, local_node_id);

        let scheduler = SyncScheduler::new();
        let initial_hlc = HLC::new(local_node_id);
        let fk_resolver_arc = Arc::new(NoOpForeignKeyResolver);

        let table1_name = "table_ok".to_string();
        let job1 = TableSyncJob::<MockRemoteDataSource>::new::<
            test_entity::Entity,
            NoOpForeignKeyResolver,
        >(
            table1_name.clone(),
            SyncTableMetadata {
                table_name: table1_name.clone(),
                last_sync_hlc: initial_hlc.clone(),
            },
            fk_resolver_arc.clone(),
        );

        let table2_name = "table_fail".to_string();
        let table2_name_captured = table2_name.clone();

        let failing_task_closure: TableSyncTaskFn<MockRemoteDataSource> = Box::new(
            move |_context_arg: &SyncContext<'_, MockRemoteDataSource>,
                  _metadata_arg: SyncTableMetadata| {
                let tn_captured_for_async = table2_name_captured.clone(); // Capture for the async block
                Box::pin(
                    async move { Err(anyhow!("Simulated failure for {}", tn_captured_for_async)) },
                )
            },
        );

        let job2 = TableSyncJob::<MockRemoteDataSource> {
            table_name: table2_name.clone(),
            initial_metadata: SyncTableMetadata {
                table_name: table2_name.clone(),
                last_sync_hlc: initial_hlc.clone(),
            },
            task: failing_task_closure,
        };

        let table3_name = "table_ok_again".to_string();
        let job3 = TableSyncJob::<MockRemoteDataSource>::new::<
            test_entity::Entity,
            NoOpForeignKeyResolver,
        >(
            table3_name.clone(),
            SyncTableMetadata {
                table_name: table3_name.clone(),
                last_sync_hlc: initial_hlc.clone(),
            },
            fk_resolver_arc.clone(),
        );

        let jobs = vec![job1, job2, job3];
        let report = scheduler.run_plan(&context, jobs).await;

        assert_eq!(report.len(), 3);
        assert!(matches!(report[0], TableSyncResult::Success(_)));
        match &report[1] {
            TableSyncResult::Failure {
                table_name: tn,
                error: e,
            } => {
                assert_eq!(tn, &table2_name);
                assert!(e.to_string().contains("Simulated failure for table_fail"));
            }
            _ => panic!("Job 2 should have failed"),
        }
        assert!(matches!(report[2], TableSyncResult::Success(_)));
        Ok(())
    }
}
