use anyhow::Result;
use chrono::{DateTime, FixedOffset};
use uuid::Uuid;

use ::sync::hlc::HLC;

use crate::entities::sync_record::ActiveModel;

pub fn create_sync_record_active_model(
    table_name: String,
    client_node_id: Uuid,
    hlc: &HLC,
) -> Result<ActiveModel> {
    use sea_orm::ActiveValue::Set;
    Ok(ActiveModel {
        table_name: Set(table_name),
        client_node_id: Set(client_node_id.to_string()),
        last_sync_hlc_ts: Set(hlc.to_rfc3339()),
        last_sync_hlc_ver: Set(hlc.version as i32),
        last_sync_hlc_nid: Set(hlc.node_id.to_string()),
        ..Default::default()
    })
}

pub fn parse_hlc(timestamp: &str, last_sync_hlc_ver: i32, last_sync_hlc_nid: &str) -> Result<HLC> {
    Ok(HLC {
        timestamp_ms: timestamp.parse::<DateTime<FixedOffset>>()?.timestamp_millis() as u64,
        version: last_sync_hlc_ver as u32,
        node_id: Uuid::parse_str(last_sync_hlc_nid)?,
    })
}
