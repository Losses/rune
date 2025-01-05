use std::time::Duration;

use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use log::warn;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

#[derive(Serialize, Debug)]
pub struct AcoustIdRequest<'a> {
    pub client: &'a str,
    pub duration: u32,
    pub fingerprint: &'a str,
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
    duration: u32,
) -> Result<AcoustIdResponse> {
    let client = Client::new();

    let fingerprint_bytes: Vec<u8> = fingerprint
        .iter()
        .flat_map(|&num| num.to_be_bytes())
        .collect();
    let encoded_fingerprint = general_purpose::STANDARD.encode(&fingerprint_bytes);

    let request = AcoustIdRequest {
        client: api_key,
        duration,
        fingerprint: &encoded_fingerprint,
    };

    let response = client
        .post("https://api.acoustid.org/v2/lookup")
        .json(&request)
        .send()
        .await?
        .json::<AcoustIdResponse>()
        .await?;

    Ok(response)
}

pub async fn identify(
    api_key: &str,
    fingerprint: Vec<u32>,
    duration: u32,
) -> Result<AcoustIdResponse> {
    let mut attempts = 0;
    loop {
        if attempts > 0 {
            sleep(Duration::from_secs(1)).await;
        }

        match identify_implementation(api_key, fingerprint.clone(), duration).await {
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
