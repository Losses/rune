#[path = "../../sync-orm/tests/support/note_transactions.rs"]
mod scenario;
use migration::{Migrator, MigratorTrait};
#[tokio::test]
async fn unrelated_business_rows_and_completion_use_the_application_schema() -> anyhow::Result<()> {
    let a = sea_orm::Database::connect("sqlite::memory:").await?;
    let b = sea_orm::Database::connect("sqlite::memory:").await?;
    let _ = migration::initialize_node_id(uuid::Uuid::new_v4().to_string());
    Migrator::up(&a, None).await?;
    Migrator::up(&b, None).await?;
    scenario::verify(a, b).await
}
