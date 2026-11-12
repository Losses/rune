use std::{future::Future, time::Duration};

use anyhow::{Context, Result};

pub(crate) const TEST_DEADLINE: Duration = Duration::from_secs(60);

pub(crate) async fn wait_until<T, F, Fut>(what: &str, mut probe: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<T>>>,
{
    tokio::time::timeout(TEST_DEADLINE, async {
        loop {
            if let Some(value) = probe().await? {
                return Ok(value);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .with_context(|| format!("timed out waiting for {what}"))?
}
