use anyhow::Result;
use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use uuid::Uuid;

use ::sync::hlc::HLC;

use crate::entities::sync_record::ActiveModel;

fn convert_to_rfc3339(timestamp_ms: u64) -> Result<String> {
    let secs = (timestamp_ms / 1000) as i64;
    let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;

    Ok(Utc
        .timestamp_opt(secs, nanos)
        .single()
        .ok_or_else(|| anyhow::anyhow!("invalid timestamp"))?
        .to_rfc3339())
}

pub fn create_sync_record_active_model(
    table_name: String,
    client_node_id: Uuid,
    hlc: &HLC,
) -> Result<ActiveModel> {
    use sea_orm::ActiveValue::Set;
    Ok(ActiveModel {
        table_name: Set(table_name),
        client_node_id: Set(client_node_id.to_string()),
        last_sync_hlc_ts: Set(convert_to_rfc3339(hlc.timestamp)?),
        last_sync_hlc_ver: Set(hlc.version as i32),
        last_sync_hlc_nid: Set(hlc.node_id.to_string()),
        ..Default::default()
    })
}

pub fn parse_hlc(timestamp: &str, last_sync_hlc_ver: i32, last_sync_hlc_nid: &str) -> Result<HLC> {
    Ok(HLC {
        timestamp: timestamp.parse::<DateTime<FixedOffset>>()?.timestamp() as u64,
        version: last_sync_hlc_ver as u32,
        node_id: Uuid::parse_str(last_sync_hlc_nid)?,
    })
}
