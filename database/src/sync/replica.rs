//! Rune library composition and transactional receive.
use super::SeaOrmSyncStore;
use super::model::Collection;
use super::model::{ObjectKey, Payload, Record, RuneModel};
use crate::sync::{import, legacy};
use anyhow::{Result, ensure};
use sea_orm::{DatabaseConnection, TransactionTrait};
use std::sync::Arc;
use sync::{StoreIdentity, SyncModel, SyncStore};
use sync_orm::{
    causal_capabilities::CausalEndpoint,
    causal_schema::clear_namespace_on,
    causal_store::namespace_model_on,
    registry::{ProjectionBudget, RegistryProjection},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct LibrarySyncStore {
    db: DatabaseConnection,
    inner: SeaOrmSyncStore,
    endpoint: CausalEndpoint<RuneModel>,
}
impl LibrarySyncStore {
    pub fn record_store(&self) -> SeaOrmSyncStore {
        self.inner.clone()
    }
    /// Explicitly traverse the registry; reads never initiate source import.
    async fn import_collections(&self, collections: &[super::model::Collection]) -> Result<()> {
        if sync_orm::causal_progress::initialization_complete_on(&self.db, self.inner.namespace())
            .await?
        {
            return Ok(());
        }
        for (index, collection) in collections.iter().enumerate() {
            let mut after = None;
            loop {
                let tx = self.db.begin().await?;
                let page = import::batch(&tx, &self.inner, collection, after).await?;
                if page.done && index + 1 == collections.len() {
                    sync_orm::causal_progress::complete_initialization_on(
                        &tx,
                        self.inner.namespace(),
                    )
                    .await?;
                }
                tx.commit().await?;
                if page.done {
                    break;
                }
                ensure!(page.next != after, "source traversal made no progress");
                after = page.next;
            }
        }
        Ok(())
    }
    pub async fn import_basic(&self) -> Result<()> {
        self.import_collections(&<RuneModel as sync::protocol::SyncModel>::collections())
            .await
    }
    pub async fn import_existing(&self, device_id: Uuid) -> Result<()> {
        ensure!(
            device_id == self.inner.author(),
            "import actor differs from local device"
        );
        self.import_basic().await
    }
    pub async fn open(
        db: DatabaseConnection,
        node_id: Uuid,
        library_id: Option<Uuid>,
    ) -> Result<Self> {
        Self::open_with_file_token(db, node_id, library_id, None).await
    }
    pub async fn open_with_file_token(
        db: DatabaseConnection,
        node_id: Uuid,
        library_id: Option<Uuid>,
        file_token: Option<Uuid>,
    ) -> Result<Self> {
        ensure!(!node_id.is_nil(), "actor identity cannot be nil");
        ensure!(
            library_id.is_none_or(|id| !id.is_nil()),
            "namespace identity cannot be nil"
        );
        RuneModel::validate_database_on(&db).await?;
        let mut existing = sync_orm::causal_store::list_namespace_ids_on(&db, 2).await?;
        ensure!(
            existing.len() <= 1,
            "multiple namespaces require explicit selection"
        );
        let reset = existing.as_slice() == [legacy::namespace()]
            && library_id.is_some_and(|id| id != legacy::namespace());
        if reset {
            legacy::reset(&db, node_id).await?;
            existing.clear();
        }
        let actual = existing
            .first()
            .copied()
            .or(library_id)
            .unwrap_or_else(Uuid::new_v4);
        ensure!(
            library_id.is_none_or(|id| id == actual),
            "namespace mismatch"
        );
        let mut rebuild_encoding = false;
        if !existing.is_empty()
            && let Some(model) = namespace_model_on(&db, actual).await?
            && model.model == RuneModel::model_id()
            && model.schema == "1"
        {
            // Candidate digests bind the old JSON arrays; rebuild causal history from source rows.
            let tx = db.begin().await?;
            clear_namespace_on(&tx, actual).await?;
            tx.commit().await?;
            rebuild_encoding = true;
        }
        let inner = if existing.is_empty() || rebuild_encoding {
            SeaOrmSyncStore::create_empty_with_file_token(db.clone(), actual, node_id, file_token)
                .await?
        } else {
            SeaOrmSyncStore::open_with_file_token(db.clone(), actual, node_id, file_token).await?
        };
        let endpoint = CausalEndpoint::with_projection(
            inner.clone(),
            Arc::new(RegistryProjection::<RuneModel>::new(ProjectionBudget {
                max_reads: 65536,
                max_mutations: 65536,
            })?),
        );
        let library = Self {
            db,
            inner,
            endpoint,
        };
        if reset || rebuild_encoding {
            library.import_existing(node_id).await?;
        }
        Ok(library)
    }
    pub async fn maintain(&self, state: &mut sync_orm::maintenance::Maintenance) -> Result<()> {
        state
            .advance::<RuneModel>(
                &self.db,
                self.identity().await?,
                chrono::Utc::now().timestamp_millis(),
            )
            .await?;
        Ok(())
    }
    pub fn endpoint(&self) -> CausalEndpoint<RuneModel> {
        self.endpoint.clone()
    }
    pub async fn identity(&self) -> Result<StoreIdentity> {
        Ok(sync::ReceiveStore::identity(&self.endpoint).await?)
    }
    pub async fn head(&self) -> Result<sync::ReceiveProgress> {
        Ok(self.endpoint.head().await?)
    }
    pub async fn get(&self, key: &ObjectKey) -> Result<Option<Record>> {
        Ok(self.endpoint.get(key).await?)
    }
    pub async fn write_local(&self, key: ObjectKey, value: Option<Payload>) -> Result<Record> {
        Ok(self.endpoint.write_local(key, value).await?)
    }
    pub async fn pending_mixes(&self) -> Result<Vec<(Uuid, String)>> {
        Ok(
            super::references::collect(&self.db, &self.inner, [Collection::Mixes])
                .await?
                .into_iter()
                .map(|(key, reason)| (key.id, reason))
                .collect(),
        )
    }
    /// Explicit diagnostic; automatic synchronization does not traverse these pages.
    pub async fn pending_references(&self) -> Result<Vec<(ObjectKey, String)>> {
        super::references::collect(
            &self.db,
            &self.inner,
            <RuneModel as sync::SyncModel>::collections(),
        )
        .await
    }
}
