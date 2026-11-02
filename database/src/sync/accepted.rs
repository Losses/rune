//! Read-only access to accepted state inside a caller-owned transaction.
use super::SeaOrmSyncStore;
use super::model::{Collection, ObjectKey, Record};
use anyhow::Result;
use sea_orm::DatabaseTransaction;
use uuid::Uuid;

pub struct AcceptedState<'a> {
    transaction: &'a DatabaseTransaction,
    store: &'a SeaOrmSyncStore,
}
impl<'a> AcceptedState<'a> {
    pub fn new(transaction: &'a DatabaseTransaction, store: &'a SeaOrmSyncStore) -> Self {
        Self { transaction, store }
    }
    pub async fn get(&self, key: &ObjectKey) -> Result<Option<Record>> {
        Ok(self
            .store
            .get_on(self.transaction, key)
            .await?
            .filter(|record| record.value.is_some()))
    }
    pub async fn page(
        &self,
        collection: Collection,
        after: Option<Uuid>,
        limits: sync::protocol::PageLimits,
    ) -> Result<sync_orm::causal_store::ProjectedPage<super::model::RuneModel>> {
        self.store
            .projected_records_on(self.transaction, collection, after, limits)
            .await
    }
}
