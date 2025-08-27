use anyhow::Result;
use log::info;
use sea_orm::EntityTrait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use ::sync::{
    chunking::DataChunk,
    core::{RemoteDataSource, RemoteRecordsWithPayload, SyncOperation},
    hlc::{HLC, HLCModel, HLCRecord},
};

#[derive(Debug)]
pub struct RemoteHttpDataSource {
    base_url: String,
    client: reqwest::Client,
}

use crate::sync::chunking::{ApplyChangesPayload, GetRemoteSubChunksPayload};

impl RemoteHttpDataSource {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            client: reqwest::Client::new(),
        }
    }

    fn build_url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

#[async_trait::async_trait]
impl RemoteDataSource for RemoteHttpDataSource {
    async fn get_remote_node_id(&self) -> Result<Uuid> {
        let url = self.build_url("/node-id");
        info!("[CLIENT] -> GET {url}");
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        let node_id: Uuid = resp.json().await?;
        Ok(node_id)
    }

    async fn get_remote_chunks<E>(
        &self,
        table_name: &str,
        after_hlc: Option<&HLC>,
    ) -> Result<Vec<DataChunk>>
    where
        E: HLCModel + EntityTrait + Send + Sync,
        E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize,
    {
        let url = self.build_url(&format!("/tables/{table_name}/chunks"));
        let mut query_params = Vec::new();
        if let Some(hlc) = after_hlc {
            query_params.push(("after_hlc_ts", hlc.to_rfc3339()?));
            query_params.push(("after_hlc_ver", hlc.version.to_string()));
            query_params.push(("after_hlc_nid", hlc.node_id.to_string()));
        }

        info!("[CLIENT] -> GET {} with query {:?}", url, query_params);

        let resp = self
            .client
            .get(&url)
            .query(&query_params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    async fn get_remote_sub_chunks<E>(
        &self,
        table_name: &str,
        parent_chunk: &DataChunk,
        sub_chunk_size: u64,
    ) -> Result<Vec<DataChunk>>
    where
        E: HLCModel + EntityTrait + Send + Sync,
        E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize,
    {
        let url = self.build_url(&format!("/tables/{table_name}/sub-chunks"));
        let payload = GetRemoteSubChunksPayload {
            parent_chunk: parent_chunk.clone(),
            sub_chunk_size,
        };
        let resp = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    async fn get_remote_records_in_hlc_range<E>(
        &self,
        table_name: &str,
        start_hlc: &HLC,
        end_hlc: &HLC,
    ) -> Result<RemoteRecordsWithPayload<E::Model>>
    where
        E: HLCModel + EntityTrait + Send + Sync,
        E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize,
    {
        let url = self.build_url(&format!("/tables/{table_name}/records"));
        let query_params = [
            ("start_hlc_ts", start_hlc.to_rfc3339()?),
            ("start_hlc_ver", start_hlc.version.to_string()),
            ("start_hlc_nid", start_hlc.node_id.to_string()),
            ("end_hlc_ts", end_hlc.to_rfc3339()?),
            ("end_hlc_ver", end_hlc.version.to_string()),
            ("end_hlc_nid", end_hlc.node_id.to_string()),
        ];
        let resp = self
            .client
            .get(&url)
            .query(&query_params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    async fn apply_remote_changes<E>(
        &self,
        table_name: &str,
        operations: Vec<SyncOperation<E::Model>>,
        client_node_id: Uuid,
        new_last_sync_hlc: &HLC,
    ) -> Result<HLC>
    where
        E: HLCModel + EntityTrait + Send + Sync,
        E::Model: HLCRecord + Send + Sync + for<'de> Deserialize<'de> + Serialize,
    {
        let url = self.build_url(&format!("/tables/{table_name}/changes"));
        let payload = ApplyChangesPayload {
            operations,
            client_node_id,
            new_last_sync_hlc: new_last_sync_hlc.clone(),
        };

        info!(
            "[CLIENT] -> POST {} with payload: {}",
            url,
            serde_json::to_string_pretty(&payload)?
        );

        let resp = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    async fn get_remote_last_sync_hlc(
        &self,
        table_name: &str,
        local_node_id: Uuid,
    ) -> Result<Option<HLC>> {
        let url = self.build_url(&format!(
            "/tables/{table_name}/last-sync-hlc/{local_node_id}"
        ));
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }
}
