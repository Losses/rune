#[macro_export]
macro_rules! impl_hlc_record_for_model {
    ($model:ty) => {
        impl HLCRecord for $model {
            fn unique_id(&self) -> String {
                self.hlc_uuid.clone()
            }

            fn created_at_hlc(&self) -> Option<HLC> {
                let ts = self.created_at_hlc_ts.parse::<i64>().ok()?;
                let nid = Uuid::parse_str(&self.created_at_hlc_nid).ok()?;
                Some(HLC {
                    timestamp: ts as u64,
                    version: self.created_at_hlc_ver as u32,
                    node_id: nid,
                })
            }

            fn updated_at_hlc(&self) -> Option<HLC> {
                let ts = self.updated_at_hlc_ts.parse::<i64>().ok()?;
                let nid = Uuid::parse_str(&self.updated_at_hlc_nid).ok()?;
                Some(HLC {
                    timestamp: ts as u64,
                    version: self.updated_at_hlc_ver as u32,
                    node_id: nid,
                })
            }

            fn data_for_hashing(&self) -> serde_json::Value {
                serde_json::to_value(self).unwrap_or_else(|e| {
                    error!("Failed to serialize model of type <{}> for hashing: {:?}", stringify!($model), e);
                    serde_json::json!({
                        "error": "Failed to serialize model for hashing",
                        "details": e.to_string()
                    })
                })
            }

            // to_summary() and full_data() use the default impl from the HLCRecord trait
        }
    };
}

#[macro_export]
macro_rules! impl_hlc_model_for_entity {
    ($entity:ty, $col_unique_id:expr, $col_updated_at_time:expr, $col_updated_at_version:expr, $col_updated_at_nid:expr) => {
        impl HLCModel for $entity {
            fn unique_id_column() -> Self::Column {
                $col_unique_id // e.g., entity::Column::HlcUuid
            }

            fn updated_at_time_column() -> Self::Column {
                // This column is expected to store timestamps as RFC3339 strings
                // or a type directly comparable with RFC3339 strings in SQL.
                $col_updated_at_time // e.g., entity::Column::UpdatedAtHlcTs
            }

            fn updated_at_version_column() -> Self::Column {
                $col_updated_at_version // e.g., entity::Column::UpdatedAtHlcVer
            }

            fn updated_at_node_id_column() -> Self::Column {
                $col_updated_at_nid // e.g., entity::Column::UpdatedAtNodeId
            }
        }
    };
}

#[macro_export]
macro_rules! impl_primary_key_from_str_for_i32_pk {
    ($pk_type:ty, $value_type:ty) => {
        impl PrimaryKeyFromStr<$value_type> for $pk_type {
            fn read_key(s: &str) -> Result<$value_type> {
                s.parse::<$value_type>()
                    .with_context(|| format!("Failed to parse primary key string '{}'", s))
            }
        }
    };
}
