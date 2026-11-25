//! The fixture server and a scriptable token source for the tests.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use apple_music::{AppleMusicClient, RetryPolicy, TokenSource};

pub use apple_music::test_fixtures::{FixtureServer, Reply};

/// Hands out numbered tokens and counts refreshes.
pub struct TestTokens {
    state: Mutex<TokenState>,
    pub with_user: bool,
}

#[derive(Default)]
struct TokenState {
    dev_generation: u32,
    user_generation: u32,
    dev_refreshes: u32,
    user_refreshes: u32,
}

impl TestTokens {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::default(),
            with_user: true,
        })
    }

    pub fn without_user() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::default(),
            with_user: false,
        })
    }

    pub fn dev_refreshes(&self) -> u32 {
        self.state.lock().unwrap().dev_refreshes
    }

    pub fn user_refreshes(&self) -> u32 {
        self.state.lock().unwrap().user_refreshes
    }
}

#[async_trait]
impl TokenSource for TestTokens {
    async fn developer_token(&self, refresh: bool) -> apple_music::Result<String> {
        let mut s = self.state.lock().unwrap();
        if refresh {
            s.dev_generation += 1;
            s.dev_refreshes += 1;
        }
        Ok(format!("dev-{}", s.dev_generation))
    }

    async fn user_token(&self, refresh: bool) -> apple_music::Result<Option<String>> {
        let mut s = self.state.lock().unwrap();
        if refresh {
            s.user_generation += 1;
            s.user_refreshes += 1;
        }
        Ok(self
            .with_user
            .then(|| format!("user-{}", s.user_generation)))
    }
}

/// Retries quickly so the tests stay fast.
pub fn fast_retry() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 5,
        base_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(40),
        max_retry_after: Duration::from_secs(5),
    }
}

pub fn client(server: &FixtureServer, tokens: Arc<TestTokens>) -> AppleMusicClient {
    AppleMusicClient::builder(tokens)
        .base_url(server.base_url.clone())
        .storefront("us")
        .retry_policy(fast_retry())
        .build()
}
