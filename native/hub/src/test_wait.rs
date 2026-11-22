use std::{future::Future, time::Duration};

use anyhow::{Context, Result};
use tokio::time as tokio_time;

pub(crate) const TEST_DEADLINE: Duration = Duration::from_secs(60);

pub(crate) async fn wait_until<T, F, Fut>(what: &str, mut probe: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<T>>>,
{
    tokio_time::timeout(TEST_DEADLINE, async {
        loop {
            if let Some(value) = probe().await? {
                return Ok(value);
            }
            tokio_time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .with_context(|| format!("timed out waiting for {what}"))?
}
