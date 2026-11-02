#[path = "support/note_transactions.rs"]
mod scenario;
#[tokio::test]
async fn unrelated_business_rows_and_completion_share_transactions() -> anyhow::Result<()> {
    let a = sea_orm::Database::connect("sqlite::memory:").await?;
    let b = sea_orm::Database::connect("sqlite::memory:").await?;
    sync_orm::causal_schema::install_fresh(&a).await?;
    sync_orm::causal_schema::install_fresh(&b).await?;
    scenario::verify(a, b).await
}
