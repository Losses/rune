//! Bounded reconciliation of rows owned by one source value.
//! Callers hold the namespace writer lock from preparation through commit.
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, IdenStatic,
    QueryFilter, QueryOrder, QuerySelect, QueryTrait, Value, sea_query::Expr,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
};

/// Generated item mapping. It contains no database access or authoring hook.
pub trait OwnedListMapping<E: EntityTrait> {
    type Item: Clone + PartialEq;
    fn max_items() -> u32;
    fn reference_tags() -> Vec<&'static str> {
        Vec::new()
    }
    fn references(_value: &Self::Item) -> Vec<(&'static str, uuid::Uuid)> {
        Vec::new()
    }
    fn owners() -> Vec<E::Column>;
    fn constants() -> Vec<(E::Column, Value)>;
    fn key_column() -> E::Column;
    fn order_column() -> Option<E::Column> {
        None
    }
    fn position(_item: &Self::Item) -> Option<i32> {
        None
    }
    fn fields() -> Vec<E::Column>;
    fn defaults() -> Vec<(E::Column, Value)>;
    fn key(item: &Self::Item) -> String;
    fn validate(item: &Self::Item) -> Result<(), DbErr>;
    fn encode(row: &crate::row_policy::RowView<E>) -> Result<Self::Item, DbErr>;
    fn values(item: &Self::Item) -> Vec<Value>;
}
#[derive(Clone, Copy, Debug)]
pub struct ListBudget {
    pub max_reads: u64,
    pub max_mutations: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListWork {
    pub rows_read: u64,
    pub mutations: u64,
}
type PlannedWrite<E> = (
    String,
    bool,
    Vec<<E as EntityTrait>::Column>,
    <E as EntityTrait>::ActiveModel,
);

/// A validated, bounded diff. Apply only in the transaction used to prepare it.
pub struct OwnedListPlan<E: EntityTrait> {
    predicate: Condition,
    key_column: E::Column,
    deletes: Vec<String>,
    writes: Vec<PlannedWrite<E>>,
    order: Option<E::Column>,
    pub work: ListWork,
}
impl<E: EntityTrait> OwnedListPlan<E> {
    pub async fn apply_on(self, tx: &sea_orm::DatabaseTransaction) -> Result<ListWork, DbErr> {
        for key in self.deletes {
            let result = E::delete_many()
                .filter(self.predicate.clone())
                .filter(self.key_column.eq(key))
                .exec(tx)
                .await?;
            if result.rows_affected != 1 {
                return Err(DbErr::Custom(
                    "owned-list row changed after planning".into(),
                ));
            }
        }
        if let Some(column) = self.order {
            for (index, (key, is_new, _, _)) in self.writes.iter().enumerate() {
                if !is_new {
                    let position =
                        -1 - i32::try_from(index).map_err(|e| DbErr::Custom(e.to_string()))?;
                    let result = E::update_many()
                        .filter(self.predicate.clone())
                        .filter(self.key_column.eq(key.clone()))
                        .col_expr(column, Expr::value(position))
                        .exec(tx)
                        .await?;
                    if result.rows_affected != 1 {
                        return Err(DbErr::Custom(
                            "owned-list row changed while reserving order".into(),
                        ));
                    }
                }
            }
        }
        for (key, is_new, fields, active) in self.writes {
            if is_new {
                E::insert(active).exec(tx).await?;
            } else {
                let mut update = E::update_many()
                    .filter(self.predicate.clone())
                    .filter(self.key_column.eq(key));
                for column in fields {
                    update = update.col_expr(column, Expr::value(active.get(column).unwrap()));
                }
                if update.exec(tx).await?.rows_affected != 1 {
                    return Err(DbErr::Custom(
                        "owned-list row changed after planning".into(),
                    ));
                }
            }
        }
        Ok(self.work)
    }
}
pub struct OwnedListPolicy<E: EntityTrait, M: OwnedListMapping<E>>(PhantomData<(E, M)>);
impl<E: EntityTrait, M: OwnedListMapping<E>> OwnedListPolicy<E, M> {
    fn select() -> sea_orm::Select<E> {
        E::find()
            .select_only()
            .column(M::key_column())
            .columns(M::fields())
    }
    pub fn validate_items(items: &[M::Item]) -> Result<(), DbErr> {
        if items.len() > M::max_items() as usize {
            return Err(DbErr::Custom("owned list exceeds declared bound".into()));
        }
        let mut keys = BTreeSet::new();
        for (index, item) in items.iter().enumerate() {
            M::validate(item)?;
            if M::key(item).is_empty() || !keys.insert(M::key(item)) {
                return Err(DbErr::Custom("invalid or duplicate owned-list key".into()));
            }
            if let Some(position) = M::position(item)
                && usize::try_from(position).ok() != Some(index)
            {
                return Err(DbErr::Custom(
                    "ordered list requires contiguous positions in payload order".into(),
                ));
            }
        }
        Ok(())
    }
    pub fn validate_mapping() -> Result<(), DbErr> {
        if M::max_items() == 0 || M::max_items() > 65_536 {
            return Err(DbErr::Custom("invalid owned-list bound".into()));
        }
        let mut seen = BTreeSet::new();
        for column in M::owners()
            .into_iter()
            .chain(M::constants().into_iter().map(|(c, _)| c))
            .chain(std::iter::once(M::key_column()))
            .chain(M::fields())
            .chain(M::defaults().into_iter().map(|(c, _)| c))
        {
            if !seen.insert(column.as_str()) {
                return Err(DbErr::Custom(format!(
                    "overlapping owned-list column {}",
                    column.as_str()
                )));
            }
        }
        if M::owners().is_empty() {
            return Err(DbErr::Custom("owned list needs an owner predicate".into()));
        }
        if let Some(order) = M::order_column()
            && !M::fields()
                .iter()
                .any(|column| column.as_str() == order.as_str())
        {
            return Err(DbErr::Custom(
                "list order must be a replicated item field".into(),
            ));
        }
        Ok(())
    }
    fn predicate(owner: &[Value]) -> Result<Condition, DbErr> {
        let columns = M::owners();
        if columns.len() != owner.len() {
            return Err(DbErr::Custom("owner value count mismatch".into()));
        }
        let mut active = <E::ActiveModel as ActiveModelTrait>::default();
        let mut condition = Condition::all();
        for (column, value) in columns
            .into_iter()
            .zip(owner.iter().cloned())
            .chain(M::constants())
        {
            active.try_set(column, value.clone())?;
            condition = condition.add(column.eq(value));
        }
        Ok(condition)
    }
    /// Maximum rows that may be read for one reconciliation.
    pub fn read_ceiling() -> u64 {
        u64::from(M::max_items()) + 1
    }
    /// Read no more than the declared maximum plus one diagnostic row.
    pub async fn read_on<C: ConnectionTrait>(
        db: &C,
        owner: &[Value],
        max_reads: u64,
    ) -> Result<Vec<M::Item>, DbErr> {
        Self::validate_mapping()?;
        if max_reads < Self::read_ceiling() {
            return Err(DbErr::Custom(
                "owned-list read budget cannot cover bound".into(),
            ));
        }
        let mut query = Self::select().filter(Self::predicate(owner)?);
        if let Some(column) = M::order_column() {
            query = query.order_by_asc(column);
        }
        let query = query
            .order_by_asc(M::key_column())
            .limit(u64::from(M::max_items()) + 1);
        let rows = db
            .query_all_raw(query.build(db.get_database_backend()))
            .await?;
        if rows.len() > M::max_items() as usize {
            return Err(DbErr::Custom(
                "existing owned list exceeds declared bound".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let item = M::encode(&crate::row_policy::RowView::new(row))?;
            M::validate(&item)?;
            if !seen.insert(M::key(&item)) {
                return Err(DbErr::Custom("duplicate owned-list key".into()));
            }
            items.push(item);
        }
        Self::validate_items(&items)?;
        Ok(items)
    }
    /// Prepare a bounded diff before any source mutation.
    pub async fn prepare_on(
        db: &sea_orm::DatabaseTransaction,
        owner: &[Value],
        incoming: &[M::Item],
        budget: ListBudget,
    ) -> Result<OwnedListPlan<E>, DbErr> {
        Self::validate_mapping()?;
        Self::validate_items(incoming)?;
        if incoming.len() > M::max_items() as usize {
            return Err(DbErr::Custom(
                "incoming owned list exceeds declared bound".into(),
            ));
        }
        let ceiling = u64::from(M::max_items()) + 1;
        if budget.max_reads < ceiling {
            return Err(DbErr::Custom(
                "owned-list read budget cannot cover bound".into(),
            ));
        }
        let predicate = Self::predicate(owner)?;
        let query = Self::select()
            .filter(predicate.clone())
            .limit(u64::from(M::max_items()) + 1);
        let rows = db
            .query_all_raw(query.build(db.get_database_backend()))
            .await?;
        let reads = rows.len() as u64;
        if reads > budget.max_reads {
            return Err(DbErr::Custom("owned-list read budget exceeded".into()));
        }
        if rows.len() > M::max_items() as usize {
            return Err(DbErr::Custom(
                "existing owned list exceeds declared bound".into(),
            ));
        }
        let mut old = BTreeMap::new();
        for row in rows {
            let item = M::encode(&crate::row_policy::RowView::new(row))?;
            M::validate(&item)?;
            if M::position(&item).is_some_and(|position| position < 0) {
                return Err(DbErr::Custom("negative stored list position".into()));
            }
            let key = M::key(&item);
            if key.is_empty() || old.insert(key, item).is_some() {
                return Err(DbErr::Custom(
                    "invalid or duplicate existing item key".into(),
                ));
            }
        }
        let mut new = BTreeMap::new();
        for item in incoming {
            M::validate(item)?;
            let key = M::key(item);
            if key.is_empty() || new.insert(key, item).is_some() {
                return Err(DbErr::Custom(
                    "invalid or duplicate incoming item key".into(),
                ));
            }
        }
        let mut deletes = Vec::new();
        let mut writes = Vec::new();
        for key in old.keys() {
            if !new.contains_key(key) {
                deletes.push(key.clone());
            }
        }
        for (key, item) in &new {
            if old.get(key).is_some_and(|old| old == *item) {
                continue;
            }
            let fields = M::fields();
            let values = M::values(item);
            if fields.len() != values.len() {
                return Err(DbErr::Custom("owned-list field count mismatch".into()));
            }
            let mut active = <E::ActiveModel as ActiveModelTrait>::default();
            for (column, value) in fields.iter().copied().zip(values) {
                active.try_set(column, value)?;
            }
            let is_new = !old.contains_key(key);
            if is_new {
                for (column, value) in M::owners()
                    .into_iter()
                    .zip(owner.iter().cloned())
                    .chain(M::constants())
                    .chain(std::iter::once((M::key_column(), key.clone().into())))
                    .chain(M::defaults())
                {
                    active.try_set(column, value)?;
                }
            }
            writes.push((key.clone(), is_new, fields, active));
        }
        let staged = if M::order_column().is_some() {
            writes.iter().filter(|(_, is_new, _, _)| !is_new).count()
        } else {
            0
        };
        let mutations = (deletes.len() + writes.len() + staged) as u64;
        if mutations > budget.max_mutations {
            return Err(DbErr::Custom("owned-list mutation budget exceeded".into()));
        }
        Ok(OwnedListPlan {
            predicate,
            key_column: M::key_column(),
            deletes,
            writes,
            order: M::order_column(),
            work: ListWork {
                rows_read: reads,
                mutations,
            },
        })
    }
    /// Prepare and apply a diff in the caller's transaction.
    pub async fn reconcile_on(
        db: &sea_orm::DatabaseTransaction,
        owner: &[Value],
        incoming: &[M::Item],
        budget: ListBudget,
    ) -> Result<ListWork, DbErr> {
        Self::prepare_on(db, owner, incoming, budget)
            .await?
            .apply_on(db)
            .await
    }
}

/// Declare item fields and owned columns without per-entity persistence code.
#[macro_export]
macro_rules! owned_list_mapping {
    (
        $vis:vis struct $item:ident, $mapping:ident for $entity:ty {
            max_items: $max:expr,
            owners: [$($owner:expr),+ $(,)?],
            constants: { $($constant:expr => $constant_value:expr),* $(,)? },
            key: $key:ident : $key_type:ty => $key_column:expr,
            fields: { $($field:ident : $ty:ty => $column:expr),* $(,)? },
            defaults: { $($default:expr => $default_value:expr),* $(,)? },
            $(order: $position:ident => $order_column:expr,)?
            $(references: { $($reference:ident => $target:literal),* $(,)? },)?
            $(reference_targets: [$($target_tag:literal),* $(,)?],)?
            $(reference_values: $reference_values:expr,)?
            $(limits: { $($bounded:ident => ($lower:expr, $upper:expr)),* $(,)? },)?
            $(validate: $validate:expr,)?
        }
    ) => {
        #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        $vis struct $item { pub $key: $key_type, $(pub $field: $ty),* }
        $vis struct $mapping;
        impl $crate::owned_list::OwnedListMapping<$entity> for $mapping {
            type Item = $item;
            fn max_items() -> u32 { $max }
            fn reference_tags() -> Vec<&'static str> { vec![$($($target,)*)? $($($target_tag,)*)?] }
            fn references(value: &Self::Item) -> Vec<(&'static str, uuid::Uuid)> {
                #[allow(unused_mut)]
                let mut references = Vec::new();
                $($( if let Some(id) = $crate::row_policy::ReferenceField::reference(&value.$reference) { references.push(($target, id)); } )*)?
                $(references.extend(($reference_values)(value));)?
                let _ = value;
                references
            }
            fn owners() -> Vec<<$entity as sea_orm::EntityTrait>::Column> { vec![$($owner),+] }
            fn constants() -> Vec<(<$entity as sea_orm::EntityTrait>::Column, sea_orm::Value)> { vec![$(($constant, ($constant_value).into())),*] }
            fn key_column() -> <$entity as sea_orm::EntityTrait>::Column { $key_column }
            $(fn order_column() -> Option<<$entity as sea_orm::EntityTrait>::Column> { Some($order_column) }
            fn position(item: &Self::Item) -> Option<i32> { Some(item.$position) })?
            fn fields() -> Vec<<$entity as sea_orm::EntityTrait>::Column> { vec![$($column),*] }
            fn defaults() -> Vec<(<$entity as sea_orm::EntityTrait>::Column, sea_orm::Value)> { vec![$(($default, ($default_value).into())),*] }
            fn key(item: &Self::Item) -> String { item.$key.to_string() }
            fn validate(item: &Self::Item) -> Result<(), sea_orm::DbErr> {
                <$key_type as $crate::row_policy::FieldCodec>::validate(&item.$key)?;
                $(<$ty as $crate::row_policy::FieldCodec>::validate(&item.$field)?;)*
                $($( $crate::row_policy::require_reference(&item.$reference); )*)?
                $($( $crate::row_policy::BoundedField::check_bounds(&item.$bounded, $lower, $upper)?; )*)?
                $(let result: Result<(), sea_orm::DbErr> = ($validate)(item); result?;)?
                Ok(())
            }
            fn encode(row: &$crate::row_policy::RowView<$entity>) -> Result<Self::Item, sea_orm::DbErr> {
                Ok($item { $key: row.read::<$key_type>($key_column)?, $($field: row.read::<$ty>($column)?),* })
            }
            fn values(item: &Self::Item) -> Vec<sea_orm::Value> { vec![$(<$ty as $crate::row_policy::FieldCodec>::to_column(&item.$field)),*] }
        }
    };
}
