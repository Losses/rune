#[path = "../../sync-orm/tests/support/note_transactions.rs"]
mod scenario;
use migration::{Migrator, MigratorTrait};
use sea_orm::Database;
use uuid::Uuid;
#[tokio::test]
async fn unrelated_business_rows_and_completion_use_the_application_schema() -> anyhow::Result<()> {
    let a = Database::connect("sqlite::memory:").await?;
    let b = Database::connect("sqlite::memory:").await?;
    let _ = migration::initialize_node_id(Uuid::new_v4().to_string());
    Migrator::up(&a, None).await?;
    Migrator::up(&b, None).await?;
    scenario::verify(a, b).await
}
