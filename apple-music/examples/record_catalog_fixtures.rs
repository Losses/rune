//! Records public catalog responses as test fixtures.
//!
//! ```text
//! AM_DEV_TOKEN=... cargo run -p apple-music --example record_catalog_fixtures -- tests/fixtures
//! ```
//!
//! Only catalog endpoints are fetched, so no user token is involved and
//! nothing personal ends up in the files.

use std::env;
use std::fs;
use std::path::PathBuf;

async fn fetch(client: &reqwest::Client, token: &str, path: &str) -> String {
    let response = client
        .get(format!("https://api.music.apple.com{path}"))
        .bearer_auth(token)
        .send()
        .await
        .expect("request failed");
    let status = response.status();
    let body = response.text().await.expect("body");
    assert!(status.is_success(), "{path} answered {status}: {body}");
    let value: serde_json::Value = serde_json::from_str(&body).expect("json");
    serde_json::to_string_pretty(&value).expect("pretty") + "\n"
}

#[tokio::main]
async fn main() {
    let token = env::var("AM_DEV_TOKEN").expect("set AM_DEV_TOKEN");
    let out = PathBuf::from(
        env::args()
            .nth(1)
            .unwrap_or_else(|| "tests/fixtures".into()),
    );
    let client = reqwest::Client::new();

    let search = fetch(
        &client,
        &token,
        "/v1/catalog/us/search?term=daft+punk+get+lucky&types=songs&limit=3",
    )
    .await;
    fs::write(out.join("catalog_search_songs.json"), search).expect("write");

    let songs = fetch(
        &client,
        &token,
        "/v1/catalog/us/songs?ids=617154366,1613600188",
    )
    .await;
    fs::write(out.join("catalog_songs.json"), songs).expect("write");
}
