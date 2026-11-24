use sea_orm::Database;
use sync_orm::causal_schema;
#[path = "support/note_transactions.rs"]
mod scenario;
#[tokio::test]
async fn unrelated_business_rows_and_completion_share_transactions() -> anyhow::Result<()> {
    let a = Database::connect("sqlite::memory:").await?;
    let b = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&a).await?;
    causal_schema::install_fresh(&b).await?;
    scenario::verify(a, b).await
}
