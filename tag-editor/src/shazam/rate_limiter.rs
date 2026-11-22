use std::time::Duration;
use tokio::sync as tokio_sync;
use tokio::sync::Mutex;
use tokio::time as tokio_time;
use tokio::time::Instant;

pub struct RateLimiter {
    interval: Duration,
    last_request: tokio_sync::Mutex<Instant>,
}

impl RateLimiter {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_request: Mutex::new(Instant::now()),
        }
    }

    pub async fn acquire(&self) {
        let mut last = self.last_request.lock().await;
        let now = Instant::now();
        let elapsed = now.duration_since(*last);
        if elapsed < self.interval {
            tokio_time::sleep(self.interval - elapsed).await;
        }
        *last = Instant::now();
    }
}
