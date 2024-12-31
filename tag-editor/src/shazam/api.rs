use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use chrono::Utc;
use rand::seq::SliceRandom;
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::spectrogram::Signature;

#[derive(Serialize, Debug)]
struct IdentifyRequest<'a> {
    geolocation: Geolocation,
    signature: SignatureRequest<'a>,
    timestamp: i64,
    timezone: &'a str,
}

#[derive(Serialize, Debug)]
struct Geolocation {
    altitude: f64,
    latitude: f64,
    longitude: f64,
}

#[derive(Serialize, Debug)]
struct SignatureRequest<'a> {
    samplems: f64,
    timestamp: i64,
    uri: &'a str,
}

#[derive(Deserialize, Debug)]
struct IdentifyResponse {
    matches: Vec<Match>,
    track: Option<Track>,
}

#[derive(Deserialize, Debug)]
pub struct Match {
    pub offset: f64,
    pub time_skew: f64,
}

#[derive(Deserialize, Debug)]
pub struct Track {
    pub title: String,
    pub subtitle: String,
    pub sections: Vec<Section>,
    pub hub: Hub,
}

#[derive(Deserialize, Debug)]
pub struct Section {
    #[serde(rename = "type")]
    pub section_type: String,
    pub metadata: Vec<Metadata>,
}

#[derive(Deserialize, Debug)]
pub struct Metadata {
    pub title: String,
    pub text: String,
}

#[derive(Deserialize, Debug)]
pub struct Hub {
    pub actions: Vec<Action>,
}

#[derive(Deserialize, Debug)]
pub struct Action {
    pub name: String,
    pub id: Option<String>,
}

pub async fn identify(signature: Signature) -> Result<(Vec<Match>, Option<Track>)> {
    let sample_rate: i32 = signature.sample_rate;
    let signature_data = signature.encode();

    let encoded_signature = general_purpose::STANDARD.encode(&signature_data);

    let request = IdentifyRequest {
        geolocation: Geolocation {
            altitude: 300.0,
            latitude: 45.0,
            longitude: 2.0,
        },
        signature: SignatureRequest {
            samplems: sample_rate as f64,
            timestamp: Utc::now().timestamp_millis(),
            uri: &format!("data:audio/vnd.shazam.sig;base64,{}", encoded_signature),
        },
        timestamp: Utc::now().timestamp_millis(),
        timezone: "Europe/Berlin",
    };

    let client = Client::new();
    let url = format!(
        "http://amp.shazam.com/discovery/v5/en/US/android/-/tag/{}/{}",
        Uuid::new_v4().to_string().to_uppercase(),
        Uuid::new_v4()
    );
    let query =
        "?sync=true&webv3=true&sampling=true&connected=&shazamapiversion=v3&sharehub=true&video=v3";

    let response = client
        .post(format!("{}{}", url, query))
        .header(
            header::USER_AGENT,
            header::HeaderValue::from_static(
                USER_AGENTS
                    .choose(&mut rand::thread_rng())
                    .ok_or_else(|| anyhow::anyhow!("Failed to choose a user agent"))?,
            ),
        )
        .header(
            header::CONTENT_LANGUAGE,
            header::HeaderValue::from_static("en_US"),
        )
        .header(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        )
        .json(&request)
        .send()
        .await?;

    if response.status().is_success() {
        let resp_data: IdentifyResponse = response.json().await?;
        Ok((resp_data.matches, resp_data.track))
    } else {
        Err(anyhow::anyhow!(
            "Failed to identify track, status: {}",
            response.status()
        ))
    }
}

const USER_AGENTS: [&str; 100] = [
    "Dalvik/2.1.0 (Linux; U; Android 5.0.2; VS980 4G Build/LRX22G)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; SM-T210 Build/KOT49H)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-P905V Build/LMY47X)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.4; Vodafone Smart Tab 4G Build/KTU84P)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.4; SM-G360H Build/KTU84P)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0.2; SM-S920L Build/LRX22G)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0; Fire Pro Build/LRX21M)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0; SM-N9005 Build/LRX21V)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; SM-G920F Build/MMB29K)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; SM-G7102 Build/KOT49H)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0; SM-G900F Build/LRX21T)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; SM-G928F Build/MMB29K)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-J500FN Build/LMY48B)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; Coolpad 3320A Build/LMY47V)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.4; SM-J110F Build/KTU84P)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; SAMSUNG-SGH-I747 Build/KOT49H)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; SAMSUNG-SM-T337A Build/KOT49H)",
    "Dalvik/1.6.0 (Linux; U; Android 4.3; SGH-T999 Build/JSS15J)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; D6603 Build/23.5.A.0.570)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-J700H Build/LMY48B)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; HTC6600LVW Build/KOT49H)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-N910G Build/LMY47X)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-N910T Build/LMY47X)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.4; C6903 Build/14.4.A.0.157)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; SM-G920F Build/MMB29K)",
    "Dalvik/1.6.0 (Linux; U; Android 4.2.2; GT-I9105P Build/JDQ39)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0; SM-G900F Build/LRX21T)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; GT-I9192 Build/KOT49H)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-G531H Build/LMY48B)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0; SM-N9005 Build/LRX21V)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; LGMS345 Build/LMY47V)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0.2; HTC One Build/LRX22G)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0.2; LG-D800 Build/LRX22G)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-G531H Build/LMY48B)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0; SM-N9005 Build/LRX21V)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.4; SM-T113 Build/KTU84P)",
    "Dalvik/1.6.0 (Linux; U; Android 4.2.2; AndyWin Build/JDQ39E)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0; Lenovo A7000-a Build/LRX21M)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; LGL16C Build/KOT49I.L16CV11a)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; GT-I9500 Build/KOT49H)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0.2; SM-A700FD Build/LRX22G)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; SM-G130HN Build/KOT49H)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; SM-N9005 Build/KOT49H)",
    "Dalvik/1.6.0 (Linux; U; Android 4.1.2; LG-E975T Build/JZO54K)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; E1 Build/KOT49H)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; GT-I9500 Build/KOT49H)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; GT-N5100 Build/KOT49H)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-A310F Build/LMY47X)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-J105H Build/LMY47V)",
    "Dalvik/1.6.0 (Linux; U; Android 4.3; GT-I9305T Build/JSS15J)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; android Build/JDQ39)",
    "Dalvik/1.6.0 (Linux; U; Android 4.2.1; HS-U970 Build/JOP40D)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.4; SM-T561 Build/KTU84P)",
    "Dalvik/1.6.0 (Linux; U; Android 4.2.2; GT-P3110 Build/JDQ39)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; SM-G925T Build/MMB29K)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; HUAWEI Y221-U22 Build/HUAWEIY221-U22)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-G530T1 Build/LMY47X)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-G920I Build/LMY47X)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0; SM-G900F Build/LRX21T)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; Vodafone Smart ultra 6 Build/LMY47V)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.4; XT1080 Build/SU6-7.7)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.4; ASUS MeMO Pad 7 Build/KTU84P)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; SM-G800F Build/KOT49H)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; GT-N7100 Build/KOT49H)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; SM-G925I Build/MMB29K)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; A0001 Build/MMB29X)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1; XT1045 Build/LPB23.13-61)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; LGMS330 Build/LMY47V)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.4; Z970 Build/KTU84P)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0; SM-N900P Build/LRX21V)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; T1-701u Build/HuaweiMediaPad)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1; HTCD100LVWPP Build/LMY47O)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; SM-G935R4 Build/MMB29M)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; SM-G930V Build/MMB29M)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0.2; ZTE Blade Q Lux Build/LRX22G)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.4; GT-I9060I Build/KTU84P)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; LGUS992 Build/MMB29M)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; SM-G900P Build/MMB29M)",
    "Dalvik/1.6.0 (Linux; U; Android 4.1.2; SGH-T999L Build/JZO54K)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-N910V Build/LMY47X)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; GT-I9500 Build/KOT49H)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-P601 Build/LMY47X)",
    "Dalvik/1.6.0 (Linux; U; Android 4.2.2; GT-S7272 Build/JDQ39)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-N910T Build/LMY47X)",
    "Dalvik/1.6.0 (Linux; U; Android 4.3; SAMSUNG-SGH-I747 Build/JSS15J)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0.2; ZTE Blade Q Lux Build/LRX22G)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; SM-G930F Build/MMB29K)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; HTC_PO582 Build/KOT49H)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0; HUAWEI MT7-TL10 Build/HuaweiMT7-TL10)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0; LG-H811 Build/MRA58K)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; SM-N7505 Build/KOT49H)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0; LG-H815 Build/MRA58K)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; LenovoA3300-HV Build/KOT49H)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.4; SM-G360G Build/KTU84P)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.4; GT-I9300I Build/KTU84P)",
    "Dalvik/2.1.0 (Linux; U; Android 5.0; SM-G900F Build/LRX21T)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; SM-J700T Build/MMB29K)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-J500FN Build/LMY48B)",
    "Dalvik/1.6.0 (Linux; U; Android 4.2.2; SM-T217S Build/JDQ39)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.4; SAMSUNG-SM-N900A Build/KTU84P)",
];
