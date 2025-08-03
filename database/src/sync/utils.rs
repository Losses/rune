use anyhow::Result;
use chrono::{DateTime, FixedOffset};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

use ::sync::hlc::HLC;

use crate::entities::sync_record::{self, ActiveModel};

pub fn create_sync_record_active_model(
    table_name: String,
    client_node_id: Uuid,
    hlc: &HLC,
) -> Result<ActiveModel> {
    use sea_orm::ActiveValue::Set;
    Ok(ActiveModel {
        table_name: Set(table_name),
        client_node_id: Set(client_node_id.to_string()),
        last_sync_hlc_ts: Set(hlc.to_rfc3339()?),
        last_sync_hlc_ver: Set(hlc.version as i32),
        last_sync_hlc_nid: Set(hlc.node_id.to_string()),
        ..Default::default()
    })
}

pub fn parse_hlc(timestamp: &str, last_sync_hlc_ver: i32, last_sync_hlc_nid: &str) -> Result<HLC> {
    Ok(HLC {
        timestamp_ms: timestamp
            .parse::<DateTime<FixedOffset>>()?
            .timestamp_millis() as u64,
        version: last_sync_hlc_ver as u32,
        node_id: Uuid::parse_str(last_sync_hlc_nid)?,
    })
}

pub async fn get_local_last_sync_hlc(
    db: &DatabaseConnection,
    table_name: &str,
    client_node_id: Uuid,
) -> Result<Option<HLC>> {
    let sync_log_model = sync_record::Entity::find()
        .filter(sync_record::Column::TableName.eq(table_name))
        .filter(sync_record::Column::ClientNodeId.eq(client_node_id.to_string()))
        .one(db)
        .await?;

    if let Some(log_entry) = sync_log_model {
        let hlc = parse_hlc(
            &log_entry.last_sync_hlc_ts,
            log_entry.last_sync_hlc_ver,
            &log_entry.last_sync_hlc_nid,
        )?;
        Ok(Some(hlc))
    } else {
        Ok(None)
    }
}
