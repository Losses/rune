//! Generate protocol dispatch from a single collection registration.

use sync::protocol::SyncError;
/// Register typed row mappings and generate protocol validation and projection.
#[macro_export]
macro_rules! row_registry {
    (
        $vis:vis $model:ident, $collection:ident, $payload:ident {
            model: $model_id:expr, schema: $schema_id:expr,
            collections: { $( $variant:ident => { tag: $tag:literal, entity: $entity:ty, mapping: $mapping:ty, value: $value:ident } ),+ $(,)? }
        }
    ) => {
        #[derive(Clone, Debug, PartialEq, Eq)]
        $vis struct $model;
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
        $vis enum $collection { $($variant),+ }
        #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        #[serde(tag = "collection", content = "value", deny_unknown_fields)]
        $vis enum $payload { $($value($value)),+ }
        impl sync::protocol::SyncModel for $model {
            type Collection = $collection;
            type Payload = $payload;
            fn model_id() -> &'static str { $model_id }
            fn schema_id() -> &'static str { $schema_id }
            fn collections() -> Vec<Self::Collection> { vec![$($collection::$variant),+] }
            fn collection_tag(collection: &Self::Collection) -> &'static str {
                match collection { $($collection::$variant => $tag),+ }
            }
            fn collection_from_tag(tag: &str) -> Option<Self::Collection> {
                match tag { $($tag => Some($collection::$variant),)+ _ => None }
            }
            fn validate_revision(key: &sync::protocol::ObjectKey<Self>, value: &Option<Self::Payload>, revision: &sync::protocol::Revision) -> sync::protocol::Result<()> {
                use sync::protocol::SyncError;
                Self::validate(key, value)?;
                let result = match (&key.collection, value) {
                    $(($collection::$variant, Some($payload::$value(value))) => <$mapping as $crate::row_policy::RowMapping<$entity>>::validate_revision(key.id, Some(value), revision),
                    ($collection::$variant, None) => <$mapping as $crate::row_policy::RowMapping<$entity>>::validate_revision(key.id, None, revision),)+
                    _ => unreachable!(),
                };
                result.map_err(|e| SyncError::Invalid(format!("{} {}: {}", Self::collection_tag(&key.collection), key.id, e)))
            }
            fn validate(key: &sync::protocol::ObjectKey<Self>, value: &Option<Self::Payload>) -> sync::protocol::Result<()> {
                use sync::protocol::SyncError;
                let result = match (&key.collection, value) {
                    $(($collection::$variant, Some($payload::$value(value))) => <$mapping as $crate::row_policy::RowMapping<$entity>>::validate(key.id, Some(value)),
                    ($collection::$variant, None) => <$mapping as $crate::row_policy::RowMapping<$entity>>::validate(key.id, None),)+
                    _ => return Err(SyncError::Invalid("collection/payload mismatch".into())),
                };
                if key.id.is_nil() { return Err(SyncError::Invalid("nil source UUID".into())); }
                if let Some(value) = value {
                    for (tag, id) in Self::references(value) {
                        if Self::collection_from_tag(tag).is_none() || id.is_nil() {
                            return Err(SyncError::Invalid("invalid declared reference".into()));
                        }
                    }
                }
                result.map_err(|e| SyncError::Invalid(format!("{} {}: {}", Self::collection_tag(&key.collection), key.id, e)))
            }
        }
        #[async_trait::async_trait]
        impl $crate::registry::PolicyRegistry for $model {
            fn validate_registry() -> Result<(), sea_orm::DbErr> { <$model>::validate_registry() }
            fn authoring_owner(value: &Self::Payload) -> Option<uuid::Uuid> {
                match value { $($payload::$value(value) => <$mapping as $crate::row_policy::RowMapping<$entity>>::authoring_owner(value)),+ }
            }
            fn projection_ceiling(collection: &Self::Collection) -> $crate::owned_list::ListBudget {
                match collection { $($collection::$variant => <$mapping as $crate::row_policy::RowMapping<$entity>>::projection_ceiling()),+ }
            }
            async fn scan_source_on(tx: &sea_orm::DatabaseTransaction, collection: &Self::Collection, after: Option<i64>, limits: $crate::row_policy::ScanLimits) -> anyhow::Result<$crate::row_policy::ScanPage<Self::Payload>> {
                Ok(Self::scan_on(tx, collection, after, limits).await?)
            }
            async fn encode_selected_on(tx: &sea_orm::DatabaseTransaction, key: &sync::protocol::ObjectKey<Self>) -> anyhow::Result<Option<Self::Payload>> {
                Ok(Self::encode_on(tx, key).await?)
            }
            async fn project_selected_on(tx: &sea_orm::DatabaseTransaction, key: &sync::protocol::ObjectKey<Self>, value: Option<&Self::Payload>) -> anyhow::Result<()> {
                Self::project_on(tx, key, value).await?;
                Ok(())
            }
        }
        impl $model {
            pub fn references(value: &$payload) -> Vec<(&'static str, uuid::Uuid)> {
                match value { $($payload::$value(value) => <$mapping as $crate::row_policy::RowMapping<$entity>>::references(value)),+ }
            }
            pub async fn local_id_on<C: sea_orm::ConnectionTrait>(db: &C, key: &sync::protocol::ObjectKey<Self>) -> Result<Option<i64>, sea_orm::DbErr> {
                match key.collection { $($collection::$variant => $crate::row_policy::RowPolicy::<$entity, $mapping>::local_id_on(db, key.id).await),+ }
            }
            pub async fn source_uuid_on<C: sea_orm::ConnectionTrait>(db: &C, collection: &$collection, local_id: i64) -> Result<Option<uuid::Uuid>, sea_orm::DbErr> {
                match collection { $($collection::$variant => $crate::row_policy::RowPolicy::<$entity, $mapping>::source_uuid_on(db, local_id).await),+ }
            }
            pub async fn contains_source_on<C: sea_orm::ConnectionTrait>(db: &C, key: &sync::protocol::ObjectKey<Self>) -> Result<bool, sea_orm::DbErr> {
                use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, SelectExt};
                match key.collection {
                    $($collection::$variant => <$entity>::find()
                        .filter(<$mapping as $crate::row_policy::RowMapping<$entity>>::uuid_column().eq(key.id.to_string()))
                        .filter(<$mapping as $crate::row_policy::RowMapping<$entity>>::source_filter())
                        .exists(db).await),+
                }
            }
            pub async fn missing_references_on<C: sea_orm::ConnectionTrait>(db: &C, value: &$payload, max_reads: u32) -> Result<Vec<sync::protocol::ObjectKey<Self>>, sea_orm::DbErr> {
                use sync::protocol::SyncModel;
                use std::collections::BTreeSet;
                let mut keys = BTreeSet::new();
                for (tag, id) in Self::references(value) {
                    let collection = Self::collection_from_tag(tag).ok_or_else(|| sea_orm::DbErr::Custom("unknown reference target".into()))?;
                    if id.is_nil() { return Err(sea_orm::DbErr::Custom("nil reference target".into())); }
                    keys.insert(sync::protocol::ObjectKey::<Self> { collection, id });
                }
                if keys.len() as u64 > u64::from(max_reads) { return Err(sea_orm::DbErr::Custom("reference read budget exceeded".into())); }
                let mut missing = Vec::new();
                for key in keys { if !Self::contains_source_on(db, &key).await? { missing.push(key); } }
                Ok(missing)
            }
            /// Check static registration before opening a store.
            pub fn validate_registry() -> Result<(), sea_orm::DbErr> {
                use sync::protocol::SyncModel;
                use std::collections::{HashMap, HashSet};
                let mut tags = HashSet::new();
                let mut owners = HashMap::new();
                if Self::model_id().is_empty() || Self::schema_id().is_empty() {
                    return Err(sea_orm::DbErr::Custom("empty model or schema identity".into()));
                }
                for collection in Self::collections() {
                    let tag = Self::collection_tag(&collection);
                    if tag.is_empty() || tag.len() > sync::protocol::MAX_TAG_BYTES || !tags.insert(tag) {
                        return Err(sea_orm::DbErr::Custom(format!("invalid or duplicate tag: {}", tag)));
                    }
                }
                $(
                    {
                        use sea_orm::EntityName;
                        let entity = <$entity>::default();
                        let table = (entity.schema_name().map(str::to_owned), entity.table_name());
                        if let Some(previous) = owners.insert(table.clone(), $tag) {
                            return Err(sea_orm::DbErr::Custom(format!("duplicate primary source table {:?}: {} and {}", table, previous, $tag)));
                        }
                    }
                    $crate::row_policy::RowPolicy::<$entity, $mapping>::validate_mapping()?;
                    for target in <$mapping as $crate::row_policy::RowMapping<$entity>>::reference_tags() {
                        if !tags.contains(target) { return Err(sea_orm::DbErr::Custom(format!("unknown reference target: {}", target))); }
                    }
                )+
                Ok(())
            }
            /// Validate actual source schema before opening a policy-backed store.
            pub async fn validate_database_on<C: sea_orm::ConnectionTrait>(db: &C) -> Result<(), sea_orm::DbErr> {
                Self::validate_registry()?;
                $($crate::row_policy::RowPolicy::<$entity, $mapping>::validate_database_on(db).await?;)+
                Ok(())
            }
            pub async fn project_on(db: &sea_orm::DatabaseTransaction, key: &sync::protocol::ObjectKey<Self>, value: Option<&$payload>) -> Result<(), sea_orm::DbErr> {
                use sync::protocol::SyncModel;
                Self::validate(key, &value.cloned()).map_err(|e| sea_orm::DbErr::Custom(format!("{:?}", e)))?;
                match (&key.collection, value) {
                    $(($collection::$variant, Some($payload::$value(value))) => $crate::row_policy::RowPolicy::<$entity, $mapping>::project_on(db, key.id, Some(value)).await,
                    ($collection::$variant, None) => $crate::row_policy::RowPolicy::<$entity, $mapping>::project_on(db, key.id, None).await,)+
                    _ => Err(sea_orm::DbErr::Custom("collection/payload mismatch".into())),
                }
            }
            /// Apply a standalone command atomically. Receive paths use project_on.
            pub async fn project(db: &sea_orm::DatabaseConnection, key: &sync::protocol::ObjectKey<Self>, value: Option<&$payload>) -> Result<(), sea_orm::DbErr> {
                use sea_orm::TransactionTrait;
                let tx = db.begin().await?;
                Self::project_on(&tx, key, value).await?;
                tx.commit().await
            }
            pub async fn scan_on(tx: &sea_orm::DatabaseTransaction, collection: &$collection, after: Option<i64>, limits: $crate::row_policy::ScanLimits) -> Result<$crate::row_policy::ScanPage<$payload>, sea_orm::DbErr> {
                match collection {
                    $($collection::$variant => {
                        let page = $crate::row_policy::RowPolicy::<$entity, $mapping>::scan_with_on(tx, after, limits, |value| {
                            let encoded = serde_json::to_vec(&$payload::$value(value.clone())).map_err(|e| sea_orm::DbErr::Custom(e.to_string()))?;
                            u64::try_from(encoded.len()).map_err(|e| sea_orm::DbErr::Custom(e.to_string()))
                        }).await?;
                        Ok($crate::row_policy::ScanPage { records: page.records.into_iter().map(|record| $crate::row_policy::ScanRecord { position: record.position, id: record.id, payload: $payload::$value(record.payload) }).collect(), next: page.next, done: page.done, rows_read: page.rows_read, bytes: page.bytes, mutations: page.mutations })
                    }),+
                }
            }
            pub async fn encode_on<C: sea_orm::ConnectionTrait>(db: &C, key: &sync::protocol::ObjectKey<Self>) -> Result<Option<$payload>, sea_orm::DbErr> {
                match key.collection {
                    $($collection::$variant => Ok($crate::row_policy::RowPolicy::<$entity, $mapping>::encode_on(db, key.id).await?.map($payload::$value)),)+
                }
            }
        }
    };
}

/// Declare each family once and generate its payload, mapping, and registry entry.
#[macro_export]
macro_rules! row_policies {
    (
        $vis:vis $model:ident, $collection:ident, $payload:ident {
            model: $model_id:expr, schema: $schema_id:expr,
            collections: { $(
                $variant:ident => {
                    tag: $tag:literal, entity: $entity:ty,
                    value: $value:ident, mapping: $mapping:ident,
                    policy: { $($policy:tt)* }
                }
            ),+ $(,)? }
        }
    ) => {
        $( $crate::row_mapping! { $vis struct $value, $mapping for $entity { $($policy)* } } )+
        $crate::row_registry! { $vis $model, $collection, $payload {
            model: $model_id, schema: $schema_id,
            collections: { $( $variant => { tag: $tag, entity: $entity, mapping: $mapping, value: $value } ),+ }
        } }
    };
}

/// A generated registry can project a selected value on the caller's transaction.
#[async_trait::async_trait]
pub trait PolicyRegistry: sync::protocol::SyncModel {
    fn validate_registry() -> Result<(), sea_orm::DbErr>;
    fn authoring_owner(value: &Self::Payload) -> Option<uuid::Uuid>;
    fn projection_ceiling(collection: &Self::Collection) -> crate::owned_list::ListBudget;
    async fn scan_source_on(
        tx: &sea_orm::DatabaseTransaction,
        collection: &Self::Collection,
        after: Option<i64>,
        limits: crate::row_policy::ScanLimits,
    ) -> anyhow::Result<crate::row_policy::ScanPage<Self::Payload>>;
    async fn encode_selected_on(
        tx: &sea_orm::DatabaseTransaction,
        key: &sync::protocol::ObjectKey<Self>,
    ) -> anyhow::Result<Option<Self::Payload>>;
    async fn project_selected_on(
        tx: &sea_orm::DatabaseTransaction,
        key: &sync::protocol::ObjectKey<Self>,
        value: Option<&Self::Payload>,
    ) -> anyhow::Result<()>;
}

/// Limits reserved before mutating any source row in a projection batch.
#[derive(Clone, Copy, Debug)]
pub struct ProjectionBudget {
    pub max_reads: u64,
    pub max_mutations: u64,
}

/// Connect a generated registry to the transactional causal endpoint.
pub struct RegistryProjection<M> {
    budget: ProjectionBudget,
    marker: std::marker::PhantomData<fn() -> M>,
}
impl<M: PolicyRegistry> RegistryProjection<M> {
    pub fn new(budget: ProjectionBudget) -> Result<Self, sea_orm::DbErr> {
        M::validate_registry()?;
        Ok(Self {
            budget,
            marker: std::marker::PhantomData,
        })
    }
}
#[async_trait::async_trait]
impl<M: PolicyRegistry> crate::causal_capabilities::TransactionProjection<M>
    for RegistryProjection<M>
{
    fn mutation_ceiling(
        &self,
        changes: &sync::protocol::ProjectionBatch<M>,
    ) -> anyhow::Result<u64> {
        let mut reads = 0u64;
        let mut mutations = 0u64;
        for change in &changes.changes {
            let bound = M::projection_ceiling(&change.key.collection);
            reads = reads
                .checked_add(bound.max_reads)
                .ok_or(SyncError::WorkLimit)?;
            mutations = mutations
                .checked_add(bound.max_mutations)
                .ok_or(SyncError::WorkLimit)?;
            if reads > self.budget.max_reads || mutations > self.budget.max_mutations {
                return Err(anyhow::anyhow!(SyncError::WorkLimit));
            }
        }
        Ok(mutations)
    }
    async fn apply(
        &self,
        tx: &sea_orm::DatabaseTransaction,
        changes: &sync::protocol::ProjectionBatch<M>,
    ) -> anyhow::Result<()> {
        self.mutation_ceiling(changes)?;
        for change in &changes.changes {
            M::project_selected_on(
                tx,
                &change.key,
                change.after.as_ref().map(|candidate| &candidate.payload),
            )
            .await?;
        }
        Ok(())
    }
}

/// Author a current source row and project the selected causal result in the
/// same transaction. The caller owns commit and initialization progress.
pub async fn author_source_on<M: PolicyRegistry>(
    tx: &sea_orm::DatabaseTransaction,
    store: &crate::causal_store::CausalReplica<M>,
    key: sync::protocol::ObjectKey<M>,
    observed: sync::protocol::Observed,
) -> anyhow::Result<crate::causal_store::CommandResult<M>> {
    let payload = M::encode_selected_on(tx, &key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("source row absent during authoring"))?;
    let result = store
        .write_observed_on(tx, key.clone(), Some(payload), observed)
        .await?;
    M::project_selected_on(
        tx,
        &key,
        result
            .projection
            .after
            .as_ref()
            .map(|candidate| &candidate.payload),
    )
    .await?;
    Ok(result)
}
