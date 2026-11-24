//! A request that takes long in total is logged, without its query or tokens.

mod common;

use std::sync::Mutex;
use std::time::Duration;

use common::{FixtureServer, Reply, TestTokens, client};
use log::{Level, LevelFilter, Log, Metadata, Record};

static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl Log for Capture {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Warn
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            WARNINGS.lock().unwrap().push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

#[tokio::test]
async fn slow_request_is_logged_without_query_or_tokens() {
    log::set_logger(&Capture).unwrap();
    log::set_max_level(LevelFilter::Warn);

    let server = FixtureServer::start().await;
    server.on(
        "GET",
        "/v1/me/storefront",
        Reply::fixture("storefront.json"),
    );
    server.delay("GET", "/v1/me/storefront", Duration::from_millis(3200));
    let c = client(&server, TestTokens::new());

    c.user_storefront().await.unwrap();

    let warnings = WARNINGS.lock().unwrap().clone();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let line = &warnings[0];
    assert!(line.contains("GET /v1/me/storefront took"), "{line}");
    assert!(line.contains("1 attempt(s)"), "{line}");
    assert!(
        !line.contains("dev-0") && !line.contains("user-0"),
        "{line}"
    );
    assert!(!line.contains('?'), "{line}");
}
