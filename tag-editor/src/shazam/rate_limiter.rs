use std::time::Duration;
use tokio::time::Instant;

pub struct RateLimiter {
    interval: Duration,
    last_request: tokio::sync::Mutex<Instant>,
}

impl RateLimiter {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_request: tokio::sync::Mutex::new(Instant::now()),
        }
    }

    pub async fn acquire(&self) {
        let mut last = self.last_request.lock().await;
        let now = Instant::now();
        let elapsed = now.duration_since(*last);
        if elapsed < self.interval {
            tokio::time::sleep(self.interval - elapsed).await;
        }
        *last = Instant::now();
    }
}
