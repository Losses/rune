use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use log::warn;
use reqwest::Client;
use rusty_chromaprint::Configuration;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use super::fingerprint::encode_fingerprint;

#[derive(Serialize, Debug)]
pub struct AcoustIdRequest<'a> {
    pub format: &'a str,
    pub client: &'a str,
    pub duration: u32,
    pub fingerprint: &'a str,
    pub meta: &'a str,
}

#[derive(Deserialize, Debug)]
pub struct AcoustIdResponse {
    pub status: String,
    pub results: Vec<AcoustIdResult>,
}

#[derive(Deserialize, Debug)]
pub struct AcoustIdResult {
    pub id: String,
    pub score: f64,
    pub recordings: Option<Vec<Recording>>,
}

#[derive(Deserialize, Debug)]
pub struct Recording {
    pub id: String,
    pub title: String,
    pub artists: Vec<Artist>,
}

#[derive(Deserialize, Debug)]
pub struct Artist {
    pub id: String,
    pub name: String,
}

async fn identify_implementation(
    api_key: &str,
    fingerprint: Vec<u32>,
    config: &Configuration,
    duration: u32,
) -> Result<String> {
    let client = Client::builder().gzip(true).build()?;

    let encoded_fingerprint = encode_fingerprint(fingerprint, config, false, true);

    let mut form_data = HashMap::new();
    form_data.insert("format", "json");
    form_data.insert("client", api_key);
    let duration_str = duration.to_string();
    form_data.insert("duration", &duration_str);
    form_data.insert("fingerprint", &encoded_fingerprint);
    form_data.insert("meta", "recordingids releaseids tracks");

    let response = client
        .post("https://api.acoustid.org/v2/lookup")
        .form(&form_data)
        .send()
        .await?;
    // .await?
    // .json::<AcoustIdResponse>()
    // .await?;

    Ok(response.text().await?)
}

pub async fn identify(
    api_key: &str,
    fingerprint: Vec<u32>,
    config: &Configuration,
    duration: u32,
) -> Result<String> {
    let mut attempts = 0;
    loop {
        if attempts > 0 {
            sleep(Duration::from_secs(1)).await;
        }

        match identify_implementation(api_key, fingerprint.clone(), config, duration).await {
            Ok(response) => return Ok(response),
            Err(e) if attempts < 3 => {
                attempts += 1;
                warn!("Attempt {} failed: {}. Retrying...", attempts, e);
                continue;
            }
            Err(e) => return Err(e),
        }
    }
}
