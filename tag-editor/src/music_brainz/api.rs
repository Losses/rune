use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Result, bail};
use log::{error, warn};
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
#[serde(untagged)]
pub enum AcoustIdResponse {
    Success {
        status: String,
        results: Vec<AcoustIdResult>,
    },
    Error {
        status: String,
        error: ErrorDetail,
    },
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
    pub releases: Option<Vec<Release>>,
}

#[derive(Deserialize, Debug)]
pub struct Artist {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize, Debug)]
pub struct Release {
    pub id: String,
    pub mediums: Option<Vec<Medium>>,
}

#[derive(Deserialize, Debug)]
pub struct Medium {
    pub format: String,
    pub position: u32,
    pub track_count: u32,
    pub tracks: Option<Vec<Track>>,
}

#[derive(Deserialize, Debug)]
pub struct Track {
    pub id: String,
    pub position: u32,
    pub title: String,
    pub artists: Vec<Artist>,
}

#[derive(Deserialize, Debug)]
pub struct ErrorDetail {
    pub code: i32,
    pub message: String,
}

async fn identify_implementation(
    api_key: &str,
    fingerprint: Vec<u32>,
    config: &Configuration,
    duration: u32,
) -> Result<Vec<AcoustIdResult>> {
    let client = Client::builder().gzip(true).build()?;

    let encoded_fingerprint = encode_fingerprint(fingerprint, config, false, true);

    let mut form_data = HashMap::new();
    form_data.insert("format", "json");
    form_data.insert("client", api_key);
    let duration_str = duration.to_string();
    form_data.insert("duration", &duration_str);
    form_data.insert("fingerprint", &encoded_fingerprint);
    form_data.insert("meta", "recordings releases tracks");

    let response = client
        .post("https://api.acoustid.org/v2/lookup")
        .form(&form_data)
        .send()
        .await?;

    let response_text = response.text().await?;

    let parsed_response: AcoustIdResponse = serde_json::from_str(&response_text)?;

    match parsed_response {
        AcoustIdResponse::Success { results, .. } => Ok(results),
        AcoustIdResponse::Error { error, .. } => bail!("Error {}: {}", error.code, error.message),
    }
}

pub async fn identify(
    api_key: &str,
    fingerprint: Vec<u32>,
    config: &Configuration,
    duration: u32,
) -> Result<Vec<AcoustIdResult>> {
    let mut attempts = 0;
    loop {
        if attempts > 0 {
            sleep(Duration::from_secs(1)).await;
        }

        match identify_implementation(api_key, fingerprint.clone(), config, duration).await {
            Ok(results) => return Ok(results),
            Err(e) if attempts < 3 => {
                attempts += 1;
                warn!("Attempt {attempts} failed: {e}. Retrying...");
                continue;
            }
            Err(e) => {
                error!("Failed after {attempts} attempts: {e}");
                return Err(e);
            }
        }
    }
}
